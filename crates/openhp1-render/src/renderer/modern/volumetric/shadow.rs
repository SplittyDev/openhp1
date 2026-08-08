use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use openhp1_scene::{RenderScene, SurfaceMode};
use wgpu::util::DeviceExt;

use crate::{Camera, unreal_to_render};

use super::super::super::DEPTH_FORMAT;
use super::super::HDR_FORMAT;

const SHADOW_SIZE: u32 = 1024;
const MAX_VISIBLE_PORTALS: usize = 128;
const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, -1.0, -0.35);
const SHAFT_SHADER: &str = include_str!("../../../shaders/modern/sky_shafts.wgsl");
const SHADER: &str = r#"
struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    direction_density: vec4<f32>,
    distance_intensity_phase: vec4<f32>,
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
    pub(super) distance_intensity_phase: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ShadowVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PortalTriangle {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    color: [f32; 4],
    direction: [f32; 4],
}

pub(super) struct DirectionalShadow {
    pub(super) uniform: wgpu::Buffer,
    pub(super) view: wgpu::TextureView,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    shaft_layout: wgpu::BindGroupLayout,
    shaft_bind_group: wgpu::BindGroup,
    shaft_pipeline: wgpu::RenderPipeline,
    shadow_sampler: wgpu::Sampler,
    portal_buffer: wgpu::Buffer,
    portals: Vec<PortalTriangle>,
    visible_portal_count: usize,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_count: usize,
    index_count: u32,
    enabled: bool,
}

impl DirectionalShadow {
    pub(super) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<ShadowVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        }
    }

    fn portal_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4];
        wgpu::VertexBufferLayout {
            array_stride: size_of::<PortalTriangle>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }

    pub(super) fn new(
        device: &wgpu::Device,
        scene_depth: &wgpu::TextureView,
        scene: &RenderScene,
    ) -> Self {
        let enabled = sky_exposed(scene);
        let vertices = shadow_vertices(scene);
        let indices = shadow_caster_indices(scene);
        let portals = shaft_portals(scene);
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
        let portal_buffer = portal_buffer(device, &portals);
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
        let shaft_bind_group = shaft_bind_group(
            device,
            &shaft_layout,
            scene_depth,
            &view,
            &shadow_sampler,
            &uniform,
        );
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
                targets: &[Some(wgpu::ColorTargetState {
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
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            uniform,
            view,
            _texture: texture,
            bind_group,
            pipeline,
            shaft_layout,
            shaft_bind_group,
            shaft_pipeline,
            shadow_sampler,
            portal_buffer,
            portals,
            visible_portal_count: 0,
            vertex_buffer,
            index_buffer,
            vertex_count: vertices.len(),
            index_count: indices.len() as u32,
            enabled,
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, scene_depth: &wgpu::TextureView) {
        self.shaft_bind_group = shaft_bind_group(
            device,
            &self.shaft_layout,
            scene_depth,
            &self.view,
            &self.shadow_sampler,
            &self.uniform,
        );
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let vertices = shadow_vertices(scene);
        let portals = shaft_portals(scene);
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

    pub(super) fn prepare(&mut self, queue: &wgpu::Queue, camera: &Camera, aspect: f32) {
        if !self.enabled {
            return;
        }
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&shadow_uniform(camera, aspect)),
        );
        let portals = visible_portals(&self.portals, camera, aspect);
        self.visible_portal_count = portals.len();
        if !portals.is_empty() {
            queue.write_buffer(&self.portal_buffer, 0, bytemuck::cast_slice(&portals));
        }
    }

    pub(super) fn render(&self, encoder: &mut wgpu::CommandEncoder) -> usize {
        if !self.enabled || self.index_count == 0 || self.visible_portal_count == 0 {
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
        self.draw_geometry(&mut pass);
        1
    }

    pub(super) fn has_geometry(&self) -> bool {
        self.index_count != 0
    }

    pub(super) fn draw_geometry<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    pub(super) fn render_shafts(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) -> usize {
        if !self.enabled || self.index_count == 0 || self.visible_portal_count == 0 {
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
        pass.set_bind_group(0, &self.shaft_bind_group, &[]);
        pass.set_vertex_buffer(0, self.portal_buffer.slice(..));
        pass.draw(0..6, 0..self.visible_portal_count as u32);
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
        ],
    })
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
                .is_some_and(|material| {
                    material.mode == SurfaceMode::Opaque && !material.volumetric_source
                })
        })
        .flat_map(|(triangle, _)| triangle.iter().copied())
        .collect()
}

