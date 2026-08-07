use std::{borrow::Cow, mem::size_of};

use bytemuck::{Pod, Zeroable};

use crate::{AmbientOcclusion, Camera};

const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const VIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const MAX_DEPTH_MIPS: u32 = 5;

#[cfg(test)]
const FULLSCREEN_SHADER: &str = include_str!("../../shaders/modern/fullscreen.wgsl");
#[cfg(test)]
const AO_COMMON_SHADER: &str = include_str!("../../shaders/modern/ao_common.wgsl");
const DEPTH_LINEARIZE_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/ao_common.wgsl"),
    include_str!("../../shaders/modern/ao_depth_linearize.wgsl"),
);
const DEPTH_DOWNSAMPLE_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/ao_common.wgsl"),
    include_str!("../../shaders/modern/ao_depth_downsample.wgsl"),
);
const AO_MAIN_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/ao_common.wgsl"),
    include_str!("../../shaders/modern/ao_main.wgsl"),
    include_str!("../../shaders/modern/ao_ssao.wgsl"),
    include_str!("../../shaders/modern/ao_xegtao.wgsl"),
);
const AO_DENOISE_SHADER: &str = concat!(
    include_str!("../../shaders/modern/fullscreen.wgsl"),
    include_str!("../../shaders/modern/ao_common.wgsl"),
    include_str!("../../shaders/modern/ao_denoise.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AoUniform {
    viewport_size: [u32; 2],
    inverse_viewport: [f32; 2],
    near_far: [f32; 2],
    tan_half_fov: [f32; 2],
    effect_radius: f32,
    visibility_scale: f32,
    depth_mip_count: u32,
    _padding: u32,
}

struct TextureTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct DepthPyramid {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    mip_views: Vec<wgpu::TextureView>,
}

struct AoResources {
    raw: TextureTarget,
    filtered: TextureTarget,
    edges: TextureTarget,
    depth: DepthPyramid,
}

struct BindGroups {
    linearize: wgpu::BindGroup,
    downsample: Vec<wgpu::BindGroup>,
    main: wgpu::BindGroup,
    denoise_first: wgpu::BindGroup,
    denoise_final: wgpu::BindGroup,
}

pub(super) struct AoRenderer {
    uniform: wgpu::Buffer,
    linearize_layout: wgpu::BindGroupLayout,
    downsample_layout: wgpu::BindGroupLayout,
    main_layout: wgpu::BindGroupLayout,
    denoise_layout: wgpu::BindGroupLayout,
    linearize_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    ssao_pipeline: wgpu::RenderPipeline,
    xegtao_pipeline: wgpu::RenderPipeline,
    denoise_first_pipeline: wgpu::RenderPipeline,
    denoise_final_pipeline: wgpu::RenderPipeline,
    resources: AoResources,
    bind_groups: BindGroups,
    size: [u32; 2],
}

impl AoRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        scene_depth: &wgpu::TextureView,
    ) -> Self {
        let size = valid_size(size);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 ambient occlusion settings"),
            size: size_of::<AoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linearize_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 AO depth-linearize layout"),
            entries: &[depth_entry(0), uniform_entry(1)],
        });
        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 AO depth-downsample layout"),
            entries: &[float_texture_entry(0), uniform_entry(1)],
        });
        let main_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 AO main layout"),
            entries: &[depth_entry(0), float_texture_entry(1), uniform_entry(2)],
        });
        let denoise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 AO denoise layout"),
            entries: &[
                float_texture_entry(0),
                float_texture_entry(1),
                uniform_entry(2),
            ],
        });

        let linearize_shader = shader(
            device,
            "OpenHP1 AO depth linearize shader",
            DEPTH_LINEARIZE_SHADER,
        );
        let downsample_shader = shader(
            device,
            "OpenHP1 AO depth downsample shader",
            DEPTH_DOWNSAMPLE_SHADER,
        );
        let main_shader = shader(device, "OpenHP1 AO main shader", AO_MAIN_SHADER);
        let denoise_shader = shader(device, "OpenHP1 AO denoise shader", AO_DENOISE_SHADER);
        let linearize_pipeline = pipeline(
            device,
            &linearize_layout,
            &linearize_shader,
            "fragment_linearize_depth",
            "OpenHP1 AO depth linearize pipeline",
            &[VIEW_DEPTH_FORMAT],
        );
        let downsample_pipeline = pipeline(
            device,
            &downsample_layout,
            &downsample_shader,
            "fragment_downsample_depth",
            "OpenHP1 AO depth downsample pipeline",
            &[VIEW_DEPTH_FORMAT],
        );
        let ssao_pipeline = pipeline(
            device,
            &main_layout,
            &main_shader,
            "fragment_ssao",
            "OpenHP1 SSAO pipeline",
            &[AO_FORMAT, AO_FORMAT],
        );
        let xegtao_pipeline = pipeline(
            device,
            &main_layout,
            &main_shader,
            "fragment_xegtao",
            "OpenHP1 XeGTAO pipeline",
            &[AO_FORMAT, AO_FORMAT],
        );
        let denoise_first_pipeline = pipeline(
            device,
            &denoise_layout,
            &denoise_shader,
            "fragment_denoise_first",
            "OpenHP1 AO first denoise pipeline",
            &[AO_FORMAT],
        );
        let denoise_final_pipeline = pipeline(
            device,
            &denoise_layout,
            &denoise_shader,
            "fragment_denoise_final",
            "OpenHP1 AO final denoise pipeline",
            &[AO_FORMAT],
        );
        let resources = AoResources::new(device, size);
        let bind_groups = BindGroups::new(
            device,
            scene_depth,
            &uniform,
            &linearize_layout,
            &downsample_layout,
            &main_layout,
            &denoise_layout,
            &resources,
        );

        Self {
            uniform,
            linearize_layout,
            downsample_layout,
            main_layout,
            denoise_layout,
            linearize_pipeline,
            downsample_pipeline,
            ssao_pipeline,
            xegtao_pipeline,
            denoise_first_pipeline,
            denoise_final_pipeline,
            resources,
            bind_groups,
            size,
        }
    }

    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        scene_depth: &wgpu::TextureView,
    ) {
        let size = valid_size(size);
        if self.size == size {
            return;
        }
        let resources = AoResources::new(device, size);
        let bind_groups = BindGroups::new(
            device,
            scene_depth,
            &self.uniform,
            &self.linearize_layout,
            &self.downsample_layout,
            &self.main_layout,
            &self.denoise_layout,
            &resources,
        );
        self.resources = resources;
        self.bind_groups = bind_groups;
        self.size = size;
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.resources.raw.view
    }

    pub(super) fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        method: AmbientOcclusion,
    ) -> usize {
        if method == AmbientOcclusion::Off {
            return 0;
        }
        let (effect_radius, visibility_scale) = match method {
            AmbientOcclusion::Off => unreachable!(),
            AmbientOcclusion::Ssao => (96.0, 1.0),
            AmbientOcclusion::XeGtao => (66.0, 1.5),
        };
        let tan_half_fov_y = (camera.vertical_fov * 0.5).tan();
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&AoUniform {
                viewport_size: self.size,
                inverse_viewport: [1.0 / self.size[0] as f32, 1.0 / self.size[1] as f32],
                near_far: [camera.near, camera.far],
                tan_half_fov: [
                    tan_half_fov_y * self.size[0] as f32 / self.size[1] as f32,
                    tan_half_fov_y,
                ],
                effect_radius,
                visibility_scale,
                depth_mip_count: self.resources.depth.mip_views.len() as u32,
                _padding: 0,
            }),
        );

        draw(
            encoder,
            &[&self.resources.depth.mip_views[0]],
            &self.linearize_pipeline,
            &self.bind_groups.linearize,
            "OpenHP1 AO depth linearize pass",
        );
        let mut passes = 1;
        let main_pipeline = match method {
            AmbientOcclusion::Off => unreachable!(),
            AmbientOcclusion::Ssao => &self.ssao_pipeline,
            AmbientOcclusion::XeGtao => {
                for (index, bind_group) in self.bind_groups.downsample.iter().enumerate() {
                    draw(
                        encoder,
                        &[&self.resources.depth.mip_views[index + 1]],
                        &self.downsample_pipeline,
                        bind_group,
                        "OpenHP1 AO depth downsample pass",
                    );
                    passes += 1;
                }
                &self.xegtao_pipeline
            }
        };
        draw(
            encoder,
            &[&self.resources.raw.view, &self.resources.edges.view],
            main_pipeline,
            &self.bind_groups.main,
            "OpenHP1 ambient occlusion pass",
        );
        draw(
            encoder,
            &[&self.resources.filtered.view],
            &self.denoise_first_pipeline,
            &self.bind_groups.denoise_first,
            "OpenHP1 AO first denoise pass",
        );
        draw(
            encoder,
            &[&self.resources.raw.view],
            &self.denoise_final_pipeline,
            &self.bind_groups.denoise_final,
            "OpenHP1 AO final denoise pass",
        );
        passes + 3
    }
}

