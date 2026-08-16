//! Typed decoding and triangulation of the UE1 map structures used by HP1.

mod actor;
mod bsp;
mod decode;
mod error;
mod level;
mod lighting;
mod mesh;
mod model;
mod polys;
mod sky;

pub use actor::{Actor, ActorProperties};
pub use bsp::{
    BspNode, BspSurface, BspVertex, ConvexLeaf, LightMap, PolyFlags, PrimitiveBounds, Zone,
};
pub use error::{Error, Result};
pub use level::{Level, ReachSpec, world_model_export};
pub use lighting::{
    ActorVertexLighting, AuthoredLight, AuthoredLightmap, LightVisibility, LightmapImage,
    VertexLighting, bsp_sphere_leaves, bsp_zone_at, bsp_zone_at_checked, hsb_to_rgb,
};
pub use mesh::TriangleMesh;
pub use model::Model;
pub use polys::{BrushPolygon, BrushPolys};
pub use sky::{Rotator, SkyZone};
