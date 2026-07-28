use std::{path::Path, sync::Arc};

use openhp1_audio::AudioClip;
use openhp1_map::{Level, Model};
use openhp1_package::{
    ObjectReader, ObjectReference, Package, PackageStore, PropertyKind, ResolveError,
    ResolvedObject,
};
use openhp1_physics::BspCollision;
use openhp1_script::{
    Bytecode, PropertyMetadata, ScriptExport, ScriptMetadata, class_defaults_reader,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use thiserror::Error;

use crate::{
    Frame, FrameRequest, FrameResponse, FrameRun, FrameSnapshot, FunctionCall, PlayerInput,
    PlayerView, StructMember, Value,
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
const TRACE_ACTORS: u16 = 0x135;
const SLEEP: u16 = 0x100;
const CLASS_IS_CHILD_OF: u16 = 0x102;
const PLAY_ANIM: u16 = 0x103;
const LOOP_ANIM: u16 = 0x104;
const FINISH_ANIM: u16 = 0x105;
const SET_COLLISION: u16 = 0x106;
const PLAY_SOUND: u16 = 0x108;
const STOP_SOUND: u16 = 0x238;
const CREATE_ANIM_CHANNEL: u16 = 0x109;
const SET_OWNER: u16 = 0x110;
const MOVE: u16 = 0x10a;
const SET_LOCATION: u16 = 0x10b;
const SPAWN: u16 = 0x116;
const TRACE: u16 = 0x115;
const SET_TIMER: u16 = 0x118;
const IS_IN_STATE: u16 = 0x119;
const IS_ANIMATING: u16 = 0x11a;
const SET_COLLISION_SIZE: u16 = 0x11b;
const GET_STATE_NAME: u16 = 0x11c;
const TRACE_TEXTURE: u16 = 0x11d;
const SET_BASE: u16 = 0x12a;
const SET_ROTATION: u16 = 0x12b;
const GET_ANIM_GROUP: u16 = 0x125;
const BONE_NUMBER: u16 = 0x10c;
const TWEEN_ANIM: u16 = 0x126;
const FINISH_INTERPOLATION: u16 = 0x12d;
const IS_A: u16 = 0x12f;
const MOVE_TO: u16 = 500;
const MOVE_TOWARD: u16 = 502;
const TURN_TO: u16 = 508;
const TURN_TOWARD: u16 = 510;
const MAKE_NOISE: u16 = 512;
const PICK_TARGET: u16 = 531;
const ADD_PAWN: u16 = 529;
const CAN_SEE: u16 = 533;
const SAVE_CONFIG: u16 = 536;
const COMPARE_GESTURE: u16 = 426;
const COMPARE_GESTURE_POINT: u16 = 427;
const RAND_RANGE: u16 = 0x409;
const SET_PHYSICS: u16 = 0xf82;
const MOVE_SMOOTH: u16 = 0xf81;
const LOG: u16 = 0x0e7;
const V_RAND: u16 = 0x0fc;
const FIND_PATH: u16 = 0x229;
const CLASS_ABSTRACT: u32 = 0x0000_0001;
const MAX_CALL_DEPTH: usize = 64;
const PROPERTY_PARAMETER: u32 = 0x80;
const PROPERTY_OUTPUT: u32 = 0x100;
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
pub struct PlayerMusic {
    pub clip: Option<AudioClip>,
    pub section: u8,
    pub transition: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleFloat {
    pub base: f32,
    pub random: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticleTexture {
    pub package: String,
    pub export_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitter {
    pub actor: usize,
    pub owner: Option<usize>,
    pub emit: bool,
    pub prime: bool,
    pub distribution: u8,
    pub style: u8,
    pub unlit: bool,
    pub particles_alive: usize,
    pub particles_max: usize,
    pub particles_emitted: usize,
    pub particles_per_second: ParticleFloat,
    pub period: ParticleFloat,
    pub lifetime: ParticleFloat,
    pub speed: ParticleFloat,
    pub angular_spread_width: ParticleFloat,
    pub angular_spread_height: ParticleFloat,
    pub source_width: ParticleFloat,
    pub source_height: ParticleFloat,
    pub source_depth: ParticleFloat,
    pub size_width: ParticleFloat,
    pub size_length: ParticleFloat,
    pub size_end_scale: ParticleFloat,
    pub size_delay: f32,
    pub size_grow_period: f32,
    pub draw_scale: f32,
    pub system_relative: bool,
    pub gravity: [f32; 3],
    pub pattern: Vec<[f32; 3]>,
    pub textures: Vec<ParticleTexture>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponAttachment {
    pub pawn: usize,
    pub weapon: usize,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActorAction {
    PlayAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
        root_motion: bool,
    },
    LoopAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
        root_motion: bool,
    },
    AwaitAnimation {
        actor: usize,
    },
    PlaySound {
        actor: usize,
        clip: AudioClip,
        location: [f32; 3],
        slot: u8,
        volume: f32,
        no_override: bool,
        radius: f32,
        pitch: f32,
    },
    StopSound {
        actor: usize,
        clip: Option<AudioClip>,
        slot: Option<u8>,
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
    SetPrePivot {
        actor: usize,
        pre_pivot: [f32; 3],
    },
    SetHidden {
        actor: usize,
        hidden: bool,
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
            | Self::PlaySound { actor, .. }
            | Self::StopSound { actor, .. }
            | Self::SpawnActor { actor, .. }
            | Self::SetLocation { actor, .. }
            | Self::SetRotation { actor, .. }
            | Self::SetPrePivot { actor, .. }
            | Self::SetHidden { actor, .. }
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

    #[error("the level has no registered player pawn")]
    MissingPlayer,

    #[error("the level has no registered LevelInfo")]
    MissingLevelInfo,

    #[error("player input is invalid: {message}")]
    InvalidPlayerInput { message: String },

    #[error("player view is invalid: {message}")]
    InvalidPlayerView { message: String },

    #[error("named native function `{class}.{function}` is not implemented")]
    UnimplementedNamedNative { class: String, function: String },

    #[error("name `{name}` is missing from package `{package}`")]
    MissingName { package: Arc<str>, name: String },

    #[error("runtime object is unresolved: {message}")]
    UnresolvedObject { message: String },

    #[error("runtime delta time {value} is invalid")]
    InvalidDeltaTime { value: f32 },

    #[error("player touch location {location:?} is invalid")]
    InvalidPlayerLocation { location: [f32; 3] },

    #[error("player touch collision failed: {0}")]
    PlayerTouchCollision(String),

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

    #[error("struct property `{property}` has no struct type")]
    MissingStructType { property: String },

    #[error("struct field `{field}` has unsupported type `{kind}`")]
    UnsupportedStructField { field: String, kind: String },
}

pub struct ScriptRuntime {
    packages: PackageStore,
    scripts: HashMap<ObjectId, Arc<ScriptExport>>,
    function_lookups: HashMap<FunctionLookup, Option<ObjectId>>,
    state_lookups: HashMap<StateLookup, Option<ObjectId>>,
    instances: HashMap<usize, InstanceState>,
    object_instances: HashMap<ObjectId, (ObjectId, InstanceState)>,
    class_defaults: HashMap<ObjectId, InstanceState>,
    class_relations: HashMap<(ObjectId, ObjectId), bool>,
    fields: HashMap<(ObjectId, String), Option<ObjectId>>,
    resolved_references: HashMap<(Arc<str>, i32), Option<ObjectId>>,
    zero_values: HashMap<ObjectId, Option<Value>>,
    frame_arguments: HashMap<ObjectId, ArgumentBindings>,
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
    failed_physics: HashMap<usize, u8>,
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
    reach_specs: Vec<NavigationReachSpec>,
    level_package: Option<Arc<str>>,
    level_info: Option<usize>,
    player_actor: Option<usize>,
    animation_sequences: HashMap<usize, HashMap<String, AnimationSequence>>,
    actor_bone_names: HashMap<usize, Vec<String>>,
    animation_commands: HashMap<usize, AnimationCommand>,
    animating: HashSet<usize>,
    player_probe_touching: HashSet<usize>,
    collision_fields: HashMap<ObjectId, movement::CollisionFields>,
    brush_collisions: HashMap<ObjectId, Arc<BspCollision>>,
    collision_actors: Vec<Option<movement::CachedCollisionActor>>,
    collision_actors_by_min_x: Vec<usize>,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StateLookup {
    class: ObjectId,
    state: String,
}

impl StateLookup {
    fn new(class: ObjectId, state: &str) -> Self {
        Self {
            class,
            state: state.to_ascii_lowercase(),
        }
    }
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

#[derive(Clone, Debug)]
struct AnimationSequence {
    group: String,
    rate: f32,
    frame_count: usize,
}

#[derive(Clone, Debug)]
struct AnimationCommand {
    sequence: String,
    relative_rate: f32,
    tween_time: f32,
    looping: bool,
    tween_only: bool,
}

#[derive(Clone, Debug)]
struct NavigationReachSpec {
    distance: i32,
    start: ObjectId,
    end: ObjectId,
    collision_radius: i32,
    collision_height: i32,
    pruned: bool,
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
    FinishAnimation(usize),
    FinishInterpolation(usize),
    MoveTo(usize),
    MoveToward(usize),
    TurnTo(usize),
    TurnToward(usize),
}

type InstanceState = HashMap<ObjectId, StoredValue>;
type ArgumentBindings = Arc<Vec<(i32, usize, bool)>>;

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
        actor::advance_lifespan,
        actor::advance_timer,
        actor::decode_latent_action,
        actor::update_touching_array,
        native::{
            animation_parameters, bone_number, collision_updates, log_arguments,
            next_navigation_step, noise_loudness, random_float, random_int, random_unit_vector,
            scalar_native, sound_arguments, target_score, trace_texture,
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
    fn positive_lifespans_expire_once_at_zero() {
        let mut lifespan = 0.1;
        assert!(!advance_lifespan(&mut lifespan, 0.04));
        assert!(advance_lifespan(&mut lifespan, 0.07));
        assert_eq!(lifespan, 0.0);
        assert!(!advance_lifespan(&mut lifespan, 0.1));
    }

    #[test]
    fn decodes_finish_interpolation_latent_state() {
        assert_eq!(
            decode_latent_action(0x12e, 7),
            LatentAction::FinishInterpolation(7)
        );
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
    fn bone_numbers_follow_case_insensitive_skeletal_order() {
        let bones = vec!["Root".to_owned(), "Head".to_owned()];
        assert_eq!(bone_number(Some(&bones), "head"), 1);
        assert_eq!(bone_number(Some(&bones), "missing"), 0);
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
    fn named_native_shims_validate_their_engine_calls() {
        assert_eq!(
            execution::named_native(
                "PlayerPawn",
                "ConsoleCommand",
                &[Value::String("GETPING".to_owned())]
            ),
            Some(Value::String(String::new()))
        );
        assert_eq!(
            execution::named_native("Decal", "DetachDecal", &[]),
            Some(Value::None)
        );
        assert_eq!(
            execution::named_native("PlayerPawn", "ConsoleCommand", &[]),
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
        assert!(
            log_arguments(&[Value::String("hello".to_owned()), Value::None, Value::None]).is_err()
        );
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
}
