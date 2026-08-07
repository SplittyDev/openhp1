use std::{
    collections::{HashMap, HashSet},
    mem::size_of,
};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use openhp1_scene::{RenderLight, RenderScene, TextureImage};
use wgpu::util::DeviceExt;

use crate::Camera;

use super::HDR_FORMAT;

const SHADER: &str = include_str!("../../shaders/modern/volumetric.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VolumetricUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    camera_position: [f32; 4],
    camera_forward: [f32; 4],
    projection: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct VolumetricInstance {
    position_radius: [f32; 4],
    color_fog: [f32; 4],
    profile: [f32; 4],
}

pub(super) struct VolumetricRenderer {
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    instance_count: usize,
}

impl VolumetricRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        scene: &RenderScene,
    ) -> Self {
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
            ],
        });
        let bind_group = bind_group(device, &layout, depth_view, &uniform);
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
        let instances = instances(scene);
        let instance_buffer = instance_buffer(device, &instances);
        Self {
            uniform,
            layout,
            bind_group,
            pipeline,
            instance_buffer,
            instance_count: instances.len(),
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView) {
        self.bind_group = bind_group(device, &self.layout, depth_view, &self.uniform);
    }

    pub(super) fn update(&self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        let instances = instances(scene);
        if instances.len() != self.instance_count {
            return false;
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        true
    }

    pub(super) fn prepare_frame(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        viewport_size: [u32; 2],
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
                camera_position: camera.position.extend(1.0).to_array(),
                camera_forward: camera.forward().extend(0.0).to_array(),
                projection: [tan_half_fov * aspect, tan_half_fov, camera.near, 0.0],
            }),
        );
    }

    pub(super) fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) -> usize {
        if self.instance_count == 0 {
            return 0;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 volumetric lighting pass"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..self.instance_count as u32);
        1
    }
}

fn instances(scene: &RenderScene) -> Vec<VolumetricInstance> {
    let mut seen = HashSet::new();
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
                .and_then(|texture| scene.textures.get(texture))
                .map(texture_light_color);
            instance(light, corona, sprite_color)
        })
        .collect()
}

fn instance(
    light: &RenderLight,
    corona: Option<&openhp1_scene::Corona>,
    sprite_color: Option<Vec3>,
) -> VolumetricInstance {
    let authored = light.volume_radius != 0 && light.volume_brightness != 0;
    let sprite = light.source_texture.is_some() && corona.is_none();
    let radius = if authored {
        (f32::from(light.volume_radius) + 1.0) * 25.0
    } else if sprite {
        50.0
    } else {
        (75.0 * corona.map_or(1.0, |corona| corona.draw_scale.abs())).clamp(50.0, 150.0)
    };
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
        color_fog: color.extend(fog).to_array(),
        profile: [profile, 0.0, 0.0, 0.0],
    }
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
        let instances = instances(&scene);
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
    fn shader_and_gpu_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<VolumetricUniform>(), 176);
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
        assert!(instances(&scene.render).len() >= scene.render.coronas.len());
    }
}
