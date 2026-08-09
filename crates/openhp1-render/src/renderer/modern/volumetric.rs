use std::{
    collections::{HashMap, HashSet},
    mem::size_of,
};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use openhp1_scene::{RenderLight, RenderScene, TextureImage};
use wgpu::util::DeviceExt;

use crate::{Camera, VolumetricDebugView, VolumetricTuning};

use super::HDR_FORMAT;

mod froxel;
mod point_shadow;
mod shadow;

use froxel::FroxelVolume;
use point_shadow::{MAX_POINT_SHADOWS, PointShadowRenderer, fixture_energy_scales, point_fixtures};
use shadow::DirectionalShadow;

const SHADER: &str = concat!(
    include_str!("../../shaders/modern/volumetric_noise.wgsl"),
    include_str!("../../shaders/modern/volumetric.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VolumetricUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    camera_position: [f32; 4],
    camera_forward: [f32; 4],
    projection: [f32; 4],
    haze: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct VolumetricInstance {
    position_radius: [f32; 4],
    color_fog: [f32; 4],
    profile: [f32; 4],
}

pub(super) struct VolumetricRenderer {
    shadow: DirectionalShadow,
    froxel: FroxelVolume,
    point_shadows: PointShadowRenderer,
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    instance_actor_indices: Vec<usize>,
    instances: Vec<VolumetricInstance>,
    instance_count: usize,
    point_volume_buffer: wgpu::Buffer,
    point_volume_count: usize,
    texture_colors: HashMap<usize, Vec3>,
    tuning: VolumetricTuning,
}

impl VolumetricRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport_size: [u32; 2],
        depth_view: &wgpu::TextureView,
        scene: &RenderScene,
    ) -> Self {
        let texture_colors = source_texture_colors(scene);
        let fixtures = point_fixtures(scene, &texture_colors);
        let shadow = DirectionalShadow::new(device, queue, depth_view, scene);
        let froxel = FroxelVolume::new(device, viewport_size, depth_view, &shadow);
        let point_shadows = PointShadowRenderer::new(device, fixtures.clone());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 volumetric lighting camera"),
            size: size_of::<VolumetricUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 volumetric lighting layout"),
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let bind_group = bind_group(
            device,
            &layout,
            depth_view,
            &uniform,
            &point_shadows.view,
            &point_shadows.sampler,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 volumetric lighting shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 volumetric lighting pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = pipeline(device, &pipeline_layout, &shader);
        let (instance_actor_indices, instances) = instances(scene, &texture_colors, &fixtures);
        let instance_count = instances.len();
        let instance_buffer = instance_buffer(device, &instances);
        let point_volume_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 shadowed point volume instances"),
            size: (MAX_POINT_SHADOWS * size_of::<VolumetricInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tuning = VolumetricTuning::default();
        Self {
            shadow,
            froxel,
            point_shadows,
            uniform,
            layout,
            bind_group,
            pipeline,
            instance_buffer,
            instance_actor_indices,
            instances,
            instance_count,
            point_volume_buffer,
            point_volume_count: 0,
            texture_colors,
            tuning,
        }
    }

    pub(super) fn set_tuning(&mut self, tuning: VolumetricTuning) {
        self.tuning = tuning;
        self.shadow.set_tuning(tuning);
    }

    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        viewport_size: [u32; 2],
        depth_view: &wgpu::TextureView,
    ) {
        self.bind_group = bind_group(
            device,
            &self.layout,
            depth_view,
            &self.uniform,
            &self.point_shadows.view,
            &self.point_shadows.sampler,
        );
        self.shadow.resize(device, depth_view);
        self.froxel
            .resize(device, viewport_size, depth_view, &self.shadow);
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        refresh_source_texture_colors(&mut self.texture_colors, scene);
        let fixtures = point_fixtures(scene, &self.texture_colors);
        let (instance_actor_indices, instances) = instances(scene, &self.texture_colors, &fixtures);
        if instances.len() != self.instances.len() {
            return false;
        }
        self.instance_actor_indices = instance_actor_indices;
        self.instances = instances;
        let Some(shadow_changes) = self.shadow.update(queue, scene) else {
            return false;
        };
        self.point_shadows.update(fixtures, shadow_changes);
        true
    }

    pub(super) fn update_textures(&mut self, textures: &[TextureImage], changed: &[usize]) -> bool {
        update_source_texture_colors(&mut self.texture_colors, textures, changed)
    }

    pub(super) fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        viewport_size: [u32; 2],
        elapsed_time: f32,
    ) {
        let aspect = viewport_size[0] as f32 / viewport_size[1].max(1) as f32;
        let view_projection = camera.view_projection(aspect);
        let tan_half_fov = (camera.vertical_fov * 0.5).tan();
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&VolumetricUniform {
                view_projection: view_projection.to_cols_array_2d(),
                inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
                camera_position: camera
                    .position
                    .extend(self.tuning.debug_view.shader_id() as f32)
                    .to_array(),
                camera_forward: camera.forward().extend(0.0).to_array(),
                projection: [
                    tan_half_fov * aspect,
                    tan_half_fov,
                    camera.near,
                    elapsed_time,
                ],
                haze: [
                    self.tuning.haze_size,
                    self.tuning.haze_density,
                    self.tuning.haze_opacity,
                    self.tuning.haze_speed,
                ],
            }),
        );
        self.shadow
            .prepare(queue, camera, aspect, viewport_size, elapsed_time);
        self.froxel.prepare(
            queue,
            camera,
            aspect,
            elapsed_time,
            self.tuning,
            &self.shadow,
        );
        let (shadowed_actor_indices, point_volumes) =
            self.point_shadows.prepare(queue, camera, aspect);
        let instances = unshadowed_instances(
            &self.instance_actor_indices,
            &self.instances,
            &shadowed_actor_indices,
        );
        self.instance_count = instances.len();
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        self.point_volume_count = point_volumes.len();
        if !point_volumes.is_empty() {
            queue.write_buffer(
                &self.point_volume_buffer,
                0,
                bytemuck::cast_slice(&point_volumes),
            );
        }
    }

    pub(super) fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) -> usize {
        let debug_view = self.tuning.debug_view;
        let directional = debug_view != VolumetricDebugView::LocalVisibility;
        let local = !matches!(
            debug_view,
            VolumetricDebugView::ApertureMask | VolumetricDebugView::DirectionalVisibility
        );
        let shadow_passes = if directional {
            self.shadow.render(encoder)
        } else {
            0
        };
        let point_shadow_passes = if local {
            self.point_shadows.render(encoder, &self.shadow)
        } else {
            0
        };
        let draw_froxel = self.froxel.has_scattering()
            && matches!(
                debug_view,
                VolumetricDebugView::Composite | VolumetricDebugView::Scattering
            );
        let froxel_passes = if draw_froxel {
            self.froxel.compute(encoder)
        } else {
            0
        };
        let draw_shafts = directional && self.shadow.has_visible_shafts();
        let draw_local = local && (self.instance_count != 0 || self.point_volume_count != 0);
        if !draw_froxel && !draw_shafts && !draw_local {
            return shadow_passes + point_shadow_passes + froxel_passes;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 additive volumetric lighting pass"),
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
        if draw_froxel {
            self.froxel.draw(&mut pass);
        }
        if draw_shafts {
            self.shadow.draw_shafts(&mut pass);
        }
        if draw_local {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            if self.instance_count != 0 && debug_view != VolumetricDebugView::LocalVisibility {
                pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                pass.draw(0..6, 0..self.instance_count as u32);
            }
            if self.point_volume_count != 0 {
                pass.set_vertex_buffer(0, self.point_volume_buffer.slice(..));
                let count = if debug_view == VolumetricDebugView::LocalVisibility {
                    1
                } else {
                    self.point_volume_count
                };
                pass.draw(0..6, 0..count as u32);
            }
        }
        shadow_passes + point_shadow_passes + froxel_passes + 1
    }
}

