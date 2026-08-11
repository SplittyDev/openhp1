use super::*;

pub(super) fn spline_weight(value: f32) -> f32 {
    let squared = value * value;
    squared * squared * (1.0 / 16.0) - squared * 0.5 + 1.0
}

pub(super) fn move_to_direction(
    physics: u8,
    mut delta: Vec3,
    velocity: Vec3,
    timer: f32,
) -> Option<Vec3> {
    if timer < 0.0 {
        return None;
    }
    if physics == PHYS_WALKING {
        delta.z = 0.0;
    }
    let distance_squared = delta.length_squared();
    if distance_squared < 1.0 || distance_squared < velocity.length_squared() * 0.05 {
        None
    } else {
        Some(delta.normalize())
    }
}

pub(super) fn has_horizontal_movement(velocity: Vec3) -> bool {
    velocity.x != 0.0 || velocity.y != 0.0
}

pub(super) fn has_movement(velocity: Vec3) -> bool {
    velocity != Vec3::ZERO
}

pub(super) fn spline_weights(alpha: f32) -> [f32; 4] {
    let weights = [
        spline_weight(alpha + 1.0),
        spline_weight(alpha),
        spline_weight(alpha - 1.0),
        spline_weight(alpha - 2.0),
    ];
    let inverse = weights.iter().sum::<f32>().recip();
    weights.map(|weight| weight * inverse)
}

pub(super) fn spline_vector(
    first: Vec3,
    second: Vec3,
    third: Vec3,
    fourth: Vec3,
    alpha: f32,
) -> Vec3 {
    let [first_weight, second_weight, third_weight, fourth_weight] = spline_weights(alpha);
    first * first_weight + second * second_weight + third * third_weight + fourth * fourth_weight
}

pub(super) fn bezier_vector(
    start: Vec3,
    start_control: Vec3,
    end_control: Vec3,
    end: Vec3,
    alpha: f32,
) -> Vec3 {
    let inverse = 1.0 - alpha;
    start * inverse.powi(3)
        + start_control * (3.0 * inverse.powi(2) * alpha)
        + end_control * (3.0 * inverse * alpha.powi(2))
        + end * alpha.powi(3)
}

pub(super) fn bezier_tangent(
    start: Vec3,
    start_control: Vec3,
    end_control: Vec3,
    end: Vec3,
    alpha: f32,
) -> Vec3 {
    let inverse = 1.0 - alpha;
    (start_control - start) * (3.0 * inverse.powi(2))
        + (end_control - start_control) * (6.0 * inverse * alpha)
        + (end - end_control) * (3.0 * alpha.powi(2))
}

pub(super) fn direction_rotator(direction: Vec3) -> [i32; 3] {
    let units = 65_536.0 / std::f32::consts::TAU;
    [
        (direction.z.atan2(direction.x.hypot(direction.y)) * units) as i32,
        (direction.y.atan2(direction.x) * units) as i32,
        0,
    ]
}

pub(super) fn lerp_rotator(first: [i32; 3], second: [i32; 3], alpha: f32) -> [i32; 3] {
    std::array::from_fn(|index| {
        (first[index] as f32 * (1.0 - alpha)) as i32 + (second[index] as f32 * alpha) as i32
    })
}

pub(super) fn spline_rotator(
    first: [i32; 3],
    second: [i32; 3],
    third: [i32; 3],
    fourth: [i32; 3],
    alpha: f32,
) -> [i32; 3] {
    let weights = [
        spline_weight(alpha + 1.0),
        spline_weight(alpha),
        spline_weight(alpha - 1.0),
        spline_weight(alpha - 2.0),
    ];
    let inverse = weights.iter().sum::<f32>().recip();
    std::array::from_fn(|index| {
        let weighted = (first[index] as f32 * weights[0]) as i32
            + (second[index] as f32 * weights[1]) as i32
            + (third[index] as f32 * weights[2]) as i32
            + (fourth[index] as f32 * weights[3]) as i32;
        (weighted as f32 * inverse) as i32
    })
}

pub(super) fn mover_rotation(old: [i32; 3], base: [i32; 3], key: [i32; 3], blend: f32) -> [i32; 3] {
    std::array::from_fn(|index| {
        let delta = base[index]
            .wrapping_add(key[index])
            .wrapping_sub(old[index]);
        old[index].wrapping_add((delta as f32 * blend) as i32)
    })
}

pub(super) fn fall_velocity(
    old_velocity: Vec3,
    acceleration: Vec3,
    gravity: Vec3,
    gravity_scale: f32,
    fluid_friction: f32,
    elapsed: f32,
) -> Vec3 {
    old_velocity * (1.0 - fluid_friction * elapsed)
        + (acceleration * 1.5 + gravity * gravity_scale) * (0.5 * elapsed)
}

