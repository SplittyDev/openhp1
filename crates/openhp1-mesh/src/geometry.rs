use glam::{Vec2, Vec3};
use openhp1_package::ObjectReader;

use crate::{
    Error, MeshTriangle, MeshVertex, Result,
    decode::{checked, checked_ref, count, read_vec, read_vec3, skip_vec},
};

pub(crate) type SerializedTriangle = ([u16; 3], [[u8; 2]; 3], u32, i32);

pub(crate) fn classic_triangles(
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

pub(crate) fn lod_triangles(
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

#[cfg(test)]
mod tests {
    use super::*;

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
