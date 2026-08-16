//! Minimal wgpu renderer for decoded HP1 geometry.

mod camera;
mod renderer;
mod settings;

pub use camera::{Camera, SceneBounds};
pub use openhp1_scene::{
    ActorSubmission, BspNode, RenderScene, SurfaceMaterial, SurfaceMode, TextureImage,
    WarpCoordinates, render_to_unreal, unreal_to_render,
};
pub use renderer::{RenderStats, Renderer};
pub use settings::{
    AmbientOcclusion, Antialiasing, DisplaySettings, RendererMode, RendererSettingError,
    RendererSettings, ToneMapper, VolumetricDebugView, VolumetricTuning,
};
