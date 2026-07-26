use glam::{IVec3, Vec3};
use openhp1_package::{ObjectReader, Package};

use crate::{
    Error, Mesh, MeshAnimationNotify, MeshAnimationSequence, Result,
    geometry::{animation_normals, classic_triangles, lod_triangles},
};

impl Mesh {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let export = &package.summary().exports[export_index];
        let class = package.summary().class_name(export).unwrap_or_default();
        if !matches!(class, "Mesh" | "LodMesh" | "SkeletalMesh") {
            return Err(Error::UnsupportedClass(class.to_owned()));
        }

        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}
        skip_primitive(&mut reader)?;

        let vertices_end = lazy_end(&mut reader)?;
        let vertices = read_vec(&mut reader, "mesh vertices", |reader| {
            Ok(unpack_vertex(reader.read_i32()?))
        })?;
        expect_lazy_end(&reader, "mesh vertices", vertices_end)?;

        let triangles_end = lazy_end(&mut reader)?;
        let mesh_triangles = read_vec(&mut reader, "mesh triangles", |reader| {
            let indices = [reader.read_u16()?, reader.read_u16()?, reader.read_u16()?];
            let uv = [
                [reader.read_u8()?, reader.read_u8()?],
                [reader.read_u8()?, reader.read_u8()?],
                [reader.read_u8()?, reader.read_u8()?],
            ];
            Ok((indices, uv, reader.read_u32()?, reader.read_i32()?))
        })?;
        expect_lazy_end(&reader, "mesh triangles", triangles_end)?;

        let animation_sequences = read_anim_sequences(&mut reader)?;
        skip_lazy_vec(&mut reader, "mesh connections", 8)?;
        skip_box(&mut reader)?;
        skip_sphere(&mut reader)?;
        skip_lazy_vec(&mut reader, "mesh vertex links", 4)?;
        let textures = read_vec(&mut reader, "mesh textures", |reader| {
            Ok(reader.read_object_reference()?)
        })?;
        skip_vec(&mut reader, "mesh bounding boxes", |reader| {
            skip_box(reader)
        })?;
        skip_vec(&mut reader, "mesh bounding spheres", |reader| {
            skip_sphere(reader)
        })?;

        let frame_vertices = count(reader.read_i32()?, "frame vertices")?;
        let animation_frames = count(reader.read_i32()?, "animation frames")?;
        reader.read_u32()?; // and flags
        reader.read_u32()?; // or flags
        let scale = read_vec3(&mut reader)?;
        let origin = read_vec3(&mut reader)?;
        let rotation_origin =
            IVec3::new(reader.read_i32()?, reader.read_i32()?, reader.read_i32()?);
        reader.read_u32()?; // current polygon
        reader.read_u32()?; // current vertex
        match package.summary().header.version {
            65 => {
                reader.read_f32()?;
            }
            66.. => skip_vec(&mut reader, "texture LOD", |reader| {
                reader.read_f32()?;
                Ok(())
            })?,
            _ => {}
        }

        validate_animation(
            &vertices,
            frame_vertices,
            animation_frames,
            &animation_sequences,
        )?;
        let first_frame_vertices = if frame_vertices == 0 {
            vertices.len()
        } else {
            frame_vertices
        };
        let geometry = if class == "Mesh" {
            let first_frame =
                vertices
                    .get(..first_frame_vertices)
                    .ok_or(Error::InvalidAnimationLayout {
                        frame_vertices,
                        animation_frames,
                        vertex_count: vertices.len(),
                    })?;
            classic_triangles(first_frame, &mesh_triangles)?
        } else {
            lod_triangles(
                &mut reader,
                &vertices,
                first_frame_vertices,
                class == "SkeletalMesh",
            )?
        };
        let normals = animation_normals(
            &vertices,
            &geometry.face_vertices,
            frame_vertices,
            animation_frames,
        );

        Ok(Self {
            triangles: geometry.triangles,
            textures,
            animation_sequences,
            frame_vertices,
            animation_frames,
            scale,
            origin,
            rotation_origin,
            vertices,
            normals,
            face_vertices: geometry.face_vertices,
        })
    }
}

fn skip_primitive(reader: &mut ObjectReader<'_>) -> Result<()> {
    skip_box(reader)?;
    skip_sphere(reader)
}

fn skip_box(reader: &mut ObjectReader<'_>) -> Result<()> {
    reader.read_bytes(6 * 4 + 1)?;
    Ok(())
}

fn skip_sphere(reader: &mut ObjectReader<'_>) -> Result<()> {
    reader.read_bytes(if reader.summary().header.version > 61 {
        16
    } else {
        12
    })?;
    Ok(())
}

