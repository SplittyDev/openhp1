use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use openhp1_script::{Bytecode, FunctionMetadata, ScriptExport, ScriptMetadata};

use crate::{Frame, FrameRequest, FrameResponse, FunctionCall};

use super::*;
use super::{
    actor::advance_lifespan,
    actor::decode_latent_action,
    actor::update_touching_array,
    native::{
        animation_parameters, bone_number, bone_position, collision_updates, log_arguments,
        next_navigation_step, noise_loudness, random_float, random_int, random_unit_vector,
        scalar_native, sound_arguments, target_score,
    },
    state::{event_disabled, probe_event_index, set_event_disabled},
};
use openhp1_map::{BspNode, BspSurface, BspVertex, Model, PolyFlags, PrimitiveBounds};
use openhp1_physics::BspCollision;

static FIXTURE_ROOT: AtomicUsize = AtomicUsize::new(0);

fn synthetic_runtime_package() -> Vec<u8> {
    synthetic_runtime_package_for("PlayerPawn")
}

fn synthetic_runtime_package_for(class_name: &str) -> Vec<u8> {
    const HEADER_SIZE: usize = 44;
    let mut class_name = class_name.as_bytes().to_vec();
    class_name.push(0);
    let name_offset = HEADER_SIZE;
    let export_offset = name_offset
        + class_name.len()
        + size_of::<u32>()
        + b"ClientTravel\0".len()
        + size_of::<u32>()
        + b"GetPlayerNetworkAddress\0".len()
        + size_of::<u32>()
        + b"Pawn\0".len()
        + size_of::<u32>()
        + b"StopWaiting\0".len()
        + size_of::<u32>()
        + b"QuidHud\0".len()
        + size_of::<u32>()
        + b"Head\0".len()
        + size_of::<u32>();
    let mut bytes = Vec::new();
    bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
    bytes.extend(61_u16.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    for value in [
        7,
        name_offset as i32,
        6,
        export_offset as i32,
        0,
        export_offset as i32,
        0,
        0,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for name in [
        class_name.as_slice(),
        b"ClientTravel\0".as_slice(),
        b"GetPlayerNetworkAddress\0".as_slice(),
        b"Pawn\0".as_slice(),
        b"StopWaiting\0".as_slice(),
        b"QuidHud\0".as_slice(),
        b"Head\0".as_slice(),
    ] {
        bytes.extend(name);
        bytes.extend(0_u32.to_le_bytes());
    }
    for (outer, name) in [(0_i32, 0_u8), (1, 1), (1, 2), (0, 3), (4, 4), (0, 5)] {
        bytes.extend([0, 0]);
        bytes.extend(outer.to_le_bytes());
        bytes.push(name);
        bytes.extend(0_u32.to_le_bytes());
        bytes.push(0);
    }
    bytes
}

fn named_native_script(export_index: usize) -> Arc<openhp1_script::ScriptExport> {
    Arc::new(openhp1_script::ScriptExport {
        export_index,
        class_name: "Function".to_owned(),
        base_field: ObjectReference::None,
        next_field: ObjectReference::None,
        script_text: ObjectReference::None,
        children: ObjectReference::None,
        friendly_name: export_index,
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
    })
}

fn log_event_script(export_index: usize, message: &str) -> Arc<openhp1_script::ScriptExport> {
    let mut bytes = vec![0x04, LOG as u8, 0x1f];
    bytes.extend(message.bytes());
    bytes.extend([0, 0x16]);
    Arc::new(openhp1_script::ScriptExport {
        export_index,
        class_name: "Function".to_owned(),
        base_field: ObjectReference::None,
        next_field: ObjectReference::None,
        script_text: ObjectReference::None,
        children: ObjectReference::None,
        friendly_name: export_index,
        line: 0,
        text_position: 0,
        bytecode: Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        },
        metadata: ScriptMetadata::Function(FunctionMetadata {
            parameter_size: None,
            native_index: 0,
            parameter_count: None,
            operator_precedence: 0,
            return_value_offset: None,
            flags: 0,
            replication_offset: None,
        }),
    })
}

fn standing_count_event_script(export_index: usize) -> Arc<openhp1_script::ScriptExport> {
    let mut bytes = vec![0x04, LOG as u8, 0x52, 0x01];
    bytes.extend(1_i32.to_le_bytes());
    bytes.push(0x16);
    Arc::new(openhp1_script::ScriptExport {
        export_index,
        class_name: "Function".to_owned(),
        base_field: ObjectReference::None,
        next_field: ObjectReference::None,
        script_text: ObjectReference::None,
        children: ObjectReference::None,
        friendly_name: export_index,
        line: 0,
        text_position: 0,
        bytecode: Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        },
        metadata: ScriptMetadata::Function(FunctionMetadata {
            parameter_size: None,
            native_index: 0,
            parameter_count: None,
            operator_precedence: 0,
            return_value_offset: None,
            flags: 0,
            replication_offset: None,
        }),
    })
}

fn execute_extended_native_from_bytecode(
    runtime: &mut ScriptRuntime,
    actor: usize,
    class: &ResolvedObject,
    package: &Arc<Package>,
    instance: &mut InstanceState,
    index: u16,
    object: Option<i32>,
    actions: &mut Vec<ActorAction>,
) -> Value {
    assert_eq!(index >> 8, 1);
    let mut bytes = vec![0x04, 0x61, u8::try_from(index & 0xff).unwrap()];
    if object.is_some() {
        bytes.push(0x20);
        bytes.extend(1_i32.to_le_bytes());
    }
    bytes.push(0x16);
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    Frame::new(&bytecode)
        .execute_hosted(|request| match request {
            FrameRequest::ResolveObject { reference: 1 } => {
                Ok(FrameResponse::Value(Value::Object(object.unwrap())))
            }
            FrameRequest::Call {
                receiver,
                function: FunctionCall::Native(native),
                arguments,
            } => {
                assert_eq!(receiver, -1);
                assert_eq!(native, index);
                runtime
                    .native(
                        actor, class, package, native, &arguments, instance, actions, 0,
                    )
                    .map(FrameResponse::Value)
            }
            _ => panic!("unexpected frame request"),
        })
        .unwrap()
}

