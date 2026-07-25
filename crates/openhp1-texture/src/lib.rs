//! Decoders for the paletted UE1 textures used by Harry Potter 1.

use std::sync::Arc;

use openhp1_package::{ObjectReference, Package, PropertyKind};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// A palette color converted from Unreal's serialized BGRA byte order.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    /// UE1 palettes usually leave this byte at zero. Surface masking is a
    /// material property, so callers should not treat zero as transparency.
    pub alpha: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Palette {
    pub colors: Vec<Color>,
}

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
    pub mips: Vec<MipLevel>,
}

impl Palette {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Palette")?;
        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}

        let count_offset = reader.absolute_position();
        let count = nonnegative(reader.read_compact_index()?, count_offset, "palette colors")?;
        let bytes = reader.read_bytes(
            count
                .checked_mul(4)
                .ok_or(Error::InvalidPaletteCount { count })?,
        )?;
        let colors = bytes
            .chunks_exact(4)
            .map(|bgra| Color {
                red: bgra[2],
                green: bgra[1],
                blue: bgra[0],
                alpha: bgra[3],
            })
            .collect();
        Ok(Self { colors })
    }
}

impl Texture {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Texture")?;
        let mut reader = package.export_reader(export_index)?;
        let mut palette = None;
        let mut declared_width = None;
        let mut declared_height = None;

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
            rgba.extend_from_slice(&[
                color.red,
                color.green,
                color.blue,
                if masked && index == 0 { 0 } else { 255 },
            ]);
        }
        Ok(rgba)
    }
}

fn require_class(package: &Package, export_index: usize, expected: &'static str) -> Result<()> {
    let summary = package.summary();
    let export = summary.exports.get(export_index).ok_or_else(|| {
        openhp1_package::Error::InvalidExportIndex {
            package: Arc::clone(&summary.source),
            index: export_index,
            export_count: summary.exports.len(),
        }
    })?;
    let actual = summary.class_name(export).unwrap_or("<class>");
    if actual != expected {
        return Err(Error::WrongClass {
            expected,
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

fn nonnegative(value: i32, offset: usize, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::NegativeCount {
        field,
        value,
        offset,
    })
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error("expected an export of class {expected}, found {actual}")]
    WrongClass {
        expected: &'static str,
        actual: String,
    },

    #[error("package `{package}` texture export {export_index} has no Palette property")]
    MissingPalette {
        package: Arc<str>,
        export_index: usize,
    },

    #[error("{field} count {value} at byte {offset:#x} is negative")]
    NegativeCount {
        field: &'static str,
        value: i32,
        offset: usize,
    },

    #[error("palette color count {count} is too large")]
    InvalidPaletteCount { count: usize },

    #[error(
        "package `{package}` texture export {export_index} mip {mip_index} ends at {actual:#x}, not its serialized lazy-array end {expected:#x}"
    )]
    InvalidLazyArrayEnd {
        package: Arc<str>,
        export_index: usize,
        mip_index: usize,
        expected: usize,
        actual: usize,
    },

    #[error("mip dimensions {width}x{height} overflow the host address space")]
    InvalidMipDimensions { width: u32, height: u32 },

    #[error("mip {mip_index} is {width}x{height} but contains {actual} palette indices")]
    InvalidMipLength {
        mip_index: usize,
        width: u32,
        height: u32,
        actual: usize,
    },

    #[error("texture has no mip at index {mip_index}")]
    MissingMip { mip_index: usize },

    #[error("palette index {index} is outside its {color_count} colors")]
    PaletteIndexOutOfRange { index: u8, color_count: usize },
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn serialized_palette_color_is_bgra() {
        let bgra = [1, 2, 3, 4];
        let color = Color {
            red: bgra[2],
            green: bgra[1],
            blue: bgra[0],
            alpha: bgra[3],
        };
        assert_eq!(
            color,
            Color {
                red: 3,
                green: 2,
                blue: 1,
                alpha: 4
            }
        );
    }
}