fn instances(
    scene: &RenderScene,
    texture_colors: &HashMap<usize, Vec3>,
    fixtures: &[point_shadow::PointFixture],
) -> (Vec<usize>, Vec<VolumetricInstance>) {
    let mut seen = HashSet::new();
    let fixture_energy_scales = fixture_energy_scales(fixtures);
    let corona_lights = scene
        .coronas
        .iter()
        .map(|corona| (corona.actor_index, corona))
        .collect::<HashMap<_, _>>();
    scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .filter(|light| {
            light.actor_index != usize::MAX
                && (light.brightness != 0
                    || light.source_texture.is_some()
                    || corona_lights.contains_key(&light.actor_index))
                && light.effect != 4
                && (light.source_texture.is_some()
                    || corona_lights.contains_key(&light.actor_index)
                    || (light.volume_radius != 0 && light.volume_brightness != 0))
                && seen.insert(light.actor_index)
        })
        .map(|light| {
            let corona = corona_lights.get(&light.actor_index).copied();
            let sprite_color = light
                .source_texture
                .and_then(|texture| texture_colors.get(&texture))
                .copied();
            (
                light.actor_index,
                instance(
                    light,
                    corona,
                    sprite_color,
                    fixture_energy_scales
                        .get(&light.actor_index)
                        .copied()
                        .unwrap_or(1.0),
                ),
            )
        })
        .unzip()
}

