use std::{mem::size_of, ops::Range};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::{
    Camera, RenderScene, SceneBounds, SurfaceMaterial, SurfaceMode, TextureImage, unreal_to_render,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const PIPELINES_PER_MODE: usize = 8;
const PIPELINE_COUNT: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    texture_coordinates: [f32; 2],
    lightmap_coordinates: [f32; 2],
    has_lightmap: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
}

pub struct Renderer {
    pipelines: [wgpu::RenderPipeline; PIPELINE_COUNT],
    backdrop_pipelines: [wgpu::RenderPipeline; 2],
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    sky_camera_buffer: Option<wgpu::Buffer>,
    sky_camera_bind_group: Option<wgpu::BindGroup>,
    texture_bind_groups: Vec<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    opaque_index_buffer: wgpu::Buffer,
    opaque_batches: Vec<DrawBatch>,
    backdrop_index_buffer: wgpu::Buffer,
    backdrop_batches: Vec<BackdropBatch>,
    blended_index_buffer: wgpu::Buffer,
    sky_blended_index_buffer: Option<wgpu::Buffer>,
    blended_surfaces: Vec<BlendedSurface>,
    depth: DepthTarget,
    sky_target: Option<SkyTarget>,
    bounds: SceneBounds,
    sky_zone: Option<openhp1_map::SkyZone>,
    target_format: wgpu::TextureFormat,
    texture_layout: wgpu::BindGroupLayout,
    sky_sampler: wgpu::Sampler,
    lightmap_view: wgpu::TextureView,
    lightmap_sampler: wgpu::Sampler,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        scene: &RenderScene,
        viewport_size: [u32; 2],
    ) -> Self {
        let lightmap_atlas =
            build_lightmap_atlas(&scene.lightmaps, device.limits().max_texture_dimension_2d);
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
                let texture = scene
                    .surface_materials
                    .get(surface)
                    .and_then(|material| material.texture)
                    .and_then(|index| scene.textures.get(index));
                let dimensions = texture.map_or([64.0, 64.0], |texture| {
                    [texture.width as f32, texture.height as f32]
                });
                let coordinates = scene.mesh.texture_coordinates[vertex_index];
                let lightmap_rectangle = scene
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
                    })
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
                Vertex {
                    position: position.to_array(),
                    texture_coordinates: [
                        coordinates.x / dimensions[0],
                        coordinates.y / dimensions[1],
                    ],
                    lightmap_coordinates,
                    has_lightmap: f32::from(lightmap_rectangle.is_some()),
                }
            })
            .collect();
        let bounds = scene_bounds(&vertices);
        let (opaque_indices, opaque_batches) = texture_batches(scene, fallback_texture);
        let (backdrop_indices, backdrop_batches) = backdrop_batches(scene);
        let blended_surfaces = blended_surfaces(scene, fallback_texture, &vertices);
        let blended_index_count = blended_surfaces
            .iter()
            .map(|surface| surface.indices.len())
            .sum::<usize>();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 BSP vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
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
        let lightmap_view = texture_view(
            device,
            queue,
            "OpenHP1 lightmap atlas",
            &lightmap_atlas.image,
        );
        let checkerboard = checkerboard();
        let texture_bind_groups = scene
            .textures
            .iter()
            .chain(std::iter::once(&checkerboard))
            .map(|texture| {
                texture_bind_group(
                    device,
                    queue,
                    &texture_layout,
                    &sampler,
                    texture,
                    &lightmap_view,
                    &lightmap_sampler,
                )
            })
            .collect();
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 BSP pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|index| {
            let mode = match index / PIPELINES_PER_MODE {
                0 => SurfaceMode::Opaque,
                1 => SurfaceMode::Translucent,
                2 => SurfaceMode::Modulated,
                _ => unreachable!(),
            };
            create_pipeline(
                device,
                target_format,
                &pipeline_layout,
                &shader,
                SurfaceMaterial {
                    mode,
                    masked: index % 4 >= 2,
                    two_sided: index % 2 != 0,
                    unlit: index % PIPELINES_PER_MODE >= 4,
                    ..Default::default()
                },
            )
        });
        let backdrop_pipelines = std::array::from_fn(|index| {
            create_backdrop_pipeline(device, target_format, &pipeline_layout, &shader, index != 0)
        });
        let sky_target = scene.sky_zone.map(|_| {
            SkyTarget::new(
                device,
                viewport_size,
                target_format,
                &texture_layout,
                &sky_sampler,
                &lightmap_view,
                &lightmap_sampler,
            )
        });

        Self {
            pipelines,
            backdrop_pipelines,
            camera_buffer,
            camera_bind_group,
            sky_camera_buffer,
            sky_camera_bind_group,
            texture_bind_groups,
            vertex_buffer,
            opaque_index_buffer,
            opaque_batches,
            backdrop_index_buffer,
            backdrop_batches,
            blended_index_buffer,
            sky_blended_index_buffer,
            blended_surfaces,
            depth: DepthTarget::new(device, viewport_size),
            sky_target,
            bounds,
            sky_zone: scene.sky_zone,
            target_format,
            texture_layout,
            sky_sampler,
            lightmap_view,
            lightmap_sampler,
        }
    }

    pub fn bounds(&self) -> SceneBounds {
        self.bounds
    }

    pub fn resize(&mut self, device: &wgpu::Device, viewport_size: [u32; 2]) {
        if self.depth.size != viewport_size {
            self.depth = DepthTarget::new(device, viewport_size);
            if self.sky_target.is_some() {
                self.sky_target = Some(SkyTarget::new(
                    device,
                    viewport_size,
                    self.target_format,
                    &self.texture_layout,
                    &self.sky_sampler,
                    &self.lightmap_view,
                    &self.lightmap_sampler,
                ));
            }
        }
    }

    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        camera: &Camera,
        viewport_size: [u32; 2],
    ) {
        let aspect = viewport_size[0] as f32 / viewport_size[1] as f32;
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
            }),
        );
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
            self.draw_scene(
                &mut pass,
                sky_camera_bind_group,
                sky_blended_index_buffer,
                &sky_batches,
                None,
            );
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 BSP render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        self.draw_scene(
            &mut pass,
            &self.camera_bind_group,
            &self.blended_index_buffer,
            &blended_batches,
            self.sky_target.as_ref().map(|target| &target.bind_group),
        );
    }

    fn draw_scene<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        camera_bind_group: &'pass wgpu::BindGroup,
        blended_index_buffer: &'pass wgpu::Buffer,
        blended_batches: &[DrawBatch],
        backdrop_bind_group: Option<&'pass wgpu::BindGroup>,
    ) {
        pass.set_bind_group(0, camera_bind_group, &[]);
        if !self.opaque_batches.is_empty()
            || !blended_batches.is_empty()
            || (backdrop_bind_group.is_some() && !self.backdrop_batches.is_empty())
        {
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        }
        if !self.opaque_batches.is_empty() {
            pass.set_index_buffer(
                self.opaque_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for batch in &self.opaque_batches {
                pass.set_pipeline(&self.pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
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
            }
        }
        if !blended_batches.is_empty() {
            pass.set_index_buffer(blended_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in blended_batches {
                pass.set_pipeline(&self.pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
            }
        }
    }
}

fn clear_color() -> wgpu::Color {
    wgpu::Color {
        r: 0.035,
        g: 0.045,
        b: 0.065,
        a: 1.0,
    }
}

struct DrawBatch {
    indices: Range<u32>,
    texture: usize,
    pipeline: usize,
}

struct BackdropBatch {
    indices: Range<u32>,
    pipeline: usize,
}

struct BlendedSurface {
    indices: Vec<u32>,
    center: Vec3,
    texture: usize,
    pipeline: usize,
}

fn backdrop_batches(scene: &RenderScene) -> (Vec<u32>, Vec<BackdropBatch>) {
    let mut buckets = [Vec::new(), Vec::new()];
    for (triangle, &surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let Some(material) = scene.surface_materials.get(surface) else {
            continue;
        };
        if material.mode == SurfaceMode::Backdrop {
            buckets[usize::from(material.two_sided)].extend_from_slice(triangle);
        }
    }

    let mut indices = Vec::new();
    let mut batches = Vec::new();
    for (pipeline, bucket) in buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        let start = indices.len() as u32;
        indices.extend(bucket);
        batches.push(BackdropBatch {
            indices: start..indices.len() as u32,
            pipeline,
        });
    }
    (indices, batches)
}

fn texture_batches(scene: &RenderScene, fallback_texture: usize) -> (Vec<u32>, Vec<DrawBatch>) {
    let mut buckets = vec![Vec::new(); (fallback_texture + 1) * PIPELINES_PER_MODE];
    for (triangle, surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let material = scene
            .surface_materials
            .get(*surface)
            .copied()
            .unwrap_or_default();
        if material.mode != SurfaceMode::Opaque {
            continue;
        }
        let texture = material
            .texture
            .filter(|index| *index < fallback_texture)
            .unwrap_or(fallback_texture);
        let pipeline = pipeline_index(material);
        buckets[texture * PIPELINES_PER_MODE + pipeline].extend_from_slice(triangle);
    }

    let mut indices = Vec::with_capacity(scene.mesh.indices.len());
    let mut batches = Vec::new();
    for (bucket, source) in buckets.into_iter().enumerate() {
        if source.is_empty() {
            continue;
        }
        let start = indices.len() as u32;
        indices.extend(source);
        batches.push(DrawBatch {
            indices: start..indices.len() as u32,
            texture: bucket / PIPELINES_PER_MODE,
            pipeline: bucket % PIPELINES_PER_MODE,
        });
    }
    (indices, batches)
}

fn blended_surfaces(
    scene: &RenderScene,
    fallback_texture: usize,
    vertices: &[Vertex],
) -> Vec<BlendedSurface> {
    let mut indices = vec![Vec::new(); scene.surface_materials.len()];
    let mut center_sums = vec![Vec3::ZERO; scene.surface_materials.len()];
    let mut triangle_counts = vec![0_u32; scene.surface_materials.len()];

    for (triangle, &surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let Some(material) = scene.surface_materials.get(surface).copied() else {
            continue;
        };
        if !matches!(
            material.mode,
            SurfaceMode::Translucent | SurfaceMode::Modulated
        ) {
            continue;
        }
        indices[surface].extend_from_slice(triangle);
        center_sums[surface] += triangle
            .iter()
            .map(|&index| Vec3::from_array(vertices[index as usize].position))
            .sum::<Vec3>()
            / 3.0;
        triangle_counts[surface] += 1;
    }

    indices
        .into_iter()
        .enumerate()
        .filter_map(|(surface, indices)| {
            if indices.is_empty() {
                return None;
            }
            let material = scene.surface_materials[surface];
            Some(BlendedSurface {
                indices,
                center: center_sums[surface] / triangle_counts[surface] as f32,
                texture: material
                    .texture
                    .filter(|index| *index < fallback_texture)
                    .unwrap_or(fallback_texture),
                pipeline: pipeline_index(material),
            })
        })
        .collect()
}

fn sorted_blended_batches(
    surfaces: &[BlendedSurface],
    camera_position: Vec3,
) -> (Vec<u32>, Vec<DrawBatch>) {
    let mut sorted = surfaces.iter().collect::<Vec<_>>();
    // Match UE1's translucent-node pass: closest surface origins first.
    sorted.sort_by(|left, right| {
        left.center
            .distance_squared(camera_position)
            .total_cmp(&right.center.distance_squared(camera_position))
    });

    let mut indices = Vec::new();
    let mut batches: Vec<DrawBatch> = Vec::new();
    for surface in sorted {
        let start = indices.len() as u32;
        indices.extend_from_slice(&surface.indices);
        let end = indices.len() as u32;
        if let Some(batch) = batches.last_mut()
            && batch.texture == surface.texture
            && batch.pipeline == surface.pipeline
        {
            batch.indices.end = end;
        } else {
            batches.push(DrawBatch {
                indices: start..end,
                texture: surface.texture,
                pipeline: surface.pipeline,
            });
        }
    }
    (indices, batches)
}

fn pipeline_index(material: SurfaceMaterial) -> usize {
    let mode = match material.mode {
        SurfaceMode::Opaque | SurfaceMode::Backdrop | SurfaceMode::Hidden => 0,
        SurfaceMode::Translucent => 1,
        SurfaceMode::Modulated => 2,
    };
    mode * PIPELINES_PER_MODE
        + usize::from(material.unlit) * 4
        + usize::from(material.masked) * 2
        + usize::from(material.two_sided)
}

fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    material: SurfaceMaterial,
) -> wgpu::RenderPipeline {
    let blended = matches!(
        material.mode,
        SurfaceMode::Translucent | SurfaceMode::Modulated
    );
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 BSP pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x2,
                    2 => Float32x2,
                    3 => Float32
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            // The Unreal-to-render axis conversion changes handedness, so UE
            // polygon winding becomes clockwise in render space.
            front_face: wgpu::FrontFace::Cw,
            cull_mode: (!material.two_sided).then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!blended),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry(
                material.mode,
                material.masked,
                material.unlit,
            )),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: blend_state(material.mode),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_backdrop_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    two_sided: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 fake-backdrop pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x2,
                    2 => Float32x2,
                    3 => Float32
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Cw,
            cull_mode: (!two_sided).then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            // The original BSP visibility pass excludes geometry behind the
            // portal. Writing the backdrop plane provides the same occlusion
            // until node/zone visibility traversal is implemented.
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_backdrop"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn fragment_entry(mode: SurfaceMode, masked: bool, unlit: bool) -> &'static str {
    match (mode, masked, unlit) {
        (SurfaceMode::Opaque, false, false) => "fragment_main",
        (SurfaceMode::Opaque, true, false) => "fragment_masked",
        (SurfaceMode::Opaque, false, true) => "fragment_unlit",
        (SurfaceMode::Opaque, true, true) => "fragment_unlit_masked",
        (SurfaceMode::Translucent | SurfaceMode::Modulated, false, _) => "fragment_blended",
        (SurfaceMode::Translucent | SurfaceMode::Modulated, true, _) => "fragment_blended_masked",
        (SurfaceMode::Backdrop | SurfaceMode::Hidden, _, _) => unreachable!(),
    }
}

