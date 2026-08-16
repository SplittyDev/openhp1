use glam::{Vec2, Vec3};

use crate::{BspNode, Error, Model, Result, decode::index};

/// Indexed triangles plus the BSP surface responsible for each triangle.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
    /// Compact BSP topology retained for camera-dependent submission order.
    pub bsp_nodes: Vec<BspNode>,
    pub positions: Vec<Vec3>,
    /// World-space normals. Environment-mapped actor meshes use these to
    /// derive camera-relative reflection coordinates.
    pub normals: Vec<Vec3>,
    /// BSP node-plane normals used for camera-side zone selection.
    pub node_plane_normals: Vec<Vec3>,
    /// Raw UE texture coordinates in texels. Divide by the selected texture's
    /// dimensions before sampling a normalized GPU texture.
    pub texture_coordinates: Vec<Vec2>,
    /// Raw lightmap texel coordinates, paired with `vertex_lightmaps`.
    pub lightmap_coordinates: Vec<Vec2>,
    pub vertex_lightmaps: Vec<Option<usize>>,
    /// Linear RGB modulation. BSP vertices default to white; actor meshes use
    /// UE1's Gouraud vertex lighting.
    pub vertex_colors: Vec<Vec3>,
    pub vertex_surfaces: Vec<usize>,
    /// Automatic pan speeds for BSP node zones 0 and 1.
    pub texture_pan_speeds: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub triangle_surfaces: Vec<usize>,
    /// BSP node responsible for each triangulated world polygon. Actor
    /// triangles appended by scene assembly are intentionally not represented.
    pub triangle_nodes: Vec<usize>,
}

impl Model {
    pub fn triangulate(&self) -> Result<TriangleMesh> {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut node_plane_normals = Vec::new();
        let mut texture_coordinates = Vec::new();
        let mut lightmap_coordinates = Vec::new();
        let mut vertex_lightmaps = Vec::new();
        let mut vertex_colors = Vec::new();
        let mut vertex_surfaces = Vec::new();
        let mut indices = Vec::new();
        let mut triangle_surfaces = Vec::new();
        let mut triangle_nodes = Vec::new();

        for (node_index, node) in self.nodes.iter().enumerate() {
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
            let normal = self.vectors
                [index(surface_data.normal, self.vectors.len(), "surface normal")?]
            .normalize_or_zero();
            let node_plane_normal = Vec3::from_array([node.plane[0], node.plane[1], node.plane[2]]);
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
            let lightmap_index = usize::try_from(surface_data.light_map)
                .ok()
                .filter(|lightmap| *lightmap < self.light_maps.len());
            if surface_data.light_map >= 0 && lightmap_index.is_none() {
                return Err(Error::InvalidIndex {
                    field: "surface lightmap",
                    value: surface_data.light_map,
                    length: self.light_maps.len(),
                });
            }
            for vertex in polygon {
                let point =
                    self.points[index(vertex.point, self.points.len(), "BSP vertex point")?];
                positions.push(point);
                normals.push(normal);
                node_plane_normals.push(node_plane_normal);
                texture_coordinates.push(surface_texture_coordinates(
                    point,
                    base,
                    texture_u,
                    texture_v,
                    surface_data.pan_u,
                    surface_data.pan_v,
                ));
                lightmap_coordinates.push(match lightmap_index {
                    Some(lightmap) => surface_lightmap_coordinates(
                        point,
                        base,
                        texture_u,
                        texture_v,
                        &self.light_maps[lightmap],
                        lightmap,
                    )?,
                    None => Vec2::ZERO,
                });
                vertex_lightmaps.push(lightmap_index);
                vertex_colors.push(Vec3::ONE);
                vertex_surfaces.push(surface);
            }
            for offset in 1..u32::from(node.vertex_count) - 1 {
                indices.extend_from_slice(&[
                    base_vertex,
                    base_vertex + offset,
                    base_vertex + offset + 1,
                ]);
                triangle_surfaces.push(surface);
                triangle_nodes.push(node_index);
            }
        }
        Ok(TriangleMesh {
            bsp_nodes: self.nodes.clone(),
            positions,
            normals,
            node_plane_normals,
            texture_coordinates,
            lightmap_coordinates,
            vertex_lightmaps,
            vertex_colors,
            vertex_surfaces,
            texture_pan_speeds: Vec::new(),
            indices,
            triangle_surfaces,
            triangle_nodes,
        })
    }
}

fn surface_lightmap_coordinates(
    point: Vec3,
    base: Vec3,
    texture_u: Vec3,
    texture_v: Vec3,
    lightmap: &crate::LightMap,
    lightmap_index: usize,
) -> Result<Vec2> {
    if !lightmap.scale[0].is_finite()
        || !lightmap.scale[1].is_finite()
        || lightmap.scale[0] <= 0.0
        || lightmap.scale[1] <= 0.0
    {
        return Err(Error::InvalidLightmapScale {
            index: lightmap_index,
            u: lightmap.scale[0],
            v: lightmap.scale[1],
        });
    }
    let pan_u = texture_u.dot(base) + lightmap.pan.x - 0.5 * lightmap.scale[0];
    let pan_v = texture_v.dot(base) + lightmap.pan.y - 0.5 * lightmap.scale[1];
    Ok(Vec2::new(
        (texture_u.dot(point) - pan_u) / lightmap.scale[0],
        (texture_v.dot(point) - pan_v) / lightmap.scale[1],
    ))
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

    use crate::LightMap;

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

    #[test]
    fn lightmap_coordinates_include_half_texel_pan_convention() {
        assert_eq!(
            super::surface_lightmap_coordinates(
                Vec3::new(12.0, 24.0, 0.0),
                Vec3::new(10.0, 20.0, 0.0),
                Vec3::X,
                Vec3::Y,
                &LightMap {
                    data_offset: 0,
                    pan: Vec3::new(2.0, 4.0, 0.0),
                    clamp: [1, 1],
                    scale: [4.0, 8.0],
                    light_actors: -1,
                },
                0,
            )
            .unwrap(),
            Vec2::splat(0.5)
        );
    }
}