fn unshadowed_instances(
    actor_indices: &[usize],
    instances: &[VolumetricInstance],
    shadowed_actor_indices: &[usize],
) -> Vec<VolumetricInstance> {
    actor_indices
        .iter()
        .zip(instances)
        .filter_map(|(actor_index, instance)| {
            (!shadowed_actor_indices.contains(actor_index)).then_some(*instance)
        })
        .collect()
}

fn instance(
    light: &RenderLight,
    corona: Option<&openhp1_scene::Corona>,
    sprite_color: Option<Vec3>,
    energy_scale: f32,
) -> VolumetricInstance {
    let authored = light.volume_radius != 0 && light.volume_brightness != 0;
    let sprite = light.source_texture.is_some() && corona.is_none();
    let radius = volume_radius(light, corona);
    let (color, fog, profile) = if authored {
        let brightness =
            f32::from(light.brightness) / 255.0 * f32::from(light.volume_brightness) / 64.0 * 5.0;
        (
            light.color() * brightness,
            f32::from(light.volume_fog) / 255.0,
            0.0,
        )
    } else {
        let strength = if sprite { 0.002 } else { 0.02 };
        (
            corona.map_or_else(
                || sprite_color.unwrap_or_else(|| light.source_color()),
                |corona| corona.color,
            ) * strength,
            0.0,
            1.0,
        )
    };
    VolumetricInstance {
        position_radius: [light.location.x, light.location.y, light.location.z, radius],
        color_fog: (color * energy_scale).extend(fog).to_array(),
        profile: [profile, 0.0, 0.0, 0.0],
    }
}

fn volume_radius(light: &RenderLight, corona: Option<&openhp1_scene::Corona>) -> f32 {
    if light.volume_radius != 0 && light.volume_brightness != 0 {
        (f32::from(light.volume_radius) + 1.0) * 25.0
    } else if light.source_texture.is_some() && corona.is_none() {
        50.0
    } else {
        (75.0 * corona.map_or(1.0, |corona| corona.draw_scale.abs())).clamp(50.0, 150.0)
    }
}

fn source_texture_colors(scene: &RenderScene) -> HashMap<usize, Vec3> {
    let mut colors = HashMap::new();
    refresh_source_texture_colors(&mut colors, scene);
    colors
}

