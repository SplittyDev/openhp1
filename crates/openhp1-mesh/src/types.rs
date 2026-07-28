use glam::{IVec3, Mat4, Quat, Vec2, Vec3};
use openhp1_package::ObjectReference;

use crate::{
    Error, Result,
    geometry::{mirror_skeletal_position, sample_triangles, vertex_normals},
};

#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Vec3,
    /// Normalized texture coordinates.
    pub texture_coordinates: Vec2,
}

#[derive(Clone, Copy, Debug)]
pub struct MeshTriangle {
    pub vertices: [MeshVertex; 3],
    pub poly_flags: u32,
    pub texture_index: i32,
}

#[derive(Clone, Debug)]
pub struct MeshAnimationNotify {
    pub time: f32,
    pub function: String,
}

#[derive(Clone, Debug)]
pub struct MeshAnimationSequence {
    pub name: String,
    pub group: String,
    pub start_frame: usize,
    pub frame_count: usize,
    pub notifications: Vec<MeshAnimationNotify>,
    pub rate: f32,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub triangles: Vec<MeshTriangle>,
    pub textures: Vec<ObjectReference>,
    pub animation_sequences: Vec<MeshAnimationSequence>,
    pub frame_vertices: usize,
    pub animation_frames: usize,
    pub scale: Vec3,
    pub origin: Vec3,
    pub rotation_origin: IVec3,
    pub default_animation: ObjectReference,
    pub(crate) vertices: Vec<Vec3>,
    pub(crate) normals: Vec<Vec3>,
    pub(crate) face_vertices: Vec<[usize; 3]>,
    pub(crate) skeletal: Option<SkeletalMesh>,
}

impl Mesh {
    /// Samples a looping animation sequence at a normalized phase.
    ///
    /// Phase zero is the first frame and phase one wraps back to it.
    pub fn sample_sequence(
        &self,
        sequence: &MeshAnimationSequence,
        phase: f32,
    ) -> Result<Vec<MeshTriangle>> {
        if !phase.is_finite() {
            return Err(Error::InvalidAnimationPhase(phase));
        }
        if sequence.frame_count == 0 {
            return Err(Error::EmptyAnimationSequence {
                name: sequence.name.clone(),
            });
        }
        if self.frame_vertices == 0 || self.vertices.is_empty() {
            return Err(Error::NoVertexAnimation);
        }

        let frame = phase.rem_euclid(1.0) * sequence.frame_count as f32;
        let local_first = frame.floor() as usize % sequence.frame_count;
        let local_second = (local_first + 1) % sequence.frame_count;
        let blend = frame.fract();
        let sequence_frame = |local| {
            sequence
                .start_frame
                .checked_add(local)
                .ok_or_else(|| Error::InvalidAnimationSequence {
                    name: sequence.name.clone(),
                    start_frame: sequence.start_frame,
                    end_frame: usize::MAX,
                    animation_frames: self.animation_frames,
                })
        };
        let first_frame = sequence_frame(local_first)?;
        let second_frame = sequence_frame(local_second)?;
        let first = self.frame_slice(&self.vertices, first_frame)?;
        let second = self.frame_slice(&self.vertices, second_frame)?;
        let first_normals = self.frame_slice(&self.normals, first_frame)?;
        let second_normals = self.frame_slice(&self.normals, second_frame)?;
        let vertices = first
            .iter()
            .zip(second)
            .map(|(first, second)| first.lerp(*second, blend))
            .collect::<Vec<_>>();
        let normals = first_normals
            .iter()
            .zip(second_normals)
            .map(|(first, second)| first.lerp(*second, blend).normalize_or_zero())
            .collect::<Vec<_>>();
        sample_triangles(&self.triangles, &self.face_vertices, &vertices, &normals)
    }

    pub fn sample_skeletal_sequence(
        &self,
        animation: &SkeletalAnimation,
        sequence: usize,
        phase: f32,
    ) -> Result<Vec<MeshTriangle>> {
        if !phase.is_finite() {
            return Err(Error::InvalidAnimationPhase(phase));
        }
        let skeletal = self.skeletal.as_ref().ok_or(Error::NoSkeletalMesh)?;
        let vertices = animation.sample(skeletal, sequence, phase)?;
        let normals = vertex_normals(&vertices, &self.face_vertices);
        sample_triangles(&self.triangles, &self.face_vertices, &vertices, &normals)
    }

