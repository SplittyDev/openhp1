use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::{
    Camera, DisplaySettings, RenderScene, RendererMode, RendererSettings, SceneBounds,
    SurfaceMaterial, SurfaceMode, TextureImage, VolumetricTuning, WarpCoordinates,
    render_to_unreal, unreal_to_render,
};

mod atlas;
mod batch;
mod classic;
mod corona;
mod lighting;
mod modern;
mod pipeline;
mod submission;
mod target;

use crate::camera::{reflected_view, warp_view};
use atlas::{AtlasRectangle, build_lightmap_atlas, lightmap_patch};
use batch::{MaterialBinding, attachment_enabled, material_bindings, mirror_geometries};
use classic::ClassicDisplay;
use corona::CoronaRenderer;
use lighting::ModernLighting;
use modern::{HDR_FORMAT, ModernRenderer};
#[cfg(test)]
use pipeline::{blend_state, color_write_mask, depth_write_enabled, fragment_entry};
use pipeline::{
    create_attachment_pipeline, create_pipeline, create_screen_pipeline,
    material_texture_bind_group, texture, texture_bind_group, write_texture_mips,
};
use submission::{SubmissionCommand, SubmissionGeometry, SubmissionPlan};
use target::{DepthTarget, SampledTarget};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_WARP_PORTAL_DEPTH: usize = 3;
const PIPELINES_PER_MODE: usize = 8;
const PIPELINE_COUNT: usize = 40;
const AUTO_UV_PER_SECOND: f32 = 64.0;
const CHECKERBOARD_MEMORY_BYTES: usize = 2 * 2 * 4;
#[cfg(test)]
const SCENE_SHADER: &str = include_str!("shaders/scene.wgsl");

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub draw_calls: usize,
    pub texture_memory_bytes: usize,
    pub lightmap_memory_bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    texture_coordinates: [f32; 2],
    texture_pan_speeds: [f32; 4],
    lightmap_coordinates: [f32; 2],
    has_lightmap: f32,
    vertex_color: [u8; 4],
    normal: [f32; 3],
    environment_map: f32,
    lighting_coordinates: [f32; 2],
    lighting_index: u32,
    uv_effect_scale: [f32; 2],
    node_plane_normal: [f32; 3],
    macro_texture_coordinates: [f32; 2],
    detail_texture_coordinates: [f32; 2],
    attachment_flags: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    world_to_view: [[f32; 4]; 4],
    camera_position: [f32; 4],
    auto_uv: [f32; 4],
    clip_plane: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FlashUniform {
    color: [f32; 4],
}

struct FlashPass {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct Mirror {
    surface: usize,
    binding: usize,
    plane: (Vec3, Vec3),
    bounds: (Vec3, Vec3),
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    blended_index_buffer: wgpu::Buffer,
    pipeline: usize,
    no_smooth: bool,
    target: SampledTarget,
}

struct WarpPortal {
    surface: usize,
    authored_plane: [f32; 4],
    source_on_positive_side: bool,
    source: WarpCoordinates,
    destination: Option<WarpCoordinates>,
    plane: (Vec3, Vec3),
    bounds: (Vec3, Vec3),
    view: PortalView,
    nested_views: Vec<PortalView>,
    pipeline: usize,
    no_smooth: bool,
    target: SampledTarget,
}

struct PortalView {
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    blended_index_buffer: wgpu::Buffer,
}

#[derive(Clone, Copy)]
enum ScenePass {
    Main,
    Sky,
    Portal,
    Reflection,
}

pub struct Renderer {
    pipelines: [wgpu::RenderPipeline; PIPELINE_COUNT],
    reflected_pipelines: Option<[wgpu::RenderPipeline; PIPELINE_COUNT]>,
    attachment_pipelines: [wgpu::RenderPipeline; 20],
    backdrop_pipelines: [wgpu::RenderPipeline; 2],
    mirror_pipelines: [wgpu::RenderPipeline; 2],
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    sky_camera_buffer: Option<wgpu::Buffer>,
    sky_camera_bind_group: Option<wgpu::BindGroup>,
    textures: Vec<wgpu::Texture>,
    texture_bind_groups: [Vec<wgpu::BindGroup>; 2],
    material_bind_groups: Vec<wgpu::BindGroup>,
    material_bindings: Vec<MaterialBinding>,
    vertices: Vec<Vertex>,
    vertex_buffer: wgpu::Buffer,
    mirrors: Vec<Mirror>,
    warp_portals: Vec<WarpPortal>,
    nested_warp_targets: Vec<SampledTarget>,
    blended_index_buffer: wgpu::Buffer,
    sky_blended_index_buffer: Option<wgpu::Buffer>,
    submission: SubmissionGeometry,
    depth: DepthTarget,
    classic_display: Option<ClassicDisplay>,
    coronas: Option<CoronaRenderer>,
    modern: Option<ModernRenderer>,
    lighting: Option<ModernLighting>,
    sky_target: Option<SampledTarget>,
    bounds: SceneBounds,
    sky_zone: Option<openhp1_scene::SkyZone>,
    target_format: wgpu::TextureFormat,
    texture_layout: wgpu::BindGroupLayout,
    texture_samplers: [wgpu::Sampler; 2],
    sky_samplers: [wgpu::Sampler; 2],
    lightmap_texture: wgpu::Texture,
    lightmap_view: wgpu::TextureView,
    lightmap_rectangles: Vec<AtlasRectangle>,
    lightmap_sampler: wgpu::Sampler,
    auto_uv: f32,
    stats: RenderStats,
    settings: RendererSettings,
    flash: FlashPass,
}

impl FlashPass {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 viewport flash uniform"),
            size: size_of::<FlashUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 viewport flash layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 viewport flash bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 viewport flash shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/flash.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 viewport flash pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 viewport flash pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(flash_target_state(target_format))],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform,
            bind_group,
        }
    }

    fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        flash: [f32; 4],
    ) -> usize {
        let color = quantized_flash(flash);
        if color == [0.0, 0.0, 0.0, 1.0] {
            return 0;
        }
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&FlashUniform { color }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 viewport flash pass"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
        1
    }
}

impl PortalView {
    fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        blended_index_count: usize,
    ) -> Self {
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 warp-portal camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 warp-portal camera bind group"),
            layout: camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let blended_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 warp-portal blended indices"),
            size: (blended_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            camera_buffer,
            camera_bind_group,
            blended_index_buffer,
        }
    }
}

impl Renderer {
    pub fn settings(&self) -> RendererSettings {
        self.settings
    }

    pub fn set_volumetric_tuning(&mut self, tuning: VolumetricTuning) {
        if let Some(modern) = &mut self.modern {
            modern.set_volumetric_tuning(tuning);
        }
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        scene: &RenderScene,
        viewport_size: [u32; 2],
    ) -> Self {
        Self::new_with_settings(
            device,
            queue,
            target_format,
            scene,
            viewport_size,
            RendererSettings::default(),
        )
    }