fn refresh_source_texture_colors(colors: &mut HashMap<usize, Vec3>, scene: &RenderScene) {
    let indices = scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .filter_map(|light| light.source_texture)
        .collect::<HashSet<_>>();
    colors.retain(|index, _| indices.contains(index));
    for index in indices {
        if let Some(texture) = scene.textures.get(index) {
            colors
                .entry(index)
                .or_insert_with(|| texture_light_color(texture));
        }
    }
}

fn update_source_texture_colors(
    colors: &mut HashMap<usize, Vec3>,
    textures: &[TextureImage],
    changed: &[usize],
) -> bool {
    for &index in changed {
        if let Some(color) = colors.get_mut(&index) {
            let Some(texture) = textures.get(index) else {
                return false;
            };
            *color = texture_light_color(texture);
        }
    }
    true
}

fn texture_light_color(texture: &TextureImage) -> Vec3 {
    let chroma = texture.rgba.chunks_exact(4).fold(Vec3::ZERO, |sum, pixel| {
        let rgb = Vec3::new(pixel[0] as f32, pixel[1] as f32, pixel[2] as f32) / 255.0;
        let alpha = pixel[3] as f32 / 255.0;
        sum + (rgb - Vec3::splat(rgb.min_element())) * alpha * rgb.max_element()
    });
    let peak = chroma.max_element();
    if peak > 0.001 {
        chroma / peak
    } else {
        Vec3::ONE
    }
}

