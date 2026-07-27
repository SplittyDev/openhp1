use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use openhp1_package::{
    ObjectReader, ObjectReference, Package, PackageStore, PropertyKind, ResolveError,
    ResolvedObject,
};
use openhp1_physics::BspCollision;
use openhp1_script::{
    Bytecode, PropertyMetadata, ScriptExport, ScriptMetadata, class_defaults_reader,
};
use thiserror::Error;

use crate::{
    Frame, FrameRequest, FrameResponse, FrameRun, FrameSnapshot, FunctionCall, StructMember, Value,
};

mod actor;
mod execution;
mod instance;
mod movement;
mod native;
mod physics;
mod state;

const GOTO_STATE: u16 = 0x071;
const ENABLE: u16 = 0x075;
const DISABLE: u16 = 0x076;
const DESTROY: u16 = 0x117;
const ALL_ACTORS: u16 = 0x130;
const SLEEP: u16 = 0x100;
const PLAY_ANIM: u16 = 0x103;
const LOOP_ANIM: u16 = 0x104;
const FINISH_ANIM: u16 = 0x105;
const SET_COLLISION: u16 = 0x106;
const PLAY_SOUND: u16 = 0x108;
const MOVE: u16 = 0x10a;
const SET_LOCATION: u16 = 0x10b;
const SPAWN: u16 = 0x116;
const SET_TIMER: u16 = 0x118;
const SET_BASE: u16 = 0x12a;
const SET_ROTATION: u16 = 0x12b;
const MOVE_TO: u16 = 500;
const RAND_RANGE: u16 = 0x409;
const SET_PHYSICS: u16 = 0xf82;
const LOG: u16 = 0x0e7;
const CLASS_ABSTRACT: u32 = 0x0000_0001;
const MAX_CALL_DEPTH: usize = 64;
const PROPERTY_PARAMETER: u32 = 0x80;
const PROPERTY_RETURN: u32 = 0x400;
const FUNCTION_NATIVE: u32 = 0x0000_0400;
const STATE_AUTO: u32 = 0x0000_0002;
const PROBE_EVENTS: [&str; 64] = [
    "Spawned",
    "Destroyed",
    "GainedChild",
    "LostChild",
    "Probe4",
    "Probe5",
    "Trigger",
    "UnTrigger",
    "Timer",
    "HitWall",
    "Falling",
    "Landed",
    "ZoneChange",
    "Touch",
    "UnTouch",
    "Bump",
    "BeginState",
    "EndState",
    "BaseChange",
    "Attach",
    "Detach",
    "ActorEntered",
    "ActorLeaving",
    "KillCredit",
    "AnimEnd",
    "EndedRotation",
    "InterpolateEnd",
    "EncroachingOn",
    "EncroachedBy",
    "FootZoneChange",
    "HeadZoneChange",
    "PainTimer",
    "SpeechTimer",
    "MayFall",
    "Probe34",
    "Die",
    "Tick",
    "PlayerTick",
    "Expired",
    "Probe39",
    "SeePlayer",
    "EnemyNotVisible",
    "HearNoise",
    "UpdateEyeHeight",
    "SeeMonster",
    "SeeFriend",
    "SpecialHandling",
    "BotDesireability",
    "Probe48",
    "Probe49",
    "Probe50",
    "Probe51",
    "Probe52",
    "Probe53",
    "Probe54",
    "Probe55",
    "Probe56",
    "Probe57",
    "Probe58",
    "Probe59",
    "Probe60",
    "Probe61",
    "Probe62",
    "All",
];

pub type DispatchResult<T> = std::result::Result<T, DispatchError>;

#[derive(Clone, Debug, PartialEq)]
pub enum ActorAction {
    PlayAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
    },
    LoopAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
    },
    AwaitAnimation {
        actor: usize,
    },
    SpawnActor {
        actor: usize,
        name: String,
        class_package: Arc<str>,
        class_export: usize,
        class_name: String,
        location: [f32; 3],
        rotation: [i32; 3],
    },
    SetLocation {
        actor: usize,
        location: [f32; 3],
    },
    SetRotation {
        actor: usize,
        rotation: [i32; 3],
    },
    DestroyActor {
        actor: usize,
    },
    Log {
        actor: usize,
        message: String,
        tag: Option<String>,
    },
    DeferredCall {
        actor: usize,
        message: String,
    },
    DispatchEvent {
        actor: usize,
        event: &'static str,
        arguments: Vec<Value>,
    },
}