impl AoResources {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        Self {
            raw: TextureTarget::new(device, size, AO_FORMAT, "OpenHP1 raw AO"),
            filtered: TextureTarget::new(device, size, AO_FORMAT, "OpenHP1 filtered AO"),
            edges: TextureTarget::new(device, size, AO_FORMAT, "OpenHP1 AO edges"),
            depth: DepthPyramid::new(device, size),
        }
    }
}

impl TextureTarget {
    fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        label: &'static str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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

impl DepthPyramid {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let mip_count = depth_mip_count(size);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 XeGTAO view-depth pyramid"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VIEW_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("OpenHP1 XeGTAO view-depth pyramid"),
            mip_level_count: Some(mip_count),
            ..Default::default()
        });
        let mip_views = (0..mip_count)
            .map(|mip_level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("OpenHP1 XeGTAO view-depth mip"),
                    base_mip_level: mip_level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        Self {
            _texture: texture,
            view,
            mip_views,
        }
    }
}

impl BindGroups {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        scene_depth: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
        linearize_layout: &wgpu::BindGroupLayout,
        downsample_layout: &wgpu::BindGroupLayout,
        main_layout: &wgpu::BindGroupLayout,
        denoise_layout: &wgpu::BindGroupLayout,
        resources: &AoResources,
    ) -> Self {
        let linearize = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 AO depth-linearize bind group"),
            layout: linearize_layout,
            entries: &[texture_entry(0, scene_depth), buffer_entry(1, uniform)],
        });
        let downsample = resources
            .depth
            .mip_views
            .windows(2)
            .map(|views| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("OpenHP1 AO depth-downsample bind group"),
                    layout: downsample_layout,
                    entries: &[texture_entry(0, &views[0]), buffer_entry(1, uniform)],
                })
            })
            .collect();
        let main = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 AO main bind group"),
            layout: main_layout,
            entries: &[
                texture_entry(0, scene_depth),
                texture_entry(1, &resources.depth.view),
                buffer_entry(2, uniform),
            ],
        });
        let denoise_first = denoise_bind_group(
            device,
            denoise_layout,
            &resources.raw.view,
            &resources.edges.view,
            uniform,
            "OpenHP1 AO first denoise bind group",
        );
        let denoise_final = denoise_bind_group(
            device,
            denoise_layout,
            &resources.filtered.view,
            &resources.edges.view,
            uniform,
            "OpenHP1 AO final denoise bind group",
        );
        Self {
            linearize,
            downsample,
            main,
            denoise_first,
            denoise_final,
        }
    }
}

