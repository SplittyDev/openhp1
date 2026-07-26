use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),
    #[error("{field} has invalid count {count}")]
    InvalidCount { field: &'static str, count: i32 },
    #[error("{field} lazy array ended at {actual:#x}, expected {expected:#x}")]
    InvalidLazyArray {
        field: &'static str,
        actual: usize,
        expected: u32,
    },
    #[error("{field} index {index} is outside 0..{length}")]
    InvalidIndex {
        field: &'static str,
        index: usize,
        length: usize,
    },
    #[error(
        "mesh animation has {frame_vertices} vertices per frame and {animation_frames} frames, but {vertex_count} vertices"
    )]
    InvalidAnimationLayout {
        frame_vertices: usize,
        animation_frames: usize,
        vertex_count: usize,
    },
    #[error(
        "animation sequence {name} uses frames {start_frame}..{end_frame}, but the mesh has {animation_frames} frames"
    )]
    InvalidAnimationSequence {
        name: String,
        start_frame: usize,
        end_frame: usize,
        animation_frames: usize,
    },
    #[error("animation sequence {name} has no frames")]
    EmptyAnimationSequence { name: String },
    #[error("animation frame {frame} is outside 0..{animation_frames}")]
    InvalidAnimationFrame {
        frame: usize,
        animation_frames: usize,
    },
    #[error("animation phase must be finite, got {0}")]
    InvalidAnimationPhase(f32),
    #[error("mesh has no vertex animation frames")]
    NoVertexAnimation,
    #[error("unsupported mesh class {0}")]
    UnsupportedClass(String),
}

pub type Result<T> = std::result::Result<T, Error>;
