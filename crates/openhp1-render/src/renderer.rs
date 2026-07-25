use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use openhp1_map::TriangleMesh;
use wgpu::util::DeviceExt;

use crate::{Camera, SceneBounds, unreal_to_render};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    depth: DepthTarget,
    bounds: SceneBounds,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        mesh: &TriangleMesh,
        viewport_size: [u32; 2],
    ) -> Self {
        let vertices: Vec<_> = mesh
            .positions
            .iter()
            .copied()
            .map(unreal_to_render)
            .map(|position| Vertex {
                position: position.to_array(),
                color: [0.74, 0.78, 0.88],
            })
            .collect();
        let bounds = scene_bounds(&vertices);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 BSP vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 BSP indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
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
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 BSP pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 BSP pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_buffer,
            camera_bind_group,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        if self.index_count != 0 {
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
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
    use super::*;

    #[test]
    fn computes_scene_bounds() {
        let vertices = [
            Vertex {
                position: [-2.0, 3.0, 1.0],
                color: [0.0; 3],
            },
            Vertex {
                position: [4.0, -1.0, 7.0],
                color: [0.0; 3],
            },
        ];
        let bounds = scene_bounds(&vertices);
        assert_eq!(bounds.minimum, Vec3::new(-2.0, -1.0, 1.0));
        assert_eq!(bounds.maximum, Vec3::new(4.0, 3.0, 7.0));
    }
}
