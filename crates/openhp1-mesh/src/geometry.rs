use glam::{Mat4, Quat, Vec2, Vec3};
use openhp1_package::{ObjectReader, ObjectReference};

use crate::{
    Error, MeshTriangle, MeshVertex, Result,
    decode::{checked, checked_ref, read_vec, read_vec3, skip_vec},
    types::{SkeletalBone, SkeletalInfluence, SkeletalMesh},
};

pub(crate) type SerializedTriangle = ([u16; 3], [[u8; 2]; 3], u32, i32);

pub(crate) struct DecodedGeometry {
    pub triangles: Vec<MeshTriangle>,
    pub face_vertices: Vec<[usize; 3]>,
    pub attachment_vertices: Option<[usize; 3]>,
    pub skeletal: Option<DecodedSkeletalMesh>,
}

pub(crate) struct DecodedSkeletalMesh {
    pub default_animation: ObjectReference,
    pub mesh: SkeletalMesh,
}

pub(crate) fn classic_triangles(
    vertices: &[Vec3],
    triangles: &[SerializedTriangle],
) -> Result<DecodedGeometry> {
    let mut faces = Vec::with_capacity(triangles.len());
    for (indices, _, _, _) in triangles {
        let face = indices.map(usize::from);
        for index in face {
            checked(vertices, index, "mesh vertex")?;
        }
        faces.push(face);
    }
    let normals = vertex_normals(vertices, &faces);
    let decoded = triangles
        .iter()
        .zip(&faces)
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
        .collect::<Result<Vec<_>>>()?;
    Ok(DecodedGeometry {
        triangles: decoded,
        face_vertices: faces,
        attachment_vertices: None,
        skeletal: None,
    })
}

