use glam::{Quat, Vec3};
use openhp1_package::{ObjectReader, Package};

use crate::{
    Error, Result, SkeletalAnimation,
    decode::{count, read_anim_sequences, read_name, read_vec},
    types::{AnimationBone, AnimationMove, AnimationTrack},
};

impl SkeletalAnimation {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let export = &package.summary().exports[export_index];
        let class = package.summary().class_name(export).unwrap_or_default();
        if class != "Animation" {
            return Err(Error::UnsupportedAnimationClass(class.to_owned()));
        }

        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}
        let bones = read_animation_bones(&mut reader)?;
        let descriptors = read_moves(&mut reader, bones.len())?;
        let sequences = read_anim_sequences(&mut reader)?;
        if sequences.len() != descriptors.len() {
            return Err(Error::InvalidSkeletalSequenceLayout {
                sequence_count: sequences.len(),
                move_count: descriptors.len(),
            });
        }

        let rotations = read_vec(&mut reader, "packed animation rotations", |reader| {
            let vector = Vec3::new(
                unpack_rotation_component(reader.read_i16()?),
                unpack_rotation_component(reader.read_i16()?),
                unpack_rotation_component(reader.read_i16()?),
            );
            let squared = vector.length_squared();
            if !squared.is_finite() || squared > 1.001 {
                return Err(Error::InvalidFloat {
                    field: "packed animation rotation",
                    value: squared,
                });
            }
            Ok(Quat::from_xyzw(
                vector.x,
                vector.y,
                vector.z,
                (1.0 - squared).max(0.0).sqrt(),
            )
            .normalize())
        })?;
        let positions = read_vec(&mut reader, "packed animation positions", |reader| {
            Ok([reader.read_i16()?, reader.read_i16()?, reader.read_i16()?])
        })?;
        let times = read_vec(&mut reader, "packed animation times", |reader| {
            Ok(reader.read_u8()?)
        })?;
        if reader.remaining() != 0 {
            return Err(Error::TrailingData {
                object: "skeletal animation",
                remaining: reader.remaining(),
            });
        }

        let mut rotation_offset = 0;
        let mut position_offset = 0;
        let mut time_offset = 0;
        let mut moves = Vec::with_capacity(descriptors.len());
        for movement in descriptors {
            let mut tracks = Vec::with_capacity(movement.tracks.len());
            for track in movement.tracks {
                let track_rotations = take(
                    &rotations,
                    &mut rotation_offset,
                    track.rotations,
                    "packed animation rotations",
                )?
                .to_vec();
                let track_positions = take(
                    &positions,
                    &mut position_offset,
                    track.positions,
                    "packed animation positions",
                )?
                .iter()
                .map(|position| {
                    Vec3::new(
                        f32::from(position[0]),
                        f32::from(position[1]),
                        f32::from(position[2]),
                    ) * (track.position_scale / 32_767.0)
                })
                .collect::<Vec<_>>();
                let mut elapsed = 0.0;
                let track_times = take(
                    &times,
                    &mut time_offset,
                    track.times,
                    "packed animation times",
                )?
                .iter()
                .map(|delta| {
                    elapsed += f32::from(*delta) * track.time_scale;
                    elapsed
                })
                .collect::<Vec<_>>();
                validate_track(
                    track_rotations.len(),
                    track_positions.len(),
                    track_times.len(),
                )?;
                tracks.push(AnimationTrack {
                    rotations: track_rotations,
                    positions: track_positions,
                    times: track_times,
                });
            }
            moves.push(AnimationMove {
                track_time: movement.track_time,
                start_bone: movement.start_bone,
                bone_indices: movement.bone_indices,
                tracks,
            });
        }
        if rotation_offset != rotations.len()
            || position_offset != positions.len()
            || time_offset != times.len()
        {
            return Err(Error::InvalidSkeletalTrack {
                quaternions: rotations.len().saturating_sub(rotation_offset),
                positions: positions.len().saturating_sub(position_offset),
                times: times.len().saturating_sub(time_offset),
            });
        }
        Ok(Self {
            sequences,
            bones,
            moves,
        })
    }
}

