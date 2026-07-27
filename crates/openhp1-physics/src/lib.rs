//! UE1-compatible collision queries over decoded map geometry.

use glam::Vec3;
use openhp1_map::Model;
use thiserror::Error;

const HULL_FLIP: i32 = 0x4000_0000;
const BOX_EPSILON: f32 = 0.1;
const TRACE_MARGIN: f32 = 1.0;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionHit {
    pub fraction: f32,
    pub normal: Vec3,
    pub node: usize,
}

#[derive(Clone, Debug)]
pub struct BspCollision {
    hulls: Vec<ConvexHull>,
}

#[derive(Clone, Debug)]
struct ConvexHull {
    node: usize,
    bounds: Aabb,
    planes: Vec<Plane>,
}

#[derive(Clone, Copy, Debug)]
struct Aabb {
    minimum: Vec3,
    maximum: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: Vec3,
    distance: f32,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("BSP node {node} collision hull starts outside the leaf-hull stream at {offset}")]
    InvalidHullOffset { node: usize, offset: i32 },

    #[error("BSP node {node} collision hull at {offset} has no terminating marker")]
    UnterminatedHull { node: usize, offset: usize },

    #[error("BSP node {node} collision hull at {offset} is missing its bounds")]
    MissingHullBounds { node: usize, offset: usize },

    #[error("BSP node {node} collision hull references missing plane node {plane}")]
    InvalidHullPlane { node: usize, plane: usize },

    #[error("BSP node {node} collision hull has non-finite bounds")]
    NonFiniteHullBounds { node: usize },

    #[error("BSP node {node} collision hull has inverted bounds")]
    InvertedHullBounds { node: usize },

    #[error("BSP node {node} collision hull has a non-finite plane")]
    NonFiniteHullPlane { node: usize },
}

impl BspCollision {
    pub fn from_model(model: &Model) -> Result<Self> {
        let mut hulls = Vec::new();
        for (node_index, node) in model.nodes.iter().enumerate() {
            if node.collision_bound < 0 {
                continue;
            }
            let offset =
                usize::try_from(node.collision_bound).map_err(|_| Error::InvalidHullOffset {
                    node: node_index,
                    offset: node.collision_bound,
                })?;
            let stream = model
                .leaf_hulls
                .get(offset..)
                .ok_or(Error::InvalidHullOffset {
                    node: node_index,
                    offset: node.collision_bound,
                })?;
            let plane_count =
                stream
                    .iter()
                    .position(|value| *value < 0)
                    .ok_or(Error::UnterminatedHull {
                        node: node_index,
                        offset,
                    })?;
            let bounds =
                stream
                    .get(plane_count + 1..plane_count + 7)
                    .ok_or(Error::MissingHullBounds {
                        node: node_index,
                        offset,
                    })?;
            let bounds = Aabb {
                minimum: Vec3::new(
                    f32::from_bits(bounds[0] as u32),
                    f32::from_bits(bounds[1] as u32),
                    f32::from_bits(bounds[2] as u32),
                ),
                maximum: Vec3::new(
                    f32::from_bits(bounds[3] as u32),
                    f32::from_bits(bounds[4] as u32),
                    f32::from_bits(bounds[5] as u32),
                ),
            };
            if !bounds.minimum.is_finite() || !bounds.maximum.is_finite() {
                return Err(Error::NonFiniteHullBounds { node: node_index });
            }
            if !bounds.minimum.cmple(bounds.maximum).all() {
                return Err(Error::InvertedHullBounds { node: node_index });
            }

            let mut planes = Vec::with_capacity(plane_count);
            for &encoded in &stream[..plane_count] {
                let flipped = encoded & HULL_FLIP != 0;
                let plane_index = (encoded & !HULL_FLIP) as usize;
                let node = model
                    .nodes
                    .get(plane_index)
                    .ok_or(Error::InvalidHullPlane {
                        node: node_index,
                        plane: plane_index,
                    })?;
                let mut plane = Plane {
                    normal: Vec3::from_array([node.plane[0], node.plane[1], node.plane[2]]),
                    distance: node.plane[3],
                };
                if !plane.normal.is_finite() || !plane.distance.is_finite() {
                    return Err(Error::NonFiniteHullPlane { node: node_index });
                }
                if flipped {
                    plane.normal = -plane.normal;
                    plane.distance = -plane.distance;
                }
                planes.push(plane);
            }
            hulls.push(ConvexHull {
                node: node_index,
                bounds,
                planes,
            });
        }
        Ok(Self { hulls })
    }

