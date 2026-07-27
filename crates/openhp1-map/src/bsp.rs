use glam::Vec3;
use openhp1_package::ObjectReference;

#[derive(Clone, Debug)]
pub struct PrimitiveBounds {
    pub minimum: Vec3,
    pub maximum: Vec3,
    pub valid: bool,
    pub sphere: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct BspNode {
    pub plane: [f32; 4],
    pub zone_mask: u64,
    pub flags: u8,
    pub vertex_pool: i32,
    pub surface: i32,
    pub back: i32,
    pub front: i32,
    pub coplanar: i32,
    pub collision_bound: i32,
    pub render_bound: i32,
    pub zones: [i32; 2],
    pub vertex_count: u8,
    pub leaves: [i32; 2],
}

#[derive(Clone, Debug)]
pub struct BspSurface {
    pub texture: ObjectReference,
    pub poly_flags: PolyFlags,
    pub base_point: i32,
    pub normal: i32,
    pub texture_u: i32,
    pub texture_v: i32,
    pub light_map: i32,
    pub brush_poly: i32,
    pub pan_u: i16,
    pub pan_v: i16,
    pub brush_actor: ObjectReference,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PolyFlags(u32);

impl PolyFlags {
    pub const INVISIBLE: Self = Self(0x0000_0001);
    pub const MASKED: Self = Self(0x0000_0002);
    pub const TRANSLUCENT: Self = Self(0x0000_0004);
    pub const NOT_SOLID: Self = Self(0x0000_0008);
    pub const MODULATED: Self = Self(0x0000_0040);
    pub const FAKE_BACKDROP: Self = Self(0x0000_0080);
    pub const TWO_SIDED: Self = Self(0x0000_0100);
    pub const AUTO_U_PAN: Self = Self(0x0000_0200);
    pub const AUTO_V_PAN: Self = Self(0x0000_0400);
    pub const UNLIT: Self = Self(0x0040_0000);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
}

#[derive(Clone, Debug)]
pub struct BspVertex {
    pub point: i32,
    pub side: i32,
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub actor: ObjectReference,
    pub connectivity: u64,
    pub visibility: u64,
}

#[derive(Clone, Debug)]
pub struct LightMap {
    pub data_offset: i32,
    pub pan: Vec3,
    pub clamp: [i32; 2],
    pub scale: [f32; 2],
    pub light_actors: i32,
}

#[derive(Clone, Debug)]
pub struct ConvexLeaf {
    pub zone: i32,
    pub permeating: i32,
    pub volumetric: i32,
    pub visible_zones: u64,
}
