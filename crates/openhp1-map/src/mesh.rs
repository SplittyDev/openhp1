use glam::{Vec2, Vec3};

use crate::{Error, Model, Result, decode::index};

/// Indexed triangles plus the BSP surface responsible for each triangle.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
    pub positions: Vec<Vec3>,
    /// Raw UE texture coordinates in texels. Divide by the selected texture's
    /// dimensions before sampling a normalized GPU texture.
    pub texture_coordinates: Vec<Vec2>,
    pub vertex_surfaces: Vec<usize>,
    pub indices: Vec<u32>,
    pub triangle_surfaces: Vec<usize>,
}

impl Model {
    pub fn triangulate(&self) -> Result<TriangleMesh> {
        let mut positions = Vec::new();
        let mut texture_coordinates = Vec::new();
        let mut vertex_surfaces = Vec::new();
        let mut indices = Vec::new();
        let mut triangle_surfaces = Vec::new();

        for node in &self.nodes {
            if node.vertex_count < 3 {
                continue;
            }
            let surface = index(node.surface, self.surfaces.len(), "node surface")?;
            let first_pool = index(node.vertex_pool, self.vertices.len(), "node vertex pool")?;
            let end_pool = first_pool
                .checked_add(usize::from(node.vertex_count))
                .filter(|end| *end <= self.vertices.len())
                .ok_or(Error::InvalidVertexPool {
                    start: first_pool,
                    count: node.vertex_count,
                    pool_len: self.vertices.len(),
                })?;
            let polygon = &self.vertices[first_pool..end_pool];
            let base_vertex = u32::try_from(positions.len()).map_err(|_| Error::MeshTooLarge {
                point_count: positions.len(),
            })?;
            let surface_data = &self.surfaces[surface];
            let base = self.points[index(
                surface_data.base_point,
                self.points.len(),
                "surface base point",
            )?];
            let texture_u = self.vectors[index(
                surface_data.texture_u,
                self.vectors.len(),
                "surface texture U",
            )?];
            let texture_v = self.vectors[index(
                surface_data.texture_v,
                self.vectors.len(),
                "surface texture V",
            )?];
            for vertex in polygon {
                let point =
                    self.points[index(vertex.point, self.points.len(), "BSP vertex point")?];
                positions.push(point);
                texture_coordinates.push(surface_texture_coordinates(
                    point,
                    base,
                    texture_u,
                    texture_v,
                    surface_data.pan_u,
                    surface_data.pan_v,
                ));
                vertex_surfaces.push(surface);
            }
            for offset in 1..u32::from(node.vertex_count) - 1 {
                indices.extend_from_slice(&[
                    base_vertex,
                    base_vertex + offset,
                    base_vertex + offset + 1,
                ]);
                triangle_surfaces.push(surface);
            }
        }
        Ok(TriangleMesh {
            positions,
            texture_coordinates,
            vertex_surfaces,
            indices,
            triangle_surfaces,
        })
    }
}

fn surface_texture_coordinates(
    point: Vec3,
    base: Vec3,
    texture_u: Vec3,
    texture_v: Vec3,
    pan_u: i16,
    pan_v: i16,
) -> Vec2 {
    let relative = point - base;
    Vec2::new(
        texture_u.dot(relative) + f32::from(pan_u),
        texture_v.dot(relative) + f32::from(pan_v),
    )
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    #[test]
    fn texture_coordinates_are_relative_to_base_and_include_pan() {
        assert_eq!(
            super::surface_texture_coordinates(
                Vec3::new(12.0, 24.0, 8.0),
                Vec3::new(10.0, 20.0, 8.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(0.0, 0.5, 0.0),
                3,
                -1,
            ),
            Vec2::new(7.0, 1.0)
        );
    }
}
