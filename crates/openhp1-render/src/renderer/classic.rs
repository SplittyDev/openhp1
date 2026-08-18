use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const SHADER: &str = concat!(
    include_str!("../shaders/modern/fullscreen.wgsl"),
    include_str!("../shaders/classic_display.wgsl"),
);
const CRT_SHADER: &str = concat!(
    include_str!("../shaders/modern/fullscreen.wgsl"),
    include_str!("../shaders/classic_crt.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DisplayUniform {
    gamma: [f32; 4],
}

pub(super) struct ClassicDisplay {
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    crt: Option<CrtDisplay>,
    format: wgpu::TextureFormat,
    size: [u32; 2],
}

struct CrtTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct CrtDisplay {
    signal: CrtTarget,
    blur: CrtTarget,
    glow: CrtTarget,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    horizontal_bind_group: wgpu::BindGroup,
    vertical_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    horizontal_pipeline: wgpu::RenderPipeline,
    vertical_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    size: [u32; 2],
}

impl ClassicDisplay {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        crt_effect: bool,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 Classic display layout"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 Classic display sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 Classic display uniform"),
            contents: bytemuck::bytes_of(&DisplayUniform { gamma: [1.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 Classic display shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 Classic display pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 Classic display pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_classic_display"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let (scene_texture, scene_view) = scene_target(device, format, size);
        let bind_group = bind_group(device, &layout, &scene_view, &sampler, &uniform);
        let crt = crt_effect.then(|| CrtDisplay::new(device, format, size));
        Self {
            _scene_texture: scene_texture,
            scene_view,
            layout,
            sampler,
            bind_group,
            uniform,
            pipeline,
            crt,
            format,
            size: valid_size(size),
        }
    }

    pub(super) fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = valid_size(size);
        if self.size == size {
            return;
        }
        let (texture, view) = scene_target(device, self.format, size);
        self.bind_group = bind_group(device, &self.layout, &view, &self.sampler, &self.uniform);
        self._scene_texture = texture;
        self.scene_view = view;
        self.size = size;
        if let Some(crt) = &mut self.crt {
            crt.resize(device, size);
        }
    }

    pub(super) fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        brightness: f32,
    ) -> usize {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&DisplayUniform {
                gamma: [classic_display_gamma(brightness), 0.0, 0.0, 0.0],
            }),
        );
        if let Some(crt) = &self.crt {
            draw_fullscreen(
                encoder,
                &crt.signal.view,
                &self.pipeline,
                &self.bind_group,
                "OpenHP1 Classic display signal pass",
            );
            crt.render(encoder, output);
            4
        } else {
            draw_fullscreen(
                encoder,
                output,
                &self.pipeline,
                &self.bind_group,
                "OpenHP1 Classic display pass",
            );
            1
        }
    }
}