struct MoveDescriptor {
    track_time: f32,
    start_bone: usize,
    bone_indices: Vec<usize>,
    tracks: Vec<TrackDescriptor>,
}

struct TrackDescriptor {
    rotations: usize,
    positions: usize,
    times: usize,
    position_scale: f32,
    time_scale: f32,
}

fn read_animation_bones(reader: &mut ObjectReader<'_>) -> Result<Vec<AnimationBone>> {
    let records = read_vec(reader, "animation reference bones", |reader| {
        let name = read_name(reader, "animation reference bone")?;
        reader.read_u32()?; // flags
        let parent = usize::try_from(reader.read_u32()?).map_err(|_| Error::InvalidIndex {
            field: "animation bone parent",
            index: usize::MAX,
            length: 0,
        })?;
        Ok((AnimationBone { name }, parent))
    })?;
    for (_, parent) in &records {
        if *parent >= records.len() {
            return Err(Error::InvalidIndex {
                field: "animation bone parent",
                index: *parent,
                length: records.len(),
            });
        }
    }
    Ok(records.into_iter().map(|(bone, _)| bone).collect())
}

fn read_moves(reader: &mut ObjectReader<'_>, bone_count: usize) -> Result<Vec<MoveDescriptor>> {
    read_vec(reader, "animation moves", |reader| {
        let root_speed = Vec3::new(reader.read_f32()?, reader.read_f32()?, reader.read_f32()?);
        let track_time = reader.read_f32()?;
        let start_bone = usize::try_from(reader.read_u32()?).map_err(|_| Error::InvalidIndex {
            field: "animation start bone",
            index: usize::MAX,
            length: bone_count,
        })?;
        reader.read_u32()?; // flags
        if !root_speed.is_finite() {
            return Err(Error::InvalidFloat {
                field: "animation root speed",
                value: f32::NAN,
            });
        }
        if !track_time.is_finite() || track_time < 0.0 {
            return Err(Error::InvalidFloat {
                field: "animation track time",
                value: track_time,
            });
        }
        let bone_indices = read_vec(reader, "animation bone indices", |reader| {
            let index = usize::try_from(reader.read_u32()?).map_err(|_| Error::InvalidIndex {
                field: "animation bone",
                index: usize::MAX,
                length: bone_count,
            })?;
            if index >= bone_count {
                return Err(Error::InvalidIndex {
                    field: "animation bone",
                    index,
                    length: bone_count,
                });
            }
            Ok(index)
        })?;
        let tracks = read_vec(reader, "animation tracks", |reader| {
            reader.read_u32()?; // flags
            let rotations = count(reader.read_compact_index()?, "animation rotations")?;
            let positions = count(reader.read_compact_index()?, "animation positions")?;
            let times = count(reader.read_compact_index()?, "animation times")?;
            let position_scale = reader.read_f32()?;
            let time_scale = reader.read_f32()?;
            if !position_scale.is_finite() || position_scale < 0.0 {
                return Err(Error::InvalidFloat {
                    field: "animation position scale",
                    value: position_scale,
                });
            }
            if !time_scale.is_finite() || time_scale < 0.0 {
                return Err(Error::InvalidFloat {
                    field: "animation time scale",
                    value: time_scale,
                });
            }
            validate_track(rotations, positions, times)?;
            Ok(TrackDescriptor {
                rotations,
                positions,
                times,
                position_scale,
                time_scale,
            })
        })?;
        if !bone_indices.is_empty() && bone_indices.len() != tracks.len() {
            return Err(Error::InvalidSkeletalTrack {
                quaternions: tracks.len(),
                positions: bone_indices.len(),
                times: 0,
            });
        }
        let end_bone = start_bone
            .checked_add(tracks.len())
            .ok_or(Error::InvalidIndex {
                field: "animation start bone",
                index: usize::MAX,
                length: bone_count,
            })?;
        if bone_indices.is_empty() && end_bone > bone_count {
            return Err(Error::InvalidIndex {
                field: "animation start bone",
                index: end_bone,
                length: bone_count,
            });
        }
        Ok(MoveDescriptor {
            track_time,
            start_bone,
            bone_indices,
            tracks,
        })
    })
}

