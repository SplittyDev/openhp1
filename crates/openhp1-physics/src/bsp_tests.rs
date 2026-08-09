use glam::{Mat3, Vec3};
use openhp1_map::{BspNode, BspSurface, BspVertex, Model, PolyFlags, PrimitiveBounds};
use openhp1_package::ObjectReference;

use super::*;

#[test]
fn decodes_and_sweeps_serialized_leaf_hull() {
    let mut model = empty_model();
    model.surfaces.push(BspSurface {
        texture: ObjectReference::Export(3),
        poly_flags: PolyFlags::HIGH_LEDGE,
        base_point: 0,
        normal: 0,
        texture_u: 0,
        texture_v: 0,
        light_map: -1,
        brush_poly: -1,
        pan_u: 0,
        pan_v: 0,
        brush_actor: ObjectReference::None,
    });
    model.surfaces.push(BspSurface {
        poly_flags: PolyFlags::default(),
        ..model.surfaces[0].clone()
    });
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
            surface: if index == 0 { 0 } else { 1 },
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
    assert!(collision.overlaps_cylinder(Vec3::ZERO, 1.0, 1.0));
    let hit = collision
        .sweep_aabb(Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO, Vec3::ONE)
        .unwrap();

    assert_eq!(collision.hull_count(), 1);
    assert!(collision.node_has_poly_flag(0, PolyFlags::HIGH_LEDGE));
    assert!(collision.overlaps_aabb(Vec3::ZERO, Vec3::ONE));
    assert!(collision.overlaps_cylinder(Vec3::ZERO, 1.0, 1.0));
    assert!(!collision.overlaps_aabb(Vec3::splat(20.0), Vec3::ONE));
    assert!(
        hit.fraction > 0.35 && hit.fraction < 0.45,
        "{}",
        hit.fraction
    );
    assert_eq!(hit.normal, Vec3::X);
    assert_eq!(hit.node, 0);
    assert_eq!(
        collision.surface_hit(hit.node),
        Some(SurfaceHit {
            texture: ObjectReference::Export(3),
            poly_flags: PolyFlags::HIGH_LEDGE,
        })
    );
    let opposite = collision
        .sweep_aabb(Vec3::new(-20.0, 0.0, 0.0), Vec3::ZERO, Vec3::ONE)
        .unwrap();
    assert_eq!(opposite.node, 1);
    assert!(!collision.node_has_poly_flag(opposite.node, PolyFlags::HIGH_LEDGE));

    let rotation = Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let hit = collision
        .sweep_transformed_aabb(
            Vec3::new(100.0, 20.0, 0.0),
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::ONE,
            Vec3::new(100.0, 0.0, 0.0),
            rotation,
            Vec3::ZERO,
            Vec3::ONE,
        )
        .unwrap();
    assert!(hit.fraction > 0.35 && hit.fraction < 0.45);
    assert!(hit.normal.abs_diff_eq(Vec3::Y, 1.0e-6));
    let corner = collision
        .sweep_transformed_cylinder(
            Vec3::new(13.0, 11.5, 0.0),
            Vec3::new(0.0, 11.5, 0.0),
            2.0,
            1.0,
            Vec3::ZERO,
            Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::ZERO,
            Vec3::ONE,
        )
        .unwrap();
    assert!(corner.normal.abs_diff_eq(Vec3::new(0.6, 0.8, 0.0), 1.0e-4));
    assert_eq!(
        collision.transformed_bounds(Vec3::new(100.0, 0.0, 0.0), rotation, Vec3::ZERO, Vec3::ONE,),
        Some((Vec3::new(100.0, 0.0, 0.0), Vec3::splat(10.1)))
    );
    assert!(collision.overlaps_transformed_aabb(
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::splat(0.2),
        Vec3::new(100.0, 0.0, 0.0),
        rotation,
        Vec3::ZERO,
        Vec3::ONE,
    ));
}