#[test]
fn dispatched_finite_function_counts_statements_not_nested_expression_tokens() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-statement-limit-{}-{}",
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
    let class_id = object_id(&package, class.export_index);
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());

    let mut bytes = Vec::new();
    for _ in 0..6_000 {
        bytes.push(0x0e);
        bytes.extend([0x81; 20]);
        bytes.push(0x28);
        bytes.extend([0x16; 20]);
    }
    bytes.extend([0x04, 0x0b]);
    runtime.scripts.insert(
        object_id(&package, function.export_index),
        Arc::new(ScriptExport {
            export_index: function.export_index,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 0,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: 0,
                replication_offset: None,
            }),
        }),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id, None, "ClientTravel", 0),
        Some(object_id(&package, function.export_index)),
    );

    assert!(
        runtime
            .dispatch_event(1, &package_path, class.export_index, "ClientTravel")
            .unwrap()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn final_function_calls_run_prebound_natives_and_propagate_failures() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-final-function-{}-{}",
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
    let caller = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 2,
    };
    runtime.class_defaults.insert(
        object_id(&package, class.export_index),
        InstanceState::default(),
    );
    runtime
        .scripts
        .insert(object_id(&package, 1), named_native_script(1));
    let caller_script = |bytes: Vec<u8>| {
        Arc::new(ScriptExport {
            export_index: caller.export_index,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: caller.export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: 0,
                replication_offset: None,
            }),
        })
    };
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();

    // Return a direct final call to PlayerPawn.ClientTravel without its required
    // arguments. The named-native diagnostic must cross the frame host boundary.
    runtime.scripts.insert(
        object_id(&package, caller.export_index),
        caller_script(vec![0x04, 0x1c, 0x02, 0x00, 0x00, 0x00, 0x16]),
    );
    let error = runtime
        .execute_function(1, &class, &caller, &[], &mut instance, &mut actions, 0)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Final(2) failed: named native function `PlayerPawn.ClientTravel` is not implemented"
    );
    assert!(actions.is_empty());

    let mut bytes = vec![0x04, 0x1c];
    bytes.extend(2_i32.to_le_bytes());
    bytes.extend([0x1f]);
    bytes.extend(b"Lev_Tut1\0");
    bytes.extend([0x24, 0x00, 0x28, 0x16]);
    runtime.scripts.insert(
        object_id(&package, caller.export_index),
        caller_script(bytes),
    );
    assert_eq!(
        runtime
            .execute_function(1, &class, &caller, &[], &mut instance, &mut actions, 0,)
            .unwrap(),
        Value::None
    );
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::ClientTravel {
            actor: 1,
            url,
            travel_type: 0,
            transfer_items: false,
        }] if url == "Lev_Tut1"
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn player_hud_initialization_spawns_the_configured_hud_subclass_once() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-player-hud-{}-{}",
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
    let player_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let hud_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 5,
    };
    let class_script = |export_index| {
        Arc::new(ScriptExport {
            export_index,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        })
    };
    let player_class_id = object_id(&package, player_class.export_index);
    let hud_class_id = object_id(&package, hud_class.export_index);
    runtime.scripts.insert(
        player_class_id.clone(),
        class_script(player_class.export_index),
    );
    runtime
        .scripts
        .insert(hud_class_id.clone(), class_script(hud_class.export_index));
    for event in [
        "Tick",
        "Spawned",
        "PreBeginPlay",
        "BeginPlay",
        "PostBeginPlay",
        "SetInitialState",
    ] {
        runtime.function_lookups.insert(
            FunctionLookup::new(hud_class_id.clone(), None, event, 0),
            None,
        );
    }

    let player_fields = [
        "myHUD",
        "HUDType",
        "Location",
        "Rotation",
        "Instigator",
        "Level",
        "XLevel",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(100 + index)))
    .collect::<HashMap<_, _>>();
    let hud_fields = [
        "Location",
        "OldLocation",
        "Rotation",
        "DesiredRotation",
        "Tag",
        "Owner",
        "Instigator",
        "Level",
        "XLevel",
        "bCollideWorld",
        "bCollideWhenPlacing",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(200 + index)))
    .collect::<HashMap<_, _>>();
    for (name, field) in &player_fields {
        runtime.fields.insert(
            (player_class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    for (name, field) in &hud_fields {
        runtime.fields.insert(
            (hud_class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }

    let player = 7;
    let player_object = runtime_actor_id(player);
    runtime.actor_classes.insert(player, player_class_id);
    runtime.object_actors.insert(player_object.clone(), player);
    runtime.actor_objects.insert(player, player_object);
    runtime.player_actor = Some(player);
    runtime.next_actor = player + 1;
    runtime.object_handle(hud_class_id.clone()).unwrap();

    let mut player_instance = InstanceState::default();
    player_instance.insert(player_fields["myHUD"].clone(), StoredValue::Object(None));
    player_instance.insert(
        player_fields["HUDType"].clone(),
        StoredValue::Object(Some(hud_class_id.clone())),
    );
    player_instance.insert(
        player_fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    player_instance.insert(
        player_fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([0; 3])),
    );
    for name in ["Instigator", "Level", "XLevel"] {
        player_instance.insert(player_fields[name].clone(), StoredValue::Object(None));
    }
    runtime.instances.insert(player, player_instance);

    let mut hud_defaults = InstanceState::default();
    hud_defaults.insert(
        hud_fields["bCollideWorld"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    hud_defaults.insert(
        hud_fields["bCollideWhenPlacing"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    runtime
        .class_defaults
        .insert(hud_class_id.clone(), hud_defaults);

    assert!(matches!(
        runtime.initialize_player_hud().unwrap().as_slice(),
        [ActorAction::SpawnActor { class_name, .. }] if class_name == "QuidHud"
    ));
    assert!(matches!(
        runtime.instances[&player].get(&player_fields["myHUD"]),
        Some(StoredValue::Object(Some(object))) if *object == runtime_actor_id(8)
    ));
    assert!(runtime.initialize_player_hud().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

fn solid_box_collision() -> Arc<BspCollision> {
    let mut model = Model {
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
    };
    model.surfaces.push(BspSurface {
        texture: ObjectReference::None,
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
    model.points = vec![
        Vec3::new(10.0, -100.0, -100.0),
        Vec3::new(10.0, 100.0, -100.0),
        Vec3::new(10.0, 100.0, 100.0),
        Vec3::new(10.0, -100.0, 100.0),
    ];
    model.vertices = (0..4).map(|point| BspVertex { point, side: -1 }).collect();
    model.nodes = [
        [1.0, 0.0, 0.0, 10.0],
        [-1.0, 0.0, 0.0, 10.0],
        [0.0, 1.0, 0.0, 10.0],
        [0.0, -1.0, 0.0, 10.0],
        [0.0, 0.0, 1.0, 10.0],
        [0.0, 0.0, -1.0, 10.0],
    ]
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
        vertex_count: if index == 0 { 4 } else { 0 },
        leaves: [0; 2],
    })
    .collect();
    model.leaf_hulls = vec![0, 1, 2, 3, 4, 5, -1];
    model.leaf_hulls.extend(
        [-10.0_f32, -10.0, -10.0, 10.0, 10.0, 10.0]
            .map(f32::to_bits)
            .map(|value| value as i32),
    );
    Arc::new(BspCollision::from_model(&model).unwrap())
}

#[test]
fn looping_timer_catches_up_through_bytecode_and_honors_callback_mutation() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-looping-timer-{}-{}",
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
    let class_id = object_id(&package, class.export_index);
    let timer_function = object_id(&package, 1);
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());
    let fields = [
        "TimeSeconds",
        "TimeDilation",
        "Physics",
        "LifeSpan",
        "bStatic",
        "bNoDelete",
        "bDeleteMe",
        "Base",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(100 + index)))
    .collect::<HashMap<_, _>>();
    for (name, field) in &fields {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            (name != &"LifeSpan").then_some(field.clone()),
        );
    }

    let level = 0;
    let actor = 1;
    let actor_object = runtime_actor_id(1_000);
    runtime.actor_classes.insert(level, class_id.clone());
    runtime.actor_classes.insert(actor, class_id.clone());
    runtime.actor_objects.insert(actor, actor_object.clone());
    runtime.object_actors.insert(actor_object, actor);
    runtime.level_info = Some(level);
    runtime.instances.insert(
        level,
        [
            (
                fields["TimeSeconds"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["TimeDilation"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.instances.insert(
        actor,
        [
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
            (
                fields["bStatic"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bNoDelete"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bDeleteMe"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Base"].clone(), StoredValue::Object(None)),
        ]
        .into_iter()
        .collect(),
    );
    let function = |bytes: Vec<u8>| {
        Arc::new(ScriptExport {
            export_index: 1,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: 0,
                replication_offset: None,
            }),
        })
    };
    runtime
        .scripts
        .insert(timer_function.clone(), function(vec![0x04, 0x0b]));
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id.clone(), None, "Timer", 0),
        Some(timer_function.clone()),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id.clone(), None, "Destroyed", 0),
        None,
    );

    let set_timer = |runtime: &mut ScriptRuntime, rate: f32, looping: bool| {
        let mut bytes = vec![0x04, 0x61, 0x18, 0x1e];
        bytes.extend(rate.to_le_bytes());
        bytes.extend([if looping { 0x27 } else { 0x28 }, 0x16]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        let mut instance = InstanceState::default();
        frame
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                assert_eq!(index, SET_TIMER);
                runtime.native(
                    actor,
                    &class,
                    &package,
                    index,
                    arguments,
                    &mut instance,
                    &mut Vec::new(),
                    0,
                )
            })
            .unwrap();
    };

    set_timer(&mut runtime, 0.1, true);
    assert_eq!(runtime.tick(0.35).unwrap(), []);
    assert_eq!(runtime.timer_callbacks(), 3);
    let timer = runtime.timers[&actor];
    assert!((timer.remaining - 0.05).abs() < 1.0e-6);

    let mut reset_timer = vec![0x0e, 0x61, 0x18, 0x1e];
    reset_timer.extend(0.5_f32.to_le_bytes());
    reset_timer.extend([0x28, 0x16, 0x04, 0x0b]);
    runtime
        .scripts
        .insert(timer_function.clone(), function(reset_timer));
    assert_eq!(runtime.tick(0.1).unwrap(), []);
    assert_eq!(runtime.timer_callbacks(), 4);
    assert!(matches!(
        runtime.timers.get(&actor),
        Some(ActorTimer {
            remaining,
            rate,
            looping: false,
        }) if (*remaining - 0.5).abs() < 1.0e-6 && (*rate - 0.5).abs() < 1.0e-6
    ));

    runtime
        .scripts
        .insert(timer_function.clone(), function(vec![0x04, 0x0b]));
    assert_eq!(runtime.tick(0.5).unwrap(), []);
    assert_eq!(runtime.timer_callbacks(), 5);
    assert!(!runtime.timers.contains_key(&actor));

    runtime.scripts.insert(
        timer_function,
        Arc::new(ScriptExport {
            export_index: 1,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: 0,
                bytes: Vec::new(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: DESTROY,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );
    set_timer(&mut runtime, 0.1, true);
    assert_eq!(
        runtime.tick(0.35).unwrap(),
        [ActorAction::DestroyActor { actor }]
    );
    assert_eq!(runtime.timer_callbacks(), 6);
    assert!(runtime.destroyed.contains(&actor));
    assert!(!runtime.timers.contains_key(&actor));
    fs::remove_dir_all(root).unwrap();
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
fn set_base_excludes_level_from_based_actors_and_standing_count() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-set-base-{}-{}",
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
    let class_id = object_id(&package, class.export_index);
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());
    let base_field = runtime_actor_id(10);
    let level_field = runtime_actor_id(11);
    let standing_count_field = object_id(&package, 0);
    let static_field = runtime_actor_id(13);
    let no_delete_field = runtime_actor_id(14);
    let delete_field = runtime_actor_id(15);
    for (name, field) in [
        ("Base", &base_field),
        ("Level", &level_field),
        ("StandingCount", &standing_count_field),
        ("bStatic", &static_field),
        ("bNoDelete", &no_delete_field),
        ("bDeleteMe", &delete_field),
    ] {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    for (event, export) in [("Attach", 1), ("Detach", 2)] {
        let function = object_id(&package, export);
        runtime
            .scripts
            .insert(function.clone(), standing_count_event_script(export));
        runtime.function_lookups.insert(
            FunctionLookup::new(class_id.clone(), None, event, 0),
            Some(function),
        );
    }
    let base_change = object_id(&package, 3);
    runtime
        .scripts
        .insert(base_change.clone(), log_event_script(3, "BaseChange"));
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id.clone(), None, "BaseChange", 0),
        Some(base_change),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id.clone(), None, "Destroyed", 0),
        None,
    );

    let level = runtime_actor_id(20);
    let child = runtime_actor_id(21);
    let real_base = runtime_actor_id(22);
    for (actor, object) in [(0, &child), (1, &real_base), (2, &level)] {
        runtime.actor_classes.insert(actor, class_id.clone());
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object.clone());
    }
    let instance = |base: Option<ObjectId>, standing_count| {
        [
            (base_field.clone(), StoredValue::Object(base)),
            (
                level_field.clone(),
                StoredValue::Object(Some(level.clone())),
            ),
            (
                standing_count_field.clone(),
                StoredValue::Value(Value::Byte(standing_count)),
            ),
            (static_field.clone(), StoredValue::Value(Value::Bool(false))),
            (
                no_delete_field.clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (delete_field.clone(), StoredValue::Value(Value::Bool(false))),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    let mut child_instance = instance(Some(level.clone()), 0);
    runtime.instances.insert(1, instance(None, 0));
    runtime.instances.insert(2, instance(None, 0));
    runtime
        .update_actor_base(0, Some(level.clone()), Some(level.clone()))
        .unwrap();
    assert!(runtime.base_children.get(&level).is_none());
    assert_eq!(
        runtime.instances[&2].get(&standing_count_field),
        Some(&StoredValue::Value(Value::Byte(0)))
    );

    let level_handle = runtime.object_handle(level.clone()).unwrap();
    let real_base_handle = runtime.object_handle(real_base.clone()).unwrap();
    let child_handle = runtime.object_handle(child).unwrap();
    let mut actions = Vec::new();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            0,
            &class,
            &package,
            &mut child_instance,
            SET_BASE,
            Some(0),
            &mut actions,
        ),
        Value::None
    );
    assert_eq!(
        child_instance.get(&base_field),
        Some(&StoredValue::Object(None))
    );
    assert_eq!(
        actions,
        vec![ActorAction::Log {
            actor: 0,
            message: "BaseChange".to_owned(),
            tag: None,
        }]
    );

    actions.clear();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            0,
            &class,
            &package,
            &mut child_instance,
            SET_BASE,
            Some(level_handle),
            &mut actions,
        ),
        Value::None
    );
    assert_eq!(
        actions,
        vec![ActorAction::Log {
            actor: 0,
            message: "BaseChange".to_owned(),
            tag: None,
        }]
    );
    assert!(runtime.base_children.get(&level).is_none());
    assert_eq!(
        runtime.instances[&2].get(&standing_count_field),
        Some(&StoredValue::Value(Value::Byte(0)))
    );

    actions.clear();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            0,
            &class,
            &package,
            &mut child_instance,
            SET_BASE,
            Some(real_base_handle),
            &mut actions,
        ),
        Value::None
    );
    assert_eq!(
        actions,
        vec![
            ActorAction::Log {
                actor: 1,
                message: "1".to_owned(),
                tag: None,
            },
            ActorAction::Log {
                actor: 0,
                message: "BaseChange".to_owned(),
                tag: None,
            },
        ]
    );
    assert_eq!(
        runtime.instances[&1].get(&standing_count_field),
        Some(&StoredValue::Value(Value::Byte(1)))
    );
    assert_eq!(runtime.base_children.get(&real_base), Some(&vec![0]));
    assert!(runtime.base_children.get(&level).is_none());

    actions.clear();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            0,
            &class,
            &package,
            &mut child_instance,
            SET_BASE,
            Some(level_handle),
            &mut actions,
        ),
        Value::None
    );
    assert_eq!(
        actions,
        vec![
            ActorAction::Log {
                actor: 1,
                message: "0".to_owned(),
                tag: None,
            },
            ActorAction::Log {
                actor: 0,
                message: "BaseChange".to_owned(),
                tag: None,
            },
        ]
    );
    assert_eq!(
        runtime.instances[&1].get(&standing_count_field),
        Some(&StoredValue::Value(Value::Byte(0)))
    );
    assert!(runtime.base_children.get(&real_base).is_none());
    assert!(runtime.base_children.get(&level).is_none());

    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            0,
            &class,
            &package,
            &mut child_instance,
            SET_BASE,
            Some(real_base_handle),
            &mut Vec::new(),
        ),
        Value::None
    );
    let mut real_base_instance = runtime.instances.remove(&1).unwrap();
    actions.clear();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            1,
            &class,
            &package,
            &mut real_base_instance,
            SET_BASE,
            Some(child_handle),
            &mut actions,
        ),
        Value::None
    );
    assert!(actions.is_empty());
    assert_eq!(
        real_base_instance.get(&base_field),
        Some(&StoredValue::Object(None))
    );
    runtime.instances.insert(0, child_instance);
    actions.clear();
    assert_eq!(
        execute_extended_native_from_bytecode(
            &mut runtime,
            1,
            &class,
            &package,
            &mut real_base_instance,
            DESTROY,
            None,
            &mut actions,
        ),
        Value::Bool(true)
    );
    assert_eq!(
        runtime.instances[&0].get(&base_field),
        Some(&StoredValue::Object(None))
    );
    assert_eq!(
        real_base_instance.get(&standing_count_field),
        Some(&StoredValue::Value(Value::Byte(0)))
    );
    assert_eq!(
        actions,
        vec![
            ActorAction::Log {
                actor: 1,
                message: "0".to_owned(),
                tag: None,
            },
            ActorAction::Log {
                actor: 0,
                message: "BaseChange".to_owned(),
                tag: None,
            },
            ActorAction::DestroyActor { actor: 1 },
        ]
    );
    assert!(runtime.base_children.get(&real_base).is_none());
    assert!(runtime.destroyed.contains(&1));
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

