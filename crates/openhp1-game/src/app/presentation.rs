use egui::{Event, Pos2, RawInput, Rect, Vec2};

use super::graphics_settings::ColorDepth;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Destination {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    integer_scale: bool,
}

pub(super) struct Presentation {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    bind_group_layout: wgpu::BindGroupLayout,
    nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    nearest_bind_group: wgpu::BindGroup,
    linear_bind_group: wgpu::BindGroup,
    true_color_pipeline: wgpu::RenderPipeline,
    rgb565_pipeline: wgpu::RenderPipeline,
}

impl Presentation {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OpenHP1 presentation layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let nearest_sampler = sampler(device, wgpu::FilterMode::Nearest, "nearest");
        let linear_sampler = sampler(device, wgpu::FilterMode::Linear, "linear");
        let (texture, view) = target(device, format, size);
        let nearest_bind_group = bind_group(
            device,
            &bind_group_layout,
            &view,
            &nearest_sampler,
            "nearest",
        );
        let linear_bind_group =
            bind_group(device, &bind_group_layout, &view, &linear_sampler, "linear");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OpenHP1 presentation shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("presentation.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OpenHP1 presentation pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let true_color_pipeline = pipeline(
            device,
            format,
            &pipeline_layout,
            &shader,
            "fragment_true_color",
            "true color",
        );
        let rgb565_pipeline = pipeline(
            device,
            format,
            &pipeline_layout,
            &shader,
            "fragment_rgb565",
            "RGB565",
        );
        Self {
            texture,
            view,
            size: valid_size(size),
            bind_group_layout,
            nearest_sampler,
            linear_sampler,
            nearest_bind_group,
            linear_bind_group,
            true_color_pipeline,
            rgb565_pipeline,
        }
    }

    pub(super) fn size(&self) -> [u32; 2] {
        self.size
    }

    pub(super) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(super) fn resize(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) {
        let size = valid_size(size);
        if self.size == size {
            return;
        }
        let (texture, view) = target(device, format, size);
        self.nearest_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &view,
            &self.nearest_sampler,
            "nearest",
        );
        self.linear_bind_group = bind_group(
            device,
            &self.bind_group_layout,
            &view,
            &self.linear_sampler,
            "linear",
        );
        self.texture = texture;
        self.view = view;
        self.size = size;
    }

    pub(super) fn transform_input(
        &self,
        input: &mut RawInput,
        surface_size: [u32; 2],
        pixels_per_point: f32,
    ) {
        let pixels_per_point = pixels_per_point.max(0.01);
        let destination = fit(self.size, surface_size);
        let screen_size = Vec2::new(
            self.size[0] as f32 / pixels_per_point,
            self.size[1] as f32 / pixels_per_point,
        );
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, screen_size));
        let map = |position: Pos2| {
            let physical = position.to_vec2() * pixels_per_point;
            Pos2::new(
                (physical.x - destination.x as f32) * self.size[0] as f32
                    / destination.width as f32
                    / pixels_per_point,
                (physical.y - destination.y as f32) * self.size[1] as f32
                    / destination.height as f32
                    / pixels_per_point,
            )
        };
        for event in &mut input.events {
            match event {
                Event::PointerMoved(position) => *position = map(*position),
                Event::PointerButton { pos, .. } | Event::Touch { pos, .. } => *pos = map(*pos),
                Event::MouseMoved(delta) => {
                    delta.x *= self.size[0] as f32 / destination.width as f32;
                    delta.y *= self.size[1] as f32 / destination.height as f32;
                }
                _ => {}
            }
        }
    }

    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        surface_size: [u32; 2],
        color_depth: ColorDepth,
    ) {
        let destination = fit(self.size, surface_size);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OpenHP1 presentation pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            destination.x as f32,
            destination.y as f32,
            destination.width as f32,
            destination.height as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(match color_depth {
            ColorDepth::TrueColor => &self.true_color_pipeline,
            ColorDepth::Rgb565 => &self.rgb565_pipeline,
        });
        pass.set_bind_group(
            0,
            if destination.integer_scale {
                &self.nearest_bind_group
            } else {
                &self.linear_bind_group
            },
            &[],
        );
        pass.draw(0..3, 0..1);
    }
}

pub(super) fn fit(source: [u32; 2], surface: [u32; 2]) -> Destination {
    let source = valid_size(source);
    let surface = valid_size(surface);
    let (width, height) = if u64::from(surface[0]) * u64::from(source[1])
        <= u64::from(surface[1]) * u64::from(source[0])
    {
        (
            surface[0],
            ((u64::from(surface[0]) * u64::from(source[1])) / u64::from(source[0])) as u32,
        )
    } else {
        (
            ((u64::from(surface[1]) * u64::from(source[0])) / u64::from(source[1])) as u32,
            surface[1],
        )
    };
    let width = width.max(1);
    let height = height.max(1);
    let scale_x = width / source[0];
    let scale_y = height / source[1];
    Destination {
        x: (surface[0] - width) / 2,
        y: (surface[1] - height) / 2,
        width,
        height,
        integer_scale: scale_x > 0
            && scale_x == scale_y
            && width % source[0] == 0
            && height % source[1] == 0,
    }
}

fn valid_size(size: [u32; 2]) -> [u32; 2] {
    [size[0].max(1), size[1].max(1)]
}

fn target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = valid_size(size);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 internal frame"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn sampler(device: &wgpu::Device, filter: wgpu::FilterMode, name: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&format!("OpenHP1 {name} presentation sampler")),
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    name: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("OpenHP1 {name} presentation bind group")),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    name: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("OpenHP1 {name} presentation pipeline")),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_fullscreen"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_centers_without_changing_aspect() {
        assert_eq!(
            fit([640, 480], [1920, 1080]),
            Destination {
                x: 240,
                y: 0,
                width: 1440,
                height: 1080,
                integer_scale: false,
            }
        );
        assert_eq!(
            fit([640, 480], [1280, 1024]),
            Destination {
                x: 0,
                y: 32,
                width: 1280,
                height: 960,
                integer_scale: true,
            }
        );
    }
}
