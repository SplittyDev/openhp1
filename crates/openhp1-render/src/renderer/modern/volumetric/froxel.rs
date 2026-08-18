use std::mem::size_of;

use bytemuck::{Pod, Zeroable};

use crate::{Camera, VolumetricTuning};

use super::{HDR_FORMAT, shadow::DirectionalShadow};

const TILE_SIZE: u32 = 8;
const DEPTH_SLICES: u32 = 64;
const SHADER: &str = concat!(
    include_str!("../../../shaders/modern/volumetric_noise.wgsl"),
    include_str!("../../../shaders/modern/froxel_volumetric.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FroxelUniform {
    inverse_view_projection: [[f32; 4]; 4],
    light_view_projections: [[[f32; 4]; 4]; 4],
    camera_position_time: [f32; 4],
    volume_size_portals: [u32; 4],
    distance_density: [f32; 4],
    haze: [f32; 4],
    shaft: [f32; 4],
}

pub(super) struct FroxelVolume {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    compute_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    compute_bind_group: wgpu::BindGroup,
    composite_uniform_bind_group: wgpu::BindGroup,
    composite_bind_group: wgpu::BindGroup,
    compute_pipeline: wgpu::ComputePipeline,
    composite_pipeline: wgpu::RenderPipeline,
    size: [u32; 3],
    portal_count: u32,
}

impl FroxelVolume {
    pub(super) fn new(
        device: &wgpu::Device,
        viewport_size: [u32; 2],
        scene_depth: &wgpu::TextureView,
        shadow: &DirectionalShadow,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OpenHP1 froxel settings"),
            size: size_of::<FroxelUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let compute_layout = compute_layout(device);
        let composite_uniform_layout = composite_uniform_layout(device);
        let composite_layout = composite_layout(device);
        let composite_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 froxel composite settings bind group"),
            layout: &composite_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OpenHP1 froxel sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let size = froxel_size(viewport_size);
        let (texture, view) = volume_texture(device, size);
        let compute_bind_group =
            compute_bind_group(device, &compute_layout, &uniform, shadow, &view);
        let composite_bind_group =
            composite_bind_group(device, &composite_layout, scene_depth, &view, &sampler);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 froxel volumetric shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OpenHP1 froxel compute pipeline layout"),
                bind_group_layouts: &[Some(&compute_layout)],
                immediate_size: 0,
            });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("OpenHP1 froxel composite pipeline layout"),
                bind_group_layouts: &[Some(&composite_uniform_layout), Some(&composite_layout)],
                immediate_size: 0,
            });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("OpenHP1 froxel compute pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &shader,
            entry_point: Some("compute_froxel"),
            compilation_options: Default::default(),
            cache: None,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OpenHP1 froxel composite pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_froxel_composite"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_froxel_composite"),
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
        });
        Self {
            _texture: texture,
            view,
            sampler,
            uniform,
            compute_layout,
            composite_layout,
            compute_bind_group,
            composite_uniform_bind_group,
            composite_bind_group,
            compute_pipeline,
            composite_pipeline,
            size,
            portal_count: 0,
        }
    }

    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        viewport_size: [u32; 2],
        scene_depth: &wgpu::TextureView,
        shadow: &DirectionalShadow,
    ) {
        let size = froxel_size(viewport_size);
        let (texture, view) = volume_texture(device, size);
        self.compute_bind_group =
            compute_bind_group(device, &self.compute_layout, &self.uniform, shadow, &view);
        self.composite_bind_group = composite_bind_group(
            device,
            &self.composite_layout,
            scene_depth,
            &view,
            &self.sampler,
        );
        self._texture = texture;
        self.view = view;
        self.size = size;
    }

    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
        elapsed_time: f32,
        tuning: VolumetricTuning,
        shadow: &DirectionalShadow,
    ) {
        let (_, portal_count) = shadow.froxel_portals();
        self.portal_count = portal_count;
        let far = (camera.far.clamp(500.0, 3_000.0) * 0.5).min(1_500.0);
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&FroxelUniform {
                inverse_view_projection: camera
                    .view_projection(aspect)
                    .inverse()
                    .to_cols_array_2d(),
                light_view_projections: *shadow.light_view_projections(),
                camera_position_time: camera.position.extend(elapsed_time).to_array(),
                volume_size_portals: [self.size[0], self.size[1], self.size[2], portal_count],
                distance_density: [
                    camera.near.max(0.1),
                    far,
                    0.00025 * tuning.haze_density,
                    tuning.shaft_intensity,
                ],
                haze: [
                    tuning.haze_size,
                    tuning.haze_density,
                    tuning.haze_opacity,
                    tuning.haze_speed,
                ],
                shaft: [
                    tuning.shaft_anisotropy.clamp(0.0, 0.99),
                    tuning.shaft_saturation,
                    0.0,
                    0.0,
                ],
            }),
        );
    }

    pub(super) fn compute(&self, encoder: &mut wgpu::CommandEncoder) -> usize {
        if self.portal_count == 0 {
            return 0;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("OpenHP1 froxel lighting pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &self.compute_bind_group, &[]);
        pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(8), 1);
        1
    }

    pub(super) fn has_scattering(&self) -> bool {
        self.portal_count != 0
    }

    pub(super) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &self.composite_uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.composite_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn froxel_size(viewport_size: [u32; 2]) -> [u32; 3] {
    [
        viewport_size[0].max(1).div_ceil(TILE_SIZE),
        viewport_size[1].max(1).div_ceil(TILE_SIZE),
        DEPTH_SLICES,
    ]
}

fn volume_texture(device: &wgpu::Device, size: [u32; 3]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 integrated froxel volume"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("OpenHP1 integrated froxel volume"),
        dimension: Some(wgpu::TextureViewDimension::D3),
        ..Default::default()
    });
    (texture, view)
}

fn compute_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("OpenHP1 froxel compute layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D3,
                },
                count: None,
            },
        ],
    })
}

fn composite_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("OpenHP1 froxel composite layout"),
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
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn composite_uniform_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("OpenHP1 froxel composite settings layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn compute_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    shadow: &DirectionalShadow,
    volume: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let (portals, _) = shadow.froxel_portals();
    let (aperture_masks, aperture_sampler) = shadow.froxel_aperture_masks();
    let (shadow_maps, shadow_sampler) = shadow.froxel_shadow_maps();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 froxel compute bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: portals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(aperture_masks),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(aperture_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(shadow_maps),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(volume),
            },
        ],
    })
}

fn composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_depth: &wgpu::TextureView,
    volume: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 froxel composite bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene_depth),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(volume),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn froxel_shader_and_uniform_layout_are_valid() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(size_of::<FroxelUniform>(), 400);
        assert_eq!(froxel_size([1024, 768]), [128, 96, 64]);
    }
}
