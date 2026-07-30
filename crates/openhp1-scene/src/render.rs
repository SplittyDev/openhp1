use openhp1_map::{LightmapImage, SkyZone, TriangleMesh};

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMaterial {
    pub texture: Option<usize>,
    pub mode: SurfaceMode,
    /// Discard palette index zero. This remains independent of the blend mode
    /// because UE1 permits masked modulated surfaces.
    pub masked: bool,
    pub two_sided: bool,
    pub unlit: bool,
    /// Derive texture coordinates from the reflected view direction.
    pub environment_map: bool,
    /// HP1-specific multiplier for blended source color.
    pub opacity: f32,
    /// UE1 zone multipliers for automatic U/V texture-coordinate panning.
    pub texture_pan_speed: [f32; 2],
}

impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            texture: None,
            mode: SurfaceMode::Opaque,
            masked: false,
            two_sided: false,
            unlit: false,
            environment_map: false,
            opacity: 1.0,
            texture_pan_speed: [0.0; 2],
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub mesh: TriangleMesh,
    pub textures: Vec<TextureImage>,
    pub lightmaps: Vec<LightmapImage>,
    /// Material for each BSP surface. Missing textures use the renderer's
    /// checkerboard.
    pub surface_materials: Vec<SurfaceMaterial>,
    /// A fixed UE1 sky-box viewpoint rendered behind the main scene.
    pub sky_zone: Option<SkyZone>,
}
