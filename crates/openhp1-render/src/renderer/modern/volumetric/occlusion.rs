use std::sync::mpsc::{self, Receiver, TryRecvError};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::Camera;

use super::super::super::DEPTH_FORMAT;
use super::shadow::{PortalTriangle, portal_volume_points};

const VERTICES_PER_VOLUME: usize = 24;
const READBACK_COUNT: usize = 2;
const OCCLUDED_FRAMES: u8 = 2;
// ponytail: The shipped corpus stays below this; raise it if an authored map exceeds the query cap.
const MAX_QUERIES: usize = 1_024;

const SHADER: &str = r#"
struct OcclusionSettings {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> settings: OcclusionSettings;

@vertex
fn vertex_occlusion(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return settings.view_projection * vec4(position, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OcclusionUniform {
    view_projection: [[f32; 4]; 4],
}

struct Readback {
    buffer: wgpu::Buffer,
    receiver: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    portal_indices: Vec<usize>,
}

pub(super) struct ShaftOcclusion {
    depth_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readbacks: [Readback; READBACK_COUNT],
    candidates: Vec<usize>,
    misses: Vec<u8>,
    geometry: Vec<PortalTriangle>,
    length: f32,
    query_capacity: usize,
}

impl ShaftOcclusion {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        depth_view: &wgpu::TextureView,
        portals: &[PortalTriangle],
        length: f32,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 shaft occlusion camera"),
            size: std::mem::size_of::<OcclusionUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 shaft occlusion layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 shaft occlusion bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 shaft occlusion shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 shaft occlusion pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 shaft occlusion pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_occlusion"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let vertices = occlusion_vertices(portals, length);
        let fallback = [0.0_f32; 3];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 shaft occlusion volumes"),
            contents: if vertices.is_empty() {
                bytemuck::bytes_of(&fallback)
            } else {
                bytemuck::cast_slice(&vertices)
            },
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let query_capacity = portals.len().clamp(1, MAX_QUERIES);
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("OpenHP1 shaft occlusion queries"),
            ty: wgpu::QueryType::Occlusion,
            count: query_capacity as u32,
        });
        let query_bytes = (query_capacity * std::mem::size_of::<u64>()) as u64;
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 shaft occlusion resolve"),
            size: query_bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readbacks = std::array::from_fn(|_| Readback {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("OpenHP1 shaft occlusion readback"),
                size: query_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            receiver: None,
            portal_indices: Vec::new(),
        });
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&OcclusionUniform {
                view_projection: glam::Mat4::IDENTITY.to_cols_array_2d(),
            }),
        );
        Self {
            depth_view: depth_view.clone(),
            pipeline,
            bind_group,
            uniform,
            vertex_buffer,
            query_set,
            resolve_buffer,
            readbacks,
            candidates: Vec::new(),
            misses: vec![0; portals.len()],
            geometry: portals.to_vec(),
            length,
            query_capacity,
        }
    }

    pub(super) fn resize(&mut self, depth_view: &wgpu::TextureView) {
        self.depth_view = depth_view.clone();
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
        portals: &[PortalTriangle],
        candidates: impl IntoIterator<Item = usize>,
        length: f32,
    ) {
        self.collect_results();
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&OcclusionUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
            }),
        );
        if self.geometry != portals || self.length != length {
            let vertices = occlusion_vertices(portals, length);
            if !vertices.is_empty() {
                queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            }
            self.geometry.clone_from_slice(portals);
            self.length = length;
        }
        self.candidates.clear();
        self.candidates
            .extend(candidates.into_iter().take(self.query_capacity));
    }

    pub(super) fn visible(&self, portal_index: usize) -> bool {
        self.misses
            .get(portal_index)
            .is_none_or(|misses| *misses < OCCLUDED_FRAMES)
    }

    pub(super) fn render(&mut self, encoder: &mut wgpu::CommandEncoder) -> usize {
        let Some(readback_index) = self
            .readbacks
            .iter()
            .position(|readback| readback.receiver.is_none())
        else {
            return 0;
        };
        if self.candidates.is_empty() {
            return 0;
        }
        let query_count = self.candidates.len() as u32;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 shaft occlusion pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: Some(&self.query_set),
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            for (query_index, &portal_index) in self.candidates.iter().enumerate() {
                let start = (portal_index * VERTICES_PER_VOLUME) as u32;
                pass.begin_occlusion_query(query_index as u32);
                pass.draw(start..start + VERTICES_PER_VOLUME as u32, 0..1);
                pass.end_occlusion_query();
            }
        }
        let byte_count = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
        encoder.resolve_query_set(&self.query_set, 0..query_count, &self.resolve_buffer, 0);
        let readback = &mut self.readbacks[readback_index];
        encoder.copy_buffer_to_buffer(&self.resolve_buffer, 0, &readback.buffer, 0, byte_count);
        let (sender, receiver) = mpsc::channel();
        encoder.map_buffer_on_submit(
            &readback.buffer,
            wgpu::MapMode::Read,
            0..byte_count,
            move |result| {
                let _ = sender.send(result);
            },
        );
        readback.portal_indices.clone_from(&self.candidates);
        readback.receiver = Some(receiver);
        1
    }

    fn collect_results(&mut self) {
        for readback in &mut self.readbacks {
            let Some(receiver) = &readback.receiver else {
                continue;
            };
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    let byte_count =
                        (readback.portal_indices.len() * std::mem::size_of::<u64>()) as u64;
                    let mapped = readback.buffer.slice(0..byte_count).get_mapped_range();
                    for (&portal_index, &samples) in readback
                        .portal_indices
                        .iter()
                        .zip(bytemuck::cast_slice::<u8, u64>(&mapped))
                    {
                        let misses = &mut self.misses[portal_index];
                        *misses = next_misses(*misses, samples);
                    }
                    drop(mapped);
                    readback.buffer.unmap();
                    readback.receiver = None;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    readback.receiver = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
    }
}

fn next_misses(misses: u8, samples: u64) -> u8 {
    if samples == 0 {
        misses.saturating_add(1)
    } else {
        0
    }
}

fn occlusion_vertices(portals: &[PortalTriangle], length: f32) -> Vec<[f32; 3]> {
    const TRIANGLES: [usize; VERTICES_PER_VOLUME] = [
        0, 1, 2, 3, 5, 4, 0, 3, 4, 0, 4, 1, 1, 4, 5, 1, 5, 2, 2, 5, 3, 2, 3, 0,
    ];
    portals
        .iter()
        .flat_map(|&portal| {
            let points = portal_volume_points(portal, length);
            TRIANGLES.map(|index| points[index].to_array())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaft_occlusion_requires_two_hidden_frames_and_resets_when_visible() {
        let misses = next_misses(0, 0);
        assert!(misses < OCCLUDED_FRAMES);
        let misses = next_misses(misses, 0);
        assert_eq!(misses, OCCLUDED_FRAMES);
        assert_eq!(next_misses(misses, 1), 0);
    }
}
