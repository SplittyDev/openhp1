use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use openhp1_package::{
    ObjectReader, ObjectReference, Package, PackageStore, PropertyKind, ResolveError,
    ResolvedObject,
};
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
mod native;
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
const SET_LOCATION: u16 = 0x10b;
const SET_TIMER: u16 = 0x118;
const SET_ROTATION: u16 = 0x12b;
const MAX_CALL_DEPTH: usize = 64;
const PROPERTY_PARAMETER: u32 = 0x80;
const PROPERTY_RETURN: u32 = 0x400;
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
    },
    LoopAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
    },
    AwaitAnimation {
        actor: usize,
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
    DeferredCall {
        actor: usize,
        message: String,
    },
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
    object_handles: HashMap<ObjectId, i32>,
    handle_objects: Vec<ObjectId>,
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
}

type InstanceState = HashMap<ObjectId, StoredValue>;

fn object_id(package: &Arc<Package>, export_index: usize) -> ObjectId {
    ObjectId {
        package: Arc::clone(&package.summary().source),
        export_index,
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
        native::{collision_updates, scalar_native},
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
    fn collision_updates_preserve_omitted_flags() {
        assert_eq!(
            collision_updates(&[Value::Bool(true), Value::None]),
            Ok([Some(true), None, None])
        );
        assert!(collision_updates(&[Value::Float(1.0)]).is_err());
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
