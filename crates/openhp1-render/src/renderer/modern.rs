use std::mem::size_of;

use bytemuck::{Pod, Zeroable};

use crate::{AmbientOcclusion, Camera, RendererSettings, ToneMapper};

use super::display_gamma;

pub(super) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModernUniform {
    inverse_viewport: [f32; 2],
    brightness_gamma: f32,
    bloom_strength: f32,
    tone_mapper: u32,
    ssao: u32,
    _padding: [u32; 2],
    projection: [f32; 4],
}

struct BloomTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(super) struct ModernPostProcess {
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    bloom_a: BloomTarget,
    bloom_b: BloomTarget,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_bind_group: wgpu::BindGroup,
    extract_bind_group: wgpu::BindGroup,
    horizontal_bind_group: wgpu::BindGroup,
    vertical_bind_group: wgpu::BindGroup,
    composite_pipeline: wgpu::RenderPipeline,
    extract_pipeline: wgpu::RenderPipeline,
    horizontal_pipeline: wgpu::RenderPipeline,
    vertical_pipeline: wgpu::RenderPipeline,
    size: [u32; 2],
    tone_mapper: ToneMapper,
    ambient_occlusion: AmbientOcclusion,
    bloom: bool,
}

impl ModernPostProcess {
    pub(super) fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        size: [u32; 2],
        settings: RendererSettings,
        depth_view: &wgpu::TextureView,
    ) -> Self {
        let size = valid_size(size);
        let (scene_texture, scene_view) = scene_target(device, size);
        let bloom_a = BloomTarget::new(device, bloom_size(size), "OpenHP1 bloom A");
        let bloom_b = BloomTarget::new(device, bloom_size(size), "OpenHP1 bloom B");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 modern post-process sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 modern post-process settings"),
            size: size_of::<ModernUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 modern post-process layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let composite_bind_group = bind_group(
            device,
            &bind_group_layout,
            &scene_view,
            &bloom_a.view,
            &sampler,
            depth_view,
            &uniform,
        );
        let extract_bind_group = bind_group(
            device,
            &bind_group_layout,
            &scene_view,
            &scene_view,
            &sampler,
            depth_view,
            &uniform,
        );
        let horizontal_bind_group = bind_group(
            device,
            &bind_group_layout,
            &bloom_a.view,
            &scene_view,
            &sampler,
            depth_view,
            &uniform,
        );
        let vertical_bind_group = bind_group(
            device,
            &bind_group_layout,
            &bloom_b.view,
            &scene_view,
            &sampler,
            depth_view,
            &uniform,
        );
        let shader = device.create_shader_module(wgpu::include_wgsl!("../modern.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 modern post-process pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let composite_pipeline = pipeline(
            device,
            &pipeline_layout,
            &shader,
            "OpenHP1 modern composite pipeline",
            "fragment_post_process",
            output_format,
        );
        let extract_pipeline = pipeline(
            device,
            &pipeline_layout,
            &shader,
            "OpenHP1 bloom extract pipeline",
            "fragment_bloom_extract",
            HDR_FORMAT,
        );
        let horizontal_pipeline = pipeline(
            device,
            &pipeline_layout,
            &shader,
            "OpenHP1 bloom horizontal pipeline",
            "fragment_bloom_horizontal",
            HDR_FORMAT,
        );
        let vertical_pipeline = pipeline(
            device,
            &pipeline_layout,
            &shader,
            "OpenHP1 bloom vertical pipeline",
            "fragment_bloom_vertical",
            HDR_FORMAT,
        );

        Self {
            _scene_texture: scene_texture,
            scene_view,
            bloom_a,
            bloom_b,
            sampler,
            uniform,
            bind_group_layout,
            composite_bind_group,
            extract_bind_group,
            horizontal_bind_group,
            vertical_bind_group,
            composite_pipeline,
            extract_pipeline,
            horizontal_pipeline,
            vertical_pipeline,
            size,
            tone_mapper: settings.tone_mapper,
            ambient_occlusion: settings.ambient_occlusion,
            bloom: settings.bloom,
        }
    }

    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        depth_view: &wgpu::TextureView,
    ) {
        let size = valid_size(size);
        if self.size == size {
            return;
        }
        let (scene_texture, scene_view) = scene_target(device, size);
        let bloom_a = BloomTarget::new(device, bloom_size(size), "OpenHP1 bloom A");
        let bloom_b = BloomTarget::new(device, bloom_size(size), "OpenHP1 bloom B");
        self.composite_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &scene_view,
            &bloom_a.view,
            &self.sampler,
            depth_view,
            &self.uniform,
        );
        self.extract_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &scene_view,
            &scene_view,
            &self.sampler,
            depth_view,
            &self.uniform,
        );
        self.horizontal_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &bloom_a.view,
            &scene_view,
            &self.sampler,
            depth_view,
            &self.uniform,
        );
        self.vertical_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &bloom_b.view,
            &scene_view,
            &self.sampler,
            depth_view,
            &self.uniform,
        );
        self._scene_texture = scene_texture;
        self.scene_view = scene_view;
        self.bloom_a = bloom_a;
        self.bloom_b = bloom_b;
        self.size = size;
    }

    pub(super) fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    pub(super) fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        brightness: f32,
        camera: &Camera,
    ) -> usize {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&ModernUniform {
                inverse_viewport: [1.0 / self.size[0] as f32, 1.0 / self.size[1] as f32],
                brightness_gamma: display_gamma(brightness),
                bloom_strength: if self.bloom { 1.5 } else { 0.0 },
                tone_mapper: tone_mapper_id(self.tone_mapper),
                ssao: u32::from(matches!(self.ambient_occlusion, AmbientOcclusion::Ssao)),
                _padding: [0; 2],
                projection: [
                    camera.near,
                    camera.far,
                    (camera.vertical_fov * 0.5).tan(),
                    self.size[0] as f32 / self.size[1] as f32,
                ],
            }),
        );
        if self.bloom {
            draw_fullscreen(
                encoder,
                &self.bloom_a.view,
                &self.extract_pipeline,
                &self.extract_bind_group,
                "OpenHP1 bloom extract pass",
            );
            draw_fullscreen(
                encoder,
                &self.bloom_b.view,
                &self.horizontal_pipeline,
                &self.horizontal_bind_group,
                "OpenHP1 bloom horizontal pass",
            );
            draw_fullscreen(
                encoder,
                &self.bloom_a.view,
                &self.vertical_pipeline,
                &self.vertical_bind_group,
                "OpenHP1 bloom vertical pass",
            );
        }
        draw_fullscreen(
            encoder,
            output,
            &self.composite_pipeline,
            &self.composite_bind_group,
            "OpenHP1 modern composite pass",
        );
        if self.bloom { 4 } else { 1 }
    }
}

fn valid_size(size: [u32; 2]) -> [u32; 2] {
    [size[0].max(1), size[1].max(1)]
}

fn bloom_size(size: [u32; 2]) -> [u32; 2] {
    [(size[0] / 4).max(1), (size[1] / 4).max(1)]
}

impl BloomTarget {
    fn new(device: &wgpu::Device, size: [u32; 2], label: &'static str) -> Self {
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
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            _texture: texture,
        }
    }
}

fn scene_target(device: &wgpu::Device, size: [u32; 2]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 modern HDR scene"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_view: &wgpu::TextureView,
    bloom_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    depth_view: &wgpu::TextureView,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 modern post-process bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(bloom_view),
            },
        ],
    })
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    label: &'static str,
    fragment_entry: &'static str,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
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
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn draw_fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    label: &'static str,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn tone_mapper_id(tone_mapper: ToneMapper) -> u32 {
    match tone_mapper {
        ToneMapper::AgX => 0,
        ToneMapper::Reinhard => 1,
        ToneMapper::Aces => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_target_is_quarter_resolution_and_never_empty() {
        assert_eq!(bloom_size([800, 600]), [200, 150]);
        assert_eq!(bloom_size(valid_size([0, 3])), [1, 1]);
    }
}
