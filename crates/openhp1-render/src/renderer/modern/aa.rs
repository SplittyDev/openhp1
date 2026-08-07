use wgpu::util::DeviceExt;

use crate::Antialiasing;

const EDGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BLEND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const AREA_SIZE: [u32; 2] = [160, 560];
const SEARCH_SIZE: [u32; 2] = [64, 16];
const AREA_BYTES: &[u8] = include_bytes!("../../shaders/modern/smaa_area.bin");
const SEARCH_BYTES: &[u8] = include_bytes!("../../shaders/modern/smaa_search.bin");

const FXAA_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/fxaa.wgsl"),
);
const SMAA_EDGES_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/smaa_edges.wgsl"),
);
const SMAA_BLEND_WEIGHTS_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/smaa_blend_weights.wgsl"),
);
const SMAA_NEIGHBORHOOD_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/smaa_neighborhood.wgsl"),
);

pub(super) struct AaRenderer {
    color: Target,
    pass: AaPass,
    format: wgpu::TextureFormat,
}

enum AaPass {
    Fxaa(FxaaPass),
    Smaa(Box<SmaaPass>),
}

struct FxaaPass {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

struct SmaaPass {
    edges: Target,
    blend: Target,
    point_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    edges_layout: wgpu::BindGroupLayout,
    weights_layout: wgpu::BindGroupLayout,
    neighborhood_layout: wgpu::BindGroupLayout,
    edges_bind_group: wgpu::BindGroup,
    weights_bind_group: wgpu::BindGroup,
    neighborhood_bind_group: wgpu::BindGroup,
    edges_pipeline: wgpu::RenderPipeline,
    weights_pipeline: wgpu::RenderPipeline,
    neighborhood_pipeline: wgpu::RenderPipeline,
    _area_texture: wgpu::Texture,
    area_view: wgpu::TextureView,
    _search_texture: wgpu::Texture,
    search_view: wgpu::TextureView,
}

struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl AaRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        output_format: wgpu::TextureFormat,
        method: Antialiasing,
    ) -> Self {
        let color = Target::new(device, size, output_format, "OpenHP1 AA input");
        let pass = match method {
            Antialiasing::Off => unreachable!(),
            Antialiasing::Fxaa => AaPass::Fxaa(FxaaPass::new(device, &color.view, output_format)),
            Antialiasing::Smaa => AaPass::Smaa(Box::new(SmaaPass::new(
                device,
                queue,
                size,
                &color.view,
                output_format,
            ))),
        };
        Self {
            color,
            pass,
            format: output_format,
        }
    }

    pub(super) fn input_view(&self) -> &wgpu::TextureView {
        &self.color.view
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        self.color = Target::new(device, size, self.format, "OpenHP1 AA input");
        match &mut self.pass {
            AaPass::Fxaa(pass) => pass.resize(device, &self.color.view),
            AaPass::Smaa(pass) => pass.resize(device, size, &self.color.view),
        }
    }

    pub(super) fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
    ) -> usize {
        match &self.pass {
            AaPass::Fxaa(pass) => {
                super::draw_fullscreen(
                    encoder,
                    output,
                    &pass.pipeline,
                    &pass.bind_group,
                    "OpenHP1 FXAA pass",
                );
                1
            }
            AaPass::Smaa(pass) => {
                super::draw_fullscreen(
                    encoder,
                    &pass.edges.view,
                    &pass.edges_pipeline,
                    &pass.edges_bind_group,
                    "OpenHP1 SMAA edge detection pass",
                );
                super::draw_fullscreen(
                    encoder,
                    &pass.blend.view,
                    &pass.weights_pipeline,
                    &pass.weights_bind_group,
                    "OpenHP1 SMAA blend weights pass",
                );
                super::draw_fullscreen(
                    encoder,
                    output,
                    &pass.neighborhood_pipeline,
                    &pass.neighborhood_bind_group,
                    "OpenHP1 SMAA neighborhood pass",
                );
                3
            }
        }
    }
}

impl FxaaPass {
    fn new(
        device: &wgpu::Device,
        color: &wgpu::TextureView,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let layout = texture_layout(device, "OpenHP1 FXAA layout", 1);
        let sampler = sampler(device, wgpu::FilterMode::Linear, "OpenHP1 FXAA sampler");
        let bind_group = bind_group(
            device,
            &layout,
            &[color],
            &sampler,
            "OpenHP1 FXAA bind group",
        );
        let shader = super::shader(device, "OpenHP1 FXAA shader", FXAA_SHADER);
        let pipeline = create_pipeline(
            device,
            &layout,
            &shader,
            "OpenHP1 FXAA pipeline",
            "fragment_fxaa",
            output_format,
        );
        Self {
            layout,
            sampler,
            bind_group,
            pipeline,
        }
    }

    fn resize(&mut self, device: &wgpu::Device, color: &wgpu::TextureView) {
        self.bind_group = bind_group(
            device,
            &self.layout,
            &[color],
            &self.sampler,
            "OpenHP1 FXAA bind group",
        );
    }
}

