use std::{mem::size_of, ops::Range};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use openhp1_scene::{Corona, CoronaVisibility};

use crate::{Camera, RenderScene, SurfaceMode, render_to_unreal, unreal_to_render};

use super::{DEPTH_FORMAT, pipeline::blend_state};

const CORONA_CACHE_CAPACITY: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CoronaCameraUniform {
    view_projection: [[f32; 4]; 4],
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CoronaInstance {
    position: [f32; 3],
    color_and_scale: [f32; 4],
}

#[derive(Debug, Eq, PartialEq)]
struct CoronaBatch {
    texture: usize,
    instances: Range<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CoronaCacheEntry {
    actor_index: Option<usize>,
    visibility: f32,
}

#[derive(Debug, Default)]
pub(super) struct CoronaCache {
    entries: [CoronaCacheEntry; CORONA_CACHE_CAPACITY],
}

pub(super) struct CoronaRenderer {
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    cache: CoronaCache,
    coronas: Vec<Corona>,
    texture_count: usize,
    visibility: CoronaVisibility,
    batches: Vec<CoronaBatch>,
    instance_count: usize,
    last_time: Option<f32>,
}

impl CoronaCache {
    fn take_from(&mut self, previous: &mut Self) {
        *self = std::mem::take(previous);
    }

    pub(super) fn update(&mut self, delta_time: f32, candidates: &[usize]) {
        let step = if delta_time.is_finite() {
            delta_time.max(0.0) * 3.0
        } else {
            0.0
        };
        for entry in &mut self.entries {
            if entry.actor_index.is_some() {
                entry.visibility -= step;
                if entry.visibility < 0.0 {
                    *entry = CoronaCacheEntry::default();
                }
            }
        }
        for &actor_index in candidates {
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.actor_index == Some(actor_index))
            {
                entry.visibility = (entry.visibility + step * 2.0).min(1.0);
                continue;
            }
            let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.actor_index.is_none())
            else {
                continue;
            };
            entry.actor_index = Some(actor_index);
            entry.visibility = (step * 2.0).min(1.0);
        }
    }

    pub(super) fn draw_records(&self) -> Vec<(usize, f32)> {
        self.entries
            .iter()
            .filter_map(|entry| {
                entry
                    .actor_index
                    .map(|actor_index| (actor_index, entry.visibility))
            })
            .collect()
    }
}

impl CoronaRenderer {
    pub(super) fn inherit_history(&mut self, previous: &mut Self) {
        self.cache.take_from(&mut previous.cache);
        self.last_time = previous.last_time;
    }

    pub(super) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        modern: bool,
        scene: &RenderScene,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 corona camera"),
            size: size_of::<CoronaCameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 corona camera layout"),
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
            label: Some("OpenHP1 corona camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 corona pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/corona.wgsl"));
        let pipeline = corona_pipeline(device, target_format, modern, &pipeline_layout, &shader);
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 corona instances"),
            size: (CORONA_CACHE_CAPACITY * size_of::<CoronaInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            camera_buffer,
            camera_bind_group,
            pipeline,
            instance_buffer,
            cache: CoronaCache::default(),
            coronas: scene.coronas.clone(),
            texture_count: scene.textures.len(),
            visibility: scene.corona_visibility.clone(),
            batches: Vec::new(),
            instance_count: 0,
            last_time: None,
        }
    }

    pub(super) fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        viewport_actor_location: Vec3,
        viewport_size: [u32; 2],
        elapsed_time: f32,
    ) {
        let aspect = viewport_size[0] as f32 / viewport_size[1].max(1) as f32;
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&CoronaCameraUniform {
                view_projection: camera.view_projection(aspect).to_cols_array_2d(),
                viewport: [viewport_size[0] as f32, viewport_size[1] as f32, 0.0, 0.0],
            }),
        );
        let candidates = corona_candidates(
            &self.coronas,
            &self.visibility,
            self.visibility.leaf_at(viewport_actor_location),
            camera,
            aspect,
        );
        let delta_time = self
            .last_time
            .map_or(0.0, |last| (elapsed_time - last).max(0.0));
        self.last_time = Some(elapsed_time);
        self.cache.update(delta_time, &candidates);
        let (instances, batches) =
            corona_instances(&self.coronas, self.texture_count, camera, &self.cache);
        self.instance_count = instances.len();
        self.batches = batches;
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
    }

    pub(super) fn update_scene(&mut self, scene: &RenderScene) {
        self.coronas.clone_from(&scene.coronas);
        self.texture_count = scene.textures.len();
        self.visibility = scene.corona_visibility.clone();
    }

    pub(super) fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        texture_bind_groups: &'pass [wgpu::BindGroup],
    ) -> usize {
        if self.instance_count == 0 {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        for batch in &self.batches {
            pass.set_bind_group(1, &texture_bind_groups[batch.texture], &[]);
            pass.draw(0..6, batch.instances.clone());
        }
        self.batches.len()
    }
}

