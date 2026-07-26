use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use openhp1_scene::TriangleMesh;
use wgpu::util::DeviceExt;

use crate::{
    Camera, RenderScene, SceneBounds, SurfaceMaterial, SurfaceMode, TextureImage, unreal_to_render,
};

mod atlas;
mod batch;
mod pipeline;
mod target;

use atlas::build_lightmap_atlas;
use batch::{
    BackdropBatch, BlendedSurface, DrawBatch, backdrop_batches, blended_surfaces,
    sorted_blended_batches, texture_batches, update_blended_centers,
};
#[cfg(test)]
use pipeline::{blend_state, fragment_entry};
use pipeline::{
    create_backdrop_pipeline, create_pipeline, texture, texture_bind_group, texture_view,
};
use target::{DepthTarget, SkyTarget};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const PIPELINES_PER_MODE: usize = 8;
const PIPELINE_COUNT: usize = 24;

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
    lightmap_coordinates: [f32; 2],
    has_lightmap: f32,
    vertex_color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    display_gamma: [f32; 4],
}

pub struct Renderer {
    pipelines: [wgpu::RenderPipeline; PIPELINE_COUNT],
    backdrop_pipelines: [wgpu::RenderPipeline; 2],
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
    blended_index_buffer: wgpu::Buffer,
    sky_blended_index_buffer: Option<wgpu::Buffer>,
    blended_surfaces: Vec<BlendedSurface>,
    depth: DepthTarget,
    sky_target: Option<SkyTarget>,
    bounds: SceneBounds,
    sky_zone: Option<openhp1_scene::SkyZone>,
    target_format: wgpu::TextureFormat,
    texture_layout: wgpu::BindGroupLayout,
    sky_sampler: wgpu::Sampler,
    lightmap_view: wgpu::TextureView,
    lightmap_sampler: wgpu::Sampler,
    stats: RenderStats,
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
                    vertex_color: pack_vertex_color(
                        scene
                            .mesh
                            .vertex_colors
                            .get(vertex_index)
                            .copied()
                            .unwrap_or(Vec3::ONE),
                    ),
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
        let lightmap_view = texture_view(
            device,
            queue,
            "OpenHP1 lightmap atlas",
            &lightmap_atlas.image,
        );
        let checkerboard = checkerboard();
        let stats = RenderStats {
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
            textures,
            texture_bind_groups,
            vertices,
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
            stats,
        }
    }

    pub fn bounds(&self) -> SceneBounds {
        self.bounds
    }

    pub fn update_vertices(&mut self, queue: &wgpu::Queue, mesh: &TriangleMesh) -> bool {
        if mesh.positions.len() != self.vertices.len() {
            return false;
        }
        for (index, vertex) in self.vertices.iter_mut().enumerate() {
            vertex.position = unreal_to_render(mesh.positions[index]).to_array();
            vertex.vertex_color =
                pack_vertex_color(mesh.vertex_colors.get(index).copied().unwrap_or(Vec3::ONE));
        }
        update_blended_centers(&mut self.blended_surfaces, &self.vertices);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        true
    }

    pub fn update_textures(
        &self,
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
        true
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
        brightness: f32,
    ) -> RenderStats {
        let mut draw_calls = 0;
        let aspect = viewport_size[0] as f32 / viewport_size[1] as f32;
        let display_gamma = [display_gamma(brightness), 0.0, 0.0, 0.0];
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
                display_gamma,
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
                    display_gamma,
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
        draw_calls += self.draw_scene(
            &mut pass,
            &self.camera_bind_group,
            &self.blended_index_buffer,
            &blended_batches,
            self.sky_target.as_ref().map(|target| &target.bind_group),
        );
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
    ) -> usize {
        let mut draw_calls = 0;
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
                pass.set_pipeline(&self.pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                draw_calls += 1;
            }
        }
        draw_calls
    }
}

fn display_gamma(brightness: f32) -> f32 {
    1.0 / (brightness * 2.0).clamp(0.05, 2.99)
}

fn pack_vertex_color(color: Vec3) -> [u8; 4] {
    let color = color.clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
    [
        color.x.round() as u8,
        color.y.round() as u8,
        color.z.round() as u8,
        255,
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

#[cfg(test)]
mod tests {
    use openhp1_scene::TriangleMesh;

    use super::*;

    #[test]
    fn shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shader.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn computes_scene_bounds() {
        let vertices = [
            Vertex {
                position: [-2.0, 3.0, 1.0],
                texture_coordinates: [0.0; 2],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
                vertex_color: [255; 4],
            },
            Vertex {
                position: [4.0, -1.0, 7.0],
                texture_coordinates: [0.0; 2],
                lightmap_coordinates: [0.0; 2],
                has_lightmap: 0.0,
                vertex_color: [255; 4],
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
    fn converts_ue1_screen_brightness_to_display_gamma() {
        assert_eq!(display_gamma(0.5), 1.0);
        assert_eq!(display_gamma(0.625), 0.8);
    }

    #[test]
    fn packs_vertex_lighting_like_fixed_function_diffuse_color() {
        assert_eq!(
            pack_vertex_color(Vec3::new(-0.5, 0.5, 2.0)),
            [0, 128, 255, 255]
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
            vertex_color: [255; 4],
        }
    }
}
