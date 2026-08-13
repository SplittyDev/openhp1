//! UE1-compatible collision queries over decoded map geometry.

use glam::{Mat3, Vec3};
use openhp1_map::{BspNode, BspVertex, Model, PolyFlags};
use openhp1_package::ObjectReference;
use thiserror::Error;

use crate::actor::sweep_cylinder_aabb;

const HULL_FLIP: i32 = 0x4000_0000;
const NODE_NOT_CSG_MASK: u8 = 0x21;
const BOX_EPSILON: f32 = 0.1;
const TRACE_MARGIN: f32 = 1.0;
const TRAVERSAL_EXTENT_SCALE: f32 = 1.1;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionHit {
    pub fraction: f32,
    pub normal: Vec3,
    pub node: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceHit {
    pub texture: ObjectReference,
    pub poly_flags: PolyFlags,
}

#[derive(Clone, Debug)]
pub struct BspCollision {
    hulls: Vec<ConvexHull>,
    hull_by_node: Vec<Option<usize>>,
    root_outside: bool,
    zone_nodes: Vec<BspNode>,
    points: Vec<Vec3>,
    vertices: Vec<BspVertex>,
    zone_actors: Vec<Option<usize>>,
    node_surfaces: Vec<Option<SurfaceHit>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BspPointRegion {
    pub leaf: i32,
    pub zone: usize,
}

#[cfg(test)]
#[path = "bsp_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
struct ConvexHull {
    node: usize,
    bounds: Aabb,
    planes: Vec<HullPlane>,
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

#[derive(Clone, Copy, Debug)]
struct HullPlane {
    node: usize,
    plane: Plane,
}

#[derive(Clone, Copy)]
enum SweepShape {
    Aabb(Vec3),
    Cylinder {
        radius: f32,
        height: f32,
        support_transform: Mat3,
    },
}

impl SweepShape {
    fn bounds(self) -> Vec3 {
        match self {
            Self::Aabb(extents) => extents,
            Self::Cylinder { .. } => Vec3::new(
                self.support(Vec3::X),
                self.support(Vec3::Y),
                self.support(Vec3::Z),
            ),
        }
    }

    fn support(self, normal: Vec3) -> f32 {
        match self {
            Self::Aabb(extents) => normal.abs().dot(extents),
            Self::Cylinder {
                radius,
                height,
                support_transform,
            } => {
                let normal = support_transform * normal;
                normal.truncate().length() * radius + normal.z.abs() * height
            }
        }
    }

    fn axis_aligned_cylinder(self) -> Option<(f32, f32)> {
        let Self::Cylinder {
            radius,
            height,
            support_transform,
        } = self
        else {
            return None;
        };
        let x = support_transform * Vec3::X;
        let y = support_transform * Vec3::Y;
        let z = support_transform * Vec3::Z;
        let x_scale = x.truncate().length();
        let y_scale = y.truncate().length();
        (x.z.abs() <= 1.0e-5
            && y.z.abs() <= 1.0e-5
            && z.truncate().length_squared() <= 1.0e-10
            && (x_scale - y_scale).abs() <= 1.0e-5
            && x.truncate().dot(y.truncate()).abs() <= 1.0e-5)
            .then_some((radius * x_scale, height * z.z.abs()))
    }
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
                planes.push(HullPlane {
                    node: plane_index,
                    plane,
                });
            }
            hulls.push(ConvexHull {
                node: node_index,
                bounds,
                planes,
            });
        }
        let zone_actors = model
            .zones
            .iter()
            .map(|zone| match zone.actor {
                ObjectReference::Export(export) => Some(export),
                _ => None,
            })
            .collect();
        let mut hull_by_node = vec![None; model.nodes.len()];
        for (hull_index, hull) in hulls.iter().enumerate() {
            hull_by_node[hull.node] = Some(hull_index);
        }
        Ok(Self {
            hulls,
            hull_by_node,
            root_outside: model.root_outside,
            zone_nodes: model.nodes.clone(),
            points: model.points.clone(),
            vertices: model.vertices.clone(),
            zone_actors,
            node_surfaces: model
                .nodes
                .iter()
                .map(|node| {
                    usize::try_from(node.surface)
                        .ok()
                        .and_then(|surface| model.surfaces.get(surface))
                        .map(|surface| SurfaceHit {
                            texture: surface.texture,
                            poly_flags: surface.poly_flags,
                        })
                })
                .collect(),
        })
    }

    pub fn sweep_aabb(&self, start: Vec3, end: Vec3, extents: Vec3) -> Option<CollisionHit> {
        if !start.is_finite()
            || !end.is_finite()
            || !extents.is_finite()
            || extents.cmplt(Vec3::ZERO).any()
        {
            return None;
        }
        if extents == Vec3::ZERO {
            self.line_trace(start, end)
        } else {
            self.sweep_shape(start, end, SweepShape::Aabb(extents))
        }
    }

    pub fn sweep_point(&self, start: Vec3, end: Vec3) -> Option<CollisionHit> {
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        self.sweep_shape(start, end, SweepShape::Aabb(Vec3::ZERO))
    }

    pub fn sweep_cylinder(
        &self,
        start: Vec3,
        end: Vec3,
        radius: f32,
        height: f32,
    ) -> Option<CollisionHit> {
        if !start.is_finite()
            || !end.is_finite()
            || !radius.is_finite()
            || !height.is_finite()
            || radius < 0.0
            || height < 0.0
        {
            return None;
        }
        self.sweep_shape(
            start,
            end,
            SweepShape::Cylinder {
                radius,
                height,
                support_transform: Mat3::IDENTITY,
            },
        )
    }

    pub fn overlaps_aabb(&self, location: Vec3, extents: Vec3) -> bool {
        location.is_finite()
            && extents.is_finite()
            && !extents.cmplt(Vec3::ZERO).any()
            && self.overlaps_shape(location, SweepShape::Aabb(extents))
    }
    pub fn overlaps_cylinder(&self, location: Vec3, radius: f32, height: f32) -> bool {
        location.is_finite()
            && radius.is_finite()
            && height.is_finite()
            && radius >= 0.0
            && height >= 0.0
            && self.overlaps_shape(
                location,
                SweepShape::Cylinder {
                    radius,
                    height,
                    support_transform: Mat3::IDENTITY,
                },
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sweep_transformed_aabb(
        &self,
        start: Vec3,
        end: Vec3,
        extents: Vec3,
        location: Vec3,
        rotation: Mat3,
        pre_pivot: Vec3,
        scale: Vec3,
    ) -> Option<CollisionHit> {
        if !start.is_finite()
            || !end.is_finite()
            || !extents.is_finite()
            || !location.is_finite()
            || !rotation.is_finite()
            || !pre_pivot.is_finite()
            || !scale.is_finite()
            || extents.cmplt(Vec3::ZERO).any()
            || scale.abs().cmple(Vec3::splat(f32::EPSILON)).any()
        {
            return None;
        }
        let world_to_local = rotation.transpose();
        let to_local = |point| world_to_local * (point - location) / scale + pre_pivot;
        self.sweep_aabb(to_local(start), to_local(end), extents / scale.abs())
            .map(|mut hit| {
                hit.normal = (rotation * (hit.normal / scale)).normalize();
                hit
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sweep_transformed_cylinder(
        &self,
        start: Vec3,
        end: Vec3,
        radius: f32,
        height: f32,
        location: Vec3,
        rotation: Mat3,
        pre_pivot: Vec3,
        scale: Vec3,
    ) -> Option<CollisionHit> {
        if !start.is_finite()
            || !end.is_finite()
            || !radius.is_finite()
            || !height.is_finite()
            || !location.is_finite()
            || !rotation.is_finite()
            || !pre_pivot.is_finite()
            || !scale.is_finite()
            || radius < 0.0
            || height < 0.0
            || scale.abs().cmple(Vec3::splat(f32::EPSILON)).any()
        {
            return None;
        }
        let world_to_local = rotation.transpose();
        let to_local = |point| world_to_local * (point - location) / scale + pre_pivot;
        self.sweep_shape(
            to_local(start),
            to_local(end),
            SweepShape::Cylinder {
                radius,
                height,
                support_transform: rotation * Mat3::from_diagonal(scale.recip()),
            },
        )
        .map(|mut hit| {
            hit.normal = (rotation * (hit.normal / scale)).normalize();
            hit
        })
    }

    pub fn overlaps_transformed_aabb(
        &self,
        center: Vec3,
        extents: Vec3,
        location: Vec3,
        rotation: Mat3,
        pre_pivot: Vec3,
        scale: Vec3,
    ) -> bool {
        if !center.is_finite()
            || !extents.is_finite()
            || !location.is_finite()
            || !rotation.is_finite()
            || !pre_pivot.is_finite()
            || !scale.is_finite()
            || extents.cmplt(Vec3::ZERO).any()
            || scale.abs().cmple(Vec3::splat(f32::EPSILON)).any()
        {
            return false;
        }
        let local_center = rotation.transpose() * (center - location) / scale + pre_pivot;
        self.overlaps_aabb(local_center, extents / scale.abs())
    }

    pub fn transformed_bounds(
        &self,
        location: Vec3,
        rotation: Mat3,
        pre_pivot: Vec3,
        scale: Vec3,
    ) -> Option<(Vec3, Vec3)> {
        if !location.is_finite()
            || !rotation.is_finite()
            || !pre_pivot.is_finite()
            || !scale.is_finite()
        {
            return None;
        }
        let minimum = self
            .hulls
            .iter()
            .map(|hull| hull.bounds.minimum)
            .reduce(Vec3::min)?;
        let maximum = self
            .hulls
            .iter()
            .map(|hull| hull.bounds.maximum)
            .reduce(Vec3::max)?;
        let center = (minimum + maximum) * 0.5;
        let extents = (maximum - minimum) * 0.5;
        Some((
            location + rotation * ((center - pre_pivot) * scale),
            rotation.abs() * (extents * scale.abs()) + Vec3::splat(BOX_EPSILON),
        ))
    }

    fn sweep_shape(&self, start: Vec3, end: Vec3, shape: SweepShape) -> Option<CollisionHit> {
        let extents = shape.bounds();
        let delta = end - start;
        let distance = delta.length();
        if distance <= f32::EPSILON {
            return None;
        }
        let direction = delta / distance;
        let trace_distance = distance + TRACE_MARGIN;
        let trace_end = start + direction * trace_distance;
        let query_bounds = Aabb {
            minimum: start.min(trace_end) - extents,
            maximum: start.max(trace_end) + extents,
        };
        let mut nearest = None;
        let hulls = &self.hulls;
        visit_reached_hulls(
            &self.zone_nodes,
            &self.hull_by_node,
            self.root_outside,
            start,
            end,
            extents,
            &mut |hull_index| {
                let hull = &hulls[hull_index];
                if hull.bounds.minimum.x > query_bounds.maximum.x
                    || hull.bounds.maximum.x < query_bounds.minimum.x
                    || hull.bounds.maximum.y < query_bounds.minimum.y
                    || hull.bounds.minimum.y > query_bounds.maximum.y
                    || hull.bounds.maximum.z < query_bounds.minimum.z
                    || hull.bounds.minimum.z > query_bounds.maximum.z
                {
                    return;
                }
                let bounds = Aabb {
                    minimum: hull.bounds.minimum + Vec3::splat(BOX_EPSILON),
                    maximum: hull.bounds.maximum - Vec3::splat(BOX_EPSILON),
                };
                if let Some((radius, height)) = shape.axis_aligned_cylinder()
                    && hull_is_axis_aligned_box(hull)
                {
                    let target = (bounds.minimum + bounds.maximum) * 0.5;
                    let target_extents = (bounds.maximum - bounds.minimum) * 0.5;
                    let Some(hit) =
                        sweep_cylinder_aabb(start, end, height, radius, target, target_extents)
                    else {
                        return;
                    };
                    let hit = CollisionHit {
                        fraction: hit.fraction,
                        normal: hit.normal,
                        node: hull
                            .planes
                            .iter()
                            .find(|plane| plane.plane.normal.abs_diff_eq(hit.normal, 0.00001))
                            .map_or(hull.node, |plane| plane.node),
                    };
                    if nearest.is_none_or(|(_, current): (usize, CollisionHit)| {
                        hit.fraction < current.fraction
                    }) {
                        nearest = Some((hull_index, hit));
                    }
                    return;
                }
                let mut cursor = SweepCursor::new(start, trace_end, shape);
                if !cursor.clip_box(bounds) {
                    return;
                }
                if !hull
                    .planes
                    .iter()
                    .copied()
                    .all(|plane| cursor.clip_plane(plane.plane, Some(plane.node)))
                {
                    return;
                }
                for (index, plane) in hull.planes.iter().copied().enumerate() {
                    for other in hull.planes[..index].iter().copied() {
                        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                            if opposite_axis_signs(plane.plane.normal, other.plane.normal, axis) {
                                cursor.clip_bevel(plane.plane, other.plane, axis);
                            }
                        }
                    }
                }
                let Some(hit_distance) = cursor.hit_distance(trace_distance) else {
                    return;
                };
                let fraction = ((hit_distance - TRACE_MARGIN).max(0.0) / distance).min(1.0);
                let hit = CollisionHit {
                    fraction,
                    normal: cursor.hit_normal,
                    node: cursor.hit_node.unwrap_or_else(|| {
                        hull.planes
                            .iter()
                            .find(|plane| {
                                plane.plane.normal.abs_diff_eq(cursor.hit_normal, 0.00001)
                            })
                            .map_or(hull.node, |plane| plane.node)
                    }),
                };
                if nearest.is_none_or(|(_, current): (usize, CollisionHit)| {
                    hit.fraction < current.fraction
                }) {
                    nearest = Some((hull_index, hit));
                }
            },
        );
        nearest.map(|(_, hit)| hit)
    }

    fn overlaps_shape(&self, location: Vec3, shape: SweepShape) -> bool {
        let extents = shape.bounds();
        let query_bounds = Aabb {
            minimum: location - extents,
            maximum: location + extents,
        };
        let mut overlaps = false;
        visit_reached_hulls(
            &self.zone_nodes,
            &self.hull_by_node,
            self.root_outside,
            location,
            location,
            extents,
            &mut |hull_index| {
                if overlaps {
                    return;
                }
                let hull = &self.hulls[hull_index];
                if hull.bounds.minimum.x > query_bounds.maximum.x
                    || hull.bounds.maximum.x < query_bounds.minimum.x
                    || hull.bounds.maximum.y < query_bounds.minimum.y
                    || hull.bounds.minimum.y > query_bounds.maximum.y
                    || hull.bounds.maximum.z < query_bounds.minimum.z
                    || hull.bounds.minimum.z > query_bounds.maximum.z
                {
                    return;
                }
                let bounds = Aabb {
                    minimum: hull.bounds.minimum + Vec3::splat(BOX_EPSILON),
                    maximum: hull.bounds.maximum - Vec3::splat(BOX_EPSILON),
                };
                let mut cursor = SweepCursor::new(location, location, shape);
                if !cursor.clip_box(bounds)
                    || !hull
                        .planes
                        .iter()
                        .copied()
                        .all(|plane| cursor.clip_plane(plane.plane, Some(plane.node)))
                {
                    return;
                }
                for (index, plane) in hull.planes.iter().copied().enumerate() {
                    for other in hull.planes[..index].iter().copied() {
                        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                            if opposite_axis_signs(plane.plane.normal, other.plane.normal, axis) {
                                cursor.clip_bevel(plane.plane, other.plane, axis);
                            }
                        }
                    }
                }
                if !cursor.no_hit {
                    overlaps = true;
                }
            },
        );
        overlaps
    }

    pub fn line_trace(&self, start: Vec3, end: Vec3) -> Option<CollisionHit> {
        let delta = end - start;
        let distance = delta.length();
        if distance <= f32::EPSILON || self.zone_nodes.is_empty() {
            return None;
        }
        let direction = delta / distance;
        let trace_distance = distance + TRACE_MARGIN;
        let mut nearest = None;
        self.line_trace_node(0, start, direction, trace_distance, &mut nearest);
        nearest.map(|(hit_distance, normal, node)| CollisionHit {
            fraction: ((hit_distance - TRACE_MARGIN).max(0.0) / distance).min(1.0),
            normal,
            node,
        })
    }

    fn line_trace_node(
        &self,
        node_index: usize,
        origin: Vec3,
        direction: Vec3,
        maximum: f32,
        nearest: &mut Option<(f32, Vec3, usize)>,
    ) {
        let Some(node) = self.zone_nodes.get(node_index) else {
            return;
        };
        let mut polygon_index = Some(node_index);
        while let Some(index) = polygon_index {
            let Some(polygon) = self.zone_nodes.get(index) else {
                break;
            };
            if !self
                .surface_hit(index)
                .is_some_and(|surface| surface.poly_flags.contains(PolyFlags::NOT_SOLID))
                && let Some(distance) =
                    self.node_ray_intersection(polygon, origin, direction, maximum)
                && nearest.is_none_or(|(current, _, _)| distance < current)
            {
                let mut normal =
                    Vec3::from_array([polygon.plane[0], polygon.plane[1], polygon.plane[2]]);
                if normal.dot(direction) > 0.0 {
                    normal = -normal;
                }
                *nearest = Some((distance, normal, index));
            }
            polygon_index = usize::try_from(polygon.coplanar).ok();
        }

        let from_side = plane_side(node.plane, origin);
        let to_side = plane_side(node.plane, origin + direction * maximum);
        if let Ok(front) = usize::try_from(node.front)
            && (from_side >= 0.0 || to_side >= 0.0)
        {
            self.line_trace_node(front, origin, direction, maximum, nearest);
        }
        if let Ok(back) = usize::try_from(node.back)
            && (from_side <= 0.0 || to_side <= 0.0)
        {
            self.line_trace_node(back, origin, direction, maximum, nearest);
        }
    }

    fn node_ray_intersection(
        &self,
        node: &BspNode,
        origin: Vec3,
        direction: Vec3,
        maximum: f32,
    ) -> Option<f32> {
        if node.vertex_count < 3 {
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
        let point = |vertex: &BspVertex| {
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

    pub fn hull_count(&self) -> usize {
        self.hulls.len()
    }

    pub fn node_has_poly_flag(&self, node: usize, flag: PolyFlags) -> bool {
        self.surface_hit(node)
            .is_some_and(|surface| surface.poly_flags.contains(flag))
    }

    pub fn surface_hit(&self, node: usize) -> Option<SurfaceHit> {
        self.node_surfaces.get(node).copied().flatten()
    }

    pub fn zone_at(&self, point: Vec3) -> Option<usize> {
        self.point_region(point).map(|region| region.zone)
    }

    pub fn point_region(&self, point: Vec3) -> Option<BspPointRegion> {
        let mut node_index = 0;
        loop {
            let node = self.zone_nodes.get(node_index)?;
            let side = Vec3::from_array([node.plane[0], node.plane[1], node.plane[2]]).dot(point)
                - node.plane[3];
            if side >= 0.0
                && let Ok(front) = usize::try_from(node.front)
            {
                node_index = front;
            } else if side <= 0.0
                && let Ok(back) = usize::try_from(node.back)
            {
                node_index = back;
            } else {
                let index = usize::from(side < 0.0);
                let zone = usize::try_from(node.zones[1 - index]).ok()?;
                return (zone < self.zone_actors.len()).then_some(BspPointRegion {
                    leaf: node.leaves[index],
                    zone,
                });
            }
        }
    }

    pub fn zone_actor_export(&self, zone: usize) -> Option<usize> {
        self.zone_actors.get(zone).copied().flatten()
    }
}

#[allow(clippy::too_many_arguments)]
fn visit_reached_hulls(
    nodes: &[BspNode],
    hull_by_node: &[Option<usize>],
    root_outside: bool,
    start: Vec3,
    end: Vec3,
    extents: Vec3,
    visit: &mut impl FnMut(usize),
) {
    #[allow(clippy::too_many_arguments)]
    fn visit_node(
        nodes: &[BspNode],
        hull_by_node: &[Option<usize>],
        parent: Option<usize>,
        node: Option<usize>,
        outside: bool,
        start: Vec3,
        end: Vec3,
        extents: Vec3,
        visit: &mut impl FnMut(usize),
    ) {
        let Some(index) = node else {
            if !outside && let Some(hull) = parent.and_then(|parent| hull_by_node[parent]) {
                visit(hull);
            }
            return;
        };
        let Some(node) = nodes.get(index) else {
            return;
        };
        let normal = Vec3::from_array([node.plane[0], node.plane[1], node.plane[2]]);
        let start_distance = normal.dot(start) - node.plane[3];
        let end_distance = normal.dot(end) - node.plane[3];
        let support = normal.abs().dot(extents * TRAVERSAL_EXTENT_SCALE);
        let reaches = [
            start_distance <= support || end_distance <= support,
            start_distance >= -support || end_distance >= -support,
        ];
        let near = usize::from(start_distance >= -support);
        let is_csg = node.vertex_count > 0 && node.flags & NODE_NOT_CSG_MASK == 0;

        for side in [near, 1 - near] {
            if !reaches[side] {
                continue;
            }
            let outside = if side == 1 {
                outside || is_csg
            } else {
                outside && !is_csg
            };
            let child = [node.back, node.front][side];
            visit_node(
                nodes,
                hull_by_node,
                Some(index),
                usize::try_from(child).ok(),
                outside,
                start,
                end,
                extents,
                visit,
            );
        }
    }

    if !nodes.is_empty() {
        visit_node(
            nodes,
            hull_by_node,
            None,
            Some(0),
            root_outside,
            start,
            end,
            extents,
            visit,
        );
    }
}

fn hull_is_axis_aligned_box(hull: &ConvexHull) -> bool {
    if hull.planes.len() != 6 {
        return false;
    }
    let mut axes = 0_u8;
    for plane in &hull.planes {
        let normal = plane.plane.normal;
        let bit = if normal.abs_diff_eq(Vec3::X, 0.00001) {
            0
        } else if normal.abs_diff_eq(Vec3::NEG_X, 0.00001) {
            1
        } else if normal.abs_diff_eq(Vec3::Y, 0.00001) {
            2
        } else if normal.abs_diff_eq(Vec3::NEG_Y, 0.00001) {
            3
        } else if normal.abs_diff_eq(Vec3::Z, 0.00001) {
            4
        } else if normal.abs_diff_eq(Vec3::NEG_Z, 0.00001) {
            5
        } else {
            return false;
        };
        axes |= 1 << bit;
    }
    axes == 0b11_1111
}

struct SweepCursor {
    start: Vec3,
    end: Vec3,
    shape: SweepShape,
    start_fraction: f32,
    end_fraction: f32,
    hit_normal: Vec3,
    hit_node: Option<usize>,
    no_hit: bool,
}

impl SweepCursor {
    fn new(start: Vec3, end: Vec3, shape: SweepShape) -> Self {
        Self {
            start,
            end,
            shape,
            start_fraction: -1.0,
            end_fraction: 2.0,
            hit_normal: Vec3::ZERO,
            hit_node: None,
            no_hit: false,
        }
    }

    fn clip_plane(&mut self, plane: Plane, node: Option<usize>) -> bool {
        let push_out = self.shape.support(plane.normal);
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
            if fraction > self.start_fraction
                || (fraction == self.start_fraction && self.hit_node.is_none() && node.is_some())
            {
                self.start_fraction = fraction;
                self.hit_normal = plane.normal;
                self.hit_node = node;
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
        self.clip_plane(
            Plane {
                normal,
                distance: point.dot(normal),
            },
            None,
        );
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
        .all(|plane| self.clip_plane(plane, None))
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