impl CrtDisplay {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let size = valid_size(size);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 Classic CRT layout"),
            entries: &[
                texture_layout_entry(0),
                texture_layout_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 Classic CRT sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 Classic CRT shader"),
            source: wgpu::ShaderSource::Wgsl(CRT_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 Classic CRT pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let horizontal_pipeline = crt_pipeline(
            device,
            format,
            &pipeline_layout,
            &shader,
            "fragment_crt_halation_horizontal",
            "OpenHP1 Classic CRT horizontal halation pipeline",
        );
        let vertical_pipeline = crt_pipeline(
            device,
            format,
            &pipeline_layout,
            &shader,
            "fragment_crt_halation_vertical",
            "OpenHP1 Classic CRT vertical halation pipeline",
        );
        let composite_pipeline = crt_pipeline(
            device,
            format,
            &pipeline_layout,
            &shader,
            "fragment_crt_composite",
            "OpenHP1 Classic CRT composite pipeline",
        );
        let signal = CrtTarget::new(device, format, size, "OpenHP1 Classic CRT signal");
        let blur = CrtTarget::new(device, format, size, "OpenHP1 Classic CRT horizontal blur");
        let glow = CrtTarget::new(device, format, size, "OpenHP1 Classic CRT glow");
        let horizontal_bind_group = crt_bind_group(
            device,
            &layout,
            &signal.view,
            &signal.view,
            &sampler,
            "horizontal",
        );
        let vertical_bind_group = crt_bind_group(
            device, &layout, &blur.view, &blur.view, &sampler, "vertical",
        );
        let composite_bind_group = crt_bind_group(
            device,
            &layout,
            &signal.view,
            &glow.view,
            &sampler,
            "composite",
        );
        Self {
            signal,
            blur,
            glow,
            layout,
            sampler,
            horizontal_bind_group,
            vertical_bind_group,
            composite_bind_group,
            horizontal_pipeline,
            vertical_pipeline,
            composite_pipeline,
            format,
            size,
        }
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = valid_size(size);
        if self.size == size {
            return;
        }
        let signal = CrtTarget::new(device, self.format, size, "OpenHP1 Classic CRT signal");
        let blur = CrtTarget::new(
            device,
            self.format,
            size,
            "OpenHP1 Classic CRT horizontal blur",
        );
        let glow = CrtTarget::new(device, self.format, size, "OpenHP1 Classic CRT glow");
        self.horizontal_bind_group = crt_bind_group(
            device,
            &self.layout,
            &signal.view,
            &signal.view,
            &self.sampler,
            "horizontal",
        );
        self.vertical_bind_group = crt_bind_group(
            device,
            &self.layout,
            &blur.view,
            &blur.view,
            &self.sampler,
            "vertical",
        );
        self.composite_bind_group = crt_bind_group(
            device,
            &self.layout,
            &signal.view,
            &glow.view,
            &self.sampler,
            "composite",
        );
        self.signal = signal;
        self.blur = blur;
        self.glow = glow;
        self.size = size;
    }

    fn render(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
        draw_fullscreen(
            encoder,
            &self.blur.view,
            &self.horizontal_pipeline,
            &self.horizontal_bind_group,
            "OpenHP1 Classic CRT horizontal halation pass",
        );
        draw_fullscreen(
            encoder,
            &self.glow.view,
            &self.vertical_pipeline,
            &self.vertical_bind_group,
            "OpenHP1 Classic CRT vertical halation pass",
        );
        draw_fullscreen(
            encoder,
            output,
            &self.composite_pipeline,
            &self.composite_bind_group,
            "OpenHP1 Classic CRT composite pass",
        );
    }
}

impl CrtTarget {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
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
        Self {
            view: texture.create_view(&Default::default()),
            _texture: texture,
        }
    }
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn crt_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &'static str,
    label: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn crt_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &wgpu::TextureView,
    glow: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(glow),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
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

fn classic_display_gamma(brightness: f32) -> f32 {
    1.0 / (brightness * 2.5)
}

fn valid_size(size: [u32; 2]) -> [u32; 2] {
    [size[0].max(1), size[1].max(1)]
}

fn scene_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = valid_size(size);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 Classic scene target"),
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
    (texture, view)
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 Classic display bind group"),
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
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_shipped_d3d_gamma_ramp_exponent() {
        assert!((classic_display_gamma(0.5) - 0.8).abs() < f32::EPSILON);
        assert!((classic_display_gamma(0.6) - 2.0 / 3.0).abs() < f32::EPSILON);
        assert_eq!(classic_display_gamma(0.4), 1.0);
    }

    #[test]
    fn applies_gamma_once_after_blending() {
        let gamma = classic_display_gamma(0.5);
        let source = 0.8_f32;
        let destination = 0.2_f32;
        let blend_then_gamma = (source + destination * (1.0 - source)).powf(gamma);
        let gamma_then_blend =
            source.powf(gamma) + destination.powf(gamma) * (1.0 - source.powf(gamma));
        assert!((blend_then_gamma - gamma_then_blend).abs() > 0.01);
        assert_eq!(SHADER.matches("pow(").count(), 1);
        assert!(!super::super::SCENE_SHADER.contains("apply_display_gamma"));
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn pc_crt_shader_is_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(CRT_SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