fn shaft_portals(scene: &RenderScene) -> Vec<PortalTriangle> {
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
            let &[a, b, c] = triangle else { return None };
            let a = unreal_to_render(*scene.mesh.positions.get(a as usize)?);
            let b = unreal_to_render(*scene.mesh.positions.get(b as usize)?);
            let c = unreal_to_render(*scene.mesh.positions.get(c as usize)?);
            Some(PortalTriangle {
                a: a.extend(1.0).to_array(),
                b: b.extend(1.0).to_array(),
                c: c.extend(1.0).to_array(),
                color: shaft_color(scene, material.texture).extend(1.0).to_array(),
                direction: portal_direction(a, b, c).extend(0.0).to_array(),
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
                && portal_in_view(*portal, view_projection)
        })
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        portal_distance_squared(*left, camera.position)
            .total_cmp(&portal_distance_squared(*right, camera.position))
    });
    visible.truncate(MAX_VISIBLE_PORTALS);
    visible
}

fn camera_on_interior_side(portal: PortalTriangle, camera_position: Vec3) -> bool {
    let a = Vec3::from_slice(&portal.a);
    let normal = (Vec3::from_slice(&portal.b) - a).cross(Vec3::from_slice(&portal.c) - a);
    normal.dot(camera_position - a) < 0.0
}

fn portal_in_view(portal: PortalTriangle, view_projection: Mat4) -> bool {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    let mut front_points = 0;
    for point in [portal.a, portal.b, portal.c] {
        let clip = view_projection * Vec3::from_slice(&point).extend(1.0);
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

fn portal_distance_squared(portal: PortalTriangle, camera_position: Vec3) -> f32 {
    let center =
        (Vec3::from_slice(&portal.a) + Vec3::from_slice(&portal.b) + Vec3::from_slice(&portal.c))
            / 3.0;
    center.distance_squared(camera_position)
}

fn shaft_color(scene: &RenderScene, texture: Option<usize>) -> Vec3 {
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

fn shadow_uniform(camera: &Camera, aspect: f32) -> ShadowUniform {
    let radius = camera.far.clamp(500.0, 3_000.0);
    let direction = SUN_DIRECTION.normalize();
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
        camera_position: camera.position.extend(1.0).to_array(),
        direction_density: [direction.x, direction.y, direction.z, 0.00025],
        distance_intensity_phase: [radius, 0.35, 0.45, 0.0],
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
    use openhp1_scene::{SurfaceMaterial, TextureImage, TriangleMesh};

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
        assert_eq!(shaft_portals(&window).len(), 1);
        assert!(shadow_caster_indices(&window).is_empty());
    }

    #[test]
    fn shaft_sources_are_culled_outside_the_camera_view() {
        let portal = |offset: Vec3| PortalTriangle {
            a: (Vec3::new(-0.5, -0.5, 0.5) + offset).extend(1.0).to_array(),
            b: (Vec3::new(0.5, -0.5, 0.5) + offset).extend(1.0).to_array(),
            c: (Vec3::new(0.0, 0.5, 0.5) + offset).extend(1.0).to_array(),
            color: [1.0; 4],
            direction: [0.0; 4],
        };
        assert!(portal_in_view(portal(Vec3::ZERO), Mat4::IDENTITY));
        assert!(!portal_in_view(
            portal(Vec3::new(3.0, 0.0, 0.0)),
            Mat4::IDENTITY
        ));
        assert!(!portal_in_view(
            portal(Vec3::new(0.0, 0.0, -2.0)),
            Mat4::IDENTITY
        ));
    }

    #[test]
    fn window_shafts_only_render_from_the_bsp_interior_side() {
        let portal = PortalTriangle {
            a: Vec3::ZERO.extend(1.0).to_array(),
            b: Vec3::X.extend(1.0).to_array(),
            c: Vec3::Y.extend(1.0).to_array(),
            color: [1.0; 4],
            direction: [0.0; 4],
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
    fn shaft_color_uses_opaque_texture_rgb_when_palette_alpha_is_zero() {
        let mut scene = scene(SurfaceMode::Opaque, false);
        scene.textures.push(TextureImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 0],
        });

        let color = shaft_color(&scene, Some(0));

        assert!(color.x > color.y * 4.0);
        assert!(color.x > color.z * 4.0);
    }

    #[test]
    #[ignore = "requires local original game files"]
    fn tut1_windows_feed_a_finite_number_of_shaft_prisms() {
        let level =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/Maps/Lev_Tut1.unr");
        let scene = openhp1_scene::LoadedScene::load(level).unwrap();
        let portals = shaft_portals(&scene.render);
        assert!(!portals.is_empty());
        assert!(portals.len() < 1_024, "{} shaft triangles", portals.len());
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

    #[test]
    fn sky_shaft_shader_and_uniform_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHAFT_SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<ShadowUniform>(), 240);
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
