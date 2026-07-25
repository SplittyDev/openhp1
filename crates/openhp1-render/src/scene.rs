use openhp1_map::TriangleMesh;

#[derive(Clone, Debug)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub mesh: TriangleMesh,
    pub textures: Vec<TextureImage>,
    /// Texture index for each BSP surface. Missing or unsupported materials use
    /// the renderer's checkerboard.
    pub surface_textures: Vec<Option<usize>>,
}