#[test]
fn static_aabb_check_separates_a_slanted_hull_on_a_box_axis() {
    let mut model = empty_model();
    let planes = [
        [1.0, 1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, 1.0],
        [-1.0, 1.0, 0.0, 1.0],
        [-1.0, -1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, -1.0, 1.0],
    ];
    model.nodes = planes
        .into_iter()
        .enumerate()
        .map(|(index, plane)| BspNode {
            plane,
            zone_mask: 0,
            flags: 0,
            vertex_pool: 0,
            surface: -1,
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
        [-1.0_f32, -1.0, -1.0, 1.0, 1.0, 1.0]
            .map(f32::to_bits)
            .map(|value| value as i32),
    );

    let collision = BspCollision::from_model(&model).unwrap();
    assert!(collision.overlaps_aabb(Vec3::ZERO, Vec3::splat(0.2)));
    // Every slanted face projection overlaps this box; its x interval is wholly outside the hull.
    assert!(!collision.overlaps_aabb(Vec3::new(1.3, 0.0, 0.0), Vec3::splat(0.2)));
}

#[test]
fn box_sweep_ignores_hulls_in_unreached_bsp_leaves() {
    let mut model = empty_model();
    model.nodes = vec![
        BspNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            front: 1,
            back: 2,
            ..empty_node()
        },
        empty_node(),
    ];
    for plane in [
        [1.0, 0.0, 0.0, 10.0],
        [-1.0, 0.0, 0.0, -8.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, -1.0, 1.0],
    ] {
        model.nodes.push(BspNode {
            plane,
            collision_bound: if model.nodes.len() == 2 { 0 } else { -1 },
            ..empty_node()
        });
    }
    model.leaf_hulls = vec![2, 3, 4, 5, 6, 7, -1];
    model.leaf_hulls.extend(
        [8.0_f32, -1.0, -1.0, 10.0, 1.0, 1.0]
            .map(f32::to_bits)
            .map(|value| value as i32),
    );

    let collision = BspCollision::from_model(&model).unwrap();

    assert!(
        collision
            .sweep_aabb(
                Vec3::new(12.0, 0.0, 0.0),
                Vec3::new(8.0, 0.0, 0.0),
                Vec3::ONE
            )
            .is_none()
    );
}

#[test]
fn point_trace_hits_bsp_polygons_from_both_sides() {
    let mut model = empty_model();
    model.points = vec![
        Vec3::new(0.0, -10.0, -10.0),
        Vec3::new(0.0, 10.0, -10.0),
        Vec3::new(0.0, 10.0, 10.0),
        Vec3::new(0.0, -10.0, 10.0),
    ];
    model.vertices = (0..4).map(|point| BspVertex { point, side: -1 }).collect();
    model.surfaces.push(BspSurface {
        texture: ObjectReference::None,
        poly_flags: PolyFlags::default(),
        base_point: 0,
        normal: 0,
        texture_u: 0,
        texture_v: 0,
        light_map: -1,
        brush_poly: -1,
        pan_u: 0,
        pan_v: 0,
        brush_actor: ObjectReference::None,
    });
    model.nodes.push(BspNode {
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
        zones: [0; 2],
        vertex_count: 4,
        leaves: [0; 2],
    });

    let collision = BspCollision::from_model(&model).unwrap();
    let front = collision
        .sweep_aabb(Vec3::X * 10.0, Vec3::NEG_X * 10.0, Vec3::ZERO)
        .unwrap();
    let back = collision
        .sweep_aabb(Vec3::NEG_X * 10.0, Vec3::X * 10.0, Vec3::ZERO)
        .unwrap();

    assert!((front.fraction - 0.45).abs() < 0.0001);
    assert_eq!(front.normal, Vec3::X);
    assert!((back.fraction - 0.45).abs() < 0.0001);
    assert_eq!(back.normal, Vec3::NEG_X);
}

fn empty_node() -> BspNode {
    BspNode {
        plane: [0.0; 4],
        zone_mask: 0,
        flags: 0,
        vertex_pool: 0,
        surface: -1,
        back: -1,
        front: -1,
        coplanar: -1,
        collision_bound: -1,
        render_bound: -1,
        zones: [0; 2],
        vertex_count: 0,
        leaves: [0; 2],
    }
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
        root_outside: false,
        linked: false,
    }
}