fn actor_reachable_bsp() -> Arc<BspCollision> {
    let planes = [
        [1.0, 0.0, 0.0, 105.0],
        [-1.0, 0.0, 0.0, -95.0],
        [0.0, 1.0, 0.0, 5.0],
        [0.0, -1.0, 0.0, 5.0],
        [0.0, 0.0, 1.0, 5.0],
        [0.0, 0.0, -1.0, 5.0],
    ];
    let model = Model {
        bounds: PrimitiveBounds {
            minimum: Vec3::ZERO,
            maximum: Vec3::ZERO,
            valid: false,
            sphere: [0.0; 4],
        },
        vectors: Vec::new(),
        points: Vec::new(),
        nodes: planes
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
                collision_bound: (index == 0).then_some(0).unwrap_or(-1),
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 0,
                leaves: [0; 2],
            })
            .collect(),
        surfaces: Vec::new(),
        vertices: Vec::new(),
        shared_side_count: 0,
        zones: Vec::new(),
        polys: ObjectReference::None,
        light_maps: Vec::new(),
        light_bits: Vec::new(),
        collision_bounds: Vec::new(),
        leaf_hulls: vec![
            0,
            1,
            2,
            3,
            4,
            5,
            -1,
            95.0_f32.to_bits() as i32,
            (-5.0_f32).to_bits() as i32,
            (-5.0_f32).to_bits() as i32,
            105.0_f32.to_bits() as i32,
            5.0_f32.to_bits() as i32,
            5.0_f32.to_bits() as i32,
        ],
        leaves: Vec::new(),
        lights: Vec::new(),
        root_outside: true,
        linked: false,
    };
    Arc::new(BspCollision::from_model(&model).unwrap())
}