fn instance_buffer(device: &wgpu::Device, instances: &[VolumetricInstance]) -> wgpu::Buffer {
    let fallback = VolumetricInstance::zeroed();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("OpenHP1 volumetric light instances"),
        contents: if instances.is_empty() {
            bytemuck::bytes_of(&fallback)
        } else {
            bytemuck::cast_slice(instances)
        },
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
    uniform: &wgpu::Buffer,
    point_shadows: &wgpu::TextureView,
    point_shadow_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 volumetric lighting bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(point_shadows),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(point_shadow_sampler),
            },
        ],
    })
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 volumetric lighting pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_volume"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<VolumetricInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                ],
            }],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_volume"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
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
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use openhp1_scene::{Corona, LightVisibility, RenderLightmap, SurfaceMaterial, TriangleMesh};

    use super::*;

    fn light(actor_index: usize, volume_radius: u8) -> RenderLight {
        RenderLight {
            actor_index,
            source_texture: None,
            location: Vec3::new(1.0, 2.0, 3.0),
            direction: -Vec3::Z,
            effect: 0,
            brightness: 64,
            hue: 10,
            saturation: 20,
            radius: 7,
            cone: 128,
            volume_brightness: 64,
            volume_fog: 128,
            volume_radius,
            visibility: LightVisibility {
                width: 1,
                height: 1,
                values: vec![255],
            },
        }
    }

    #[test]
    fn packs_authored_and_fallback_lights_once() {
        let mut zero_brightness_corona = light(4, 0);
        zero_brightness_corona.brightness = 0;
        let mut visible_sprite = light(7, 0);
        visible_sprite.brightness = 0;
        visible_sprite.source_texture = Some(0);
        let mut large_fallback = light(5, 0);
        large_fallback.radius = 255;
        let scene = RenderScene {
            mesh: TriangleMesh::default(),
            textures: vec![TextureImage {
                width: 2,
                height: 1,
                rgba: vec![255, 255, 255, 255, 255, 128, 0, 255],
            }],
            lightmaps: Vec::new(),
            realtime_lightmaps: vec![
                RenderLightmap {
                    ambient: Vec3::ZERO,
                    lights: vec![
                        light(3, 7),
                        zero_brightness_corona,
                        light(6, 0),
                        large_fallback,
                        visible_sprite,
                    ],
                },
                RenderLightmap {
                    ambient: Vec3::ZERO,
                    lights: vec![light(3, 7)],
                },
            ],
            coronas: vec![
                Corona {
                    actor_index: 4,
                    location: Vec3::ZERO,
                    texture: 0,
                    draw_scale: 1.0,
                    color: Vec3::ONE,
                },
                Corona {
                    actor_index: 5,
                    location: Vec3::ZERO,
                    texture: 0,
                    draw_scale: 1.0,
                    color: Vec3::ONE,
                },
            ],
            surface_materials: Vec::<SurfaceMaterial>::new(),
            sky_zone: None,
        };
        let texture_colors = source_texture_colors(&scene);
        let fixtures = point_fixtures(&scene, &texture_colors);
        let (actor_indices, instances) = instances(&scene, &texture_colors, &fixtures);
        assert_eq!(actor_indices, [3, 4, 5, 7]);
        assert_eq!(instances.len(), 4);
        assert_eq!(&instances[0].position_radius[..3], &[1.0, 2.0, 3.0]);
        assert_eq!(instances[0].position_radius[3], 200.0);
        assert!((instances[0].color_fog[3] - 128.0 / 255.0).abs() < 0.0001);
        assert_eq!(instances[0].profile[0], 0.0);
        assert_eq!(instances[1].position_radius[3], 75.0);
        assert_eq!(&instances[1].color_fog[..3], &[0.02; 3]);
        assert_eq!(instances[1].color_fog[3], 0.0);
        assert_eq!(instances[1].profile[0], 1.0);
        assert_eq!(instances[2].position_radius[3], 75.0);
        assert_eq!(instances[3].position_radius[3], 50.0);
        assert_eq!(instances[3].color_fog[0], 0.002);
        assert!(instances[3].color_fog[1] > 0.001);
        assert_eq!(instances[3].color_fog[2], 0.0);
        assert_eq!(instances[3].profile[0], 1.0);
    }

    #[test]
    fn shadowed_lights_do_not_keep_an_unshadowed_volume() {
        let instances = [
            VolumetricInstance {
                position_radius: [1.0; 4],
                color_fog: [1.0; 4],
                profile: [1.0; 4],
            },
            VolumetricInstance {
                position_radius: [2.0; 4],
                color_fog: [2.0; 4],
                profile: [2.0; 4],
            },
        ];
        let unshadowed = unshadowed_instances(&[3, 7], &instances, &[7]);
        assert_eq!(unshadowed.len(), 1);
        assert_eq!(unshadowed[0].position_radius, [1.0; 4]);
    }

    #[test]
    fn refreshes_only_reported_source_texture_colors() {
        let textures = [TextureImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        }];
        let mut colors = HashMap::from([(0, Vec3::ZERO)]);
        assert!(update_source_texture_colors(&mut colors, &textures, &[]));
        assert_eq!(colors[&0], Vec3::ZERO);
        assert!(update_source_texture_colors(&mut colors, &textures, &[0]));
        assert_eq!(colors[&0], Vec3::X);
    }

    #[test]
    fn shader_and_gpu_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<VolumetricUniform>(), 192);
        assert_eq!(size_of::<VolumetricInstance>(), 48);
    }

    #[test]
    #[ignore = "requires local original game files"]
    fn local_corona_sources_reach_volumetric_lights() {
        let level =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/Maps/Lev_Tut1.unr");
        let scene = openhp1_scene::LoadedScene::load(level).unwrap();
        assert!(!scene.render.coronas.is_empty());
        assert!(
            scene
                .render
                .realtime_lightmaps
                .iter()
                .flat_map(|lightmap| &lightmap.lights)
                .any(|light| light.source_texture.is_some() && light.brightness == 0)
        );
        let colors = source_texture_colors(&scene.render);
        let fixtures = point_fixtures(&scene.render, &colors);
        assert!(instances(&scene.render, &colors, &fixtures).1.len() >= scene.render.coronas.len());
    }
}
