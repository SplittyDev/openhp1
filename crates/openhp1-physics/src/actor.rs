use glam::{Mat3, Vec3};

const TRACE_MARGIN: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorCollisionHit {
    pub fraction: f32,
    pub normal: Vec3,
}

#[cfg(test)]
#[path = "actor_tests.rs"]
mod tests;

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

pub(super) fn sweep_cylinder_aabb(
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
