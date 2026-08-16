//! UE1-compatible collision queries over decoded map geometry.

mod actor;
mod bsp;

pub use actor::{ActorCollisionHit, boxes_overlap, cylinders_overlap, sweep_box, sweep_cylinder};
pub use bsp::{BspCollision, CollisionHit, Error, Result, SurfaceGeometry, SurfaceHit};