fn denoise_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    input: &wgpu::TextureView,
    edges: &wgpu::TextureView,
    uniform: &wgpu::Buffer,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            texture_entry(0, input),
            texture_entry(1, edges),
            buffer_entry(2, uniform),
        ],
    })
}

fn pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &'static str,
    label: &'static str,
    formats: &[wgpu::TextureFormat],
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    let targets = formats
        .iter()
        .map(|format| {
            Some(wgpu::ColorTargetState {
                format: *format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect::<Vec<_>>();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_fullscreen"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    targets: &[&wgpu::TextureView],
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    label: &'static str,
) {
    let attachments = targets
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect::<Vec<_>>();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn shader(device: &wgpu::Device, label: &'static str, source: &'static str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    })
}

fn depth_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn float_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn valid_size(size: [u32; 2]) -> [u32; 2] {
    [size[0].max(1), size[1].max(1)]
}

fn depth_mip_count(size: [u32; 2]) -> u32 {
    (u32::BITS - size[0].max(size[1]).leading_zeros()).min(MAX_DEPTH_MIPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_pyramid_has_up_to_five_levels() {
        assert_eq!(depth_mip_count([1, 1]), 1);
        assert_eq!(depth_mip_count([8, 3]), 4);
        assert_eq!(depth_mip_count([800, 600]), 5);
    }

    #[test]
    fn ao_uniform_matches_wgsl_alignment() {
        assert_eq!(size_of::<AoUniform>(), 48);
    }

    #[test]
    fn ao_shader_sources_are_complete() {
        assert!(FULLSCREEN_SHADER.contains("vertex_fullscreen"));
        assert!(AO_COMMON_SHADER.contains("struct AoSettings"));
        assert!(AO_MAIN_SHADER.contains("fragment_ssao"));
        assert!(AO_MAIN_SHADER.contains("fragment_xegtao"));
        assert!(AO_MAIN_SHADER.contains("const SLICE_COUNT = 9u;"));
        assert!(AO_MAIN_SHADER.contains("Copyright (C) 2016-2021, Intel Corporation"));
        assert!(AO_DENOISE_SHADER.contains("fragment_denoise_final"));
    }

    #[test]
    fn ao_shaders_are_valid_wgsl() {
        for shader in [
            DEPTH_LINEARIZE_SHADER,
            DEPTH_DOWNSAMPLE_SHADER,
            AO_MAIN_SHADER,
            AO_DENOISE_SHADER,
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
}