    pub fn new_with_settings(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        scene: &RenderScene,
        viewport_size: [u32; 2],
        settings: RendererSettings,
    ) -> Self {
        let modern_enabled = settings.mode == RendererMode::Modern;
        let scene_format = if modern_enabled {
            HDR_FORMAT
        } else {
            target_format
        };
        let lightmap_atlas = build_lightmap_atlas(
            if modern_enabled {
                &[]
            } else {
                &scene.lightmaps
            },
            device.limits().max_texture_dimension_2d,
        );
        let fallback_texture = scene.textures.len();
        let vertices: Vec<_> = scene
            .mesh
            .positions
            .iter()
            .copied()
            .enumerate()
            .map(|(vertex_index, position)| {
                let position = unreal_to_render(position);
                let surface = scene.mesh.vertex_surfaces[vertex_index];
                let material = scene
                    .surface_materials
                    .get(surface)
                    .copied()
                    .unwrap_or_default();
                let texture = material.texture.and_then(|index| scene.textures.get(index));
                let dimensions = texture_coordinate_dimensions(texture);
                let coordinates = scene.mesh.texture_coordinates[vertex_index];
                let attachment_draw_scale = |draw_scale: f32| {
                    if draw_scale.is_finite() && draw_scale != 0.0 {
                        draw_scale
                    } else {
                        1.0
                    }
                };
                let attachment_pixel_scale = |texture: Option<usize>| {
                    texture.and_then(|index| scene.textures.get(index)).map_or(
                        [1.0; 2],
                        |texture| {
                            [
                                texture.width as f32 / texture.logical_width.max(1) as f32,
                                texture.height as f32 / texture.logical_height.max(1) as f32,
                            ]
                        },
                    )
                };
                let lightmap_index = scene
                    .surface_materials
                    .get(surface)
                    .filter(|material| !material.unlit)
                    .and_then(|_| {
                        scene
                            .mesh
                            .vertex_lightmaps
                            .get(vertex_index)
                            .copied()
                            .flatten()
                    });
                let lightmap_rectangle = lightmap_index
                    .and_then(|lightmap| lightmap_atlas.rectangles.get(lightmap))
                    .copied();
                let lightmap_coordinates =
                    lightmap_rectangle.map_or(lightmap_atlas.neutral_coordinates(), |rectangle| {
                        let coordinates = scene.mesh.lightmap_coordinates[vertex_index];
                        [
                            (rectangle.x as f32 + coordinates.x)
                                / lightmap_atlas.image.width as f32,
                            (rectangle.y as f32 + coordinates.y)
                                / lightmap_atlas.image.height as f32,
                        ]
                    });
                let lighting_coordinates = lightmap_index
                    .and_then(|index| scene.lightmaps.get(index))
                    .map_or([0.0; 2], |lightmap| {
                        let coordinates = scene.mesh.lightmap_coordinates[vertex_index];
                        [
                            coordinates.x / lightmap.width as f32,
                            coordinates.y / lightmap.height as f32,
                        ]
                    });
                let macro_coordinates = macro_attachment_coordinates(
                    coordinates.to_array(),
                    material.bsp_texture_pan,
                    attachment_draw_scale(material.macro_draw_scale),
                );
                let macro_scale = attachment_pixel_scale(material.macro_texture);
                let detail_coordinates = detail_attachment_coordinates(
                    coordinates.to_array(),
                    material.bsp_texture_pan,
                    attachment_draw_scale(material.detail_draw_scale),
                );
                let detail_scale = attachment_pixel_scale(material.detail_texture);
                Vertex {
                    position: position.to_array(),
                    texture_coordinates: [
                        coordinates.x / dimensions[0],
                        coordinates.y / dimensions[1],
                    ],
                    texture_pan_speeds: scene.mesh.texture_pan_speeds.get(vertex_index).map_or(
                        [0.0; 4],
                        |speeds| {
                            [
                                speeds[0] / dimensions[0],
                                speeds[1] / dimensions[1],
                                speeds[2] / dimensions[0],
                                speeds[3] / dimensions[1],
                            ]
                        },
                    ),
                    lightmap_coordinates,
                    has_lightmap: f32::from(lightmap_rectangle.is_some()),
                    vertex_color: pack_vertex_color(
                        scene
                            .mesh
                            .vertex_colors
                            .get(vertex_index)
                            .copied()
                            .unwrap_or(Vec3::ONE),
                        material.opacity,
                    ),
                    normal: unreal_to_render(
                        scene
                            .mesh
                            .normals
                            .get(vertex_index)
                            .copied()
                            .unwrap_or(Vec3::ZERO),
                    )
                    .normalize_or_zero()
                    .to_array(),
                    environment_map: f32::from(material.environment_map),
                    lighting_coordinates,
                    lighting_index: lightmap_index
                        .filter(|&index| index < scene.realtime_lightmaps.len())
                        .and_then(|index| u32::try_from(index).ok())
                        .unwrap_or(u32::MAX),
                    uv_effect_scale: if material.environment_map {
                        [material.texture_draw_scale, 0.0]
                    } else {
                        normalized_small_wavy_scale(material.small_wavy, dimensions)
                    },
                    node_plane_normal: unreal_to_render(
                        scene
                            .mesh
                            .node_plane_normals
                            .get(vertex_index)
                            .copied()
                            .unwrap_or(Vec3::ZERO),
                    )
                    .to_array(),
                    macro_texture_coordinates: [
                        macro_coordinates[0] * macro_scale[0],
                        macro_coordinates[1] * macro_scale[1],
                    ],
                    detail_texture_coordinates: [
                        detail_coordinates[0] * detail_scale[0],
                        detail_coordinates[1] * detail_scale[1],
                    ],
                    attachment_flags: attachment_enabled(material, settings.detail_textures)
                        .map(u32::from),
                }
            })
            .collect();
        let bounds = scene_bounds(&vertices);
        let (material_bindings, surface_bindings) =
            material_bindings(scene, fallback_texture, settings.detail_textures);
        let mirror_geometries = mirror_geometries(scene, &surface_bindings);
        let warp_portal_geometries = scene
            .warp_portals
            .iter()
            .filter_map(|portal| {
                let indices = scene
                    .mesh
                    .indices
                    .chunks_exact(3)
                    .zip(&scene.mesh.triangle_surfaces)
                    .filter(|(_, surface)| **surface == portal.surface)
                    .flat_map(|(triangle, _)| triangle.iter().copied())
                    .collect::<Vec<_>>();
                (!indices.is_empty()).then_some((*portal, indices))
            })
            .collect::<Vec<_>>();
        let submission_index_count = scene.mesh.indices.len();
        let submission = SubmissionGeometry::new(scene, surface_bindings.clone());
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 BSP vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let blended_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 blended BSP indices"),
            size: (submission_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_blended_index_buffer = scene.sky_zone.map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("OpenHP1 sky blended BSP indices"),
                size: (submission_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_camera_buffer = scene.sky_zone.map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("OpenHP1 sky camera"),
                size: size_of::<CameraUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let sky_camera_bind_group = sky_camera_buffer.as_ref().map(|buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("OpenHP1 sky camera bind group"),
                layout: &camera_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 texture layout"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture_samplers = [false, true].map(|no_smooth| {
            texture_sampler(
                device,
                "OpenHP1 texture sampler",
                wgpu::AddressMode::Repeat,
                no_smooth,
                modern_enabled,
            )
        });
        let sky_samplers = [false, true].map(|no_smooth| {
            texture_sampler(
                device,
                "OpenHP1 sky sampler",
                wgpu::AddressMode::ClampToEdge,
                no_smooth,
                modern_enabled,
            )
        });
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 lightmap sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lightmap_texture = texture(
            device,
            queue,
            "OpenHP1 lightmap atlas",
            &lightmap_atlas.image,
        );
        let lightmap_view = lightmap_texture.create_view(&Default::default());
        let checkerboard = checkerboard();
        let mut stats = RenderStats {
            draw_calls: 0,
            texture_memory_bytes: scene
                .textures
                .iter()
                .map(TextureImage::byte_len)
                .sum::<usize>()
                + CHECKERBOARD_MEMORY_BYTES,
            lightmap_memory_bytes: lightmap_atlas.image.rgba.len(),
        };
        let textures = scene
            .textures
            .iter()
            .chain(std::iter::once(&checkerboard))
            .map(|image| texture(device, queue, "OpenHP1 texture", image))
            .collect::<Vec<_>>();
        let texture_bind_groups = std::array::from_fn(|filter| {
            textures
                .iter()
                .map(|texture| {
                    let view = texture.create_view(&Default::default());
                    texture_bind_group(
                        device,
                        &texture_layout,
                        &texture_samplers[filter],
                        &view,
                        &lightmap_view,
                        &lightmap_sampler,
                    )
                })
                .collect()
        });
        let material_bind_groups = build_material_bind_groups(
            device,
            &texture_layout,
            &texture_samplers,
            &textures,
            fallback_texture,
            &lightmap_view,
            &lightmap_sampler,
            &material_bindings,
        );
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/scene.wgsl"));
        let lighting_layout = modern_enabled.then(|| ModernLighting::layout(device));
        let mut bind_group_layouts = vec![Some(&camera_layout), Some(&texture_layout)];
        if let Some(layout) = &lighting_layout {
            bind_group_layouts.push(Some(layout));
        }
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 BSP pipeline layout"),
            bind_group_layouts: &bind_group_layouts,
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|index| {
            let mode = match index / PIPELINES_PER_MODE {
                0 => SurfaceMode::Opaque,
                1 => SurfaceMode::Translucent,
                2 => SurfaceMode::Modulated,
                3 => SurfaceMode::AlphaBlended,
                4 => SurfaceMode::DepthOnly,
                _ => unreachable!(),
            };
            create_pipeline(
                device,
                scene_format,
                &pipeline_layout,
                &shader,
                SurfaceMaterial {
                    mode,
                    masked: index % 4 >= 2,
                    two_sided: index % 2 != 0,
                    unlit: index % PIPELINES_PER_MODE >= 4,
                    ..Default::default()
                },
                modern_enabled,
                false,
            )
        });
        let attachment_entries = [
            if modern_enabled {
                "fragment_modern_macro"
            } else {
                "fragment_macro"
            },
            if modern_enabled {
                "fragment_modern_attachment_light"
            } else {
                "fragment_attachment_light"
            },
            "fragment_detail_0",
            "fragment_detail_1",
            "fragment_detail_2",
        ];
        let attachment_pipelines = std::array::from_fn(|index| {
            create_attachment_pipeline(
                device,
                scene_format,
                &pipeline_layout,
                &shader,
                attachment_entries[index % 5],
                index / 5 % 2 != 0,
                index >= 10,
                modern_enabled,
            )
        });
        let reflected_pipelines = (!mirror_geometries.is_empty()).then(|| {
            std::array::from_fn(|index| {
                let mode = match index / PIPELINES_PER_MODE {
                    0 => SurfaceMode::Opaque,
                    1 => SurfaceMode::Translucent,
                    2 => SurfaceMode::Modulated,
                    3 => SurfaceMode::AlphaBlended,
                    4 => SurfaceMode::DepthOnly,
                    _ => unreachable!(),
                };
                create_pipeline(
                    device,
                    scene_format,
                    &pipeline_layout,
                    &shader,
                    SurfaceMaterial {
                        mode,
                        masked: index % 4 >= 2,
                        two_sided: index % 2 != 0,
                        unlit: index % PIPELINES_PER_MODE >= 4,
                        ..Default::default()
                    },
                    modern_enabled,
                    true,
                )
            })
        });
        let backdrop_pipelines = std::array::from_fn(|index| {
            create_screen_pipeline(
                device,
                scene_format,
                &pipeline_layout,
                &shader,
                index != 0,
                if modern_enabled {
                    "fragment_backdrop_modern"
                } else {
                    "fragment_backdrop"
                },
                modern_enabled,
            )
        });
        let mirror_pipelines = std::array::from_fn(|index| {
            create_screen_pipeline(
                device,
                scene_format,
                &pipeline_layout,
                &shader,
                index != 0,
                if modern_enabled {
                    "fragment_modern_mirror"
                } else {
                    "fragment_mirror"
                },
                modern_enabled,
            )
        });
        let sky_target = scene.sky_zone.map(|_| {
            SampledTarget::new(
                device,
                viewport_size,
                scene_format,
                &texture_layout,
                [&sky_samplers[0], &sky_samplers[1]],
                &lightmap_view,
                &lightmap_sampler,
            )
        });
        let mirrors = mirror_geometries
            .into_iter()
            .filter_map(|geometry| {
                let plane = mirror_plane(scene, &vertices, geometry.surface)?;
                let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("OpenHP1 mirror camera"),
                    size: size_of::<CameraUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("OpenHP1 mirror camera bind group"),
                    layout: &camera_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: camera_buffer.as_entire_binding(),
                    }],
                });
                let blended_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("OpenHP1 mirror blended indices"),
                    size: (scene.mesh.indices.len() * size_of::<u32>()).max(size_of::<u32>())
                        as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                Some(Mirror {
                    surface: geometry.surface,
                    binding: geometry.binding,
                    plane,
                    bounds: surface_bounds(scene, &vertices, geometry.surface)?,
                    camera_buffer,
                    camera_bind_group,
                    blended_index_buffer,
                    pipeline: geometry.pipeline,
                    no_smooth: geometry.no_smooth,
                    target: SampledTarget::new(
                        device,
                        viewport_size,
                        scene_format,
                        &texture_layout,
                        [&sky_samplers[0], &sky_samplers[1]],
                        &lightmap_view,
                        &lightmap_sampler,
                    ),
                })
            })
            .collect();
        let warp_portals = warp_portal_geometries
            .into_iter()
            .filter_map(|(portal, _)| {
                let plane = mirror_plane(scene, &vertices, portal.surface)?;
                Some(WarpPortal {
                    surface: portal.surface,
                    authored_plane: portal.plane,
                    source_on_positive_side: portal.source_on_positive_side,
                    source: portal.source,
                    destination: portal.destination,
                    plane,
                    bounds: surface_bounds(scene, &vertices, portal.surface)?,
                    view: PortalView::new(device, &camera_layout, submission_index_count),
                    nested_views: (1..MAX_WARP_PORTAL_DEPTH)
                        .map(|_| PortalView::new(device, &camera_layout, submission_index_count))
                        .collect(),
                    pipeline: usize::from(scene.surface_materials[portal.surface].two_sided),
                    no_smooth: scene.surface_materials[portal.surface].no_smooth,
                    target: SampledTarget::new(
                        device,
                        viewport_size,
                        scene_format,
                        &texture_layout,
                        [&sky_samplers[0], &sky_samplers[1]],
                        &lightmap_view,
                        &lightmap_sampler,
                    ),
                })
            })
            .collect::<Vec<_>>();
        let nested_warp_targets = (!warp_portals.is_empty())
            .then(|| {
                (1..MAX_WARP_PORTAL_DEPTH)
                    .map(|_| {
                        SampledTarget::new(
                            device,
                            viewport_size,
                            scene_format,
                            &texture_layout,
                            [&sky_samplers[0], &sky_samplers[1]],
                            &lightmap_view,
                            &lightmap_sampler,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let depth = DepthTarget::new(device, viewport_size, modern_enabled);
        let classic_display = (!modern_enabled).then(|| {
            ClassicDisplay::new(device, target_format, viewport_size, settings.crt_effect)
        });
        let coronas = (!scene.coronas.is_empty()).then(|| {
            CoronaRenderer::new(device, scene_format, modern_enabled, scene, &texture_layout)
        });
        let modern = modern_enabled.then(|| {
            ModernRenderer::new(
                (device, queue),
                target_format,
                viewport_size,
                settings,
                &depth.view,
                scene,
            )
        });
        let lighting = lighting_layout
            .as_ref()
            .map(|layout| ModernLighting::new(device, queue, layout, scene));
        if let Some(lighting) = &lighting {
            stats.lightmap_memory_bytes += lighting.memory_bytes();
        }
        let flash = FlashPass::new(device, target_format);

        Self {
            pipelines,
            reflected_pipelines,
            attachment_pipelines,
            backdrop_pipelines,
            mirror_pipelines,
            camera_buffer,
            camera_bind_group,
            sky_camera_buffer,
            sky_camera_bind_group,
            textures,
            texture_bind_groups,
            material_bind_groups,
            material_bindings,
            vertices,
            vertex_buffer,
            mirrors,
            warp_portals,
            nested_warp_targets,
            blended_index_buffer,
            sky_blended_index_buffer,
            submission,
            depth,
            classic_display,
            coronas,
            modern,
            lighting,
            sky_target,
            bounds,
            sky_zone: scene.sky_zone,
            target_format,
            texture_layout,
            texture_samplers,
            sky_samplers,
            lightmap_texture,
            lightmap_view,
            lightmap_rectangles: lightmap_atlas.rectangles,
            lightmap_sampler,
            auto_uv: 0.0,
            stats,
            settings,
            flash,
        }
    }

    pub fn bounds(&self) -> SceneBounds {
        self.bounds
    }

    pub fn update_vertices(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let mesh = &scene.mesh;
        if mesh.positions.len() != self.vertices.len() {
            return false;
        }
        for (index, vertex) in self.vertices.iter_mut().enumerate() {
            let material = mesh
                .vertex_surfaces
                .get(index)
                .and_then(|surface| scene.surface_materials.get(*surface))
                .copied()
                .unwrap_or_default();
            vertex.position = unreal_to_render(mesh.positions[index]).to_array();
            vertex.vertex_color = pack_vertex_color(
                mesh.vertex_colors.get(index).copied().unwrap_or(Vec3::ONE),
                material.opacity,
            );
            vertex.normal =
                unreal_to_render(mesh.normals.get(index).copied().unwrap_or(Vec3::ZERO))
                    .normalize_or_zero()
                    .to_array();
            vertex.environment_map = f32::from(material.environment_map);
        }
        for mirror in &mut self.mirrors {
            let Some(plane) = mirror_plane(scene, &self.vertices, mirror.surface) else {
                return false;
            };
            mirror.plane = plane;
            let Some(bounds) = surface_bounds(scene, &self.vertices, mirror.surface) else {
                return false;
            };
            mirror.bounds = bounds;
        }
        for portal in &mut self.warp_portals {
            let Some(plane) = mirror_plane(scene, &self.vertices, portal.surface) else {
                return false;
            };
            portal.plane = plane;
            let Some(bounds) = surface_bounds(scene, &self.vertices, portal.surface) else {
                return false;
            };
            portal.bounds = bounds;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        true
    }

    pub fn update_scene(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        if scene.textures.len() + 1 != self.textures.len()
            || self.coronas.is_some() != !scene.coronas.is_empty()
            || !self.update_vertices(queue, scene)
        {
            return false;
        }
        let (bindings, surface_bindings) = material_bindings(
            scene,
            self.textures.len() - 1,
            self.settings.detail_textures,
        );
        if bindings != self.material_bindings || !self.submission.refresh(scene, surface_bindings) {
            return false;
        }
        for portal in &mut self.warp_portals {
            let Some(scene_portal) = scene
                .warp_portals
                .iter()
                .find(|candidate| candidate.surface == portal.surface)
            else {
                return false;
            };
            portal.authored_plane = scene_portal.plane;
            portal.source_on_positive_side = scene_portal.source_on_positive_side;
            portal.source = scene_portal.source;
            portal.destination = scene_portal.destination;
        }
        if let Some(coronas) = &mut self.coronas {
            coronas.update_scene(scene);
        }
        self.lighting
            .as_ref()
            .is_none_or(|lighting| lighting.update(queue, scene))
            && self
                .modern
                .as_mut()
                .is_none_or(|modern| modern.update_scene(queue, scene))
    }

    pub fn reload_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &RenderScene,
    ) {
        let auto_uv = self.auto_uv;
        let mut renderer = Self::new_with_settings(
            device,
            queue,
            self.target_format,
            scene,
            self.depth.size,
            self.settings,
        );
        renderer.auto_uv = auto_uv;
        if let (Some(replacement), Some(previous)) =
            (renderer.coronas.as_mut(), self.coronas.as_mut())
        {
            replacement.inherit_history(previous);
        }
        *self = renderer;
    }

    pub fn update_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[TextureImage],
        changed: &[usize],
    ) -> bool {
        if images.len() + 1 != self.textures.len() {
            return false;
        }
        let mut recreated = false;
        for &index in changed {
            let Some(image) = images.get(index) else {
                return false;
            };
            let Some(current) = self.textures.get(index) else {
                return false;
            };
            if texture_needs_recreation(
                current.width(),
                current.height(),
                current.mip_level_count(),
                image,
            ) {
                let replacement = texture(device, queue, "OpenHP1 texture", image);
                let view = replacement.create_view(&Default::default());
                for filter in 0..2 {
                    self.texture_bind_groups[filter][index] = texture_bind_group(
                        device,
                        &self.texture_layout,
                        &self.texture_samplers[filter],
                        &view,
                        &self.lightmap_view,
                        &self.lightmap_sampler,
                    );
                }
                self.textures[index] = replacement;
                recreated = true;
            } else if !write_texture_mips(queue, current, image) {
                return false;
            }
        }
        if recreated {
            self.material_bind_groups = build_material_bind_groups(
                device,
                &self.texture_layout,
                &self.texture_samplers,
                &self.textures,
                self.textures.len() - 1,
                &self.lightmap_view,
                &self.lightmap_sampler,
                &self.material_bindings,
            );
        }
        self.stats.texture_memory_bytes =
            images.iter().map(TextureImage::byte_len).sum::<usize>() + CHECKERBOARD_MEMORY_BYTES;
        self.modern
            .as_mut()
            .is_none_or(|modern| modern.update_textures(images, changed))
    }

    pub fn update_lightmaps(
        &self,
        queue: &wgpu::Queue,
        images: &[openhp1_scene::LightmapImage],
        changed: &[usize],
    ) -> bool {
        if self.lighting.is_some() {
            return true;
        }
        if images.len() != self.lightmap_rectangles.len() {
            return false;
        }
        for &index in changed {
            let (Some(image), Some(rectangle)) =
                (images.get(index), self.lightmap_rectangles.get(index))
            else {
                return false;
            };
            if image.width != rectangle.width || image.height != rectangle.height {
                return false;
            }
            let Some(patch) = lightmap_patch(image) else {
                return false;
            };
            let mut destination = self.lightmap_texture.as_image_copy();
            destination.origin = wgpu::Origin3d {
                x: rectangle.x - 1,
                y: rectangle.y - 1,
                z: 0,
            };
            queue.write_texture(
                destination,
                &patch.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(patch.width * 4),
                    rows_per_image: Some(patch.height),
                },
                wgpu::Extent3d {
                    width: patch.width,
                    height: patch.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        true
    }

    pub fn resize(&mut self, device: &wgpu::Device, viewport_size: [u32; 2]) {
        if self.depth.size != viewport_size {
            self.depth = DepthTarget::new(device, viewport_size, self.modern.is_some());
            if let Some(modern) = self.modern.as_mut() {
                modern.resize(device, viewport_size, &self.depth.view);
            }
            if let Some(classic_display) = self.classic_display.as_mut() {
                classic_display.resize(device, viewport_size);
            }
            if self.sky_target.is_some() {
                self.sky_target = Some(SampledTarget::new(
                    device,
                    viewport_size,
                    if self.modern.is_some() {
                        HDR_FORMAT
                    } else {
                        self.target_format
                    },
                    &self.texture_layout,
                    [&self.sky_samplers[0], &self.sky_samplers[1]],
                    &self.lightmap_view,
                    &self.lightmap_sampler,
                ));
            }
            for mirror in &mut self.mirrors {
                mirror.target = SampledTarget::new(
                    device,
                    viewport_size,
                    if self.modern.is_some() {
                        HDR_FORMAT
                    } else {
                        self.target_format
                    },
                    &self.texture_layout,
                    [&self.sky_samplers[0], &self.sky_samplers[1]],
                    &self.lightmap_view,
                    &self.lightmap_sampler,
                );
            }
            for portal in &mut self.warp_portals {
                portal.target = SampledTarget::new(
                    device,
                    viewport_size,
                    if self.modern.is_some() {
                        HDR_FORMAT
                    } else {
                        self.target_format
                    },
                    &self.texture_layout,
                    [&self.sky_samplers[0], &self.sky_samplers[1]],
                    &self.lightmap_view,
                    &self.lightmap_sampler,
                );
            }
            for target in &mut self.nested_warp_targets {
                *target = SampledTarget::new(
                    device,
                    viewport_size,
                    if self.modern.is_some() {
                        HDR_FORMAT
                    } else {
                        self.target_format
                    },
                    &self.texture_layout,
                    [&self.sky_samplers[0], &self.sky_samplers[1]],
                    &self.lightmap_view,
                    &self.lightmap_sampler,
                );
            }
        }
    }

    pub fn advance_time(&mut self, delta_time: f32) {
        self.auto_uv += delta_time * AUTO_UV_PER_SECOND;
    }

    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        camera: &Camera,
        viewport_actor_location: Vec3,
        viewport_size: [u32; 2],
        display: DisplaySettings,
        flash: [f32; 4],
    ) -> RenderStats {
        let mut draw_calls = 0;
        let aspect = viewport_size[0] as f32 / viewport_size[1] as f32;
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
                world_to_view: camera.view().to_cols_array_2d(),
                camera_position: camera.position.extend(1.0).to_array(),
                auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                clip_plane: Vec4::ZERO.to_array(),
            }),
        );
        if let Some(modern) = &mut self.modern {
            modern.prepare_frame(
                queue,
                camera,
                viewport_size,
                self.auto_uv / AUTO_UV_PER_SECOND,
            );
        }
        if let Some(coronas) = &mut self.coronas {
            coronas.prepare_frame(
                queue,
                camera,
                viewport_actor_location,
                viewport_size,
                self.auto_uv / AUTO_UV_PER_SECOND,
            );
        }
        let main_plan = self
            .submission
            .plan(camera.position, &self.material_bindings);
        if !main_plan.indices.is_empty() {
            queue.write_buffer(
                &self.blended_index_buffer,
                0,
                bytemuck::cast_slice(&main_plan.indices),
            );
        }

        if let (
            Some(sky_zone),
            Some(sky_camera_buffer),
            Some(sky_camera_bind_group),
            Some(sky_blended_index_buffer),
            Some(sky_target),
        ) = (
            self.sky_zone,
            self.sky_camera_buffer.as_ref(),
            self.sky_camera_bind_group.as_ref(),
            self.sky_blended_index_buffer.as_ref(),
            self.sky_target.as_ref(),
        ) {
            let sky_camera = camera.for_sky_zone(sky_zone);
            queue.write_buffer(
                sky_camera_buffer,
                0,
                bytemuck::bytes_of(&CameraUniform {
                    view_projection: sky_camera.view_projection(aspect).to_cols_array_2d(),
                    world_to_view: sky_camera.view().to_cols_array_2d(),
                    camera_position: sky_camera.position.extend(1.0).to_array(),
                    auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                    clip_plane: Vec4::ZERO.to_array(),
                }),
            );
            let sky_plan = self
                .submission
                .plan(sky_camera.position, &self.material_bindings);
            if !sky_plan.indices.is_empty() {
                queue.write_buffer(
                    sky_blended_index_buffer,
                    0,
                    bytemuck::cast_slice(&sky_plan.indices),
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 sky-zone render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sky_target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &sky_target.depth.view,
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
            draw_calls += self.draw_scene(
                &mut pass,
                sky_camera_bind_group,
                sky_blended_index_buffer,
                &sky_plan,
                None,
                None,
                None,
                ScenePass::Sky,
            );
        }

        for root in 0..self.warp_portals.len() {
            let portal = &self.warp_portals[root];
            let Some(destination) = portal.destination else {
                continue;
            };
            let (position, forward, up, world_to_view) = warp_view(
                camera.position,
                camera.forward(),
                camera.up(),
                camera.view(),
                portal.source,
                destination,
            );
            let (source_point, source_normal) = portal.plane;
            let destination_point = unreal_to_render(
                portal
                    .source
                    .transform_to(destination, render_to_unreal(source_point)),
            );
            let destination_normal = unreal_to_render(
                portal
                    .source
                    .transform_vector_to(destination, render_to_unreal(source_normal)),
            );
            let clip_plane = -mirror_clip_plane(position, destination_point, destination_normal);

            // ponytail: use the portal view's center ray until the renderer owns UE1's
            // scanline portal spans; replace this selector when partially visible nested
            // portals must branch within one target.
            let mirror = self
                .mirrors
                .iter()
                .filter_map(|mirror| {
                    ray_surface_distance(position, forward, mirror.plane, mirror.bounds)
                        .map(|distance| (mirror, distance))
                })
                .min_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(mirror, _)| mirror);

            if let Some(mirror) = mirror {
                let (reflected_position, reflected_forward, reflected_up, reflected_view) =
                    reflected_view(
                        position,
                        forward,
                        up,
                        world_to_view,
                        mirror.plane.0,
                        mirror.plane.1,
                    );
                let nested = self
                    .warp_portals
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.destination.is_some()
                            && warp_portal_active(candidate, reflected_position)
                    })
                    .filter_map(|(index, candidate)| {
                        ray_surface_distance(
                            reflected_position,
                            reflected_forward,
                            candidate.plane,
                            candidate.bounds,
                        )
                        .map(|distance| (index, distance))
                    })
                    .min_by(|(_, left), (_, right)| left.total_cmp(right))
                    .map(|(index, _)| index);

                if let Some(nested) = nested {
                    let nested_portal = &self.warp_portals[nested];
                    let nested_destination = nested_portal
                        .destination
                        .expect("nested warp portal was filtered by destination");
                    let (nested_position, _, _, nested_world_to_view) = warp_view(
                        reflected_position,
                        reflected_forward,
                        reflected_up,
                        reflected_view,
                        nested_portal.source,
                        nested_destination,
                    );
                    let nested_destination_point =
                        unreal_to_render(nested_portal.source.transform_to(
                            nested_destination,
                            render_to_unreal(nested_portal.plane.0),
                        ));
                    let nested_destination_normal =
                        unreal_to_render(nested_portal.source.transform_vector_to(
                            nested_destination,
                            render_to_unreal(nested_portal.plane.1),
                        ));
                    let nested_clip_plane = -mirror_clip_plane(
                        nested_position,
                        nested_destination_point,
                        nested_destination_normal,
                    );
                    let view = &portal.nested_views[1];
                    let target = &self.nested_warp_targets[1];
                    queue.write_buffer(
                        &view.camera_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_projection: (Mat4::perspective_rh(
                                camera.vertical_fov,
                                aspect,
                                camera.near,
                                camera.far,
                            ) * nested_world_to_view)
                                .to_cols_array_2d(),
                            world_to_view: nested_world_to_view.to_cols_array_2d(),
                            camera_position: nested_position.extend(1.0).to_array(),
                            auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                            clip_plane: nested_clip_plane.to_array(),
                        }),
                    );
                    let plan = self
                        .submission
                        .plan(nested_position, &self.material_bindings);
                    if !plan.indices.is_empty() {
                        queue.write_buffer(
                            &view.blended_index_buffer,
                            0,
                            bytemuck::cast_slice(&plan.indices),
                        );
                    }
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("OpenHP1 nested warp-portal render pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target.view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(clear_color()),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &target.depth.view,
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
                    draw_calls += self.draw_scene(
                        &mut pass,
                        &view.camera_bind_group,
                        &view.blended_index_buffer,
                        &plan,
                        None,
                        None,
                        None,
                        ScenePass::Reflection,
                    );
                }

                let view = &portal.nested_views[0];
                queue.write_buffer(
                    &view.camera_buffer,
                    0,
                    bytemuck::bytes_of(&CameraUniform {
                        view_projection: (Mat4::perspective_rh(
                            camera.vertical_fov,
                            aspect,
                            camera.near,
                            camera.far,
                        ) * reflected_view)
                            .to_cols_array_2d(),
                        world_to_view: reflected_view.to_cols_array_2d(),
                        camera_position: reflected_position.extend(1.0).to_array(),
                        auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                        clip_plane: mirror_clip_plane(position, mirror.plane.0, mirror.plane.1)
                            .to_array(),
                    }),
                );
                let plan = self
                    .submission
                    .plan(reflected_position, &self.material_bindings);
                if !plan.indices.is_empty() {
                    queue.write_buffer(
                        &view.blended_index_buffer,
                        0,
                        bytemuck::cast_slice(&plan.indices),
                    );
                }
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("OpenHP1 warp-portal mirror render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mirror.target.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear_color()),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &mirror.target.depth.view,
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
                draw_calls += self.draw_scene(
                    &mut pass,
                    &view.camera_bind_group,
                    &view.blended_index_buffer,
                    &plan,
                    None,
                    nested.map(|nested| (&self.warp_portals[nested], &self.nested_warp_targets[1])),
                    None,
                    ScenePass::Reflection,
                );
            }

            let view = &portal.view;
            queue.write_buffer(
                &view.camera_buffer,
                0,
                bytemuck::bytes_of(&CameraUniform {
                    view_projection: (Mat4::perspective_rh(
                        camera.vertical_fov,
                        aspect,
                        camera.near,
                        camera.far,
                    ) * world_to_view)
                        .to_cols_array_2d(),
                    world_to_view: world_to_view.to_cols_array_2d(),
                    camera_position: position.extend(1.0).to_array(),
                    auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                    clip_plane: clip_plane.to_array(),
                }),
            );
            let plan = self.submission.plan(position, &self.material_bindings);
            if !plan.indices.is_empty() {
                queue.write_buffer(
                    &view.blended_index_buffer,
                    0,
                    bytemuck::cast_slice(&plan.indices),
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 warp-portal render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &portal.target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &portal.target.depth.view,
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
            draw_calls += self.draw_scene(
                &mut pass,
                &view.camera_bind_group,
                &view.blended_index_buffer,
                &plan,
                None,
                None,
                mirror,
                ScenePass::Portal,
            );
        }

        for mirror in &self.mirrors {
            let (plane_point, plane_normal) = mirror.plane;
            let (mirror_camera_position, mirror_world_to_view) =
                camera.reflected_view(plane_point, plane_normal);
            let clip_plane = mirror_clip_plane(camera.position, plane_point, plane_normal);
            queue.write_buffer(
                &mirror.camera_buffer,
                0,
                bytemuck::bytes_of(&CameraUniform {
                    view_projection: (Mat4::perspective_rh(
                        camera.vertical_fov,
                        aspect,
                        camera.near,
                        camera.far,
                    ) * mirror_world_to_view)
                        .to_cols_array_2d(),
                    world_to_view: mirror_world_to_view.to_cols_array_2d(),
                    camera_position: mirror_camera_position.extend(1.0).to_array(),
                    auto_uv: [self.auto_uv, 0.0, 0.0, 0.0],
                    clip_plane: clip_plane.to_array(),
                }),
            );
            let plan = self
                .submission
                .plan(mirror_camera_position, &self.material_bindings);
            if !plan.indices.is_empty() {
                queue.write_buffer(
                    &mirror.blended_index_buffer,
                    0,
                    bytemuck::cast_slice(&plan.indices),
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 mirror render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &mirror.target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &mirror.target.depth.view,
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
            draw_calls += self.draw_scene(
                &mut pass,
                &mirror.camera_bind_group,
                &mirror.blended_index_buffer,
                &plan,
                None,
                None,
                None,
                ScenePass::Reflection,
            );
        }

        if !main_plan.indices.is_empty() {
            queue.write_buffer(
                &self.blended_index_buffer,
                0,
                bytemuck::cast_slice(&main_plan.indices),
            );
        }

        let scene_target = self
            .modern
            .as_ref()
            .map(ModernRenderer::scene_view)
            .or_else(|| {
                self.classic_display
                    .as_ref()
                    .map(ClassicDisplay::scene_view)
            })
            .unwrap_or(target);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 BSP render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene_target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.view,
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
        draw_calls += self.draw_scene(
            &mut pass,
            &self.camera_bind_group,
            &self.blended_index_buffer,
            &main_plan,
            self.sky_target.as_ref(),
            None,
            None,
            ScenePass::Main,
        );
        if let Some(coronas) = &self.coronas {
            draw_calls += coronas.draw(&mut pass, &self.texture_bind_groups[0]);
        }
        drop(pass);
        if let Some(modern) = &mut self.modern {
            draw_calls += modern.render(
                queue,
                encoder,
                target,
                display.brightness,
                display.contrast,
                camera,
            );
            draw_calls += self.flash.render(queue, encoder, target, flash);
        } else if let Some(classic_display) = &self.classic_display {
            draw_calls += self
                .flash
                .render(queue, encoder, classic_display.scene_view(), flash);
            draw_calls += classic_display.render(queue, encoder, target, display.brightness);
        } else {
            draw_calls += self.flash.render(queue, encoder, target, flash);
        }
        RenderStats {
            draw_calls,
            ..self.stats
        }
    }

    fn draw_scene<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        submission_index_buffer: &'pass wgpu::Buffer,
        plan: &SubmissionPlan,
        backdrop_target: Option<&'pass SampledTarget>,
        nested_warp_portal: Option<(&'pass WarpPortal, &'pass SampledTarget)>,
        nested_mirror: Option<&'pass Mirror>,
        scene_pass: ScenePass,
    ) -> usize {
        let mut draw_calls = 0;
        let pipelines = if matches!(scene_pass, ScenePass::Reflection) {
            self.reflected_pipelines
                .as_ref()
                .expect("mirror pass requires reflected pipelines")
        } else {
            &self.pipelines
        };
        let reflected = matches!(scene_pass, ScenePass::Reflection);
        pass.set_bind_group(0, camera_bind_group, &[]);
        if let Some(lighting) = &self.lighting {
            pass.set_bind_group(2, &lighting.bind_group, &[]);
        }
        if !plan.commands.is_empty() {
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(submission_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        }
        for command in &plan.commands {
            match command {
                SubmissionCommand::Geometry { batch, .. } => {
                    pass.set_pipeline(&pipelines[batch.pipeline]);
                    pass.set_bind_group(1, &self.material_bind_groups[batch.binding], &[]);
                    pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                    draw_calls += 1;
                    draw_calls += self.draw_attachments(
                        pass,
                        batch.binding,
                        batch.indices.clone(),
                        reflected,
                    );
                }
                SubmissionCommand::Portal { surface, indices } => {
                    let portal = if matches!(scene_pass, ScenePass::Main) {
                        self.warp_portals.iter().find(|portal| {
                            portal.surface == *surface && portal.destination.is_some()
                        })
                    } else {
                        nested_warp_portal
                            .filter(|(portal, _)| portal.surface == *surface)
                            .map(|(portal, _)| portal)
                    };
                    let target = nested_warp_portal
                        .filter(|(portal, _)| portal.surface == *surface)
                        .map(|(_, target)| target);
                    if let Some(portal) = portal {
                        let bind_group = target.map_or_else(
                            || portal.target.bind_group(portal.no_smooth),
                            |target| target.bind_group(portal.no_smooth),
                        );
                        pass.set_bind_group(1, bind_group, &[]);
                        pass.set_pipeline(&self.mirror_pipelines[portal.pipeline]);
                        pass.draw_indexed(indices.clone(), 0, 0..1);
                        draw_calls += 1;
                    }
                }
                SubmissionCommand::Mirror { surface, indices } => {
                    let mirror = if matches!(scene_pass, ScenePass::Main) {
                        self.mirrors
                            .iter()
                            .find(|mirror| mirror.surface == *surface)
                    } else {
                        nested_mirror.filter(|mirror| mirror.surface == *surface)
                    };
                    if let Some(mirror) = mirror {
                        pass.set_bind_group(1, mirror.target.bind_group(mirror.no_smooth), &[]);
                        pass.set_pipeline(&self.mirror_pipelines[mirror.pipeline]);
                        pass.draw_indexed(indices.clone(), 0, 0..1);
                        draw_calls += 1;
                        pass.set_bind_group(1, &self.material_bind_groups[mirror.binding], &[]);
                        draw_calls +=
                            self.draw_attachments(pass, mirror.binding, indices.clone(), reflected);
                    }
                }
                SubmissionCommand::Backdrop {
                    indices,
                    pipeline,
                    no_smooth,
                    ..
                } => {
                    if let Some(target) = backdrop_target {
                        pass.set_bind_group(1, target.bind_group(*no_smooth), &[]);
                        pass.set_pipeline(&self.backdrop_pipelines[*pipeline]);
                        pass.draw_indexed(indices.clone(), 0, 0..1);
                        draw_calls += 1;
                    }
                }
            }
        }
        draw_calls
    }

    fn draw_attachments<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        binding: usize,
        indices: std::ops::Range<u32>,
        reflected: bool,
    ) -> usize {
        let material = self.material_bindings[binding];
        let mut draw_calls = 0;
        for pipeline in attachment_pipeline_indices(material) {
            pass.set_pipeline(
                &self.attachment_pipelines
                    [attachment_pipeline_index(material, pipeline, reflected)],
            );
            pass.draw_indexed(indices.clone(), 0, 0..1);
            draw_calls += 1;
        }
        draw_calls
    }
}

fn attachment_pipeline_index(material: MaterialBinding, pass: usize, reflected: bool) -> usize {
    pass + usize::from(material.pipeline % 2 != 0) * 5 + usize::from(reflected) * 10
}

fn attachment_pipeline_indices(material: MaterialBinding) -> impl Iterator<Item = usize> {
    let mut passes = [None; 5];
    let mut count = 0;
    if material.macro_enabled {
        passes[count] = Some(0);
        count += 1;
        if material.lit {
            passes[count] = Some(1);
            count += 1;
        }
    }
    if material.detail_enabled {
        for pipeline in 2..5 {
            passes[count] = Some(pipeline);
            count += 1;
        }
    }
    passes.into_iter().flatten()
}

#[allow(clippy::too_many_arguments)]
fn build_material_bind_groups(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    samplers: &[wgpu::Sampler; 2],
    textures: &[wgpu::Texture],
    fallback_texture: usize,
    lightmap_view: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
    materials: &[MaterialBinding],
) -> Vec<wgpu::BindGroup> {
    let view =
        |index: usize| textures[index.min(fallback_texture)].create_view(&Default::default());
    materials
        .iter()
        .map(|material| {
            let base = view(material.texture);
            let macro_texture = view(material.macro_texture);
            let detail_texture = view(material.detail_texture);
            material_texture_bind_group(
                device,
                layout,
                samplers,
                material.no_smooth,
                &base,
                &macro_texture,
                &detail_texture,
                lightmap_view,
                lightmap_sampler,
            )
        })
        .collect()
}

fn detail_attachment_coordinates(
    coordinates: [f32; 2],
    bsp_pan: [f32; 2],
    draw_scale: f32,
) -> [f32; 2] {
    [
        (coordinates[0] - bsp_pan[0]) / draw_scale,
        (coordinates[1] - bsp_pan[1]) / draw_scale,
    ]
}

fn macro_attachment_coordinates(
    coordinates: [f32; 2],
    bsp_pan: [f32; 2],
    draw_scale: f32,
) -> [f32; 2] {
    detail_attachment_coordinates(coordinates, bsp_pan, draw_scale)
        .map(|coordinate| coordinate + 0.5)
}

fn texture_needs_recreation(
    width: u32,
    height: u32,
    mip_level_count: u32,
    image: &TextureImage,
) -> bool {
    width != image.width || height != image.height || mip_level_count != image.mip_level_count()
}

fn quantized_flash(flash: [f32; 4]) -> [f32; 4] {
    flash.map(|component| {
        ((component * 256.0 - 0.5)
            .round_ties_even()
            .clamp(0.0, 255.0))
            / 255.0
    })
}

fn texture_filter(no_smooth: bool) -> wgpu::FilterMode {
    if no_smooth {
        wgpu::FilterMode::Nearest
    } else {
        wgpu::FilterMode::Linear
    }
}

fn texture_sampler(
    device: &wgpu::Device,
    label: &'static str,
    address_mode: wgpu::AddressMode,
    no_smooth: bool,
    modern: bool,
) -> wgpu::Sampler {
    let (filter, mipmap_filter, anisotropy_clamp) = texture_sampler_settings(modern, no_smooth);
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter,
        anisotropy_clamp,
        ..Default::default()
    })
}

