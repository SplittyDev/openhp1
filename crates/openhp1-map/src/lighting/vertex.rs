use std::collections::HashSet;

use glam::Vec3;
use openhp1_package::{ObjectReference, Package};

use crate::{Model, Result};

use super::{decode_ambient, decode_level_ambient, decode_light, hsb_to_rgb};

#[derive(Clone, Copy, Debug)]
struct VertexLight {
    location: Vec3,
    color: Vec3,
    radius: f32,
    effect: u8,
}

#[derive(Clone, Debug)]
pub struct VertexLighting {
    level_ambient: Vec3,
    zone_ambient: Vec<Vec3>,
    lights: Vec<VertexLight>,
}

#[derive(Clone, Debug)]
pub struct ActorVertexLighting {
    ambient: Vec3,
    scale_glow: f32,
    lights: Vec<VertexLight>,
}

impl ActorVertexLighting {
    pub fn color(&self, location: Vec3, normal: Vec3, unlit: bool) -> Vec3 {
        if unlit {
            return (self.ambient + self.scale_glow * 0.5) * 2.0;
        }
        let mut color = Vec3::ZERO;
        for light in &self.lights {
            let direction = light.location - location;
            let distance = direction.length();
            if distance == 0.0 || distance >= light.radius {
                continue;
            }
            let attenuation = 1.0 - distance / light.radius;
            let angle_attenuation = (direction / distance).dot(normal).abs();
            color += light.color * (attenuation * angle_attenuation);
        }
        (self.ambient + color * (self.scale_glow * 1.5)) * 2.0
    }
}

impl Model {
    pub fn vertex_lighting(&self, package: &Package) -> Result<VertexLighting> {
        let level_ambient = decode_level_ambient(package)?;
        let level_ambient = hsb_to_rgb(
            level_ambient.hue,
            level_ambient.saturation,
            level_ambient.brightness,
        );
        let mut zone_ambient = Vec::with_capacity(self.zones.len());
        for zone in &self.zones {
            zone_ambient.push(match zone.actor {
                ObjectReference::Export(export_index) => {
                    let ambient = decode_ambient(package, export_index)?;
                    hsb_to_rgb(ambient.hue, ambient.saturation, ambient.brightness)
                }
                _ => level_ambient,
            });
        }

        let mut seen = HashSet::new();
        let mut lights = Vec::new();
        for reference in &self.lights {
            let ObjectReference::Export(export_index) = *reference else {
                continue;
            };
            if !seen.insert(export_index) {
                continue;
            }
            let light = decode_light(package, export_index)?;
            if light.light_type == 0 || light.brightness == 0 || light.corona || light.special_lit {
                continue;
            }
            lights.push(VertexLight {
                location: light.location,
                color: hsb_to_rgb(light.hue, light.saturation, light.brightness),
                radius: (f32::from(light.radius) + 1.0) * 25.0,
                effect: light.effect,
            });
        }
        Ok(VertexLighting {
            level_ambient,
            zone_ambient,
            lights,
        })
    }

    pub fn zone_at(&self, point: Vec3) -> usize {
        bsp_zone_at(&self.nodes, self.zones.len(), point)
    }

    fn blocks_light(&self, from: Vec3, to: Vec3) -> bool {
        let offset = to - from;
        let distance = offset.length();
        if self.nodes.is_empty() || distance < 0.01 {
            return false;
        }
        self.trace_light_node(0, from, offset / distance, 0.01, distance + 1.0)
    }

    fn trace_light_node(
        &self,
        node_index: usize,
        origin: Vec3,
        direction: Vec3,
        minimum: f32,
        maximum: f32,
    ) -> bool {
        let Some(node) = self.nodes.get(node_index) else {
            return false;
        };
        let mut polygon_index = Some(node_index);
        while let Some(index) = polygon_index {
            let Some(polygon) = self.nodes.get(index) else {
                break;
            };
            if polygon.flags & 4 == 0
                && self
                    .node_ray_intersection(polygon, origin, direction, maximum)
                    .is_some_and(|distance| distance >= minimum && distance < maximum)
            {
                return true;
            }
            polygon_index = usize::try_from(polygon.coplanar).ok();
        }

        let from_side = plane_side(node.plane, origin);
        let to_side = plane_side(node.plane, origin + direction * maximum);
        if let Ok(front) = usize::try_from(node.front)
            && (from_side >= 0.0 || to_side >= 0.0)
            && self.trace_light_node(front, origin, direction, minimum, maximum)
        {
            return true;
        }
        if let Ok(back) = usize::try_from(node.back)
            && (from_side <= 0.0 || to_side <= 0.0)
            && self.trace_light_node(back, origin, direction, minimum, maximum)
        {
            return true;
        }
        false
    }

    fn node_ray_intersection(
        &self,
        node: &crate::BspNode,
        origin: Vec3,
        direction: Vec3,
        maximum: f32,
    ) -> Option<f32> {
        if node.vertex_count < 3 {
            return None;
        }
        let surface = usize::try_from(node.surface)
            .ok()
            .and_then(|index| self.surfaces.get(index))?;
        if surface.poly_flags.contains(crate::PolyFlags::NOT_SOLID) {
            return None;
        }
        let from_side = plane_side(node.plane, origin);
        let to_side = plane_side(node.plane, origin + direction * maximum);
        if from_side.is_sign_positive() == to_side.is_sign_positive()
            && from_side != 0.0
            && to_side != 0.0
        {
            return None;
        }
        let first = usize::try_from(node.vertex_pool).ok()?;
        let polygon = self
            .vertices
            .get(first..first.checked_add(usize::from(node.vertex_count))?)?;
        let point = |vertex: &crate::BspVertex| {
            usize::try_from(vertex.point)
                .ok()
                .and_then(|index| self.points.get(index))
                .copied()
        };
        let a = point(&polygon[0])?;
        let mut b = point(&polygon[1])?;
        let mut nearest = None;
        for vertex in &polygon[2..] {
            let c = point(vertex)?;
            if let Some(distance) = ray_triangle(origin, direction, maximum, [a, b, c]) {
                nearest = Some(nearest.map_or(distance, |current: f32| current.min(distance)));
            }
            b = c;
        }
        nearest
    }
}

