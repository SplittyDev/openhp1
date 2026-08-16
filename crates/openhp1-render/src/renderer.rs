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
mod lighting;
mod modern;
mod pipeline;
mod target;

use crate::camera::{reflected_view, warp_view};
use atlas::{AtlasRectangle, build_lightmap_atlas, lightmap_patch};
use batch::{
    BackdropBatch, BlendedSurface, DrawBatch, backdrop_batches, blended_surfaces,
    mirror_geometries, sorted_blended_batches, texture_batches, update_blended_centers,
};
use classic::ClassicDisplay;
use lighting::ModernLighting;
use modern::{HDR_FORMAT, ModernRenderer};
#[cfg(test)]
use pipeline::{blend_state, fragment_entry};
use pipeline::{create_pipeline, create_screen_pipeline, texture, texture_bind_group};
use target::{DepthTarget, SampledTarget};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_WARP_PORTAL_DEPTH: usize = 3;
const PIPELINES_PER_MODE: usize = 8;
const PIPELINE_COUNT: usize = 32;
const AUTO_UV_PER_SECOND: f32 = 64.0;
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
    small_wavy_scale: [f32; 2],
    node_plane_normal: [f32; 3],
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
    plane: (Vec3, Vec3),
    bounds: (Vec3, Vec3),
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    pipeline: usize,
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
    index_buffer: wgpu::Buffer,
    index_count: u32,
    pipeline: usize,
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
    backdrop_pipelines: [wgpu::RenderPipeline; 2],
    mirror_pipelines: [wgpu::RenderPipeline; 2],
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    sky_camera_buffer: Option<wgpu::Buffer>,
    sky_camera_bind_group: Option<wgpu::BindGroup>,
    textures: Vec<wgpu::Texture>,
    texture_bind_groups: Vec<wgpu::BindGroup>,
    vertices: Vec<Vertex>,
    vertex_buffer: wgpu::Buffer,
    opaque_index_buffer: wgpu::Buffer,
    opaque_batches: Vec<DrawBatch>,
    backdrop_index_buffer: wgpu::Buffer,
    backdrop_batches: Vec<BackdropBatch>,
    mirrors: Vec<Mirror>,
    warp_portals: Vec<WarpPortal>,
    nested_warp_targets: Vec<SampledTarget>,
    blended_index_buffer: wgpu::Buffer,
    sky_blended_index_buffer: Option<wgpu::Buffer>,
    blended_surfaces: Vec<BlendedSurface>,
    depth: DepthTarget,
    classic_display: Option<ClassicDisplay>,
    modern: Option<ModernRenderer>,
    lighting: Option<ModernLighting>,
    sky_target: Option<SampledTarget>,
    bounds: SceneBounds,
    sky_zone: Option<openhp1_scene::SkyZone>,
    target_format: wgpu::TextureFormat,
    texture_layout: wgpu::BindGroupLayout,
    sky_sampler: wgpu::Sampler,
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
                let dimensions = texture.map_or([64.0, 64.0], |texture| {
                    [texture.width as f32, texture.height as f32]
                });
                let coordinates = scene.mesh.texture_coordinates[vertex_index];
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
                    small_wavy_scale: normalized_small_wavy_scale(material.small_wavy, dimensions),
                    node_plane_normal: unreal_to_render(
                        scene
                            .mesh
                            .node_plane_normals
                            .get(vertex_index)
                            .copied()
                            .unwrap_or(Vec3::ZERO),
                    )
                    .to_array(),
                }
            })
            .collect();
        let bounds = scene_bounds(&vertices);
        let (opaque_indices, opaque_batches) = texture_batches(scene, fallback_texture);
        let (backdrop_indices, backdrop_batches) = backdrop_batches(scene);
        let mirror_geometries = mirror_geometries(scene);
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
        let blended_surfaces = blended_surfaces(scene, fallback_texture, &vertices);
        let blended_index_count = blended_surfaces
            .iter()
            .map(|surface| surface.indices.len())
            .sum::<usize>();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 BSP vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let opaque_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 opaque BSP indices"),
            contents: bytemuck::cast_slice(&opaque_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let backdrop_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 fake-backdrop indices"),
            size: (backdrop_indices.len() * size_of::<u32>()).max(size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !backdrop_indices.is_empty() {
            queue.write_buffer(
                &backdrop_index_buffer,
                0,
                bytemuck::cast_slice(&backdrop_indices),
            );
        }
        let blended_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 blended BSP indices"),
            size: (blended_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_blended_index_buffer = scene.sky_zone.map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("OpenHP1 sky blended BSP indices"),
                size: (blended_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
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
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 sky sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
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
                .map(|texture| texture.rgba.len())
                .sum::<usize>()
                + checkerboard.rgba.len(),
            lightmap_memory_bytes: lightmap_atlas.image.rgba.len(),
        };
        let textures = scene
            .textures
            .iter()
            .chain(std::iter::once(&checkerboard))
            .map(|image| texture(device, queue, "OpenHP1 texture", image))
            .collect::<Vec<_>>();
        let texture_bind_groups = textures
            .iter()
            .map(|texture| {
                let view = texture.create_view(&Default::default());
                texture_bind_group(
                    device,
                    &texture_layout,
                    &sampler,
                    &view,
                    &lightmap_view,
                    &lightmap_sampler,
                )
            })
            .collect();
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
        let reflected_pipelines = (!mirror_geometries.is_empty()).then(|| {
            std::array::from_fn(|index| {
                let mode = match index / PIPELINES_PER_MODE {
                    0 => SurfaceMode::Opaque,
                    1 => SurfaceMode::Translucent,
                    2 => SurfaceMode::Modulated,
                    3 => SurfaceMode::AlphaBlended,
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
            )
        });
        let mirror_pipelines = std::array::from_fn(|index| {
            create_screen_pipeline(
                device,
                scene_format,
                &pipeline_layout,
                &shader,
                index != 0,
                "fragment_mirror",
            )
        });
        let sky_target = scene.sky_zone.map(|_| {
            SampledTarget::new(
                device,
                viewport_size,
                scene_format,
                &texture_layout,
                &sky_sampler,
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
                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("OpenHP1 mirror indices"),
                    contents: bytemuck::cast_slice(&geometry.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                Some(Mirror {
                    surface: geometry.surface,
                    plane,
                    bounds: surface_bounds(scene, &vertices, geometry.surface)?,
                    camera_buffer,
                    camera_bind_group,
                    index_buffer,
                    index_count: geometry.indices.len() as u32,
                    pipeline: geometry.pipeline,
                    target: SampledTarget::new(
                        device,
                        viewport_size,
                        scene_format,
                        &texture_layout,
                        &sky_sampler,
                        &lightmap_view,
                        &lightmap_sampler,
                    ),
                })
            })
            .collect();
        let warp_portals = warp_portal_geometries
            .into_iter()
            .filter_map(|(portal, indices)| {
                let plane = mirror_plane(scene, &vertices, portal.surface)?;
                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("OpenHP1 warp-portal indices"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                Some(WarpPortal {
                    surface: portal.surface,
                    authored_plane: portal.plane,
                    source_on_positive_side: portal.source_on_positive_side,
                    source: portal.source,
                    destination: portal.destination,
                    plane,
                    bounds: surface_bounds(scene, &vertices, portal.surface)?,
                    view: PortalView::new(device, &camera_layout, blended_index_count),
                    nested_views: (1..MAX_WARP_PORTAL_DEPTH)
                        .map(|_| PortalView::new(device, &camera_layout, blended_index_count))
                        .collect(),
                    index_buffer,
                    index_count: indices.len() as u32,
                    pipeline: usize::from(scene.surface_materials[portal.surface].two_sided),
                    target: SampledTarget::new(
                        device,
                        viewport_size,
                        scene_format,
                        &texture_layout,
                        &sky_sampler,
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
                            &sky_sampler,
                            &lightmap_view,
                            &lightmap_sampler,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let depth = DepthTarget::new(device, viewport_size, modern_enabled);
        let classic_display =
            (!modern_enabled).then(|| ClassicDisplay::new(device, target_format, viewport_size));
        let modern = modern_enabled.then(|| {
            ModernRenderer::new(
                (device, queue),
                target_format,
                viewport_size,
                settings,
                &depth.view,
                scene,
                &texture_layout,
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
            backdrop_pipelines,
            mirror_pipelines,
            camera_buffer,
            camera_bind_group,
            sky_camera_buffer,
            sky_camera_bind_group,
            textures,
            texture_bind_groups,
            vertices,
            vertex_buffer,
            opaque_index_buffer,
            opaque_batches,
            backdrop_index_buffer,
            backdrop_batches,
            mirrors,
            warp_portals,
            nested_warp_targets,
            blended_index_buffer,
            sky_blended_index_buffer,
            blended_surfaces,
            depth,
            classic_display,
            modern,
            lighting,
            sky_target,
            bounds,
            sky_zone: scene.sky_zone,
            target_format,
            texture_layout,
            sky_sampler,
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
        update_blended_centers(&mut self.blended_surfaces, &self.vertices);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        true
    }

    pub fn update_scene(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        if scene.textures.len() + 1 != self.textures.len() || !self.update_vertices(queue, scene) {
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
        *self = renderer;
    }

    pub fn update_textures(
        &mut self,
        queue: &wgpu::Queue,
        images: &[TextureImage],
        changed: &[usize],
    ) -> bool {
        if images.len() + 1 != self.textures.len() {
            return false;
        }
        for &index in changed {
            let (Some(texture), Some(image)) = (self.textures.get(index), images.get(index)) else {
                return false;
            };
            let expected = usize::try_from(image.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(image.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4));
            let Some(bytes_per_row) = image.width.checked_mul(4) else {
                return false;
            };
            if texture.width() != image.width
                || texture.height() != image.height
                || expected != Some(image.rgba.len())
            {
                return false;
            }
            queue.write_texture(
                texture.as_image_copy(),
                &image.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(image.height),
                },
                texture.size(),
            );
        }
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
                    &self.sky_sampler,
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
                    &self.sky_sampler,
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
                    &self.sky_sampler,
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
                    &self.sky_sampler,
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
        let (blended_indices, blended_batches) =
            sorted_blended_batches(&self.blended_surfaces, camera.position);
        if !blended_indices.is_empty() {
            queue.write_buffer(
                &self.blended_index_buffer,
                0,
                bytemuck::cast_slice(&blended_indices),
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
            let (sky_indices, sky_batches) =
                sorted_blended_batches(&self.blended_surfaces, sky_camera.position);
            if !sky_indices.is_empty() {
                queue.write_buffer(
                    sky_blended_index_buffer,
                    0,
                    bytemuck::cast_slice(&sky_indices),
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
                &sky_batches,
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
                    let (indices, batches) =
                        sorted_blended_batches(&self.blended_surfaces, nested_position);
                    if !indices.is_empty() {
                        queue.write_buffer(
                            &view.blended_index_buffer,
                            0,
                            bytemuck::cast_slice(&indices),
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
                        &batches,
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
                let (indices, batches) =
                    sorted_blended_batches(&self.blended_surfaces, reflected_position);
                if !indices.is_empty() {
                    queue.write_buffer(
                        &view.blended_index_buffer,
                        0,
                        bytemuck::cast_slice(&indices),
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
                    &batches,
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
            let (indices, batches) = sorted_blended_batches(&self.blended_surfaces, position);
            if !indices.is_empty() {
                queue.write_buffer(
                    &view.blended_index_buffer,
                    0,
                    bytemuck::cast_slice(&indices),
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
                &batches,
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
                &self.blended_index_buffer,
                &[],
                None,
                None,
                None,
                ScenePass::Reflection,
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
                    load: wgpu::LoadOp::Clear(clear_color()),
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
            &blended_batches,
            self.sky_target.as_ref().map(|target| &target.bind_group),
            None,
            None,
            ScenePass::Main,
        );
        if let Some(modern) = &self.modern {
            draw_calls += modern.draw_scene_effects(&mut pass, &self.texture_bind_groups);
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
            classic_display.render(queue, encoder, target, display.brightness);
            draw_calls += 1;
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
        blended_index_buffer: &'pass wgpu::Buffer,
        blended_batches: &[DrawBatch],
        backdrop_bind_group: Option<&'pass wgpu::BindGroup>,
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
        pass.set_bind_group(0, camera_bind_group, &[]);
        if let Some(lighting) = &self.lighting {
            pass.set_bind_group(2, &lighting.bind_group, &[]);
        }
        if !self.opaque_batches.is_empty()
            || !blended_batches.is_empty()
            || (backdrop_bind_group.is_some() && !self.backdrop_batches.is_empty())
            || nested_warp_portal.is_some()
            || nested_mirror.is_some()
            || (matches!(scene_pass, ScenePass::Main) && !self.warp_portals.is_empty())
            || (matches!(scene_pass, ScenePass::Main) && !self.mirrors.is_empty())
        {
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        }
        if matches!(scene_pass, ScenePass::Main) {
            for portal in self
                .warp_portals
                .iter()
                .filter(|portal| portal.destination.is_some())
            {
                pass.set_index_buffer(portal.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_bind_group(1, &portal.target.bind_group, &[]);
                pass.set_pipeline(&self.mirror_pipelines[portal.pipeline]);
                pass.draw_indexed(0..portal.index_count, 0, 0..1);
                draw_calls += 1;
            }
        }
        if let Some((portal, target)) = nested_warp_portal {
            pass.set_index_buffer(portal.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_bind_group(1, &target.bind_group, &[]);
            pass.set_pipeline(&self.mirror_pipelines[portal.pipeline]);
            pass.draw_indexed(0..portal.index_count, 0, 0..1);
            draw_calls += 1;
        }
        if let Some(mirror) = nested_mirror {
            pass.set_index_buffer(mirror.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_bind_group(1, &mirror.target.bind_group, &[]);
            pass.set_pipeline(&self.mirror_pipelines[mirror.pipeline]);
            pass.draw_indexed(0..mirror.index_count, 0, 0..1);
            draw_calls += 1;
        }
        if !self.opaque_batches.is_empty() {
            pass.set_index_buffer(
                self.opaque_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for batch in &self.opaque_batches {
                pass.set_pipeline(&pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                draw_calls += 1;
            }
        }
        if matches!(scene_pass, ScenePass::Main) {
            for mirror in &self.mirrors {
                pass.set_index_buffer(mirror.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_bind_group(1, &mirror.target.bind_group, &[]);
                pass.set_pipeline(&self.mirror_pipelines[mirror.pipeline]);
                pass.draw_indexed(0..mirror.index_count, 0, 0..1);
                draw_calls += 1;
            }
        }
        if let Some(backdrop_bind_group) = backdrop_bind_group
            && !self.backdrop_batches.is_empty()
        {
            pass.set_index_buffer(
                self.backdrop_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.set_bind_group(1, backdrop_bind_group, &[]);
            for batch in &self.backdrop_batches {
                pass.set_pipeline(&self.backdrop_pipelines[batch.pipeline]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                draw_calls += 1;
            }
        }
        if !blended_batches.is_empty() {
            pass.set_index_buffer(blended_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in blended_batches {
                pass.set_pipeline(&pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                draw_calls += 1;
            }
        }
        draw_calls
    }
}

fn quantized_flash(flash: [f32; 4]) -> [f32; 4] {
    flash.map(|component| {
        ((component * 256.0 - 0.5)
            .round_ties_even()
            .clamp(0.0, 255.0))
            / 255.0
    })
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
        rgba: vec![
            255, 0, 255, 255, 24, 24, 24, 255, 24, 24, 24, 255, 255, 0, 255, 255,
        ],
    }
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
            include_str!("shaders/modern/corona.wgsl"),
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
        let corona_shader = include_str!("shaders/modern/corona.wgsl");
        assert!(shader.contains("const WHITE = 1.25;"));
        assert!(shader.contains("const LUMINANCE = vec3(0.2126, 0.7152, 0.0722);"));
        assert!(shader.contains("let luminance = dot(color, LUMINANCE);"));
        assert!(shader.contains("return color * (mapped_luminance / luminance);"));
        assert!(shader.contains("let encoded = srgb_encode(clamp(mapped"));
        assert!(shader.contains("let contrasted = display_contrast(encoded);"));
        assert!(modern::BLOOM_SHADER.contains("const THRESHOLD = 1.0;"));
        assert!(modern::BLOOM_SHADER.contains("const KNEE = 0.1;"));
        assert!(shader.contains("textureLoad(ao_texture"));
        assert!(shader.contains("scene.a >= 0.5"));
        assert!(scene_shader.contains("vec4(color.rgb, 0.0)"));
        assert!(scene_shader.contains("min(light.color.rgb * strength, vec3(1.0))"));
        assert!(scene_shader.contains("srgb_to_linear(color.rgb * illumination * 2.0)"));
        assert!(scene_shader.contains("srgb_to_linear(color.rgb * input.vertex_color.rgb)"));
        assert!(corona_shader.contains("corner * vec2(1.6, 1.6 * aspect) * color_and_scale.w;"));
        assert!(corona_shader.contains("srgb_to_linear(lit) * CORONA_HDR_GAIN"));
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
            "output.texture_coordinates = texture_coordinates + texture_pan_speed * camera.auto_uv.x;\n        if any(small_wavy_scale != vec2(0.0)) {"
        ));
        assert!(shader.contains("let time = camera.auto_uv.x / 64.0;"));
        assert!(shader.contains("8.0 * sin(time) + 4.0 * cos(2.3 * time),"));
        assert!(shader.contains("8.0 * cos(time) + 4.0 * sin(2.3 * time),"));
        assert!(shader.contains(
            "output.texture_coordinates = output.texture_coordinates\n                + small_wavy_scale * small_wavy_offset;"
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
                small_wavy_scale: [0.0; 2],
                node_plane_normal: [0.0; 3],
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
                small_wavy_scale: [0.0; 2],
                node_plane_normal: [0.0; 3],
            },
        ];
        let bounds = scene_bounds(&vertices);
        assert_eq!(bounds.minimum, Vec3::new(-2.0, -1.0, 1.0));
        assert_eq!(bounds.maximum, Vec3::new(4.0, 3.0, 7.0));
    }

    #[test]
    fn batches_by_texture_and_material_while_skipping_hidden_surfaces() {
        let scene = RenderScene {
            mesh: TriangleMesh {
                indices: (0..12).collect(),
                triangle_surfaces: vec![0, 1, 2, 3],
                ..Default::default()
            },
            textures: vec![checkerboard(), checkerboard()],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![],
            surface_materials: vec![
                SurfaceMaterial {
                    texture: Some(1),
                    masked: true,
                    two_sided: true,
                    ..Default::default()
                },
                SurfaceMaterial::default(),
                SurfaceMaterial {
                    texture: Some(0),
                    ..Default::default()
                },
                SurfaceMaterial {
                    mode: SurfaceMode::Hidden,
                    ..Default::default()
                },
            ],
            warp_portals: vec![],
            sky_zone: None,
        };
        let (indices, batches) = texture_batches(&scene, 2);
        assert_eq!(indices, [6, 7, 8, 0, 1, 2, 3, 4, 5]);
        assert_eq!(
            batches
                .iter()
                .map(|batch| (batch.texture, batch.pipeline, batch.indices.clone()))
                .collect::<Vec<_>>(),
            [(0, 0, 0..3), (1, 3, 3..6), (2, 0, 6..9)]
        );
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
            warp_portals: vec![],
            sky_zone: None,
        };

        assert!(texture_batches(&scene, 0).0.is_empty());
        let geometries = mirror_geometries(&scene);
        assert_eq!(geometries.len(), 2);
        assert_eq!(geometries[0].surface, 0);
        assert_eq!(geometries[0].indices, [0, 1, 2]);
        assert_eq!(geometries[0].pipeline, 0);
        assert_eq!(geometries[1].surface, 1);
        assert_eq!(geometries[1].indices, [3, 4, 5]);
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
    fn extracts_and_sorts_blended_surfaces_by_camera_distance() {
        let vertices = [
            vertex_at(0.0, 0.0, 10.0),
            vertex_at(1.0, 0.0, 10.0),
            vertex_at(0.0, 1.0, 10.0),
            vertex_at(0.0, 0.0, 1.0),
            vertex_at(1.0, 0.0, 1.0),
            vertex_at(0.0, 1.0, 1.0),
        ];
        let scene = RenderScene {
            mesh: TriangleMesh {
                indices: (0..6).collect(),
                triangle_surfaces: vec![0, 1],
                ..Default::default()
            },
            textures: vec![checkerboard(), checkerboard()],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![],
            surface_materials: vec![
                SurfaceMaterial {
                    texture: Some(0),
                    mode: SurfaceMode::Translucent,
                    ..Default::default()
                },
                SurfaceMaterial {
                    texture: Some(1),
                    mode: SurfaceMode::Modulated,
                    masked: true,
                    ..Default::default()
                },
            ],
            warp_portals: vec![],
            sky_zone: None,
        };
        let surfaces = blended_surfaces(&scene, 2, &vertices);
        let (indices, batches) = sorted_blended_batches(&surfaces, Vec3::ZERO);
        assert_eq!(indices, [3, 4, 5, 0, 1, 2]);
        assert_eq!(
            batches
                .iter()
                .map(|batch| (batch.texture, batch.pipeline, batch.indices.clone()))
                .collect::<Vec<_>>(),
            [(1, 18, 0..3), (0, 8, 3..6)]
        );
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
    fn environment_maps_are_opt_in_and_use_surreal_reflection_coordinates() {
        assert!(!SurfaceMaterial::default().environment_map);
        assert_eq!(SurfaceMaterial::default().opacity, 1.0);
        assert!(
            SurfaceMaterial {
                environment_map: true,
                ..Default::default()
            }
            .environment_map
        );

        let coordinates = |position: Vec3, normal: Vec3, camera: Vec3| {
            let incident = (position - camera).normalize_or_zero();
            let reflection = incident - 2.0 * incident.dot(normal) * normal;
            (reflection.truncate() + glam::Vec2::ONE) * (128.0 / 255.0)
        };
        let centered = coordinates(-Vec3::Z, Vec3::Z, Vec3::ZERO);
        assert!(centered.abs_diff_eq(glam::Vec2::splat(128.0 / 255.0), 0.000_001));
        let camera_relative = coordinates(-Vec3::Z, Vec3::Z, Vec3::new(-1.0, 0.0, -1.0));
        assert!(
            camera_relative.abs_diff_eq(glam::Vec2::new(256.0 / 255.0, 128.0 / 255.0), 0.000_001)
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
    fn extracts_only_fake_backdrop_triangles() {
        let scene = RenderScene {
            mesh: TriangleMesh {
                indices: (0..9).collect(),
                triangle_surfaces: vec![0, 1, 2],
                ..Default::default()
            },
            textures: vec![],
            lightmaps: vec![],
            realtime_lightmaps: vec![],
            coronas: vec![],
            surface_materials: vec![
                SurfaceMaterial::default(),
                SurfaceMaterial {
                    mode: SurfaceMode::Backdrop,
                    ..Default::default()
                },
                SurfaceMaterial {
                    mode: SurfaceMode::Backdrop,
                    two_sided: true,
                    ..Default::default()
                },
            ],
            warp_portals: vec![],
            sky_zone: None,
        };
        let (indices, batches) = backdrop_batches(&scene);
        assert_eq!(indices, [3, 4, 5, 6, 7, 8]);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].indices, 0..3);
        assert_eq!(batches[1].indices, 3..6);
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
            small_wavy_scale: [0.0; 2],
            node_plane_normal: [0.0; 3],
        }
    }
}