fn texture_sampler_settings(
    modern: bool,
    no_smooth: bool,
) -> (wgpu::FilterMode, wgpu::MipmapFilterMode, u16) {
    let filter = texture_filter(no_smooth);
    if modern && !no_smooth {
        (filter, wgpu::MipmapFilterMode::Linear, 16)
    } else {
        (filter, wgpu::MipmapFilterMode::Nearest, 1)
    }
}

fn flash_blend_state() -> wgpu::BlendState {
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::SrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
}

fn flash_target_state(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(flash_blend_state()),
        write_mask: wgpu::ColorWrites::COLOR,
    }
}

fn normalized_small_wavy_scale(enabled: bool, dimensions: [f32; 2]) -> [f32; 2] {
    if enabled {
        [1.0 / dimensions[0], 1.0 / dimensions[1]]
    } else {
        [0.0; 2]
    }
}

fn display_gamma(brightness: f32) -> f32 {
    1.0 / (brightness * 2.0).clamp(0.05, 2.99)
}

fn pack_vertex_color(color: Vec3, opacity: f32) -> [u8; 4] {
    let color = color.clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
    let opacity = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    [
        color.x.round() as u8,
        color.y.round() as u8,
        color.z.round() as u8,
        (opacity * 255.0).round() as u8,
    ]
}