fn corona_candidates(
    coronas: &[Corona],
    visibility: &CoronaVisibility,
    viewport_actor_leaf: Option<usize>,
    camera: &Camera,
    aspect: f32,
) -> Vec<usize> {
    let camera_location = render_to_unreal(camera.position);
    let Some(leaf) = viewport_actor_leaf else {
        return Vec::new();
    };
    let mut static_candidates = coronas
        .iter()
        .filter_map(|corona| {
            corona
                .static_leaf_orders
                .iter()
                .find(|(candidate_leaf, _)| *candidate_leaf == leaf)
                .map(|(_, order)| (*order, corona.actor_index))
        })
        .collect::<Vec<_>>();
    static_candidates.sort_unstable_by_key(|(order, _)| *order);
    let mut candidates = static_candidates
        .into_iter()
        .map(|(_, actor_index)| actor_index)
        .collect::<Vec<_>>();
    candidates.extend(
        coronas
            .iter()
            .filter(|corona| {
                corona.texture.is_some()
                    && corona.light_brightness != 0
                    && corona.dynamic_leaves.contains(&leaf)
                    && corona.dynamic_admission_radius.is_some_and(|radius| {
                        sphere_intersects_view(camera, aspect, corona.location, radius)
                    })
            })
            .map(|corona| corona.actor_index),
    );
    candidates.retain(|actor_index| {
        coronas
            .iter()
            .find(|corona| corona.actor_index == *actor_index)
            .is_some_and(|corona| {
                corona.texture.is_some() && visibility.line_clear(camera_location, corona.location)
            })
    });
    candidates
}

fn sphere_intersects_view(camera: &Camera, aspect: f32, location: Vec3, radius: f32) -> bool {
    let view = camera.view().transform_point3(unreal_to_render(location));
    let z = -view.z;
    let vertical = (camera.vertical_fov * 0.5).tan();
    let horizontal = vertical * aspect;
    let horizontal_radius = radius * (1.0 + horizontal * horizontal).sqrt();
    let vertical_radius = radius * (1.0 + vertical * vertical).sqrt();
    view.x + z * horizontal >= -horizontal_radius
        && -view.x + z * horizontal >= -horizontal_radius
        && view.y + z * vertical >= -vertical_radius
        && -view.y + z * vertical >= -vertical_radius
}

fn corona_instances(
    coronas: &[Corona],
    texture_count: usize,
    camera: &Camera,
    cache: &CoronaCache,
) -> (Vec<CoronaInstance>, Vec<CoronaBatch>) {
    let mut entries = Vec::new();
    for (actor_index, visibility) in cache.draw_records() {
        let Some(corona) = coronas
            .iter()
            .find(|corona| corona.actor_index == actor_index)
        else {
            continue;
        };
        let Some(texture) = corona.texture else {
            continue;
        };
        let position = unreal_to_render(corona.location);
        if -camera.view().transform_point3(position).z <= 1.0 {
            continue;
        }
        entries.push((
            texture.min(texture_count),
            CoronaInstance {
                position: position.to_array(),
                color_and_scale: [
                    corona.color.x * visibility,
                    corona.color.y * visibility,
                    corona.color.z * visibility,
                    corona.draw_scale,
                ],
            },
        ));
    }
    let mut batches: Vec<CoronaBatch> = Vec::new();
    for (index, (texture, _)) in entries.iter().enumerate() {
        if let Some(batch) = batches.last_mut()
            && batch.texture == *texture
        {
            batch.instances.end += 1;
        } else {
            batches.push(CoronaBatch {
                texture: *texture,
                instances: index as u32..index as u32 + 1,
            });
        }
    }
    (
        entries.into_iter().map(|(_, instance)| instance).collect(),
        batches,
    )
}

