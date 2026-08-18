use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use openhp1_scene::{LightmapImage, RenderScene};
use wgpu::util::DeviceExt;

use super::{
    atlas::{AtlasRectangle, build_lightmap_atlas},
    pipeline::texture,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuLightmap {
    ambient: [f32; 4],
    light_range: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuLight {
    position_radius: [f32; 4],
    direction_outer: [f32; 4],
    color: [f32; 4],
    visibility: [f32; 4],
    effect: [u32; 4],
}

pub(super) struct ModernLighting {
    lightmap_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
    lightmap_count: usize,
    light_count: usize,
    visibility_rectangles: Vec<AtlasRectangle>,
    atlas_size: [u32; 2],
    memory_bytes: usize,
    _visibility_texture: wgpu::Texture,
}

impl ModernLighting {
    pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 modern lighting layout"),
            entries: &[
                storage_layout_entry(0),
                storage_layout_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        scene: &RenderScene,
    ) -> Self {
        let visibility_images = visibility_images(scene);
        let atlas =
            build_lightmap_atlas(&visibility_images, device.limits().max_texture_dimension_2d);
        let atlas_size = [atlas.image.width, atlas.image.height];
        let (lightmaps, lights) = gpu_data(scene, &atlas.rectangles, atlas_size);
        let lightmap_buffer = storage_buffer(device, "OpenHP1 modern lightmaps", &lightmaps);
        let light_buffer = storage_buffer(device, "OpenHP1 modern lights", &lights);
        let visibility_texture = texture(
            device,
            queue,
            "OpenHP1 authored visibility atlas",
            &atlas.image,
        );
        let visibility_view = visibility_texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 authored visibility sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 modern lighting bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lightmap_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&visibility_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            lightmap_buffer,
            light_buffer,
            bind_group,
            lightmap_count: lightmaps.len(),
            light_count: lights.len(),
            visibility_rectangles: atlas.rectangles,
            atlas_size,
            memory_bytes: atlas.image.rgba.len()
                + lightmaps.len() * size_of::<GpuLightmap>()
                + lights.len() * size_of::<GpuLight>(),
            _visibility_texture: visibility_texture,
        }
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }

    pub(super) fn update(&self, queue: &wgpu::Queue, scene: &RenderScene) -> bool {
        if scene.realtime_lightmaps.len() != self.lightmap_count
            || scene
                .realtime_lightmaps
                .iter()
                .map(|lightmap| lightmap.lights.len())
                .sum::<usize>()
                != self.light_count
            || scene
                .realtime_lightmaps
                .iter()
                .flat_map(|lightmap| &lightmap.lights)
                .zip(&self.visibility_rectangles)
                .any(|(light, rectangle)| {
                    light.visibility.width != rectangle.width
                        || light.visibility.height != rectangle.height
                })
        {
            return false;
        }
        let (lightmaps, lights) = gpu_data(scene, &self.visibility_rectangles, self.atlas_size);
        if lightmaps.len() != self.lightmap_count || lights.len() != self.light_count {
            return false;
        }
        queue.write_buffer(&self.lightmap_buffer, 0, bytemuck::cast_slice(&lightmaps));
        queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&lights));
        true
    }
}

fn storage_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_buffer<T: Pod>(device: &wgpu::Device, label: &str, values: &[T]) -> wgpu::Buffer {
    let bytes = bytemuck::cast_slice(values);
    let zero = T::zeroed();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: if bytes.is_empty() {
            bytemuck::bytes_of(&zero)
        } else {
            bytes
        },
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

fn visibility_images(scene: &RenderScene) -> Vec<LightmapImage> {
    scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .map(|light| LightmapImage {
            width: light.visibility.width,
            height: light.visibility.height,
            rgba: light
                .visibility
                .values
                .iter()
                .flat_map(|&value| [value, value, value, 255])
                .collect(),
        })
        .collect()
}

fn gpu_data(
    scene: &RenderScene,
    visibility_rectangles: &[AtlasRectangle],
    atlas_size: [u32; 2],
) -> (Vec<GpuLightmap>, Vec<GpuLight>) {
    let mut lights = Vec::new();
    let mut rectangle = 0;
    let lightmaps = scene
        .realtime_lightmaps
        .iter()
        .map(|lightmap| {
            let first = lights.len();
            for light in &lightmap.lights {
                let region = visibility_rectangles[rectangle];
                rectangle += 1;
                lights.push(GpuLight {
                    position_radius: [
                        light.location.x,
                        light.location.y,
                        light.location.z,
                        (f32::from(light.radius) + 1.0) * 25.0,
                    ],
                    direction_outer: [
                        light.direction.x,
                        light.direction.y,
                        light.direction.z,
                        1.0 - f32::from(light.cone) / 255.0,
                    ],
                    color: light.color().extend(0.0).to_array(),
                    visibility: [
                        region.width as f32 / atlas_size[0] as f32,
                        region.height as f32 / atlas_size[1] as f32,
                        region.x as f32 / atlas_size[0] as f32,
                        region.y as f32 / atlas_size[1] as f32,
                    ],
                    effect: [u32::from(light.effect), u32::from(light.dark), 0, 0],
                });
            }
            GpuLightmap {
                ambient: lightmap.ambient.extend(0.0).to_array(),
                light_range: [first as u32, (lights.len() - first) as u32, 0, 0],
            }
        })
        .collect();
    (lightmaps, lights)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use openhp1_scene::{
        LightVisibility, RenderLight, RenderLightmap, SurfaceMaterial, TriangleMesh,
    };

    use super::*;

    #[test]
    fn packs_authored_light_range_visibility_and_unclamped_ambient() {
        let scene = RenderScene {
            mesh: TriangleMesh::default(),
            textures: Vec::new(),
            lightmaps: Vec::new(),
            realtime_lightmaps: vec![RenderLightmap {
                ambient: Vec3::splat(1.5),
                lights: vec![RenderLight {
                    actor_index: 3,
                    source_texture: None,
                    location: Vec3::new(1.0, 2.0, 3.0),
                    direction: -Vec3::Z,
                    effect: 8,
                    brightness: 64,
                    hue: 10,
                    saturation: 20,
                    radius: 7,
                    cone: 128,
                    dark: true,
                    volume_brightness: 64,
                    volume_fog: 0,
                    volume_radius: 0,
                    visibility: LightVisibility {
                        width: 4,
                        height: 2,
                        values: vec![255; 8],
                    },
                }],
            }],
            coronas: Vec::new(),
            corona_visibility: Default::default(),
            actor_submissions: Vec::new(),
            surface_materials: Vec::<SurfaceMaterial>::new(),
            transmission_masks: Default::default(),
            warp_portals: Vec::new(),
            sky_zone: None,
        };
        let (lightmaps, lights) = gpu_data(
            &scene,
            &[AtlasRectangle {
                x: 8,
                y: 4,
                width: 4,
                height: 2,
            }],
            [32, 16],
        );

        assert_eq!(lightmaps[0].ambient[0], 1.5);
        assert_eq!(lightmaps[0].light_range, [0, 1, 0, 0]);
        assert_eq!(lights[0].position_radius, [1.0, 2.0, 3.0, 200.0]);
        assert_eq!(lights[0].visibility, [0.125, 0.125, 0.25, 0.25]);
        assert_eq!(lights[0].effect[0], 8);
        assert_eq!(lights[0].effect[1], 1);
    }
}
