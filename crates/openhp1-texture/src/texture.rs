use std::sync::Arc;

use openhp1_package::{ObjectReference, Package, PropertyKind};

use crate::{
    Error, Palette, Result,
    decode::{nonnegative, require_class},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub width_bits: u8,
    pub height_bits: u8,
    /// Indices into the texture's palette, in row-major order.
    pub indices: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Texture {
    pub palette: ObjectReference,
    pub declared_width: Option<u32>,
    pub declared_height: Option<u32>,
    pub render_flags: TextureRenderFlags,
    pub mips: Vec<MipLevel>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextureRenderFlags {
    pub invisible: bool,
    pub masked: bool,
    pub fake_backdrop: bool,
    pub two_sided: bool,
}

impl Texture {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Texture")?;
        let mut reader = package.export_reader(export_index)?;
        let mut palette = None;
        let mut declared_width = None;
        let mut declared_height = None;
        let mut render_flags = TextureRenderFlags::default();

        while let Some(property) = reader.next_property()? {
            let name = reader.summary().name(property.name);
            match name {
                "Palette" if property.kind == PropertyKind::Object => {
                    palette = Some(reader.property_reader(&property).read_object_reference()?);
                }
                "USize" if property.kind == PropertyKind::Int => {
                    declared_width = Some(reader.property_reader(&property).read_u32()?);
                }
                "VSize" if property.kind == PropertyKind::Int => {
                    declared_height = Some(reader.property_reader(&property).read_u32()?);
                }
                _ if property.kind == PropertyKind::Bool => {
                    render_flags.set(name, property.bool_value.unwrap_or(false));
                }
                _ => {}
            }
        }

        let palette = palette.ok_or_else(|| Error::MissingPalette {
            package: Arc::clone(&package.summary().source),
            export_index,
        })?;
        let mip_count = usize::from(reader.read_u8()?);
        let mut mips = Vec::with_capacity(mip_count);
        for mip_index in 0..mip_count {
            // A serialized TLazyArray begins with the absolute position at
            // which its byte payload ends. This permits the original loader to
            // skip bulk data without reading it.
            let expected_data_end = reader.read_u32()? as usize;
            let count_offset = reader.absolute_position();
            let pixel_count =
                nonnegative(reader.read_compact_index()?, count_offset, "mip pixels")?;
            let indices = reader.read_bytes(pixel_count)?.to_vec();
            let actual_data_end = reader.absolute_position();
            if actual_data_end != expected_data_end {
                return Err(Error::InvalidLazyArrayEnd {
                    package: Arc::clone(&package.summary().source),
                    export_index,
                    mip_index,
                    expected: expected_data_end,
                    actual: actual_data_end,
                });
            }

            let width = reader.read_u32()?;
            let height = reader.read_u32()?;
            let width_bits = reader.read_u8()?;
            let height_bits = reader.read_u8()?;
            let expected_pixels = usize::try_from(width)
                .ok()
                .and_then(|width| {
                    usize::try_from(height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .ok_or(Error::InvalidMipDimensions { width, height })?;
            if indices.len() != expected_pixels {
                return Err(Error::InvalidMipLength {
                    mip_index,
                    width,
                    height,
                    actual: indices.len(),
                });
            }

            mips.push(MipLevel {
                width,
                height,
                width_bits,
                height_bits,
                indices,
            });
        }

        Ok(Self {
            palette,
            declared_width,
            declared_height,
            render_flags,
            mips,
        })
    }

    /// Expands one paletted mip to RGBA8. Masking is deliberately a caller
    /// choice because UE1 stores it on the surface/material, not the palette.
    pub fn rgba(&self, mip_index: usize, palette: &Palette, masked: bool) -> Result<Vec<u8>> {
        let mip = self
            .mips
            .get(mip_index)
            .ok_or(Error::MissingMip { mip_index })?;
        let mut rgba = Vec::with_capacity(mip.indices.len() * 4);
        for &index in &mip.indices {
            let color =
                palette
                    .colors
                    .get(usize::from(index))
                    .ok_or(Error::PaletteIndexOutOfRange {
                        index,
                        color_count: palette.colors.len(),
                    })?;
            if masked && index == 0 {
                rgba.extend_from_slice(&[0; 4]);
            } else {
                rgba.extend_from_slice(&[color.red, color.green, color.blue, 255]);
            }
        }
        Ok(rgba)
    }
}

impl TextureRenderFlags {
    fn set(&mut self, name: &str, value: bool) {
        if name.eq_ignore_ascii_case("bInvisible") {
            self.invisible = value;
        } else if name.eq_ignore_ascii_case("bMasked") {
            self.masked = value;
        } else if name.eq_ignore_ascii_case("bFakeBackdrop") {
            self.fake_backdrop = value;
        } else if name.eq_ignore_ascii_case("bTwoSided") {
            self.two_sided = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use openhp1_package::ObjectReference;

    use crate::{Color, MipLevel, Palette, Texture, TextureRenderFlags};

    #[test]
    fn masked_palette_index_zero_becomes_transparent_black() {
        let texture = Texture {
            palette: ObjectReference::None,
            declared_width: Some(2),
            declared_height: Some(1),
            render_flags: TextureRenderFlags::default(),
            mips: vec![MipLevel {
                width: 2,
                height: 1,
                width_bits: 1,
                height_bits: 0,
                indices: vec![0, 1],
            }],
        };
        let palette = Palette {
            colors: vec![
                Color {
                    red: 255,
                    green: 0,
                    blue: 255,
                    alpha: 0,
                },
                Color {
                    red: 1,
                    green: 2,
                    blue: 3,
                    alpha: 0,
                },
            ],
        };
        assert_eq!(
            texture.rgba(0, &palette, true).unwrap(),
            [0, 0, 0, 0, 1, 2, 3, 255]
        );
    }

    #[test]
    fn reads_texture_render_booleans_case_insensitively() {
        let mut flags = TextureRenderFlags::default();
        flags.set("bMASKED", true);
        flags.set("bTwoSided", true);
        assert!(flags.masked);
        assert!(flags.two_sided);
    }
}
