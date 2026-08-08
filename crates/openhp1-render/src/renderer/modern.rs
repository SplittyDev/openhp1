use std::{borrow::Cow, mem::size_of, ops::Range};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::{
    AmbientOcclusion, Antialiasing, Camera, RenderScene, RendererSettings, SurfaceMode, ToneMapper,
    VolumetricTuning, unreal_to_render,
};

use super::{DEPTH_FORMAT, display_gamma, pipeline::blend_state};

mod aa;
mod ao;
mod volumetric;

use aa::AaRenderer;
use ao::AoRenderer;
use volumetric::VolumetricRenderer;

pub(super) const COMPOSITE_SHADER: &str = concat!(
    include_str!("../shaders/modern/fullscreen.wgsl"),
    include_str!("../shaders/modern/composite.wgsl"),
    include_str!("../shaders/modern/tone_mapping.wgsl"),
);
pub(super) const BLOOM_SHADER: &str = concat!(
    include_str!("../shaders/modern/fullscreen.wgsl"),
    include_str!("../shaders/modern/bloom.wgsl"),
);

pub(super) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModernUniform {
    brightness_gamma: f32,
    bloom_strength: f32,
    contrast: f32,
    tone_mapper: u32,
    ambient_occlusion: u32,
    _padding: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CoronaCameraUniform {
    view_projection: [[f32; 4]; 4],
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CoronaInstance {
    position: [f32; 3],
    color_and_scale: [f32; 4],
}

#[derive(Eq, PartialEq)]
struct CoronaBatch {
    texture: usize,
    instances: Range<u32>,
}

struct CoronaRenderer {
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    instance_count: usize,
    batches: Vec<CoronaBatch>,
}

struct BloomTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(super) struct ModernRenderer {
    coronas: Option<CoronaRenderer>,
    volumetrics: VolumetricRenderer,
    ao: AoRenderer,
    aa: Option<AaRenderer>,
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    bloom_a: BloomTarget,
    bloom_b: BloomTarget,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    composite_layout: wgpu::BindGroupLayout,
    bloom_layout: wgpu::BindGroupLayout,
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
    volumetric_lighting: bool,
}

impl ModernRenderer {
    pub(super) fn new(
        gpu: (&wgpu::Device, &wgpu::Queue),
        output_format: wgpu::TextureFormat,
        size: [u32; 2],
        settings: RendererSettings,
        depth_view: &wgpu::TextureView,
        scene: &RenderScene,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let (device, queue) = gpu;
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
        let ao = AoRenderer::new(device, size, depth_view);
        let aa = (settings.antialiasing != Antialiasing::Off)
            .then(|| AaRenderer::new(device, queue, size, output_format, settings.antialiasing));
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 modern composite layout"),
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 bloom layout"),
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
            ],
        });
        let composite_bind_group = composite_bind_group(
            device,
            &composite_layout,
            &scene_view,
            &bloom_a.view,
            ao.view(),
            &sampler,
            &uniform,
        );
        let extract_bind_group = bloom_bind_group(device, &bloom_layout, &scene_view, &sampler);
        let horizontal_bind_group =
            bloom_bind_group(device, &bloom_layout, &bloom_a.view, &sampler);
        let vertical_bind_group = bloom_bind_group(device, &bloom_layout, &bloom_b.view, &sampler);
        let composite_shader = shader(device, "OpenHP1 modern composite shader", COMPOSITE_SHADER);
        let bloom_shader = shader(device, "OpenHP1 bloom shader", BLOOM_SHADER);
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OpenHP1 modern composite pipeline layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OpenHP1 bloom pipeline layout"),
                bind_group_layouts: &[Some(&bloom_layout)],
                immediate_size: 0,
            });
        let composite_pipeline = pipeline(
            device,
            &composite_pipeline_layout,
            &composite_shader,
            "OpenHP1 modern composite pipeline",
            "fragment_composite",
            output_format,
        );
        let extract_pipeline = pipeline(
            device,
            &bloom_pipeline_layout,
            &bloom_shader,
            "OpenHP1 bloom extract pipeline",
            "fragment_bloom_extract",
            HDR_FORMAT,
        );
        let horizontal_pipeline = pipeline(
            device,
            &bloom_pipeline_layout,
            &bloom_shader,
            "OpenHP1 bloom horizontal pipeline",
            "fragment_bloom_horizontal",
            HDR_FORMAT,
        );
        let vertical_pipeline = pipeline(
            device,
            &bloom_pipeline_layout,
            &bloom_shader,
            "OpenHP1 bloom vertical pipeline",
            "fragment_bloom_vertical",
            HDR_FORMAT,
        );
        let coronas =
            (!scene.coronas.is_empty()).then(|| CoronaRenderer::new(device, scene, texture_layout));
        let volumetrics = VolumetricRenderer::new(device, depth_view, scene);

        Self {
            coronas,
            volumetrics,
            ao,
            aa,
            _scene_texture: scene_texture,
            scene_view,
            bloom_a,
            bloom_b,
            sampler,
            uniform,
            composite_layout,
            bloom_layout,
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
            volumetric_lighting: settings.volumetric_lighting,
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
        self.ao.resize(device, size, depth_view);
        self.volumetrics.resize(device, depth_view);
        if let Some(aa) = &mut self.aa {
            aa.resize(device, size);
        }
        self.composite_bind_group = composite_bind_group(
            device,
            &self.composite_layout,
            &scene_view,
            &bloom_a.view,
            self.ao.view(),
            &self.sampler,
            &self.uniform,
        );
        self.extract_bind_group =
            bloom_bind_group(device, &self.bloom_layout, &scene_view, &self.sampler);
        self.horizontal_bind_group =
            bloom_bind_group(device, &self.bloom_layout, &bloom_a.view, &self.sampler);
        self.vertical_bind_group =
            bloom_bind_group(device, &self.bloom_layout, &bloom_b.view, &self.sampler);
        self._scene_texture = scene_texture;
        self.scene_view = scene_view;
        self.bloom_a = bloom_a;
        self.bloom_b = bloom_b;
        self.size = size;
    }

    pub(super) fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    pub(super) fn set_volumetric_tuning(&mut self, tuning: VolumetricTuning) {
        self.volumetrics.set_tuning(tuning);
    }

    pub(super) fn update_scene(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let coronas_updated = match self.coronas.as_mut() {
            Some(coronas) => coronas.update(queue, scene),
            None => scene.coronas.is_empty(),
        };
        coronas_updated && self.volumetrics.update(queue, scene)
    }

    pub(super) fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        viewport_size: [u32; 2],
        elapsed_time: f32,
    ) {
        if let Some(coronas) = &self.coronas {
            coronas.prepare_frame(queue, camera, viewport_size);
        }
        self.volumetrics
            .prepare_frame(queue, camera, viewport_size, elapsed_time);
    }

    pub(super) fn draw_scene_effects<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        texture_bind_groups: &'pass [wgpu::BindGroup],
    ) -> usize {
        if self.volumetric_lighting {
            return 0;
        }
        self.coronas
            .as_ref()
            .map_or(0, |coronas| coronas.draw(pass, texture_bind_groups))
    }

    pub(super) fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        brightness: f32,
        contrast: f32,
        camera: &Camera,
    ) -> usize {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&ModernUniform {
                brightness_gamma: display_gamma(brightness),
                bloom_strength: if self.bloom { 1.5 } else { 0.0 },
                contrast,
                tone_mapper: tone_mapper_id(self.tone_mapper),
                ambient_occlusion: u32::from(self.ambient_occlusion != AmbientOcclusion::Off),
                _padding: [0; 3],
            }),
        );
        let volumetric_passes = if self.volumetric_lighting {
            self.volumetrics.render(encoder, &self.scene_view)
        } else {
            0
        };
        let ao_passes = self
            .ao
            .render(queue, encoder, camera, self.ambient_occlusion);
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
        let composite_target = self.aa.as_ref().map_or(output, AaRenderer::input_view);
        draw_fullscreen(
            encoder,
            composite_target,
            &self.composite_pipeline,
            &self.composite_bind_group,
            "OpenHP1 modern composite pass",
        );
        let aa_passes = self.aa.as_ref().map_or(0, |aa| aa.render(encoder, output));
        volumetric_passes + ao_passes + (if self.bloom { 4 } else { 1 }) + aa_passes
    }
}