fn clear_color() -> wgpu::Color {
    wgpu::Color {
        r: 0.035,
        g: 0.045,
        b: 0.065,
        a: 1.0,
    }
}

fn checkerboard() -> TextureImage {
    TextureImage {
        width: 2,
        height: 2,
        logical_width: 2,
        logical_height: 2,
        rgba: vec![
            255, 0, 255, 255, 24, 24, 24, 255, 24, 24, 24, 255, 255, 0, 255, 255,
        ],
        mips: Vec::new(),
    }
}

fn texture_coordinate_dimensions(texture: Option<&TextureImage>) -> [f32; 2] {
    texture.map_or([64.0, 64.0], |texture| {
        texture
            .logical_dimensions()
            .map(|dimension| dimension as f32)
    })
}

fn scene_bounds(vertices: &[Vertex]) -> SceneBounds {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for vertex in vertices {
        let position = Vec3::from_array(vertex.position);
        minimum = minimum.min(position);
        maximum = maximum.max(position);
    }
    if vertices.is_empty() {
        minimum = Vec3::ZERO;
        maximum = Vec3::ONE;
    }
    SceneBounds { minimum, maximum }
}

fn mirror_plane(
    scene: &RenderScene,
    vertices: &[Vertex],
    mirror_surface: usize,
) -> Option<(Vec3, Vec3)> {
    scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
        .find_map(|(triangle, &surface)| {
            if surface != mirror_surface {
                return None;
            }
            let &[a, b, c] = triangle else {
                return None;
            };
            let [a, b, c] = [a, b, c].map(|vertex| usize::try_from(vertex).ok());
            let (a, b, c) = (a?, b?, c?);
            let [a, b, c] = [a, b, c].map(|vertex| {
                vertices
                    .get(vertex)
                    .map(|vertex| Vec3::from_array(vertex.position))
            });
            let (a, b, c) = (a?, b?, c?);
            let normal = (b - a).cross(c - a).normalize_or_zero();
            (normal.length_squared() > 0.0).then_some((a, normal))
        })
}