impl ActorAction {
    pub fn actor(&self) -> usize {
        match self {
            Self::PlayAnimation { actor, .. }
            | Self::LoopAnimation { actor, .. }
            | Self::AwaitAnimation { actor }
            | Self::SpawnActor { actor, .. }
            | Self::SetLocation { actor, .. }
            | Self::SetRotation { actor, .. }
            | Self::DestroyActor { actor }
            | Self::Log { actor, .. }
            | Self::DeferredCall { actor, .. }
            | Self::DispatchEvent { actor, .. } => *actor,
        }
    }
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error(transparent)]
    Resolve(#[from] ResolveError),

    #[error(transparent)]
    Script(#[from] openhp1_script::Error),

    #[error(transparent)]
    Vm(#[from] crate::Error),

    #[error("script call depth exceeds {MAX_CALL_DEPTH}")]
    CallDepth,

    #[error("script class export {export_index} has no class metadata")]
    InvalidClass { export_index: usize },

    #[error("runtime object handle space is exhausted")]
    ObjectLimit,

    #[error("runtime object handle {handle} is invalid")]
    InvalidObjectHandle { handle: i32 },

    #[error("runtime object handle {handle} does not identify a registered actor")]
    InvalidActorHandle { handle: i32 },

    #[error("actor {actor} instance is already active")]
    ActiveActorContext { actor: usize },

    #[error("runtime actor {actor} is not registered")]
    UnregisteredActor { actor: usize },

    #[error("named native function `{class}.{function}` is not implemented")]
    UnimplementedNamedNative { class: String, function: String },

    #[error("name `{name}` is missing from package `{package}`")]
    MissingName { package: Arc<str>, name: String },

    #[error("runtime object is unresolved: {message}")]
    UnresolvedObject { message: String },

    #[error("runtime delta time {value} is invalid")]
    InvalidDeltaTime { value: f32 },

    #[error("state `{state}` has an invalid label table at execution offset {offset:#x}")]
    InvalidStateLabelTable { state: String, offset: usize },

    #[error("state `{state}` label `{label}` points outside {length} execution bytes")]
    InvalidStateLabel {
        state: String,
        label: String,
        length: usize,
    },

    #[error("script property export {export_index} has invalid array dimension {dimension}")]
    InvalidArrayDimension { export_index: usize, dimension: i32 },

    #[error("property `{property}` array index {index} is outside array length {length}")]
    ArrayPropertyIndex {
        property: String,
        index: usize,
        length: usize,
    },

    #[error("property `{property}` fixed-array storage is invalid")]
    InvalidArrayProperty { property: String },
}

pub struct ScriptRuntime {
    packages: PackageStore,
    scripts: HashMap<ObjectId, Arc<ScriptExport>>,
    function_lookups: HashMap<FunctionLookup, Option<ObjectId>>,
    instances: HashMap<usize, InstanceState>,
    class_defaults: HashMap<ObjectId, InstanceState>,
    class_relations: HashMap<(ObjectId, ObjectId), bool>,
    fields: HashMap<(ObjectId, String), Option<ObjectId>>,
    resolved_fields: HashMap<(Arc<str>, i32), Option<ObjectId>>,
    zero_values: HashMap<ObjectId, Option<Value>>,
    frame_arguments: HashMap<ObjectId, Arc<Vec<(i32, usize)>>>,
    struct_members: HashMap<ObjectId, Arc<Vec<(i32, StructMember)>>>,
    actor_classes: HashMap<usize, ObjectId>,
    actor_states: HashMap<usize, Option<String>>,
    state_frames: HashMap<usize, StateFrame>,
    state_revisions: HashMap<usize, u64>,
    active_state_actor: Option<usize>,
    pending_latent: Option<LatentAction>,
    state_resumes: usize,
    tick_functions: HashMap<usize, ResolvedObject>,
    failed_ticks: HashSet<usize>,
    disabled_events: HashMap<(usize, String), HashSet<String>>,
    object_actors: HashMap<ObjectId, usize>,
    actor_objects: HashMap<usize, ObjectId>,
    destroyed: HashSet<usize>,
    timers: HashMap<usize, ActorTimer>,
    timer_callbacks: usize,
    random_state: u32,
    object_handles: HashMap<ObjectId, i32>,
    handle_objects: Vec<ObjectId>,
    next_actor: usize,
    collision: Option<Arc<BspCollision>>,
    level_package: Option<Arc<str>>,
    level_info: Option<usize>,
    collision_fields: HashMap<ObjectId, movement::CollisionFields>,
    collision_actors: Vec<Option<movement::CachedCollisionActor>>,
    grounded_world: HashMap<usize, [f32; 3]>,
    actor_bases: HashMap<usize, Option<ObjectId>>,
    base_children: HashMap<ObjectId, Vec<usize>>,
    touching: HashSet<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ObjectId {
    package: Arc<str>,
    export_index: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FunctionLookup {
    class: ObjectId,
    state: Option<String>,
    function: String,
    depth: usize,
}

impl FunctionLookup {
    fn new(class: ObjectId, state: Option<&str>, function: &str, depth: usize) -> Self {
        Self {
            class,
            state: state.map(str::to_ascii_lowercase),
            function: function.to_ascii_lowercase(),
            depth,
        }
    }
}

#[derive(Clone, Debug)]
enum StoredValue {
    Value(Value),
    Array(Vec<StoredValue>),
    Name(String),
    Object(Option<ObjectId>),
    UnresolvedObject(String),
    SelfObject,
}

#[derive(Clone, Copy, Debug)]
struct ActorTimer {
    remaining: f32,
    rate: f32,
    looping: bool,
}

struct StateFrame {
    state: ObjectId,
    frame: FrameSnapshot,
    latent: LatentAction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LatentAction {
    Continue,
    Stop,
    Sleep(f32),
    FinishAnimation,
    MoveTo,
}

type InstanceState = HashMap<ObjectId, StoredValue>;

fn object_id(package: &Arc<Package>, export_index: usize) -> ObjectId {
    ObjectId {
        package: Arc::clone(&package.summary().source),
        export_index,
    }
}

fn runtime_actor_id(actor: usize) -> ObjectId {
    ObjectId {
        package: Arc::from("<runtime>"),
        export_index: actor,
    }
}

fn object_reference(index: i32) -> ObjectReference {
    if index == 0 {
        ObjectReference::None
    } else if index > 0 {
        ObjectReference::Export(index as usize - 1)
    } else {
        ObjectReference::Import(index.unsigned_abs() as usize - 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        actor::advance_timer,
        native::{
            animation_parameters, collision_updates, log_arguments, random_float, random_int,
            scalar_native,
        },
        state::{event_disabled, probe_event_index, set_event_disabled},
    };

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
    fn scalar_comparisons_cover_bool_int_and_float_families() {
        assert_eq!(
            scalar_native(0xf2, &[Value::Bool(true), Value::Bool(true)]),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            scalar_native(0xf3, &[Value::Bool(true), Value::Bool(false)]),
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
            scalar_native(0xec, &[Value::Int(0x141)]),
            Ok(Value::String("A".to_owned()))
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
        assert!(
            log_arguments(&[Value::String("hello".to_owned()), Value::None, Value::None]).is_err()
        );
    }

    #[test]
    fn disabled_events_are_case_insensitive_and_scoped_to_actor_state() {
        let mut disabled = HashMap::new();
        set_event_disabled(&mut disabled, 2, Some("Beano"), "Tick", true);

        assert!(event_disabled(&disabled, 2, Some("beano"), "TICK"));
        assert!(!event_disabled(&disabled, 2, Some("KillBean"), "Tick"));
        assert!(!event_disabled(&disabled, 3, Some("Beano"), "Tick"));

        set_event_disabled(&mut disabled, 2, Some("BEANO"), "tick", false);
        assert!(!event_disabled(&disabled, 2, Some("Beano"), "Tick"));
        assert_eq!(probe_event_index("tick"), Some(36));
    }

    #[test]
    fn fmax_matches_unreal_native_ordering() {
        assert_eq!(
            scalar_native(0xf5, &[Value::Float(2.0), Value::Float(3.0)]),
            Ok(Value::Float(3.0))
        );
        assert_eq!(
            scalar_native(0xf5, &[Value::Float(2.0), Value::Float(f32::NAN)]),
            Ok(Value::Float(2.0))
        );
    }

    #[test]
    fn basic_vector_arithmetic_matches_unreal_natives() {
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
    }

    #[test]
    fn requested_core_math_and_random_natives_match_unreal_semantics() {
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
}