    pub fn sweep_aabb(&self, start: Vec3, end: Vec3, extents: Vec3) -> Option<CollisionHit> {
        if !start.is_finite()
            || !end.is_finite()
            || !extents.is_finite()
            || extents.cmplt(Vec3::ZERO).any()
        {
            return None;
        }
        let delta = end - start;
        let distance = delta.length();
        if distance <= f32::EPSILON {
            return None;
        }
        let direction = delta / distance;
        let trace_distance = distance + TRACE_MARGIN;
        let trace_end = start + direction * trace_distance;
        let mut nearest = None;

        // ponytail: scan decoded hulls linearly until collision profiling justifies
        // retaining the package BSP traversal alongside them.
        for hull in &self.hulls {
            let bounds = Aabb {
                minimum: hull.bounds.minimum + Vec3::splat(BOX_EPSILON),
                maximum: hull.bounds.maximum - Vec3::splat(BOX_EPSILON),
            };
            let mut cursor = SweepCursor::new(start, trace_end, extents);
            if !cursor.clip_box(bounds) {
                continue;
            }
            if !hull
                .planes
                .iter()
                .copied()
                .all(|plane| cursor.clip_plane(plane))
            {
                continue;
            }
            for (index, plane) in hull.planes.iter().copied().enumerate() {
                for other in hull.planes[..index].iter().copied() {
                    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                        if opposite_axis_signs(plane.normal, other.normal, axis) {
                            cursor.clip_bevel(plane, other, axis);
                        }
                    }
                }
            }
            let Some(hit_distance) = cursor.hit_distance(trace_distance) else {
                continue;
            };
            let fraction = ((hit_distance - TRACE_MARGIN).max(0.0) / distance).min(1.0);
            let hit = CollisionHit {
                fraction,
                normal: cursor.hit_normal,
                node: hull.node,
            };
            if nearest.is_none_or(|current: CollisionHit| hit.fraction < current.fraction) {
                nearest = Some(hit);
            }
        }
        nearest
    }

    pub fn hull_count(&self) -> usize {
        self.hulls.len()
    }
}

struct SweepCursor {
    start: Vec3,
    end: Vec3,
    extents: Vec3,
    start_fraction: f32,
    end_fraction: f32,
    hit_normal: Vec3,
    no_hit: bool,
}

impl SweepCursor {
    fn new(start: Vec3, end: Vec3, extents: Vec3) -> Self {
        Self {
            start,
            end,
            extents,
            start_fraction: -1.0,
            end_fraction: 2.0,
            hit_normal: Vec3::ZERO,
            no_hit: false,
        }
    }

    fn clip_plane(&mut self, plane: Plane) -> bool {
        let push_out = plane.normal.abs().dot(self.extents);
        let start_distance = plane.normal.dot(self.start) - plane.distance;
        let end_distance = plane.normal.dot(self.end) - plane.distance;
        let front_face = start_distance - end_distance;
        let mut box_distance = start_distance - push_out;
        if start_distance > end_distance && box_distance >= -push_out && box_distance < 0.0 {
            box_distance = 0.0;
        }
        let denominator = start_distance - end_distance;
        let fraction = if denominator.abs() <= f32::EPSILON {
            0.0
        } else {
            box_distance / denominator
        };

        if front_face > 0.00001 {
            if fraction > self.start_fraction {
                self.start_fraction = fraction;
                self.hit_normal = plane.normal;
            }
        } else if front_face < -0.00001 {
            self.end_fraction = self.end_fraction.min(fraction);
        } else if start_distance > push_out && end_distance > push_out {
            self.no_hit = true;
            return false;
        }

        if self.start_fraction < self.end_fraction {
            true
        } else {
            self.no_hit = true;
            false
        }
    }

