use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use openhp1_scene::{RenderScene, SurfaceMode};
use wgpu::util::DeviceExt;

use crate::{Camera, unreal_to_render};

use super::super::super::DEPTH_FORMAT;

const SHADOW_SIZE: u32 = 1024;
const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, -1.0, -0.35);
const SHADER: &str = r#"
struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    direction: vec4<f32>,
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
    pub(super) direction: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowVertex {
    position: [f32; 3],
}

pub(super) struct DirectionalShadow {
    pub(super) uniform: wgpu::Buffer,
    pub(super) view: wgpu::TextureView,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_count: usize,
    index_count: u32,
    enabled: bool,
}

impl DirectionalShadow {
    pub(super) fn new(device: &wgpu::Device, scene: &RenderScene) -> Self {
        let enabled = sky_exposed(scene);
        let vertices = shadow_vertices(scene);
        let indices = shadow_caster_indices(scene);
        let fallback_vertex = ShadowVertex::zeroed();
        let fallback_index = 0_u32;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 volumetric shadow vertices"),
            contents: if vertices.is_empty() {
                bytemuck::bytes_of(&fallback_vertex)
            } else {
                bytemuck::cast_slice(&vertices)
            },
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OpenHP1 volumetric shadow indices"),
            contents: if indices.is_empty() {
                bytemuck::bytes_of(&fallback_index)
            } else {
                bytemuck::cast_slice(&indices)
            },
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 volumetric sun shadow settings"),
            size: size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 volumetric sun shadow bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 volumetric sun shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
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
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<ShadowVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
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
        Self {
            uniform,
            view,
            _texture: texture,
            bind_group,
            pipeline,
            vertex_buffer,
            index_buffer,
            vertex_count: vertices.len(),
            index_count: indices.len() as u32,
            enabled,
        }
    }

    pub(super) fn update(&self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let vertices = shadow_vertices(scene);
        if vertices.len() != self.vertex_count || sky_exposed(scene) != self.enabled {
            return false;
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
        true
    }

    pub(super) fn prepare(&self, queue: &wgpu::Queue, camera: &Camera) {
        if !self.enabled {
            return;
        }
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&shadow_uniform(camera)),
        );
    }

    pub(super) fn render(&self, encoder: &mut wgpu::CommandEncoder) -> usize {
        if !self.enabled || self.index_count == 0 {
            return 0;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 volumetric sun shadow pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.view,
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
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
        1
    }
}

fn sky_exposed(scene: &RenderScene) -> bool {
    scene.sky_zone.is_some()
        && scene
            .surface_materials
            .iter()
            .any(|material| material.mode == SurfaceMode::Backdrop)
}

fn shadow_vertices(scene: &RenderScene) -> Vec<ShadowVertex> {
    scene
        .mesh
        .positions
        .iter()
        .map(|&position| ShadowVertex {
            position: unreal_to_render(position).to_array(),
        })
        .collect()
}

fn shadow_caster_indices(scene: &RenderScene) -> Vec<u32> {
    scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
        .filter(|(_, surface)| {
            scene
                .surface_materials
                .get(**surface)
                .is_some_and(|material| material.mode == SurfaceMode::Opaque)
        })
        .flat_map(|(triangle, _)| triangle.iter().copied())
        .collect()
}

fn shadow_uniform(camera: &Camera) -> ShadowUniform {
    let radius = camera.far.clamp(500.0, 3_000.0);
    let direction = SUN_DIRECTION.normalize();
    let center = camera.position + camera.forward() * radius * 0.35;
    let eye = center - direction * radius * 2.0;
    let view = Mat4::look_at_rh(eye, center, Vec3::Z);
    let projection = Mat4::orthographic_rh(-radius, radius, -radius, radius, 1.0, radius * 4.0);
    ShadowUniform {
        light_view_projection: (projection * view).to_cols_array_2d(),
        direction: direction.extend(0.0).to_array(),
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;
    use openhp1_scene::{SurfaceMaterial, TriangleMesh};

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
                ..Default::default()
            }],
            sky_zone: sky.then_some(openhp1_scene::SkyZone {
                location: Vec3::ZERO,
                rotation: Default::default(),
            }),
        }
    }

    #[test]
    fn sky_shafts_require_a_real_backdrop_portal() {
        assert!(!sky_exposed(&scene(SurfaceMode::Opaque, true)));
        assert!(!sky_exposed(&scene(SurfaceMode::Backdrop, false)));
        assert!(sky_exposed(&scene(SurfaceMode::Backdrop, true)));
    }

    #[test]
    fn only_opaque_geometry_casts_sun_shadows() {
        assert_eq!(
            shadow_caster_indices(&scene(SurfaceMode::Opaque, true)),
            [0, 1, 2]
        );
        assert!(shadow_caster_indices(&scene(SurfaceMode::Backdrop, true)).is_empty());
        assert!(shadow_caster_indices(&scene(SurfaceMode::Translucent, true)).is_empty());
    }
}
