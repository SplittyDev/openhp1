use super::DEPTH_FORMAT;

pub(super) struct DepthTarget {
    pub(super) view: wgpu::TextureView,
    pub(super) size: [u32; 2],
}

pub(super) struct SkyTarget {
    _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) depth: DepthTarget,
}

impl SkyTarget {
    pub(super) fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        lightmap_view: &wgpu::TextureView,
        lightmap_sampler: &wgpu::Sampler,
    ) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OpenHP1 sky-zone color"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OpenHP1 sky-zone texture bind group"),
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
            ],
        });
        Self {
            _texture: texture,
            view,
            bind_group,
            depth: DepthTarget::new(device, size),
        }
    }
}

impl DepthTarget {
    pub(super) fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            size,
        }
    }
}
