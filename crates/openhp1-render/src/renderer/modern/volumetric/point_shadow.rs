use std::{
    collections::{HashMap, HashSet},
    mem::size_of,
};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use openhp1_scene::{RenderLight, RenderScene};

use crate::Camera;

use super::super::super::DEPTH_FORMAT;
use super::{VolumetricInstance, shadow::DirectionalShadow, texture_light_color};

// ponytail: Fixed local-light budget; make shadow allocation dynamic if authored scenes exceed it.
pub(super) const MAX_POINT_SHADOWS: usize = 20;
const FACE_COUNT: usize = 6;
const SHADOW_SIZE: u32 = 128;
const SCATTERING_STRENGTH: f32 = 0.003;
const MAX_SCATTERING_RADIUS: f32 = 300.0;
const FIXTURE_APERTURE_DISTANCE: f32 = 32.0;
const FIXTURE_CLUSTER_DISTANCE: f32 = 64.0;
const MAX_FIXTURE_SAMPLES: usize = 3;
const DENSE_FIXTURE_RADIUS: f32 = 150.0;
const SHADER: &str = r#"
struct FaceSettings {
    view_projection: mat4x4<f32>,
    source_position_aperture_distance: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> settings: FaceSettings;

struct ShadowOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) transmission: f32,
};

@vertex
fn vertex_shadow(
    @location(0) position: vec3<f32>,
    @location(1) transmission: f32,
) -> ShadowOutput {
    var output: ShadowOutput;
    output.position = settings.view_projection * vec4(position, 1.0);
    output.world_position = position;
    output.transmission = transmission;
    return output;
}

