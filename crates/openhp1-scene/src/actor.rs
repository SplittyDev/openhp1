use std::ops::Range;

use glam::Vec3;

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
    pub draw_scale: f32,
    pub draw_type: u8,
    pub hidden: bool,
    pub unlit: bool,
    pub mesh: Option<SceneObjectId>,
    pub mesh_name: Option<String>,
    pub animation: Option<SceneActorAnimation>,
    pub render: Option<SceneActorRenderRange>,
    /// Decode or rendering limitations attached to this actor.
    pub diagnostics: Vec<String>,
}
