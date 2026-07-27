//! UE1-compatible collision queries over decoded map geometry.

use glam::{Mat3, Vec3};
use openhp1_map::{BspNode, Model, bsp_zone_at};
use openhp1_package::ObjectReference;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorCollisionHit {
    pub fraction: f32,
    pub normal: Vec3,
}

#[derive(Clone, Debug)]
pub struct BspCollision {
    hulls: Vec<ConvexHull>,
    hulls_by_min_x: Vec<usize>,
    zone_nodes: Vec<BspNode>,
    zone_actors: Vec<Option<usize>>,
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

#[derive(Clone, Copy)]
enum SweepShape {
    Aabb(Vec3),
    Cylinder { radius: f32, height: f32 },
}

impl SweepShape {
    fn bounds(self) -> Vec3 {
        match self {
            Self::Aabb(extents) => extents,
            Self::Cylinder { radius, height } => Vec3::new(radius, radius, height),
        }
    }

    fn support(self, normal: Vec3) -> f32 {
        match self {
            Self::Aabb(extents) => normal.abs().dot(extents),
            Self::Cylinder { radius, height } => {
                normal.truncate().length() * radius + normal.z.abs() * height
            }
        }
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
                planes.push(plane);
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
        let mut hulls_by_min_x = (0..hulls.len()).collect::<Vec<_>>();
        hulls_by_min_x.sort_unstable_by(|&left, &right| {
            hulls[left]
                .bounds
                .minimum
                .x
                .total_cmp(&hulls[right].bounds.minimum.x)
                .then_with(|| left.cmp(&right))
        });
        Ok(Self {
            hulls,
            hulls_by_min_x,
            zone_nodes: model.nodes.clone(),
            zone_actors,
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
        self.sweep_shape(start, end, SweepShape::Aabb(extents))
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
        self.sweep_shape(start, end, SweepShape::Cylinder { radius, height })
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
        let candidate_count = self
            .hulls_by_min_x
            .partition_point(|&index| self.hulls[index].bounds.minimum.x <= query_bounds.maximum.x);
        let mut nearest = None;

        for &hull_index in &self.hulls_by_min_x[..candidate_count] {
            let hull = &self.hulls[hull_index];
            if hull.bounds.maximum.x < query_bounds.minimum.x
                || hull.bounds.maximum.y < query_bounds.minimum.y
                || hull.bounds.minimum.y > query_bounds.maximum.y
                || hull.bounds.maximum.z < query_bounds.minimum.z
                || hull.bounds.minimum.z > query_bounds.maximum.z
            {
                continue;
            }
            let bounds = Aabb {
                minimum: hull.bounds.minimum + Vec3::splat(BOX_EPSILON),
                maximum: hull.bounds.maximum - Vec3::splat(BOX_EPSILON),
            };
            if let SweepShape::Cylinder { radius, height } = shape
                && hull_is_axis_aligned_box(hull)
            {
                let target = (bounds.minimum + bounds.maximum) * 0.5;
                let target_extents = (bounds.maximum - bounds.minimum) * 0.5;
                let Some(hit) =
                    sweep_cylinder_aabb(start, end, height, radius, target, target_extents)
                else {
                    continue;
                };
                let hit = CollisionHit {
                    fraction: hit.fraction,
                    normal: hit.normal,
                    node: hull.node,
                };
                if nearest.is_none_or(|(current_index, current): (usize, CollisionHit)| {
                    hit.fraction < current.fraction
                        || (hit.fraction == current.fraction && hull_index < current_index)
                }) {
                    nearest = Some((hull_index, hit));
                }
                continue;
            }
            let mut cursor = SweepCursor::new(start, trace_end, shape);
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
            if nearest.is_none_or(|(current_index, current): (usize, CollisionHit)| {
                hit.fraction < current.fraction
                    || (hit.fraction == current.fraction && hull_index < current_index)
            }) {
                nearest = Some((hull_index, hit));
            }
        }
        nearest.map(|(_, hit)| hit)
    }

    pub fn hull_count(&self) -> usize {
        self.hulls.len()
    }

    pub fn zone_at(&self, point: Vec3) -> usize {
        bsp_zone_at(&self.zone_nodes, self.zone_actors.len(), point)
    }

    pub fn zone_actor_export(&self, zone: usize) -> Option<usize> {
        self.zone_actors.get(zone).copied().flatten()
    }
}

pub fn sweep_cylinder(
    start: Vec3,
    end: Vec3,
    height: f32,
    radius: f32,
    target: Vec3,
    target_height: f32,
    target_radius: f32,
) -> Option<ActorCollisionHit> {
    if !start.is_finite()
        || !end.is_finite()
        || !target.is_finite()
        || ![height, radius, target_height, target_radius]
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0)
    {
        return None;
    }
    let delta = end - start;
    let distance = delta.length();
    if distance <= f32::EPSILON {
        return None;
    }
    if cylinders_overlap(start, height, radius, target, target_height, target_radius) {
        return None;
    }
    let direction = delta / distance;
    let trace_distance = distance + TRACE_MARGIN;
    let combined_height = height + target_height;
    let combined_radius = radius + target_radius;
    let local = start - target;
    let mut nearest = trace_distance;

    let horizontal_a = direction.x.mul_add(direction.x, direction.y * direction.y);
    if horizontal_a > f32::EPSILON {
        let horizontal_b = 2.0 * local.x.mul_add(direction.x, local.y * direction.y);
        let horizontal_c =
            local.x.mul_add(local.x, local.y * local.y) - combined_radius * combined_radius;
        let discriminant = horizontal_b.mul_add(horizontal_b, -4.0 * horizontal_a * horizontal_c);
        if discriminant >= 0.0 {
            let root = discriminant.sqrt();
            for candidate in [
                (-horizontal_b - root) / (2.0 * horizontal_a),
                (-horizontal_b + root) / (2.0 * horizontal_a),
            ] {
                let z = local.z + direction.z * candidate;
                if candidate >= 0.0 && candidate < nearest && z.abs() <= combined_height {
                    nearest = candidate;
                }
            }
        }
    }

    if direction.z.abs() > f32::EPSILON {
        for cap in [-combined_height, combined_height] {
            let candidate = (cap - local.z) / direction.z;
            let x = local.x + direction.x * candidate;
            let y = local.y + direction.y * candidate;
            if candidate >= 0.0
                && candidate < nearest
                && x.mul_add(x, y * y) <= combined_radius * combined_radius
            {
                nearest = candidate;
            }
        }
    }
    if nearest >= trace_distance {
        return None;
    }

    let hit_position = start + direction * nearest;
    let offset = hit_position - target;
    let radial_distance = offset.x.mul_add(offset.x, offset.y * offset.y).sqrt();
    let vertical_depth = combined_height - offset.z.abs();
    let radial_depth = combined_radius - radial_distance;
    let normal = if vertical_depth <= radial_depth || radial_distance < 0.000001 {
        Vec3::new(0.0, 0.0, if offset.z >= 0.0 { 1.0 } else { -1.0 })
    } else {
        Vec3::new(offset.x / radial_distance, offset.y / radial_distance, 0.0)
    };
    Some(ActorCollisionHit {
        fraction: ((nearest - TRACE_MARGIN).max(0.0) / distance).min(1.0),
        normal,
    })
}

pub fn sweep_box(
    start: Vec3,
    end: Vec3,
    moving_extents: Vec3,
    target: Vec3,
    target_extents: Vec3,
    rotation: Mat3,
) -> Option<ActorCollisionHit> {
    if !start.is_finite()
        || !end.is_finite()
        || !moving_extents.is_finite()
        || !target.is_finite()
        || !target_extents.is_finite()
        || !rotation.is_finite()
        || moving_extents.min_element() < 0.0
        || target_extents.min_element() < 0.0
    {
        return None;
    }
    let world_to_local = rotation.transpose();
    let start = world_to_local * (start - target);
    let end = world_to_local * (end - target);
    let extents = target_extents + world_to_local.abs() * moving_extents;
    if start.abs().cmplt(extents).all() {
        return None;
    }

    let delta = end - start;
    let distance = delta.length();
    if distance <= f32::EPSILON {
        return None;
    }
    let mut entry = 0.0_f32;
    let mut exit = 1.0_f32;
    let mut normal = Vec3::ZERO;
    for axis in 0..3 {
        let origin = start[axis];
        let direction = delta[axis];
        let extent = extents[axis];
        if direction.abs() <= f32::EPSILON {
            if origin < -extent || origin > extent {
                return None;
            }
            continue;
        }
        let first = (-extent - origin) / direction;
        let second = (extent - origin) / direction;
        let near = first.min(second);
        let far = first.max(second);
        if near > entry {
            entry = near;
            normal = Vec3::ZERO;
            normal[axis] = if direction > 0.0 { -1.0 } else { 1.0 };
        }
        exit = exit.min(far);
        if entry > exit {
            return None;
        }
    }
    if !(0.0..=1.0).contains(&entry) || normal == Vec3::ZERO {
        return None;
    }
    Some(ActorCollisionHit {
        fraction: ((entry * distance - TRACE_MARGIN).max(0.0) / distance).min(1.0),
        normal: rotation * normal,
    })
}

fn sweep_cylinder_aabb(
    start: Vec3,
    end: Vec3,
    height: f32,
    radius: f32,
    target: Vec3,
    target_extents: Vec3,
) -> Option<ActorCollisionHit> {
    let start = start - target;
    let end = end - target;
    let combined_height = height + target_extents.z;
    let closest = start
        .truncate()
        .clamp(-target_extents.truncate(), target_extents.truncate());
    if start.z.abs() < combined_height
        && start.truncate().distance_squared(closest) < radius * radius
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
    let mut nearest = trace_distance;
    let mut normal = Vec3::ZERO;
    let mut consider = |candidate: f32, candidate_normal: Vec3| {
        if candidate >= 0.0 && candidate < nearest {
            nearest = candidate;
            normal = candidate_normal;
        }
    };

    if direction.x.abs() > f32::EPSILON {
        for sign in [-1.0_f32, 1.0] {
            let candidate = (sign * (target_extents.x + radius) - start.x) / direction.x;
            let point = start + direction * candidate;
            if point.y.abs() <= target_extents.y && point.z.abs() <= combined_height {
                consider(candidate, Vec3::X * sign);
            }
        }
    }
    if direction.y.abs() > f32::EPSILON {
        for sign in [-1.0_f32, 1.0] {
            let candidate = (sign * (target_extents.y + radius) - start.y) / direction.y;
            let point = start + direction * candidate;
            if point.x.abs() <= target_extents.x && point.z.abs() <= combined_height {
                consider(candidate, Vec3::Y * sign);
            }
        }
    }

    let horizontal_a = direction.x.mul_add(direction.x, direction.y * direction.y);
    if horizontal_a > f32::EPSILON {
        for x_sign in [-1.0_f32, 1.0] {
            for y_sign in [-1.0_f32, 1.0] {
                let corner = Vec3::new(x_sign * target_extents.x, y_sign * target_extents.y, 0.0);
                let local = start - corner;
                let horizontal_b = 2.0 * local.x.mul_add(direction.x, local.y * direction.y);
                let horizontal_c = local.x.mul_add(local.x, local.y * local.y) - radius * radius;
                let discriminant =
                    horizontal_b.mul_add(horizontal_b, -4.0 * horizontal_a * horizontal_c);
                if discriminant < 0.0 {
                    continue;
                }
                let root = discriminant.sqrt();
                for candidate in [
                    (-horizontal_b - root) / (2.0 * horizontal_a),
                    (-horizontal_b + root) / (2.0 * horizontal_a),
                ] {
                    let point = start + direction * candidate;
                    if point.x * x_sign >= target_extents.x
                        && point.y * y_sign >= target_extents.y
                        && point.z.abs() <= combined_height
                    {
                        let radial = (point - corner).truncate().normalize_or_zero();
                        consider(candidate, radial.extend(0.0));
                    }
                }
            }
        }
    }

    if direction.z.abs() > f32::EPSILON {
        for sign in [-1.0_f32, 1.0] {
            let candidate = (sign * combined_height - start.z) / direction.z;
            let point = start + direction * candidate;
            let closest = point
                .truncate()
                .clamp(-target_extents.truncate(), target_extents.truncate());
            if point.truncate().distance_squared(closest) <= radius * radius {
                consider(candidate, Vec3::Z * sign);
            }
        }
    }
    if nearest >= trace_distance {
        return None;
    }
    Some(ActorCollisionHit {
        fraction: ((nearest - TRACE_MARGIN).max(0.0) / distance).min(1.0),
        normal,
    })
}

pub fn boxes_overlap(
    first: Vec3,
    first_extents: Vec3,
    second: Vec3,
    second_extents: Vec3,
    second_rotation: Mat3,
) -> bool {
    if !first.is_finite()
        || !first_extents.is_finite()
        || !second.is_finite()
        || !second_extents.is_finite()
        || !second_rotation.is_finite()
        || first_extents.min_element() < 0.0
        || second_extents.min_element() < 0.0
    {
        return false;
    }
    let world_to_second = second_rotation.transpose();
    let local_first = world_to_second * (first - second);
    let extents = second_extents + world_to_second.abs() * first_extents;
    local_first.abs().cmplt(extents).all()
}

fn hull_is_axis_aligned_box(hull: &ConvexHull) -> bool {
    if hull.planes.len() != 6 {
        return false;
    }
    let mut axes = 0_u8;
    for plane in &hull.planes {
        let normal = plane.normal;
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

pub fn cylinders_overlap(
    first: Vec3,
    first_height: f32,
    first_radius: f32,
    second: Vec3,
    second_height: f32,
    second_radius: f32,
) -> bool {
    if !first.is_finite()
        || !second.is_finite()
        || ![first_height, first_radius, second_height, second_radius]
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0)
    {
        return false;
    }
    let delta = first - second;
    let height = first_height + second_height;
    let radius = first_radius + second_radius;
    delta.z.abs() < height && delta.x.mul_add(delta.x, delta.y * delta.y) < radius * radius
}

struct SweepCursor {
    start: Vec3,
    end: Vec3,
    shape: SweepShape,
    start_fraction: f32,
    end_fraction: f32,
    hit_normal: Vec3,
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
            no_hit: false,
        }
    }