impl SmaaPass {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        color: &wgpu::TextureView,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let edges = Target::new(device, size, EDGE_FORMAT, "OpenHP1 SMAA edges");
        let blend = Target::new(device, size, BLEND_FORMAT, "OpenHP1 SMAA blend weights");
        let point_sampler = sampler(
            device,
            wgpu::FilterMode::Nearest,
            "OpenHP1 SMAA point sampler",
        );
        let linear_sampler = sampler(
            device,
            wgpu::FilterMode::Linear,
            "OpenHP1 SMAA linear sampler",
        );
        let edges_layout = texture_layout(device, "OpenHP1 SMAA edges layout", 1);
        let weights_layout = texture_layout(device, "OpenHP1 SMAA weights layout", 3);
        let neighborhood_layout = texture_layout(device, "OpenHP1 SMAA neighborhood layout", 2);
        let area_texture = lookup_texture(
            device,
            queue,
            AREA_SIZE,
            wgpu::TextureFormat::Rg8Unorm,
            AREA_BYTES,
            "OpenHP1 SMAA area lookup",
        );
        let area_view = area_texture.create_view(&Default::default());
        let search_texture = lookup_texture(
            device,
            queue,
            SEARCH_SIZE,
            wgpu::TextureFormat::R8Unorm,
            SEARCH_BYTES,
            "OpenHP1 SMAA search lookup",
        );
        let search_view = search_texture.create_view(&Default::default());
        let edges_bind_group = bind_group(
            device,
            &edges_layout,
            &[color],
            &point_sampler,
            "OpenHP1 SMAA edges bind group",
        );
        let weights_bind_group = bind_group(
            device,
            &weights_layout,
            &[&edges.view, &area_view, &search_view],
            &linear_sampler,
            "OpenHP1 SMAA weights bind group",
        );
        let neighborhood_bind_group = bind_group(
            device,
            &neighborhood_layout,
            &[color, &blend.view],
            &linear_sampler,
            "OpenHP1 SMAA neighborhood bind group",
        );
        let edges_shader = super::shader(device, "OpenHP1 SMAA edges shader", SMAA_EDGES_SHADER);
        let weights_shader = super::shader(
            device,
            "OpenHP1 SMAA weights shader",
            SMAA_BLEND_WEIGHTS_SHADER,
        );
        let neighborhood_shader = super::shader(
            device,
            "OpenHP1 SMAA neighborhood shader",
            SMAA_NEIGHBORHOOD_SHADER,
        );
        let edges_pipeline = create_pipeline(
            device,
            &edges_layout,
            &edges_shader,
            "OpenHP1 SMAA edges pipeline",
            "fragment_smaa_edges",
            EDGE_FORMAT,
        );
        let weights_pipeline = create_pipeline(
            device,
            &weights_layout,
            &weights_shader,
            "OpenHP1 SMAA weights pipeline",
            "fragment_smaa_blend_weights",
            BLEND_FORMAT,
        );
        let neighborhood_pipeline = create_pipeline(
            device,
            &neighborhood_layout,
            &neighborhood_shader,
            "OpenHP1 SMAA neighborhood pipeline",
            "fragment_smaa_neighborhood",
            output_format,
        );
        Self {
            edges,
            blend,
            point_sampler,
            linear_sampler,
            edges_layout,
            weights_layout,
            neighborhood_layout,
            edges_bind_group,
            weights_bind_group,
            neighborhood_bind_group,
            edges_pipeline,
            weights_pipeline,
            neighborhood_pipeline,
            _area_texture: area_texture,
            area_view,
            _search_texture: search_texture,
            search_view,
        }
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2], color: &wgpu::TextureView) {
        self.edges = Target::new(device, size, EDGE_FORMAT, "OpenHP1 SMAA edges");
        self.blend = Target::new(device, size, BLEND_FORMAT, "OpenHP1 SMAA blend weights");
        self.edges_bind_group = bind_group(
            device,
            &self.edges_layout,
            &[color],
            &self.point_sampler,
            "OpenHP1 SMAA edges bind group",
        );
        self.weights_bind_group = bind_group(
            device,
            &self.weights_layout,
            &[&self.edges.view, &self.area_view, &self.search_view],
            &self.linear_sampler,
            "OpenHP1 SMAA weights bind group",
        );
        self.neighborhood_bind_group = bind_group(
            device,
            &self.neighborhood_layout,
            &[color, &self.blend.view],
            &self.linear_sampler,
            "OpenHP1 SMAA neighborhood bind group",
        );
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        label: &'static str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

fn texture_layout(
    device: &wgpu::Device,
    label: &'static str,
    texture_count: u32,
) -> wgpu::BindGroupLayout {
    let mut entries = (0..texture_count)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        })
        .collect::<Vec<_>>();
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: texture_count,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    textures: &[&wgpu::TextureView],
    sampler: &wgpu::Sampler,
    label: &'static str,
) -> wgpu::BindGroup {
    let mut entries = textures
        .iter()
        .enumerate()
        .map(|(binding, texture)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: wgpu::BindingResource::TextureView(texture),
        })
        .collect::<Vec<_>>();
    entries.push(wgpu::BindGroupEntry {
        binding: textures.len() as u32,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &entries,
    })
}

fn sampler(device: &wgpu::Device, filter: wgpu::FilterMode, label: &'static str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

fn lookup_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    bytes: &[u8],
    label: &'static str,
) -> wgpu::Texture {
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytes,
    )
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    label: &'static str,
    entry: &'static str,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    super::pipeline(
        device,
        &pipeline_layout,
        shader,
        label,
        entry,
        output_format,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antialiasing_shaders_are_valid_wgsl() {
        for shader in [
            FXAA_SHADER,
            SMAA_EDGES_SHADER,
            SMAA_BLEND_WEIGHTS_SHADER,
            SMAA_NEIGHBORHOOD_SHADER,
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(shader).unwrap();
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn smaa_lookup_textures_have_reference_dimensions() {
        assert_eq!(AREA_BYTES.len(), (AREA_SIZE[0] * AREA_SIZE[1] * 2) as usize);
        assert_eq!(
            SEARCH_BYTES.len(),
            (SEARCH_SIZE[0] * SEARCH_SIZE[1]) as usize
        );
    }
}