fn unpack_rotation_component(value: i16) -> f32 {
    (f32::from(value) * (std::f32::consts::FRAC_PI_2 / 32_767.0)).sin()
}

fn validate_track(rotations: usize, positions: usize, times: usize) -> Result<()> {
    let keyed = |count| count <= 1 || count == times;
    if !keyed(rotations) || !keyed(positions) || (times == 0 && (rotations > 1 || positions > 1)) {
        return Err(Error::InvalidSkeletalTrack {
            quaternions: rotations,
            positions,
            times,
        });
    }
    Ok(())
}

fn take<'a, T>(
    values: &'a [T],
    offset: &mut usize,
    count: usize,
    field: &'static str,
) -> Result<&'a [T]> {
    let end = offset.checked_add(count).ok_or(Error::InvalidIndex {
        field,
        index: usize::MAX,
        length: values.len(),
    })?;
    let selected = values.get(*offset..end).ok_or(Error::InvalidIndex {
        field,
        index: end,
        length: values.len(),
    })?;
    *offset = end;
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_support::{package, push_f32, push_i32, push_u32},
        types::{SkeletalBone, SkeletalInfluence, SkeletalMesh},
    };

    #[test]
    fn decodes_and_samples_hp1_compressed_tracks() {
        let names = [
            "None",
            "Core",
            "Class",
            "Animation",
            "TestAnimation",
            "Root",
            "Wave",
            "Movement",
            "Notify",
        ];
        let mut payload = vec![0, 1, 5];
        push_u32(&mut payload, 0);
        push_u32(&mut payload, 0);
        payload.push(1);
        for value in [0.0, 0.0, 0.0, 1.0] {
            push_f32(&mut payload, value);
        }
        push_u32(&mut payload, 0);
        push_u32(&mut payload, 0);
        payload.push(1);
        push_u32(&mut payload, 0);
        payload.push(1);
        push_u32(&mut payload, 0);
        payload.extend([2, 1, 2]);
        push_f32(&mut payload, 2.0);
        push_f32(&mut payload, 0.5);
        payload.extend([1, 6, 7]);
        push_i32(&mut payload, 0);
        push_i32(&mut payload, 2);
        payload.push(1);
        push_f32(&mut payload, 0.25);
        payload.push(8);
        push_f32(&mut payload, 2.0);
        payload.push(2);
        for value in [0_i16, 0, 0, 0, 0, 16_384] {
            payload.extend(value.to_le_bytes());
        }
        payload.push(1);
        for value in [16_384_i16, 0, 0] {
            payload.extend(value.to_le_bytes());
        }
        payload.extend([2, 0, 2]);

        let package = package("synthetic animation", &names, 3, 4, payload);
        let animation = SkeletalAnimation::decode(&package, 0).unwrap();
        assert_eq!(animation.sequences[0].name, "Wave");
        assert_eq!(animation.sequences[0].group, "Movement");
        assert_eq!(animation.sequences[0].notifications[0].function, "Notify");
        assert_eq!(animation.moves[0].tracks[0].times, [0.0, 1.0]);

        let mesh = SkeletalMesh {
            points: vec![Vec3::X],
            bones: vec![SkeletalBone {
                name: "Root".to_owned(),
                orientation: Quat::IDENTITY,
                position: Vec3::ZERO,
                parent: 0,
            }],
            influences: vec![vec![SkeletalInfluence {
                bone: 0,
                weight: 1.0,
                local_position: Vec3::X,
            }]],
            weapon_bone: Some(0),
            weapon_adjust: glam::Mat4::IDENTITY,
        };
        let sampled = animation.sample(&mesh, 0, 0.5).unwrap();
        assert!(sampled[0].abs_diff_eq(Vec3::new(1.707, 0.707, 0.0), 0.002));
    }
}
