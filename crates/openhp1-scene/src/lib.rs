//! CPU-side scene assembly for decoded OpenHP1 levels.

mod actor;
mod coordinates;
mod loader;
mod render;
mod runtime;
mod texture_overrides;
mod window_masks;

pub use actor::{SceneActor, SceneActorAnimation, SceneActorRenderRange, SceneObjectId};
pub use coordinates::{render_to_unreal, unreal_to_render};
pub use loader::LoadedScene;
pub use openhp1_map::{BspNode, LightVisibility, LightmapImage, Rotator, SkyZone, TriangleMesh};
pub use render::{
    ActorSubmission, Corona, CoronaVisibility, RenderLight, RenderLightmap, RenderScene,
    SurfaceMaterial, SurfaceMode, TextureImage, TextureMipImage, TransmissionMask, WarpCoordinates,
    WarpPortal,
};
pub use runtime::{
    apply_runtime_actions, apply_runtime_actions_with, initialize_runtime, initialize_runtime_with,
    initialize_runtime_with_console, initialize_runtime_with_console_unstarted, sync_runtime_pose,
};
pub use texture_overrides::load_texture_override;