fn read_anim_sequences(reader: &mut ObjectReader<'_>) -> Result<Vec<MeshAnimationSequence>> {
    read_vec(reader, "animation sequences", |reader| {
        let name = read_name(reader, "animation sequence name")?;
        let group = read_name(reader, "animation sequence group")?;
        let start_frame = count(reader.read_i32()?, "animation sequence start frame")?;
        let frame_count = count(reader.read_i32()?, "animation sequence frames")?;
        let notifications = read_vec(reader, "animation notifications", |reader| {
            Ok(MeshAnimationNotify {
                time: reader.read_f32()?,
                function: read_name(reader, "animation notification function")?,
            })
        })?;
        Ok(MeshAnimationSequence {
            name,
            group,
            start_frame,
            frame_count,
            notifications,
            rate: reader.read_f32()?,
        })
    })
}

fn read_name(reader: &mut ObjectReader<'_>, field: &'static str) -> Result<String> {
    let index = reader.read_name_index(field)?;
    Ok(reader.summary().name(index).to_owned())
}

fn validate_animation(
    vertices: &[Vec3],
    frame_vertices: usize,
    animation_frames: usize,
    sequences: &[MeshAnimationSequence],
) -> Result<()> {
    if !vertices.is_empty() {
        let expected =
            frame_vertices
                .checked_mul(animation_frames)
                .ok_or(Error::InvalidAnimationLayout {
                    frame_vertices,
                    animation_frames,
                    vertex_count: vertices.len(),
                })?;
        if expected != vertices.len() {
            return Err(Error::InvalidAnimationLayout {
                frame_vertices,
                animation_frames,
                vertex_count: vertices.len(),
            });
        }
    }
    for sequence in sequences {
        let end_frame = sequence
            .start_frame
            .checked_add(sequence.frame_count)
            .ok_or_else(|| Error::InvalidAnimationSequence {
                name: sequence.name.clone(),
                start_frame: sequence.start_frame,
                end_frame: usize::MAX,
                animation_frames,
            })?;
        if !vertices.is_empty() && end_frame > animation_frames {
            return Err(Error::InvalidAnimationSequence {
                name: sequence.name.clone(),
                start_frame: sequence.start_frame,
                end_frame,
                animation_frames,
            });
        }
    }
    Ok(())
}

fn lazy_end(reader: &mut ObjectReader<'_>) -> Result<Option<u32>> {
    Ok((reader.summary().header.version > 61)
        .then(|| reader.read_u32())
        .transpose()?)
}

fn expect_lazy_end(
    reader: &ObjectReader<'_>,
    field: &'static str,
    expected: Option<u32>,
) -> Result<()> {
    if let Some(expected) = expected
        && reader.absolute_position() != expected as usize
    {
        return Err(Error::InvalidLazyArray {
            field,
            actual: reader.absolute_position(),
            expected,
        });
    }
    Ok(())
}

fn skip_lazy_vec(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    element_size: usize,
) -> Result<()> {
    let end = lazy_end(reader)?;
    let count = count(reader.read_compact_index()?, field)?;
    reader.read_bytes(count.checked_mul(element_size).ok_or(Error::InvalidCount {
        field,
        count: i32::MAX,
    })?)?;
    expect_lazy_end(reader, field, end)
}

pub(crate) fn read_vec<T>(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    mut read: impl FnMut(&mut ObjectReader<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = count(reader.read_compact_index()?, field)?;
    (0..count).map(|_| read(reader)).collect()
}

pub(crate) fn skip_vec(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    mut read: impl FnMut(&mut ObjectReader<'_>) -> Result<()>,
) -> Result<()> {
    let count = count(reader.read_compact_index()?, field)?;
    for _ in 0..count {
        read(reader)?;
    }
    Ok(())
}

pub(crate) fn count(value: i32, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        count: value,
    })
}

pub(crate) fn checked<T: Copy>(values: &[T], index: usize, field: &'static str) -> Result<T> {
    checked_ref(values, index, field).copied()
}

pub(crate) fn checked_ref<'a, T>(
    values: &'a [T],
    index: usize,
    field: &'static str,
) -> Result<&'a T> {
    values.get(index).ok_or(Error::InvalidIndex {
        field,
        index,
        length: values.len(),
    })
}

pub(crate) fn read_vec3(reader: &mut ObjectReader<'_>) -> Result<Vec3> {
    Ok(Vec3::new(
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
    ))
}

