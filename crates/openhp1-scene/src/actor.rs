use std::ops::Range;

use glam::{Mat4, Vec3};

use crate::Rotator;

/// Stable identity for an exported object within one original package.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SceneObjectId {
    pub package: String,
    /// Zero-based package export index.
    pub export_index: usize,
}

#[derive(Clone, Debug)]
pub struct SceneActorAnimation {
    pub sequence: String,
    pub phase: f32,
    pub rate: f32,
    pub frame_count: usize,
}

#[derive(Clone, Debug)]
pub struct SceneActorRenderRange {
    pub vertices: Range<usize>,
    pub indices: Range<usize>,
}

/// Decoded state retained for one actor referenced by a level.
#[derive(Clone, Debug)]
pub struct SceneActor {
    pub id: SceneObjectId,
    pub name: String,
    pub class: Option<SceneObjectId>,
    pub class_name: String,
    /// Unreal-space location.
    pub location: Vec3,
    pub rotation: Rotator,
    pub pre_pivot: Vec3,
    pub main_scale: Vec3,
    pub draw_scale: f32,
    pub draw_type: u8,
    pub hidden: bool,
    pub unlit: bool,
    pub brush: Option<SceneObjectId>,
    pub mesh: Option<SceneObjectId>,
    pub mesh_name: Option<String>,
    pub animation: Option<SceneActorAnimation>,
    pub render: Option<SceneActorRenderRange>,
    pub(crate) mesh_transform: Option<Mat4>,
    pub(crate) mesh_to_object: Option<Mat4>,
    pub(crate) visual_bounds: Option<(Vec3, Vec3)>,
    /// Decode or rendering limitations attached to this actor.
    pub diagnostics: Vec<String>,
}
