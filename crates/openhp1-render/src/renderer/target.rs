use super::DEPTH_FORMAT;

pub(super) struct DepthTarget {
    pub(super) view: wgpu::TextureView,
    pub(super) size: [u32; 2],
}

pub(super) struct SampledTarget {
    _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    bind_groups: [wgpu::BindGroup; 2],
    pub(super) depth: DepthTarget,
}

impl SampledTarget {
    pub(super) fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        texture_layout: &wgpu::BindGroupLayout,
        samplers: [&wgpu::Sampler; 2],
        lightmap_view: &wgpu::TextureView,
        lightmap_sampler: &wgpu::Sampler,
    ) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 sampled scene target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_groups = samplers.map(|sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("OpenHP1 sampled scene target bind group"),
                layout: texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(lightmap_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(lightmap_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        });
        Self {
            _texture: texture,
            view,
            bind_groups,
            depth: DepthTarget::new(device, size, false),
        }
    }

    pub(super) fn bind_group(&self, no_smooth: bool) -> &wgpu::BindGroup {
        &self.bind_groups[usize::from(no_smooth)]
    }
}

impl DepthTarget {
    pub(super) fn new(device: &wgpu::Device, size: [u32; 2], sampleable: bool) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 depth"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if sampleable {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            size,
        }
    }
}