    fn clip_bevel(&mut self, first: Plane, second: Plane, axis: Vec3) {
        let first_cross = axis.cross(first.normal);
        let second_cross = axis.cross(second.normal);
        if first_cross.dot(second_cross) <= 0.00001 {
            return;
        }
        let line = first.normal.cross(second.normal);
        let length_squared = line.length_squared();
        if length_squared < 0.000001 {
            return;
        }
        let point = (first.distance * second.normal.cross(line)
            + second.distance * line.cross(first.normal))
            / length_squared;
        let line = line.normalize();
        let mut normal = axis.cross(line).normalize();
        if first.normal.dot(normal) < 0.0 {
            normal = -normal;
        }
        self.clip_plane(Plane {
            normal,
            distance: point.dot(normal),
        });
    }

    fn clip_box(&mut self, bounds: Aabb) -> bool {
        [
            Plane {
                normal: Vec3::X,
                distance: bounds.maximum.x,
            },
            Plane {
                normal: Vec3::NEG_X,
                distance: -bounds.minimum.x,
            },
            Plane {
                normal: Vec3::Y,
                distance: bounds.maximum.y,
            },
            Plane {
                normal: Vec3::NEG_Y,
                distance: -bounds.minimum.y,
            },
            Plane {
                normal: Vec3::Z,
                distance: bounds.maximum.z,
            },
            Plane {
                normal: Vec3::NEG_Z,
                distance: -bounds.minimum.z,
            },
        ]
        .into_iter()
        .all(|plane| self.clip_plane(plane))
    }

    fn hit_distance(&self, trace_distance: f32) -> Option<f32> {
        (!self.no_hit
            && self.start_fraction > -1.0
            && self.start_fraction < self.end_fraction
            && self.end_fraction > 0.0)
            .then(|| (self.start_fraction * trace_distance - BOX_EPSILON).max(0.0))
    }
}

fn opposite_axis_signs(first: Vec3, second: Vec3, axis: Vec3) -> bool {
    let first = first.dot(axis);
    let second = second.dot(axis);
    (first < 0.0 && second > 0.0) || (first > 0.0 && second < 0.0)
}

#[cfg(test)]
mod tests {
    use openhp1_map::{BspNode, Model, PrimitiveBounds};
    use openhp1_package::ObjectReference;

    use super::*;

    #[test]
    fn decodes_and_sweeps_serialized_leaf_hull() {
        let mut model = empty_model();
        let planes = [
            [1.0, 0.0, 0.0, 10.0],
            [-1.0, 0.0, 0.0, 10.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, -1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, 10.0],
            [0.0, 0.0, -1.0, 10.0],
        ];
        model.nodes = planes
            .into_iter()
            .enumerate()
            .map(|(index, plane)| BspNode {
                plane,
                zone_mask: 0,
                flags: 0,
                vertex_pool: 0,
                surface: 0,
                back: -1,
                front: -1,
                coplanar: -1,
                collision_bound: if index == 0 { 0 } else { -1 },
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 0,
                leaves: [0; 2],
            })
            .collect();
        model.leaf_hulls = vec![0, 1, 2, 3, 4, 5, -1];
        model.leaf_hulls.extend(
            [-10.0_f32, -10.0, -10.0, 10.0, 10.0, 10.0]
                .map(f32::to_bits)
                .map(|value| value as i32),
        );

        let collision = BspCollision::from_model(&model).unwrap();
        let hit = collision
            .sweep_aabb(Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO, Vec3::ONE)
            .unwrap();

        assert_eq!(collision.hull_count(), 1);
        assert!(
            hit.fraction > 0.35 && hit.fraction < 0.45,
            "{}",
            hit.fraction
        );
        assert_eq!(hit.normal, Vec3::X);
    }

    fn empty_model() -> Model {
        Model {
            bounds: PrimitiveBounds {
                minimum: Vec3::ZERO,
                maximum: Vec3::ZERO,
                valid: false,
                sphere: [0.0; 4],
            },
            vectors: Vec::new(),
            points: Vec::new(),
            nodes: Vec::new(),
            surfaces: Vec::new(),
            vertices: Vec::new(),
            shared_side_count: 0,
            zones: Vec::new(),
            polys: ObjectReference::None,
            light_maps: Vec::new(),
            light_bits: Vec::new(),
            collision_bounds: Vec::new(),
            leaf_hulls: Vec::new(),
            leaves: Vec::new(),
            lights: Vec::new(),
            root_outside: true,
            linked: false,
        }
    }
}
