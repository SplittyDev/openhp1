//! Decoding of the vertex-mesh assets used by Harry Potter 1.

use glam::{IVec3, Vec2, Vec3};
use openhp1_package::{ObjectReader, ObjectReference, Package};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),
    #[error("{field} has invalid count {count}")]
    InvalidCount { field: &'static str, count: i32 },
    #[error("{field} lazy array ended at {actual:#x}, expected {expected:#x}")]
    InvalidLazyArray {
        field: &'static str,
        actual: usize,
        expected: u32,
    },
    #[error("{field} index {index} is outside 0..{length}")]
    InvalidIndex {
        field: &'static str,
        index: usize,
        length: usize,
    },
    #[error("unsupported mesh class {0}")]
    UnsupportedClass(String),
}

pub type Result<T> = std::result::Result<T, Error>;

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
pub struct Mesh {
    pub triangles: Vec<MeshTriangle>,
    pub textures: Vec<ObjectReference>,
    pub scale: Vec3,
    pub origin: Vec3,
    pub rotation_origin: IVec3,
}

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

type SerializedTriangle = ([u16; 3], [[u8; 2]; 3], u32, i32);

fn classic_triangles(
    vertices: &[Vec3],
    triangles: &[SerializedTriangle],
) -> Result<Vec<MeshTriangle>> {
    let mut faces = Vec::with_capacity(triangles.len());
    for (indices, _, _, _) in triangles {
        let face = indices.map(usize::from);
        for index in face {
            checked(vertices, index, "mesh vertex")?;
        }
        faces.push(face);
    }
    let normals = vertex_normals(vertices, &faces);
    triangles
        .iter()
        .zip(faces)
        .map(|(&(indices, uv, poly_flags, texture_index), face)| {
            let mut corners = [MeshVertex {
                position: Vec3::ZERO,
                normal: Vec3::ZERO,
                texture_coordinates: Vec2::ZERO,
            }; 3];
            for corner in 0..3 {
                corners[corner] = MeshVertex {
                    position: checked(vertices, usize::from(indices[corner]), "mesh vertex")?,
                    normal: normals[face[corner]],
                    texture_coordinates: Vec2::new(
                        f32::from(uv[corner][0]) / 255.0,
                        f32::from(uv[corner][1]) / 255.0,
                    ),
                };
            }
            Ok(MeshTriangle {
                vertices: corners,
                poly_flags,
                texture_index,
            })
        })
        .collect()
}