#[test]
fn actor_reachable_dispatches_check_location_and_rejects_pruned_or_blocked_routes() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-actor-reachable-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let pawn_path = system.join("Pawn.u");
    let navigation_path = system.join("Navigation.u");
    fs::write(&pawn_path, synthetic_runtime_package()).unwrap();
    fs::write(
        &navigation_path,
        synthetic_runtime_package_for("NavigationPoint"),
    )
    .unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let pawn_package = runtime.packages.load_path(&pawn_path).unwrap();
    let navigation_package = runtime.packages.load_path(&navigation_path).unwrap();
    let pawn_class = ResolvedObject {
        package: Arc::clone(&pawn_package),
        export_index: 0,
    };
    let pawn_class_id = object_id(&pawn_package, 0);
    let navigation_class_id = object_id(&navigation_package, 0);
    runtime.scripts.insert(
        navigation_class_id.clone(),
        Arc::new(openhp1_script::ScriptExport {
            export_index: 0,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 0,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                raw_len: 0,
                bytes: Vec::new(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        }),
    );
    let function = ResolvedObject {
        package: Arc::clone(&pawn_package),
        export_index: 1,
    };
    runtime.scripts.insert(
        object_id(&pawn_package, 1),
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
            bytecode: Bytecode {
                version: 61,
                raw_len: 0,
                bytes: Vec::new(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: ACTOR_REACHABLE,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );

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
        "bCollideWorld",
        "bCollideWhenPlacing",
        "bStatic",
        "bMovable",
        "Physics",
        "BaseEyeHeight",
        "bIsPlayer",
        "bPlayerOnly",
        "Paths",
        "PrunedPaths",
        "ZoneGravity",
        "ZoneVelocity",
        "ZoneGroundFriction",
        "ZoneFluidFriction",
        "ZoneTerminalVelocity",
        "bWaterZone",
        "bPainZone",
        "DamageType",
        "ReducedDamageType",
        "bCanSwim",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        (
            name,
            ObjectId {
                package: Arc::from("<actor-reachable-test>"),
                export_index: index,
            },
        )
    })
    .collect::<HashMap<_, _>>();
    for class in [&pawn_class_id, &navigation_class_id] {
        for (name, field) in &fields {
            runtime.fields.insert(
                (class.clone(), name.to_ascii_lowercase()),
                if class == &pawn_class_id && matches!(*name, "Paths" | "PrunedPaths") {
                    None
                } else {
                    Some(field.clone())
                },
            );
        }
        runtime
            .fields
            .insert((class.clone(), "mainscale".to_owned()), None);
    }

    let pawn = runtime_actor_id(1);
    let navigation = runtime_actor_id(2);
    runtime.actor_classes.insert(1, pawn_class_id.clone());
    runtime.actor_classes.insert(2, navigation_class_id);
    runtime.actor_classes.insert(0, pawn_class_id.clone());
    runtime.object_actors.insert(pawn.clone(), 1);
    runtime.object_actors.insert(navigation.clone(), 2);
    runtime.actor_objects.insert(1, pawn.clone());
    runtime.actor_objects.insert(2, navigation.clone());
    runtime.next_actor = 3;
    runtime.instances.insert(
        2,
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([100.0, 0.0, 0.0])),
            ),
            (
                fields["bPlayerOnly"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["Paths"].clone(),
                StoredValue::Array(vec![StoredValue::Value(Value::Int(0))]),
            ),
            (
                fields["PrunedPaths"].clone(),
                StoredValue::Array(Vec::new()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.reach_specs.push(NavigationReachSpec {
        index: 0,
        distance: 100,
        start: pawn,
        end: navigation.clone(),
        collision_radius: 20,
        collision_height: 40,
        pruned: false,
    });
    runtime.reach_specs.push(NavigationReachSpec {
        index: 1,
        distance: 100,
        start: runtime_actor_id(1),
        end: navigation.clone(),
        collision_radius: 20,
        collision_height: 40,
        pruned: false,
    });
    runtime.instances.insert(
        0,
        [
            (
                fields["ZoneGravity"].clone(),
                StoredValue::Value(Value::Vector([0.0, 0.0, -950.0])),
            ),
            (
                fields["ZoneVelocity"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["ZoneGroundFriction"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["ZoneFluidFriction"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["ZoneTerminalVelocity"].clone(),
                StoredValue::Value(Value::Float(2_500.0)),
            ),
            (
                fields["bWaterZone"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bPainZone"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["DamageType"].clone(),
                StoredValue::Value(Value::NameText("None".to_owned())),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.collision = Some(actor_reachable_bsp());
    runtime.level_info = Some(0);
    let navigation = runtime.object_handle(navigation).unwrap();
    let mut pawn_instance = [
        (fields["Location"].clone(), Value::Vector([0.0; 3])),
        (fields["CollisionHeight"].clone(), Value::Float(20.0)),
        (fields["CollisionRadius"].clone(), Value::Float(10.0)),
        (fields["bCollideActors"].clone(), Value::Bool(true)),
        (fields["bBlockPlayers"].clone(), Value::Bool(true)),
        (fields["bCollideWorld"].clone(), Value::Bool(false)),
        (fields["bCollideWhenPlacing"].clone(), Value::Bool(true)),
        (fields["bStatic"].clone(), Value::Bool(false)),
        (fields["bMovable"].clone(), Value::Bool(true)),
        (fields["Physics"].clone(), Value::Byte(4)),
        (fields["BaseEyeHeight"].clone(), Value::Float(0.0)),
        (fields["bIsPlayer"].clone(), Value::Bool(true)),
        (
            fields["ReducedDamageType"].clone(),
            Value::NameText("None".to_owned()),
        ),
    ]
    .into_iter()
    .map(|(field, value)| (field, StoredValue::Value(value)))
    .collect::<InstanceState>();

    let collision = runtime.collision.as_ref().unwrap();
    assert!(collision.overlaps_cylinder(Vec3::new(100.0, 0.0, 0.0), 10.0, 20.0));
    assert!(!collision.overlaps_cylinder(Vec3::new(120.0, 0.0, 0.0), 10.0, 20.0));

    assert_eq!(
        runtime
            .execute_function(
                1,
                &pawn_class,
                &function,
                &[Value::Object(navigation)],
                &mut pawn_instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(true),
    );
    runtime.reach_specs[0].pruned = true;
    assert_eq!(
        runtime
            .execute_function(
                1,
                &pawn_class,
                &function,
                &[Value::Object(navigation)],
                &mut pawn_instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(false),
    );
    runtime.reach_specs[0].pruned = false;
    let blocker = runtime_actor_id(3);
    runtime.actor_classes.insert(3, pawn_class_id);
    runtime.object_actors.insert(blocker.clone(), 3);
    runtime.actor_objects.insert(3, blocker);
    runtime.instances.insert(
        3,
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([50.0, 0.0, 0.0])),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(20.0)),
            ),
            (
                fields["CollisionRadius"].clone(),
                StoredValue::Value(Value::Float(20.0)),
            ),
            (
                fields["bCollideActors"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bBlockPlayers"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.next_actor = 4;
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    assert_eq!(
        runtime
            .execute_function(
                1,
                &pawn_class,
                &function,
                &[Value::Object(navigation)],
                &mut pawn_instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(false),
    );
    assert!(matches!(
        pawn_instance.get(&fields["Location"]),
        Some(StoredValue::Value(Value::Vector([0.0, 0.0, 0.0])))
    ));
    runtime.instances.get_mut(&2).unwrap().insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0, 0.0, -100.0])),
    );
    pawn_instance.insert(
        fields["Physics"].clone(),
        StoredValue::Value(Value::Byte(1)),
    );
    assert_eq!(
        runtime
            .execute_function(
                1,
                &pawn_class,
                &function,
                &[Value::Object(navigation)],
                &mut pawn_instance,
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert!(matches!(
        pawn_instance.get(&fields["Location"]),
        Some(StoredValue::Value(Value::Vector([0.0, 0.0, 0.0])))
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn set_location_places_through_bytecode_and_finds_or_rejects_world_bsp() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-set-location-{}-{}",
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
    let class_id = object_id(&package, class.export_index);
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
        "bCollideWorld",
        "bCollideWhenPlacing",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        (
            name,
            ObjectId {
                package: Arc::from("<set-location-test>"),
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
    for (index, object) in [(0, actor), (1, other)] {
        runtime.actor_classes.insert(index, class_id.clone());
        runtime.object_actors.insert(object.clone(), index);
        runtime.actor_objects.insert(index, object);
    }
    runtime.next_actor = 2;
    let instance_at = |location, collide_when_placing| {
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector(location)),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(20.0)),
            ),
            (
                fields["CollisionRadius"].clone(),
                StoredValue::Value(Value::Float(10.0)),
            ),
            (
                fields["CollisionWidth"].clone(),
                StoredValue::Value(Value::Float(10.0)),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["CollideType"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
            (
                fields["bCollideActors"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bBlockActors"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bBlockPlayers"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (fields["Brush"].clone(), StoredValue::Object(None)),
            (
                fields["PrePivot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["bCollideWorld"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bCollideWhenPlacing"].clone(),
                StoredValue::Value(Value::Bool(collide_when_placing)),
            ),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    let mut instance = instance_at([0.0; 3], false);
    runtime
        .instances
        .insert(1, instance_at([25.0, 0.0, 0.0], false));

    let execute = |runtime: &mut ScriptRuntime,
                   instance: &mut InstanceState,
                   location: [f32; 3],
                   actions: &mut Vec<ActorAction>| {
        let mut bytes = vec![0x04, 0x61, 0x0b, 0x23];
        bytes.extend(location.into_iter().flat_map(f32::to_le_bytes));
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
                assert_eq!(call, FunctionCall::Native(0x10b));
                runtime.native(0, &class, &package, 0x10b, arguments, instance, actions, 0)
            })
            .unwrap()
    };

    let mut actions = Vec::new();
    assert_eq!(
        execute(&mut runtime, &mut instance, [25.0, 0.0, 0.0], &mut actions),
        Value::Bool(true)
    );
    assert_eq!(
        instance.get(&fields["Location"]),
        Some(&StoredValue::Value(Value::Vector([25.0, 0.0, 0.0])))
    );
    assert!(matches!(
        actions.as_slice(),
        [
            ActorAction::SetLocation {
                actor: 0,
                location: [25.0, 0.0, 0.0],
            },
            ActorAction::DispatchEvent {
                actor: 0,
                event: "Touch",
                ..
            },
            ActorAction::DispatchEvent {
                actor: 1,
                event: "Touch",
                ..
            },
        ]
    ));
    assert!(runtime.touching.contains(&(0, 1)));

    actions.clear();
    assert_eq!(
        execute(&mut runtime, &mut instance, [100.0, 0.0, 0.0], &mut actions),
        Value::Bool(true)
    );
    assert!(matches!(
        actions.as_slice(),
        [
            ActorAction::SetLocation {
                actor: 0,
                location: [100.0, 0.0, 0.0],
            },
            ActorAction::DispatchEvent {
                actor: 0,
                event: "UnTouch",
                ..
            },
            ActorAction::DispatchEvent {
                actor: 1,
                event: "UnTouch",
                ..
            },
        ]
    ));
    assert!(!runtime.touching.contains(&(0, 1)));

    runtime
        .actor_bases
        .insert(0, Some(runtime.actor_objects[&1].clone()));
    runtime.collision = Some(placement_test_collision(10.0));
    instance.insert(
        fields["bCollideWhenPlacing"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    actions.clear();
    assert_eq!(
        execute(&mut runtime, &mut instance, [0.0; 3], &mut actions),
        Value::Bool(true)
    );
    assert_eq!(
        instance.get(&fields["Location"]),
        Some(&StoredValue::Value(Value::Vector([20.0, 0.0, 0.0])))
    );
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::SetLocation {
            actor: 0,
            location: [20.0, 0.0, 0.0],
        }]
    ));
    assert!(!runtime.touching.contains(&(0, 1)));

    runtime.collision = Some(placement_test_collision(100.0));
    actions.clear();
    assert_eq!(
        execute(&mut runtime, &mut instance, [0.0; 3], &mut actions),
        Value::Bool(false)
    );
    assert_eq!(
        instance.get(&fields["Location"]),
        Some(&StoredValue::Value(Value::Vector([20.0, 0.0, 0.0])))
    );
    assert!(actions.is_empty());
    fs::remove_dir_all(root).unwrap();
}

fn placement_test_collision(half_extent: f32) -> Arc<BspCollision> {
    let mut model = Model {
        bounds: PrimitiveBounds {
            minimum: glam::Vec3::ZERO,
            maximum: glam::Vec3::ZERO,
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
    };
    model.nodes = [
        [1.0, 0.0, 0.0, half_extent],
        [-1.0, 0.0, 0.0, half_extent],
        [0.0, 1.0, 0.0, half_extent],
        [0.0, -1.0, 0.0, half_extent],
        [0.0, 0.0, 1.0, half_extent],
        [0.0, 0.0, -1.0, half_extent],
    ]
    .into_iter()
    .enumerate()
    .map(|(index, plane)| openhp1_map::BspNode {
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
        [
            -half_extent,
            -half_extent,
            -half_extent,
            half_extent,
            half_extent,
            half_extent,
        ]
        .map(f32::to_bits)
        .map(|value| value as i32),
    );
    Arc::new(BspCollision::from_model(&model).unwrap())
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
fn integer_logical_right_shift_dispatches_from_bytecode_through_runtime_native() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-logical-right-shift-{}-{}",
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
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();
    let mut execute = |left: i32, right: i32| {
        let mut bytes = vec![0x04, 0xc4, 0x1d];
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
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                assert_eq!(index, 0xc4);
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

    assert_eq!(execute(i32::MIN, 31), Value::Int(1));
    assert_eq!(execute(i32::MIN, 32), Value::Int(i32::MIN));
    assert_eq!(execute(-1, -1), Value::Int(1));
    fs::remove_dir_all(root).unwrap();
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
fn is_animating_root_bone_dispatches_to_the_matching_channel() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-is-animating-{}-{}",
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
    runtime
        .actor_bone_names
        .insert(0, vec!["Root".to_owned(), "Head".to_owned()]);
    runtime.animation_channels.insert(
        0,
        vec![AnimationChannel {
            root_bone: 1,
            actor: 1,
        }],
    );
    runtime.animating.insert(0);
    runtime.animating.insert(1);

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
                assert_eq!(index, IS_ANIMATING);
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

    assert_eq!(
        execute(&mut runtime, vec![0x04, 0x61, 0x1a, 0x21, 6, 0, 0, 0, 0x16],),
        Value::Bool(true)
    );
    runtime.animating.remove(&1);
    assert_eq!(
        execute(&mut runtime, vec![0x04, 0x61, 0x1a, 0x21, 6, 0, 0, 0, 0x16],),
        Value::Bool(false)
    );
    assert_eq!(
        execute(&mut runtime, vec![0x04, 0x61, 0x1a, 0x16]),
        Value::Bool(true)
    );
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
fn modify_sound_dispatches_the_hp1_action() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-modify-sound-{}-{}",
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
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();
    let run = |runtime: &mut ScriptRuntime,
               sound: Option<i32>,
               slot: u8,
               parameter: u8,
               instance: &mut InstanceState,
               actions: &mut Vec<ActorAction>| {
        let mut bytes = vec![0x04, 0x62, 0x37];
        match sound {
            Some(sound) => {
                bytes.push(0x20);
                bytes.extend(sound.to_le_bytes());
            }
            None => bytes.push(0x2a),
        }
        bytes.push(0x1e);
        bytes.extend(0.75_f32.to_le_bytes());
        bytes.extend([0x24, parameter, 0x24, slot, 0x16]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        Frame::new(&bytecode)
            .execute_hosted(|request| match request {
                FrameRequest::ResolveObject { reference } => {
                    assert_eq!(sound, Some(reference));
                    Ok(FrameResponse::Value(Value::Object(reference)))
                }
                FrameRequest::Call {
                    function: FunctionCall::Native(index),
                    arguments,
                    ..
                } => runtime
                    .native(0, &class, &package, index, &arguments, instance, actions, 0)
                    .map(FrameResponse::Value),
                _ => unreachable!(),
            })
            .unwrap()
    };

    assert_eq!(
        run(&mut runtime, None, 3, 1, &mut instance, &mut actions),
        Value::Bool(false)
    );
    assert!(actions.is_empty());

    let sound = runtime_actor_id(90);
    let sound_handle = runtime.object_handle(sound.clone()).unwrap();
    let other_sound = runtime_actor_id(91);
    let other_sound_handle = runtime.object_handle(other_sound.clone()).unwrap();
    assert!(runtime.start_sound(0, 3, sound.clone(), 1.0, 1.0, false));

    assert_eq!(
        run(&mut runtime, None, 3, 1, &mut instance, &mut actions),
        Value::Bool(true)
    );
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::ModifySound {
            actor: 0,
            slot: 3,
            parameter: 1,
            value,
        }] if (*value - 0.75).abs() < f32::EPSILON
    ));
    actions.clear();

    assert_eq!(
        run(
            &mut runtime,
            Some(sound_handle),
            3,
            2,
            &mut instance,
            &mut actions,
        ),
        Value::Bool(true)
    );
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::ModifySound { .. }]
    ));
    assert!((runtime.sound_channels[&(0, 3)].pitch - 0.75).abs() < f32::EPSILON);
    actions.clear();

    assert_eq!(
        run(
            &mut runtime,
            Some(other_sound_handle),
            3,
            1,
            &mut instance,
            &mut actions,
        ),
        Value::Bool(false)
    );
    assert!(actions.is_empty());
    assert_eq!(
        run(&mut runtime, None, 0, 1, &mut instance, &mut actions),
        Value::Bool(false)
    );
    assert!(actions.is_empty());

    assert_eq!(
        runtime
            .native(
                0,
                &class,
                &package,
                STOP_SOUND,
                &[Value::None, Value::Byte(3)],
                &mut instance,
                &mut actions,
                0,
            )
            .unwrap(),
        Value::None
    );
    assert!(!runtime.sound_channels.contains_key(&(0, 3)));
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::StopSound { .. }]
    ));
    actions.clear();
    assert_eq!(
        run(&mut runtime, None, 3, 1, &mut instance, &mut actions),
        Value::Bool(false)
    );

    assert!(runtime.start_sound(0, 3, sound.clone(), 0.5, 1.0, false));
    runtime.tick_sound_channels(0.5);
    assert_eq!(
        run(&mut runtime, None, 3, 1, &mut instance, &mut actions),
        Value::Bool(false)
    );
    assert!(actions.is_empty());

    assert!(runtime.start_sound(0, 3, sound.clone(), 1.0, 1.0, false));
    assert!(!runtime.start_sound(0, 3, other_sound.clone(), 1.0, 1.0, true));
    assert_eq!(runtime.sound_channels[&(0, 3)].sound, sound);
    assert!(runtime.start_sound(0, 3, other_sound, 1.0, 1.0, false));
    assert!(!runtime.sound_channels.contains_key(&(0, 0)));
    assert!(runtime.start_sound(0, 0, runtime_actor_id(92), 1.0, 1.0, false));
    assert!(!runtime.sound_channels.contains_key(&(0, 0)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cross_vector_vector_dispatches_from_bytecode_through_runtime_native() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-cross-{}-{}",
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
    let mut bytes = vec![0x04, 0xdc];
    for vector in [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] {
        bytes.push(0x23);
        bytes.extend(vector.into_iter().flat_map(f32::to_le_bytes));
    }
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
    let result = frame
        .execute(|call, arguments| {
            let FunctionCall::Native(index) = call else {
                unreachable!()
            };
            assert_eq!(index, 0xdc);
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
        .unwrap();
    assert_eq!(result, Value::Vector([-3.0, 6.0, -3.0]));
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
fn autonomous_physics_dispatches_from_tick_without_double_advancing() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-autonomous-physics-{}-{}",
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
        export_index: 1,
    };
    let function = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 2,
    };
    let class_id = object_id(&package, class.export_index);
    runtime.scripts.insert(
        class_id.clone(),
        Arc::new(openhp1_script::ScriptExport {
            export_index: class.export_index,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        }),
    );
    runtime.scripts.insert(
        object_id(&package, function.export_index),
        Arc::new(openhp1_script::ScriptExport {
            export_index: function.export_index,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 2,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: AUTONOMOUS_PHYSICS,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );

    let fields = [
        "TimeSeconds",
        "TimeDilation",
        "Physics",
        "Rotation",
        "RotationRate",
        "bRotateToDesired",
        "bFixedRotationDir",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(100 + index)))
    .collect::<HashMap<_, _>>();
    for (name, field) in &fields {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    runtime
        .fields
        .insert((class_id.clone(), "lifespan".to_owned()), None);

    let level = 0;
    let actor = 1;
    runtime.actor_classes.insert(level, class_id.clone());
    runtime.actor_classes.insert(actor, class_id);
    runtime.level_info = Some(level);
    runtime.tick_functions.insert(actor, function);
    runtime.instances.insert(
        level,
        [
            (
                fields["TimeSeconds"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["TimeDilation"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.instances.insert(
        actor,
        [
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(5)),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["RotationRate"].clone(),
                StoredValue::Value(Value::Rotator([0, 1_000, 0])),
            ),
            (
                fields["bRotateToDesired"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bFixedRotationDir"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
        ]
        .into_iter()
        .collect(),
    );

    assert_eq!(
        runtime.tick(0.02).unwrap(),
        [ActorAction::SetRotation {
            actor,
            rotation: [0, 20, 0],
        }]
    );
    assert_eq!(
        runtime.instances[&actor].get(&fields["Rotation"]),
        Some(&StoredValue::Value(Value::Rotator([0, 20, 0])))
    );
    fs::remove_dir_all(root).unwrap();
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
    runtime
        .scripts
        .insert(object_id(&package, 1), named_native_script(1));
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
fn update_url_numeric_native_dispatches_optional_defaults() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-update-url-{}-{}",
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
                native_index: UPDATE_URL,
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
                    Value::String("Name".to_owned()),
                    Value::String("Harry".to_owned()),
                ],
            )
            .unwrap(),
        [ActorAction::UpdateUrl {
            actor: 17,
            option: "Name".to_owned(),
            value: "Harry".to_owned(),
            save_default: false,
        }]
    );
    assert_eq!(
        runtime
            .execute_actor_function(
                17,
                &class,
                &function,
                &[
                    Value::String("Class".to_owned()),
                    Value::String("Wizard".to_owned()),
                    Value::None,
                ],
            )
            .unwrap(),
        [ActorAction::UpdateUrl {
            actor: 17,
            option: "Class".to_owned(),
            value: "Wizard".to_owned(),
            save_default: false,
        }]
    );
    assert_eq!(
        runtime
            .execute_actor_function(
                17,
                &class,
                &function,
                &[
                    Value::String("Voice".to_owned()),
                    Value::String("Harry".to_owned()),
                    Value::Bool(true),
                ],
            )
            .unwrap(),
        [ActorAction::UpdateUrl {
            actor: 17,
            option: "Voice".to_owned(),
            value: "Harry".to_owned(),
            save_default: true,
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
    runtime
        .scripts
        .insert(object_id(&package, 2), named_native_script(2));

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

fn stair_collision(current_floor: f32, forward_floor: f32) -> Arc<BspCollision> {
    let mut model = Model {
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
    };
    for (minimum_x, maximum_x, floor) in [(-80.0, 0.0, current_floor), (0.0, 80.0, forward_floor)] {
        let first_plane = model.nodes.len();
        let collision_bound = model.leaf_hulls.len() as i32;
        let planes = [
            [1.0, 0.0, 0.0, maximum_x],
            [-1.0, 0.0, 0.0, -minimum_x],
            [0.0, 1.0, 0.0, 80.0],
            [0.0, -1.0, 0.0, 80.0],
            [0.0, 0.0, 1.0, floor],
            [0.0, 0.0, -1.0, 100.0],
        ];
        model.nodes.extend(
            planes
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
                    collision_bound: if index == 0 { collision_bound } else { -1 },
                    render_bound: -1,
                    zones: [0; 2],
                    vertex_count: 0,
                    leaves: [0; 2],
                }),
        );
        model
            .leaf_hulls
            .extend((first_plane..first_plane + planes.len()).map(|index| index as i32));
        model.leaf_hulls.push(-1);
        model.leaf_hulls.extend(
            [minimum_x, -80.0, -100.0, maximum_x, 80.0, floor]
                .map(f32::to_bits)
                .map(|value| value as i32),
        );
    }
    Arc::new(BspCollision::from_model(&model).unwrap())
}

fn find_stair_rotation_native(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &mut InstanceState,
    delta_time: f32,
) -> Value {
    runtime
        .native(
            17,
            class,
            &class.package,
            FIND_STAIR_ROTATION,
            &[Value::Float(delta_time)],
            instance,
            &mut Vec::new(),
            0,
        )
        .unwrap()
}

#[test]
fn find_stair_rotation_uses_floor_samples_interpolates_and_matches_pitch_normalization() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-find-stair-rotation-{}-{}",
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
        "Rotation",
        "Location",
        "CollisionRadius",
        "CollisionHeight",
        "EyeHeight",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        let field = runtime_actor_id(index + 31);
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
        (name, field)
    })
    .collect::<HashMap<_, _>>();
    let mut instance = InstanceState::default();
    instance.insert(
        fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([4_000, 0, 0])),
    );
    instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([-10.0, 0.0, 10.0])),
    );
    instance.insert(
        fields["CollisionRadius"].clone(),
        StoredValue::Value(Value::Float(1.0)),
    );
    instance.insert(
        fields["CollisionHeight"].clone(),
        StoredValue::Value(Value::Float(10.0)),
    );
    instance.insert(
        fields["EyeHeight"].clone(),
        StoredValue::Value(Value::Float(10.0)),
    );

    runtime.collision = Some(stair_collision(0.0, 0.0));
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.34),
        Value::Int(4_000)
    );
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.01),
        Value::Int(3_680)
    );
    runtime.collision = Some(stair_collision(0.0, 20.0));
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.33),
        Value::Int(5_400)
    );
    runtime.collision = Some(stair_collision(20.0, 0.0));
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.33),
        Value::Int(-5_000)
    );
    instance.insert(
        fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([0x1_0000, 0, 0])),
    );
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.0),
        Value::Int(-0x1_0000)
    );
    assert_eq!(
        instance.get(&fields["Rotation"]),
        Some(&StoredValue::Value(Value::Rotator([-0x1_0000, 0, 0])))
    );
    instance.insert(
        fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([0x8000, 0, 0])),
    );
    assert_eq!(
        find_stair_rotation_native(&mut runtime, &class, &mut instance, 0.0),
        Value::Int(0x8000)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pawn_stop_waiting_named_native_cancels_sleep_through_function_execution() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-stop-waiting-{}-{}",
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
        export_index: 3,
    };
    let function = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 4,
    };
    runtime
        .scripts
        .insert(object_id(&package, 4), named_native_script(4));
    runtime.state_frames.insert(
        17,
        StateFrame {
            state: object_id(&package, 3),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::Sleep(10.0),
        },
    );

    assert_eq!(
        runtime
            .execute_actor_function(17, &class, &function, &[])
            .unwrap(),
        []
    );
    assert_eq!(
        runtime.state_frames.get(&17).unwrap().latent,
        LatentAction::Sleep(0.0)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pick_any_target_rejects_beyond_sight_radius_without_changing_outputs() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-pick-any-target-{}-{}",
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
    let receiver_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let target_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 1,
    };
    let pawn_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 3,
    };
    let function = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 2,
    };
    let class_script = |export_index| {
        Arc::new(openhp1_script::ScriptExport {
            export_index,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        })
    };
    for export_index in [0, 1, 3] {
        runtime.scripts.insert(
            object_id(&package, export_index),
            class_script(export_index),
        );
    }
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
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: 0x216,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        }),
    );

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
                package: Arc::from("<pick-any-target-test>"),
                export_index: index,
            },
        )
    })
    .collect::<HashMap<_, _>>();
    for class in [&receiver_class, &target_class, &pawn_class] {
        let class_id = object_id(&package, class.export_index);
        for (name, field) in &fields {
            runtime.fields.insert(
                (class_id.clone(), name.to_ascii_lowercase()),
                Some(field.clone()),
            );
        }
        runtime
            .fields
            .insert((class_id, "mainscale".to_owned()), None);
    }
    let target_class_id = object_id(&package, target_class.export_index);
    let proj_target_field = runtime_actor_id(10);
    runtime.fields.insert(
        (target_class_id, "bprojtarget".to_owned()),
        Some(proj_target_field.clone()),
    );
    let receiver = runtime_actor_id(20);
    let eligible = runtime_actor_id(21);
    let not_a_projectile_target = runtime_actor_id(22);
    let pawn = runtime_actor_id(23);
    for (actor, class, object) in [
        (0, &receiver_class, &receiver),
        (1, &target_class, &eligible),
        (2, &target_class, &not_a_projectile_target),
        (3, &pawn_class, &pawn),
    ] {
        runtime
            .actor_classes
            .insert(actor, object_id(&package, class.export_index));
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object.clone());
    }
    runtime.next_actor = 4;
    let mut receiver_instance = InstanceState::default();
    receiver_instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    receiver_instance.insert(
        fields["BaseEyeHeight"].clone(),
        StoredValue::Value(Value::Float(20.0)),
    );
    receiver_instance.insert(
        fields["SightRadius"].clone(),
        StoredValue::Value(Value::Float(50.0)),
    );
    runtime.instances.insert(0, receiver_instance.clone());
    let target_instance = |projectile_target, location| {
        let mut instance = InstanceState::default();
        instance.insert(
            proj_target_field.clone(),
            StoredValue::Value(Value::Bool(projectile_target)),
        );
        instance.insert(
            fields["Location"].clone(),
            StoredValue::Value(Value::Vector(location)),
        );
        instance.insert(
            fields["CollisionHeight"].clone(),
            StoredValue::Value(Value::Float(40.0)),
        );
        instance
    };
    runtime
        .instances
        .insert(1, target_instance(true, [100.0, 0.0, 0.0]));
    runtime
        .instances
        .insert(2, target_instance(false, [50.0, 0.0, 0.0]));
    runtime.instances.insert(
        3,
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([200.0, 0.0, 0.0])),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(40.0)),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let arguments = [
        Value::Float(0.5),
        Value::Float(2_500.0),
        Value::Vector([1.0, 0.0, 0.0]),
        Value::Vector([0.0, 0.0, 0.0]),
    ];
    let (value, best_aim, best_dist) = runtime
        .pick_target(0, &receiver_class, &receiver_instance, &arguments, false)
        .unwrap();
    assert_eq!(value, Value::Object(0));
    assert_eq!(best_aim, 0.5);
    assert_eq!(best_dist, 2_500.0);

    let mut outputs = Vec::new();
    runtime
        .execute_actor_function_with_outputs(
            0,
            &receiver_class,
            &function,
            &arguments,
            &mut outputs,
        )
        .unwrap();
    assert_eq!(&outputs[..2], &arguments[..2]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn named_native_shims_validate_their_engine_calls() {
    let mut actions = Vec::new();
    let mut state_frames = HashMap::default();
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "PlayerPawn",
            "ConsoleCommand",
            &[Value::String("GETPING".to_owned())],
            &mut actions,
        ),
        Some(Value::String(String::new()))
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "Decal",
            "DetachDecal",
            &[],
            &mut actions,
        ),
        Some(Value::None)
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "PlayerPawn",
            "ConsoleCommand",
            &[],
            &mut actions,
        ),
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
fn make_noise_bytecode_records_the_instigator_and_dispatches_hear_noise() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-make-noise-{}-{}",
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
        export_index: 3,
    };
    let class_id = object_id(&package, class.export_index);
    let fields = [
        "Instigator",
        "Level",
        "TimeSeconds",
        "PawnList",
        "nextPawn",
        "Location",
        "bIsPlayer",
        "Enemy",
        "noise1time",
        "noise1spot",
        "noise1loudness",
        "noise2time",
        "noise2spot",
        "noise2loudness",
        "Alertness",
        "HearingThreshold",
        "Stimulus",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(100 + index)))
    .collect::<HashMap<_, _>>();
    for (name, field) in &fields {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    runtime
        .fields
        .insert((class_id.clone(), "netmode".to_owned()), None);
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());

    let level = runtime_actor_id(1);
    let source = runtime_actor_id(2);
    let noise = runtime_actor_id(3);
    let listener = runtime_actor_id(4);
    for (actor, object) in [(0, &level), (1, &source), (2, &noise), (3, &listener)] {
        runtime.actor_classes.insert(actor, class_id.clone());
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object.clone());
    }
    runtime.instances.insert(
        0,
        [
            (
                fields["TimeSeconds"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["PawnList"].clone(),
                StoredValue::Object(Some(noise.clone())),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.instances.insert(
        2,
        [
            (
                fields["nextPawn"].clone(),
                StoredValue::Object(Some(listener.clone())),
            ),
            (
                fields["bIsPlayer"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Enemy"].clone(), StoredValue::Object(None)),
            (
                fields["noise1time"].clone(),
                StoredValue::Value(Value::Float(-1.0)),
            ),
            (
                fields["noise1spot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["noise1loudness"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["noise2time"].clone(),
                StoredValue::Value(Value::Float(-1.0)),
            ),
            (
                fields["noise2spot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["noise2loudness"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.instances.insert(
        3,
        [
            (fields["nextPawn"].clone(), StoredValue::Object(None)),
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([10.0, 0.0, 0.0])),
            ),
            (
                fields["bIsPlayer"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["Alertness"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["HearingThreshold"].clone(),
                StoredValue::Value(Value::Float(0.1)),
            ),
            (
                fields["Stimulus"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let mut source_instance = [
        (
            fields["Instigator"].clone(),
            StoredValue::Object(Some(noise)),
        ),
        (fields["Level"].clone(), StoredValue::Object(Some(level))),
        (
            fields["Location"].clone(),
            StoredValue::Value(Value::Vector([1.0, 2.0, 3.0])),
        ),
    ]
    .into_iter()
    .collect::<InstanceState>();

    let hear_noise = object_id(&package, 4);
    runtime.scripts.insert(
        hear_noise.clone(),
        Arc::new(ScriptExport {
            export_index: 4,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 4,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: 10,
                bytes: vec![0x04, 0xe7, 0x1f, b'h', b'e', b'a', b'r', b'd', 0, 0x16],
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: 0,
                replication_offset: None,
            }),
        }),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(class_id, None, "HearNoise", 0),
        Some(hear_noise),
    );

    let mut bytes = vec![0x04, 0x62, 0x00, 0x1e];
    bytes.extend(1.0_f32.to_le_bytes());
    bytes.push(0x16);
    let execute = |runtime: &mut ScriptRuntime,
                   source_instance: &mut InstanceState,
                   actions: &mut Vec<ActorAction>| {
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes: bytes.clone(),
            tokens: Vec::new(),
        };
        Frame::new(&bytecode)
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                runtime.native(
                    1,
                    &class,
                    &package,
                    index,
                    arguments,
                    source_instance,
                    actions,
                    0,
                )
            })
            .unwrap()
    };
    let mut actions = Vec::new();
    assert_eq!(
        execute(&mut runtime, &mut source_instance, &mut actions),
        Value::None
    );
    assert!(
        matches!(
            &actions[..],
            [ActorAction::Log {
                actor: 3,
                message,
                tag: None,
            }] if message == "heard"
        ),
        "{actions:?}"
    );
    let noise_instance = runtime.instances.get(&2).unwrap();
    assert_eq!(
        noise_instance.get(&fields["noise1time"]),
        Some(&StoredValue::Value(Value::Float(1.0)))
    );
    assert_eq!(
        noise_instance.get(&fields["noise1spot"]),
        Some(&StoredValue::Value(Value::Vector([1.0, 2.0, 3.0])))
    );
    assert_eq!(
        noise_instance.get(&fields["noise1loudness"]),
        Some(&StoredValue::Value(Value::Float(1.0)))
    );
    assert_eq!(
        noise_instance.get(&fields["noise2time"]),
        Some(&StoredValue::Value(Value::Float(-1.0)))
    );
    assert_eq!(
        runtime.instances[&3].get(&fields["Stimulus"]),
        Some(&StoredValue::Value(Value::Float(2.0)))
    );

    actions.clear();
    assert_eq!(
        execute(&mut runtime, &mut source_instance, &mut actions),
        Value::None
    );
    assert!(actions.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sound_natives_validate_calls() {
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
    let spec = |index, start, end, distance| NavigationReachSpec {
        index,
        distance,
        start,
        end,
        collision_radius: 40,
        collision_height: 40,
        pruned: false,
    };
    let specs = [
        spec(0, start.clone(), long.clone(), 10),
        spec(1, long, target.clone(), 10),
        spec(2, start.clone(), short.clone(), 3),
        spec(3, short.clone(), target.clone(), 3),
    ];
    assert_eq!(
        next_navigation_step(&specs, &start, &target, 20, 20),
        Some(short)
    );
    assert_eq!(next_navigation_step(&specs, &start, &target, 50, 20), None);
}

#[test]
fn find_path_to_dispatches_numeric_native_through_navigation_graph() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-find-path-to-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    let navigation_path = system.join("Navigation.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();
    fs::write(
        &navigation_path,
        synthetic_runtime_package_for("NavigationPoint"),
    )
    .unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let navigation_package = runtime.packages.load_path(&navigation_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let class_id = object_id(&package, 0);
    let navigation_class_id = object_id(&navigation_package, 0);
    runtime.scripts.insert(
        navigation_class_id.clone(),
        Arc::new(ScriptExport {
            export_index: 0,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 0,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                raw_len: 0,
                bytes: Vec::new(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        }),
    );
    runtime.scripts.insert(
        class_id.clone(),
        Arc::new(ScriptExport {
            export_index: 0,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 0,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                raw_len: 0,
                bytes: Vec::new(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        }),
    );
    let fields = [
        "Location",
        "CollisionRadius",
        "CollisionHeight",
        "CollisionWidth",
        "Rotation",
        "CollideType",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
        "Brush",
        "PrePivot",
        "BaseEyeHeight",
        "bCollideWorld",
        "bCollideWhenPlacing",
        "bStatic",
        "bMovable",
        "Physics",
        "bIsPlayer",
        "bPlayerOnly",
        "Paths",
        "PrunedPaths",
        "bCanDoSpecial",
        "SpecialGoal",
        "RouteCache",
        "bEndPoint",
        "bSpecialCost",
        "ExtraCost",
        "cost",
        "ZoneGravity",
        "ZoneVelocity",
        "ZoneGroundFriction",
        "ZoneFluidFriction",
        "ZoneTerminalVelocity",
        "bWaterZone",
        "bPainZone",
        "DamageType",
        "ReducedDamageType",
        "bCanSwim",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        (
            name,
            ObjectId {
                package: Arc::from("<find-path-to-test>"),
                export_index: index,
            },
        )
    })
    .collect::<HashMap<_, _>>();
    for class in [&class_id, &navigation_class_id] {
        for (name, field) in &fields {
            runtime.fields.insert(
                (class.clone(), name.to_ascii_lowercase()),
                Some(field.clone()),
            );
        }
        runtime
            .fields
            .insert((class.clone(), "mainscale".to_owned()), None);
    }
    let pawn = runtime_actor_id(1);
    let start = runtime_actor_id(2);
    let short = runtime_actor_id(3);
    let long = runtime_actor_id(4);
    let target = runtime_actor_id(5);
    let alternate = navigation_class_id.clone();
    for (actor, object) in [
        (1, pawn),
        (2, start.clone()),
        (3, short.clone()),
        (4, long.clone()),
        (5, target.clone()),
        (6, alternate.clone()),
    ] {
        runtime.actor_classes.insert(
            actor,
            if actor == 1 {
                class_id.clone()
            } else {
                navigation_class_id.clone()
            },
        );
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object);
    }
    let instance = |location| {
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector(location)),
            ),
            (
                fields["CollisionRadius"].clone(),
                StoredValue::Value(Value::Float(5.0)),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(5.0)),
            ),
            (
                fields["CollisionWidth"].clone(),
                StoredValue::Value(Value::Float(5.0)),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["CollideType"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
            (
                fields["bCollideActors"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bBlockActors"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bBlockPlayers"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Brush"].clone(), StoredValue::Object(None)),
            (
                fields["PrePivot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["BaseEyeHeight"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["bCollideWorld"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bCollideWhenPlacing"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bStatic"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bMovable"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(4)),
            ),
            (
                fields["bIsPlayer"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bPlayerOnly"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Paths"].clone(), StoredValue::Array(Vec::new())),
            (
                fields["PrunedPaths"].clone(),
                StoredValue::Array(Vec::new()),
            ),
            (
                fields["bCanDoSpecial"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bCanSwim"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["ReducedDamageType"].clone(),
                StoredValue::Value(Value::NameText("None".to_owned())),
            ),
            (fields["SpecialGoal"].clone(), StoredValue::Object(None)),
            (
                fields["RouteCache"].clone(),
                StoredValue::Array(vec![StoredValue::Object(None); 16]),
            ),
            (
                fields["bEndPoint"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bSpecialCost"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["ExtraCost"].clone(),
                StoredValue::Value(Value::Int(0)),
            ),
            (fields["cost"].clone(), StoredValue::Value(Value::Int(0))),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    runtime.instances.insert(2, instance([0.0; 3]));
    runtime.instances.insert(3, instance([20.0, 0.0, 0.0]));
    runtime.instances.insert(4, instance([60.0, 0.0, 0.0]));
    runtime.instances.insert(5, instance([100.0, 0.0, 0.0]));
    runtime.instances.insert(6, instance([2_000.0, 0.0, 0.0]));
    let mut next_spec_index = 0;
    let mut spec = |start, end, distance| {
        let index = next_spec_index;
        next_spec_index += 1;
        NavigationReachSpec {
            index,
            distance,
            start,
            end,
            collision_radius: 20,
            collision_height: 20,
            pruned: false,
        }
    };
    runtime.reach_specs = vec![
        spec(start.clone(), long.clone(), 10),
        spec(long.clone(), target.clone(), 10),
        spec(start.clone(), short.clone(), 3),
        spec(short.clone(), target.clone(), 3),
    ];
    for (actor, paths) in [(2, vec![0, 2]), (3, vec![3]), (4, vec![1]), (5, vec![])] {
        runtime.instances.get_mut(&actor).unwrap().insert(
            fields["Paths"].clone(),
            StoredValue::Array(
                paths
                    .into_iter()
                    .map(|path| StoredValue::Value(Value::Int(path)))
                    .collect(),
            ),
        );
    }

    let mut bytes = vec![0x04, 0x62, 0x06, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.extend([0x27, 0x28, 0x16]);
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let execute = |runtime: &mut ScriptRuntime, destination, pawn_location, can_do_special| {
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Vector(destination));
        let mut pawn_instance = instance(pawn_location);
        pawn_instance.insert(
            fields["bCanDoSpecial"].clone(),
            StoredValue::Value(Value::Bool(can_do_special)),
        );
        let result = frame.execute(|call, arguments| {
            assert_eq!(call, FunctionCall::Native(0x206));
            runtime.native(
                1,
                &class,
                &package,
                0x206,
                arguments,
                &mut pawn_instance,
                &mut Vec::new(),
                0,
            )
        });
        (result, pawn_instance)
    };
    let expected = runtime.object_handle(target.clone()).unwrap();
    assert_eq!(
        execute(&mut runtime, [100.0, 0.0, 0.0], [0.0; 3], true)
            .0
            .unwrap(),
        Value::Object(expected)
    );
    assert_eq!(
        execute(&mut runtime, [700.0, 0.0, 0.0], [0.0; 3], true)
            .0
            .unwrap(),
        Value::Object(0)
    );
    // The closest destination node is BSP-occluded. The next visible target
    // is routed from the farther endpoint because the nearer node fails the
    // full actorReachable check through its pruned authored reachspec.
    let level = runtime_actor_id(0);
    runtime.actor_classes.insert(0, class_id.clone());
    runtime.object_actors.insert(level.clone(), 0);
    runtime.actor_objects.insert(0, level);
    runtime.instances.insert(
        0,
        [
            (
                fields["ZoneGravity"].clone(),
                Value::Vector([0.0, 0.0, -950.0]),
            ),
            (fields["ZoneVelocity"].clone(), Value::Vector([0.0; 3])),
            (fields["ZoneGroundFriction"].clone(), Value::Float(1.0)),
            (fields["ZoneFluidFriction"].clone(), Value::Float(1.0)),
            (
                fields["ZoneTerminalVelocity"].clone(),
                Value::Float(2_500.0),
            ),
            (fields["bWaterZone"].clone(), Value::Bool(false)),
            (fields["bPainZone"].clone(), Value::Bool(false)),
            (
                fields["DamageType"].clone(),
                Value::NameText("None".to_owned()),
            ),
        ]
        .into_iter()
        .map(|(field, value)| (field, StoredValue::Value(value)))
        .collect(),
    );
    runtime.collision = Some(solid_box_collision());
    runtime.level_info = Some(0);
    runtime.instances.insert(2, instance([-800.0, 0.0, 0.0]));
    runtime.instances.insert(3, instance([100.0, 0.0, 0.0]));
    runtime.instances.insert(4, instance([-700.0, 0.0, 0.0]));
    runtime.instances.insert(5, instance([-250.0, 0.0, 0.0]));
    let mut pruned = spec(target.clone(), start.clone(), 1);
    pruned.pruned = true;
    runtime.reach_specs = vec![
        pruned,
        spec(start.clone(), long.clone(), 1),
        spec(long.clone(), target.clone(), 1),
    ];
    for (actor, paths) in [(2, vec![5]), (3, vec![]), (4, vec![6]), (5, vec![4])] {
        runtime.instances.get_mut(&actor).unwrap().insert(
            fields["Paths"].clone(),
            StoredValue::Array(
                paths
                    .into_iter()
                    .map(|path| StoredValue::Value(Value::Int(path)))
                    .collect(),
            ),
        );
    }
    assert!(
        runtime
            .actor_reachable(1, &class, &instance([-1_300.0, 0.0, 0.0]), 4)
            .unwrap()
    );
    assert_eq!(
        runtime
            .fast_trace_native(
                &class,
                &[
                    Value::Vector([0.0, 0.0, 0.0]),
                    Value::Vector([100.0, 0.0, 0.0]),
                ],
                &instance([-1_300.0, 0.0, 0.0]),
            )
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        runtime
            .fast_trace_native(
                &class,
                &[
                    Value::Vector([0.0, 0.0, 0.0]),
                    Value::Vector([-250.0, 0.0, 0.0]),
                ],
                &instance([-1_300.0, 0.0, 0.0]),
            )
            .unwrap(),
        Value::Bool(true)
    );
    let expected_long = runtime.object_handle(long.clone()).unwrap();
    assert_eq!(
        execute(&mut runtime, [0.0, 0.0, 0.0], [-1_300.0, 0.0, 0.0], true)
            .0
            .unwrap(),
        Value::Object(expected_long)
    );
    let special = ResolvedObject {
        package: Arc::clone(&navigation_package),
        export_index: 1,
    };
    runtime
        .class_defaults
        .insert(navigation_class_id.clone(), InstanceState::default());
    runtime.scripts.insert(
        object_id(&navigation_package, special.export_index),
        Arc::new(ScriptExport {
            export_index: special.export_index,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: 6,
                bytes: vec![0x04, 0x20, 1, 0, 0, 0],
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: 0,
                replication_offset: None,
            }),
        }),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(navigation_class_id, None, "SpecialHandling", 0),
        Some(object_id(&navigation_package, special.export_index)),
    );
    let (result, pawn_instance) =
        execute(&mut runtime, [0.0, 0.0, 0.0], [-1_300.0, 0.0, 0.0], true);
    assert_eq!(result.unwrap(), Value::Object(0));
    assert_eq!(
        pawn_instance.get(&fields["SpecialGoal"]),
        Some(&StoredValue::Object(Some(alternate)))
    );
    assert!(matches!(
        pawn_instance.get(&fields["RouteCache"]),
        Some(StoredValue::Array(entries))
            if entries.iter().all(|entry| matches!(entry, StoredValue::Object(None)))
    ));
    fs::remove_dir_all(root).unwrap();
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
fn set_rotation_uses_move_actor_for_rotated_bounds_bases_and_touches() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-set-rotation-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package_for("TestPawn")).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let class_id = object_id(&package, class.export_index);
    let class_script = |export_index, base_field| {
        Arc::new(ScriptExport {
            export_index,
            class_name: "Class".to_owned(),
            base_field,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Class(openhp1_script::ClassMetadata {
                state: openhp1_script::StateMetadata {
                    probe_mask: 0,
                    ignore_mask: 0,
                    label_table_offset: 0,
                    flags: 0,
                },
                old_record_size: None,
                flags: 0,
                guid: [0; 16],
                dependencies: Vec::new(),
                package_imports: Vec::new(),
                within: None,
                config_name: None,
                defaults_offset: 0,
            }),
        })
    };
    runtime.scripts.insert(
        class_id.clone(),
        class_script(0, ObjectReference::Export(3)),
    );
    runtime.scripts.insert(
        object_id(&package, 3),
        class_script(3, ObjectReference::None),
    );
    assert!(runtime.class_has_name(&class, "Pawn").unwrap());

    let fields = [
        "Location",
        "Rotation",
        "CollisionHeight",
        "CollisionRadius",
        "CollisionWidth",
        "CollideType",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
        "bCollideWorld",
        "bStatic",
        "bMovable",
        "Brush",
        "PrePivot",
        "ViewRotation",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(100 + index)))
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
    runtime
        .fields
        .insert((class_id.clone(), "standingcount".to_owned()), None);

    let instance = |location: [f32; 3], block_actors: bool| {
        [
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector(location)),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["CollisionHeight"].clone(),
                StoredValue::Value(Value::Float(2.0)),
            ),
            (
                fields["CollisionRadius"].clone(),
                StoredValue::Value(Value::Float(10.0)),
            ),
            (
                fields["CollisionWidth"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["CollideType"].clone(),
                StoredValue::Value(Value::Byte(2)),
            ),
            (
                fields["bCollideActors"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bBlockActors"].clone(),
                StoredValue::Value(Value::Bool(block_actors)),
            ),
            (
                fields["bBlockPlayers"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bCollideWorld"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bStatic"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bMovable"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (fields["Brush"].clone(), StoredValue::Object(None)),
            (
                fields["PrePivot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["ViewRotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    for actor in 1..=3 {
        let object = runtime_actor_id(actor);
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object);
        runtime.actor_classes.insert(actor, class_id.clone());
    }
    runtime.next_actor = 4;
    let parent = runtime.actor_objects[&1].clone();
    let mut current_instance = instance([0.0; 3], true);
    runtime
        .instances
        .insert(2, instance([4.0, 0.0, 0.0], false));
    runtime.instances.insert(3, instance([0.0, 6.0, 0.0], true));
    runtime.update_actor_base(2, Some(parent), None).unwrap();

    let execute = |runtime: &mut ScriptRuntime,
                   current_instance: &mut InstanceState,
                   rotation: [i32; 3],
                   actions: &mut Vec<ActorAction>| {
        let mut bytes = vec![0x04, 0x61, 0x2b, 0x22];
        for value in rotation {
            bytes.extend(value.to_le_bytes());
        }
        bytes.push(0x16);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        Frame::new(&bytecode).execute_hosted(|request| match request {
            FrameRequest::Call {
                function,
                arguments,
                ..
            } => match function {
                FunctionCall::Native(index) => {
                    assert_eq!(index, SET_ROTATION);
                    runtime
                        .native(
                            1,
                            &class,
                            &package,
                            index,
                            &arguments,
                            current_instance,
                            actions,
                            0,
                        )
                        .map(FrameResponse::Value)
                }
                _ => panic!("unexpected frame call"),
            },
            _ => panic!("unexpected frame request"),
        })
    };

    let mut actions = Vec::new();
    assert_eq!(
        execute(
            &mut runtime,
            &mut current_instance,
            [0, 16_384, 0],
            &mut actions,
        )
        .unwrap(),
        Value::Bool(false)
    );
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            ActorAction::DispatchEvent {
                actor: 1 | 3,
                event: "Bump",
                ..
            }
        )
    }));
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, ActorAction::SetRotation { .. }))
    );
    assert_eq!(
        current_instance.get(&fields["Rotation"]),
        Some(&StoredValue::Value(Value::Rotator([0; 3])))
    );

    runtime.instances.get_mut(&3).unwrap().insert(
        fields["bBlockActors"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    actions.clear();

    assert_eq!(
        execute(
            &mut runtime,
            &mut current_instance,
            [0, 16_384, 0],
            &mut actions,
        )
        .unwrap(),
        Value::Bool(true)
    );
    assert!(actions.contains(&ActorAction::SetRotation {
        actor: 1,
        rotation: [0, 16_384, 0],
    }));
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            ActorAction::DispatchEvent {
                actor: 1,
                event: "Touch",
                ..
            }
        )
    }));
    assert_eq!(
        current_instance.get(&fields["Rotation"]),
        Some(&StoredValue::Value(Value::Rotator([0, 16_384, 0])))
    );
    assert_eq!(
        runtime.instances[&2].get(&fields["Rotation"]),
        Some(&StoredValue::Value(Value::Rotator([0, 16_384, 0])))
    );
    assert_eq!(
        runtime.instances[&2].get(&fields["ViewRotation"]),
        Some(&StoredValue::Value(Value::Rotator([0, 16_384, 0])))
    );
    let StoredValue::Value(Value::Vector(location)) =
        runtime.instances[&2].get(&fields["Location"]).unwrap()
    else {
        panic!("based actor Location is not a vector");
    };
    assert!(Vec3::from_array(*location).abs_diff_eq(Vec3::new(0.0, 4.0, 0.0), 1.0e-5));

    actions.clear();
    assert_eq!(
        execute(&mut runtime, &mut current_instance, [0; 3], &mut actions,).unwrap(),
        Value::Bool(true)
    );
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            ActorAction::DispatchEvent {
                actor: 1,
                event: "UnTouch",
                ..
            }
        )
    }));
    current_instance.insert(
        fields["bCollideWorld"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime.collision = Some(placement_test_collision(100.0));
    actions.clear();
    assert_eq!(
        execute(
            &mut runtime,
            &mut current_instance,
            [0, 16_384, 0],
            &mut actions,
        )
        .unwrap(),
        Value::Bool(true)
    );
    assert!(actions.contains(&ActorAction::SetRotation {
        actor: 1,
        rotation: [0, 16_384, 0],
    }));
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
