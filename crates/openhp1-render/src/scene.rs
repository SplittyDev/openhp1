use openhp1_map::TriangleMesh;

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
    Masked,
    /// Not submitted to the GPU. Fake backdrops use this until sky-zone
    /// rendering exists.
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SurfaceMaterial {
    pub texture: Option<usize>,
    pub mode: SurfaceMode,
    pub two_sided: bool,
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub mesh: TriangleMesh,
    pub textures: Vec<TextureImage>,
    /// Material for each BSP surface. Missing textures use the renderer's
    /// checkerboard.
    pub surface_materials: Vec<SurfaceMaterial>,
}