pub(crate) fn lod_triangles(
    reader: &mut ObjectReader<'_>,
    vertices: &[Vec3],
    frame_vertices: usize,
    skeletal: bool,
) -> Result<DecodedGeometry> {
    skip_vec(reader, "collapse points", |reader| {
        reader.read_u16()?;
        Ok(())
    })?;
    skip_vec(reader, "face levels", |reader| {
        reader.read_u16()?;
        Ok(())
    })?;
    let mut faces = read_vec(reader, "LOD faces", |reader| {
        Ok((
            [reader.read_u16()?, reader.read_u16()?, reader.read_u16()?],
            reader.read_u16()?,
        ))
    })?;
    if skeletal {
        for (indices, _) in &mut faces {
            mirror_skeletal_face(indices);
        }
    }
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
    let special_faces = read_vec(reader, "LOD special faces", |reader| {
        Ok([
            reader.read_u16()?,
            reader.read_u16()?,
            reader.read_u16()?,
            reader.read_u16()?,
        ])
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

    let skeletal = skeletal.then(|| decode_skeletal_mesh(reader)).transpose()?;
    let skeletal_points = skeletal.as_ref().map(|skeletal| {
        skeletal
            .mesh
            .points
            .iter()
            .copied()
            .map(mirror_skeletal_position)
            .collect::<Vec<_>>()
    });
    let vertices = skeletal_points.as_deref().unwrap_or(vertices);
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
    let attachment_vertices = if let Some(face) = special_faces.first() {
        let remap_vertex = |vertex| -> Result<usize> {
            let vertex = usize::from(vertex);
            if remap.is_empty() {
                Ok(vertex)
            } else {
                Ok(usize::from(checked(
                    &remap,
                    vertex,
                    "attachment animation vertex remap",
                )?))
            }
        };
        Some([
            remap_vertex(face[0])?,
            remap_vertex(face[1])?,
            remap_vertex(face[2])?,
        ])
    } else {
        None
    };
    if let Some(indices) = attachment_vertices {
        for index in indices {
            checked(vertices, index, "mesh attachment vertex")?;
        }
    }
    let normals = vertex_normals(vertices, &face_vertices);
    let mut triangles = Vec::with_capacity(faces.len());
    for ((indices, material_index), face_vertices) in faces.into_iter().zip(&face_vertices) {
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
    Ok(DecodedGeometry {
        triangles,
        face_vertices,
        attachment_vertices,
        skeletal,
    })
}

fn decode_skeletal_mesh(reader: &mut ObjectReader<'_>) -> Result<DecodedSkeletalMesh> {
    skip_vec(reader, "extended mesh wedges", |reader| {
        reader.read_u16()?;
        reader.read_u16()?;
        reader.read_f32()?;
        reader.read_f32()?;
        Ok(())
    })?;
    let points = read_vec(reader, "skeletal mesh points", read_vec3)?;
    let bones = read_vec(reader, "reference skeleton", |reader| {
        let name = crate::decode::read_name(reader, "reference skeleton bone")?;
        reader.read_u32()?; // flags
        let orientation = Quat::from_xyzw(
            reader.read_f32()?,
            reader.read_f32()?,
            reader.read_f32()?,
            reader.read_f32()?,
        );
        let position = read_vec3(reader)?;
        reader.read_f32()?; // length
        read_vec3(reader)?; // size
        reader.read_u32()?; // children
        let parent = usize::try_from(reader.read_u32()?).map_err(|_| Error::InvalidIndex {
            field: "reference skeleton parent",
            index: usize::MAX,
            length: 0,
        })?;
        if !orientation.is_finite() || orientation.length_squared() <= f32::EPSILON {
            return Err(Error::InvalidFloat {
                field: "reference skeleton orientation",
                value: orientation.length_squared(),
            });
        }
        if !position.is_finite() {
            return Err(Error::InvalidFloat {
                field: "reference skeleton position",
                value: f32::NAN,
            });
        }
        Ok(SkeletalBone {
            name,
            orientation: orientation.normalize(),
            position,
            parent,
        })
    })?;
    for (index, bone) in bones.iter().enumerate() {
        if bone.parent > index || (index == 0 && bone.parent != 0) {
            return Err(Error::InvalidIndex {
                field: "reference skeleton parent",
                index: bone.parent,
                length: index + 1,
            });
        }
    }
    let weight_indices = read_vec(reader, "bone weight indices", |reader| {
        let start = usize::from(reader.read_u16()?);
        let count = usize::from(reader.read_u16()?);
        reader.read_u16()?;
        reader.read_u16()?;
        Ok((start, count))
    })?;
    let weights = read_vec(reader, "bone weights", |reader| {
        Ok((
            usize::from(reader.read_u16()?),
            f32::from(reader.read_u16()?) / 65_535.0,
        ))
    })?;
    let local_points = read_vec(reader, "skeletal mesh local points", read_vec3)?;
    reader.read_u32()?; // skeletal depth
    let default_animation = reader.read_object_reference()?;
    let weapon_bone = reader.read_u32()?;
    let weapon_bone = (weapon_bone != u32::MAX).then_some(weapon_bone as usize);
    if weapon_bone.is_some_and(|weapon_bone| weapon_bone >= bones.len()) {
        return Err(Error::InvalidIndex {
            field: "weapon bone",
            index: weapon_bone.unwrap(),
            length: bones.len(),
        });
    }
    let weapon_origin = read_vec3(reader)?;
    let weapon_x = read_vec3(reader)?;
    let weapon_y = read_vec3(reader)?;
    let weapon_z = read_vec3(reader)?;
    if ![weapon_origin, weapon_x, weapon_y, weapon_z]
        .into_iter()
        .all(Vec3::is_finite)
    {
        return Err(Error::InvalidFloat {
            field: "weapon adjustment",
            value: f32::NAN,
        });
    }
    let weapon_adjust = Mat4::from_cols(
        weapon_x.extend(0.0),
        weapon_y.extend(0.0),
        weapon_z.extend(0.0),
        (-weapon_origin).extend(1.0),
    );

    if weight_indices.len() != bones.len() || weights.len() != local_points.len() {
        return Err(Error::InvalidSkeletalWeights {
            bones: bones.len(),
            weight_indices: weight_indices.len(),
            weights: weights.len(),
            local_points: local_points.len(),
        });
    }
    let mut influences = vec![Vec::new(); points.len()];
    for (bone, &(start, count)) in weight_indices.iter().enumerate() {
        let end = start.checked_add(count).ok_or(Error::InvalidIndex {
            field: "bone weight",
            index: usize::MAX,
            length: weights.len(),
        })?;
        let selected = weights.get(start..end).ok_or(Error::InvalidIndex {
            field: "bone weight",
            index: end,
            length: weights.len(),
        })?;
        for (offset, &(point, weight)) in selected.iter().enumerate() {
            let local_position = checked(&local_points, start + offset, "skeletal local point")?;
            let point_influences = influences.get_mut(point).ok_or(Error::InvalidIndex {
                field: "skeletal point",
                index: point,
                length: points.len(),
            })?;
            point_influences.push(SkeletalInfluence {
                bone,
                weight,
                local_position,
            });
        }
    }
    Ok(DecodedSkeletalMesh {
        default_animation,
        mesh: SkeletalMesh {
            points,
            bones,
            influences,
            weapon_bone,
            weapon_adjust,
        },
    })
}

pub(crate) fn mirror_skeletal_position(mut position: Vec3) -> Vec3 {
    position.y = -position.y;
    position
}

fn mirror_skeletal_face(face: &mut [u16; 3]) {
    face.swap(0, 1);
}

pub(crate) fn sample_triangles(
    triangles: &[MeshTriangle],
    faces: &[[usize; 3]],
    vertices: &[Vec3],
    normals: &[Vec3],
) -> Result<Vec<MeshTriangle>> {
    if triangles.len() != faces.len() {
        return Err(Error::InvalidIndex {
            field: "mesh animation face",
            index: faces.len(),
            length: triangles.len(),
        });
    }
    triangles
        .iter()
        .zip(faces)
        .map(|(triangle, face)| {
            let mut triangle = *triangle;
            for (corner, &vertex) in face.iter().enumerate() {
                triangle.vertices[corner].position =
                    checked(vertices, vertex, "mesh animation vertex")?;
                triangle.vertices[corner].normal =
                    checked(normals, vertex, "mesh animation normal")?;
            }
            Ok(triangle)
        })
        .collect()
}

pub(crate) fn vertex_normals(vertices: &[Vec3], faces: &[[usize; 3]]) -> Vec<Vec3> {
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

pub(crate) fn animation_normals(
    vertices: &[Vec3],
    faces: &[[usize; 3]],
    frame_vertices: usize,
    animation_frames: usize,
) -> Vec<Vec3> {
    if frame_vertices == 0 || animation_frames == 0 {
        return Vec::new();
    }
    vertices
        .chunks_exact(frame_vertices)
        .take(animation_frames)
        .flat_map(|frame| vertex_normals(frame, faces))
        .collect()
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

    #[test]
    fn mirrors_ue1_skeletal_positions_and_winding() {
        assert_eq!(
            mirror_skeletal_position(Vec3::new(1.0, 2.0, 3.0)),
            Vec3::new(1.0, -2.0, 3.0)
        );
        let mut face = [1, 2, 3];
        mirror_skeletal_face(&mut face);
        assert_eq!(face, [2, 1, 3]);
    }

    #[test]
    fn applies_sampled_positions_and_normals() {
        let triangles = [MeshTriangle {
            vertices: [MeshVertex {
                position: Vec3::ZERO,
                normal: Vec3::Z,
                texture_coordinates: Vec2::ZERO,
            }; 3],
            poly_flags: 7,
            texture_index: 2,
        }];
        let sampled = sample_triangles(
            &triangles,
            &[[0, 1, 2]],
            &[Vec3::ZERO, Vec3::X, Vec3::Z],
            &[-Vec3::Y; 3],
        )
        .unwrap();
        assert_eq!(sampled[0].vertices[2].position, Vec3::Z);
        assert_eq!(sampled[0].vertices[0].normal, -Vec3::Y);
        assert_eq!(sampled[0].poly_flags, 7);
        assert_eq!(sampled[0].texture_index, 2);
    }
}
