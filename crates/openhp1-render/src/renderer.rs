use std::{mem::size_of, ops::Range};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::{
    Camera, RenderScene, SceneBounds, SurfaceMaterial, SurfaceMode, TextureImage, unreal_to_render,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const PIPELINES_PER_MODE: usize = 4;
const PIPELINE_COUNT: usize = 12;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    texture_coordinates: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
}

pub struct Renderer {
    pipelines: [wgpu::RenderPipeline; PIPELINE_COUNT],
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_groups: Vec<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    opaque_index_buffer: wgpu::Buffer,
    opaque_batches: Vec<DrawBatch>,
    blended_index_buffer: wgpu::Buffer,
    blended_surfaces: Vec<BlendedSurface>,
    depth: DepthTarget,
    bounds: SceneBounds,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        scene: &RenderScene,
        viewport_size: [u32; 2],
    ) -> Self {
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
                Vertex {
                    position: position.to_array(),
                    texture_coordinates: [
                        coordinates.x / dimensions[0],
                        coordinates.y / dimensions[1],
                    ],
                }
            })
            .collect();
        let bounds = scene_bounds(&vertices);
        let (opaque_indices, opaque_batches) = texture_batches(scene, fallback_texture);
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
        let blended_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 blended BSP indices"),
            size: (blended_index_count * size_of::<u32>()).max(size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
        let checkerboard = checkerboard();
        let texture_bind_groups = scene
            .textures
            .iter()
            .chain(std::iter::once(&checkerboard))
            .map(|texture| texture_bind_group(device, queue, &texture_layout, &sampler, texture))
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
                mode,
                index % PIPELINES_PER_MODE >= 2,
                index % 2 != 0,
            )
        });

        Self {
            pipelines,
            camera_buffer,
            camera_bind_group,
            texture_bind_groups,
            vertex_buffer,
            opaque_index_buffer,
            opaque_batches,
            blended_index_buffer,
            blended_surfaces,
            depth: DepthTarget::new(device, viewport_size),
            bounds,
        }
    }

    pub fn bounds(&self) -> SceneBounds {
        self.bounds
    }

    pub fn resize(&mut self, device: &wgpu::Device, viewport_size: [u32; 2]) {
        if self.depth.size != viewport_size {
            self.depth = DepthTarget::new(device, viewport_size);
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
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 BSP render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.035,
                        g: 0.045,
                        b: 0.065,
                        a: 1.0,
                    }),
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
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        if !self.opaque_batches.is_empty() || !blended_batches.is_empty() {
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
        if !blended_batches.is_empty() {
            pass.set_index_buffer(
                self.blended_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for batch in &blended_batches {
                pass.set_pipeline(&self.pipelines[batch.pipeline]);
                pass.set_bind_group(1, &self.texture_bind_groups[batch.texture], &[]);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
            }
        }
    }
}

struct DrawBatch {
    indices: Range<u32>,
    texture: usize,
    pipeline: usize,
}

struct BlendedSurface {
    indices: Vec<u32>,
    center: Vec3,
    texture: usize,
    pipeline: usize,
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
        SurfaceMode::Opaque | SurfaceMode::Hidden => 0,
        SurfaceMode::Translucent => 1,
        SurfaceMode::Modulated => 2,
    };
    mode * PIPELINES_PER_MODE + usize::from(material.masked) * 2 + usize::from(material.two_sided)
}

fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    mode: SurfaceMode,
    masked: bool,
    two_sided: bool,
) -> wgpu::RenderPipeline {
    let blended = matches!(mode, SurfaceMode::Translucent | SurfaceMode::Modulated);
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
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
            }],
        },
        primitive: wgpu::PrimitiveState {
            // The Unreal-to-render axis conversion changes handedness, so UE
            // polygon winding becomes clockwise in render space.
            front_face: wgpu::FrontFace::Cw,
            cull_mode: (!two_sided).then_some(wgpu::Face::Back),
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
            entry_point: Some(fragment_entry(mode, masked)),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: blend_state(mode),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn fragment_entry(mode: SurfaceMode, masked: bool) -> &'static str {
    match (mode, masked) {
        (SurfaceMode::Opaque, false) => "fragment_main",
        (SurfaceMode::Opaque, true) => "fragment_masked",
        (SurfaceMode::Translucent | SurfaceMode::Modulated, false) => "fragment_blended",
        (SurfaceMode::Translucent | SurfaceMode::Modulated, true) => "fragment_blended_masked",
        (SurfaceMode::Hidden, _) => unreachable!(),
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
        SurfaceMode::Hidden => unreachable!(),
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
) -> wgpu::BindGroup {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("OpenHP1 texture"),
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
    );
    let view = texture.create_view(&Default::default());
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
        ],
    })
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
            },
            Vertex {
                position: [4.0, -1.0, 7.0],
                texture_coordinates: [0.0; 2],
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
        };
        let surfaces = blended_surfaces(&scene, 2, &vertices);
        let (indices, batches) = sorted_blended_batches(&surfaces, Vec3::ZERO);
        assert_eq!(indices, [3, 4, 5, 0, 1, 2]);
        assert_eq!(
            batches
                .iter()
                .map(|batch| (batch.texture, batch.pipeline, batch.indices.clone()))
                .collect::<Vec<_>>(),
            [(1, 10, 0..3), (0, 4, 3..6)]
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
            fragment_entry(SurfaceMode::Modulated, true),
            "fragment_blended_masked"
        );
    }

    fn vertex_at(x: f32, y: f32, z: f32) -> Vertex {
        Vertex {
            position: [x, y, z],
            texture_coordinates: [0.0; 2],
        }
    }
}