    fn clip_plane(&mut self, plane: Plane) -> bool {
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
    use super::*;
    use openhp1_map::{BspNode, Model, PrimitiveBounds};

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

    #[test]
    fn cylinder_sweep_distinguishes_side_caps_and_misses() {
        let side = sweep_cylinder(
            Vec3::new(-20.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
            2.0,
            2.0,
            Vec3::ZERO,
            3.0,
            3.0,
        )
        .unwrap();
        assert!((side.fraction - 0.35).abs() < 0.0001);
        assert_eq!(side.normal, Vec3::NEG_X);

        let cap = sweep_cylinder(
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::new(0.0, 0.0, -20.0),
            2.0,
            2.0,
            Vec3::ZERO,
            3.0,
            3.0,
        )
        .unwrap();
        assert!((cap.fraction - 0.35).abs() < 0.0001);
        assert_eq!(cap.normal, Vec3::Z);

        assert!(
            sweep_cylinder(
                Vec3::new(-20.0, 20.0, 0.0),
                Vec3::new(20.0, 20.0, 0.0),
                2.0,
                2.0,
                Vec3::ZERO,
                3.0,
                3.0,
            )
            .is_none()
        );
    }

    #[test]
    fn cylinder_sweep_allows_movement_out_of_an_existing_overlap() {
        assert!(sweep_cylinder(Vec3::ZERO, Vec3::X * 10.0, 2.0, 2.0, Vec3::X, 2.0, 2.0,).is_none());
    }

    #[test]
    fn cylinder_overlap_uses_ue1_strict_boundaries() {
        assert!(cylinders_overlap(
            Vec3::ZERO,
            2.0,
            2.0,
            Vec3::new(3.0, 0.0, 0.0),
            2.0,
            2.0,
        ));
        assert!(!cylinders_overlap(
            Vec3::ZERO,
            2.0,
            2.0,
            Vec3::new(4.0, 0.0, 0.0),
            2.0,
            2.0,
        ));
        assert!(!cylinders_overlap(
            Vec3::ZERO,
            2.0,
            2.0,
            Vec3::new(0.0, 0.0, 4.0),
            2.0,
            2.0,
        ));
    }

    #[test]
    fn box_sweep_uses_collision_width_and_rotation() {
        let rotation = Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
        assert!(
            sweep_box(
                Vec3::new(20.0, -20.0, 0.0),
                Vec3::new(20.0, 20.0, 0.0),
                Vec3::ONE,
                Vec3::ZERO,
                Vec3::new(10.0, 2.0, 3.0),
                rotation,
            )
            .is_none()
        );
        let hit = sweep_box(
            Vec3::new(-20.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::ONE,
            Vec3::ZERO,
            Vec3::new(10.0, 2.0, 3.0),
            rotation,
        )
        .unwrap();
        assert!((hit.fraction - 0.4).abs() < 0.0001);
        assert!(hit.normal.abs_diff_eq(Vec3::NEG_X, 0.000001));
        assert!(boxes_overlap(
            Vec3::X * 2.0,
            Vec3::ONE,
            Vec3::ZERO,
            Vec3::new(10.0, 2.0, 3.0),
            rotation,
        ));
    }

    #[test]
    fn cylinder_sweep_rounds_horizontal_box_corners() {
        let hit = sweep_cylinder_aabb(
            Vec3::new(-200.0, -1188.0, -293.0),
            Vec3::new(-550.0, -1188.0, -293.0),
            42.0,
            22.0,
            Vec3::new(-304.0, -1544.0, -272.0),
            Vec3::new(16.0, 344.0, 64.0),
        )
        .unwrap();

        assert!(hit.normal.x > 0.0);
        assert!(hit.normal.y > 0.0);
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
