use glam::Vec3;

use crate::{
    Error, Model, Result,
    decode::{index, point_index},
};

/// Indexed triangles plus the BSP surface responsible for each triangle.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
    pub triangle_surfaces: Vec<usize>,
}

impl Model {
    pub fn triangulate(&self) -> Result<TriangleMesh> {
        let positions = self.points.clone();
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
            let first = point_index(polygon[0].point, self.points.len())?;
            for pair in polygon[1..].windows(2) {
                indices.extend_from_slice(&[
                    first,
                    point_index(pair[0].point, self.points.len())?,
                    point_index(pair[1].point, self.points.len())?,
                ]);
                triangle_surfaces.push(surface);
            }
        }
        Ok(TriangleMesh {
            positions,
            indices,
            triangle_surfaces,
        })
    }
}
