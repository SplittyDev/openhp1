use std::{
    collections::{HashMap, VecDeque},
    mem::size_of,
};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
use openhp1_scene::{RenderScene, SurfaceMode, TextureImage};
use wgpu::util::DeviceExt;

use crate::{Camera, VolumetricDebugView, VolumetricTuning, unreal_to_render};

use super::super::super::DEPTH_FORMAT;
use super::super::HDR_FORMAT;

const SHADOW_SIZE: u32 = 1024;
const APERTURE_MASK_SIZE: u32 = 128;
// ponytail: Fixed wall-direction budget; raise it only if authored maps need more slices.
const MAX_SHADOW_DIRECTIONS: usize = 4;
const MAX_VISIBLE_PORTALS: usize = 128;
const MAX_MOTES_PER_PORTAL: u32 = 64;
const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, -1.0, -0.35);
const SHAFT_SHADER: &str = concat!(
    include_str!("../../../shaders/modern/volumetric_noise.wgsl"),
    include_str!("../../../shaders/modern/sky_shafts.wgsl"),
);
const SHADER: &str = r#"
struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    direction_density: vec4<f32>,
    distance_intensity_pixel: vec4<f32>,
    haze: vec4<f32>,
    dust: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> settings: ShadowSettings;

@vertex
fn vertex_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return settings.light_view_projection * vec4(position, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ShadowUniform {
    pub(super) light_view_projection: [[f32; 4]; 4],
    pub(super) view_projection: [[f32; 4]; 4],
    pub(super) inverse_view_projection: [[f32; 4]; 4],
    pub(super) camera_position: [f32; 4],
    pub(super) direction_density: [f32; 4],
    pub(super) distance_intensity_pixel: [f32; 4],
    pub(super) haze: [f32; 4],
    pub(super) dust: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ShadowVertex {
    position: [f32; 3],
    transmission: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PortalTriangle {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    color: [f32; 4],
    direction: [f32; 4],
    uv_a: [f32; 4],
    uv_b: [f32; 4],
    uv_c: [f32; 4],
}

struct ShadowSlice {
    uniform: wgpu::Buffer,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    shaft_bind_group: wgpu::BindGroup,
    portal_buffer: wgpu::Buffer,
    portal_count: usize,
}

pub(super) struct DirectionalShadow {
    _texture: wgpu::Texture,
    _aperture_texture: wgpu::Texture,
    aperture_view: wgpu::TextureView,
    aperture_sampler: wgpu::Sampler,
    aperture_layers: HashMap<usize, u32>,
    slices: Vec<ShadowSlice>,
    pipeline: wgpu::RenderPipeline,
    shaft_layout: wgpu::BindGroupLayout,
    shaft_pipeline: wgpu::RenderPipeline,
    mote_pipeline: wgpu::RenderPipeline,
    shadow_sampler: wgpu::Sampler,
    portals: Vec<PortalTriangle>,
    vertex_buffer: wgpu::Buffer,
    vertex_count: usize,
    enabled: bool,
    tuning: VolumetricTuning,
}

impl DirectionalShadow {
    pub(super) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32];
        wgpu::VertexBufferLayout {
            array_stride: size_of::<ShadowVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }

    fn portal_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];
        wgpu::VertexBufferLayout {
            array_stride: size_of::<PortalTriangle>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }

    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_depth: &wgpu::TextureView,
        scene: &RenderScene,
    ) -> Self {
        let enabled = sky_exposed(scene);
        let vertices = shadow_vertices(scene);
        let (aperture_texture, aperture_view, aperture_sampler, aperture_layers) =
            aperture_masks(device, queue, scene);
        let portals = shaft_portals(scene, &aperture_layers);
        let fallback_vertex = ShadowVertex::zeroed();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 volumetric shadow vertices"),
            contents: if vertices.is_empty() {
                bytemuck::bytes_of(&fallback_vertex)
            } else {
                bytemuck::cast_slice(&vertices)
            },
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 volumetric sun shadow layout"),
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
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 volumetric sun shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: MAX_SHADOW_DIRECTIONS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 volumetric sun shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 volumetric sun shadow shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 volumetric sun shadow pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 volumetric sun shadow pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_shadow"),
                compilation_options: Default::default(),
                buffers: &[Self::vertex_layout()],
            },
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let shaft_layout = shaft_layout(device);
        let slices = (0..MAX_SHADOW_DIRECTIONS)
            .map(|layer| {
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("OpenHP1 volumetric sun shadow settings"),
                    size: size_of::<ShadowUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("OpenHP1 volumetric sun shadow map"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("OpenHP1 volumetric sun shadow bind group"),
                    layout: &layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                });
                let shaft_bind_group = shaft_bind_group(
                    device,
                    &shaft_layout,
                    scene_depth,
                    &view,
                    &shadow_sampler,
                    &uniform,
                    (&aperture_view, &aperture_sampler),
                );
                ShadowSlice {
                    uniform,
                    view,
                    bind_group,
                    shaft_bind_group,
                    portal_buffer: portal_buffer(device, &portals),
                    portal_count: 0,
                }
            })
            .collect();
        let shaft_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 volumetric sky shaft shader"),
            source: wgpu::ShaderSource::Wgsl(SHAFT_SHADER.into()),
        });
        let shaft_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OpenHP1 volumetric sky shaft pipeline layout"),
                bind_group_layouts: &[Some(&shaft_layout)],
                immediate_size: 0,
            });
        let additive_target = || {
            Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let shaft_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 volumetric sky shaft pipeline"),
            layout: Some(&shaft_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shaft_shader,
                entry_point: Some("vertex_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[Self::portal_layout()],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shaft_shader,
                entry_point: Some("fragment_sky_shafts"),
                compilation_options: Default::default(),
                targets: &[additive_target()],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mote_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 volumetric dust mote pipeline"),
            layout: Some(&shaft_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shaft_shader,
                entry_point: Some("vertex_dust_mote"),
                compilation_options: Default::default(),
                buffers: &[Self::portal_layout()],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shaft_shader,
                entry_point: Some("fragment_dust_mote"),
                compilation_options: Default::default(),
                targets: &[additive_target()],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            _texture: texture,
            _aperture_texture: aperture_texture,
            aperture_view,
            aperture_sampler,
            aperture_layers,
            slices,
            pipeline,
            shaft_layout,
            shaft_pipeline,
            mote_pipeline,
            shadow_sampler,
            portals,
            vertex_buffer,
            vertex_count: vertices.len(),
            enabled,
            tuning: VolumetricTuning::default(),
        }
    }

    pub(super) fn set_tuning(&mut self, tuning: VolumetricTuning) {
        self.tuning = tuning;
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, scene_depth: &wgpu::TextureView) {
        for slice in &mut self.slices {
            slice.shaft_bind_group = shaft_bind_group(
                device,
                &self.shaft_layout,
                scene_depth,
                &slice.view,
                &self.shadow_sampler,
                &slice.uniform,
                (&self.aperture_view, &self.aperture_sampler),
            );
        }
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let vertices = shadow_vertices(scene);
        let portals = shaft_portals(scene, &self.aperture_layers);
        if vertices.len() != self.vertex_count
            || portals.len() != self.portals.len()
            || sky_exposed(scene) != self.enabled
        {
            return false;
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
        self.portals = portals;
        true
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
        viewport_size: [u32; 2],
        elapsed_time: f32,
    ) {
        if !self.enabled {
            return;
        }
        let visible = visible_portals(&self.portals, camera, aspect);
        let groups = portal_direction_groups(&visible);
        for slice in &mut self.slices {
            slice.portal_count = 0;
        }
        for (slice, portals) in self.slices.iter_mut().zip(groups) {
            let direction = Vec3::from_slice(&portals[0].direction);
            queue.write_buffer(
                &slice.uniform,
                0,
                bytemuck::bytes_of(&shadow_uniform(
                    camera,
                    aspect,
                    direction,
                    viewport_size,
                    elapsed_time,
                    self.tuning,
                )),
            );
            queue.write_buffer(&slice.portal_buffer, 0, bytemuck::cast_slice(&portals));
            slice.portal_count = portals.len();
        }
    }

    pub(super) fn render(&self, encoder: &mut wgpu::CommandEncoder) -> usize {
        if !self.enabled || self.vertex_count == 0 {
            return 0;
        }
        let mut pass_count = 0;
        for slice in self.slices.iter().filter(|slice| slice.portal_count != 0) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 volumetric sun shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &slice.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &slice.bind_group, &[]);
            self.draw_geometry(&mut pass);
            pass_count += 1;
        }
        pass_count
    }

    pub(super) fn has_geometry(&self) -> bool {
        self.vertex_count != 0
    }

    pub(super) fn draw_geometry<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count as u32, 0..1);
    }

    pub(super) fn render_shafts(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) -> usize {
        if !self.enabled
            || self.vertex_count == 0
            || self.slices.iter().all(|slice| slice.portal_count == 0)
        {
            return 0;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 volumetric sky shaft pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shaft_pipeline);
        for slice in self.slices.iter().filter(|slice| slice.portal_count != 0) {
            pass.set_bind_group(0, &slice.shaft_bind_group, &[]);
            pass.set_vertex_buffer(0, slice.portal_buffer.slice(..));
            pass.draw(0..6, 0..slice.portal_count as u32);
        }
        if matches!(
            self.tuning.debug_view,
            VolumetricDebugView::Composite | VolumetricDebugView::Scattering
        ) {
            pass.set_pipeline(&self.mote_pipeline);
            for slice in self.slices.iter().filter(|slice| slice.portal_count != 0) {
                pass.set_bind_group(0, &slice.shaft_bind_group, &[]);
                pass.set_vertex_buffer(0, slice.portal_buffer.slice(..));
                pass.draw(
                    0..6 * self.tuning.dust_density.min(MAX_MOTES_PER_PORTAL),
                    0..slice.portal_count as u32,
                );
            }
        }
        1
    }
}