impl CoronaRenderer {
    fn new(
        device: &wgpu::Device,
        scene: &RenderScene,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 modern corona camera"),
            size: size_of::<CoronaCameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 modern corona camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 modern corona camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 modern corona pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/modern/corona.wgsl"));
        let pipeline = corona_pipeline(device, &pipeline_layout, &shader);
        let (instances, batches) = corona_instances(scene);
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 modern corona instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            camera_buffer,
            camera_bind_group,
            pipeline,
            instance_buffer,
            instance_count: instances.len(),
            batches,
        }
    }

    fn update(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let (instances, batches) = corona_instances(scene);
        if instances.len() != self.instance_count || batches != self.batches {
            return false;
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        true
    }

    fn prepare_frame(&self, queue: &wgpu::Queue, camera: &Camera, viewport_size: [u32; 2]) {
        let aspect = viewport_size[0] as f32 / viewport_size[1].max(1) as f32;
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CoronaCameraUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
                viewport: [viewport_size[0] as f32, viewport_size[1] as f32, 0.0, 0.0],
            }),
        );
    }

    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        texture_bind_groups: &'pass [wgpu::BindGroup],
    ) -> usize {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        for batch in &self.batches {
            pass.set_bind_group(1, &texture_bind_groups[batch.texture], &[]);
            pass.draw(0..6, batch.instances.clone());
        }
        self.batches.len()
    }
}