pub(super) fn rotators_equal(left: [i32; 3], right: [i32; 3]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left as u16 == right as u16)
}

pub(super) fn clamp_pawn_pitch(desired: i32, rate: i32) -> i32 {
    let desired = desired & 0xffff;
    if desired < 0x8000 {
        desired.min(rate)
    } else if desired < 0x10000 - rate {
        0x10000 - rate
    } else {
        desired
    }
}

pub(super) fn turn_to_shortest(from: i32, to: i32, speed: i32) -> i32 {
    let from = from & 0xffff;
    let to = to & 0xffff;
    if from > to {
        if from - to < 0x8000 {
            (from - (from - to).min(speed)) & 0xffff
        } else {
            (from + (to + 0x10000 - from).min(speed)) & 0xffff
        }
    } else if to - from < 0x8000 {
        (from + (to - from).min(speed)) & 0xffff
    } else {
        (from - (from + 0x10000 - to).min(speed)) & 0xffff
    }
}

pub(super) fn turn_to_fixed(from: i32, to: i32, direction: i32) -> i32 {
    let from = from & 0xffff;
    let to = to & 0xffff;
    if direction > 0 {
        if from > to {
            (from + direction.min(to - from + 0x10000)) & 0xffff
        } else {
            (from + direction.min(to - from)) & 0xffff
        }
    } else if from < to {
        (from + direction.max(to - from - 0x10000)) & 0xffff
    } else {
        (from + direction.max(to - from)) & 0xffff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotator_turning_wraps_like_ue1() {
        assert_eq!(turn_to_shortest(65_000, 100, 200), 65_200);
        assert_eq!(turn_to_shortest(100, 65_000, 200), 65_436);
        assert_eq!(turn_to_fixed(65_000, 100, 1_000), 100);
        assert_eq!(turn_to_fixed(100, 65_000, -1_000), 65_000);
        assert!(rotators_equal([-1, 0, 65_536], [65_535, 0, 0]));
    }

    #[test]
    fn pawn_pitch_uses_ue1_rotation_rate_bounds() {
        assert_eq!(clamp_pawn_pitch(0, 3_072), 0);
        assert_eq!(clamp_pawn_pitch(1_000, 3_072), 1_000);
        assert_eq!(clamp_pawn_pitch(-1_000, 3_072), 64_536);
        assert_eq!(clamp_pawn_pitch(16_384, 3_072), 3_072);
        assert_eq!(clamp_pawn_pitch(-16_384, 3_072), 62_464);
    }

    #[test]
    fn falling_applies_ue1_half_step_gravity_and_fluid_drag() {
        assert_eq!(
            fall_velocity(
                Vec3::new(10.0, 0.0, -10.0),
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, -512.0),
                2.0,
                1.0,
                0.02,
            ),
            Vec3::new(9.8, 0.0, -20.04)
        );
    }

    #[test]
    fn move_to_uses_horizontal_walking_distance_and_ue1_arrival_threshold() {
        assert!(has_horizontal_movement(Vec3::X));
        assert!(has_horizontal_movement(Vec3::Y));
        assert!(!has_horizontal_movement(Vec3::Z));
        assert!(has_movement(Vec3::X));
        assert!(has_movement(Vec3::Y));
        assert!(has_movement(Vec3::Z));
        assert!(!has_movement(Vec3::ZERO));
        assert_eq!(
            move_to_direction(PHYS_WALKING, Vec3::new(3.0, 4.0, 100.0), Vec3::ZERO, 1.0),
            Some(Vec3::new(0.6, 0.8, 0.0))
        );
        assert_eq!(
            move_to_direction(PHYS_WALKING, Vec3::X, Vec3::splat(4.0), 1.0),
            None
        );
        assert_eq!(
            move_to_direction(PHYS_WALKING, Vec3::X * 100.0, Vec3::ZERO, -0.1),
            None
        );
    }

    #[test]
    fn interpolation_spline_matches_ue1_weighting() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        assert!(
            spline_vector(points[0], points[1], points[2], points[3], 0.0)
                .abs_diff_eq(Vec3::new(8.0 / 17.0, 4.5 / 17.0, 0.0), 0.0001)
        );
        assert!(
            spline_vector(points[0], points[1], points[2], points[3], 1.0)
                .abs_diff_eq(Vec3::new(4.5 / 17.0, 8.0 / 17.0, 4.5 / 17.0), 0.0001)
        );
    }

    #[test]
    fn mover_rotation_uses_wrapping_ue1_rotator_math() {
        assert_eq!(
            mover_rotation([100, -100, i32::MAX], [300, 100, i32::MIN], [0; 3], 0.5),
            [200, 0, i32::MAX]
        );
    }
}
