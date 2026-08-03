use std::{path::Path, sync::Arc};

use glam::Vec3;
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

use crate::{
    Frame, FrameRequest, FrameResponse, FrameRun, FrameSnapshot, FunctionCall, PlayerInput,
    PlayerView, StructMember, Value,
};

mod action;
mod actor;
mod error;
mod execution;
mod instance;
mod movement;
mod native;
mod physics;
mod state;

pub use action::{
    ActorAction, ParticleColor, ParticleEmitter, ParticleFloat, ParticleTexture, PlayerMusic,
    RuntimeObject, WeaponAttachment,
};
pub use error::{DispatchError, DispatchResult};

const GOTO_STATE: u16 = 0x071;
const ENABLE: u16 = 0x075;
const DISABLE: u16 = 0x076;
const DESTROY: u16 = 0x117;
const ALL_ACTORS: u16 = 0x130;
const TRACE_ACTORS: u16 = 0x135;
const RADIUS_ACTORS: u16 = 0x136;
const VISIBLE_ACTORS: u16 = 0x137;
const VISIBLE_COLLIDING_ACTORS: u16 = 0x138;
const SLEEP: u16 = 0x100;
const BONE_POS: u16 = 0x101;
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
const GET_WORLD_COLLISION_BOX: u16 = 0x11e;
const SET_BASE: u16 = 0x12a;
const SET_ROTATION: u16 = 0x12b;
const ROT_RAND: u16 = 0x140;
const WARP: u16 = 0x13a;
const UNWARP: u16 = 0x13b;
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
const LINE_OF_SIGHT_TO: u16 = 514;
const PICK_TARGET: u16 = 531;
const ADD_PAWN: u16 = 529;
const REMOVE_PAWN: u16 = 530;
const PLAYER_CAN_SEE_ME: u16 = 532;
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
    frame_zero_values: HashMap<ObjectId, Arc<FrameZeroValues>>,
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
    player_alt_fire_pressed: bool,
    animation_sequences: HashMap<usize, HashMap<String, AnimationSequence>>,
    actor_bone_names: HashMap<usize, Vec<String>>,
    actor_bone_positions: HashMap<usize, Vec<[f32; 3]>>,
    actor_visual_bounds: HashMap<usize, (Vec3, Vec3)>,
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

#[derive(Clone, Debug, PartialEq)]
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
    notifications: Vec<(f32, String)>,
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

struct FrameZeroValues {
    locals: Vec<(i32, Value)>,
    array_elements: Vec<(i32, Value)>,
}

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

fn particle_acceleration(gravity: Vec3, zone_gravity: Vec3, modifier: f32) -> Vec3 {
    gravity + zone_gravity * modifier
}

fn zone_actor_at(
    collision: &BspCollision,
    location: Vec3,
    level_package: Option<&Arc<str>>,
    object_actors: &HashMap<ObjectId, usize>,
    level_info: Option<usize>,
) -> Option<usize> {
    collision
        .zone_at(location)
        .and_then(|zone| collision.zone_actor_export(zone))
        .and_then(|export_index| {
            level_package.and_then(|package| {
                object_actors
                    .get(&ObjectId {
                        package: Arc::clone(package),
                        export_index,
                    })
                    .copied()
            })
        })
        .or(level_info)
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
#[path = "world_tests.rs"]
mod tests;
