//! Decoding of the vertex-mesh assets used by Harry Potter 1.

mod decode;
mod error;
mod geometry;
mod types;

pub use error::{Error, Result};
pub use types::{Mesh, MeshAnimationNotify, MeshAnimationSequence, MeshTriangle, MeshVertex};