fn surface_bounds(
    scene: &RenderScene,
    vertices: &[Vertex],
    surface: usize,
) -> Option<(Vec3, Vec3)> {
    scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
        .filter(|(_, triangle_surface)| **triangle_surface == surface)
        .flat_map(|(triangle, _)| triangle)
        .filter_map(|&index| usize::try_from(index).ok())
        .filter_map(|index| vertices.get(index))
        .map(|vertex| Vec3::from_array(vertex.position))
        .fold(None, |bounds, position| {
            Some(
                bounds.map_or((position, position), |(minimum, maximum): (Vec3, Vec3)| {
                    (minimum.min(position), maximum.max(position))
                }),
            )
        })
}

fn ray_surface_distance(
    position: Vec3,
    direction: Vec3,
    plane: (Vec3, Vec3),
    bounds: (Vec3, Vec3),
) -> Option<f32> {
    let denominator = direction.dot(plane.1);
    if denominator.abs() < 1.0e-5 {
        return None;
    }
    let distance = (plane.0 - position).dot(plane.1) / denominator;
    if distance <= 1.0e-3 {
        return None;
    }
    let point = position + direction * distance;
    let tolerance = Vec3::splat(0.5);
    (point.cmpge(bounds.0 - tolerance).all() && point.cmple(bounds.1 + tolerance).all())
        .then_some(distance)
}