@fragment
fn fragment_shadow(input: ShadowOutput) {
    let fixture_distance = distance(
        input.world_position,
        settings.source_position_aperture_distance.xyz,
    );
    if fixture_distance <= settings.source_position_aperture_distance.w
        && input.transmission >= 0.65 {
        discard;
    }
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FaceUniform {
    view_projection: [[f32; 4]; 4],
    source_position_aperture_distance: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PointSource {
    actor_index: usize,
    position: Vec3,
    color: Vec3,
    radius: f32,
    fixture_emitter: bool,
    source_sprite: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct PointFixture {
    actor_indices: Vec<usize>,
    sources: Vec<PointSource>,
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
    sources: Vec<PointFixture>,
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
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            view,
            sampler,
            _texture: texture,
            pipeline,
            faces,
            sources: point_fixtures(scene),
            selected: Vec::new(),
        }
    }

    pub(super) fn update(&mut self, scene: &RenderScene) {
        self.sources = point_fixtures(scene);
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
    ) -> (Vec<usize>, Vec<VolumetricInstance>) {
        let (shadowed_actor_indices, selected) = select_sources(&self.sources, camera, aspect);
        self.selected = selected;
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
        let instances = self
            .selected
            .iter()
            .enumerate()
            .map(|(shadow_index, source)| VolumetricInstance {
                position_radius: source.position.extend(source.radius).to_array(),
                color_fog: (source.color * SCATTERING_STRENGTH).extend(0.0).to_array(),
                profile: [1.0, shadow_index as f32 + 1.0, source.radius, 0.0],
            })
            .collect();
        (shadowed_actor_indices, instances)
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
    let coronas = scene
        .coronas
        .iter()
        .map(|corona| (corona.actor_index, corona))
        .collect::<std::collections::HashMap<_, _>>();
    let mut seen = HashSet::new();
    scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .filter(|light| {
            let visible_emitter =
                light.source_texture.is_some() || coronas.contains_key(&light.actor_index);
            let authored_volume =
                light.brightness != 0 && light.volume_radius != 0 && light.volume_brightness != 0;
            light.actor_index != usize::MAX
                && light.effect != 4
                && (visible_emitter || authored_volume)
                && seen.insert(light.actor_index)
        })
        .map(|light| {
            let corona = coronas.get(&light.actor_index).copied();
            let fixture_emitter = light.source_texture.is_some() || corona.is_some();
            let source_sprite = light.source_texture.is_some() && corona.is_none();
            let color = corona
                .map(|corona| corona.color)
                .or_else(|| {
                    light
                        .source_texture
                        .and_then(|texture| scene.textures.get(texture))
                        .map(texture_light_color)
                })
                .unwrap_or_else(|| light.source_color());
            point_source(light, color, fixture_emitter, source_sprite)
        })
        .collect()
}

fn point_fixtures(scene: &RenderScene) -> Vec<PointFixture> {
    cluster_sources(point_sources(scene))
}

// ponytail: Scene light counts are small; replace this O(n²) grouping only if profiling says so.
fn cluster_sources(mut sources: Vec<PointSource>) -> Vec<PointFixture> {
    sources.sort_unstable_by_key(|source| source.actor_index);
    let mut assigned = vec![false; sources.len()];
    let mut fixtures = Vec::new();
    let maximum_distance_squared = FIXTURE_CLUSTER_DISTANCE * FIXTURE_CLUSTER_DISTANCE;

    for start in 0..sources.len() {
        if assigned[start] {
            continue;
        }
        assigned[start] = true;
        let mut members = vec![sources[start]];
        let mut pending = vec![start];
        if !sources[start].fixture_emitter {
            fixtures.push(fixture(members));
            continue;
        }
        while let Some(current) = pending.pop() {
            for candidate in 0..sources.len() {
                if !assigned[candidate]
                    && sources[candidate].fixture_emitter
                    && sources[current]
                        .position
                        .distance_squared(sources[candidate].position)
                        <= maximum_distance_squared
                {
                    assigned[candidate] = true;
                    pending.push(candidate);
                    members.push(sources[candidate]);
                }
            }
        }
        members.sort_unstable_by_key(|source| source.actor_index);
        fixtures.push(fixture(members));
    }
    fixtures
}

fn fixture(members: Vec<PointSource>) -> PointFixture {
    let dense = members.len() > MAX_FIXTURE_SAMPLES;
    let sprite_energy_divisor = members
        .iter()
        .all(|source| source.source_sprite)
        .then(|| members.len().max(MAX_FIXTURE_SAMPLES));
    let sample_step = members.len().div_ceil(MAX_FIXTURE_SAMPLES);
    PointFixture {
        actor_indices: members.iter().map(|source| source.actor_index).collect(),
        sources: members
            .iter()
            .step_by(sample_step)
            .take(MAX_FIXTURE_SAMPLES)
            .map(|source| {
                let mut source = *source;
                if dense {
                    source.radius = DENSE_FIXTURE_RADIUS;
                    source.color /= MAX_FIXTURE_SAMPLES as f32;
                } else if let Some(divisor) = sprite_energy_divisor {
                    source.color /= divisor as f32;
                }
                source
            })
            .collect(),
    }
}

pub(super) fn fixture_energy_scales(scene: &RenderScene) -> HashMap<usize, f32> {
    point_fixtures(scene)
        .into_iter()
        .flat_map(|fixture| {
            let scale = fixture_energy_scale(&fixture);
            fixture
                .actor_indices
                .into_iter()
                .map(move |actor_index| (actor_index, scale))
        })
        .collect()
}

fn fixture_energy_scale(fixture: &PointFixture) -> f32 {
    if fixture.actor_indices.len() > MAX_FIXTURE_SAMPLES {
        1.0 / fixture.actor_indices.len() as f32
    } else if fixture.sources.iter().all(|source| source.source_sprite) {
        1.0 / MAX_FIXTURE_SAMPLES as f32
    } else {
        1.0
    }
}

fn point_source(
    light: &RenderLight,
    color: Vec3,
    fixture_emitter: bool,
    source_sprite: bool,
) -> PointSource {
    PointSource {
        actor_index: light.actor_index,
        position: light.location,
        color,
        radius: if source_sprite {
            super::volume_radius(light, None)
        } else {
            ((f32::from(light.radius) + 1.0) * 25.0).clamp(150.0, MAX_SCATTERING_RADIUS)
        },
        fixture_emitter,
        source_sprite,
    }
}

fn select_sources(
    fixtures: &[PointFixture],
    camera: &Camera,
    aspect: f32,
) -> (Vec<usize>, Vec<PointSource>) {
    let mut visible = fixtures
        .iter()
        .filter_map(|fixture| {
            let mut sources = fixture
                .sources
                .iter()
                .copied()
                .filter(|source| source_in_view(*source, camera, aspect))
                .collect::<Vec<_>>();
            sources.sort_unstable_by(|left, right| {
                left.position
                    .distance_squared(camera.position)
                    .total_cmp(&right.position.distance_squared(camera.position))
                    .then_with(|| left.actor_index.cmp(&right.actor_index))
            });
            (!sources.is_empty()).then_some((fixture, sources))
        })
        .collect::<Vec<_>>();
    visible.sort_unstable_by(|(left_fixture, left), (right_fixture, right)| {
        left[0]
            .position
            .distance_squared(camera.position)
            .total_cmp(&right[0].position.distance_squared(camera.position))
            .then_with(|| left_fixture.actor_indices[0].cmp(&right_fixture.actor_indices[0]))
    });
    let mut shadowed_actor_indices = Vec::new();
    let mut selected = Vec::new();
    'samples: for sample in 0..MAX_FIXTURE_SAMPLES {
        for (fixture, sources) in &visible {
            let Some(source) = sources.get(sample) else {
                continue;
            };
            if sample == 0 {
                shadowed_actor_indices.extend_from_slice(&fixture.actor_indices);
            }
            selected.push(*source);
            if selected.len() == MAX_POINT_SHADOWS {
                break 'samples;
            }
        }
    }
    (shadowed_actor_indices, selected)
}

fn source_in_view(source: PointSource, camera: &Camera, aspect: f32) -> bool {
    let position = camera.view().transform_point3(source.position);
    let depth = -position.z;
    if depth + source.radius < camera.near || depth - source.radius > camera.far {
        return false;
    }
    let half_height = depth.max(0.0) * (camera.vertical_fov * 0.5).tan();
    position.y.abs() <= half_height + source.radius
        && position.x.abs() <= half_height * aspect + source.radius
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
        source_position_aperture_distance: source
            .position
            .extend(FIXTURE_APERTURE_DISTANCE)
            .to_array(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(radius: u8) -> RenderLight {
        RenderLight {
            actor_index: 1,
            source_texture: None,
            location: Vec3::ZERO,
            direction: Vec3::NEG_Z,
            effect: 0,
            brightness: 64,
            hue: 0,
            saturation: 0,
            radius,
            cone: 128,
            volume_brightness: 0,
            volume_fog: 0,
            volume_radius: 0,
            visibility: openhp1_scene::LightVisibility {
                width: 1,
                height: 1,
                values: vec![255],
            },
        }
    }

    #[test]
    fn shadowed_volume_caps_the_authored_light_range() {
        assert_eq!(
            point_source(&light(64), Vec3::ONE, false, false).radius,
            300.0
        );
    }

    #[test]
    fn clustered_sprite_flames_use_compact_shared_volumes() {
        let mut candle = light(64);
        candle.source_texture = Some(0);
        let single = fixture(vec![point_source(&candle, Vec3::ONE, true, true)]);
        let sources = (0..3)
            .map(|actor_index| {
                let mut source = point_source(&candle, Vec3::ONE, true, true);
                source.actor_index = actor_index;
                source.position.x = actor_index as f32 * 16.0;
                source
            })
            .collect();
        let fixture = &cluster_sources(sources)[0];

        assert_eq!(single.sources[0].radius, 50.0);
        assert_eq!(single.sources[0].color, Vec3::splat(1.0 / 3.0));
        assert_eq!(fixture_energy_scale(&single), 1.0 / 3.0);
        assert_eq!(fixture.sources.len(), 3);
        assert_eq!(fixture.sources[0].radius, 50.0);
        assert_eq!(fixture.sources[0].color, Vec3::splat(1.0 / 3.0));
        assert_eq!(fixture_energy_scale(fixture), 1.0 / 3.0);
    }

    #[test]
    fn nearest_sources_win_the_shadow_budget_deterministically() {
        let sources = (0..24)
            .map(|index| PointSource {
                actor_index: index,
                position: Vec3::new(0.0, 0.0, -10.0 - index as f32 * 10.0),
                color: Vec3::ONE,
                radius: 100.0,
                fixture_emitter: true,
                source_sprite: false,
            })
            .collect::<Vec<_>>();
        let fixtures = sources
            .iter()
            .map(|source| PointFixture {
                actor_indices: vec![source.actor_index],
                sources: vec![*source],
            })
            .collect::<Vec<_>>();
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 1_000.0);
        let (_, selected) = select_sources(&fixtures, &camera, 1.0);
        assert_eq!(
            selected
                .iter()
                .map(|source| source.actor_index)
                .collect::<Vec<_>>(),
            (0..MAX_POINT_SHADOWS).collect::<Vec<_>>()
        );
    }

    #[test]
    fn offscreen_sources_do_not_consume_the_shadow_budget() {
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 1_000.0);
        let sources = [
            PointSource {
                actor_index: 1,
                position: Vec3::new(0.0, 0.0, -50.0),
                color: Vec3::ONE,
                radius: 10.0,
                fixture_emitter: true,
                source_sprite: false,
            },
            PointSource {
                actor_index: 2,
                position: Vec3::new(500.0, 0.0, -50.0),
                color: Vec3::ONE,
                radius: 10.0,
                fixture_emitter: true,
                source_sprite: false,
            },
        ];
        let fixtures = sources.map(|source| PointFixture {
            actor_indices: vec![source.actor_index],
            sources: vec![source],
        });
        assert_eq!(select_sources(&fixtures, &camera, 1.0).1, vec![sources[0]]);
    }

    #[test]
    fn dense_fixture_keeps_three_emission_samples_and_bounded_energy() {
        let sources = (0..9)
            .map(|index| PointSource {
                actor_index: index,
                position: Vec3::new(index as f32 * 30.0, 0.0, 0.0),
                color: Vec3::ONE,
                radius: 300.0,
                fixture_emitter: true,
                source_sprite: false,
            })
            .collect();
        let fixtures = cluster_sources(sources);
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].actor_indices, (0..9).collect::<Vec<_>>());
        assert_eq!(
            fixtures[0]
                .sources
                .iter()
                .map(|source| source.actor_index)
                .collect::<Vec<_>>(),
            vec![0, 3, 6]
        );
        assert_eq!(fixture_energy_scale(&fixtures[0]), 1.0 / 9.0);
        assert_eq!(fixtures[0].sources[0].radius, DENSE_FIXTURE_RADIUS);
        assert_eq!(fixtures[0].sources[0].color, Vec3::splat(1.0 / 3.0));
    }

    #[test]
    fn authored_volumes_do_not_join_visible_fixture_clusters() {
        let sources = [
            PointSource {
                actor_index: 1,
                position: Vec3::ZERO,
                color: Vec3::ONE,
                radius: 300.0,
                fixture_emitter: true,
                source_sprite: false,
            },
            PointSource {
                actor_index: 2,
                position: Vec3::ZERO,
                color: Vec3::ONE,
                radius: 300.0,
                fixture_emitter: false,
                source_sprite: false,
            },
        ];
        let fixtures = cluster_sources(sources.into());
        assert_eq!(fixtures.len(), 2);
        assert!(
            fixtures
                .iter()
                .all(|fixture| fixture.actor_indices.len() == 1)
        );
    }

    #[test]
    #[ignore = "requires local original game files"]
    fn tut1_fixture_types_keep_distinct_shadow_profiles() {
        let level =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/Maps/Lev_Tut1.unr");
        let scene = openhp1_scene::LoadedScene::load(level).unwrap();
        let fixtures = point_fixtures(&scene.render);
        let chandelier = fixtures
            .iter()
            .find(|fixture| fixture.actor_indices.len() >= 20)
            .unwrap();
        let candleholder = fixtures
            .iter()
            .find(|fixture| {
                fixture.actor_indices.len() == 3
                    && fixture.sources.iter().all(|source| source.source_sprite)
            })
            .unwrap();
        let candle = fixtures
            .iter()
            .find(|fixture| fixture.actor_indices.len() == 1 && fixture.sources[0].source_sprite)
            .unwrap();
        let corona_fixture = fixtures
            .iter()
            .find(|fixture| fixture.actor_indices.len() == 1 && !fixture.sources[0].source_sprite)
            .unwrap();

        assert_eq!(chandelier.sources.len(), MAX_FIXTURE_SAMPLES);
        assert_eq!(chandelier.sources[0].radius, DENSE_FIXTURE_RADIUS);
        assert!(fixture_energy_scale(chandelier) <= 0.05);
        assert_eq!(candleholder.sources.len(), 3);
        assert_eq!(candleholder.sources[0].radius, 50.0);
        assert_eq!(fixture_energy_scale(candleholder), 1.0 / 3.0);
        assert_eq!(candle.sources[0].radius, 50.0);
        assert_eq!(fixture_energy_scale(candle), 1.0 / 3.0);
        assert_eq!(corona_fixture.sources[0].radius, MAX_SCATTERING_RADIUS);
        assert_eq!(fixture_energy_scale(corona_fixture), 1.0);
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
        assert_eq!(size_of::<FaceUniform>(), 80);
    }
}
