use glam::{IVec3, Vec3};
use openhp1_package::{ObjectReader, Package};

use crate::{
    Error, Mesh, Result,
    geometry::{classic_triangles, lod_triangles},
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

        skip_anim_sequences(&mut reader)?;
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

        let frame_vertices = reader.read_i32()?;
        reader.read_i32()?; // animation frame count
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

        let triangles = if class == "Mesh" {
            classic_triangles(&vertices, &mesh_triangles)?
        } else {
            lod_triangles(
                &mut reader,
                &vertices,
                frame_vertices,
                class == "SkeletalMesh",
            )?
        };

        Ok(Self {
            triangles,
            textures,
            scale,
            origin,
            rotation_origin,
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

fn skip_anim_sequences(reader: &mut ObjectReader<'_>) -> Result<()> {
    skip_vec(reader, "animation sequences", |reader| {
        reader.read_compact_index()?; // name
        reader.read_compact_index()?; // group
        reader.read_i32()?;
        reader.read_i32()?;
        skip_vec(reader, "animation notifications", |reader| {
            reader.read_f32()?;
            reader.read_compact_index()?;
            Ok(())
        })?;
        reader.read_f32()?;
        Ok(())
    })
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
    use super::*;

    #[test]
    fn unpacks_signed_ue1_vertex_components() {
        let packed = ((-512_i32 & 0x3ff) << 22) | ((-1024_i32 & 0x7ff) << 11) | 0x7ff;
        assert_eq!(unpack_vertex(packed), Vec3::new(-1.0, -1024.0, -512.0));
    }
}
