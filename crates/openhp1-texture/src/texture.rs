use std::sync::Arc;

use openhp1_package::{ObjectReader, ObjectReference, Package, PropertyKind};

use crate::{Error, Palette, Result, decode::nonnegative};

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
    pub translucent: bool,
    pub modulated: bool,
    pub fake_backdrop: bool,
    pub two_sided: bool,
}

impl Texture {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let class = texture_class(package, export_index)?;
        let mut reader = package.export_reader(export_index)?;
        let mut palette = None;
        let mut declared_width = None;
        let mut declared_height = None;
        let mut clamp_width = None;
        let mut clamp_height = None;
        let mut source_texture = ObjectReference::None;
        let mut render_heat = 255;
        let mut rising = false;
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
                "UClamp" if property.kind == PropertyKind::Int => {
                    clamp_width = Some(reader.property_reader(&property).read_u32()?);
                }
                "VClamp" if property.kind == PropertyKind::Int => {
                    clamp_height = Some(reader.property_reader(&property).read_u32()?);
                }
                "SourceTexture" if property.kind == PropertyKind::Object => {
                    source_texture = reader.property_reader(&property).read_object_reference()?;
                }
                "RenderHeat" if property.kind == PropertyKind::Byte => {
                    render_heat = reader.property_reader(&property).read_u8()?;
                }
                "bRising" if property.kind == PropertyKind::Bool => {
                    rising = property.bool_value.unwrap_or(false);
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
        let mut mips = read_mips(package, export_index, &mut reader, class)?;
        let procedural_width = clamp_width.or(declared_width);
        let procedural_height = clamp_height.or(declared_height);
        if class != TextureClass::Regular {
            let width = procedural_width.unwrap_or_else(|| mips.first().map_or(0, |mip| mip.width));
            let height =
                procedural_height.unwrap_or_else(|| mips.first().map_or(0, |mip| mip.height));
            mips = vec![empty_mip(width, height)?];
        }
        match class {
            TextureClass::Regular => {}
            TextureClass::Wet => {
                // A WetTexture starts from its source image; water simulation
                // later displaces those palette indices.
                if let ObjectReference::Export(source_index) = source_texture
                    && source_index != export_index
                    && let source = Self::decode(package, source_index)?
                    && let (Some(destination), Some(source)) =
                        (mips.first_mut(), source.mips.first())
                    && destination.width == source.width
                    && destination.height == source.height
                {
                    destination.indices.clone_from(&source.indices);
                }
            }
            TextureClass::Fire => {
                let count_offset = reader.absolute_position();
                let spark_count =
                    nonnegative(reader.read_compact_index()?, count_offset, "fire sparks")?;
                let sparks = reader.read_bytes(
                    spark_count
                        .checked_mul(8)
                        .ok_or(Error::InvalidFireSparkCount { count: spark_count })?,
                )?;
                if let Some(mip) = mips.first_mut() {
                    render_fire_snapshot(mip, sparks, render_heat, rising);
                }
            }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextureClass {
    Regular,
    Wet,
    Fire,
}

fn texture_class(package: &Package, export_index: usize) -> Result<TextureClass> {
    let summary = package.summary();
    let export = summary.exports.get(export_index).ok_or_else(|| {
        openhp1_package::Error::InvalidExportIndex {
            package: Arc::clone(&summary.source),
            index: export_index,
            export_count: summary.exports.len(),
        }
    })?;
    let actual = summary.class_name(export).unwrap_or("<class>");
    match actual {
        "Texture" => Ok(TextureClass::Regular),
        "WetTexture" => Ok(TextureClass::Wet),
        "FireTexture" => Ok(TextureClass::Fire),
        _ => Err(Error::WrongClass {
            expected: "Texture, WetTexture, or FireTexture",
            actual: actual.to_owned(),
        }),
    }
}

fn read_mips(
    package: &Package,
    export_index: usize,
    reader: &mut ObjectReader<'_>,
    class: TextureClass,
) -> Result<Vec<MipLevel>> {
    let mip_count = usize::from(reader.read_u8()?);
    let mut mips = Vec::with_capacity(mip_count);
    for mip_index in 0..mip_count {
        // Versions 63+ prefix each TLazyArray with the absolute end of its
        // payload. Older HP1 packages store the payload directly.
        let expected_data_end = (package.summary().header.version >= 63)
            .then(|| reader.read_u32())
            .transpose()?;
        let count_offset = reader.absolute_position();
        let serialized_pixels =
            nonnegative(reader.read_compact_index()?, count_offset, "mip pixels")?;
        let indices = reader.read_bytes(serialized_pixels)?.to_vec();
        if let Some(expected_data_end) = expected_data_end {
            let actual_data_end = reader.absolute_position();
            if actual_data_end != expected_data_end as usize {
                return Err(Error::InvalidLazyArrayEnd {
                    package: Arc::clone(&package.summary().source),
                    export_index,
                    mip_index,
                    expected: expected_data_end as usize,
                    actual: actual_data_end,
                });
            }
        }

        let width = reader.read_u32()?;
        let height = reader.read_u32()?;
        let width_bits = reader.read_u8()?;
        let height_bits = reader.read_u8()?;
        let expected_pixels = pixel_count(width, height)?;
        if class == TextureClass::Regular && indices.len() != expected_pixels {
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
    Ok(mips)
}

fn empty_mip(width: u32, height: u32) -> Result<MipLevel> {
    Ok(MipLevel {
        width,
        height,
        width_bits: width.checked_ilog2().unwrap_or(0) as u8,
        height_bits: height.checked_ilog2().unwrap_or(0) as u8,
        indices: vec![0; pixel_count(width, height)?],
    })
}

fn pixel_count(width: u32, height: u32) -> Result<usize> {
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(Error::InvalidMipDimensions { width, height })
}

fn render_fire_snapshot(mip: &mut MipLevel, sparks: &[u8], render_heat: u8, rising: bool) {
    if mip.width == 0 || mip.height == 0 {
        return;
    }
    let width = mip.width as usize;
    let height = mip.height as usize;
    let mut random = 0x6d2b_79f5_u32;
    let heat_loss = 1.0 - f32::from(255 - render_heat) / 16.0;
    let fade = |sum: usize| {
        (((sum as f32 + 0.5) * 0.25 + heat_loss)
            .round()
            .clamp(0.0, 255.0)) as u8
    };
    let mut buffer = vec![0; mip.indices.len()];

    // ponytail: this deterministic warm-up is a static viewer snapshot.
    // Move the same update into a runtime texture tick when animation lands.
    for _ in 0..32 {
        for spark in sparks.chunks_exact(8) {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = usize::from(spark[2]);
            let y = usize::from(spark[3]);
            if x < width && y < height {
                mip.indices[x + y * width] = (random >> 24) as u8;
            }
        }
        let rise = usize::from(rising);
        for y in 0..height {
            let source_y = (y + rise) % height;
            let next_y = (source_y + 1) % height;
            for x in 0..width {
                let left = if x == 0 { width - 1 } else { x - 1 };
                let right = if x + 1 == width { 0 } else { x + 1 };
                buffer[x + y * width] = fade(
                    usize::from(mip.indices[left + source_y * width])
                        + usize::from(mip.indices[x + source_y * width])
                        + usize::from(mip.indices[right + source_y * width])
                        + usize::from(mip.indices[x + next_y * width]),
                );
            }
        }
        mip.indices.copy_from_slice(&buffer);
    }
}

impl TextureRenderFlags {
    fn set(&mut self, name: &str, value: bool) {
        if name.eq_ignore_ascii_case("bInvisible") {
            self.invisible = value;
        } else if name.eq_ignore_ascii_case("bMasked") {
            self.masked = value;
        } else if name.eq_ignore_ascii_case("bTransparent") {
            self.translucent = value;
        } else if name.eq_ignore_ascii_case("bModulate") {
            self.modulated = value;
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
        flags.set("bTransparent", true);
        flags.set("bModulate", true);
        flags.set("bTwoSided", true);
        assert!(flags.masked);
        assert!(flags.translucent);
        assert!(flags.modulated);
        assert!(flags.two_sided);
    }

    #[test]
    fn fire_snapshot_produces_repeatable_pixels() {
        let mut first = MipLevel {
            width: 8,
            height: 8,
            width_bits: 3,
            height_bits: 3,
            indices: vec![0; 64],
        };
        let mut second = first.clone();
        let sparks = [0, 255, 4, 6, 0, 0, 0, 0];
        super::render_fire_snapshot(&mut first, &sparks, 223, true);
        super::render_fire_snapshot(&mut second, &sparks, 223, true);
        assert_eq!(first, second);
        assert!(first.indices.iter().any(|index| *index != 0));
    }
}