    pub fn sample_skeletal_sequence_with_root_motion(
        &self,
        animation: &SkeletalAnimation,
        sequence: usize,
        phase: f32,
    ) -> Result<(Vec<MeshTriangle>, Vec3)> {
        if !phase.is_finite() {
            return Err(Error::InvalidAnimationPhase(phase));
        }
        let skeletal = self.skeletal.as_ref().ok_or(Error::NoSkeletalMesh)?;
        let (vertices, root_motion) =
            animation.sample_with_root_motion(skeletal, sequence, phase)?;
        let normals = vertex_normals(&vertices, &self.face_vertices);
        Ok((
            sample_triangles(&self.triangles, &self.face_vertices, &vertices, &normals)?,
            root_motion,
        ))
    }

    fn frame_slice<'a>(&self, values: &'a [Vec3], frame: usize) -> Result<&'a [Vec3]> {
        let invalid_layout = || Error::InvalidAnimationLayout {
            frame_vertices: self.frame_vertices,
            animation_frames: self.animation_frames,
            vertex_count: values.len(),
        };
        let start = frame
            .checked_mul(self.frame_vertices)
            .ok_or_else(invalid_layout)?;
        let end = start
            .checked_add(self.frame_vertices)
            .ok_or_else(invalid_layout)?;
        values.get(start..end).ok_or(Error::InvalidAnimationFrame {
            frame,
            animation_frames: self.animation_frames,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SkeletalAnimation {
    pub sequences: Vec<MeshAnimationSequence>,
    pub(crate) bones: Vec<AnimationBone>,
    pub(crate) moves: Vec<AnimationMove>,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationBone {
    pub name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationMove {
    pub track_time: f32,
    pub start_bone: usize,
    pub bone_indices: Vec<usize>,
    pub tracks: Vec<AnimationTrack>,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationTrack {
    pub rotations: Vec<Quat>,
    pub positions: Vec<Vec3>,
    pub times: Vec<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct SkeletalMesh {
    pub points: Vec<Vec3>,
    pub bones: Vec<SkeletalBone>,
    pub influences: Vec<Vec<SkeletalInfluence>>,
}

#[derive(Clone, Debug)]
pub(crate) struct SkeletalBone {
    pub name: String,
    pub orientation: Quat,
    pub position: Vec3,
    pub parent: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SkeletalInfluence {
    pub bone: usize,
    pub weight: f32,
    pub local_position: Vec3,
}

impl SkeletalAnimation {
    pub(crate) fn sample(
        &self,
        mesh: &SkeletalMesh,
        sequence: usize,
        phase: f32,
    ) -> Result<Vec<Vec3>> {
        self.sample_pose(mesh, sequence, phase, false)
            .map(|(points, _)| points)
    }

    pub(crate) fn sample_with_root_motion(
        &self,
        mesh: &SkeletalMesh,
        sequence: usize,
        phase: f32,
    ) -> Result<(Vec<Vec3>, Vec3)> {
        self.sample_pose(mesh, sequence, phase, true)
    }

    fn sample_pose(
        &self,
        mesh: &SkeletalMesh,
        sequence: usize,
        phase: f32,
        extract_root_motion: bool,
    ) -> Result<(Vec<Vec3>, Vec3)> {
        let sequence_index = sequence;
        let sequence = self
            .sequences
            .get(sequence_index)
            .ok_or(Error::InvalidIndex {
                field: "skeletal animation sequence",
                index: sequence_index,
                length: self.sequences.len(),
            })?;
        if sequence.frame_count == 0 {
            return Err(Error::EmptyAnimationSequence {
                name: sequence.name.clone(),
            });
        }
        let movement =
            self.moves
                .get(sequence_index)
                .ok_or(Error::InvalidSkeletalSequenceLayout {
                    sequence_count: self.sequences.len(),
                    move_count: self.moves.len(),
                })?;
        let time = phase.rem_euclid(1.0) * movement.track_time;
        let mut local = mesh
            .bones
            .iter()
            .map(|bone| (bone.orientation, bone.position))
            .collect::<Vec<_>>();
        let mut root_motion = Vec3::ZERO;
        for (track_index, track) in movement.tracks.iter().enumerate() {
            let animation_bone = movement
                .bone_indices
                .get(track_index)
                .copied()
                .unwrap_or(movement.start_bone + track_index);
            let animation_bone_index = animation_bone;
            let Some(animation_bone) = self.bones.get(animation_bone_index) else {
                return Err(Error::InvalidIndex {
                    field: "animation bone",
                    index: animation_bone_index,
                    length: self.bones.len(),
                });
            };
            let mesh_bone = mesh
                .bones
                .iter()
                .position(|bone| bone.name.eq_ignore_ascii_case(&animation_bone.name))
                .or_else(|| {
                    (self.bones.len() == mesh.bones.len()
                        && animation_bone_index < mesh.bones.len())
                    .then_some(animation_bone_index)
                })
                .ok_or(Error::InvalidIndex {
                    field: "mesh animation bone",
                    index: track_index,
                    length: mesh.bones.len(),
                })?;
            let fallback = local[mesh_bone];
            local[mesh_bone] = track.sample(time, movement.track_time, fallback);
            if extract_root_motion && mesh_bone == 0 {
                let start = track.sample(0.0, movement.track_time, fallback).1;
                root_motion = local[mesh_bone].1 - start;
                local[mesh_bone].1 = start;
            }
        }

        let mut global = Vec::with_capacity(mesh.bones.len());
        for (index, bone) in mesh.bones.iter().enumerate() {
            let (orientation, position) = local[index];
            let orientation = orientation.conjugate();
            let transform = Mat4::from_rotation_translation(orientation, position);
            global.push(if bone.parent == index {
                transform
            } else {
                global[bone.parent] * transform
            });
        }

        let mut points = mesh.points.clone();
        for (point, influences) in points.iter_mut().zip(&mesh.influences) {
            if influences.is_empty() {
                continue;
            }
            *point = influences.iter().fold(Vec3::ZERO, |position, influence| {
                position
                    + global[influence.bone].transform_point3(influence.local_position)
                        * influence.weight
            });
        }
        points
            .iter_mut()
            .for_each(|point| *point = mirror_skeletal_position(*point));
        Ok((points, mirror_skeletal_position(root_motion)))
    }
}

impl AnimationTrack {
    fn sample(&self, time: f32, duration: f32, fallback: (Quat, Vec3)) -> (Quat, Vec3) {
        let (first, second, blend) = self.key_interval(time, duration);
        let orientation = match self.rotations.as_slice() {
            [] => fallback.0,
            [orientation] => *orientation,
            orientations => orientations[first]
                .slerp(orientations[second], blend)
                .normalize(),
        };
        let position = match self.positions.as_slice() {
            [] => fallback.1,
            [position] => *position,
            positions => positions[first].lerp(positions[second], blend),
        };
        (orientation, position)
    }

    fn key_interval(&self, time: f32, duration: f32) -> (usize, usize, f32) {
        if self.times.len() <= 1 {
            return (0, 0, 0.0);
        }
        let first = self
            .times
            .partition_point(|key| *key <= time)
            .saturating_sub(1)
            .min(self.times.len() - 1);
        let second = (first + 1) % self.times.len();
        let end = if second == 0 {
            duration
        } else {
            self.times[second]
        };
        let blend = if end > self.times[first] {
            (time - self.times[first]) / (end - self.times[first])
        } else {
            0.0
        };
        (first, second, blend.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_skeletal_root_translation_from_the_pose() {
        let mesh = SkeletalMesh {
            points: vec![Vec3::ZERO],
            bones: vec![SkeletalBone {
                name: "Root".to_owned(),
                orientation: Quat::IDENTITY,
                position: Vec3::ZERO,
                parent: 0,
            }],
            influences: vec![vec![SkeletalInfluence {
                bone: 0,
                weight: 1.0,
                local_position: Vec3::ZERO,
            }]],
        };
        let animation = SkeletalAnimation {
            sequences: vec![MeshAnimationSequence {
                name: "Move".to_owned(),
                group: "None".to_owned(),
                start_frame: 0,
                frame_count: 2,
                notifications: Vec::new(),
                rate: 1.0,
            }],
            bones: vec![AnimationBone {
                name: "Root".to_owned(),
            }],
            moves: vec![AnimationMove {
                track_time: 1.0,
                start_bone: 0,
                bone_indices: vec![0],
                tracks: vec![AnimationTrack {
                    rotations: Vec::new(),
                    positions: vec![Vec3::ZERO, Vec3::new(4.0, 2.0, 6.0)],
                    times: vec![0.0, 1.0],
                }],
            }],
        };

        let (points, motion) = animation.sample_with_root_motion(&mesh, 0, 0.5).unwrap();

        assert_eq!(points, vec![Vec3::ZERO]);
        assert_eq!(motion, Vec3::new(2.0, -1.0, 3.0));
    }
}