fn warp_portal_active(portal: &WarpPortal, camera_position: Vec3) -> bool {
    warp_portal_side_active(
        portal.authored_plane,
        portal.source_on_positive_side,
        camera_position,
    )
}

fn warp_portal_side_active(
    authored_plane: [f32; 4],
    source_on_positive_side: bool,
    camera_position: Vec3,
) -> bool {
    let camera = render_to_unreal(camera_position);
    let side = Vec3::from_array([authored_plane[0], authored_plane[1], authored_plane[2]])
        .dot(camera)
        - authored_plane[3];
    (side >= 0.0) == source_on_positive_side
}

fn mirror_clip_plane(camera: Vec3, point: Vec3, normal: Vec3) -> Vec4 {
    let mut normal = normal.normalize_or_zero();
    if (camera - point).dot(normal) < 0.0 {
        normal = -normal;
    }
    normal.extend(-point.dot(normal))
}

#[cfg(test)]
mod tests {
    use openhp1_scene::TriangleMesh;

    use super::*;

    #[test]
    fn shaders_are_valid_wgsl() {
        for shader in [
            include_str!("shaders/scene.wgsl"),
            include_str!("shaders/corona.wgsl"),
            include_str!("shaders/flash.wgsl"),
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
    fn modern_backdrop_preserves_linear_sky_color() {
        let fragment = include_str!("shaders/scene.wgsl")
            .split_once("fn fragment_backdrop_modern")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        assert!(!fragment.contains("srgb_to_linear"));
    }

    #[test]
    fn modern_shader_keeps_tone_mapping_and_effect_invariants() {
        let shader = modern::COMPOSITE_SHADER;
        let scene_shader = include_str!("shaders/scene.wgsl");
        let corona_shader = include_str!("shaders/corona.wgsl");
        assert!(shader.contains("const WHITE = 1.25;"));
        assert!(shader.contains("const LUMINANCE = vec3(0.2126, 0.7152, 0.0722);"));
        assert!(shader.contains("let luminance = dot(color, LUMINANCE);"));
        assert!(shader.contains("return color * (mapped_luminance / luminance);"));
        assert!(shader.contains("let encoded = srgb_encode(clamp(mapped"));
        assert!(shader.contains("let contrasted = display_contrast(encoded);"));
        assert!(modern::BLOOM_SHADER.contains("const THRESHOLD = 1.0;"));
        assert!(modern::BLOOM_SHADER.contains("const KNEE = 0.1;"));
        assert!(!shader.contains("ao_texture"));
        assert!(scene_shader.contains("vec4(color.rgb, 0.0)"));
        assert!(scene_shader.contains("min(light.color.rgb * strength, vec3(1.0))"));
        assert!(scene_shader.contains("color.rgb * illumination * 2.0"));
        assert!(scene_shader.contains("srgb_to_linear(display_color.rgb)"));
        assert!(scene_shader.contains("srgb_to_linear(color.rgb * input.vertex_color.rgb)"));
        assert!(corona_shader.contains("corner * vec2(1.6, 1.6 * aspect) * color_and_scale.w;"));
        assert!(corona_shader.contains("srgb_to_linear(lit) * 4.0"));
        assert!(!corona_shader.contains("distance_scale"));
        assert!(!corona_shader.contains("color.a * CORONA_HDR_GAIN"));

        let white = 1.25_f32;
        let mapped_white = white * (1.0 + white / (white * white)) / (1.0 + white);
        assert!((mapped_white - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn texture_pan_uses_node_plane_when_surface_normal_opposes_it() {
        let shader = include_str!("shaders/scene.wgsl");
        assert!(shader.contains(
            "let texture_pan_speed = select(\n            texture_pan_speeds.xy,\n            texture_pan_speeds.zw,\n            dot(camera.camera_position.xyz - position, node_plane_normal) > 0.0,\n        );"
        ));

        let surface_normal = Vec3::X;
        let node_plane_normal = Vec3::NEG_X;
        let camera_offset = Vec3::NEG_X;
        assert!(camera_offset.dot(surface_normal) < 0.0);
        assert!(camera_offset.dot(node_plane_normal) > 0.0);
        let speeds = [[1.0, 2.0], [3.0, 4.0]];
        let select = |offset: Vec3| speeds[usize::from(offset.dot(node_plane_normal) > 0.0)];
        assert_eq!(select(camera_offset), speeds[1]);
        assert_eq!(select(-camera_offset), speeds[0]);
    }

    #[test]
    fn small_wavy_uses_original_formula_in_normalized_texture_units() {
        assert_eq!(
            normalized_small_wavy_scale(true, [64.0, 128.0]),
            [1.0 / 64.0, 1.0 / 128.0]
        );
        assert_eq!(normalized_small_wavy_scale(false, [64.0, 128.0]), [0.0; 2]);

        let shader = include_str!("shaders/scene.wgsl");
        assert!(shader.contains(
            "output.texture_coordinates = texture_coordinates + texture_pan_speed * camera.auto_uv.x;\n        if any(uv_effect_scale != vec2(0.0)) {"
        ));
        assert!(shader.contains("let time = camera.auto_uv.x / 64.0;"));
        assert!(shader.contains("8.0 * sin(time) + 4.0 * cos(2.3 * time),"));
        assert!(shader.contains("8.0 * cos(time) + 4.0 * sin(2.3 * time),"));
        assert!(shader.contains(
            "output.texture_coordinates = output.texture_coordinates\n                + uv_effect_scale * small_wavy_offset;"
        ));
    }

    #[test]
    fn computes_scene_bounds() {
        let vertices = [
            Vertex {
                position: [-2.0, 3.0, 1.0],
                texture_coordinates: [0.0; 2],
                texture_pan_speeds: [0.0; 4],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
                vertex_color: [255; 4],
                normal: [0.0, 1.0, 0.0],
                environment_map: 0.0,
                lighting_coordinates: [0.0; 2],
                lighting_index: u32::MAX,
                uv_effect_scale: [0.0; 2],
                node_plane_normal: [0.0; 3],
                macro_texture_coordinates: [0.0; 2],
                detail_texture_coordinates: [0.0; 2],
                attachment_flags: [0; 2],
            },
            Vertex {
                position: [4.0, -1.0, 7.0],
                texture_coordinates: [0.0; 2],
                texture_pan_speeds: [0.0; 4],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
                vertex_color: [255; 4],
                normal: [0.0, 1.0, 0.0],
                environment_map: 0.0,
                lighting_coordinates: [0.0; 2],
                lighting_index: u32::MAX,
                uv_effect_scale: [0.0; 2],
                node_plane_normal: [0.0; 3],
                macro_texture_coordinates: [0.0; 2],
                detail_texture_coordinates: [0.0; 2],
                attachment_flags: [0; 2],
            },
        ];
        let bounds = scene_bounds(&vertices);
        assert_eq!(bounds.minimum, Vec3::new(-2.0, -1.0, 1.0));
        assert_eq!(bounds.maximum, Vec3::new(4.0, 3.0, 7.0));
    }

    #[test]
    fn no_smooth_selects_point_filter_and_separate_batches() {
        assert_eq!(texture_filter(false), wgpu::FilterMode::Linear);
        assert_eq!(texture_filter(true), wgpu::FilterMode::Nearest);
        assert_eq!(
            texture_sampler_settings(false, false),
            (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Nearest, 1,)
        );
        assert_eq!(
            texture_sampler_settings(true, false),
            (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear, 16,)
        );
        assert_eq!(
            texture_sampler_settings(true, true),
            (
                wgpu::FilterMode::Nearest,
                wgpu::MipmapFilterMode::Nearest,
                1,
            )
        );
        assert_eq!(
            pipeline::fragment_compilation_options(false).constants,
            &[("texture_lod_bias", -0.5)]
        );
        assert_eq!(
            pipeline::fragment_compilation_options(true).constants,
            &[("texture_lod_bias", 0.0)]
        );
        assert!(SCENE_SHADER.contains("override texture_lod_bias: f32 = -0.5;"));
    }

    #[test]
    fn attachment_batches_group_only_by_bound_resources_and_pipeline() {
        let scene = RenderScene {
            mesh: TriangleMesh {
                indices: (0..9).collect(),
                triangle_surfaces: vec![0, 1, 2],
                ..Default::default()
            },
            textures: vec![
                checkerboard(),
                checkerboard(),
                checkerboard(),
                checkerboard(),
            ],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![],
            corona_visibility: Default::default(),
            actor_submissions: vec![],
            surface_materials: vec![
                SurfaceMaterial {
                    texture: Some(0),
                    macro_texture: Some(1),
                    detail_texture: Some(2),
                    macro_draw_scale: 2.0,
                    bsp_texture_pan: [3.0, 4.0],
                    ..Default::default()
                },
                SurfaceMaterial {
                    texture: Some(0),
                    macro_texture: Some(1),
                    detail_texture: Some(2),
                    macro_draw_scale: 8.0,
                    bsp_texture_pan: [9.0, 10.0],
                    ..Default::default()
                },
                SurfaceMaterial {
                    texture: Some(0),
                    macro_texture: Some(1),
                    detail_texture: Some(3),
                    ..Default::default()
                },
            ],
            transmission_masks: Default::default(),
            warp_portals: vec![],
            sky_zone: None,
        };

        let (bindings, surfaces) = material_bindings(&scene, 4, true);
        assert_eq!(bindings.len(), 2);
        assert_eq!(surfaces[0], surfaces[1]);
        assert_ne!(surfaces[1], surfaces[2]);
    }

    #[test]
    fn portal_and_fog_map_suppress_only_the_detail_pass() {
        let bindings_for = |material| {
            material_bindings(
                &RenderScene {
                    textures: vec![checkerboard(), checkerboard(), checkerboard()],
                    mesh: TriangleMesh::default(),
                    lightmaps: vec![],
                    realtime_lightmaps: vec![],
                    coronas: vec![],
                    corona_visibility: Default::default(),
                    actor_submissions: vec![],
                    surface_materials: vec![material],
                    transmission_masks: Default::default(),
                    warp_portals: vec![],
                    sky_zone: None,
                },
                3,
                true,
            )
            .0[0]
        };
        let attached = SurfaceMaterial {
            macro_texture: Some(1),
            detail_texture: Some(2),
            ..Default::default()
        };
        assert!(bindings_for(attached).detail_enabled);
        assert!(
            !bindings_for(SurfaceMaterial {
                portal: true,
                ..attached
            })
            .detail_enabled
        );
        assert!(
            !bindings_for(SurfaceMaterial {
                fog_map_attached: true,
                ..attached
            })
            .detail_enabled
        );
        assert!(
            bindings_for(SurfaceMaterial {
                portal: true,
                ..attached
            })
            .macro_enabled
        );
    }

    #[test]
    fn animated_texture_shape_changes_select_resource_recreation() {
        let image = TextureImage {
            width: 4,
            height: 4,
            logical_width: 4,
            logical_height: 4,
            rgba: vec![0; 4 * 4 * 4],
            mips: vec![openhp1_scene::TextureMipImage {
                width: 2,
                height: 2,
                rgba: vec![0; 2 * 2 * 4],
            }],
        };

        assert!(!texture_needs_recreation(4, 4, 2, &image));
        assert!(texture_needs_recreation(8, 8, 2, &image));
        assert!(texture_needs_recreation(4, 4, 1, &image));
    }

    #[test]
    fn replacement_resolution_does_not_change_texture_coordinate_scale() {
        let image = TextureImage {
            width: 1024,
            height: 512,
            logical_width: 256,
            logical_height: 128,
            rgba: Vec::new(),
            mips: Vec::new(),
        };

        assert_eq!(texture_coordinate_dimensions(Some(&image)), [256.0, 128.0]);
    }

    #[test]
    fn mirrored_surfaces_keep_their_own_reflection_planes() {
        let vertices = [
            vertex_at(2.0, 0.0, 0.0),
            vertex_at(2.0, 1.0, 0.0),
            vertex_at(2.0, 0.0, 1.0),
            vertex_at(0.0, 3.0, 0.0),
            vertex_at(0.0, 3.0, 1.0),
            vertex_at(1.0, 3.0, 0.0),
        ];
        let scene = RenderScene {
            mesh: TriangleMesh {
                indices: (0..6).collect(),
                triangle_surfaces: vec![0, 1],
                vertex_surfaces: vec![2; 6],
                ..Default::default()
            },
            textures: vec![],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![],
            corona_visibility: Default::default(),
            actor_submissions: vec![],
            surface_materials: vec![
                SurfaceMaterial {
                    mirror: true,
                    ..Default::default()
                },
                SurfaceMaterial {
                    mirror: true,
                    two_sided: true,
                    ..Default::default()
                },
                SurfaceMaterial::default(),
            ],
            transmission_masks: Default::default(),
            warp_portals: vec![],
            sky_zone: None,
        };

        let (_, surfaces) = material_bindings(&scene, 0, false);
        let geometries = mirror_geometries(&scene, &surfaces);
        assert_eq!(geometries.len(), 2);
        assert_eq!(geometries[0].surface, 0);
        assert_eq!(geometries[0].binding, surfaces[0]);
        assert_eq!(geometries[0].pipeline, 0);
        assert_eq!(geometries[1].surface, 1);
        assert_eq!(geometries[1].binding, surfaces[1]);
        assert_eq!(geometries[1].pipeline, 1);
        assert_eq!(
            mirror_plane(&scene, &vertices, 0),
            Some((Vec3::new(2.0, 0.0, 0.0), Vec3::X))
        );
        assert_eq!(
            mirror_plane(&scene, &vertices, 1),
            Some((Vec3::new(0.0, 3.0, 0.0), Vec3::Y))
        );
    }

    #[test]
    fn mirror_clip_plane_keeps_only_the_viewer_side_of_the_portal() {
        let point = Vec3::new(0.0, -10.0, 0.0);
        let plane = mirror_clip_plane(Vec3::ZERO, point, -Vec3::Y);

        assert!(plane.dot(Vec3::ZERO.extend(1.0)) > 0.0);
        assert_eq!(plane.dot(point.extend(1.0)), 0.0);
        assert!(plane.dot(Vec3::new(0.0, -11.0, 0.0).extend(1.0)) < 0.0);
    }

    #[test]
    fn warp_portal_uses_the_zone_actor_on_the_opposite_bsp_side() {
        let plane = [0.0, 1.0, 0.0, 10.0];
        let positive_camera = unreal_to_render(Vec3::new(0.0, 12.0, 0.0));
        let negative_camera = unreal_to_render(Vec3::new(0.0, 8.0, 0.0));

        assert!(warp_portal_side_active(plane, true, positive_camera));
        assert!(!warp_portal_side_active(plane, true, negative_camera));
        assert!(warp_portal_side_active(plane, false, negative_camera));
    }

    #[test]
    fn uses_ue1_blend_equations() {
        let translucent = blend_state(SurfaceMode::Translucent).unwrap();
        assert_eq!(translucent.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(translucent.color.dst_factor, wgpu::BlendFactor::OneMinusSrc);

        let modulated = blend_state(SurfaceMode::Modulated).unwrap();
        assert_eq!(modulated.color.src_factor, wgpu::BlendFactor::Dst);
        assert_eq!(modulated.color.dst_factor, wgpu::BlendFactor::Src);

        let alpha = blend_state(SurfaceMode::AlphaBlended).unwrap();
        assert_eq!(alpha.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(alpha.color.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
        let target = flash_target_state(wgpu::TextureFormat::Rgba8Unorm);
        let flash = target.blend.unwrap();
        assert_eq!(flash.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(flash.color.dst_factor, wgpu::BlendFactor::SrcAlpha);
        assert!(!target.write_mask.contains(wgpu::ColorWrites::ALPHA));
        assert_eq!(
            fragment_entry(SurfaceMode::Modulated, true, false, false),
            "fragment_blended_masked"
        );
        assert_eq!(
            fragment_entry(SurfaceMode::Opaque, false, true, false),
            "fragment_unlit"
        );
    }

    #[test]
    fn depth_only_pipeline_state_remains_available_while_submission_is_deferred() {
        let material = SurfaceMaterial {
            mode: SurfaceMode::DepthOnly,
            masked: true,
            macro_texture: Some(0),
            detail_texture: Some(0),
            ..Default::default()
        };
        assert_eq!(attachment_enabled(material, true), [false, false]);
        assert!(depth_write_enabled(SurfaceMode::DepthOnly));
        assert_eq!(
            color_write_mask(SurfaceMode::DepthOnly),
            wgpu::ColorWrites::empty()
        );
        assert!(blend_state(SurfaceMode::DepthOnly).is_none());
        assert_eq!(
            fragment_entry(SurfaceMode::DepthOnly, true, false, false),
            "fragment_masked"
        );
        assert_eq!(
            fragment_entry(SurfaceMode::DepthOnly, true, false, true),
            "fragment_modern_masked"
        );
    }

    #[test]
    fn viewport_flash_quantizes_then_blends_like_d3d() {
        let blend = |scene: [f32; 3], flash: [f32; 4]| {
            let flash = quantized_flash(flash);
            std::array::from_fn(|index| (flash[index] + scene[index] * flash[3]).min(1.0))
        };

        assert_eq!(
            blend([0.2, 0.4, 0.8], [0.0, 0.0, 0.0, 1.0]),
            [0.2, 0.4, 0.8]
        );
        assert_eq!(blend([0.2, 0.4, 0.8], [0.0; 4]), [0.0; 3]);
        let fractional = blend([0.5, 0.25, 0.75], [0.2, 0.4, 0.0, 128.0 / 255.0]);
        let expected = [
            0.2 + 0.5 * 128.0 / 255.0,
            0.4 + 0.25 * 128.0 / 255.0,
            0.75 * 128.0 / 255.0,
        ];
        assert!(
            fractional
                .into_iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1.0e-6)
        );
        assert_eq!(blend([1.0; 3], [1.0, 0.5, 0.25, 1.0]), [1.0; 3]);
        assert_eq!(
            quantized_flash([-1.0, 0.5, 2.0, 0.5]),
            [0.0, 128.0 / 255.0, 1.0, 128.0 / 255.0]
        );
        assert_eq!(quantized_flash([0.1; 4]), [25.0 / 255.0; 4]);
        assert_eq!(quantized_flash([1.0 / 256.0; 4]), [0.0; 4]);
        assert_eq!(quantized_flash([2.0 / 256.0; 4]), [2.0 / 255.0; 4]);
        assert_eq!(quantized_flash([3.0 / 256.0; 4]), [2.0 / 255.0; 4]);
        assert_eq!(quantized_flash([4.0 / 256.0; 4]), [4.0 / 255.0; 4]);
    }

    #[test]
    fn unlit_blended_surfaces_skip_scene_lighting() {
        for mode in [
            SurfaceMode::Translucent,
            SurfaceMode::Modulated,
            SurfaceMode::AlphaBlended,
        ] {
            assert_eq!(
                fragment_entry(mode, false, true, false),
                "fragment_unlit_blended"
            );
            assert_eq!(
                fragment_entry(mode, true, true, false),
                "fragment_unlit_blended_masked"
            );
            assert_eq!(
                fragment_entry(mode, false, true, true),
                "fragment_modern_unlit_blended"
            );
            assert_eq!(
                fragment_entry(mode, true, true, true),
                "fragment_modern_unlit_blended_masked"
            );
        }
    }

    #[test]
    fn converts_modern_screen_brightness_to_display_gamma() {
        assert_eq!(display_gamma(0.5), 1.0);
        assert_eq!(display_gamma(0.625), 0.8);
    }

    #[test]
    fn reconstructs_view_position_from_wgpu_depth() {
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 1_000.0);
        let aspect = 16.0 / 9.0;
        let point = Vec3::new(2.0, 1.0, -10.0);
        let clip = camera.view_projection(aspect) * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let distance = camera.near * camera.far / (camera.far - ndc.z * (camera.far - camera.near));
        let tan_half_fov = (camera.vertical_fov * 0.5).tan();
        let reconstructed = Vec3::new(
            ndc.x * distance * tan_half_fov * aspect,
            ndc.y * distance * tan_half_fov,
            -distance,
        );
        assert!((reconstructed - point).length() < 0.000_01);
    }

    #[test]
    fn packs_vertex_lighting_like_fixed_function_diffuse_color() {
        assert_eq!(
            pack_vertex_color(Vec3::new(-0.5, 0.5, 2.0), 0.25),
            [0, 128, 255, 64]
        );
    }

    #[test]
    fn environment_maps_match_texture_info_reflection_color_and_unlit_precedence() {
        assert!(!SurfaceMaterial::default().environment_map);
        let environment = |position: Vec3,
                           normal: Vec3,
                           camera: Vec3,
                           world_to_view: glam::Mat3,
                           dimensions: glam::Vec2,
                           multiplier: f32| {
            let incident = (position - camera).normalize_or_zero();
            let reflection = incident - 2.0 * incident.dot(normal) * normal;
            let view_reflection = world_to_view * reflection;
            let texture_info_scale = dimensions * multiplier / 256.0;
            let raw = (view_reflection.truncate() + glam::Vec2::ONE) * 128.0 * texture_info_scale;
            let light = view_reflection.z.max(0.0).powf(0.25);
            let color = Vec4::new(light, light, light, 0.0);
            (raw / dimensions, color)
        };
        let (basis_coordinates, dark) = environment(
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            Vec3::ZERO,
            glam::Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2),
            glam::Vec2::new(96.0, 40.0),
            0.75,
        );
        assert!(basis_coordinates.abs_diff_eq(glam::Vec2::new(0.75, 0.375), 0.000_001));
        assert!(dark.abs_diff_eq(Vec4::ZERO, 0.000_001));

        let (centered, bright) = environment(
            -Vec3::Z,
            Vec3::Z,
            Vec3::ZERO,
            glam::Mat3::IDENTITY,
            glam::Vec2::new(37.0, 91.0),
            1.25,
        );
        assert!(centered.abs_diff_eq(glam::Vec2::splat(0.625), 0.000_001));
        assert!(bright.abs_diff_eq(Vec4::new(1.0, 1.0, 1.0, 0.0), 0.000_001));

        assert!(
            SCENE_SHADER.contains("output.environment_color = vec4(vec3(environment_light), 0.0);")
        );
        assert!(SCENE_SHADER.contains("color.a * input.environment_color.a"));
        assert!(SCENE_SHADER.contains(
            "return select(color, vec4(color.rgb, 1.0), input.environment_color.r >= 0.0);"
        ));
        assert!(SCENE_SHADER.contains("return vec4(color.rgb * input.vertex_color.rgb, alpha);"));
        assert!(
            SCENE_SHADER.contains(
                "return vec4(srgb_to_linear(color.rgb * input.vertex_color.rgb), alpha);"
            )
        );
        let destination = Vec3::new(0.2, 0.4, 0.6);
        let source = Vec3::new(1.0, 0.5, 0.25);
        let environment_alpha = bright.w;
        assert_eq!(
            source * environment_alpha + destination * (1.0 - environment_alpha),
            destination
        );
        assert!(SCENE_SHADER.contains("return preserve_environment_coverage(input, color);"));
        assert!(SCENE_SHADER.contains(
            "return select(texture_alpha, final_alpha, input.environment_color.r >= 0.0);"
        ));
        assert_eq!(
            SCENE_SHADER
                .matches("masked_alpha(input, texture_color.a, color.a)")
                .count(),
            8
        );
        let masked_alpha = |texture_alpha, final_alpha, environment| {
            if environment {
                final_alpha
            } else {
                texture_alpha
            }
        };
        assert_eq!(masked_alpha(1.0, environment_alpha, true), 0.0);
        assert_eq!(masked_alpha(1.0, 0.0, false), 1.0);
        assert_eq!(
            pipeline::fragment_entry(SurfaceMode::Opaque, true, false, false),
            "fragment_masked"
        );
        assert_eq!(
            pipeline::fragment_entry(SurfaceMode::Opaque, true, false, true),
            "fragment_modern_masked"
        );
        assert_eq!(
            pipeline::fragment_entry(SurfaceMode::Opaque, true, true, false),
            "fragment_unlit_masked"
        );
        assert_eq!(
            pipeline::fragment_entry(SurfaceMode::Opaque, true, true, true),
            "fragment_modern_unlit_masked"
        );
    }

    #[test]
    fn lightmap_atlas_replicates_edge_texels_into_gutters() {
        let atlas = build_lightmap_atlas(
            &[openhp1_scene::LightmapImage {
                width: 2,
                height: 1,
                rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
            }],
            512,
        );
        let rectangle = atlas.rectangles[0];
        let pixel = |x: u32, y: u32| {
            let offset = ((y * atlas.image.width + x) * 4) as usize;
            &atlas.image.rgba[offset..offset + 4]
        };
        assert_eq!(pixel(rectangle.x - 1, rectangle.y), [10, 20, 30, 255]);
        assert_eq!(pixel(rectangle.x, rectangle.y), [10, 20, 30, 255]);
        assert_eq!(pixel(rectangle.x + 1, rectangle.y), [40, 50, 60, 255]);
        assert_eq!(pixel(rectangle.x + 2, rectangle.y), [40, 50, 60, 255]);
    }

    #[test]
    fn attachment_passes_saturate_in_native_order() {
        assert_eq!(
            macro_attachment_coordinates([37.0, -11.0], [5.0, -3.0], 2.0),
            [16.5, -3.5]
        );
        assert_eq!(
            detail_attachment_coordinates([37.0, -11.0], [5.0, -3.0], 2.0),
            [16.0, -4.0]
        );
        let base = 0.8_f32;
        let macro_pass = (base * 1.0 * 2.0).min(1.0);
        let light_pass = (macro_pass * 0.25 * 2.0).min(1.0);
        let collapsed = (base * 1.0 * 2.0 * 0.25 * 2.0).min(1.0);
        assert_eq!(light_pass, 0.5);
        assert_eq!(collapsed, 0.8);

        let destination = 0.2_f32;
        let base_modulated = destination * 1.0 * 2.0;
        let native_macro = base_modulated * 1.0 * 2.0;
        let folded_once = destination * (1.0 * 1.0) * 2.0;
        assert_eq!(native_macro, 0.8);
        assert_eq!(folded_once, 0.4);

        let detail_alpha = ((380.0_f32 / 190.0 - 1.0) * 100.0).round() / 255.0;
        let detail_source = detail_alpha * 0.75 + (1.0 - detail_alpha) * (128.0 / 255.0);
        let detail_pass = (light_pass * detail_source * 2.0).min(1.0);
        assert!((detail_pass - 0.599_231).abs() < 0.000_001);
        let detail_visible =
            |setting: bool, attachment: bool, fog_map: bool| setting && attachment && !fog_map;
        assert!(detail_visible(true, true, false));
        assert!(!detail_visible(false, true, false));
        assert!(!detail_visible(true, true, true));
        assert!(SCENE_SHADER.contains("380.0 * 0.23679848"));
        assert!(SCENE_SHADER.contains("4.223 * 4.223"));
    }

    #[test]
    fn attachment_pass_planner_matches_multitexture_and_filter_rules() {
        let binding = |macro_enabled, detail_enabled, lit| MaterialBinding {
            texture: 0,
            macro_texture: 1,
            detail_texture: 2,
            no_smooth: true,
            pipeline: 0,
            macro_enabled,
            detail_enabled,
            lit,
        };
        assert_eq!(
            attachment_pipeline_indices(binding(true, true, true)).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(
            attachment_pipeline_indices(binding(false, true, true)).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(pipeline::SMOOTH_SAMPLER_INDEX, 0);
        let one_sided = binding(true, false, true);
        let two_sided = MaterialBinding {
            pipeline: 1,
            ..one_sided
        };
        assert_eq!(attachment_pipeline_index(one_sided, 0, false), 0);
        assert_eq!(attachment_pipeline_index(two_sided, 0, false), 5);
        assert_eq!(attachment_pipeline_index(one_sided, 0, true), 10);

        let modern_macro = SCENE_SHADER
            .split_once("fn fragment_modern_macro")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        assert!(modern_macro.contains("return sample_macro(input);"));
        assert!(!modern_macro.contains("srgb_to_linear"));
        let neutral = 128.0_f32 / 255.0;
        assert_eq!(neutral * 2.0, 256.0 / 255.0);

        for function in [
            "fragment_macro",
            "fragment_modern_macro",
            "fragment_attachment_light",
            "fragment_modern_attachment_light",
            "detail_source",
        ] {
            let body = SCENE_SHADER
                .split_once(&format!("fn {function}"))
                .unwrap()
                .1
                .split_once("\n}")
                .unwrap()
                .0;
            assert!(body.contains("clip_to_portal(input);"));
            assert!(!body.contains("sample_color"));
        }
        assert!(SCENE_SHADER.contains("if input.attachment_flags.x != 0u"));
        assert!(SCENE_SHADER.contains("fn fragment_modern_mirror"));
    }

    #[test]
    fn detail_alpha_rounds_halfway_values_to_even() {
        let round_ties_even = |value: f32| {
            let lower = value.floor();
            let fraction = value - lower;
            if fraction > 0.5 || (fraction == 0.5 && lower as u32 % 2 == 1) {
                lower + 1.0
            } else {
                lower
            }
        };

        assert_eq!(round_ties_even(0.5), 0.0);
        assert_eq!(round_ties_even(1.5), 2.0);
        assert_eq!(round_ties_even(2.5), 2.0);
        assert_eq!(round_ties_even(3.5), 4.0);
        assert!(SCENE_SHADER.contains("fraction == 0.5 && u32(lower) % 2u == 1u"));
    }

    fn vertex_at(x: f32, y: f32, z: f32) -> Vertex {
        Vertex {
            position: [x, y, z],
            texture_coordinates: [0.0; 2],
            texture_pan_speeds: [0.0; 4],
            lightmap_coordinates: [0.0; 2],
            has_lightmap: 0.0,
            vertex_color: [255; 4],
            normal: [0.0, 1.0, 0.0],
            environment_map: 0.0,
            lighting_coordinates: [0.0; 2],
            lighting_index: u32::MAX,
            uv_effect_scale: [0.0; 2],
            node_plane_normal: [0.0; 3],
            macro_texture_coordinates: [0.0; 2],
            detail_texture_coordinates: [0.0; 2],
            attachment_flags: [0; 2],
        }
    }
}