impl VertexLighting {
    pub fn for_actor(
        &self,
        model: &Model,
        center: Vec3,
        ambient_glow: u8,
        scale_glow: f32,
    ) -> ActorVertexLighting {
        let ambient_glow = if ambient_glow == 255 {
            0.2
        } else {
            f32::from(ambient_glow) / 255.0
        };
        let ambient = self
            .zone_ambient
            .get(model.zone_at(center))
            .copied()
            .unwrap_or(self.level_ambient)
            + ambient_glow;
        let lights = self
            .lights
            .iter()
            .copied()
            .filter(|light| {
                let mut offset = light.location - center;
                if light.effect == 17 {
                    offset.z = 0.0;
                }
                offset.length_squared() < light.radius * light.radius
                    && !model.blocks_light(light.location, center)
            })
            .collect();
        ActorVertexLighting {
            ambient,
            scale_glow,
            lights,
        }
    }
}

pub fn bsp_zone_at(nodes: &[crate::BspNode], zone_count: usize, point: Vec3) -> usize {
    bsp_zone_at_checked(nodes, zone_count, point).unwrap_or(0)
}

pub fn bsp_zone_at_checked(
    nodes: &[crate::BspNode],
    zone_count: usize,
    point: Vec3,
) -> Option<usize> {
    let mut node_index = 0;
    loop {
        let node = nodes.get(node_index)?;
        let side = plane_side(node.plane, point);
        let child = if side >= 0.0 { node.front } else { node.back };
        if let Ok(child) = usize::try_from(child) {
            node_index = child;
            continue;
        }
        return usize::try_from(if side >= 0.0 {
            node.zones[1]
        } else {
            node.zones[0]
        })
        .ok()
        .filter(|zone| *zone < zone_count);
    }
}

fn plane_side(plane: [f32; 4], point: Vec3) -> f32 {
    Vec3::from_array([plane[0], plane[1], plane[2]]).dot(point) - plane[3]
}

fn ray_triangle(origin: Vec3, direction: Vec3, maximum: f32, points: [Vec3; 3]) -> Option<f32> {
    let edge1 = points[1] - points[0];
    let edge2 = points[2] - points[0];
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let t = origin - points[0];
    let u = t.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge2.dot(q) * inverse;
    (distance > f32::EPSILON && distance <= maximum).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_zone_on_each_side_of_a_bsp_leaf() {
        let node = crate::BspNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            zone_mask: 0,
            flags: 0,
            vertex_pool: 0,
            surface: 0,
            back: -1,
            front: -1,
            coplanar: -1,
            collision_bound: -1,
            render_bound: -1,
            zones: [2, 1],
            vertex_count: 0,
            leaves: [-1; 2],
        };
        let nodes = [node];
        assert_eq!(bsp_zone_at(&nodes, 3, Vec3::X), 1);
        assert_eq!(bsp_zone_at(&nodes, 3, -Vec3::X), 2);
    }

    #[test]
    fn checked_zone_lookup_distinguishes_zone_zero_from_outside_the_model() {
        let node = crate::BspNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            zone_mask: 0,
            flags: 0,
            vertex_pool: 0,
            surface: 0,
            back: -1,
            front: -1,
            coplanar: -1,
            collision_bound: -1,
            render_bound: -1,
            zones: [0, 1],
            vertex_count: 0,
            leaves: [-1; 2],
        };
        assert_eq!(bsp_zone_at_checked(&[node], 2, -Vec3::X), Some(0));
        assert_eq!(bsp_zone_at_checked(&[], 2, Vec3::ZERO), None);
    }

    #[test]
    fn vertex_lighting_matches_ue1_ambient_and_radial_falloff() {
        let lighting = ActorVertexLighting {
            ambient: Vec3::splat(0.1),
            scale_glow: 1.0,
            lights: vec![VertexLight {
                location: Vec3::new(5.0, 0.0, 0.0),
                color: Vec3::splat(0.2),
                radius: 10.0,
                effect: 0,
            }],
        };
        assert_eq!(lighting.color(Vec3::ZERO, Vec3::X, false), Vec3::splat(0.5));
        assert_eq!(lighting.color(Vec3::ZERO, Vec3::X, true), Vec3::splat(1.2));
    }

    #[test]
    fn light_trace_intersects_two_sided_triangle() {
        let triangle = [
            Vec3::new(2.0, -1.0, -1.0),
            Vec3::new(2.0, 1.0, -1.0),
            Vec3::new(2.0, 0.0, 1.0),
        ];
        assert_eq!(ray_triangle(Vec3::ZERO, Vec3::X, 10.0, triangle), Some(2.0));
        assert_eq!(
            ray_triangle(Vec3::new(4.0, 0.0, 0.0), -Vec3::X, 10.0, triangle),
            Some(2.0)
        );
    }
}