fn lod_triangles(
    reader: &mut ObjectReader<'_>,
    vertices: &[Vec3],
    frame_vertices: i32,
    skeletal: bool,
) -> Result<Vec<MeshTriangle>> {
    skip_vec(reader, "collapse points", |reader| {
        reader.read_u16()?;
        Ok(())
    })?;
    skip_vec(reader, "face levels", |reader| {
        reader.read_u16()?;
        Ok(())
    })?;
    let faces = read_vec(reader, "LOD faces", |reader| {
        Ok((
            [reader.read_u16()?, reader.read_u16()?, reader.read_u16()?],
            reader.read_u16()?,
        ))
    })?;
    skip_vec(reader, "collapse wedges", |reader| {
        reader.read_u16()?;
        Ok(())
    })?;
    let wedges = read_vec(reader, "LOD wedges", |reader| {
        Ok((reader.read_u16()?, [reader.read_u8()?, reader.read_u8()?]))
    })?;
    let materials = read_vec(reader, "LOD materials", |reader| {
        Ok((reader.read_u32()?, reader.read_i32()?))
    })?;
    skip_vec(reader, "LOD special faces", |reader| {
        reader.read_bytes(8)?;
        Ok(())
    })?;
    reader.read_u32()?; // model vertices
    let special_vertices = reader.read_u32()? as usize;
    reader.read_f32()?; // maximum mesh scale
    reader.read_f32()?; // LOD hysteresis
    reader.read_f32()?; // LOD strength
    reader.read_u32()?; // minimum LOD vertices
    reader.read_f32()?; // LOD morph
    reader.read_f32()?; // LOD Z displacement
    let remap = read_vec(reader, "animation vertex remap", |reader| {
        Ok(reader.read_u16()?)
    })?;
    reader.read_u32()?; // old frame vertices

    let skeletal_points;
    let vertices = if skeletal && vertices.is_empty() {
        skip_vec(reader, "extended mesh wedges", |reader| {
            reader.read_bytes(12)?;
            Ok(())
        })?;
        skeletal_points = read_vec(reader, "skeletal mesh points", read_vec3)?;
        &skeletal_points
    } else {
        vertices
    };
    let frame_vertices = count(frame_vertices, "frame vertices")?;
    let frame_vertices = if frame_vertices == 0 {
        vertices.len()
    } else {
        frame_vertices.min(vertices.len())
    };
    let face_vertices = faces
        .iter()
        .map(|(indices, _)| {
            let mut vertices = [0; 3];
            for corner in 0..3 {
                let &(vertex, _) = checked_ref(&wedges, usize::from(indices[corner]), "LOD wedge")?;
                let base = usize::from(vertex) + special_vertices;
                let vertex = if remap.is_empty() {
                    base
                } else {
                    usize::from(checked(&remap, base, "animation vertex remap")?)
                };
                if vertex >= frame_vertices {
                    return Err(Error::InvalidIndex {
                        field: "LOD vertex",
                        index: vertex,
                        length: frame_vertices,
                    });
                }
                vertices[corner] = vertex;
            }
            Ok(vertices)
        })
        .collect::<Result<Vec<_>>>()?;
    let normals = vertex_normals(vertices, &face_vertices);
    let mut triangles = Vec::with_capacity(faces.len());
    for ((indices, material_index), face_vertices) in faces.into_iter().zip(face_vertices) {
        let &(poly_flags, texture_index) =
            checked_ref(&materials, usize::from(material_index), "LOD material")?;
        let mut corners = [MeshVertex {
            position: Vec3::ZERO,
            normal: Vec3::ZERO,
            texture_coordinates: Vec2::ZERO,
        }; 3];
        for corner in 0..3 {
            let &(_, uv) = checked_ref(&wedges, usize::from(indices[corner]), "LOD wedge")?;
            let vertex = face_vertices[corner];
            corners[corner] = MeshVertex {
                position: vertices[vertex],
                normal: normals[vertex],
                texture_coordinates: Vec2::new(f32::from(uv[0]) / 255.0, f32::from(uv[1]) / 255.0),
            };
        }
        triangles.push(MeshTriangle {
            vertices: corners,
            poly_flags,
            texture_index,
        });
    }
    Ok(triangles)
}

fn vertex_normals(vertices: &[Vec3], faces: &[[usize; 3]]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; vertices.len()];
    for &[a, b, c] in faces {
        let normal = (vertices[b] - vertices[a])
            .cross(vertices[c] - vertices[a])
            .normalize_or_zero();
        normals[a] += normal;
        normals[b] += normal;
        normals[c] += normal;
    }
    normals.into_iter().map(Vec3::normalize_or_zero).collect()
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

fn read_vec<T>(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    mut read: impl FnMut(&mut ObjectReader<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = count(reader.read_compact_index()?, field)?;
    (0..count).map(|_| read(reader)).collect()
}

fn skip_vec(
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

fn count(value: i32, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        count: value,
    })
}

fn checked<T: Copy>(values: &[T], index: usize, field: &'static str) -> Result<T> {
    checked_ref(values, index, field).copied()
}

fn checked_ref<'a, T>(values: &'a [T], index: usize, field: &'static str) -> Result<&'a T> {
    values.get(index).ok_or(Error::InvalidIndex {
        field,
        index,
        length: values.len(),
    })
}

fn read_vec3(reader: &mut ObjectReader<'_>) -> Result<Vec3> {
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

    #[test]
    fn averages_normals_across_shared_mesh_vertices() {
        let vertices = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        let normals = vertex_normals(&vertices, &[[0, 1, 2], [0, 3, 1]]);
        assert_eq!(normals[0], Vec3::new(0.0, 1.0, 1.0).normalize());
        assert_eq!(normals[1], normals[0]);
        assert_eq!(normals[2], Vec3::Z);
        assert_eq!(normals[3], Vec3::Y);
    }
}