fn blend_state(mode: SurfaceMode) -> Option<wgpu::BlendState> {
    let color = match mode {
        SurfaceMode::Opaque => return None,
        SurfaceMode::Translucent => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        },
        SurfaceMode::Modulated => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Dst,
            dst_factor: wgpu::BlendFactor::Src,
            operation: wgpu::BlendOperation::Add,
        },
        SurfaceMode::Backdrop | SurfaceMode::Hidden => unreachable!(),
    };
    Some(wgpu::BlendState {
        color,
        alpha: color,
    })
}

fn texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &TextureImage,
    lightmap_view: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = texture_view(device, queue, "OpenHP1 texture", image);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(lightmap_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
        ],
    })
}

fn texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &TextureImage,
) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Keep palette bytes unconverted for UE1's brightness-based blend
                // equations. Opaque shaders perform the sRGB conversion explicitly.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &image.rgba,
        )
        .create_view(&Default::default())
}

#[derive(Clone, Copy, Debug, Default)]
struct AtlasRectangle {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct LightmapAtlas {
    image: TextureImage,
    rectangles: Vec<AtlasRectangle>,
    neutral: AtlasRectangle,
}

impl LightmapAtlas {
    fn neutral_coordinates(&self) -> [f32; 2] {
        [
            (self.neutral.x as f32 + 0.5) / self.image.width as f32,
            (self.neutral.y as f32 + 0.5) / self.image.height as f32,
        ]
    }
}

#[derive(Clone, Copy)]
struct AtlasItem {
    source: Option<usize>,
    width: u32,
    height: u32,
}

fn build_lightmap_atlas(
    lightmaps: &[openhp1_map::LightmapImage],
    maximum_dimension: u32,
) -> LightmapAtlas {
    let mut items = Vec::with_capacity(lightmaps.len() + 1);
    items.push(AtlasItem {
        source: None,
        width: 1,
        height: 1,
    });
    items.extend(
        lightmaps
            .iter()
            .enumerate()
            .map(|(source, image)| AtlasItem {
                source: Some(source),
                width: image.width,
                height: image.height,
            }),
    );
    items.sort_unstable_by_key(|item| std::cmp::Reverse(item.height));

    let widest = items.iter().map(|item| item.width + 2).max().unwrap_or(3);
    let mut atlas_width = widest.next_power_of_two().max(512).min(maximum_dimension);
    let (placements, atlas_height) = loop {
        if let Some(result) = pack_atlas(&items, atlas_width, maximum_dimension) {
            break result;
        }
        assert!(
            atlas_width < maximum_dimension,
            "lightmaps exceed the GPU's {maximum_dimension}px texture limit"
        );
        atlas_width = (atlas_width * 2).min(maximum_dimension);
    };

    let mut rgba = vec![128; atlas_width as usize * atlas_height as usize * 4];
    for alpha in rgba.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    let mut rectangles = vec![AtlasRectangle::default(); lightmaps.len()];
    let mut neutral = AtlasRectangle::default();
    for (item, rectangle) in items.iter().zip(placements) {
        match item.source {
            Some(source) => {
                copy_with_gutter(&mut rgba, atlas_width, rectangle, &lightmaps[source].rgba);
                rectangles[source] = rectangle;
            }
            None => neutral = rectangle,
        }
    }
    LightmapAtlas {
        image: TextureImage {
            width: atlas_width,
            height: atlas_height,
            rgba,
        },
        rectangles,
        neutral,
    }
}

fn pack_atlas(
    items: &[AtlasItem],
    atlas_width: u32,
    maximum_height: u32,
) -> Option<(Vec<AtlasRectangle>, u32)> {
    let mut placements = Vec::with_capacity(items.len());
    let (mut x, mut y, mut row_height) = (0, 0, 0);
    for item in items {
        let padded_width = item.width + 2;
        let padded_height = item.height + 2;
        if padded_width > atlas_width {
            return None;
        }
        if x + padded_width > atlas_width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        if y + padded_height > maximum_height {
            return None;
        }
        placements.push(AtlasRectangle {
            x: x + 1,
            y: y + 1,
            width: item.width,
            height: item.height,
        });
        x += padded_width;
        row_height = row_height.max(padded_height);
    }
    Some((placements, (y + row_height).max(1)))
}

fn copy_with_gutter(atlas: &mut [u8], atlas_width: u32, rectangle: AtlasRectangle, source: &[u8]) {
    for target_y in rectangle.y - 1..=rectangle.y + rectangle.height {
        let source_y = target_y
            .saturating_sub(rectangle.y)
            .min(rectangle.height - 1);
        for target_x in rectangle.x - 1..=rectangle.x + rectangle.width {
            let source_x = target_x
                .saturating_sub(rectangle.x)
                .min(rectangle.width - 1);
            let source_offset = ((source_y * rectangle.width + source_x) * 4) as usize;
            let target_offset = ((target_y * atlas_width + target_x) * 4) as usize;
            atlas[target_offset..target_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
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

struct DepthTarget {
    view: wgpu::TextureView,
    size: [u32; 2],
}

struct SkyTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    depth: DepthTarget,
}

impl SkyTarget {
    fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        lightmap_view: &wgpu::TextureView,
        lightmap_sampler: &wgpu::Sampler,
    ) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 sky-zone color"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 sky-zone texture bind group"),
            layout: texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(lightmap_sampler),
                },
            ],
        });
        Self {
            _texture: texture,
            view,
            bind_group,
            depth: DepthTarget::new(device, size),
        }
    }
}

impl DepthTarget {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 depth"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            size,
        }
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

#[cfg(test)]
mod tests {
    use openhp1_map::TriangleMesh;

    use super::*;

    #[test]
    fn computes_scene_bounds() {
        let vertices = [
            Vertex {
                position: [-2.0, 3.0, 1.0],
                texture_coordinates: [0.0; 2],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
            },
            Vertex {
                position: [4.0, -1.0, 7.0],
                texture_coordinates: [0.0; 2],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
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
        assert_eq!(
            fragment_entry(SurfaceMode::Modulated, true, false),
            "fragment_blended_masked"
        );
        assert_eq!(
            fragment_entry(SurfaceMode::Opaque, false, true),
            "fragment_unlit"
        );
    }

    #[test]
    fn lightmap_atlas_replicates_edge_texels_into_gutters() {
        let atlas = build_lightmap_atlas(
            &[openhp1_map::LightmapImage {
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
            lightmap_coordinates: [0.0; 2],
            has_lightmap: 0.0,
        }
    }
}
