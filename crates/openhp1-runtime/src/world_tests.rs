use std::{
    cell::RefCell,
    fs,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use openhp1_script::{Bytecode, FunctionMetadata, ScriptExport, ScriptMetadata, Token};

use crate::{
    ConsoleCommandAction, ConsoleCommands, Frame, FrameRequest, FrameResponse, FunctionCall,
    frame::StructMember,
};

use super::*;
use super::{
    actor::advance_lifespan,
    actor::decode_latent_action,
    actor::update_touching_array,
    execution::portable_call_value,
    native::{
        animation_parameters, bone_number, bone_position, collision_updates, log_arguments,
        next_navigation_step, noise_loudness, random_float, random_int, random_unit_vector,
        scalar_native, sound_arguments, target_score,
    },
    state::{event_disabled, probe_event_index, set_event_disabled},
};
use openhp1_map::{BspNode, BspSurface, BspVertex, Model, PolyFlags, PrimitiveBounds, Zone};
use openhp1_physics::BspCollision;

static FIXTURE_ROOT: AtomicUsize = AtomicUsize::new(0);

fn synthetic_runtime_package() -> Vec<u8> {
    synthetic_runtime_package_for("PlayerPawn")
}

fn synthetic_class_script(export_index: usize) -> Arc<ScriptExport> {
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
}

fn synthetic_runtime_package_for(class_name: &str) -> Vec<u8> {
    synthetic_runtime_package_with_member(class_name, "ClientTravel")
}

fn synthetic_runtime_package_with_member(class_name: &str, member_name: &str) -> Vec<u8> {
    synthetic_runtime_package_with_extras(class_name, member_name, &[], false)
}

fn synthetic_mover_runtime_package() -> Vec<u8> {
    synthetic_runtime_package_with_extras(
        "Mover",
        "BumpMove",
        &[
            b"IsRelevantToMover\0",
            b"State\0",
            b"Move\0",
            b"None\0",
            b"DoOpen\0",
            b"FinishedOpening\0",
        ],
        true,
    )
}

fn synthetic_mover_projectile_package() -> Vec<u8> {
    synthetic_runtime_package_with_extras(
        "Projectile",
        "ClientTravel",
        &[b"IsRelevantToMover\0"],
        false,
    )
}

fn synthetic_runtime_package_with_extras(
    class_name: &str,
    member_name: &str,
    extra_names: &[&[u8]],
    state_member: bool,
) -> Vec<u8> {
    const HEADER_SIZE: usize = 44;
    let mut class_name = class_name.as_bytes().to_vec();
    class_name.push(0);
    let mut member_name = member_name.as_bytes().to_vec();
    member_name.push(0);
    let names = [
        class_name.as_slice(),
        member_name.as_slice(),
        b"GetPlayerNetworkAddress\0".as_slice(),
        b"Pawn\0".as_slice(),
        b"StopWaiting\0".as_slice(),
        b"QuidHud\0".as_slice(),
        b"Head\0".as_slice(),
        b"ConsoleCommand\0".as_slice(),
        b"CallConsoleCommand\0".as_slice(),
    ];
    let name_offset = HEADER_SIZE;
    let export_offset = name_offset
        + names
            .iter()
            .chain(extra_names)
            .map(|name| name.len() + size_of::<u32>())
            .sum::<usize>();
    let mut bytes = Vec::new();
    bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
    bytes.extend(61_u16.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    for value in [
        i32::try_from(names.len() + extra_names.len()).unwrap(),
        name_offset as i32,
        9 + i32::from(state_member),
        export_offset as i32,
        0,
        export_offset as i32,
        0,
        0,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for name in names.iter().chain(extra_names) {
        bytes.extend(*name);
        bytes.extend(0_u32.to_le_bytes());
    }
    for (class, outer, name) in [
        (0_u8, 0_i32, 0_u8),
        (u8::from(state_member) * 10, 1, 1),
        (0, 1, 2),
        (0, 0, 3),
        (0, 4, 4),
        (0, 0, 5),
        (0, 1, 6),
        (0, 1, 7),
        (0, 1, 8),
    ] {
        bytes.extend([class, 0]);
        bytes.extend(outer.to_le_bytes());
        bytes.push(name);
        bytes.extend(0_u32.to_le_bytes());
        bytes.push(0);
    }
    if state_member {
        bytes.extend([0, 0]);
        bytes.extend(0_i32.to_le_bytes());
        bytes.push(10);
        bytes.extend(0_u32.to_le_bytes());
        bytes.push(0);
    }
    bytes
}

fn synthetic_particle_parent_package() -> Vec<u8> {
    const HEADER_SIZE: usize = 44;
    let names = ["ParticleFX", "ChildParticleFX"];
    let name_offset = HEADER_SIZE;
    let names_len = names.iter().map(|name| name.len() + 5).sum::<usize>();
    let export_offset = name_offset + names_len;
    let class_payload = |base| {
        let mut payload = Vec::new();
        for reference in [base, 0, 0, 0, 0] {
            payload.extend(compact_index(reference));
        }
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u16.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend([0; 16]);
        for _ in 0..5 {
            payload.extend(compact_index(0));
        }
        payload
    };
    let payloads = [class_payload(0), class_payload(1)];
    let export_table = |payload_offset| {
        let mut table = Vec::new();
        let mut serial_offset = payload_offset;
        for (index, payload) in payloads.iter().enumerate() {
            table.extend(compact_index(0));
            table.extend(compact_index(0));
            table.extend(0_i32.to_le_bytes());
            table.extend(compact_index(index as i32));
            table.extend(0_u32.to_le_bytes());
            table.extend(compact_index(payload.len() as i32));
            table.extend(compact_index(serial_offset as i32));
            serial_offset += payload.len();
        }
        table
    };
    let mut payload_offset = export_offset;
    let export = loop {
        let table = export_table(payload_offset);
        let next = export_offset + table.len();
        if next == payload_offset {
            break table;
        }
        payload_offset = next;
    };
    let mut bytes = Vec::new();
    bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
    bytes.extend(62_u16.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    for value in [
        names.len() as i32,
        name_offset as i32,
        payloads.len() as i32,
        export_offset as i32,
        0,
        export_offset as i32,
        0,
        0,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for name in names {
        bytes.extend(name.as_bytes());
        bytes.push(0);
        bytes.extend(0_u32.to_le_bytes());
    }
    bytes.extend(export);
    assert_eq!(bytes.len(), payload_offset);
    for payload in payloads {
        bytes.extend(payload);
    }
    bytes
}

fn compact_index(value: i32) -> Vec<u8> {
    let negative = value < 0;
    let mut value = value.unsigned_abs();
    let mut bytes = vec![(value as u8 & 0x3f) | if negative { 0x80 } else { 0 }];
    value >>= 6;
    if value != 0 {
        bytes[0] |= 0x40;
    }
    while value != 0 {
        let mut byte = value as u8 & 0x7f;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
    }
    bytes
}

fn synthetic_config_package() -> Vec<u8> {
    let names = [
        "None",
        "DerivedConfig",
        "TestConfig",
        "GlobalBase",
        "ConfigValue",
        "SecondValue",
        "ThirdValue",
        "GlobalValue",
        "Class",
        "IntProperty",
        "Core",
        "User",
        "Mode",
        "ObjectValue",
        "ClassValue",
        "Tint",
        "Tags",
        "TagElement",
        "Priorities",
        "EMode",
        "Color",
        "SomeObject",
        "SomeClass",
        "ByteProperty",
        "ObjectProperty",
        "ClassProperty",
        "StructProperty",
        "ArrayProperty",
        "StrProperty",
        "NameProperty",
        "Enum",
        "Struct",
        "Object",
        "ModeZero",
        "ModeOne",
    ];
    let name_offset = 44;
    let names_len = names.iter().map(|name| name.len() + 5).sum::<usize>();
    let import_offset = name_offset + names_len;
    let mut import = Vec::new();
    for object_name in [9, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32] {
        import.extend(compact_index(10));
        import.extend(compact_index(8));
        import.extend(0_i32.to_le_bytes());
        import.extend(compact_index(object_name));
    }
    let export_offset = import_offset + import.len();
    let class_payload = |base, config_name| {
        let mut payload = Vec::new();
        for reference in [base, 0, 0, 0, 0] {
            payload.extend(compact_index(reference));
        }
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u16.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(CLASS_CONFIG.to_le_bytes());
        payload.extend([0; 16]);
        payload.extend(compact_index(0));
        payload.extend(compact_index(0));
        payload.extend(compact_index(0));
        payload.extend(compact_index(config_name));
        payload.extend(compact_index(0));
        payload
    };
    let property_payload = |array_dimension: i32, flags: u32, references: &[i32]| {
        let mut payload = vec![0, 0, 0];
        payload.extend(array_dimension.to_le_bytes());
        payload.extend(flags.to_le_bytes());
        payload.extend(compact_index(0));
        for reference in references {
            payload.extend(compact_index(*reference));
        }
        payload
    };
    let enum_payload = || {
        let mut payload = vec![0, 0, 0];
        payload.extend(compact_index(2));
        payload.extend(compact_index(33));
        payload.extend(compact_index(34));
        payload
    };
    let payloads = [
        class_payload(2, 0),
        class_payload(3, 11),
        class_payload(0, 0),
        property_payload(1, PROPERTY_CONFIG, &[]),
        property_payload(1, PROPERTY_CONFIG, &[]),
        property_payload(1, PROPERTY_CONFIG, &[]),
        property_payload(1, PROPERTY_GLOBAL_CONFIG, &[]),
        property_payload(1, PROPERTY_CONFIG, &[15]),
        property_payload(1, PROPERTY_CONFIG, &[]),
        property_payload(1, PROPERTY_CONFIG, &[]),
        property_payload(1, PROPERTY_CONFIG, &[16]),
        property_payload(1, PROPERTY_CONFIG, &[13]),
        property_payload(1, 0, &[]),
        property_payload(2, PROPERTY_CONFIG, &[]),
        enum_payload(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    let export_table = |payload_offset| {
        let mut table = Vec::new();
        let mut serial_offset = payload_offset;
        for (index, payload) in payloads.iter().enumerate() {
            let (class, outer, name): (i32, i32, i32) = match index {
                0 => (0, 0, 1),
                1 => (0, 0, 2),
                2 => (0, 0, 3),
                3 => (-1, 2, 4),
                4 => (-1, 2, 5),
                5 => (-1, 2, 6),
                6 => (-1, 3, 7),
                7 => (-2, 1, 12),
                8 => (-3, 1, 13),
                9 => (-4, 1, 14),
                10 => (-5, 1, 15),
                11 => (-6, 1, 16),
                12 => (-7, 0, 17),
                13 => (-8, 1, 18),
                14 => (-9, 0, 19),
                15 => (-10, 0, 20),
                16 => (-11, 0, 21),
                17 => (0, 0, 22),
                _ => unreachable!(),
            };
            table.extend(compact_index(class));
            table.extend(compact_index(0));
            table.extend(outer.to_le_bytes());
            table.extend(compact_index(name));
            table.extend(0_u32.to_le_bytes());
            table.extend(compact_index(payload.len() as i32));
            if !payload.is_empty() {
                table.extend(compact_index(serial_offset as i32));
            }
            serial_offset += payload.len();
        }
        table
    };
    let mut payload_offset = export_offset;
    let export = loop {
        let table = export_table(payload_offset);
        let next = export_offset + table.len();
        if next == payload_offset {
            break table;
        }
        payload_offset = next;
    };
    let mut bytes = Vec::new();
    bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
    bytes.extend(62_u16.to_le_bytes());
    bytes.extend(0_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    for value in [
        names.len() as i32,
        name_offset as i32,
        payloads.len() as i32,
        export_offset as i32,
        11,
        import_offset as i32,
        0,
        0,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for name in names {
        bytes.extend(name.as_bytes());
        bytes.push(0);
        bytes.extend(0_u32.to_le_bytes());
    }
    bytes.extend(import);
    bytes.extend(export);
    assert_eq!(bytes.len(), payload_offset);
    for payload in payloads {
        bytes.extend(payload);
    }
    bytes
}

struct RecordingConsole {
    calls: Rc<RefCell<Vec<(usize, String, String)>>>,
}

impl ConsoleCommandHost for RecordingConsole {
    fn console_command(
        &mut self,
        actor: usize,
        class: &str,
        command: &str,
    ) -> ConsoleCommandResponse {
        self.calls
            .borrow_mut()
            .push((actor, class.to_owned(), command.to_owned()));
        ConsoleCommandResponse {
            output: "host response".to_owned(),
            handled: true,
        }
    }
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

fn run_save_config(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &mut InstanceState,
) {
    let bytecode = Bytecode {
        version: 76,
        bytes: vec![0x04, 0x62, 0x18, 0x16],
        raw_len: 4,
        tokens: Vec::new(),
    };
    let package = Arc::clone(&class.package);
    let mut frame = Frame::new(&bytecode);
    let mut actions = Vec::new();
    assert_eq!(
        frame
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                runtime.native(
                    0,
                    class,
                    &package,
                    index,
                    arguments,
                    instance,
                    &mut actions,
                    0,
                )
            })
            .unwrap(),
        Value::None
    );
}

fn save_snapshot_runtime(root: &std::path::Path, value: i32) -> ScriptRuntime {
    let mut runtime = ScriptRuntime::new(root).unwrap();
    let package = runtime.packages.load("Test").unwrap();
    let class = object_id(&package, 0);
    let actor_object = object_id(&package, 15);
    let property = object_id(&package, 3);
    let animation_frame = object_id(&package, 4);
    let base = object_id(&package, 5);
    let level = object_id(&package, 6);
    runtime.object_handle(actor_object.clone()).unwrap();
    runtime.object_actors.insert(actor_object.clone(), 7);
    runtime.actor_objects.insert(7, actor_object);
    runtime.actor_classes.insert(7, class.clone());
    runtime.fields.insert(
        (class.clone(), "animframe".to_owned()),
        Some(animation_frame.clone()),
    );
    runtime
        .fields
        .insert((class.clone(), "base".to_owned()), Some(base.clone()));
    runtime
        .fields
        .insert((class.clone(), "level".to_owned()), Some(level.clone()));
    runtime.instances.insert(
        7,
        [
            (property.clone(), StoredValue::Value(Value::Int(value))),
            (animation_frame, StoredValue::Value(Value::Float(0.375))),
            (base, StoredValue::Object(None)),
            (level, StoredValue::Object(None)),
            (
                object_id(&package, 10),
                StoredValue::Value(Value::Struct(std::collections::HashMap::from([(
                    "Target".to_owned(),
                    Value::Object(1),
                )]))),
            ),
            (
                object_id(&package, 11),
                StoredValue::Object(Some(host_console_id())),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime
        .actor_states
        .insert(7, Some("SavedState".to_owned()));
    runtime.state_revisions.insert(7, 9);
    set_event_disabled(&mut runtime.disabled_events, 7, None, "Touch", true);
    set_event_disabled(
        &mut runtime.disabled_events,
        7,
        Some("SavedState"),
        "Tick",
        true,
    );
    runtime.state_frames.insert(
        7,
        StateFrame {
            state: class,
            frame: FrameSnapshot::from_save_parts(
                12,
                std::collections::HashMap::from([(5, Value::Object(1))]),
            ),
            latent: LatentAction::FinishAnimation(7),
        },
    );
    runtime.timers.insert(
        7,
        ActorTimer {
            remaining: 1.25,
            rate: 3.0,
            looping: true,
        },
    );
    runtime.random_state = 0x1234_5678;
    runtime.animation_commands.insert(
        7,
        AnimationCommand {
            sequence: "Wave".to_owned(),
            relative_rate: 0.5,
            tween_time: 0.0,
            looping: false,
            tween_only: false,
            root_motion: false,
        },
    );
    runtime.animating.insert(7);
    runtime.sound_channels.insert(
        (7, 2),
        SoundChannel {
            sound: object_id(&package, 15),
            remaining: 2.0,
            pitch: 1.0,
        },
    );
    runtime
}

#[test]
fn hp_menu_save_accepts_transient_audio_and_restores_non_player_state() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-save-snapshot-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    fs::write(system.join("Test.u"), synthetic_config_package()).unwrap();

    let mut console = ConsoleCommands::production(&root, (640, 480), vec![(640, 480)]).unwrap();
    let mut actions = Vec::new();
    let mut state_frames = HashMap::default();
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            7,
            "Console",
            "ConsoleCommand",
            &[Value::String("SaveGame 9".to_owned())],
            &mut actions,
            Some(&mut console),
        ),
        Some(Value::Bool(true))
    );
    assert_eq!(
        console.take_actions(),
        [ConsoleCommandAction::SaveGame { slot: 9 }]
    );

    let snapshot = save_snapshot_runtime(&root, 99)
        .save_game("Maps/Test.unr")
        .unwrap();
    let mut restored = save_snapshot_runtime(&root, -1);
    let actions = restored.restore_game("maps/test.unr", &snapshot).unwrap();
    let package = restored.packages.load("Test").unwrap();
    let property = object_id(&package, 3);
    assert_eq!(
        restored.instances[&7][&property],
        StoredValue::Value(Value::Int(99))
    );
    assert_eq!(
        restored.instances[&7][&object_id(&package, 11)],
        StoredValue::Object(Some(host_console_id()))
    );
    let StoredValue::Value(Value::Struct(target)) =
        &restored.instances[&7][&object_id(&package, 10)]
    else {
        panic!("nested object handle did not restore");
    };
    let Value::Object(target) = target["Target"] else {
        panic!("nested target is not an object");
    };
    assert_eq!(
        restored.object_for_handle(target).unwrap(),
        object_id(&package, 15)
    );
    assert_eq!(restored.actor_states[&7].as_deref(), Some("SavedState"));
    assert_eq!(restored.state_revisions[&7], 9);
    assert!(event_disabled(&restored.disabled_events, 7, None, "Touch"));
    assert!(event_disabled(
        &restored.disabled_events,
        7,
        Some("SavedState"),
        "Tick"
    ));
    assert_eq!(
        restored.timers[&7],
        ActorTimer {
            remaining: 1.25,
            rate: 3.0,
            looping: true,
        }
    );
    assert!(matches!(
        restored.state_frames[&7].latent,
        LatentAction::FinishAnimation(7)
    ));
    assert_eq!(restored.random_state, 0x1234_5678);
    assert!(restored.animating.contains(&7));
    assert!(restored.sound_channels.is_empty());
    assert!(actions.iter().any(|action| {
        matches!(
            action,
            ActorAction::RestoreAnimation {
                actor: 7,
                sequence,
                phase,
                looping: false,
                ..
            } if sequence == "Wave" && *phase == 0.375
        )
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn intrinsic_class_field_returns_the_runtime_actor_class() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-intrinsic-class-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(
        &package_path,
        synthetic_runtime_package_with_member("Object", "Class"),
    )
    .unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = object_id(&package, 3);
    runtime.actor_classes.insert(7, class.clone());
    let expected = runtime.object_handle(class).unwrap();

    assert_eq!(
        runtime
            .context_field_value(7, -1, &package, 2, &InstanceState::default())
            .unwrap(),
        Value::Object(expected)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn player_console_and_menu_bridges_emit_authored_host_actions() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-player-console-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    for (file, class, member) in [
        ("Player.u", "PlayerPawn", "Player"),
        ("Console.u", "baseConsole", "Console"),
        ("Save.u", "HPConsole", "SaveSelectedSlot"),
        ("Load.u", "HPConsole", "LoadSelectedSlot"),
        ("Travel.u", "HPConsole", "ChangeLevel"),
        ("SpaceFlag.u", "baseConsole", "bSpaceReleased"),
        ("Book.u", "HPConsole", "menuBook"),
        ("Page.u", "FEBook", "QuidMatchPage"),
        ("Unlock.u", "FEQuidMatchPage", "UnlockQuidditch"),
        ("Finish.u", "FEQuidMatchPage", "FinishGame"),
    ] {
        fs::write(
            system.join(file),
            synthetic_runtime_package_with_member(class, member),
        )
        .unwrap();
    }

    let console = ConsoleCommands::headless(&root).unwrap();
    let mut runtime = ScriptRuntime::new(&root).unwrap();
    runtime.set_console_command_host(console.clone());
    runtime.player_actor = Some(7);
    runtime.actor_objects.insert(7, runtime_actor_id(7));
    let player_package = runtime.packages.load_path(system.join("Player.u")).unwrap();
    let console_package = runtime
        .packages
        .load_path(system.join("Console.u"))
        .unwrap();
    let player_class = ResolvedObject {
        package: Arc::clone(&player_package),
        export_index: 0,
    };
    let instance = InstanceState::default();

    let player = runtime
        .context_field_value(7, -1, &player_package, 2, &instance)
        .unwrap();
    let Value::Object(player) = player else {
        panic!("Player did not resolve to the host bridge");
    };
    let console_handle = runtime
        .context_field_value(7, player, &console_package, 2, &instance)
        .unwrap();
    let Value::Object(console_handle) = runtime
        .dynamic_cast(&player_class, &console_package, 1, console_handle)
        .unwrap()
    else {
        panic!("baseConsole cast rejected the host bridge");
    };

    let space_flag = runtime
        .packages
        .load_path(system.join("SpaceFlag.u"))
        .unwrap();
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();
    runtime
        .set_context_field(
            7,
            console_handle,
            &space_flag,
            2,
            Value::Bool(true),
            &mut instance,
            &mut actions,
        )
        .unwrap();
    assert_eq!(
        runtime
            .context_field_value(7, console_handle, &space_flag, 2, &instance)
            .unwrap(),
        Value::Bool(true)
    );

    for (file, expected) in [
        ("Save.u", ConsoleCommandAction::SaveGame { slot: 99 }),
        ("Load.u", ConsoleCommandAction::OpenSave { slot: 99 }),
    ] {
        let source = runtime.packages.load_path(system.join(file)).unwrap();
        runtime
            .dispatch_context_call(
                7,
                &player_class,
                console_handle,
                &source,
                FunctionCall::Virtual(1),
                &[],
                &mut instance,
                &mut actions,
                0,
            )
            .unwrap();
        assert_eq!(console.take_actions(), [expected]);
    }
    assert!(actions.is_empty());

    let source = runtime.packages.load_path(system.join("Travel.u")).unwrap();
    runtime
        .dispatch_context_call(
            7,
            &player_class,
            console_handle,
            &source,
            FunctionCall::Virtual(1),
            &[Value::String("Lev_Tut2?peer".to_owned()), Value::Bool(true)],
            &mut instance,
            &mut actions,
            0,
        )
        .unwrap();
    assert_eq!(
        actions,
        [ActorAction::ClientTravel {
            actor: 7,
            url: "Lev_Tut2?peer".to_owned(),
            travel_type: 0,
            transfer_items: true,
        }]
    );
    actions.clear();

    let book_source = runtime.packages.load_path(system.join("Book.u")).unwrap();
    let book = runtime
        .context_field_value(7, console_handle, &book_source, 2, &instance)
        .unwrap();
    let page_source = runtime.packages.load_path(system.join("Page.u")).unwrap();
    let Value::Object(book) = runtime
        .dynamic_cast(&player_class, &page_source, 1, book)
        .unwrap()
    else {
        panic!("FEBook cast rejected the host bridge");
    };
    let page = runtime
        .context_field_value(7, book, &page_source, 2, &instance)
        .unwrap();
    let unlock_source = runtime.packages.load_path(system.join("Unlock.u")).unwrap();
    let Value::Object(page) = runtime
        .dynamic_cast(&player_class, &unlock_source, 1, page)
        .unwrap()
    else {
        panic!("FEQuidMatchPage cast rejected the host bridge");
    };
    runtime
        .dispatch_context_call(
            7,
            &player_class,
            page,
            &unlock_source,
            FunctionCall::Virtual(1),
            &[Value::String("Broom".to_owned())],
            &mut instance,
            &mut actions,
            0,
        )
        .unwrap();
    assert_eq!(
        actions,
        [ActorAction::UnlockQuidditch { actor: 7, level: 1 }]
    );
    actions.clear();
    let finish_source = runtime.packages.load_path(system.join("Finish.u")).unwrap();
    runtime
        .dispatch_context_call(
            7,
            &player_class,
            page,
            &finish_source,
            FunctionCall::Virtual(1),
            &[Value::Int(170), Value::Int(80)],
            &mut instance,
            &mut actions,
            0,
        )
        .unwrap();
    assert_eq!(
        actions,
        [ActorAction::FinishQuidditchMatch {
            actor: 7,
            team0_score: 170,
            opponent_score: 80,
        }]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_snapshot_rejects_truncated_and_unknown_versions() {
    assert!(matches!(
        ScriptRuntime::saved_game_map(b"OHPS"),
        Err(DispatchError::SaveState { .. })
    ));
    assert!(matches!(
        ScriptRuntime::saved_game_map(b"OHPS\x02\0"),
        Err(DispatchError::SaveState { .. })
    ));
}

#[test]
fn save_config_native_persists_config_properties_without_mutating_default_ini() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-save-config-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    let default_ini =
        "[Core.System]\nPaths=*.u\n\n[Keep]\nValue=default\n\n[Test.GlobalBase]\nGlobalValue=8\n";
    let def_user_ini = "[DefaultPlayer]\nName=Player\n\n[Test.DerivedConfig]\nConfigValue=7\n";
    fs::write(system.join("Default.ini"), default_ini).unwrap();
    fs::write(system.join("DefUser.ini"), def_user_ini).unwrap();
    fs::write(system.join("HP.exe"), []).unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_config_package()).unwrap();

    let settings = root.join("settings");
    let mut runtime = ScriptRuntime::new_with_settings_dir(&root, &settings).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let mut instance = runtime.load_class_defaults(&class, 0).unwrap();
    assert_eq!(
        instance.get(&object_id(&package, 3)),
        Some(&StoredValue::Value(Value::Int(7)))
    );
    assert_eq!(
        instance.get(&object_id(&package, 6)),
        Some(&StoredValue::Value(Value::Int(8)))
    );
    instance.insert(object_id(&package, 3), StoredValue::Value(Value::Int(42)));
    instance.insert(object_id(&package, 4), StoredValue::Value(Value::Int(31)));
    instance.insert(object_id(&package, 5), StoredValue::Value(Value::Int(32)));
    instance.insert(object_id(&package, 6), StoredValue::Value(Value::Int(99)));
    instance.insert(object_id(&package, 7), StoredValue::Value(Value::Byte(1)));
    instance.insert(
        object_id(&package, 8),
        StoredValue::Object(Some(object_id(&package, 16))),
    );
    instance.insert(
        object_id(&package, 9),
        StoredValue::Object(Some(object_id(&package, 17))),
    );
    instance.insert(
        object_id(&package, 10),
        StoredValue::Value(Value::Struct(std::collections::HashMap::from([
            ("R".to_owned(), Value::Byte(1)),
            ("G".to_owned(), Value::Byte(2)),
            ("B".to_owned(), Value::Byte(3)),
            ("A".to_owned(), Value::Byte(4)),
        ]))),
    );
    instance.insert(
        object_id(&package, 11),
        StoredValue::Array(vec![StoredValue::Value(Value::String(
            "FirstTag".to_owned(),
        ))]),
    );
    instance.insert(
        object_id(&package, 13),
        StoredValue::Array(vec![
            StoredValue::Name("First".to_owned()),
            StoredValue::Name("Second".to_owned()),
        ]),
    );
    run_save_config(&mut runtime, &class, &mut instance);
    let user_ini = fs::read_to_string(settings.join("User.ini")).unwrap();
    assert!(user_ini.contains("[DefaultPlayer]\nName=Player"));
    assert_eq!(user_ini.matches("[Test.DerivedConfig]").count(), 1);
    assert!(user_ini.contains("ConfigValue=42"));
    assert!(user_ini.contains("SecondValue=31"));
    assert!(user_ini.contains("ThirdValue=32"));
    assert!(user_ini.contains("Mode=ModeOne"));
    assert!(user_ini.contains("ObjectValue=Test.SomeObject"));
    assert!(user_ini.contains("ClassValue=Test.SomeClass"));
    assert!(user_ini.contains("Tint=(R=1,G=2,B=3,A=4)"));
    assert!(user_ini.contains("Tags=FirstTag"));
    assert!(user_ini.contains("Priorities[0]=First\nPriorities[1]=Second"));
    let hp_ini = fs::read_to_string(settings.join("HP.ini")).unwrap();
    assert!(hp_ini.contains("[Test.GlobalBase]\nGlobalValue=99"));

    instance.insert(object_id(&package, 3), StoredValue::Value(Value::Int(43)));
    instance.insert(object_id(&package, 4), StoredValue::Value(Value::Int(33)));
    instance.insert(object_id(&package, 5), StoredValue::Value(Value::Int(34)));
    instance.insert(object_id(&package, 6), StoredValue::Value(Value::Int(100)));
    instance.insert(object_id(&package, 7), StoredValue::Value(Value::Byte(0)));
    instance.insert(
        object_id(&package, 10),
        StoredValue::Value(Value::Struct(std::collections::HashMap::from([
            ("R".to_owned(), Value::Byte(5)),
            ("G".to_owned(), Value::Byte(6)),
            ("B".to_owned(), Value::Byte(7)),
            ("A".to_owned(), Value::Byte(8)),
        ]))),
    );
    instance.insert(
        object_id(&package, 11),
        StoredValue::Array(vec![
            StoredValue::Value(Value::String("FinalTag".to_owned())),
            StoredValue::Value(Value::String("SecondTag".to_owned())),
        ]),
    );
    run_save_config(&mut runtime, &class, &mut instance);
    let user_ini = fs::read_to_string(settings.join("User.ini")).unwrap();
    assert_eq!(user_ini.matches("[Test.DerivedConfig]").count(), 1);
    assert!(user_ini.contains("ConfigValue=43"));
    assert!(user_ini.contains("SecondValue=33"));
    assert!(user_ini.contains("ThirdValue=34"));
    assert!(!user_ini.contains("ConfigValue=42"));
    assert!(user_ini.contains("Mode=ModeZero"));
    assert!(!user_ini.contains("Mode=ModeOne"));
    assert!(user_ini.contains("Tint=(R=5,G=6,B=7,A=8)"));
    assert!(user_ini.contains("Tags=FinalTag\nTags=SecondTag"));
    assert!(!user_ini.contains("Tags=FirstTag"));
    let hp_ini = fs::read_to_string(settings.join("HP.ini")).unwrap();
    assert!(hp_ini.contains("[Test.GlobalBase]\nGlobalValue=100"));
    assert!(!hp_ini.contains("GlobalValue=99"));

    let defaults = runtime.load_class_defaults(&class, 0).unwrap();
    assert_eq!(
        defaults.get(&object_id(&package, 3)),
        Some(&StoredValue::Value(Value::Int(43)))
    );
    assert_eq!(
        defaults.get(&object_id(&package, 6)),
        Some(&StoredValue::Value(Value::Int(100)))
    );

    let casefolded_ini = user_ini
        .replace("ObjectValue=Test.SomeObject", "ObjectValue=tEsT.sOmEoBjEcT")
        .replace("ClassValue=Test.SomeClass", "ClassValue=tEsT.sOmEcLaSs");
    fs::write(settings.join("User.ini"), &casefolded_ini).unwrap();
    let mut fresh_runtime = ScriptRuntime::new_with_settings_dir(&root, &settings).unwrap();
    let fresh_package = fresh_runtime.packages.load_path(&package_path).unwrap();
    let fresh_class = ResolvedObject {
        package: Arc::clone(&fresh_package),
        export_index: 0,
    };
    let fresh_defaults = fresh_runtime.load_class_defaults(&fresh_class, 0).unwrap();
    for (index, value) in [(3, 43), (4, 33), (5, 34), (6, 100)] {
        assert_eq!(
            fresh_defaults.get(&object_id(&fresh_package, index)),
            Some(&StoredValue::Value(Value::Int(value)))
        );
    }
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 7)),
        Some(&StoredValue::Value(Value::Byte(0)))
    );
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 8)),
        Some(&StoredValue::Object(Some(object_id(&fresh_package, 16))))
    );
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 9)),
        Some(&StoredValue::Object(Some(object_id(&fresh_package, 17))))
    );
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 10)),
        Some(&StoredValue::Value(Value::Struct(
            std::collections::HashMap::from([
                ("R".to_owned(), Value::Byte(5)),
                ("G".to_owned(), Value::Byte(6)),
                ("B".to_owned(), Value::Byte(7)),
                ("A".to_owned(), Value::Byte(8)),
            ])
        )))
    );
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 11)),
        Some(&StoredValue::Array(vec![
            StoredValue::Value(Value::String("FinalTag".to_owned())),
            StoredValue::Value(Value::String("SecondTag".to_owned())),
        ]))
    );
    assert_eq!(
        fresh_defaults.get(&object_id(&fresh_package, 13)),
        Some(&StoredValue::Array(vec![
            StoredValue::Name("First".to_owned()),
            StoredValue::Name("Second".to_owned()),
        ]))
    );
    fs::write(
        settings.join("User.ini"),
        casefolded_ini.replacen("Mode=ModeZero", "Mode=NotAMode", 1),
    )
    .unwrap();
    let mut malformed_runtime = ScriptRuntime::new_with_settings_dir(&root, &settings).unwrap();
    let malformed_package = malformed_runtime.packages.load_path(&package_path).unwrap();
    let malformed_class = ResolvedObject {
        package: Arc::clone(&malformed_package),
        export_index: 0,
    };
    assert!(matches!(
        malformed_runtime.load_class_defaults(&malformed_class, 0),
        Err(DispatchError::InvalidConfigValue { property, .. }) if property == "Mode"
    ));
    assert_eq!(
        fs::read_to_string(system.join("Default.ini")).unwrap(),
        default_ini
    );
    assert_eq!(
        fs::read_to_string(system.join("DefUser.ini")).unwrap(),
        def_user_ini
    );
    assert!(!system.join("User.ini").exists());
    assert!(!system.join("HP.ini").exists());
    assert!(fs::read_dir(&settings).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")
    }));
    fs::remove_dir_all(root).unwrap();
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
fn call_arguments_make_package_local_names_portable() {
    let package = Package::parse(
        "Caller.u",
        synthetic_runtime_package_with_extras("PlayerPawn", "TriggerEvent", &[b"Intro\0"], false)
            .into(),
    )
    .unwrap();
    let intro = package
        .summary()
        .names
        .iter()
        .position(|name| name.value.eq_ignore_ascii_case("Intro"))
        .unwrap() as i32;
    let value = Value::Array(vec![Value::Name(intro)]);

    assert_eq!(
        portable_call_value(&package, &value).unwrap(),
        Value::Array(vec![Value::NameText("Intro".to_owned())])
    );
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
fn player_ui_state_reads_the_authored_harry_counters() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-player-ui-{}-{}",
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
    let class_id = object_id(&package, 0);
    runtime
        .scripts
        .insert(class_id.clone(), synthetic_class_script(0));
    let names = [
        "lifePotions",
        "MaxLifePotions",
        "numBeans",
        "numStars",
        "iFireSeedCount",
        "WizardCards",
        "maxPointsPerHouse",
        "numHousePointsHarry",
        "numHousePointsGryffindor",
        "numHousePointsSlytherin",
        "numHousePointsHufflepuff",
        "numHousePointsRavenclaw",
    ];
    let fields = names
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

    let player = 7;
    runtime.actor_classes.insert(player, class_id);
    runtime.player_actor = Some(player);
    let mut instance = InstanceState::default();
    for (name, value) in [
        ("lifePotions", Value::Float(3.0)),
        ("MaxLifePotions", Value::Float(4.0)),
        ("numBeans", Value::Int(17)),
        ("numStars", Value::Int(4)),
        ("iFireSeedCount", Value::Int(6)),
        ("maxPointsPerHouse", Value::Int(500)),
        ("numHousePointsHarry", Value::Int(25)),
        ("numHousePointsGryffindor", Value::Int(42)),
        ("numHousePointsSlytherin", Value::Int(18)),
        ("numHousePointsHufflepuff", Value::Int(12)),
        ("numHousePointsRavenclaw", Value::Int(15)),
    ] {
        instance.insert(fields[name].clone(), StoredValue::Value(value));
    }
    let card = |id, has_card| {
        StoredValue::Value(Value::Struct(
            [
                ("ID".to_owned(), Value::Int(id)),
                ("bHasCard".to_owned(), Value::Bool(has_card)),
            ]
            .into_iter()
            .collect(),
        ))
    };
    instance.insert(
        fields["WizardCards"].clone(),
        StoredValue::Array(vec![card(101, true), card(2, false), card(69, true)]),
    );
    runtime.instances.insert(player, instance);

    assert_eq!(
        runtime.player_ui_state().unwrap(),
        PlayerUiState {
            health: 0.75,
            beans: 17,
            stars: 4,
            fire_seeds: 6,
            cards: 2,
            wizard_cards: {
                let mut cards = [None; 25];
                cards[0] = Some(101);
                cards[2] = Some(69);
                cards
            },
            max_points_per_house: 500,
            house_points_harry: 25,
            house_points_gryffindor: 42,
            house_points_slytherin: 18,
            house_points_hufflepuff: 12,
            house_points_ravenclaw: 15,
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn player_input_populates_broom_channels() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-broom-input-{}-{}",
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
        .scripts
        .insert(class_id.clone(), synthetic_class_script(class.export_index));
    let fields = [
        "aBaseX",
        "aBaseY",
        "aStrafe",
        "aMouseX",
        "aMouseY",
        "aBroomYaw",
        "aBroomPitch",
        "bAltFire",
        "bBroomYawLeft",
        "bBroomYawRight",
        "bBroomPitchUp",
        "bBroomPitchDown",
        "bBroomBoost",
        "bBroomBrake",
        "bBroomAction",
        "bPressedJump",
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

    let player = 7;
    let player_object = runtime_actor_id(player);
    runtime.actor_classes.insert(player, class_id);
    runtime.object_actors.insert(player_object.clone(), player);
    runtime.actor_objects.insert(player, player_object);
    runtime.player_actor = Some(player);
    runtime.instances.insert(player, InstanceState::default());

    runtime
        .set_player_input(PlayerInput {
            base_x: -3_000.0,
            base_y: 6_000.0,
            mouse_x: 192.0,
            mouse_y: -96.0,
            jump: true,
            broom_pitch_up: true,
            broom_boost: true,
            broom_brake: true,
            ..PlayerInput::default()
        })
        .unwrap();
    let instance = &runtime.instances[&player];
    for (name, value) in [
        ("aBroomYaw", Value::Float(192.0)),
        ("aBroomPitch", Value::Float(96.0)),
        ("bBroomYawLeft", Value::Byte(1)),
        ("bBroomYawRight", Value::Byte(0)),
        ("bBroomPitchUp", Value::Byte(1)),
        ("bBroomPitchDown", Value::Byte(0)),
        ("bBroomBoost", Value::Byte(1)),
        ("bBroomBrake", Value::Byte(1)),
        ("bBroomAction", Value::Byte(1)),
    ] {
        assert_eq!(
            instance.get(&fields[name]),
            Some(&StoredValue::Value(value))
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn carried_actor_space_input_dispatches_alt_fire_after_updating_weapon_pose() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-carried-actor-input-{}-{}",
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
    let class_id = object_id(&package, 0);
    runtime
        .scripts
        .insert(class_id.clone(), synthetic_class_script(0));
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());
    runtime
        .scripts
        .insert(object_id(&package, 1), named_native_script(1));
    let mut bytes = vec![0x04, 0x1c];
    bytes.extend(2_i32.to_le_bytes());
    bytes.push(0x1f);
    bytes.extend(b"Thrown\0");
    bytes.extend([0x24, 0x00, 0x28, 0x16]);
    runtime.scripts.insert(
        object_id(&package, 2),
        Arc::new(ScriptExport {
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
        FunctionLookup::new(class_id.clone(), None, "AltFire", 0),
        Some(object_id(&package, 2)),
    );

    let fields = [
        "aBaseX",
        "aBaseY",
        "aStrafe",
        "aMouseX",
        "aMouseY",
        "aBroomYaw",
        "aBroomPitch",
        "bAltFire",
        "bBroomYawLeft",
        "bBroomYawRight",
        "bBroomPitchUp",
        "bBroomPitchDown",
        "bBroomBoost",
        "bBroomBrake",
        "bBroomAction",
        "bPressedJump",
        "aForward",
        "aTurn",
        "aLookUp",
        "CarryingActor",
        "WeaponLoc",
        "WeaponRot",
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

    let player = 7;
    let player_object = runtime_actor_id(player);
    runtime.actor_classes.insert(player, class_id);
    runtime.object_actors.insert(player_object.clone(), player);
    runtime.actor_objects.insert(player, player_object);
    runtime.player_actor = Some(player);
    runtime.instances.insert(
        player,
        [(fields["CarryingActor"].clone(), StoredValue::Object(None))]
            .into_iter()
            .collect(),
    );

    runtime
        .set_actor_weapon_pose(player, [10.0, 20.0, 30.0], [4_096, 8_192, -2_048])
        .unwrap();
    assert!(
        runtime
            .tick_player(
                PlayerInput {
                    space_pressed: true,
                    ..PlayerInput::default()
                },
                1.0 / 60.0,
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        runtime.host_console_instance.get("bspacepressed"),
        Some(&StoredValue::Value(Value::Bool(true)))
    );
    assert!(
        runtime
            .tick_player(
                PlayerInput {
                    space_released: true,
                    ..PlayerInput::default()
                },
                1.0 / 60.0,
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        runtime.host_console_instance.get("bspacepressed"),
        Some(&StoredValue::Value(Value::Bool(false)))
    );
    runtime.instances.get_mut(&player).unwrap().insert(
        fields["CarryingActor"].clone(),
        StoredValue::Object(Some(runtime_actor_id(99))),
    );
    assert!(
        runtime
            .tick_player(PlayerInput::default(), 1.0 / 60.0)
            .unwrap()
            .is_empty()
    );
    let actions = runtime
        .tick_player(
            PlayerInput {
                space_pressed: true,
                ..PlayerInput::default()
            },
            1.0 / 60.0,
        )
        .unwrap();
    assert!(
        matches!(
            actions.as_slice(),
            [ActorAction::ClientTravel { url, .. }] if url == "Thrown"
        ),
        "{actions:?}"
    );
    let instance = &runtime.instances[&player];
    assert_eq!(
        instance.get(&fields["WeaponLoc"]),
        Some(&StoredValue::Value(Value::Vector([10.0, 20.0, 30.0])))
    );
    assert_eq!(
        instance.get(&fields["WeaponRot"]),
        Some(&StoredValue::Value(Value::Rotator([4_096, 8_192, -2_048])))
    );
    assert!(
        runtime
            .tick_player(PlayerInput::default(), 1.0 / 60.0)
            .unwrap()
            .is_empty()
    );
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
    let player_class_id = object_id(&package, player_class.export_index);
    let hud_class_id = object_id(&package, hud_class.export_index);
    runtime.scripts.insert(
        player_class_id.clone(),
        synthetic_class_script(player_class.export_index),
    );
    runtime.scripts.insert(
        hud_class_id.clone(),
        synthetic_class_script(hud_class.export_index),
    );
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
        zones: vec![
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
        ],
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
        zones: [1; 2],
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
fn spawn_bytecode_uses_bsp_find_spot_before_allocating_a_handle() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-spawn-placement-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_runtime_package()).unwrap();

    let collision_box = |minimum: Vec3, maximum: Vec3| {
        let planes = [
            [0.0, 1.0, 0.0, maximum.y],
            [1.0, 0.0, 0.0, maximum.x],
            [-1.0, 0.0, 0.0, -minimum.x],
            [0.0, -1.0, 0.0, -minimum.y],
            [0.0, 0.0, 1.0, maximum.z],
            [0.0, 0.0, -1.0, -minimum.z],
        ];
        let mut leaf_hulls = vec![0, 1, 2, 3, 4, 5, -1];
        leaf_hulls.extend(
            [
                minimum.x, minimum.y, minimum.z, maximum.x, maximum.y, maximum.z,
            ]
            .map(f32::to_bits)
            .map(|value| value as i32),
        );
        Arc::new(
            BspCollision::from_model(&Model {
                bounds: PrimitiveBounds {
                    minimum,
                    maximum,
                    valid: true,
                    sphere: [0.0; 4],
                },
                vectors: Vec::new(),
                points: vec![
                    Vec3::new(minimum.x, maximum.y, minimum.z),
                    Vec3::new(maximum.x, maximum.y, minimum.z),
                    Vec3::new(maximum.x, maximum.y, maximum.z),
                    Vec3::new(minimum.x, maximum.y, maximum.z),
                ],
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
                        collision_bound: if index == 0 { 0 } else { -1 },
                        render_bound: -1,
                        zones: [0; 2],
                        vertex_count: if index == 0 { 4 } else { 0 },
                        leaves: [0; 2],
                    })
                    .collect(),
                surfaces: Vec::new(),
                vertices: (0..4).map(|point| BspVertex { point, side: -1 }).collect(),
                shared_side_count: 0,
                zones: Vec::new(),
                polys: ObjectReference::None,
                light_maps: Vec::new(),
                light_bits: Vec::new(),
                collision_bounds: Vec::new(),
                leaf_hulls,
                leaves: Vec::new(),
                lights: Vec::new(),
                root_outside: true,
                linked: false,
            })
            .unwrap(),
        )
    };

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let source_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 5,
    };
    let spawned_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let spawned_class_id = object_id(&package, spawned_class.export_index);
    runtime.scripts.insert(
        spawned_class_id.clone(),
        Arc::new(ScriptExport {
            export_index: spawned_class.export_index,
            class_name: "Class".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: spawned_class.export_index,
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
    for event in [
        "Tick",
        "Spawned",
        "PreBeginPlay",
        "BeginPlay",
        "PostBeginPlay",
        "SetInitialState",
    ] {
        runtime.function_lookups.insert(
            FunctionLookup::new(spawned_class_id.clone(), None, event, 0),
            None,
        );
    }

    let source_fields = ["Location", "Rotation", "Instigator", "Level", "XLevel"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, runtime_actor_id(300 + index)))
        .collect::<HashMap<_, _>>();
    let spawned_fields = [
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
        "CollisionHeight",
        "CollisionRadius",
        "CollisionWidth",
        "CollideType",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
        "Brush",
        "PrePivot",
        "MainScale",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| (name, runtime_actor_id(400 + index)))
    .collect::<HashMap<_, _>>();
    let source_class_id = object_id(&package, source_class.export_index);
    for (name, field) in &source_fields {
        runtime.fields.insert(
            (source_class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    for (name, field) in &spawned_fields {
        runtime.fields.insert(
            (spawned_class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }

    let mut defaults = InstanceState::default();
    for name in ["Instigator", "Level", "XLevel", "Owner", "Brush"] {
        defaults.insert(spawned_fields[name].clone(), StoredValue::Object(None));
    }
    for name in ["Location", "OldLocation", "PrePivot"] {
        defaults.insert(
            spawned_fields[name].clone(),
            StoredValue::Value(Value::Vector([0.0; 3])),
        );
    }
    for name in ["Rotation", "DesiredRotation"] {
        defaults.insert(
            spawned_fields[name].clone(),
            StoredValue::Value(Value::Rotator([0; 3])),
        );
    }
    defaults.insert(
        spawned_fields["Tag"].clone(),
        StoredValue::Name("QuidHud".to_owned()),
    );
    for name in [
        "bCollideWorld",
        "bCollideWhenPlacing",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
    ] {
        defaults.insert(
            spawned_fields[name].clone(),
            StoredValue::Value(Value::Bool(name != "bCollideWorld")),
        );
    }
    for (name, value) in [
        ("CollisionHeight", 5.0),
        ("CollisionRadius", 5.0),
        ("CollisionWidth", 7.0),
    ] {
        defaults.insert(
            spawned_fields[name].clone(),
            StoredValue::Value(Value::Float(value)),
        );
    }
    defaults.insert(
        spawned_fields["CollideType"].clone(),
        StoredValue::Value(Value::Byte(2)),
    );
    runtime
        .class_defaults
        .insert(spawned_class_id.clone(), defaults);
    let mut source_instance = InstanceState::default();
    source_instance.insert(
        source_fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    source_instance.insert(
        source_fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([0; 3])),
    );
    for name in ["Instigator", "Level", "XLevel"] {
        source_instance.insert(source_fields[name].clone(), StoredValue::Object(None));
    }
    let spawned_class_handle = runtime.object_handle(spawned_class_id.clone()).unwrap();
    let mut run_spawn = |runtime: &mut ScriptRuntime, location: [f32; 3]| {
        let mut bytes = vec![0x04, 0x61, 0x16, 0x00];
        bytes.extend(1_i32.to_le_bytes());
        bytes.extend([0x2a, 0x0b, 0x23]);
        for component in location {
            bytes.extend(component.to_le_bytes());
        }
        bytes.push(0x22);
        for component in [0_i32; 3] {
            bytes.extend(component.to_le_bytes());
        }
        bytes.push(0x16);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(1, Value::Object(spawned_class_handle));
        let mut actions = Vec::new();
        let value = frame
            .execute(|call, arguments| {
                let FunctionCall::Native(index) = call else {
                    unreachable!()
                };
                runtime.native(
                    99,
                    &source_class,
                    &package,
                    index,
                    arguments,
                    &mut source_instance,
                    &mut actions,
                    0,
                )
            })
            .unwrap();
        (value, actions)
    };

    runtime.collision = Some(collision_box(
        Vec3::new(-100.0, -1.0, -100.0),
        Vec3::new(100.0, 0.0, 100.0),
    ));
    let (Value::Object(first_handle), first_actions) = run_spawn(&mut runtime, [0.0, -0.95, 0.0])
    else {
        panic!("Spawn did not return an object handle");
    };
    assert_ne!(first_handle, 0);
    let [
        ActorAction::SpawnActor {
            actor: first_actor,
            location: first_location,
            ..
        },
    ] = first_actions.as_slice()
    else {
        panic!("Spawn did not emit its host action");
    };
    assert_eq!(first_location[0], 0.0);
    assert!(first_location[1] < -7.9);
    assert_eq!(first_location[2], 0.0);
    assert!(matches!(
        runtime.instances[first_actor].get(&spawned_fields["OldLocation"]),
        Some(StoredValue::Value(Value::Vector(location))) if location == first_location
    ));

    runtime.collision = Some(collision_box(Vec3::splat(100.0), Vec3::splat(101.0)));
    let (Value::Object(second_handle), _) = run_spawn(&mut runtime, *first_location) else {
        panic!("Spawn did not return an object handle");
    };
    assert_ne!(
        second_handle, 0,
        "another actor must not block Spawn placement"
    );

    let cylinder_corner = {
        let minimum = Vec3::new(0.65, 0.65, -0.5);
        let maximum = Vec3::new(1.25, 1.25, 0.5);
        let planes = [
            [1.0, 1.0, 0.0, 2.2],
            [-1.0, -1.0, 0.0, -1.6],
            [1.0, -1.0, 0.0, 0.3],
            [-1.0, 1.0, 0.0, 0.3],
            [0.0, 0.0, 1.0, 0.5],
            [0.0, 0.0, -1.0, 0.5],
        ];
        let mut leaf_hulls = vec![0, 1, 2, 3, 4, 5, -1];
        leaf_hulls.extend(
            [
                minimum.x, minimum.y, minimum.z, maximum.x, maximum.y, maximum.z,
            ]
            .map(f32::to_bits)
            .map(|value| value as i32),
        );
        Arc::new(
            BspCollision::from_model(&Model {
                bounds: PrimitiveBounds {
                    minimum,
                    maximum,
                    valid: true,
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
                        collision_bound: if index == 0 { 0 } else { -1 },
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
                leaf_hulls,
                leaves: Vec::new(),
                lights: Vec::new(),
                root_outside: true,
                linked: false,
            })
            .unwrap(),
        )
    };
    assert!(cylinder_corner.overlaps_aabb(Vec3::ZERO, Vec3::ONE));
    assert!(!cylinder_corner.overlaps_cylinder(Vec3::ZERO, 1.0, 1.0));
    let defaults = runtime.class_defaults.get_mut(&spawned_class_id).unwrap();
    defaults.insert(
        spawned_fields["CollideType"].clone(),
        StoredValue::Value(Value::Byte(0)),
    );
    for name in ["CollisionHeight", "CollisionRadius", "CollisionWidth"] {
        defaults.insert(
            spawned_fields[name].clone(),
            StoredValue::Value(Value::Float(1.0)),
        );
    }
    runtime.collision = Some(cylinder_corner);
    let (Value::Object(cylinder_handle), cylinder_actions) = run_spawn(&mut runtime, [0.0; 3])
    else {
        panic!("cylinder Spawn did not return an object handle");
    };
    assert_ne!(cylinder_handle, 0);
    assert!(matches!(
        cylinder_actions.as_slice(),
        [ActorAction::SpawnActor { location, .. }] if *location == [0.0; 3]
    ));

    runtime.collision = Some(collision_box(Vec3::splat(-100.0), Vec3::splat(100.0)));
    assert_eq!(run_spawn(&mut runtime, [0.0; 3]).0, Value::Object(0));
    assert_eq!(runtime.next_actor, 3);
    fs::remove_dir_all(root).unwrap();
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
fn zone_zero_is_out_of_world() {
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
        None,
        "UE1 zone zero is outside the world, not LevelInfo fallback",
    );
}

#[test]
fn particle_acceleration_uses_negative_zone_modifier() {
    assert_eq!(
        particle_acceleration(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -100.0), -0.5),
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
                zones: [1; 2],
                vertex_count: 0,
                leaves: [0; 2],
            })
            .collect(),
        surfaces: Vec::new(),
        vertices: Vec::new(),
        shared_side_count: 0,
        zones: vec![
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
        ],
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
    runtime.scripts.insert(
        pawn_class_id.clone(),
        Arc::clone(&runtime.scripts[&navigation_class_id]),
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
        "bKillZone",
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
                fields["bKillZone"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["bPainZone"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (
                fields["DamageType"].clone(),
                StoredValue::Name("None".to_owned()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    runtime.collision = Some(actor_reachable_bsp());
    runtime.level_info = Some(0);
    assert_eq!(
        runtime
            .zone_physics(Vec3::ZERO, 0, &runtime.instances[&0].clone())
            .unwrap()
            .unwrap()
            .damage_type
            .as_deref(),
        Some("None"),
    );
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["bKillZone"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    assert!(
        runtime
            .zone_physics(Vec3::ZERO, 0, &runtime.instances[&0].clone())
            .unwrap()
            .is_none(),
        "authored kill zones must follow the FellOutOfWorld path",
    );
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["bKillZone"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
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

    runtime.instances.insert(0, instance.clone());
    actions.clear();
    let (placed, mut actions) = runtime.place_actor(0, [25.0, 0.0, 0.0]).unwrap();
    assert!(placed);
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::SetLocation {
            actor: 0,
            location: [25.0, 0.0, 0.0],
        }]
    ));
    instance = runtime.instances.remove(&0).unwrap();

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
        zones: vec![
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
            Zone {
                actor: ObjectReference::None,
                connectivity: 0,
                visibility: 0,
            },
        ],
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
        zones: [1; 2],
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
               parameter: u8,
               sound: Option<i32>,
               slot: u8,
               instance: &mut InstanceState,
               actions: &mut Vec<ActorAction>| {
        let mut bytes = vec![0x04, 0x62, 0x37, 0x24, parameter, 0x1e];
        bytes.extend(0.75_f32.to_le_bytes());
        match sound {
            Some(sound) => {
                bytes.push(0x20);
                bytes.extend(sound.to_le_bytes());
            }
            None => bytes.push(0x2a),
        }
        bytes.extend([0x24, slot, 0x16]);
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
        run(&mut runtime, 1, None, 3, &mut instance, &mut actions),
        Value::Bool(false)
    );
    assert!(actions.is_empty());

    let sound = runtime_actor_id(90);
    let sound_handle = runtime.object_handle(sound.clone()).unwrap();
    let other_sound = runtime_actor_id(91);
    let other_sound_handle = runtime.object_handle(other_sound.clone()).unwrap();
    assert!(runtime.start_sound(0, 3, sound.clone(), 1.0, 1.0, false));

    assert_eq!(
        run(&mut runtime, 1, None, 3, &mut instance, &mut actions),
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
            2,
            Some(sound_handle),
            3,
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
            1,
            Some(other_sound_handle),
            3,
            &mut instance,
            &mut actions,
        ),
        Value::Bool(false)
    );
    assert!(actions.is_empty());
    assert_eq!(
        run(&mut runtime, 1, None, 0, &mut instance, &mut actions),
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
        run(&mut runtime, 1, None, 3, &mut instance, &mut actions),
        Value::Bool(false)
    );

    assert!(runtime.start_sound(0, 3, sound.clone(), 0.5, 1.0, false));
    runtime.tick_sound_channels(0.5);
    assert_eq!(
        run(&mut runtime, 1, None, 3, &mut instance, &mut actions),
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
fn player_console_command_calls_the_host_through_bytecode() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-console-command-{}-{}",
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
        export_index: 8,
    };
    runtime
        .class_defaults
        .insert(object_id(&package, 0), InstanceState::default());
    runtime
        .scripts
        .insert(object_id(&package, 7), named_native_script(7));
    runtime.scripts.insert(
        object_id(&package, 8),
        Arc::new(ScriptExport {
            export_index: 8,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 8,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                bytes: [
                    0x04, 0x1c, 8, 0, 0, 0, 0x1f, b'G', b'E', b'T', b'P', b'I', b'N', b'G', 0, 0x16,
                ]
                .to_vec(),
                raw_len: 16,
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
    let calls = Rc::new(RefCell::new(Vec::new()));
    runtime.set_console_command_host(RecordingConsole {
        calls: Rc::clone(&calls),
    });

    assert_eq!(
        runtime
            .execute_function(
                17,
                &class,
                &caller,
                &[],
                &mut InstanceState::default(),
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::String("host response".to_owned()),
    );
    assert_eq!(
        calls.borrow().as_slice(),
        [(17, "PlayerPawn".to_owned(), "GETPING".to_owned())],
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn player_console_command_uses_the_production_host_for_queries_and_settings() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-production-console-command-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(
        system.join("Default.ini"),
        "[Core.System]\nPaths=*.u\n[Engine.Engine]\nViewportManager=WinDrv.WindowsClient\nAudioDevice=Galaxy.GalaxyAudioSubsystem\n[WinDrv.WindowsClient]\nBrightness=0.400000\n[Galaxy.GalaxyAudioSubsystem]\nMusicVolume=128\n",
    )
    .unwrap();
    fs::write(
        system.join("DefUser.ini"),
        "[Engine.Input]\nW=MoveForward\n",
    )
    .unwrap();
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
        export_index: 8,
    };
    runtime
        .class_defaults
        .insert(object_id(&package, 0), InstanceState::default());
    runtime
        .scripts
        .insert(object_id(&package, 7), named_native_script(7));
    runtime.scripts.insert(
        object_id(&package, 8),
        Arc::new(ScriptExport {
            export_index: 8,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 8,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                bytes: [
                    0x04, 0x1c, 8, 0, 0, 0, 0x1f, b'G', b'E', b'T', b'P', b'I', b'N', b'G', 0, 0x16,
                ]
                .to_vec(),
                raw_len: 16,
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
    let mut console =
        ConsoleCommands::production(&root, (640, 480), vec![(640, 480), (1024, 768)]).unwrap();
    runtime.set_console_command_host(console.clone());

    assert_eq!(
        runtime
            .execute_function(
                17,
                &class,
                &caller,
                &[],
                &mut InstanceState::default(),
                &mut Vec::new(),
                0,
            )
            .unwrap(),
        Value::String("0".to_owned()),
    );
    let mut actions = Vec::new();
    let mut state_frames = HashMap::default();
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "Actor",
            "ConsoleCommand",
            &[Value::String("GETLOSS".to_owned())],
            &mut actions,
            Some(&mut console),
        ),
        Some(Value::String("0".to_owned()))
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "Console",
            "ConsoleCommand",
            &[Value::String("SaveGame 9".to_owned())],
            &mut actions,
            Some(&mut console),
        ),
        Some(Value::Bool(true))
    );
    assert_eq!(
        console.console_command(
            17,
            "PlayerPawn",
            "get ini:Engine.Engine.ViewportManager Brightness",
        ),
        ConsoleCommandResponse {
            output: "0.400000".to_owned(),
            handled: true,
        },
    );
    assert_eq!(
        console
            .console_command(17, "PlayerPawn", "KEYBINDING W")
            .output,
        "MoveForward",
    );
    console.console_command(17, "PlayerPawn", "SetRes 1024x768");
    assert_eq!(
        console.take_actions(),
        [
            ConsoleCommandAction::SaveGame { slot: 9 },
            ConsoleCommandAction::SetResolution {
                width: 1024,
                height: 768,
            },
        ],
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
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut console = RecordingConsole {
        calls: Rc::clone(&calls),
    };
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "Actor",
            "ConsoleCommand",
            &[Value::String("GetRes".to_owned())],
            &mut actions,
            Some(&mut console),
        ),
        Some(Value::String("host response".to_owned()))
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            18,
            "Console",
            "ConsoleCommand",
            &[Value::String("exit".to_owned())],
            &mut actions,
            Some(&mut console),
        ),
        Some(Value::Bool(true))
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            17,
            "Decal",
            "DetachDecal",
            &[],
            &mut actions,
            None,
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
            None,
        ),
        None
    );
    assert_eq!(
        execution::named_native(
            &mut state_frames,
            19,
            "PlayerPawn",
            "ConsoleCommand",
            &[Value::String("GETPING".to_owned())],
            &mut actions,
            None,
        ),
        None
    );
    assert_eq!(
        calls.borrow().as_slice(),
        [
            (17, "Actor".to_owned(), "GetRes".to_owned()),
            (18, "Console".to_owned(), "exit".to_owned()),
        ],
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
fn null_context_string_defaults_before_caps() {
    let bytecode = Bytecode {
        version: 76,
        raw_len: 28,
        bytes: vec![
            0x0f, 0x00, 1, 0, 0, 0, 0xeb, 0x19, 0x2a, 9, 0, 4, 0x1f, b'i', b'g', b'n', b'o', b'r',
            b'e', b'd', 0, 0x16, 0x04, 0x00, 1, 0, 0, 0,
        ],
        tokens: Vec::new(),
    };

    assert_eq!(
        Frame::new(&bytecode).execute(|call, arguments| {
            let FunctionCall::Native(index) = call else {
                unreachable!()
            };
            scalar_native(index, arguments)
        }),
        Ok(Value::String(String::new()))
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
fn navigation_uses_the_shortest_unpruned_step() {
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
    assert_eq!(next_navigation_step(&specs, &start, &target), Some(short));
}

#[test]
fn hp1_find_path_ignores_pawn_collision_size() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-hp1-find-path-{}-{}",
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
    let collision_radius = runtime_actor_id(100);
    let collision_height = runtime_actor_id(101);
    runtime.fields.insert(
        (class_id.clone(), "collisionradius".to_owned()),
        Some(collision_radius.clone()),
    );
    runtime.fields.insert(
        (class_id, "collisionheight".to_owned()),
        Some(collision_height.clone()),
    );
    let mut instance = [
        (
            collision_radius,
            StoredValue::Value(Value::Float(73.009_04)),
        ),
        (
            collision_height,
            StoredValue::Value(Value::Float(74.292_564)),
        ),
    ]
    .into_iter()
    .collect::<InstanceState>();
    let start = object_id(&package, 0);
    let target = object_id(&package, 1);
    runtime.reach_specs.push(NavigationReachSpec {
        index: 0,
        distance: 1,
        start: start.clone(),
        end: target.clone(),
        collision_radius: 60,
        collision_height: 60,
        pruned: false,
    });
    let start_handle = runtime.object_handle(start).unwrap();
    let target_handle = runtime.object_handle(target).unwrap();

    let mut bytes = vec![0x04, 0x62, 0x29, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.push(0x00);
    bytes.extend(8_i32.to_le_bytes());
    bytes.push(0x16);
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    frame.set_local(7, Value::Object(start_handle));
    frame.set_local(8, Value::NameText("ClientTravel".to_owned()));
    let result = frame.execute(|call, arguments| {
        assert_eq!(call, FunctionCall::Native(0x229));
        runtime.native(
            1,
            &class,
            &package,
            0x229,
            arguments,
            &mut instance,
            &mut Vec::new(),
            0,
        )
    });

    assert_eq!(result.unwrap(), Value::Object(target_handle));
    fs::remove_dir_all(root).unwrap();
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
        "bKillZone",
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
            (fields["bKillZone"].clone(), Value::Bool(false)),
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
fn latent_movement_matches_retail_acceleration_direction_and_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-latent-acceleration-{}-{}",
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
    runtime.scripts.insert(
        class_id.clone(),
        Arc::new(ScriptExport {
            export_index: class.export_index,
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
    runtime
        .class_defaults
        .insert(class_id.clone(), InstanceState::default());

    let acceleration = object_id(&package, 1);
    let movement_state = object_id(&package, 2);
    let goto_replacement = object_id(&package, 4);
    let begin_state = object_id(&package, 6);
    let move_target = object_id(&package, 7);
    let observed_acceleration = object_id(&package, 8);
    let fields = [
        ("Acceleration", acceleration.clone()),
        ("MoveTarget", move_target.clone()),
        ("MoveTimer", runtime_actor_id(200)),
        ("Physics", runtime_actor_id(201)),
        ("Location", runtime_actor_id(202)),
        ("Destination", runtime_actor_id(203)),
        ("Velocity", runtime_actor_id(204)),
        ("DesiredRotation", runtime_actor_id(205)),
        ("AccelRate", runtime_actor_id(206)),
        ("TimeSeconds", runtime_actor_id(207)),
        ("TimeDilation", runtime_actor_id(208)),
        ("Rotation", runtime_actor_id(209)),
        ("bCanStrafe", runtime_actor_id(210)),
    ]
    .into_iter()
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

    let mut observe_receiver = vec![0x0f, 0x01];
    observe_receiver.extend(9_i32.to_le_bytes());
    observe_receiver.extend([0x19, 0x01]);
    observe_receiver.extend(8_i32.to_le_bytes());
    observe_receiver.extend(5_u16.to_le_bytes());
    observe_receiver.push(12);
    observe_receiver.push(0x01);
    observe_receiver.extend(2_i32.to_le_bytes());
    observe_receiver.extend([0x04, 0x0b]);
    runtime.scripts.insert(
        movement_state.clone(),
        Arc::new(ScriptExport {
            export_index: movement_state.export_index,
            class_name: "State".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: movement_state.export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: observe_receiver.len(),
                bytes: observe_receiver.clone(),
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::State(openhp1_script::StateMetadata {
                probe_mask: 0,
                ignore_mask: 0,
                label_table_offset: 0,
                flags: 0,
            }),
        }),
    );
    let function_script = |export_index, bytes: Vec<u8>| {
        Arc::new(ScriptExport {
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
    };
    let mut goto_bytes = vec![0x04, GOTO_STATE as u8, 0x21];
    goto_bytes.extend(2_i32.to_le_bytes());
    goto_bytes.push(0x16);
    runtime.scripts.insert(
        goto_replacement.clone(),
        function_script(goto_replacement.export_index, goto_bytes),
    );
    runtime.scripts.insert(
        begin_state.clone(),
        function_script(begin_state.export_index, observe_receiver),
    );
    runtime.state_lookups.insert(
        StateLookup::new(class_id.clone(), "GetPlayerNetworkAddress"),
        Some(movement_state.clone()),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(
            class_id.clone(),
            Some("GetPlayerNetworkAddress"),
            "BeginState",
            0,
        ),
        Some(begin_state),
    );

    let level = 0;
    let caller = 7;
    let receiver = 8;
    let level_object = runtime_actor_id(700);
    let caller_object = runtime_actor_id(701);
    let receiver_object = runtime_actor_id(702);
    for (actor, object) in [
        (level, level_object),
        (caller, caller_object),
        (receiver, receiver_object.clone()),
    ] {
        runtime.actor_classes.insert(actor, class_id.clone());
        runtime.actor_objects.insert(actor, object.clone());
        runtime.object_actors.insert(object, actor);
    }
    runtime.level_info = Some(level);
    let instance = |acceleration_value| {
        [
            (
                acceleration.clone(),
                StoredValue::Value(Value::Vector(acceleration_value)),
            ),
            (move_target.clone(), StoredValue::Object(None)),
            (
                observed_acceleration.clone(),
                StoredValue::Value(Value::Vector([-1.0; 3])),
            ),
            (
                fields["MoveTimer"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["Physics"].clone(),
                StoredValue::Value(Value::Byte(physics::PHYS_WALKING)),
            ),
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["Destination"].clone(),
                StoredValue::Value(Value::Vector([100.0, 0.0, 0.0])),
            ),
            (
                fields["Velocity"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["DesiredRotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["AccelRate"].clone(),
                StoredValue::Value(Value::Float(100.0)),
            ),
            (
                fields["TimeSeconds"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["TimeDilation"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0; 3])),
            ),
            (
                fields["bCanStrafe"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    runtime.instances.insert(level, instance([0.0; 3]));
    let mut caller_instance = instance([11.0, 12.0, 13.0]);
    caller_instance.insert(
        move_target.clone(),
        StoredValue::Object(Some(receiver_object)),
    );
    runtime.instances.insert(caller, caller_instance);
    runtime
        .instances
        .insert(receiver, instance([50.0, 25.0, 0.0]));

    runtime
        .actor_states
        .insert(caller, Some("OldState".to_owned()));
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state.clone(),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::MoveToward(receiver),
        },
    );
    let goto_function = runtime.resolved_object(&goto_replacement).unwrap();
    runtime
        .execute_actor_function(caller, &class, &goto_function, &[])
        .unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0; 3])))
    );
    assert_eq!(
        runtime.instances[&caller].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([11.0, 12.0, 13.0])))
    );
    assert_eq!(
        runtime.instances[&caller].get(&observed_acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0; 3]))),
        "replacement BeginState must observe the cleared receiver"
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        acceleration.clone(),
        StoredValue::Value(Value::Vector([30.0, 40.0, 0.0])),
    );
    runtime.instances.get_mut(&caller).unwrap().insert(
        observed_acceleration.clone(),
        StoredValue::Value(Value::Vector([-1.0; 3])),
    );
    runtime
        .actor_states
        .insert(caller, Some("OldState".to_owned()));
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state.clone(),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::FinishAnimation(receiver),
        },
    );
    runtime
        .execute_actor_function(caller, &class, &goto_function, &[])
        .unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([30.0, 40.0, 0.0])))
    );
    assert_eq!(
        runtime.instances[&caller].get(&observed_acceleration),
        Some(&StoredValue::Value(Value::Vector([30.0, 40.0, 0.0])))
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["MoveTimer"].clone(),
        StoredValue::Value(Value::Float(10.0)),
    );
    runtime.instances.get_mut(&caller).unwrap().insert(
        observed_acceleration.clone(),
        StoredValue::Value(Value::Vector([-1.0; 3])),
    );
    runtime
        .actor_states
        .insert(caller, Some("GetPlayerNetworkAddress".to_owned()));
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state.clone(),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::MoveTo(receiver),
        },
    );
    runtime.tick(0.0).unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([100.0, 0.0, 0.0]))),
        "active latent movement must retain its acceleration"
    );
    assert_eq!(
        runtime.instances[&caller].get(&observed_acceleration),
        Some(&StoredValue::Value(Value::Vector([-1.0; 3])))
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["Physics"].clone(),
        StoredValue::Value(Value::Byte(physics::PHYS_FLYING)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["Rotation"].clone(),
        StoredValue::Value(Value::Rotator([0, 16_384, 0])),
    );
    runtime.tick(0.0).unwrap();
    let Some(StoredValue::Value(Value::Vector(acceleration_value))) =
        runtime.instances[&receiver].get(&acceleration)
    else {
        panic!("missing flying acceleration");
    };
    assert!(acceleration_value[0].abs() < 1.0e-4);
    assert!((acceleration_value[1] - 100.0).abs() < 1.0e-4);
    assert!(acceleration_value[2].abs() < 1.0e-4);
    assert_eq!(
        runtime.instances[&receiver].get(&fields["DesiredRotation"]),
        Some(&StoredValue::Value(Value::Rotator([0; 3]))),
        "a non-strafing flyer must accelerate along its facing while turning toward the destination"
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["bCanStrafe"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime.tick(0.0).unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([100.0, 0.0, 0.0]))),
        "a strafing flyer must accelerate directly toward the destination"
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["MoveTimer"].clone(),
        StoredValue::Value(Value::Float(-1.0)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["bCanStrafe"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        acceleration.clone(),
        StoredValue::Value(Value::Vector([0.0, 100.0, 0.0])),
    );
    runtime.tick(0.0).unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0, 100.0, 0.0]))),
        "completed non-strafing flying movement must retain its acceleration"
    );
    assert_eq!(
        runtime.instances[&caller].get(&observed_acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0, 100.0, 0.0]))),
        "resumed caller state must observe the retained flying acceleration"
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["bCanStrafe"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["MoveTimer"].clone(),
        StoredValue::Value(Value::Float(-1.0)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        acceleration.clone(),
        StoredValue::Value(Value::Vector([9.0, 8.0, 7.0])),
    );
    runtime.instances.get_mut(&caller).unwrap().insert(
        observed_acceleration.clone(),
        StoredValue::Value(Value::Vector([-1.0; 3])),
    );
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state.clone(),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::MoveTo(receiver),
        },
    );
    runtime.tick(0.0).unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0; 3]))),
        "completed strafing flying movement must clear its acceleration"
    );

    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["Physics"].clone(),
        StoredValue::Value(Value::Byte(physics::PHYS_WALKING)),
    );
    runtime.instances.get_mut(&receiver).unwrap().insert(
        fields["MoveTimer"].clone(),
        StoredValue::Value(Value::Float(-1.0)),
    );
    runtime.instances.get_mut(&caller).unwrap().insert(
        observed_acceleration.clone(),
        StoredValue::Value(Value::Vector([-1.0; 3])),
    );
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state.clone(),
            frame: FrameSnapshot::at(0),
            latent: LatentAction::MoveTo(receiver),
        },
    );
    runtime.tick(0.0).unwrap();
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0; 3])))
    );
    assert_eq!(
        runtime.instances[&caller].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([11.0, 12.0, 13.0])))
    );
    assert_eq!(
        runtime.instances[&caller].get(&observed_acceleration),
        Some(&StoredValue::Value(Value::Vector([0.0; 3]))),
        "resumed caller state must observe the cleared receiver"
    );

    runtime
        .fields
        .remove(&(class_id.clone(), "acceleration".to_owned()));
    runtime.instances.get_mut(&receiver).unwrap().insert(
        acceleration.clone(),
        StoredValue::Value(Value::Vector([9.0, 8.0, 7.0])),
    );
    runtime
        .actor_states
        .insert(caller, Some("OldState".to_owned()));
    runtime.state_frames.insert(
        caller,
        StateFrame {
            state: movement_state,
            frame: FrameSnapshot::at(0),
            latent: LatentAction::MoveTo(receiver),
        },
    );
    let _error = runtime
        .execute_actor_function(caller, &class, &goto_function, &[])
        .unwrap_err();
    assert_eq!(runtime.actor_states[&caller].as_deref(), Some("OldState"));
    assert_eq!(
        runtime.instances[&caller].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([11.0, 12.0, 13.0])))
    );
    assert_eq!(
        runtime.instances[&receiver].get(&acceleration),
        Some(&StoredValue::Value(Value::Vector([9.0, 8.0, 7.0]))),
        "receiver instance must be restored when clearing acceleration fails"
    );
    fs::remove_dir_all(root).unwrap();
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
fn relevant_projectile_dispatches_mover_bump_state_and_motion() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-mover-projectile-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_mover_runtime_package()).unwrap();
    let projectile_package_path = system.join("Projectile.u");
    fs::write(
        &projectile_package_path,
        synthetic_mover_projectile_package(),
    )
    .unwrap();
    let spell_package_path = system.join("spellFlip.u");
    fs::write(
        &spell_package_path,
        synthetic_runtime_package_for("spellFlip"),
    )
    .unwrap();
    let other_projectile_package_path = system.join("OtherProjectile.u");
    fs::write(
        &other_projectile_package_path,
        synthetic_runtime_package_for("OtherProjectile"),
    )
    .unwrap();
    let actor_package_path = system.join("Actor.u");
    fs::write(&actor_package_path, synthetic_runtime_package_for("Actor")).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let projectile_package = runtime
        .packages
        .load_path(&projectile_package_path)
        .unwrap();
    let spell_package = runtime.packages.load_path(&spell_package_path).unwrap();
    let other_projectile_package = runtime
        .packages
        .load_path(&other_projectile_package_path)
        .unwrap();
    let actor_package = runtime.packages.load_path(&actor_package_path).unwrap();
    let mover_class_id = object_id(&package, 0);
    let projectile_class_id = object_id(&projectile_package, 0);
    let spell_class_id = object_id(&spell_package, 0);
    let other_projectile_class_id = object_id(&other_projectile_package, 0);
    let actor_class_id = object_id(&actor_package, 0);
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
    runtime
        .scripts
        .insert(mover_class_id.clone(), class_script(0));
    runtime
        .scripts
        .insert(projectile_class_id.clone(), class_script(0));
    runtime
        .scripts
        .insert(spell_class_id.clone(), class_script(0));
    runtime
        .scripts
        .insert(other_projectile_class_id.clone(), class_script(0));
    runtime
        .scripts
        .insert(actor_class_id.clone(), class_script(0));
    runtime
        .class_defaults
        .insert(mover_class_id.clone(), InstanceState::default());
    runtime
        .class_defaults
        .insert(projectile_class_id.clone(), InstanceState::default());
    runtime
        .class_defaults
        .insert(spell_class_id.clone(), InstanceState::default());
    runtime
        .class_defaults
        .insert(other_projectile_class_id.clone(), InstanceState::default());
    runtime
        .class_defaults
        .insert(actor_class_id.clone(), InstanceState::default());
    runtime
        .class_relations
        .insert((spell_class_id.clone(), projectile_class_id.clone()), true);
    runtime.class_relations.insert(
        (
            other_projectile_class_id.clone(),
            projectile_class_id.clone(),
        ),
        true,
    );

    let state_id = object_id(&package, 1);
    let mut state_bytes = vec![0x1b];
    state_bytes.extend(13_i32.to_le_bytes());
    state_bytes.extend([0x16, 0x61, 0x2d, 0x16, 0x1b]);
    state_bytes.extend(14_i32.to_le_bytes());
    state_bytes.extend([0x16, 0x08]);
    let label_table_offset = state_bytes.len();
    state_bytes.push(0x0c);
    state_bytes.extend(11_i32.to_le_bytes());
    state_bytes.extend(0_u32.to_le_bytes());
    state_bytes.extend(12_i32.to_le_bytes());
    state_bytes.extend(0_u32.to_le_bytes());
    runtime.scripts.insert(
        state_id.clone(),
        Arc::new(ScriptExport {
            export_index: 1,
            class_name: "State".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: 1,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 76,
                raw_len: state_bytes.len(),
                bytes: state_bytes,
                tokens: vec![Token {
                    offset: label_table_offset,
                    depth: 0,
                    opcode: 0x0c,
                    call: None,
                }],
            },
            metadata: ScriptMetadata::State(openhp1_script::StateMetadata {
                probe_mask: 0,
                ignore_mask: 0,
                label_table_offset: 0,
                flags: 0,
            }),
        }),
    );
    runtime.state_lookups.insert(
        StateLookup::new(mover_class_id.clone(), "BumpMove"),
        Some(state_id),
    );

    let function_script = |export_index, bytes: Vec<u8>| {
        Arc::new(ScriptExport {
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
    };
    let state_callback = |message: &[u8], final_location: Option<[f32; 3]>| {
        let mut bytes = Vec::new();
        if let Some(location) = final_location {
            bytes.extend([0x61, 0x0b, 0x23]);
            bytes.extend(location.into_iter().flat_map(f32::to_le_bytes));
            bytes.push(0x16);
        }
        bytes.extend([0x04, 0xe7, 0x1f]);
        bytes.extend(message);
        bytes.extend([0, 0x16]);
        bytes
    };
    for (name, export_index, message, final_location) in [
        ("DoOpen", 5, b"DoOpen".as_slice(), None),
        (
            "FinishedOpening",
            6,
            b"FinishedOpening".as_slice(),
            Some([96.0, 0.0, 0.0]),
        ),
    ] {
        let function = object_id(&projectile_package, export_index);
        runtime.scripts.insert(
            function.clone(),
            function_script(export_index, state_callback(message, final_location)),
        );
        runtime.function_lookups.insert(
            FunctionLookup::new(mover_class_id.clone(), Some("BumpMove"), name, 1),
            Some(function),
        );
    }
    let other_parameter = 2_i32;
    let b_proj_target = object_id(&projectile_package, 2);
    let relevance_checked = object_id(&projectile_package, 3);
    let mut relevant = vec![0x14, 0x2d, 0x01];
    relevant.extend(4_i32.to_le_bytes());
    relevant.push(0x27);
    relevant.extend([0xe7, 0x1f]);
    relevant.extend(b"mover relevance\0");
    relevant.push(0x16);
    let projectile_branch = relevant.len();
    relevant.extend([0x07, 0, 0, 0x2d, 0x01]);
    relevant.extend(3_i32.to_le_bytes());
    let cast_branch = relevant.len();
    relevant.extend([0x07, 0, 0, 0x77, 0x2e]);
    relevant.extend(1_i32.to_le_bytes());
    relevant.push(0x00);
    relevant.extend(other_parameter.to_le_bytes());
    relevant.extend([0x2a, 0x16, 0x04, 0x19, 0x2e]);
    relevant.extend(1_i32.to_le_bytes());
    relevant.push(0x00);
    relevant.extend(other_parameter.to_le_bytes());
    relevant.extend([0x06, 0x00, 0x04, 0x1b]);
    relevant.extend(9_i32.to_le_bytes());
    relevant.extend([0x16, 0x04, 0x28]);
    let any_bump = relevant.len();
    relevant.extend([0x04, 0x27]);
    relevant[projectile_branch + 1..projectile_branch + 3]
        .copy_from_slice(&(any_bump as u16).to_le_bytes());
    let rejected_projectile = any_bump - 2;
    relevant[cast_branch + 1..cast_branch + 3]
        .copy_from_slice(&(rejected_projectile as u16).to_le_bytes());

    let relevant_id = object_id(&projectile_package, 4);
    runtime.scripts.insert(
        relevant_id.clone(),
        function_script(relevant_id.export_index, relevant),
    );
    runtime.frame_arguments.insert(
        relevant_id.clone(),
        Arc::new(vec![(other_parameter, 0, false)]),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(mover_class_id.clone(), None, "IsRelevant", 0),
        Some(relevant_id),
    );

    let projectile_relevance = |marker: i32, accepted: bool, message: &[u8]| {
        let mut bytes = vec![0x14, 0x2d, 0x01];
        bytes.extend(marker.to_le_bytes());
        bytes.push(0x27);
        bytes.extend([0xe7, 0x1f]);
        bytes.extend(message);
        bytes.extend([0, 0x16, 0x04, if accepted { 0x27 } else { 0x28 }]);
        bytes
    };
    let spell_relevance = object_id(&spell_package, 1);
    let spell_checked = object_id(&spell_package, 2);
    runtime.scripts.insert(
        spell_relevance.clone(),
        function_script(
            spell_relevance.export_index,
            projectile_relevance(3, true, b"spellFlip relevance"),
        ),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(spell_class_id.clone(), None, "IsRelevantToMover", 1),
        Some(spell_relevance),
    );
    let other_relevance = object_id(&other_projectile_package, 1);
    let other_checked = object_id(&other_projectile_package, 2);
    runtime.scripts.insert(
        other_relevance.clone(),
        function_script(
            other_relevance.export_index,
            projectile_relevance(3, false, b"other projectile relevance"),
        ),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(
            other_projectile_class_id.clone(),
            None,
            "IsRelevantToMover",
            1,
        ),
        Some(other_relevance),
    );

    let key_pos = object_id(&package, 4);
    let location = object_id(&package, 5);
    let base_pos = object_id(&package, 6);
    let move_increment = object_id(&package, 7);
    let instance_variable = |reference: i32| {
        let mut bytes = vec![0x01];
        bytes.extend(reference.to_le_bytes());
        bytes
    };
    let key_position_x = || {
        let mut bytes = vec![0x36];
        bytes.extend(9_i32.to_le_bytes());
        bytes.extend([0x1a, 0x26]);
        bytes.extend(instance_variable(5));
        bytes
    };
    let other_location_x = || {
        let mut bytes = vec![0x36];
        bytes.extend(9_i32.to_le_bytes());
        bytes.extend([0x19, 0x00]);
        bytes.extend(other_parameter.to_le_bytes());
        bytes.extend(5_u16.to_le_bytes());
        bytes.push(12);
        bytes.extend(instance_variable(6));
        bytes
    };
    let mut bump = vec![0x0f, 0x1a, 0x26];
    bump.extend(instance_variable(5));
    bump.push(0xd8);
    bump.extend(instance_variable(6));
    bump.extend(instance_variable(7));
    bump.push(0x16);
    let direction_branch = bump.len();
    bump.extend([0x07, 0, 0, 0xb1, 0xaf]);
    bump.extend(other_location_x());
    bump.extend([0x36]);
    bump.extend(9_i32.to_le_bytes());
    bump.extend(instance_variable(6));
    bump.extend([0x16, 0x1e]);
    bump.extend(0.0_f32.to_le_bytes());
    bump.push(0x16);
    bump.push(0xb9);
    bump.extend(key_position_x());
    bump.extend(instance_variable(8));
    bump.push(0x16);
    let direction_end = bump.len();
    bump.extend([0x06, 0, 0]);
    let negative_direction = bump.len();
    bump.push(0xb8);
    bump.extend(key_position_x());
    bump.extend(instance_variable(8));
    bump.push(0x16);
    let goto_state = bump.len();
    bump[direction_branch + 1..direction_branch + 3]
        .copy_from_slice(&(negative_direction as u16).to_le_bytes());
    bump[direction_end + 1..direction_end + 3].copy_from_slice(&(goto_state as u16).to_le_bytes());
    bump.extend([0x71, 0x21]);
    bump.extend(1_i32.to_le_bytes());
    bump.push(0x21);
    bump.extend(11_i32.to_le_bytes());
    bump.extend([0x16, 0x04, 0x0b]);
    let bump_id = object_id(&package, 3);
    runtime
        .scripts
        .insert(bump_id.clone(), function_script(3, bump));
    runtime
        .frame_arguments
        .insert(bump_id.clone(), Arc::new(vec![(other_parameter, 0, false)]));
    runtime
        .struct_members
        .insert(bump_id.clone(), Arc::new(vec![(9, StructMember::X)]));
    runtime.function_lookups.insert(
        FunctionLookup::new(mover_class_id.clone(), None, "Bump", 0),
        Some(bump_id),
    );

    let fields = [
        "Location",
        "OldLocation",
        "Velocity",
        "Rotation",
        "CollisionHeight",
        "CollisionRadius",
        "CollisionWidth",
        "CollideType",
        "bCollideActors",
        "bBlockActors",
        "bBlockPlayers",
        "bCollideWorld",
        "bCollideWhenPlacing",
        "bStatic",
        "bMovable",
        "bInterpolating",
        "Brush",
        "PrePivot",
        "TimeSeconds",
        "TimeDilation",
        "Physics",
        "LifeSpan",
        "OldPos",
        "OldRot",
        "BaseRot",
        "PhysAlpha",
        "PhysRate",
        "KeyNum",
        "MoverGlideType",
        "KeyRot",
        "ZoneGravity",
        "ZoneVelocity",
        "ZoneGroundFriction",
        "ZoneFluidFriction",
        "ZoneTerminalVelocity",
        "bWaterZone",
        "Base",
        "Level",
        "MaxMountHeight",
        "MaxStepHeight",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        (
            name,
            if name == "Location" {
                location.clone()
            } else {
                runtime_actor_id(100 + index)
            },
        )
    })
    .collect::<HashMap<_, _>>();
    for class in [
        &mover_class_id,
        &projectile_class_id,
        &spell_class_id,
        &other_projectile_class_id,
        &actor_class_id,
    ] {
        for (name, field) in &fields {
            runtime.fields.insert(
                (class.clone(), name.to_ascii_lowercase()),
                Some(field.clone()),
            );
        }
        runtime
            .fields
            .insert((class.clone(), "mainscale".to_owned()), None);
        runtime
            .fields
            .insert((class.clone(), "standingcount".to_owned()), None);
        runtime
            .fields
            .insert((class.clone(), "bpainzone".to_owned()), None);
        runtime
            .fields
            .insert((class.clone(), "bkillzone".to_owned()), None);
        runtime
            .fields
            .insert((class.clone(), "damagetype".to_owned()), None);
    }
    runtime.fields.insert(
        (mover_class_id.clone(), "keypos".to_owned()),
        Some(key_pos.clone()),
    );
    runtime.fields.insert(
        (mover_class_id.clone(), "basepos".to_owned()),
        Some(base_pos.clone()),
    );
    let instance = |location| {
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
                StoredValue::Value(Value::Float(10.0)),
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
                fields["CollideType"].clone(),
                StoredValue::Value(Value::Byte(0)),
            ),
            (
                fields["bCollideActors"].clone(),
                StoredValue::Value(Value::Bool(true)),
            ),
            (
                fields["bBlockActors"].clone(),
                StoredValue::Value(Value::Bool(false)),
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
                fields["bInterpolating"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Brush"].clone(), StoredValue::Object(None)),
            (
                fields["PrePivot"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
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
            (
                fields["LifeSpan"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["ZoneGravity"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["ZoneVelocity"].clone(),
                StoredValue::Value(Value::Vector([0.0; 3])),
            ),
            (
                fields["ZoneGroundFriction"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["ZoneFluidFriction"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["ZoneTerminalVelocity"].clone(),
                StoredValue::Value(Value::Float(1_000.0)),
            ),
            (
                fields["bWaterZone"].clone(),
                StoredValue::Value(Value::Bool(false)),
            ),
            (fields["Base"].clone(), StoredValue::Object(None)),
            (fields["Level"].clone(), StoredValue::Object(None)),
            (
                fields["MaxMountHeight"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
            (
                fields["MaxStepHeight"].clone(),
                StoredValue::Value(Value::Float(0.0)),
            ),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    let mover_object = runtime_actor_id(1);
    let projectile_object = runtime_actor_id(2);
    for (actor, object, class) in [
        (0, mover_object, mover_class_id.clone()),
        (1, projectile_object.clone(), spell_class_id.clone()),
    ] {
        runtime.object_actors.insert(object.clone(), actor);
        runtime.actor_objects.insert(actor, object);
        runtime.actor_classes.insert(actor, class);
    }
    runtime.next_actor = 2;
    let mut mover_instance = instance([0.0; 3]);
    mover_instance.insert(b_proj_target.clone(), StoredValue::Value(Value::Bool(true)));
    mover_instance.insert(
        relevance_checked.clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    mover_instance.insert(
        key_pos.clone(),
        StoredValue::Array(vec![StoredValue::Value(Value::Vector([0.0; 3])); 8]),
    );
    mover_instance.insert(
        location.clone(),
        StoredValue::Value(Value::Vector([64.0, 0.0, 0.0])),
    );
    mover_instance.insert(
        base_pos,
        StoredValue::Value(Value::Vector([16.0, 0.0, 0.0])),
    );
    mover_instance.insert(move_increment, StoredValue::Value(Value::Float(32.0)));
    mover_instance.insert(
        fields["bInterpolating"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime.instances.insert(0, mover_instance);
    let mut projectile_instance = instance([-40.0, 0.0, 0.0]);
    projectile_instance.insert(
        spell_checked.clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    assert!(
        runtime
            .class_is_a(
                ResolvedObject {
                    package: Arc::clone(&spell_package),
                    export_index: 0,
                },
                &ResolvedObject {
                    package: Arc::clone(&projectile_package),
                    export_index: 0,
                },
            )
            .unwrap()
    );
    let spell_class = ResolvedObject {
        package: Arc::clone(&spell_package),
        export_index: 0,
    };
    let mover_class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let mut moving_mover = runtime.instances.remove(&0).unwrap();
    let mut stored_projectile = projectile_instance.clone();
    stored_projectile.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([100.0, 0.0, 0.0])),
    );
    runtime.instances.insert(1, stored_projectile);
    let mover_query = runtime
        .test_move_actor(0, &mover_class, [80.0, 0.0, 0.0], &moving_mover)
        .unwrap();
    assert_eq!(mover_query.actor, None);
    assert_eq!(
        moving_mover.get(&relevance_checked),
        Some(&StoredValue::Value(Value::Bool(false)))
    );
    assert_eq!(
        runtime.instances[&1].get(&spell_checked),
        Some(&StoredValue::Value(Value::Bool(false)))
    );
    let mut mover_actions = Vec::new();
    let mover_hit = runtime
        .try_move_actor(
            0,
            &mover_class,
            [80.0, 0.0, 0.0],
            &mut moving_mover,
            &mut mover_actions,
        )
        .unwrap();
    assert_eq!(mover_hit.actor, None);
    assert_eq!(mover_hit.fraction, 1.0);
    assert_eq!(
        moving_mover.get(&relevance_checked),
        Some(&StoredValue::Value(Value::Bool(true))),
        "real mover movement must retain IsRelevant mutations on its active instance"
    );
    assert!(mover_actions.iter().any(|action| matches!(
        action,
        ActorAction::Log {
            actor: 0,
            message,
            tag: None,
        } if message == "mover relevance"
    )));
    moving_mover.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    moving_mover.insert(
        relevance_checked.clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    runtime.instances.remove(&1);
    runtime.instances.insert(0, moving_mover);
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();

    let query_hit = runtime
        .test_move_actor(1, &spell_class, [80.0, 0.0, 0.0], &projectile_instance)
        .unwrap();
    assert_eq!(query_hit.actor, None);
    assert_eq!(
        runtime.instances[&0].get(&relevance_checked),
        Some(&StoredValue::Value(Value::Bool(false))),
        "collision probes must discard Mover.IsRelevant instance mutations"
    );
    assert_eq!(
        projectile_instance.get(&spell_checked),
        Some(&StoredValue::Value(Value::Bool(false))),
        "collision probes must discard virtual projectile mutations"
    );
    assert!(!runtime.instances.contains_key(&1));

    runtime.level_info = Some(1);
    let mut actions = Vec::new();
    let hit = runtime
        .try_move_actor(
            1,
            &spell_class,
            [80.0, 0.0, 0.0],
            &mut projectile_instance,
            &mut actions,
        )
        .unwrap();
    assert_eq!(hit.actor, None);
    assert_eq!(hit.fraction, 1.0);
    assert_eq!(
        projectile_instance.get(&fields["Location"]),
        Some(&StoredValue::Value(Value::Vector([40.0, 0.0, 0.0]))),
        "a relevant non-blocking projectile must cross the mover after sending Bump"
    );
    assert_eq!(
        runtime.instances[&0].get(&relevance_checked),
        Some(&StoredValue::Value(Value::Bool(true)))
    );
    assert_eq!(
        projectile_instance.get(&spell_checked),
        Some(&StoredValue::Value(Value::Bool(true)))
    );
    let relevance_log = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ActorAction::Log {
                    actor: 0,
                    message,
                    tag: None,
                } if message == "mover relevance"
            )
        })
        .expect("Mover.IsRelevant action was discarded");
    let do_open_position = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ActorAction::Log {
                    actor: 0,
                    message,
                    tag: None,
                } if message == "DoOpen"
            )
        })
        .expect("relevant projectile did not run Mover.Bump at contact");
    assert!(relevance_log < do_open_position);
    assert!(!actions.iter().any(|action| matches!(
        action,
        ActorAction::DispatchEvent {
            actor: 0,
            event: "Bump",
            ..
        }
    )));
    runtime
        .actor_bases
        .insert(1, Some(runtime.actor_objects[&0].clone()));
    projectile_instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([-40.0, 0.0, 0.0])),
    );
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    let based_hit = runtime
        .try_move_actor(
            1,
            &spell_class,
            [80.0, 0.0, 0.0],
            &mut projectile_instance,
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        based_hit.actor, None,
        "mover relevance must not turn a non-blocking contact into a collision"
    );
    runtime.actor_bases.remove(&1);
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    runtime.instances.insert(1, projectile_instance);

    assert_eq!(
        runtime.actor_states.get(&0),
        Some(&Some("BumpMove".to_owned()))
    );
    assert_eq!(
        runtime.instances[&0].get(&key_pos),
        Some(&StoredValue::Array(vec![
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([16.0, 0.0, 0.0])),
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([0.0; 3])),
            StoredValue::Value(Value::Vector([0.0; 3])),
        ]))
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        ActorAction::Log {
            actor: 0,
            message,
            tag: None,
        } if message == "DoOpen"
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        ActorAction::Log { message, .. } if message == "FinishedOpening"
    )));
    assert_eq!(
        runtime.state_frames[&0].latent,
        LatentAction::FinishInterpolation(0)
    );
    assert_eq!(
        runtime.instances[&0].get(&fields["bInterpolating"]),
        Some(&StoredValue::Value(Value::Bool(true)))
    );
    runtime.tick_functions.clear();
    assert!(runtime.tick(0.0).unwrap().is_empty());
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["bInterpolating"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    let completion_actions = runtime.tick(0.0).unwrap();
    let final_location_action = completion_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ActorAction::SetLocation {
                    actor: 0,
                    location: [96.0, 0.0, 0.0],
                }
            )
        })
        .expect("completed interpolation did not place the pillar");
    let finished_opening = completion_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ActorAction::Log {
                    actor: 0,
                    message,
                    tag: None,
                } if message == "FinishedOpening"
            )
        })
        .expect("FinishedOpening did not run after interpolation");
    assert!(final_location_action < finished_opening);
    assert_eq!(
        runtime.instances[&0].get(&fields["Location"]),
        Some(&StoredValue::Value(Value::Vector([96.0, 0.0, 0.0])))
    );
    runtime.instances.remove(&1);

    runtime.instances.get_mut(&0).unwrap().insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    runtime.instances.get_mut(&0).unwrap().insert(
        relevance_checked.clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    runtime.object_actors.remove(&projectile_object);
    let other_projectile_object = runtime_actor_id(3);
    runtime
        .object_actors
        .insert(other_projectile_object.clone(), 1);
    runtime
        .actor_objects
        .insert(1, other_projectile_object.clone());
    runtime
        .actor_classes
        .insert(1, other_projectile_class_id.clone());
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    let mut rejected_instance = instance([-40.0, 0.0, 0.0]);
    rejected_instance.insert(other_checked, StoredValue::Value(Value::Bool(false)));
    let mut rejected_actions = Vec::new();
    let rejected_hit = runtime
        .try_move_actor(
            1,
            &ResolvedObject {
                package: Arc::clone(&other_projectile_package),
                export_index: 0,
            },
            [80.0, 0.0, 0.0],
            &mut rejected_instance,
            &mut rejected_actions,
        )
        .unwrap();
    assert_eq!(rejected_hit.actor, None);
    assert_eq!(rejected_hit.fraction, 1.0);
    assert_eq!(
        rejected_instance.get(&object_id(&other_projectile_package, 2)),
        Some(&StoredValue::Value(Value::Bool(true)))
    );
    assert!(!rejected_actions.iter().any(|action| matches!(
        action,
        ActorAction::DispatchEvent {
            actor: 0,
            event: "Bump",
            ..
        }
    )));

    for property in ["bBlockActors", "bBlockPlayers"] {
        runtime.instances.get_mut(&0).unwrap().insert(
            fields[property].clone(),
            StoredValue::Value(Value::Bool(true)),
        );
        rejected_instance.insert(
            fields[property].clone(),
            StoredValue::Value(Value::Bool(true)),
        );
    }
    rejected_instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([-40.0, 0.0, 0.0])),
    );
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    let physically_blocked = runtime
        .try_move_actor(
            1,
            &ResolvedObject {
                package: Arc::clone(&other_projectile_package),
                export_index: 0,
            },
            [80.0, 0.0, 0.0],
            &mut rejected_instance,
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        physically_blocked.actor,
        Some(0),
        "mover relevance must not override authored physical blocking flags"
    );

    let mover_brush = runtime_actor_id(999);
    runtime
        .brush_collisions
        .insert(mover_brush.clone(), solid_box_collision());
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["Brush"].clone(),
        StoredValue::Object(Some(mover_brush.clone())),
    );
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([100.0, 0.0, 0.0])),
    );
    rejected_instance.insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([140.0, 0.0, 0.0])),
    );
    rejected_instance.insert(
        fields["bCollideWorld"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime.collision = Some(solid_box_collision());
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    let brush_hit = runtime
        .try_move_actor(
            1,
            &ResolvedObject {
                package: Arc::clone(&other_projectile_package),
                export_index: 0,
            },
            [-80.0, 0.0, 0.0],
            &mut rejected_instance,
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(brush_hit.actor, Some(0));
    assert_eq!(
        brush_hit.node,
        Some(0),
        "mover brush hits must retain the surface node used by mounting"
    );
    let rejected_class = ResolvedObject {
        package: Arc::clone(&other_projectile_package),
        export_index: 0,
    };
    let mount_location = Vec3::from_array(
        runtime
            .actor_vector(&rejected_class, &rejected_instance, "Location")
            .unwrap(),
    );
    assert!(
        runtime
            .movement_hit_has_poly_flag(
                1,
                &rejected_instance,
                brush_hit,
                mount_location,
                21.0,
                PolyFlags::HIGH_LEDGE,
            )
            .unwrap(),
        "mount surface flags must come from the hit mover brush"
    );
    let mut mount_instance = rejected_instance.clone();
    for (name, value) in [
        ("Rotation", Value::Rotator([0, 32_768, 0])),
        ("MaxMountHeight", Value::Float(40.0)),
        ("MaxStepHeight", Value::Float(4.0)),
        ("Physics", Value::Byte(1)),
    ] {
        runtime
            .set_actor_value(&rejected_class, &mut mount_instance, name, value)
            .unwrap();
    }
    let mut mount_actions = Vec::new();
    assert!(
        runtime
            .try_mount(
                1,
                &rejected_class,
                &mut mount_instance,
                brush_hit,
                &mut mount_actions,
            )
            .unwrap(),
        "a reachable high-ledge mover top must mount through actor-aware traces"
    );
    assert_eq!(
        runtime.actor_bases.get(&1),
        Some(&Some(runtime.actor_objects[&0].clone())),
        "mounting a mover must base the pawn on that mover"
    );
    runtime
        .set_actor_base(
            1,
            &rejected_class,
            &mut mount_instance,
            None,
            &mut mount_actions,
        )
        .unwrap();

    let mut moving_brush = runtime.instances.remove(&0).unwrap();
    moving_brush.insert(
        fields["bCollideWorld"].clone(),
        StoredValue::Value(Value::Bool(true)),
    );
    runtime
        .set_actor_value(
            &mover_class,
            &mut moving_brush,
            "OldPos",
            Value::Vector([100.0, 0.0, 0.0]),
        )
        .unwrap();
    for (name, value) in [
        ("BasePos", Value::Vector([0.0; 3])),
        ("OldRot", Value::Rotator([0; 3])),
        ("BaseRot", Value::Rotator([0; 3])),
        ("PhysAlpha", Value::Float(0.0)),
        ("PhysRate", Value::Float(1.0)),
        ("KeyNum", Value::Byte(1)),
        ("MoverGlideType", Value::Byte(0)),
        ("bInterpolating", Value::Bool(true)),
    ] {
        runtime
            .set_actor_value(&mover_class, &mut moving_brush, name, value)
            .unwrap();
    }
    moving_brush.insert(
        key_pos.clone(),
        StoredValue::Array(vec![StoredValue::Value(Value::Vector([0.0; 3])); 8]),
    );
    let key_rot = runtime
        .find_property(&mover_class, "KeyRot", 0)
        .unwrap()
        .unwrap();
    moving_brush.insert(
        key_rot,
        StoredValue::Array(vec![StoredValue::Value(Value::Rotator([0; 3])); 8]),
    );
    runtime.instances.insert(1, rejected_instance.clone());
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    runtime
        .tick_moving_brush(0, &mover_class, &mut moving_brush, 1.0, &mut Vec::new())
        .unwrap();
    assert_eq!(
        runtime
            .actor_bool(&mover_class, &moving_brush, "bInterpolating")
            .unwrap(),
        false
    );
    assert_eq!(
        runtime
            .actor_float_any(&mover_class, &moving_brush, "PhysAlpha")
            .unwrap(),
        0.7941,
        "moving brushes use UE1's 0.51-unit-shrunken world collision bounds"
    );
    assert!(
        Vec3::from_array(
            runtime
                .actor_vector(&mover_class, &moving_brush, "Location")
                .unwrap()
        )
        .abs_diff_eq(Vec3::new(20.59, 0.0, 0.0), 1.0e-5)
    );

    runtime.instances.get_mut(&1).unwrap().insert(
        fields["ZoneGravity"].clone(),
        StoredValue::Value(Value::Vector([0.0, 0.0, -200.0])),
    );
    runtime.collision = Some(placement_test_collision(100.0));
    for (name, value) in [
        ("Location", Value::Vector([0.0, 0.0, 200.0])),
        ("Velocity", Value::Vector([0.0; 3])),
        ("OldPos", Value::Vector([0.0, 0.0, 200.0])),
        ("BasePos", Value::Vector([0.0, 0.0, 200.0])),
        ("bInterpolating", Value::Bool(false)),
    ] {
        runtime
            .set_actor_value(&mover_class, &mut moving_brush, name, value)
            .unwrap();
    }
    moving_brush.insert(
        key_pos.clone(),
        StoredValue::Array(vec![StoredValue::Value(Value::Vector([0.0; 3])); 8]),
    );
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    runtime
        .tick_moving_brush(0, &mover_class, &mut moving_brush, 1.0, &mut Vec::new())
        .unwrap();
    let gravity_location = Vec3::from_array(
        runtime
            .actor_vector(&mover_class, &moving_brush, "Location")
            .unwrap(),
    );
    assert!(
        gravity_location.abs_diff_eq(Vec3::new(0.0, 0.0, 110.59), 1.0e-4),
        "gravity location is {gravity_location:?}"
    );
    assert!(
        Vec3::from_array(
            runtime
                .actor_vector(&mover_class, &moving_brush, "OldPos")
                .unwrap()
        )
        .abs_diff_eq(Vec3::new(0.0, 0.0, 110.59), 1.0e-4)
    );
    let Some(StoredValue::Array(key_positions)) = moving_brush.get(&key_pos) else {
        panic!("KeyPos is not an array");
    };
    let StoredValue::Value(Value::Vector(key_position)) = key_positions[1] else {
        panic!("KeyPos[1] is not a vector");
    };
    assert!(Vec3::from_array(key_position).abs_diff_eq(Vec3::new(0.0, 0.0, -89.41), 1.0e-4));
    runtime.instances.get_mut(&1).unwrap().insert(
        fields["ZoneGravity"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );
    runtime.collision = Some(solid_box_collision());
    runtime
        .set_actor_value(
            &mover_class,
            &mut moving_brush,
            "OldPos",
            Value::Vector([100.0, 0.0, 0.0]),
        )
        .unwrap();
    runtime
        .set_actor_value(
            &mover_class,
            &mut moving_brush,
            "BasePos",
            Value::Vector([0.0; 3]),
        )
        .unwrap();
    moving_brush.insert(
        key_pos.clone(),
        StoredValue::Array(vec![StoredValue::Value(Value::Vector([0.0; 3])); 8]),
    );

    let encroaching_on = object_id(&package, 7);
    runtime.scripts.insert(
        encroaching_on.clone(),
        function_script(encroaching_on.export_index, vec![0x04, 0x27]),
    );
    runtime.function_lookups.insert(
        FunctionLookup::new(mover_class_id.clone(), None, "EncroachingOn", 0),
        Some(encroaching_on.clone()),
    );
    runtime.actor_states.insert(0, None);
    runtime.state_frames.remove(&0);
    for name in ["bBlockActors", "bBlockPlayers"] {
        moving_brush.insert(fields[name].clone(), StoredValue::Value(Value::Bool(true)));
    }
    moving_brush.insert(
        fields["bCollideWorld"].clone(),
        StoredValue::Value(Value::Bool(false)),
    );
    for (name, value) in [
        ("Location", Value::Vector([100.0, 0.0, 0.0])),
        ("PhysAlpha", Value::Float(0.0)),
        ("bInterpolating", Value::Bool(true)),
    ] {
        runtime
            .set_actor_value(&mover_class, &mut moving_brush, name, value)
            .unwrap();
    }
    let mut blocker = instance([0.0; 3]);
    for name in ["bBlockActors", "bBlockPlayers"] {
        blocker.insert(fields[name].clone(), StoredValue::Value(Value::Bool(true)));
    }
    runtime.instances.insert(1, blocker);
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    runtime
        .tick_moving_brush(0, &mover_class, &mut moving_brush, 1.0, &mut Vec::new())
        .unwrap();
    assert_eq!(
        runtime
            .actor_vector(&mover_class, &moving_brush, "Location")
            .unwrap(),
        [100.0, 0.0, 0.0],
        "a blocking EncroachingOn result restores the moving brush"
    );
    assert_eq!(
        runtime
            .actor_float_any(&mover_class, &moving_brush, "PhysAlpha")
            .unwrap(),
        0.0
    );
    runtime.instances.remove(&1);
    runtime.instances.insert(0, moving_brush);

    runtime
        .instances
        .get_mut(&0)
        .unwrap()
        .insert(fields["Brush"].clone(), StoredValue::Object(None));
    runtime.instances.get_mut(&0).unwrap().insert(
        fields["Location"].clone(),
        StoredValue::Value(Value::Vector([0.0; 3])),
    );

    runtime
        .instances
        .get_mut(&0)
        .unwrap()
        .insert(b_proj_target, StoredValue::Value(Value::Bool(false)));
    runtime.actor_states.insert(0, None);
    runtime.state_frames.remove(&0);
    runtime.object_actors.remove(&other_projectile_object);
    let actor_object = runtime_actor_id(4);
    runtime.object_actors.insert(actor_object.clone(), 1);
    runtime.actor_objects.insert(1, actor_object);
    runtime.actor_classes.insert(1, actor_class_id.clone());
    runtime.collision_actors.clear();
    runtime.collision_actors_by_min_x.clear();
    let mut actor_instance = instance([-40.0, 0.0, 0.0]);
    let mut actor_actions = Vec::new();
    let actor_hit = runtime
        .try_move_actor(
            1,
            &ResolvedObject {
                package: actor_package,
                export_index: 0,
            },
            [80.0, 0.0, 0.0],
            &mut actor_instance,
            &mut actor_actions,
        )
        .unwrap();
    assert_eq!(
        actor_hit.actor, None,
        "BT_AnyBump must notify without making non-blocking actors solid"
    );
    assert!(actor_actions.iter().any(|action| matches!(
        action,
        ActorAction::Log {
            actor: 0,
            message,
            tag: None,
        } if message == "DoOpen"
    )));
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
fn missing_loop_animation_is_a_no_op_through_extended_native_dispatch() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-missing-loop-animation-{}-{}",
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
    runtime.animation_sequences.insert(
        0,
        [(
            "all".to_owned(),
            AnimationSequence {
                group: String::new(),
                rate: 1.0,
                frame_count: 1,
                notifications: Vec::new(),
            },
        )]
        .into_iter()
        .collect(),
    );
    let bytecode = Bytecode {
        version: 76,
        raw_len: 9,
        // LoopAnim(Name'ClientTravel'); `ClientTravel` is absent from the actor mesh.
        bytes: vec![0x04, 0x61, 0x04, 0x21, 1, 0, 0, 0, 0x16],
        tokens: Vec::new(),
    };
    let mut instance = InstanceState::default();
    let mut actions = Vec::new();

    assert_eq!(
        Frame::new(&bytecode)
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
        Value::None
    );
    assert!(matches!(
        actions.as_slice(),
        [ActorAction::LoopAnimation { sequence, .. }] if sequence == "ClientTravel"
    ));
    assert!(!runtime.animation_commands.contains_key(&0));
    assert!(!runtime.animating.contains(&0));
    assert!(instance.is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn spawned_animation_command_waits_until_sequence_metadata_is_known() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-pending-spawn-animation-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(
        &package_path,
        synthetic_runtime_package_with_member("PlayerPawn", "Breathe"),
    )
    .unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let class = ResolvedObject {
        package: Arc::clone(&package),
        export_index: 0,
    };
    let class_id = object_id(&package, class.export_index);
    let fields = [
        ("AnimSequence", runtime_actor_id(800)),
        ("bAnimLoop", runtime_actor_id(801)),
        ("bAnimNotify", runtime_actor_id(802)),
        ("bAnimFinished", runtime_actor_id(803)),
        ("AnimFrame", runtime_actor_id(804)),
        ("AnimLast", runtime_actor_id(805)),
        ("AnimRate", runtime_actor_id(806)),
        ("AnimMinRate", runtime_actor_id(807)),
        ("TweenRate", runtime_actor_id(808)),
    ]
    .into_iter()
    .collect::<HashMap<_, _>>();
    for (name, field) in &fields {
        runtime.fields.insert(
            (class_id.clone(), name.to_ascii_lowercase()),
            Some(field.clone()),
        );
    }
    let instance = || {
        fields
            .iter()
            .map(|(name, field)| {
                let value = if *name == "AnimSequence" {
                    StoredValue::Name("None".to_owned())
                } else if name.starts_with('b') {
                    StoredValue::Value(Value::Bool(false))
                } else {
                    StoredValue::Value(Value::Float(0.0))
                };
                (field.clone(), value)
            })
            .collect::<InstanceState>()
    };
    let bytecode = Bytecode {
        version: 76,
        raw_len: 9,
        // LoopAnim(Name'Breathe') through the extended-native frame path.
        bytes: vec![0x04, 0x61, 0x04, 0x21, 1, 0, 0, 0, 0x16],
        tokens: Vec::new(),
    };

    let mut pending_instance = instance();
    let mut actions = Vec::new();
    Frame::new(&bytecode)
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
                &mut pending_instance,
                &mut actions,
                0,
            )
        })
        .unwrap();
    assert!(runtime.animation_commands.contains_key(&0));
    assert!(runtime.animating.contains(&0));
    assert_eq!(
        pending_instance.get(&fields["AnimSequence"]),
        Some(&StoredValue::Name("None".to_owned()))
    );
    runtime.actor_classes.insert(0, class_id.clone());
    runtime.instances.insert(0, pending_instance);
    runtime
        .set_actor_animation_sequences(
            0,
            [("breathe".to_owned(), String::new(), 2.0, 2, Vec::new())],
        )
        .unwrap();
    assert!(runtime.animation_commands.contains_key(&0));
    assert_eq!(
        runtime.instances[&0].get(&fields["AnimSequence"]),
        Some(&StoredValue::Name("Breathe".to_owned()))
    );

    let mut missing_instance = instance();
    let mut missing_actions = Vec::new();
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
                &mut missing_instance,
                &mut missing_actions,
                0,
            )
        })
        .unwrap();
    assert!(runtime.animation_commands.contains_key(&1));
    assert!(runtime.animating.contains(&1));
    runtime.actor_classes.insert(1, class_id);
    runtime.instances.insert(1, missing_instance);
    runtime
        .set_actor_animation_sequences(1, [("All".to_owned(), String::new(), 1.0, 1, Vec::new())])
        .unwrap();
    assert!(!runtime.animation_commands.contains_key(&1));
    assert!(!runtime.animating.contains(&1));
    assert_eq!(
        runtime.instances[&1].get(&fields["AnimSequence"]),
        Some(&StoredValue::Name("None".to_owned()))
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn particle_emitters_blend_immediate_superclass_defaults() {
    let root = std::env::temp_dir().join(format!(
        "openhp1-runtime-particle-parent-{}-{}",
        std::process::id(),
        FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let system = root.join("System");
    fs::create_dir_all(&system).unwrap();
    fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
    let package_path = system.join("Test.u");
    fs::write(&package_path, synthetic_particle_parent_package()).unwrap();

    let mut runtime = ScriptRuntime::new(&root).unwrap();
    let package = runtime.packages.load_path(&package_path).unwrap();
    let parent_class = object_id(&package, 0);
    let child_class = object_id(&package, 1);
    let fields = [
        ("ParticlesPerSec", runtime_actor_id(500)),
        ("SourceWidth", runtime_actor_id(501)),
        ("SizeWidth", runtime_actor_id(502)),
        ("AlphaStart", runtime_actor_id(503)),
        ("AlphaEnd", runtime_actor_id(504)),
        ("ParentBlend", runtime_actor_id(505)),
    ]
    .into_iter()
    .collect::<std::collections::HashMap<_, _>>();
    for class in [&parent_class, &child_class] {
        for (name, field) in &fields {
            runtime.fields.insert(
                (class.clone(), name.to_ascii_lowercase()),
                Some(field.clone()),
            );
        }
    }
    let float_param = |base, random| {
        StoredValue::Value(Value::Struct(std::collections::HashMap::from([
            ("Base".to_owned(), Value::Float(base)),
            ("Rand".to_owned(), Value::Float(random)),
        ])))
    };
    let values = |rate, source_width, size_width, alpha_start, alpha_end, parent_blend| {
        [
            (fields["ParticlesPerSec"].clone(), float_param(rate, 0.0)),
            (
                fields["SourceWidth"].clone(),
                float_param(source_width, 0.0),
            ),
            (fields["SizeWidth"].clone(), float_param(size_width, 0.0)),
            (fields["AlphaStart"].clone(), float_param(alpha_start, 0.0)),
            (fields["AlphaEnd"].clone(), float_param(alpha_end, 0.0)),
            (
                fields["ParentBlend"].clone(),
                StoredValue::Value(Value::Float(parent_blend)),
            ),
        ]
        .into_iter()
        .collect::<InstanceState>()
    };
    runtime.class_defaults.insert(
        parent_class.clone(),
        values(16.0, 12.0, 6.0, 0.75, 1.0, 0.0),
    );
    for (actor, blend) in [(7, 0.25), (8, -0.5)] {
        runtime.actor_classes.insert(actor, child_class.clone());
        runtime
            .instances
            .insert(actor, values(8.0, 4.0, 2.0, 0.25, 0.5, blend));
    }

    let mut emitters = runtime.particle_emitters().unwrap();
    emitters.sort_by_key(|emitter| emitter.actor);

    assert_eq!(emitters[0].parent_particles_per_second.unwrap().base, 16.0);
    assert_eq!(emitters[0].source_width.base, 6.0);
    assert_eq!(emitters[0].size_width.base, 3.0);
    assert_eq!(emitters[0].alpha_start.base, 0.375);
    assert_eq!(emitters[0].alpha_end.base, 0.625);
    assert_eq!(emitters[1].parent_particles_per_second.unwrap().base, 16.0);
    assert_eq!(emitters[1].source_width.base, 4.0);

    fs::remove_dir_all(root).unwrap();
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
