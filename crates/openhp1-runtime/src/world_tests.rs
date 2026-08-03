use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

use openhp1_script::Bytecode;

use crate::{Frame, FunctionCall};

use super::*;
use super::{
    actor::advance_lifespan,
    actor::advance_timer,
    actor::decode_latent_action,
    actor::update_touching_array,
    native::{
        animation_parameters, bone_number, bone_position, collision_updates, log_arguments,
        next_navigation_step, noise_loudness, random_float, random_int, random_unit_vector,
        scalar_native, sound_arguments, target_score, trace_texture,
    },
    state::{event_disabled, probe_event_index, set_event_disabled},
};
use openhp1_map::PrimitiveBounds;

static FIXTURE_ROOT: AtomicUsize = AtomicUsize::new(0);

fn synthetic_runtime_package() -> Vec<u8> {
    const HEADER_SIZE: usize = 44;
    let name_offset = HEADER_SIZE;
    let export_offset = name_offset
        + b"PlayerPawn\0".len()
        + size_of::<u32>()
        + b"ClientTravel\0".len()
        + size_of::<u32>()
        + b"GetPlayerNetworkAddress\0".len()
        + size_of::<u32>();
    let mut bytes = Vec::new();
    bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
    bytes.extend(61_u16.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    for value in [
        3,
        name_offset as i32,
        3,
        export_offset as i32,
        0,
        export_offset as i32,
        0,
        0,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for name in [
        b"PlayerPawn\0".as_slice(),
        b"ClientTravel\0".as_slice(),
        b"GetPlayerNetworkAddress\0".as_slice(),
    ] {
        bytes.extend(name);
        bytes.extend(0_u32.to_le_bytes());
    }
    for (outer, name) in [(0_i32, 0_u8), (1, 1), (1, 2)] {
        bytes.extend([0, 0]);
        bytes.extend(outer.to_le_bytes());
        bytes.push(name);
        bytes.extend(0_u32.to_le_bytes());
        bytes.push(0);
    }
    bytes
}

#[test]
fn looping_timer_keeps_fractional_overshoot() {
    let mut timer = ActorTimer {
        remaining: 0.1,
        rate: 0.1,
        looping: true,
    };
    assert!(advance_timer(&mut timer, 0.15));
    assert!((timer.remaining - 0.05).abs() < 1.0e-6);
    assert!(!advance_timer(&mut timer, 0.04));
    assert!(advance_timer(&mut timer, 0.01));
    assert!((timer.remaining - 0.1).abs() < 1.0e-6);
}

#[test]
fn positive_lifespans_expire_once_at_zero() {
    let mut lifespan = 0.1;
    assert!(!advance_lifespan(&mut lifespan, 0.04));
    assert!(advance_lifespan(&mut lifespan, 0.07));
    assert_eq!(lifespan, 0.0);
    assert!(!advance_lifespan(&mut lifespan, 0.1));
}

#[test]
fn particle_acceleration_uses_negative_zone_modifier_and_level_fallback() {
    let collision = BspCollision::from_model(&Model {
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
    })
    .unwrap();
    assert_eq!(
        zone_actor_at(&collision, Vec3::ZERO, None, &HashMap::default(), Some(4),),
        Some(4),
    );
    assert_eq!(
        particle_acceleration(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -100.0), -0.5,),
        Vec3::new(1.0, 2.0, 53.0),
    );
}

#[test]
fn decodes_finish_interpolation_latent_state() {
    assert_eq!(
        decode_latent_action(0x12e, 7),
        LatentAction::FinishInterpolation(7)
    );
}

#[test]
fn player_can_see_me_native_skips_the_active_pawn_and_accepts_coincidence() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-player-can-see-me-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let class_id = object_id(&package, 0);
    let level = runtime_actor_id(10);
    let current = runtime_actor_id(11);
    let other = runtime_actor_id(12);
    let level_field = runtime_actor_id(20);
    let pawn_list_field = runtime_actor_id(21);
    let next_pawn_field = runtime_actor_id(22);
    let location_field = runtime_actor_id(23);
    let view_rotation_field = runtime_actor_id(24);
    let behind_view_field = runtime_actor_id(25);
    let base_eye_height_field = runtime_actor_id(26);
    for (name, field) in [
        ("Level", &level_field),
        ("PawnList", &pawn_list_field),
        ("nextPawn", &next_pawn_field),
        ("Location", &location_field),
        ("ViewRotation", &view_rotation_field),
        ("bBehindView", &behind_view_field),
        ("BaseEyeHeight", &base_eye_height_field),
    ] {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    for (actor, object) in [(0, &level), (1, &current), (2, &other)] {
        runtime.actor_classes.insert(actor, class_id.clone());
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object.clone());
    }

    let mut level_instance = InstanceState::default();
    level_instance.insert(pawn_list_field, StoredValue::Object(Some(current.clone())));
    runtime.instances.insert(0, level_instance);
    let mut other_instance = InstanceState::default();
    other_instance.insert(next_pawn_field.clone(), StoredValue::Object(None));
    other_instance.insert(
        location_field.clone(),
        StoredValue::Value(Value::Vector([0.0, 0.0, 0.0])),
    );
    other_instance.insert(
        view_rotation_field,
        StoredValue::Value(Value::Rotator([0, 0, 0])),
    );
    other_instance.insert(behind_view_field, StoredValue::Value(Value::Bool(false)));
    other_instance.insert(base_eye_height_field, StoredValue::Value(Value::Float(0.0)));
    runtime.instances.insert(2, other_instance);
    let mut current_instance = InstanceState::default();
    current_instance.insert(level_field, StoredValue::Object(Some(level)));
    current_instance.insert(next_pawn_field, StoredValue::Object(Some(other)));
    current_instance.insert(
        location_field,
        StoredValue::Value(Value::Vector([0.0, 0.0, 0.0])),
    );

    assert_eq!(
        runtime
            .native(
                1,
                &class,
                &package,
                0x214,
                &[],
                &mut current_instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(true),
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn line_of_sight_to_dispatches_numeric_native() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-line-of-sight-to-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let class_id = object_id(&package, 0);
    let fields = [
        "Location",
        "CollisionHeight",
        "CollisionRadius",
        "CollisionWidth",
        "Rotation",
        "CollideType",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
        "Brush",
        "PrePivot",
        "BaseEyeHeight",
        "SightRadius",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        (
            name,
            ObjectId {
                package: Arc::from("<line-of-sight-to-test>"),
                export_index: index,
            },
        )
    })
    .collect::<HashMap<_, _>>();
    for (name, field) in &fields {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    runtime
        .fields
        .insert((class_id.clone(), "mainscale".to_owned()), None);
    let actor = runtime_actor_id(1);
    let other = runtime_actor_id(2);
    for (index, object) in [(1, actor), (2, other.clone())] {
        runtime.actor_classes.insert(index, class_id.clone());
        runtime.object_actors.insert(object.clone(), index);
        runtime.actor_objects.insert(index, object);
    }
    runtime.next_actor = 3;
    let mut instance = InstanceState::default();
    instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    instance.insert(
        fields["BaseEyeHeight"].clone(),
        StoredValue::Value(Value::Float(20.0)),
    );
    instance.insert(
        fields["SightRadius"].clone(),
        StoredValue::Value(Value::Float(100.0)),
    );
    runtime.instances.insert(
        2,
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([50.0, 0.0, 0.0])),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(40.0)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let other = runtime.object_handle(other).unwrap();

    assert_eq!(
        runtime
            .native(
                1,
                &class,
                &package,
                0x202,
                &[Value::Object(other)],
                &mut instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(true),
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn touch_events_keep_the_engine_touching_array_in_sync() {
    let first = runtime_actor_id(1);
    let second = runtime_actor_id(2);
    let mut values = vec![
        StoredValue::Object(None),
        StoredValue::Object(None),
        StoredValue::Object(None),
        StoredValue::Object(None),
    ];

    update_touching_array(&mut values, first.clone(), true);
    update_touching_array(&mut values, first.clone(), true);
    update_touching_array(&mut values, second.clone(), true);
    assert!(matches!(
        &values[..],
        [
            StoredValue::Object(Some(value)),
            StoredValue::Object(Some(other)),
            StoredValue::Object(None),
            StoredValue::Object(None),
        ] if value == &first && other == &second
    ));

    update_touching_array(&mut values, first, false);
    assert!(matches!(values[0], StoredValue::Object(None)));
}

#[test]
fn integer_division_is_checked() {
    assert_eq!(
        scalar_native(0x91, &[Value::Int(7), Value::Int(2)]),
        Ok(Value::Int(3))
    );
    assert!(scalar_native(0x91, &[Value::Int(1), Value::Int(0)]).is_err());
    assert_eq!(
        scalar_native(0x9c, &[Value::Int(0x1_ffff), Value::Int(0xffff)]),
        Ok(Value::Int(0xffff))
    );
}

#[test]
fn integer_left_shift_executes_through_native_dispatch_with_masked_count() {
    let execute = |left: i32, right: i32| {
        let mut bytes = vec![0x04, 0x94, 0x1d];
        bytes.extend(left.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(right.to_le_bytes());
        bytes.push(0x16);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame
            .execute(|call, arguments| {
                assert_eq!(call, FunctionCall::Native(0x94));
                scalar_native(0x94, arguments)
            })
            .unwrap()
    };

    assert_eq!(execute(1, 31), Value::Int(i32::MIN));
    assert_eq!(execute(1, 32), Value::Int(1));
    assert_eq!(execute(1, -1), Value::Int(i32::MIN));
}

#[test]
fn integer_right_shift_executes_through_native_dispatch_with_sign_extension() {
    let execute = |left: i32, right: i32| {
        let mut bytes = vec![0x04, 0x95, 0x1d];
        bytes.extend(left.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(right.to_le_bytes());
        bytes.push(0x16);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame
            .execute(|call, arguments| {
                assert_eq!(call, FunctionCall::Native(0x95));
                scalar_native(0x95, arguments)
            })
            .unwrap()
    };

    assert_eq!(execute(i32::MIN, 31), Value::Int(-1));
    assert_eq!(execute(i32::MIN, 32), Value::Int(i32::MIN));
    assert_eq!(execute(-1, -1), Value::Int(-1));
}

#[test]
fn float_remainder_uses_unreal_dividend_sign() {
    assert_eq!(
        scalar_native(0xad, &[Value::Float(-7.5), Value::Float(2.0)]),
        Ok(Value::Float(-1.5))
    );
}

#[test]
fn tangent_uses_radians() {
    let Value::Float(value) =
        scalar_native(0xbd, &[Value::Float(std::f32::consts::FRAC_PI_4)]).unwrap()
    else {
        panic!("expected float");
    };
    assert!((value - 1.0).abs() < 1.0e-6);
}

#[test]
fn sine_uses_radians() {
    let Value::Float(value) =
        scalar_native(0xbb, &[Value::Float(std::f32::consts::FRAC_PI_2)]).unwrap()
    else {
        panic!("expected float");
    };
    assert!((value - 1.0).abs() < 1.0e-6);
}

#[test]
fn cosine_dispatches_from_bytecode_through_runtime_native() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-cos-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let mut bytes = vec![0x04, 0xbc, 0x1e];
    bytes.extend(std::f32::consts::PI.to_le_bytes());
    bytes.push(0x16);
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();
    let Value::Float(value) = frame
        .execute(|call, arguments| {
            let FunctionCall::Native(index) = call else {
                unreachable!()
            };
            runtime.native(
                0,
                &class,
                &package,
                index,
                arguments,
                &mut instance,
                &mut actions,
                0,
            )
        })
        .unwrap()
    else {
        panic!("expected float");
    };
    assert!((value + 1.0).abs() < 1.0e-6);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vector_vector_multiply_dispatches_from_extended_bytecode_through_runtime_native() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-vector-multiply-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let mut bytes = vec![0x04, 0x61, 0x28];
    for local in [7_i32, 8] {
        bytes.push(0x00);
        bytes.extend(local.to_le_bytes());
    }
    bytes.push(0x16);
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    frame.set_local(7, Value::Vector([2.0, -3.0, 0.5]));
    frame.set_local(8, Value::Vector([4.0, 5.0, -6.0]));
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();
    assert_eq!(
        frame
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                runtime.native(
                    0,
                    &class,
                    &package,
                    index,
                    arguments,
                    &mut instance,
                    &mut actions,
                    0,
                )
            })
            .unwrap(),
        Value::Vector([8.0, -15.0, -3.0])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bone_numbers_follow_case_insensitive_skeletal_order() {
    let bones = vec!["Root".to_owned(), "Head".to_owned()];
    assert_eq!(bone_number(Some(&bones), "head"), 1);
    assert_eq!(bone_number(Some(&bones), "missing"), 0);
}

#[test]
fn bone_positions_use_the_current_case_insensitive_skeletal_pose() {
    let bones = vec!["Root".to_owned(), "Head".to_owned()];
    let positions = [[10.0, 20.0, 30.0], [40.0, 50.0, 60.0]];
    assert_eq!(
        bone_position(Some(&bones), Some(&positions), "head"),
        [40.0, 50.0, 60.0]
    );
    assert_eq!(
        bone_position(Some(&bones), Some(&positions), "missing"),
        [0.0; 3]
    );
}

#[test]
fn scalar_natives_distinguish_bad_operands_from_unknown_indices() {
    assert_eq!(
        scalar_native(0x97, &[Value::Byte(1), Value::Int(0)]),
        Err("Greater_IntInt does not accept operands (byte, int)".to_owned())
    );
    assert_eq!(
        scalar_native(0xffff, &[]),
        Err("native 0xffff is not implemented".to_owned())
    );
}

#[test]
fn client_travel_named_native_dispatches_through_function_execution() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-client-travel-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let function = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 1,
    };
    runtime.scripts.insert(
        object_id(&package, 1),
        Arc::new(openhp1_script::ScriptExport {
            export_index: 1,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: openhp1_script::Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );
    assert_eq!(
        runtime
            .execute_actor_function(
                17,
                &class,
                &function,
                &[
                    Value::String("Lev2_HogFront?entry".to_owned()),
                    Value::Byte(2),
                    Value::Bool(true),
                ],
            )
            .unwrap(),
        [ActorAction::ClientTravel {
            actor: 17,
            url: "Lev2_HogFront?entry".to_owned(),
            travel_type: 2,
            transfer_items: true,
        }]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn get_player_network_address_named_native_dispatches_through_function_execution() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-get-player-network-address-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let function = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 2,
    };
    runtime.scripts.insert(
        object_id(&package, 2),
        Arc::new(openhp1_script::ScriptExport {
            export_index: 2,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 2,
            line: 0,
            text_position: 0,
            bytecode: openhp1_script::Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );

    assert_eq!(
        runtime
            .execute_function(
                17,
                &class,
                &function,
                &[],
                &mut InstanceState::default(),
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::String(String::new()),
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn named_native_shims_validate_their_engine_calls() {
    let mut actions = Vec::new();
    assert_eq!(
        execution::named_native(
            17,
            "PlayerPawn",
            "ConsoleCommand",
            &[Value::String("GETPING".to_owned())],
            &mut actions,
        ),
        Some(Value::String(String::new()))
    );
    assert_eq!(
        execution::named_native(17, "Decal", "DetachDecal", &[], &mut actions),
        Some(Value::None)
    );
    assert_eq!(
        execution::named_native(17, "PlayerPawn", "ConsoleCommand", &[], &mut actions),
        None
    );
}

#[test]
fn scalar_comparisons_cover_bool_int_and_float_families() {
    assert_eq!(
        scalar_native(0x98, &[Value::None, Value::Int(0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x83, &[Value::Bool(true), Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf2, &[Value::Bool(true), Value::Bool(true)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf3, &[Value::Bool(true), Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf2, &[Value::None, Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x98, &[Value::Int(2), Value::Int(2)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x99, &[Value::Int(3), Value::Int(2)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb2, &[Value::Float(2.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb3, &[Value::Float(3.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb4, &[Value::Float(2.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb5, &[Value::Float(f32::NAN), Value::Float(f32::NAN)]),
        Ok(Value::Bool(true))
    );
}

#[test]
fn pick_target_score_rejects_targets_behind_or_beyond_range() {
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(100.0, 0.0, 0.0),
            0.5
        ),
        Some((1.0, 100.0))
    );
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(-100.0, 0.0, 0.0),
            0.0
        ),
        None
    );
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(2_501.0, 0.0, 0.0),
            0.0
        ),
        None
    );
}

#[test]
fn observed_string_natives_match_unreal_semantics() {
    assert_eq!(
        scalar_native(
            0x70,
            &[
                Value::String("Harry".to_owned()),
                Value::String(" Potter".to_owned())
            ]
        ),
        Ok(Value::String("Harry Potter".to_owned()))
    );
    assert_eq!(
        scalar_native(
            0xa8,
            &[
                Value::String("Hello".to_owned()),
                Value::String("world".to_owned())
            ]
        ),
        Ok(Value::String("Hello world".to_owned()))
    );
    assert_eq!(
        scalar_native(
            0x7f,
            &[
                Value::String("Hogwarts".to_owned()),
                Value::Int(3),
                Value::Int(4)
            ]
        ),
        Ok(Value::String("wart".to_owned()))
    );
    assert_eq!(
        scalar_native(0xea, &[Value::String("Hogwarts".to_owned()), Value::Int(4)]),
        Ok(Value::String("arts".to_owned()))
    );
    assert_eq!(
        scalar_native(0xec, &[Value::Int(0x141)]),
        Ok(Value::String("A".to_owned()))
    );
    assert_eq!(
        scalar_native(0xed, &[Value::String("Alohomora".to_owned())]),
        Ok(Value::Int(65))
    );
}

#[test]
fn collision_updates_preserve_omitted_flags() {
    assert_eq!(
        collision_updates(&[Value::Bool(true), Value::None]),
        Ok([Some(true), None, None])
    );
    assert!(collision_updates(&[Value::Float(1.0)]).is_err());
}

#[test]
fn log_arguments_preserve_optional_tags() {
    assert_eq!(
        log_arguments(&[Value::String("hello".to_owned()), Value::Name(7)]),
        Ok(("hello", Some(&Value::Name(7))))
    );
    assert!(log_arguments(&[Value::Int(1)]).is_err());
    assert!(log_arguments(&[Value::String("hello".to_owned()), Value::None, Value::None]).is_err());
}

#[test]
fn noise_loudness_must_be_a_finite_float() {
    assert_eq!(noise_loudness(&[Value::Float(0.5)]), Ok(0.5));
    assert!(noise_loudness(&[Value::Float(f32::NAN)]).is_err());
    assert!(noise_loudness(&[]).is_err());
}

#[test]
fn landing_surface_and_sound_natives_validate_calls() {
    assert_eq!(
        trace_texture(&[
            Value::Vector([0.0; 3]),
            Value::Vector([0.0, 0.0, -16.0]),
            Value::Int(0),
            Value::Bool(false),
        ]),
        Ok(Value::Object(0))
    );
    assert!(trace_texture(&[Value::Int(0)]).is_err());
    assert!(
        sound_arguments(
            "PlayOwnedSound",
            &[
                Value::Object(1),
                Value::Byte(0),
                Value::Float(1.0),
                Value::Bool(false),
            ],
        )
        .is_ok()
    );
    assert!(sound_arguments("PlayOwnedSound", &[Value::Int(1)]).is_err());
}

#[test]
fn disabled_events_are_case_insensitive_and_scoped_to_actor_state() {
    let mut disabled = HashMap::default();
    set_event_disabled(&mut disabled, 2, Some("Beano"), "Tick", true);

    assert!(event_disabled(&disabled, 2, Some("beano"), "TICK"));
    assert!(!event_disabled(&disabled, 2, Some("KillBean"), "Tick"));
    assert!(!event_disabled(&disabled, 3, Some("Beano"), "Tick"));

    set_event_disabled(&mut disabled, 2, Some("BEANO"), "tick", false);
    assert!(!event_disabled(&disabled, 2, Some("Beano"), "Tick"));
    assert_eq!(probe_event_index("tick"), Some(36));
}

#[test]
fn float_min_max_match_unreal_native_ordering() {
    assert_eq!(
        scalar_native(0xf4, &[Value::Float(2.0), Value::Float(3.0)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(0xf4, &[Value::Float(2.0), Value::Float(f32::NAN)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(0xf5, &[Value::Float(2.0), Value::Float(3.0)]),
        Ok(Value::Float(3.0))
    );
    assert_eq!(
        scalar_native(0xf5, &[Value::Float(2.0), Value::Float(f32::NAN)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(
            0xf6,
            &[Value::Float(5.0), Value::Float(1.0), Value::Float(3.0)]
        ),
        Ok(Value::Float(3.0))
    );
}

#[test]
fn basic_vector_arithmetic_matches_unreal_natives() {
    assert_eq!(scalar_native(0x8f, &[Value::Int(7)]), Ok(Value::Int(-7)));
    assert_eq!(
        scalar_native(
            0xd9,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([1.0, 2.0, 3.0])
            ]
        ),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(
            0xd7,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([4.0, 5.0, 6.0])
            ]
        ),
        Ok(Value::Vector([5.0, 7.0, 9.0]))
    );
    assert_eq!(
        scalar_native(0xd4, &[Value::Vector([1.0, 2.0, 3.0]), Value::Float(2.0)]),
        Ok(Value::Vector([2.0, 4.0, 6.0]))
    );
    assert_eq!(
        scalar_native(
            0xdb,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([4.0, 5.0, 6.0])
            ]
        ),
        Ok(Value::Float(32.0))
    );
    let quarter_yaw = Value::Rotator([0, 16_384, 0]);
    let Value::Vector(rotated) = scalar_native(
        0x114,
        &[Value::Vector([1.0, 0.0, 0.0]), quarter_yaw.clone()],
    )
    .unwrap() else {
        panic!("expected vector rotation");
    };
    assert!(glam::Vec3::from_array(rotated).abs_diff_eq(glam::Vec3::Y, 1.0e-6));
    let Value::Vector(unrotated) =
        scalar_native(0x113, &[Value::Vector(rotated), quarter_yaw]).unwrap()
    else {
        panic!("expected inverse vector rotation");
    };
    assert!(glam::Vec3::from_array(unrotated).abs_diff_eq(glam::Vec3::X, 1.0e-6));
    assert_eq!(
        scalar_native(
            0x12c,
            &[
                Value::Vector([1.0, -2.0, 3.0]),
                Value::Vector([0.0, 1.0, 0.0])
            ],
        ),
        Ok(Value::Vector([1.0, 2.0, 3.0]))
    );
}

#[test]
fn navigation_uses_the_shortest_unpruned_reachable_step() {
    let start = runtime_actor_id(1);
    let short = runtime_actor_id(2);
    let long = runtime_actor_id(3);
    let target = runtime_actor_id(4);
    let spec = |start, end, distance| NavigationReachSpec {
        distance,
        start,
        end,
        collision_radius: 40,
        collision_height: 40,
        pruned: false,
    };
    let specs = [
        spec(start.clone(), long.clone(), 10),
        spec(long, target.clone(), 10),
        spec(start.clone(), short.clone(), 3),
        spec(short.clone(), target.clone(), 3),
    ];
    assert_eq!(
        next_navigation_step(&specs, &start, &target, 20, 20),
        Some(short)
    );
    assert_eq!(next_navigation_step(&specs, &start, &target, 50, 20), None);
}

#[test]
fn rotator_addition_wraps_each_ue1_component() {
    assert_eq!(
        scalar_native(
            0x13c,
            &[
                Value::Rotator([i32::MAX, 2, -4]),
                Value::Rotator([1, 3, -5])
            ]
        ),
        Ok(Value::Rotator([i32::MIN, 5, -9]))
    );
    assert_eq!(
        scalar_native(
            0x13d,
            &[
                Value::Rotator([i32::MIN, i32::MAX, 1]),
                Value::Rotator([1, -1, 2])
            ]
        ),
        Ok(Value::Rotator([i32::MAX, i32::MIN, -1]))
    );
}

#[test]
fn requested_core_math_and_random_natives_match_unreal_semantics() {
    assert_eq!(
        scalar_native(0xc1, &[Value::Float(9.0)]),
        Ok(Value::Float(3.0))
    );
    assert_eq!(
        scalar_native(0xfb, &[Value::Int(12), Value::Int(-5), Value::Int(10)]),
        Ok(Value::Int(10))
    );
    assert_eq!(
        scalar_native(0xba, &[Value::Float(-2.5)]),
        Ok(Value::Float(2.5))
    );
    assert_eq!(
        scalar_native(0xe2, &[Value::Vector([3.0, 0.0, 4.0])]),
        Ok(Value::Vector([0.6, 0.0, 0.8]))
    );
    assert_eq!(
        scalar_native(0xe2, &[Value::Vector([0.0; 3])]),
        Ok(Value::Vector([0.0; 3]))
    );

    let mut state = 0x6d2b_79f5;
    for _ in 0..100 {
        assert!((0..7).contains(&random_int(&mut state, 7)));
        assert!((0.0..1.0).contains(&random_float(&mut state)));
    }
    assert_eq!(random_int(&mut state, 0), 0);
}

#[test]
fn random_vectors_are_normalized_and_deterministic() {
    let mut first = 0x6d2b_79f5;
    let mut second = first;
    let vector = random_unit_vector(&mut first);
    assert!((vector.length() - 1.0).abs() < 1.0e-6);
    assert_eq!(vector, random_unit_vector(&mut second));
}

#[test]
fn rot_rand_dispatches_the_extended_native_and_uses_optional_roll() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-rot-rand-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let execute = |runtime: &mut ScriptRuntime, bytes: Vec<u8>| {
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        let mut instance = InstanceState::default();
        let mut actions = Vec::new();
        frame
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                runtime.native(
                    0,
                    &class,
                    &package,
                    index,
                    arguments,
                    &mut instance,
                    &mut actions,
                    0,
                )
            })
            .unwrap()
    };

    runtime.random_state = 0x8000_0000;
    assert_eq!(
        execute(&mut runtime, vec![0x04, 0x61, 0x40, 0x16]),
        Value::Rotator([0x8908, 0x8008, 0])
    );
    runtime.random_state = 0x8000_0000;
    assert_eq!(
        execute(&mut runtime, vec![0x04, 0x61, 0x40, 0x27, 0x16]),
        Value::Rotator([0x8908, 0x8008, 0xaa91])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn animation_parameters_preserve_optional_tween_time() {
    assert_eq!(
        animation_parameters("LoopAnim", &[Value::None, Value::Float(0.5)]),
        Ok((1.0, 0.5))
    );
    assert_eq!(animation_parameters("PlayAnim", &[]), Ok((1.0, 0.0)));
}

#[test]
fn function_lookups_are_case_insensitive_and_state_scoped() {
    let class = ObjectId {
        package: Arc::from("Test.u"),
        export_index: 7,
    };
    let lower = FunctionLookup::new(class.clone(), Some("patrol"), "tick", 2);
    let upper = FunctionLookup::new(class.clone(), Some("PATROL"), "TICK", 2);
    let other_state = FunctionLookup::new(class, Some("waiting"), "tick", 2);

    assert_eq!(lower, upper);
    assert_ne!(lower, other_state);
}

#[test]
fn state_lookups_are_case_insensitive() {
    let class = ObjectId {
        package: Arc::from("Test.u"),
        export_index: 7,
    };
    assert_eq!(
        StateLookup::new(class.clone(), "patrol"),
        StateLookup::new(class, "PATROL")
    );
}
