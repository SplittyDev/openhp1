use std::sync::Arc;

use openhp1_package::ResolveError;
use thiserror::Error;

use super::MAX_CALL_DEPTH;

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

    #[error("runtime actor {actor} placement failed: {message}")]
    ActorPlacement { actor: usize, message: String },

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

    #[error("array property `{property}` has no inner type")]
    MissingArrayInner { property: String },

    #[error(
        "array property `{property}` has invalid length {length} for {remaining} payload bytes"
    )]
    InvalidDynamicArrayLength {
        property: String,
        length: i32,
        remaining: usize,
    },

    #[error("struct property `{property}` has no struct type")]
    MissingStructType { property: String },

    #[error("struct field `{field}` has unsupported type `{kind}`")]
    UnsupportedStructField { field: String, kind: String },

    #[error("config property `{property}` is invalid: {message}")]
    InvalidConfigValue { property: String, message: String },

    #[error("save state is invalid: {message}")]
    SaveState { message: String },
}

pub type DispatchResult<T> = std::result::Result<T, DispatchError>;