fn unpack_vertex(packed: i32) -> Vec3 {
    Vec3::new(
        (packed << 21 >> 21) as f32,
        (packed << 10 >> 21) as f32,
        (packed >> 22) as f32,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn unpacks_signed_ue1_vertex_components() {
        let packed = ((-512_i32 & 0x3ff) << 22) | ((-1024_i32 & 0x7ff) << 11) | 0x7ff;
        assert_eq!(unpack_vertex(packed), Vec3::new(-1.0, -1024.0, -512.0));
    }

    #[test]
    fn decodes_and_samples_synthetic_vertex_animation() {
        let package = synthetic_mesh_package();
        let mesh = Mesh::decode(&package, 0).unwrap();
        let sequence = &mesh.animation_sequences[0];

        assert_eq!(sequence.name, "Idle");
        assert_eq!(sequence.group, "Movement");
        assert_eq!(sequence.start_frame, 0);
        assert_eq!(sequence.frame_count, 2);
        assert_eq!(sequence.rate, 10.0);
        assert_eq!(sequence.notifications[0].time, 0.25);
        assert_eq!(sequence.notifications[0].function, "Step");
        assert_eq!(mesh.frame_vertices, 3);
        assert_eq!(mesh.animation_frames, 2);

        let halfway = mesh.sample_sequence(sequence, 0.25).unwrap();
        assert_eq!(halfway[0].vertices[0].position, Vec3::Z * 0.5);
        let wrapped = mesh.sample_sequence(sequence, 1.25).unwrap();
        assert_eq!(
            wrapped[0].vertices[0].position,
            halfway[0].vertices[0].position
        );
        assert!(matches!(
            mesh.sample_sequence(sequence, f32::NAN),
            Err(Error::InvalidAnimationPhase(_))
        ));
    }

    fn synthetic_mesh_package() -> Package {
        let names = [
            "None", "Core", "Class", "Mesh", "TestMesh", "Idle", "Movement", "Step",
        ];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            push_u32(&mut name_table, 0);
        }

        let mut import_table = vec![1, 2];
        push_i32(&mut import_table, 0);
        import_table.push(3);

        let mut payload = vec![0];
        payload.extend([0; 25 + 12]);
        payload.push(6);
        for (x, y, z) in [
            (0, 0, 0),
            (1, 0, 0),
            (0, 1, 0),
            (0, 0, 1),
            (1, 0, 1),
            (0, 1, 1),
        ] {
            push_i32(&mut payload, pack_vertex(x, y, z));
        }
        payload.push(1);
        for index in [0_u16, 1, 2] {
            payload.extend(index.to_le_bytes());
        }
        payload.extend([0; 6]);
        push_u32(&mut payload, 0);
        push_i32(&mut payload, 0);
        payload.extend([1, 5, 6]);
        push_i32(&mut payload, 0);
        push_i32(&mut payload, 2);
        payload.push(1);
        push_f32(&mut payload, 0.25);
        payload.push(7);
        push_f32(&mut payload, 10.0);
        payload.push(0);
        payload.extend([0; 25 + 12]);
        payload.extend([0; 4]);
        push_i32(&mut payload, 3);
        push_i32(&mut payload, 2);
        payload.extend([0; 8]);
        for value in [1.0, 1.0, 1.0, 0.0, 0.0, 0.0] {
            push_f32(&mut payload, value);
        }
        payload.extend([0; 20]);

        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export_prefix = vec![0x81, 0];
        push_i32(&mut export_prefix, 0);
        export_prefix.push(4);
        push_u32(&mut export_prefix, 0);
        export_prefix.extend(compact_index(payload.len() as i32));
        let mut payload_offset = export_offset + export_prefix.len() + 1;
        loop {
            let encoded = compact_index(payload_offset as i32);
            let next = export_offset + export_prefix.len() + encoded.len();
            if next == payload_offset {
                export_prefix.extend(encoded);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        push_u32(&mut bytes, openhp1_package::PACKAGE_MAGIC);
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        push_u32(&mut bytes, 0);
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            push_i32(&mut bytes, value as i32);
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export_prefix);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        Package::parse("synthetic mesh", Arc::from(bytes)).unwrap()
    }

    fn compact_index(value: i32) -> Vec<u8> {
        let negative = value < 0;
        let mut value = value.unsigned_abs();
        let mut bytes = vec![(value as u8 & 0x3f) | if negative { 0x80 } else { 0 }];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = value as u8 & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }

    fn pack_vertex(x: i32, y: i32, z: i32) -> i32 {
        ((z & 0x3ff) << 22) | ((y & 0x7ff) << 11) | (x & 0x7ff)
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend(value.to_le_bytes());
    }
}
