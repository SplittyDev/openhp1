use glam::{Mat3, Vec3};

use super::*;

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
