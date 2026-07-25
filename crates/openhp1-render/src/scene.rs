use openhp1_map::{SkyZone, TriangleMesh};

#[derive(Clone, Debug)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfaceMode {
    #[default]
    Opaque,
    Translucent,
    Modulated,
    /// Samples the rendered sky zone in screen space.
    Backdrop,
    /// Not submitted to the GPU.
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurfaceMaterial {
    pub texture: Option<usize>,
    pub mode: SurfaceMode,
    /// Discard palette index zero. This remains independent of the blend mode
    /// because UE1 permits masked modulated surfaces.
    pub masked: bool,
    pub two_sided: bool,
    pub unlit: bool,
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub mesh: TriangleMesh,
    pub textures: Vec<TextureImage>,
    /// Material for each BSP surface. Missing textures use the renderer's
    /// checkerboard.
    pub surface_materials: Vec<SurfaceMaterial>,
    /// A fixed UE1 sky-box viewpoint rendered behind the main scene.
    pub sky_zone: Option<SkyZone>,
}
