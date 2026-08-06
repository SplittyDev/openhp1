//! UnrealScript execution and package-backed runtime state.

mod console;
mod error;
mod frame;
mod opcode;
mod player;
mod value;
mod world;

pub use console::{ConsoleCommandAction, ConsoleCommands};
pub use error::{Error, Result};
pub use frame::{Frame, FunctionCall};
pub use player::{PlayerInput, PlayerView};
pub use value::Value;
pub use world::{
    ActorAction, ConsoleCommandHost, ConsoleCommandResponse, DispatchError, DispatchResult,
    ParticleColor, ParticleEmitter, ParticleFloat, ParticleTexture, ParticleWind, PlayerMusic,
    PlayerTravelState, PlayerUiState, RuntimeObject, ScriptRuntime, WeaponAttachment,
};

pub(crate) use frame::{
    FrameRequest, FrameResponse, FrameRun, FrameSnapshot, IteratorValue, StructMember, rotator_axes,
};

const MAX_EXPRESSION_DEPTH: usize = 64;
