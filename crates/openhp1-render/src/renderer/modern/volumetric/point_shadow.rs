use std::{collections::HashSet, mem::size_of};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use openhp1_scene::{RenderLight, RenderScene};

use crate::Camera;

use super::super::super::DEPTH_FORMAT;
use super::{VolumetricInstance, shadow::DirectionalShadow, texture_light_color};

pub(super) const MAX_POINT_SHADOWS: usize = 4;
const FACE_COUNT: usize = 6;
const SHADOW_SIZE: u32 = 256;
const SHADER: &str = r#"
struct FaceSettings {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> settings: FaceSettings;

@vertex
fn vertex_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return settings.view_projection * vec4(position, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FaceUniform {
    view_projection: [[f32; 4]; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PointSource {
    actor_index: usize,
    position: Vec3,
    color: Vec3,
    radius: f32,
}

struct Face {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    view: wgpu::TextureView,
}

pub(super) struct PointShadowRenderer {
    pub(super) view: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
    _texture: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    faces: Vec<Face>,
    sources: Vec<PointSource>,
    selected: Vec<PointSource>,
}

impl PointShadowRenderer {
    pub(super) fn new(device: &wgpu::Device, scene: &RenderScene) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 volumetric point shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: (MAX_POINT_SHADOWS * FACE_COUNT) as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("OpenHP1 volumetric point shadow cube array"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            base_array_layer: 0,
            array_layer_count: Some((MAX_POINT_SHADOWS * FACE_COUNT) as u32),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 volumetric point shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 volumetric point shadow face layout"),
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
        let faces = (0..MAX_POINT_SHADOWS * FACE_COUNT)
            .map(|layer| {
                let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("OpenHP1 volumetric point shadow face settings"),
                    size: size_of::<FaceUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("OpenHP1 volumetric point shadow face bind group"),
                    layout: &layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("OpenHP1 volumetric point shadow face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                Face {
                    uniform,
                    bind_group,
                    view,
                }
            })
            .collect();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 volumetric point shadow shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 volumetric point shadow pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 volumetric point shadow pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_shadow"),
                compilation_options: Default::default(),
                buffers: &[DirectionalShadow::vertex_layout()],
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
            view,
            sampler,
            _texture: texture,
            pipeline,
            faces,
            sources: point_sources(scene),
            selected: Vec::new(),
        }
    }

    pub(super) fn update(&mut self, scene: &RenderScene) {
        self.sources = point_sources(scene);
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
    ) -> (Vec<usize>, Vec<VolumetricInstance>) {
        self.selected = select_sources(&self.sources, camera.position);
        for (shadow_index, source) in self.selected.iter().enumerate() {
            for face_index in 0..FACE_COUNT {
                let face = &self.faces[shadow_index * FACE_COUNT + face_index];
                queue.write_buffer(
                    &face.uniform,
                    0,
                    bytemuck::bytes_of(&face_uniform(*source, face_index)),
                );
            }
        }
        let actor_indices = self
            .selected
            .iter()
            .map(|source| source.actor_index)
            .collect();
        let instances = self
            .selected
            .iter()
            .enumerate()
            .map(|(shadow_index, source)| VolumetricInstance {
                position_radius: source.position.extend(source.radius).to_array(),
                color_fog: (source.color * 0.00003).extend(0.0).to_array(),
                profile: [1.0, shadow_index as f32 + 1.0, source.radius, 0.0],
            })
            .collect();
        (actor_indices, instances)
    }

    pub(super) fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        geometry: &DirectionalShadow,
    ) -> usize {
        if self.selected.is_empty() || !geometry.has_geometry() {
            return 0;
        }
        for face in self.faces.iter().take(self.selected.len() * FACE_COUNT) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 volumetric point shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &face.view,
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
            pass.set_bind_group(0, &face.bind_group, &[]);
            geometry.draw_geometry(&mut pass);
        }
        self.selected.len() * FACE_COUNT
    }
}

fn point_sources(scene: &RenderScene) -> Vec<PointSource> {
    let corona_colors = scene
        .coronas
        .iter()
        .map(|corona| (corona.actor_index, corona.color))
        .collect::<std::collections::HashMap<_, _>>();
    let mut seen = HashSet::new();
    scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .filter(|light| {
            let visible_emitter =
                light.source_texture.is_some() || corona_colors.contains_key(&light.actor_index);
            let authored_volume =
                light.brightness != 0 && light.volume_radius != 0 && light.volume_brightness != 0;
            light.actor_index != usize::MAX
                && light.effect != 4
                && (visible_emitter || authored_volume)
                && seen.insert(light.actor_index)
        })
        .map(|light| {
            let color = corona_colors
                .get(&light.actor_index)
                .copied()
                .or_else(|| {
                    light
                        .source_texture
                        .and_then(|texture| scene.textures.get(texture))
                        .map(texture_light_color)
                })
                .unwrap_or_else(|| light.source_color());
            point_source(light, color)
        })
        .collect()
}

fn point_source(light: &RenderLight, color: Vec3) -> PointSource {
    PointSource {
        actor_index: light.actor_index,
        position: light.location,
        color,
        radius: ((f32::from(light.radius) + 1.0) * 25.0).clamp(150.0, 600.0),
    }
}

fn select_sources(sources: &[PointSource], camera: Vec3) -> Vec<PointSource> {
    let mut selected = sources.to_vec();
    selected.sort_unstable_by(|left, right| {
        left.position
            .distance_squared(camera)
            .total_cmp(&right.position.distance_squared(camera))
            .then_with(|| left.actor_index.cmp(&right.actor_index))
    });
    selected.truncate(MAX_POINT_SHADOWS);
    selected
}

fn face_uniform(source: PointSource, face: usize) -> FaceUniform {
    let (direction, up) = match face {
        0 => (Vec3::X, -Vec3::Y),
        1 => (-Vec3::X, -Vec3::Y),
        2 => (Vec3::Y, Vec3::Z),
        3 => (-Vec3::Y, -Vec3::Z),
        4 => (Vec3::Z, -Vec3::Y),
        _ => (-Vec3::Z, -Vec3::Y),
    };
    let view = Mat4::look_to_rh(source.position, direction, up);
    let projection = Mat4::perspective_rh(90_f32.to_radians(), 1.0, 1.0, source.radius);
    FaceUniform {
        view_projection: (projection * view).to_cols_array_2d(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_sources_win_the_shadow_budget_deterministically() {
        let sources = (0..6)
            .map(|index| PointSource {
                actor_index: index,
                position: Vec3::new(index as f32 * 10.0, 0.0, 0.0),
                color: Vec3::ONE,
                radius: 100.0,
            })
            .collect::<Vec<_>>();
        let selected = select_sources(&sources, Vec3::new(35.0, 0.0, 0.0));
        assert_eq!(
            selected
                .iter()
                .map(|source| source.actor_index)
                .collect::<Vec<_>>(),
            [3, 4, 2, 5]
        );
    }

    #[test]
    fn point_shadow_shader_and_uniform_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<FaceUniform>(), 64);
    }
}