fn corona_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    modern: bool,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 corona pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_corona"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<CoronaInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
            }],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if modern {
                "fragment_corona_modern"
            } else {
                "fragment_corona_classic"
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: blend_state(SurfaceMode::Translucent),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_coronas_ramp_up_and_missing_coronas_fade_out() {
        let mut cache = CoronaCache::default();

        cache.update(0.1, &[7]);
        assert!((cache.draw_records()[0].1 - 0.6).abs() < f32::EPSILON * 2.0);

        cache.update(0.1, &[7]);
        assert!((cache.draw_records()[0].1 - 0.9).abs() < f32::EPSILON * 2.0);

        cache.update(0.1, &[]);
        assert!((cache.draw_records()[0].1 - 0.6).abs() < f32::EPSILON * 2.0);

        cache.update(0.3, &[]);
        assert!(cache.draw_records().is_empty());
    }

    #[test]
    fn corona_cache_survives_renderer_resource_reload() {
        let mut previous = CoronaCache::default();
        previous.update(0.1, &[7]);
        let mut replacement = CoronaCache::default();

        replacement.take_from(&mut previous);

        assert_eq!(replacement.draw_records(), [(7, 0.6)]);
        assert!(previous.draw_records().is_empty());
    }

    #[test]
    fn missing_viewport_actor_leaf_fades_instead_of_clearing_the_cache() {
        let mut cache = CoronaCache::default();
        cache.update(0.1, &[7]);
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 100.0);

        let candidates =
            corona_candidates(&[], &CoronaVisibility::default(), None, &camera, 4.0 / 3.0);
        cache.update(0.1, &candidates);

        let records = cache.draw_records();
        assert_eq!(records[0].0, 7);
        assert!((records[0].1 - 0.3).abs() < f32::EPSILON * 2.0);
    }

    #[test]
    fn static_candidates_use_the_viewport_actor_leaf_across_camera_changes() {
        let coronas = [Corona {
            actor_index: 7,
            location: Vec3::new(100.0, 0.0, 0.0),
            texture: Some(0),
            draw_scale: 1.0,
            color: Vec3::ONE,
            static_leaf_orders: vec![(3, 0)],
            dynamic_light_radius: None,
            dynamic_admission_radius: None,
            dynamic_leaves: Vec::new(),
            light_brightness: 0,
        }];
        let visibility = CoronaVisibility::default();
        let normal = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 100.0);
        let aiming = Camera::looking_at(Vec3::new(50.0, 20.0, 10.0), -Vec3::Z, 100.0);

        assert_eq!(
            corona_candidates(&coronas, &visibility, Some(3), &normal, 4.0 / 3.0),
            [7]
        );
        assert_eq!(
            corona_candidates(&coronas, &visibility, Some(3), &aiming, 4.0 / 3.0),
            [7]
        );
    }

    #[test]
    fn static_coronas_are_also_scanned_through_dynamic_leaf_lights() {
        let coronas = [Corona {
            actor_index: 7,
            location: Vec3::new(100.0, 0.0, 0.0),
            texture: Some(0),
            draw_scale: 1.0,
            color: Vec3::ONE,
            static_leaf_orders: vec![(2, 0)],
            dynamic_light_radius: Some(200.0),
            dynamic_admission_radius: Some(200.0),
            dynamic_leaves: vec![3],
            light_brightness: 255,
        }];
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 100.0);

        assert_eq!(
            corona_candidates(
                &coronas,
                &CoronaVisibility::default(),
                Some(3),
                &camera,
                4.0 / 3.0,
            ),
            [7]
        );
    }

    #[test]
    fn cache_capacity_and_draw_records_match_the_native_limits() {
        let mut cache = CoronaCache::default();
        cache.update(0.1, &(0..33).collect::<Vec<_>>());
        assert_eq!(cache.draw_records().len(), 32);
        assert!(!cache.draw_records().iter().any(|(actor, _)| *actor == 32));

        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 100.0);
        cache = CoronaCache::default();
        cache.update(0.1, &[7, 8, 99]);
        let coronas = vec![
            Corona {
                actor_index: 7,
                location: Vec3::new(10.0, 0.0, 0.0),
                texture: Some(3),
                draw_scale: 0.25,
                color: Vec3::new(0.5, 1.0, 0.25),
                static_leaf_orders: Vec::new(),
                dynamic_light_radius: None,
                dynamic_admission_radius: None,
                dynamic_leaves: Vec::new(),
                light_brightness: 0,
            },
            Corona {
                actor_index: 8,
                location: Vec3::new(0.5, 0.0, 0.0),
                texture: Some(0),
                draw_scale: 1.0,
                color: Vec3::ONE,
                static_leaf_orders: Vec::new(),
                dynamic_light_radius: None,
                dynamic_admission_radius: None,
                dynamic_leaves: Vec::new(),
                light_brightness: 0,
            },
        ];
        let (instances, batches) = corona_instances(&coronas, 1, &camera, &cache);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].position, [0.0, 0.0, -10.0]);
        assert_eq!(instances[0].color_and_scale, [0.3, 0.6, 0.15, 0.25]);
        assert_eq!(
            batches,
            [CoronaBatch {
                texture: 1,
                instances: 0..1
            }]
        );
        assert_eq!(size_of::<CoronaCameraUniform>(), 80);
    }
}
