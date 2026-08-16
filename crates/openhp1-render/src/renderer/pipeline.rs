use std::mem::size_of;

use crate::{SurfaceMaterial, SurfaceMode, TextureImage};

use super::{DEPTH_FORMAT, Vertex};

pub(super) fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    material: SurfaceMaterial,
    modern: bool,
    reflected: bool,
) -> wgpu::RenderPipeline {
    let blended = matches!(
        material.mode,
        SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended
    );
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 BSP pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x2,
                    2 => Float32x4,
                    3 => Float32x2,
                    4 => Float32,
                    5 => Unorm8x4,
                    6 => Float32x3,
                    7 => Float32,
                    8 => Float32x2,
                    9 => Uint32,
                    10 => Float32x2,
                    11 => Float32x3
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            // The Unreal-to-render axis conversion changes handedness, so UE
            // polygon winding becomes clockwise in render space.
            front_face: front_face(reflected),
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!blended),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry(
                material.mode,
                material.masked,
                material.unlit,
                modern,
            )),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: blend_state(material.mode),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn front_face(reflected: bool) -> wgpu::FrontFace {
    if reflected {
        wgpu::FrontFace::Ccw
    } else {
        wgpu::FrontFace::Cw
    }
}

pub(super) fn create_screen_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    _two_sided: bool,
    fragment_entry: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("OpenHP1 screen-projected surface pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x2,
                    2 => Float32x4,
                    3 => Float32x2,
                    4 => Float32,
                    5 => Unorm8x4,
                    6 => Float32x3,
                    7 => Float32,
                    8 => Float32x2,
                    9 => Uint32,
                    10 => Float32x2,
                    11 => Float32x3
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Cw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            // The original BSP visibility pass excludes geometry behind the
            // portal. Writing the backdrop plane provides the same occlusion
            // until node/zone visibility traversal is implemented.
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn fragment_entry(
    mode: SurfaceMode,
    masked: bool,
    unlit: bool,
    modern: bool,
) -> &'static str {
    if unlit
        && matches!(
            mode,
            SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended
        )
    {
        return match (modern, masked) {
            (false, false) => "fragment_unlit_blended",
            (false, true) => "fragment_unlit_blended_masked",
            (true, false) => "fragment_modern_unlit_blended",
            (true, true) => "fragment_modern_unlit_blended_masked",
        };
    }
    if modern {
        return match (mode, masked, unlit) {
            (SurfaceMode::Opaque, false, false) => "fragment_modern",
            (SurfaceMode::Opaque, true, false) => "fragment_modern_masked",
            (SurfaceMode::Opaque, false, true) => "fragment_modern_unlit",
            (SurfaceMode::Opaque, true, true) => "fragment_modern_unlit_masked",
            (
                SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended,
                false,
                _,
            ) => "fragment_modern_blended",
            (
                SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended,
                true,
                _,
            ) => "fragment_modern_blended_masked",
            (SurfaceMode::Backdrop | SurfaceMode::Hidden, _, _) => unreachable!(),
        };
    }
    match (mode, masked, unlit) {
        (SurfaceMode::Opaque, false, false) => "fragment_main",
        (SurfaceMode::Opaque, true, false) => "fragment_masked",
        (SurfaceMode::Opaque, false, true) => "fragment_unlit",
        (SurfaceMode::Opaque, true, true) => "fragment_unlit_masked",
        (
            SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended,
            false,
            _,
        ) => "fragment_blended",
        (
            SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended,
            true,
            _,
        ) => "fragment_blended_masked",
        (SurfaceMode::Backdrop | SurfaceMode::Hidden, _, _) => unreachable!(),
    }
}

pub(super) fn blend_state(mode: SurfaceMode) -> Option<wgpu::BlendState> {
    let color = match mode {
        SurfaceMode::Opaque => return None,
        SurfaceMode::Translucent => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        },
        SurfaceMode::Modulated => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Dst,
            dst_factor: wgpu::BlendFactor::Src,
            operation: wgpu::BlendOperation::Add,
        },
        SurfaceMode::AlphaBlended => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        SurfaceMode::Backdrop | SurfaceMode::Hidden => unreachable!(),
    };
    Some(wgpu::BlendState {
        color,
        alpha: color,
    })
}

pub(super) fn texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    lightmap_view: &wgpu::TextureView,
    lightmap_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("OpenHP1 texture bind group"),
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
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(lightmap_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(lightmap_sampler),
            },
        ],
    })
}

pub(super) fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &TextureImage,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: image.mip_level_count(),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // UE1's fixed-function path modulates palette and lightmap
        // samples directly in display space.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    assert!(write_texture_mips(queue, &texture, image));
    texture
}

pub(super) fn write_texture_mips(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    image: &TextureImage,
) -> bool {
    if texture.width() != image.width
        || texture.height() != image.height
        || texture.mip_level_count() != image.mip_level_count()
        || !valid_mip_chain(image)
    {
        return false;
    }
    for (level, (width, height, rgba)) in texture_levels(image).enumerate() {
        let mut destination = texture.as_image_copy();
        destination.mip_level = level as u32;
        queue.write_texture(
            destination,
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    true
}

fn valid_mip_chain(image: &TextureImage) -> bool {
    if image.width == 0
        || image.height == 0
        || image.mip_level_count() > image.width.max(image.height).ilog2() + 1
    {
        return false;
    }
    texture_levels(image)
        .enumerate()
        .all(|(level, (width, height, rgba))| {
            let expected_width = image.width.checked_shr(level as u32).unwrap_or(0).max(1);
            let expected_height = image.height.checked_shr(level as u32).unwrap_or(0).max(1);
            width == expected_width
                && height == expected_height
                && usize::try_from(width)
                    .ok()
                    .and_then(|width| {
                        usize::try_from(height)
                            .ok()
                            .and_then(|height| width.checked_mul(height))
                    })
                    .and_then(|pixels| pixels.checked_mul(4))
                    == Some(rgba.len())
        })
}

fn texture_levels(image: &TextureImage) -> impl Iterator<Item = (u32, u32, &[u8])> {
    std::iter::once((image.width, image.height, image.rgba.as_slice())).chain(
        image
            .mips
            .iter()
            .map(|mip| (mip.width, mip.height, mip.rgba.as_slice())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflected_view_reverses_the_render_space_front_face() {
        assert!(matches!(front_face(false), wgpu::FrontFace::Cw));
        assert!(matches!(front_face(true), wgpu::FrontFace::Ccw));
    }

    #[test]
    fn validates_exact_authored_mip_rows() {
        let image = TextureImage {
            width: 8,
            height: 8,
            rgba: vec![8; 8 * 8 * 4],
            mips: [(4, 4), (2, 2), (1, 1)]
                .map(|(width, height)| openhp1_scene::TextureMipImage {
                    width,
                    height,
                    rgba: vec![width as u8; (width * height * 4) as usize],
                })
                .into(),
        };
        assert!(valid_mip_chain(&image));
        assert_eq!(
            texture_levels(&image)
                .map(|(width, height, rgba)| (width, height, rgba[0]))
                .collect::<Vec<_>>(),
            [(8, 8, 8), (4, 4, 4), (2, 2, 2), (1, 1, 1)]
        );
        assert!(valid_mip_chain(&TextureImage {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            mips: Vec::new(),
        }));
        let mut invalid = image;
        invalid.mips[0].width = 3;
        assert!(!valid_mip_chain(&invalid));
    }
}