fn corona_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 modern corona pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_corona"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<CoronaInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
            }],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            // ponytail: depth testing approximates UE1's center-point BSP trace;
            // add a visibility query only if partial edge occlusion becomes visible.
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_corona"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: blend_state(SurfaceMode::Translucent),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn corona_instances(scene: &RenderScene) -> (Vec<CoronaInstance>, Vec<CoronaBatch>) {
    let mut entries = scene
        .coronas
        .iter()
        .map(|corona| {
            (
                corona.texture.min(scene.textures.len()),
                CoronaInstance {
                    position: unreal_to_render(corona.location).to_array(),
                    color_and_scale: [
                        corona.color.x,
                        corona.color.y,
                        corona.color.z,
                        corona.draw_scale,
                    ],
                },
            )
        })
        .collect::<Vec<_>>();
    entries.sort_unstable_by_key(|(texture, _)| *texture);

    let mut batches: Vec<CoronaBatch> = Vec::new();
    for (index, (texture, _)) in entries.iter().enumerate() {
        if let Some(batch) = batches.last_mut()
            && batch.texture == *texture
        {
            batch.instances.end += 1;
        } else {
            batches.push(CoronaBatch {
                texture: *texture,
                instances: index as u32..index as u32 + 1,
            });
        }
    }
    (
        entries.into_iter().map(|(_, instance)| instance).collect(),
        batches,
    )
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

fn composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_view: &wgpu::TextureView,
    bloom_view: &wgpu::TextureView,
    ao_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 modern composite bind group"),
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
                resource: wgpu::BindingResource::TextureView(bloom_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(ao_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

fn bloom_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 bloom bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(texture),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn shader(device: &wgpu::Device, label: &'static str, source: &'static str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
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
    use glam::Vec3;
    use openhp1_scene::{Corona, RenderScene, TriangleMesh};

    use super::*;

    #[test]
    fn bloom_target_is_quarter_resolution_and_never_empty() {
        assert_eq!(bloom_size([800, 600]), [200, 150]);
        assert_eq!(bloom_size(valid_size([0, 3])), [1, 1]);
    }

    #[test]
    fn modern_uniform_matches_shader_layout() {
        assert_eq!(size_of::<ModernUniform>(), 32);
        assert_eq!(std::mem::offset_of!(ModernUniform, contrast), 8);
        assert_eq!(std::mem::offset_of!(ModernUniform, ambient_occlusion), 16);
        assert_eq!(size_of::<CoronaCameraUniform>(), 80);
        assert_eq!(std::mem::offset_of!(CoronaCameraUniform, viewport), 64);
    }

    #[test]
    fn modern_post_process_shaders_are_valid_wgsl() {
        for shader in [COMPOSITE_SHADER, BLOOM_SHADER] {
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
    fn batches_coronas_by_skin_texture() {
        let scene = RenderScene {
            mesh: TriangleMesh::default(),
            textures: vec![super::super::checkerboard(), super::super::checkerboard()],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![
                Corona {
                    actor_index: 1,
                    location: Vec3::new(1.0, 2.0, 3.0),
                    texture: 1,
                    draw_scale: 0.2,
                    color: Vec3::ONE,
                },
                Corona {
                    actor_index: 2,
                    location: Vec3::new(4.0, 5.0, 6.0),
                    texture: 0,
                    draw_scale: 0.3,
                    color: Vec3::ONE,
                },
                Corona {
                    actor_index: 3,
                    location: Vec3::new(7.0, 8.0, 9.0),
                    texture: 1,
                    draw_scale: 0.4,
                    color: Vec3::ONE,
                },
                Corona {
                    actor_index: 4,
                    location: Vec3::ZERO,
                    texture: 99,
                    draw_scale: 0.1,
                    color: Vec3::ONE,
                },
            ],
            surface_materials: vec![],
            sky_zone: None,
        };
        let (instances, batches) = corona_instances(&scene);
        assert_eq!(instances[0].position, [5.0, 6.0, -4.0]);
        assert_eq!(
            batches
                .iter()
                .map(|batch| (batch.texture, batch.instances.clone()))
                .collect::<Vec<_>>(),
            [(0, 0..1), (1, 1..3), (2, 3..4)]
        );
    }
}
