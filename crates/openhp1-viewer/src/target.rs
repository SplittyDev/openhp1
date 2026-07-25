use eframe::{
    egui::TextureId,
    egui_wgpu::RenderState,
    wgpu::{self, FilterMode},
};

pub(crate) struct ColorTarget {
    _texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) id: TextureId,
    size: [u32; 2],
}

impl ColorTarget {
    pub(crate) fn new(state: &RenderState, size: [u32; 2]) -> Self {
        let (texture, view) = create_color_texture(&state.device, size);
        let id = state.renderer.write().register_native_texture(
            &state.device,
            &view,
            FilterMode::Linear,
        );
        Self {
            _texture: texture,
            view,
            id,
            size,
        }
    }

    pub(crate) fn resize(&mut self, state: &RenderState, size: [u32; 2]) {
        if self.size == size {
            return;
        }
        let (texture, view) = create_color_texture(&state.device, size);
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &view,
                FilterMode::Linear,
                self.id,
            );
        self._texture = texture;
        self.view = view;
        self.size = size;
    }
}

fn create_color_texture(
    device: &wgpu::Device,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 viewport"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}
