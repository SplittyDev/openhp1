//! Decoding and pose sampling for the mesh assets used by Harry Potter 1.

mod animation;
mod decode;
mod error;
mod geometry;
#[cfg(test)]
mod test_support;
mod types;

pub use error::{Error, Result};
pub use types::{
    Mesh, MeshAnimationNotify, MeshAnimationSequence, MeshSample, MeshTriangle, MeshVertex,
    SkeletalAnimation,
};
