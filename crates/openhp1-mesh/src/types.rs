use glam::{IVec3, Vec2, Vec3};
use openhp1_package::ObjectReference;

#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Vec3,
    /// Normalized texture coordinates.
    pub texture_coordinates: Vec2,
}

#[derive(Clone, Copy, Debug)]
pub struct MeshTriangle {
    pub vertices: [MeshVertex; 3],
    pub poly_flags: u32,
    pub texture_index: i32,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub triangles: Vec<MeshTriangle>,
    pub textures: Vec<ObjectReference>,
    pub scale: Vec3,
    pub origin: Vec3,
    pub rotation_origin: IVec3,
}