fn shaft_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("OpenHP1 volumetric sky shaft layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn shaft_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_depth: &wgpu::TextureView,
    shadow: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
    aperture: (&wgpu::TextureView, &wgpu::Sampler),
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 volumetric sky shaft bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene_depth),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shadow),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(aperture.0),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(aperture.1),
            },
        ],
    })
}

fn aperture_masks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &RenderScene,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Sampler,
    HashMap<usize, u32>,
) {
    let mut texture_indices = scene
        .surface_materials
        .iter()
        .filter(|material| material.volumetric_source && material.mode != SurfaceMode::Backdrop)
        .filter_map(|material| material.texture)
        .collect::<Vec<_>>();
    texture_indices.sort_unstable();
    texture_indices.dedup();

    let mut layers = HashMap::new();
    let mut bytes = vec![255; (APERTURE_MASK_SIZE * APERTURE_MASK_SIZE) as usize];
    for texture_index in texture_indices {
        let Some(image) = scene.textures.get(texture_index) else {
            continue;
        };
        let layer = bytes.len() as u32 / (APERTURE_MASK_SIZE * APERTURE_MASK_SIZE);
        layers.insert(texture_index, layer);
        bytes.extend(aperture_mask(image));
    }
    let layer_count = bytes.len() as u32 / (APERTURE_MASK_SIZE * APERTURE_MASK_SIZE);
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("OpenHP1 volumetric aperture masks"),
            size: wgpu::Extent3d {
                width: APERTURE_MASK_SIZE,
                height: APERTURE_MASK_SIZE,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &bytes,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("OpenHP1 volumetric aperture mask array"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("OpenHP1 volumetric aperture mask sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    (texture, view, sampler, layers)
}

fn aperture_mask(image: &TextureImage) -> Vec<u8> {
    if image.width == 0 || image.height == 0 {
        return vec![255; (APERTURE_MASK_SIZE * APERTURE_MASK_SIZE) as usize];
    }
    let luminance = (0..APERTURE_MASK_SIZE)
        .flat_map(|y| {
            (0..APERTURE_MASK_SIZE).map(move |x| {
                let source_x = x * image.width / APERTURE_MASK_SIZE;
                let source_y = y * image.height / APERTURE_MASK_SIZE;
                let offset = ((source_y * image.width + source_x) * 4) as usize;
                let red = u32::from(image.rgba[offset]);
                let green = u32::from(image.rgba[offset + 1]);
                let blue = u32::from(image.rgba[offset + 2]);
                ((red * 54 + green * 183 + blue * 19) >> 8) as u8
            })
        })
        .collect::<Vec<_>>();
    let size = APERTURE_MASK_SIZE as usize;
    let mut wall = vec![false; luminance.len()];
    let mut queue = VecDeque::new();
    // ponytail: Shipped windows have no aperture channel; use authored masks if future assets add one.
    for index in 0..luminance.len() {
        let x = index % size;
        let y = index / size;
        if (x == 0 || y == 0 || x + 1 == size || y + 1 == size) && luminance[index] >= 96 {
            wall[index] = true;
            queue.push_back(index);
        }
    }
    while let Some(index) = queue.pop_front() {
        let x = index % size;
        let y = index / size;
        for neighbor in [
            (x > 0).then(|| index - 1),
            (x + 1 < size).then(|| index + 1),
            (y > 0).then(|| index - size),
            (y + 1 < size).then(|| index + size),
        ]
        .into_iter()
        .flatten()
        {
            if !wall[neighbor] && luminance[neighbor] >= 96 {
                wall[neighbor] = true;
                queue.push_back(neighbor);
            }
        }
    }
    luminance
        .into_iter()
        .zip(wall)
        .map(|(luminance, wall)| {
            if wall {
                0
            } else {
                luminance
                    .saturating_sub(24)
                    .saturating_mul(8)
                    .min(136_u8.saturating_sub(luminance).saturating_mul(8))
            }
        })
        .collect()
}

fn portal_buffer(device: &wgpu::Device, portals: &[PortalTriangle]) -> wgpu::Buffer {
    let fallback = PortalTriangle::zeroed();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("OpenHP1 volumetric shaft apertures"),
        contents: if portals.is_empty() {
            bytemuck::bytes_of(&fallback)
        } else {
            bytemuck::cast_slice(portals)
        },
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}

fn sky_exposed(scene: &RenderScene) -> bool {
    scene
        .surface_materials
        .iter()
        .any(|material| material.volumetric_source)
}

fn shadow_vertices(scene: &RenderScene) -> Vec<ShadowVertex> {
    let mut vertices = Vec::new();
    for (triangle, surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        if !is_shadow_caster(scene, *surface) {
            continue;
        }
        let &[a, b, c] = triangle else {
            continue;
        };
        let [Some(a), Some(b), Some(c)] =
            [a, b, c].map(|index| scene.mesh.positions.get(index as usize).copied())
        else {
            continue;
        };
        let transmission = triangle_transmission(scene, triangle, *surface);
        vertices.extend([a, b, c].into_iter().map(|position| ShadowVertex {
            position: unreal_to_render(position).to_array(),
            transmission,
        }));
    }
    vertices
}

fn triangle_transmission(scene: &RenderScene, triangle: &[u32], surface: usize) -> f32 {
    let Some(image) = scene
        .surface_materials
        .get(surface)
        .and_then(|material| material.texture)
        .and_then(|texture| scene.textures.get(texture))
    else {
        return 0.0;
    };
    let mut uv = Vec2::ZERO;
    for index in triangle {
        let Some(texture_coordinates) = scene.mesh.texture_coordinates.get(*index as usize) else {
            return 0.0;
        };
        uv += *texture_coordinates;
    }
    uv /= triangle.len() as f32;
    let x = uv.x.rem_euclid(image.width as f32) as u32;
    let y = uv.y.rem_euclid(image.height as f32) as u32;
    let offset = ((y * image.width + x) * 4) as usize;
    let Some(pixel) = image.rgba.get(offset..offset + 3) else {
        return 0.0;
    };
    (f32::from(pixel[0]) * 0.2126 + f32::from(pixel[1]) * 0.7152 + f32::from(pixel[2]) * 0.0722)
        / 255.0
}

fn is_shadow_caster(scene: &RenderScene, surface: usize) -> bool {
    scene
        .surface_materials
        .get(surface)
        .is_some_and(|material| material.mode == SurfaceMode::Opaque && !material.volumetric_source)
}

fn shaft_portals(
    scene: &RenderScene,
    aperture_layers: &HashMap<usize, u32>,
) -> Vec<PortalTriangle> {
    scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
        .filter_map(|(triangle, surface)| {
            let material = scene.surface_materials.get(*surface)?;
            if !material.volumetric_source {
                return None;
            }
            let &[a_index, b_index, c_index] = triangle else {
                return None;
            };
            let a = unreal_to_render(*scene.mesh.positions.get(a_index as usize)?);
            let b = unreal_to_render(*scene.mesh.positions.get(b_index as usize)?);
            let c = unreal_to_render(*scene.mesh.positions.get(c_index as usize)?);
            let texture_size = material
                .texture
                .and_then(|index| scene.textures.get(index))
                .map_or(Vec2::ONE, |texture| {
                    Vec2::new(texture.width as f32, texture.height as f32)
                });
            let uv = |vertex: u32| {
                scene
                    .mesh
                    .texture_coordinates
                    .get(vertex as usize)
                    .copied()
                    .unwrap_or(Vec2::ZERO)
                    / texture_size
            };
            let layer = if material.mode == SurfaceMode::Backdrop {
                0
            } else {
                material
                    .texture
                    .and_then(|texture| aperture_layers.get(&texture).copied())
                    .unwrap_or(0)
            };
            Some(PortalTriangle {
                a: a.extend(1.0).to_array(),
                b: b.extend(1.0).to_array(),
                c: c.extend(1.0).to_array(),
                color: shaft_color(scene, triangle, material.texture)
                    .extend(1.0)
                    .to_array(),
                direction: portal_direction(a, b, c).extend(layer as f32).to_array(),
                uv_a: uv(a_index).extend(0.0).extend(0.0).to_array(),
                uv_b: uv(b_index).extend(0.0).extend(0.0).to_array(),
                uv_c: uv(c_index).extend(0.0).extend(0.0).to_array(),
            })
        })
        .collect()
}

fn portal_direction(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let normal = (b - a).cross(c - a).normalize_or_zero();
    let horizontal_normal = Vec3::new(normal.x, 0.0, normal.z).normalize_or_zero();
    let mut direction = SUN_DIRECTION.normalize();
    if horizontal_normal != Vec3::ZERO {
        let outward = direction.dot(horizontal_normal);
        if outward > 0.0 {
            direction -= horizontal_normal * (2.0 * outward);
        }
        direction -= horizontal_normal * (0.35 + direction.dot(horizontal_normal)).max(0.0);
    }
    direction.normalize()
}

fn visible_portals(
    portals: &[PortalTriangle],
    camera: &Camera,
    aspect: f32,
) -> Vec<PortalTriangle> {
    let view_projection = camera.view_projection(aspect);
    let mut visible = portals
        .iter()
        .copied()
        .filter(|portal| {
            camera_on_interior_side(*portal, camera.position)
                && portal_in_view(*portal, view_projection, camera.far)
        })
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        portal_distance_squared(*left, camera.position)
            .total_cmp(&portal_distance_squared(*right, camera.position))
    });
    visible.truncate(MAX_VISIBLE_PORTALS);
    visible
}

fn portal_direction_groups(portals: &[PortalTriangle]) -> Vec<Vec<PortalTriangle>> {
    let mut groups: Vec<Vec<PortalTriangle>> = Vec::new();
    for &portal in portals {
        let direction = Vec3::from_slice(&portal.direction);
        if let Some(group) = groups
            .iter_mut()
            .find(|group| Vec3::from_slice(&group[0].direction).dot(direction) > 0.999)
        {
            group.push(portal);
        } else if groups.len() < MAX_SHADOW_DIRECTIONS {
            groups.push(vec![portal]);
        }
    }
    groups
}

fn camera_on_interior_side(portal: PortalTriangle, camera_position: Vec3) -> bool {
    let a = Vec3::from_slice(&portal.a);
    let normal = (Vec3::from_slice(&portal.b) - a).cross(Vec3::from_slice(&portal.c) - a);
    normal.dot(camera_position - a) < 0.0
}

fn portal_in_view(portal: PortalTriangle, view_projection: Mat4, view_distance: f32) -> bool {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    let mut front_points = 0;
    let extrusion = Vec3::from_slice(&portal.direction) * shaft_length(view_distance);
    for point in [portal.a, portal.b, portal.c]
        .into_iter()
        .flat_map(|point| {
            [
                Vec3::from_slice(&point),
                Vec3::from_slice(&point) + extrusion,
            ]
        })
    {
        let clip = view_projection * point.extend(1.0);
        if clip.w <= 0.001 {
            continue;
        }
        let ndc = clip.truncate() / clip.w;
        minimum = minimum.min(ndc);
        maximum = maximum.max(ndc);
        front_points += 1;
    }
    front_points != 0
        && minimum.x <= 1.0
        && maximum.x >= -1.0
        && minimum.y <= 1.0
        && maximum.y >= -1.0
        && minimum.z <= 1.0
        && maximum.z >= 0.0
}

fn shaft_length(view_distance: f32) -> f32 {
    (view_distance * 0.5).min(1_500.0)
}

fn portal_distance_squared(portal: PortalTriangle, camera_position: Vec3) -> f32 {
    let center =
        (Vec3::from_slice(&portal.a) + Vec3::from_slice(&portal.b) + Vec3::from_slice(&portal.c))
            / 3.0;
    center.distance_squared(camera_position)
}

fn shaft_color(scene: &RenderScene, triangle: &[u32], texture: Option<usize>) -> Vec3 {
    if let Some(lightmap) = triangle
        .first()
        .and_then(|&vertex| scene.mesh.vertex_lightmaps.get(vertex as usize))
        .copied()
        .flatten()
        .and_then(|index| scene.lightmaps.get(index))
    {
        let coordinates = triangle.iter().fold(Vec2::ZERO, |sum, &vertex| {
            sum + scene.mesh.lightmap_coordinates[vertex as usize]
        }) / triangle.len() as f32;
        let x = coordinates
            .x
            .clamp(0.0, lightmap.width.saturating_sub(1) as f32) as usize;
        let y = coordinates
            .y
            .clamp(0.0, lightmap.height.saturating_sub(1) as f32) as usize;
        let pixel = &lightmap.rgba[(y * lightmap.width as usize + x) * 4..][..3];
        let color = Vec3::new(
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        ) / 255.0;
        if color.max_element() > 0.0 {
            return color / color.max_element() * 0.55;
        }
    }

    let Some(texture) = texture.and_then(|index| scene.textures.get(index)) else {
        return Vec3::new(1.0, 0.82, 0.62);
    };
    let pixels = texture.rgba.chunks_exact(4);
    let count = pixels.len().max(1) as f32;
    let sum = pixels.fold(Vec3::ZERO, |sum, pixel| {
        sum + Vec3::new(
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        )
    });
    (sum / (count * 255.0)).max(Vec3::splat(0.05))
}

fn shadow_uniform(
    camera: &Camera,
    aspect: f32,
    direction: Vec3,
    viewport_size: [u32; 2],
    elapsed_time: f32,
    tuning: VolumetricTuning,
) -> ShadowUniform {
    let radius = camera.far.clamp(500.0, 3_000.0);
    let direction = direction.normalize();
    let center = snap_shadow_center(
        camera.position + camera.forward() * radius * 0.35,
        direction,
        radius,
    );
    let eye = center - direction * radius * 2.0;
    let view = Mat4::look_at_rh(eye, center, Vec3::Z);
    let projection = Mat4::orthographic_rh(-radius, radius, -radius, radius, 1.0, radius * 4.0);
    ShadowUniform {
        light_view_projection: (projection * view).to_cols_array_2d(),
        view_projection: camera.view_projection(aspect).to_cols_array_2d(),
        inverse_view_projection: camera.view_projection(aspect).inverse().to_cols_array_2d(),
        camera_position: camera.position.extend(elapsed_time).to_array(),
        direction_density: [
            direction.x,
            direction.y,
            direction.z,
            0.00025 * tuning.haze_density,
        ],
        distance_intensity_pixel: [
            radius,
            0.35,
            1.0 / viewport_size[0].max(1) as f32,
            1.0 / viewport_size[1].max(1) as f32,
        ],
        haze: [
            tuning.haze_size,
            tuning.haze_density,
            tuning.haze_opacity,
            tuning.haze_speed,
        ],
        dust: [
            tuning.dust_size,
            tuning.dust_opacity,
            tuning.dust_speed,
            tuning.debug_view.shader_id() as f32,
        ],
    }
}

fn snap_shadow_center(center: Vec3, direction: Vec3, radius: f32) -> Vec3 {
    let texel = radius * 2.0 / SHADOW_SIZE as f32;
    let light_rotation = Mat4::look_to_rh(Vec3::ZERO, direction, Vec3::Z);
    let mut light_center = light_rotation.transform_point3(center);
    light_center.x = (light_center.x / texel).round() * texel;
    light_center.y = (light_center.y / texel).round() * texel;
    light_rotation.inverse().transform_point3(light_center)
}

#[cfg(test)]
mod tests {
    use glam::Vec2;
    use openhp1_scene::{LightmapImage, SurfaceMaterial, TextureImage, TriangleMesh};

    use super::*;

    fn scene(mode: SurfaceMode, sky: bool) -> RenderScene {
        RenderScene {
            mesh: TriangleMesh {
                positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                texture_coordinates: vec![Vec2::ZERO; 3],
                lightmap_coordinates: vec![Vec2::ZERO; 3],
                indices: vec![0, 1, 2],
                normals: vec![Vec3::Z; 3],
                vertex_colors: vec![Vec3::ONE; 3],
                vertex_lightmaps: vec![None; 3],
                vertex_surfaces: vec![0; 3],
                triangle_surfaces: vec![0],
            },
            textures: Vec::new(),
            lightmaps: Vec::new(),
            realtime_lightmaps: Vec::new(),
            coronas: Vec::new(),
            surface_materials: vec![SurfaceMaterial {
                mode,
                volumetric_source: mode == SurfaceMode::Backdrop,
                ..Default::default()
            }],
            sky_zone: sky.then_some(openhp1_scene::SkyZone {
                location: Vec3::ZERO,
                rotation: Default::default(),
            }),
        }
    }

    #[test]
    fn sky_shafts_require_an_authored_or_classified_aperture() {
        assert!(!sky_exposed(&scene(SurfaceMode::Opaque, true)));
        assert!(sky_exposed(&scene(SurfaceMode::Backdrop, false)));
        assert!(sky_exposed(&scene(SurfaceMode::Backdrop, true)));

        let mut window = scene(SurfaceMode::Opaque, false);
        window.surface_materials[0].volumetric_source = true;
        assert!(sky_exposed(&window));
        assert_eq!(shaft_portals(&window, &HashMap::new()).len(), 1);
        assert!(shadow_vertices(&window).is_empty());
    }

    #[test]
    fn shaft_sources_are_culled_outside_the_camera_view() {
        let portal = |offset: Vec3| PortalTriangle {
            a: (Vec3::new(-0.5, -0.5, 0.5) + offset).extend(1.0).to_array(),
            b: (Vec3::new(0.5, -0.5, 0.5) + offset).extend(1.0).to_array(),
            c: (Vec3::new(0.0, 0.5, 0.5) + offset).extend(1.0).to_array(),
            color: [1.0; 4],
            direction: [0.0; 4],
            uv_a: [0.0; 4],
            uv_b: [0.0; 4],
            uv_c: [0.0; 4],
        };
        assert!(portal_in_view(portal(Vec3::ZERO), Mat4::IDENTITY, 1.0));
        assert!(!portal_in_view(
            portal(Vec3::new(3.0, 0.0, 0.0)),
            Mat4::IDENTITY,
            1.0,
        ));
        assert!(!portal_in_view(
            portal(Vec3::new(0.0, 0.0, -2.0)),
            Mat4::IDENTITY,
            1.0,
        ));
    }

    #[test]
    fn shaft_sources_remain_visible_while_their_extruded_volume_is_in_view() {
        let portal = PortalTriangle {
            a: Vec3::new(2.0, -0.5, 0.5).extend(1.0).to_array(),
            b: Vec3::new(2.5, -0.5, 0.5).extend(1.0).to_array(),
            c: Vec3::new(2.0, 0.5, 0.5).extend(1.0).to_array(),
            color: [1.0; 4],
            direction: Vec3::NEG_X.extend(0.0).to_array(),
            uv_a: [0.0; 4],
            uv_b: [0.0; 4],
            uv_c: [0.0; 4],
        };

        assert!(portal_in_view(portal, Mat4::IDENTITY, 4.0));
    }

    #[test]
    fn window_shafts_only_render_from_the_bsp_interior_side() {
        let portal = PortalTriangle {
            a: Vec3::ZERO.extend(1.0).to_array(),
            b: Vec3::X.extend(1.0).to_array(),
            c: Vec3::Y.extend(1.0).to_array(),
            color: [1.0; 4],
            direction: [0.0; 4],
            uv_a: [0.0; 4],
            uv_b: [0.0; 4],
            uv_c: [0.0; 4],
        };
        assert!(camera_on_interior_side(portal, -Vec3::Z));
        assert!(!camera_on_interior_side(portal, Vec3::Z));
    }

    #[test]
    fn opposite_wall_shafts_remain_downward_and_point_inward() {
        let left = portal_direction(Vec3::ZERO, Vec3::Y, Vec3::Z);
        let right = portal_direction(Vec3::ZERO, Vec3::Z, Vec3::Y);

        assert!(left.y < 0.0);
        assert!(right.y < 0.0);
        assert!(left.x < 0.0);
        assert!(right.x > 0.0);
    }

    #[test]
    fn portal_shadow_groups_share_matching_directions_and_split_opposites() {
        let portal = |direction: Vec3| PortalTriangle {
            a: [0.0; 4],
            b: [0.0; 4],
            c: [0.0; 4],
            color: [1.0; 4],
            direction: direction.extend(0.0).to_array(),
            uv_a: [0.0; 4],
            uv_b: [0.0; 4],
            uv_c: [0.0; 4],
        };
        let nearly_same = (Vec3::X + Vec3::Z * 0.01).normalize();

        let groups =
            portal_direction_groups(&[portal(Vec3::X), portal(nearly_same), portal(Vec3::NEG_X)]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1].len(), 1);
    }

    #[test]
    fn shaft_color_uses_opaque_texture_rgb_when_palette_alpha_is_zero() {
        let mut scene = scene(SurfaceMode::Opaque, false);
        scene.textures.push(TextureImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 0],
        });

        let color = shaft_color(&scene, &[0, 1, 2], Some(0));

        assert!(color.x > color.y * 4.0);
        assert!(color.x > color.z * 4.0);
    }

    #[test]
    fn aperture_mask_rejects_wall_and_painted_frame() {
        let mut rgba = vec![200; 5 * 5 * 4];
        for y in 1..4 {
            for x in 1..4 {
                rgba[(y * 5 + x) * 4..(y * 5 + x + 1) * 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        rgba[(2 * 5 + 2) * 4..(2 * 5 + 3) * 4].copy_from_slice(&[40, 70, 140, 255]);
        let mask = aperture_mask(&TextureImage {
            width: 5,
            height: 5,
            rgba,
        });

        assert_eq!(
            mask.len(),
            (APERTURE_MASK_SIZE * APERTURE_MASK_SIZE) as usize
        );
        assert_eq!(mask[0], 0);
        assert_eq!(mask[64 * APERTURE_MASK_SIZE as usize + 32], 0);
        assert!(mask[64 * APERTURE_MASK_SIZE as usize + 64] > 200);
    }

    #[test]
    fn shaft_color_prefers_the_authored_window_lightmap_tint() {
        let mut scene = scene(SurfaceMode::Opaque, false);
        scene.mesh.vertex_lightmaps.fill(Some(0));
        scene.lightmaps.push(LightmapImage {
            width: 1,
            height: 1,
            rgba: vec![8, 16, 64, 255],
        });

        let color = shaft_color(&scene, &[0, 1, 2], None);

        assert!(color.z > color.x * 4.0);
        assert!(color.z > color.y * 2.0);
    }

    #[test]
    #[ignore = "requires local original game files"]
    fn tut1_windows_feed_a_finite_number_of_shaft_prisms() {
        let level =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/Maps/Lev_Tut1.unr");
        let scene = openhp1_scene::LoadedScene::load(level).unwrap();
        let aperture_layers = scene
            .render
            .surface_materials
            .iter()
            .filter(|material| material.volumetric_source)
            .filter_map(|material| material.texture)
            .enumerate()
            .map(|(layer, texture)| (texture, layer as u32 + 1))
            .collect();
        let portals = shaft_portals(&scene.render, &aperture_layers);
        assert!(!portals.is_empty());
        assert!(portals.len() < 1_024, "{} shaft triangles", portals.len());
        assert!(portals.iter().any(|portal| portal.direction[3] > 0.0));
        assert!(portals.iter().any(|portal| portal.uv_a != portal.uv_b));
    }

    #[test]
    fn only_opaque_geometry_casts_sun_shadows() {
        assert_eq!(shadow_vertices(&scene(SurfaceMode::Opaque, true)).len(), 3);
        assert!(shadow_vertices(&scene(SurfaceMode::Backdrop, true)).is_empty());
        assert!(shadow_vertices(&scene(SurfaceMode::Translucent, true)).is_empty());
    }

    #[test]
    fn textured_shadow_vertices_preserve_bright_fixture_apertures() {
        let mut scene = scene(SurfaceMode::Opaque, false);
        scene.surface_materials[0].texture = Some(0);
        scene.textures.push(TextureImage {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        });

        let vertices = shadow_vertices(&scene);
        assert!(vertices.iter().all(|vertex| vertex.transmission == 1.0));

        scene.textures[0].rgba[..3].fill(0);
        let vertices = shadow_vertices(&scene);
        assert!(vertices.iter().all(|vertex| vertex.transmission == 0.0));
    }

    #[test]
    fn fixture_transmission_does_not_leak_across_shared_vertices() {
        let mut scene = scene(SurfaceMode::Opaque, false);
        scene.mesh.positions.push(Vec3::ONE);
        scene.mesh.texture_coordinates.push(Vec2::ZERO);
        scene.mesh.lightmap_coordinates.push(Vec2::ZERO);
        scene.mesh.indices.extend([0, 2, 3]);
        scene.mesh.normals.push(Vec3::Z);
        scene.mesh.vertex_colors.push(Vec3::ONE);
        scene.mesh.vertex_lightmaps.push(None);
        scene.mesh.vertex_surfaces.push(1);
        scene.mesh.triangle_surfaces.push(1);
        scene.surface_materials[0].texture = Some(0);
        scene.surface_materials.push(SurfaceMaterial {
            mode: SurfaceMode::Opaque,
            texture: Some(1),
            ..Default::default()
        });
        scene.textures.extend([
            TextureImage {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            },
            TextureImage {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            },
        ]);

        let vertices = shadow_vertices(&scene);
        assert!(
            vertices[..3]
                .iter()
                .all(|vertex| vertex.transmission == 1.0)
        );
        assert!(
            vertices[3..]
                .iter()
                .all(|vertex| vertex.transmission == 0.0)
        );
    }

    #[test]
    #[ignore = "requires local original game files"]
    fn tut1_lantern_fixtures_contain_transmissive_panes_and_opaque_frames() {
        let level =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/Maps/Lev_Tut1.unr");
        let loaded = openhp1_scene::LoadedScene::load(level).unwrap();
        let scene = &loaded.render;
        let corona_actors = scene
            .coronas
            .iter()
            .map(|corona| corona.actor_index)
            .collect::<std::collections::HashSet<_>>();
        let mut seen = std::collections::HashSet::new();
        let sources = scene
            .realtime_lightmaps
            .iter()
            .flat_map(|lightmap| &lightmap.lights)
            .filter(|light| {
                corona_actors.contains(&light.actor_index) && seen.insert(light.actor_index)
            });
        let mut split_fixtures = 0;
        for source in sources {
            let transmissions = scene
                .mesh
                .indices
                .chunks_exact(3)
                .zip(&scene.mesh.triangle_surfaces)
                .filter(|(triangle, surface)| {
                    is_shadow_caster(scene, **surface)
                        && (triangle.iter().fold(Vec3::ZERO, |center, &vertex| {
                            center + unreal_to_render(scene.mesh.positions[vertex as usize])
                        }) / 3.0)
                            .distance(source.location)
                            <= 32.0
                })
                .map(|(triangle, surface)| triangle_transmission(scene, triangle, *surface))
                .collect::<Vec<_>>();
            if transmissions.iter().any(|value| *value >= 0.65)
                && transmissions.iter().any(|value| *value < 0.65)
            {
                split_fixtures += 1;
            }
        }
        assert!(split_fixtures >= 4, "found {split_fixtures} split fixtures");
    }

    #[test]
    fn sky_shaft_shader_and_uniform_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHAFT_SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<ShadowUniform>(), 272);
        assert_eq!(size_of::<ShadowVertex>(), 16);
        assert_eq!(size_of::<PortalTriangle>(), 128);
    }

    #[test]
    fn sun_shadow_center_is_stable_within_one_texel() {
        let direction = SUN_DIRECTION.normalize();
        let radius = 1_000.0;
        let first = snap_shadow_center(Vec3::new(20.0, 30.0, 40.0), direction, radius);
        let second = snap_shadow_center(Vec3::new(20.1, 30.1, 40.0), direction, radius);
        let rotation = Mat4::look_to_rh(Vec3::ZERO, direction, Vec3::Z);
        let first_light = rotation.transform_point3(first);
        let second_light = rotation.transform_point3(second);
        assert!(
            (first_light.truncate() - second_light.truncate())
                .abs()
                .max_element()
                < 0.0001
        );
    }
}
