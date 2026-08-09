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
    pub wet: Option<WetTexture>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextureRenderFlags {
    pub invisible: bool,
    pub masked: bool,
    pub translucent: bool,
    pub modulated: bool,
    pub fake_backdrop: bool,
    pub two_sided: bool,
    pub mirrored: bool,
}

pub fn texture_poly_flags(package: &Package, export_index: usize) -> Result<u32> {
    let mut reader = package.export_reader(export_index)?;
    let mut flags = 0;
    while let Some(property) = reader.next_property()? {
        if property.kind == PropertyKind::Bool
            && let Some(flag) = texture_poly_flag(reader.summary().name(property.name))
        {
            if property.bool_value.unwrap_or(false) {
                flags |= flag;
            } else {
                flags &= !flag;
            }
        }
    }
    Ok(flags)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WetTexture {
    pub source_texture: ObjectReference,
    pub drops: Vec<WaterDrop>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaterDrop {
    pub kind: u8,
    pub depth: u8,
    pub x: u8,
    pub y: u8,
    pub bytes: [u8; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct WaterCell {
    pressure: f32,
    velocity: f32,
}

#[derive(Clone, Debug)]
pub struct WaterAnimation {
    width: usize,
    height: usize,
    source: Vec<u8>,
    indices: Vec<u8>,
    fields: [Vec<WaterCell>; 2],
    current: usize,
    accumulator: f32,
    random: u32,
    drops: Vec<WaterDrop>,
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
        let mut water_drop_count = 0;
        let mut water_drops = Vec::new();
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
                "NumDrops" if property.kind == PropertyKind::Int => {
                    let offset = reader.property_reader(&property).absolute_position();
                    water_drop_count = nonnegative(
                        reader.property_reader(&property).read_i32()?,
                        offset,
                        "water drops",
                    )?;
                }
                "Drops" if property.kind == PropertyKind::Struct => {
                    let index = property.array_index.unwrap_or_default();
                    let mut value = reader.property_reader(&property);
                    let actual = value.remaining();
                    if actual != 8 {
                        return Err(Error::InvalidWaterDropLength { index, actual });
                    }
                    let bytes = value.read_bytes(8)?;
                    water_drops.push((
                        index,
                        WaterDrop {
                            kind: bytes[0],
                            depth: bytes[1],
                            x: bytes[2],
                            y: bytes[3],
                            bytes: bytes[4..8].try_into().expect("checked water drop size"),
                        },
                    ));
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
                // Water starts from its source image and later displaces it.
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
            TextureClass::Ice => {}
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
        let wet = (class == TextureClass::Wet)
            .then(|| decode_water_drops(water_drop_count, water_drops))
            .transpose()?
            .map(|drops| WetTexture {
                source_texture,
                drops,
            });

        Ok(Self {
            palette,
            declared_width,
            declared_height,
            render_flags,
            mips,
            wet,
        })
    }

    /// Expands one paletted mip to RGBA8. Masking is deliberately a caller
    /// choice because UE1 stores it on the surface/material, not the palette.
    pub fn rgba(&self, mip_index: usize, palette: &Palette, masked: bool) -> Result<Vec<u8>> {
        let mip = self
            .mips
            .get(mip_index)
            .ok_or(Error::MissingMip { mip_index })?;
        rgba(&mip.indices, palette, masked)
    }
}

impl WetTexture {
    pub fn animate(
        &self,
        width: u32,
        height: u32,
        source_width: u32,
        source_height: u32,
        source_indices: &[u8],
    ) -> Result<Option<WaterAnimation>> {
        for drop in &self.drops {
            if !matches!(drop.kind, 0 | 1 | 2 | 3 | 6 | 7 | 12 | 16) {
                return Err(Error::UnsupportedWaterDropType(drop.kind));
            }
        }
        if width == 0 || height == 0 {
            return Err(Error::InvalidWaterDimensions { width, height });
        }
        if source_width == 0 || source_height == 0 {
            return Err(Error::InvalidWaterDimensions {
                width: source_width,
                height: source_height,
            });
        }
        let source_count = pixel_count(source_width, source_height)?;
        if source_indices.len() != source_count {
            return Err(Error::InvalidWaterSourceLength {
                width: source_width,
                height: source_height,
                actual: source_indices.len(),
            });
        }

        // Fire.dll builds a target-sized LocalSourceBitmap only when both
        // WetTexture dimensions are at least as large as SourceTexture.
        if source_width > width || source_height > height {
            return Ok(None);
        }

        let count = pixel_count(width, height)?;
        let mut source = Vec::with_capacity(count);
        for y in 0..height {
            let source_y = u64::from(y) * u64::from(source_height) / u64::from(height);
            for x in 0..width {
                let source_x = u64::from(x) * u64::from(source_width) / u64::from(width);
                let source_index = source_x + source_y * u64::from(source_width);
                source.push(source_indices[source_index as usize]);
            }
        }
        Ok(Some(WaterAnimation {
            width: width as usize,
            height: height as usize,
            indices: source.clone(),
            source,
            fields: [
                vec![WaterCell::default(); count],
                vec![WaterCell::default(); count],
            ],
            current: 0,
            accumulator: 0.0,
            random: 0x6d2b_79f5,
            drops: self.drops.clone(),
        }))
    }
}

impl WaterAnimation {
    pub fn tick(&mut self, delta_time: f32) -> bool {
        if !delta_time.is_finite() || delta_time <= 0.0 {
            return false;
        }
        self.accumulator += delta_time.min(0.1);
        let mut changed = false;
        while self.accumulator >= 1.0 / 30.0 {
            self.step();
            self.accumulator -= 1.0 / 30.0;
            changed = true;
        }
        changed
    }

    pub fn rgba(&self, palette: &Palette, masked: bool) -> Result<Vec<u8>> {
        rgba(&self.indices, palette, masked)
    }

    fn step(&mut self) {
        self.apply_drops();
        let current = self.current;
        let next = 1 - current;
        let (source, destination) = if current == 0 {
            let (source, destination) = self.fields.split_at_mut(1);
            (&source[0], &mut destination[0])
        } else {
            let (destination, source) = self.fields.split_at_mut(1);
            (&source[0], &mut destination[0])
        };
        for y in 0..self.height {
            let up = if y == 0 { self.height - 1 } else { y - 1 };
            let down = if y + 1 == self.height { 0 } else { y + 1 };
            for x in 0..self.width {
                let left = if x == 0 { self.width - 1 } else { x - 1 };
                let right = if x + 1 == self.width { 0 } else { x + 1 };
                let index = x + y * self.width;
                let pressure = source[index].pressure;
                let mut velocity = source[index].velocity
                    + (-2.0 * pressure
                        + source[left + y * self.width].pressure
                        + source[right + y * self.width].pressure)
                        * 0.25
                    + (-2.0 * pressure
                        + source[x + up * self.width].pressure
                        + source[x + down * self.width].pressure)
                        * 0.25;
                let pressure = (pressure + velocity) * 0.999;
                velocity = (velocity - 0.005 * pressure) * 0.998;
                destination[index] = WaterCell { pressure, velocity };
            }
        }
        self.current = next;
        self.render_source();
    }

    fn apply_drops(&mut self) {
        let field = &mut self.fields[self.current];
        // ponytail: SurrealEngine covers the wave step and emitters 0/1; the
        // other HP1-used emitters are minimal semantic versions until a
        // differential capture gives us their exact native behavior.
        for drop in &mut self.drops {
            let base_x = usize::from(drop.x) * self.width / 128;
            let base_y = usize::from(drop.y) * self.height / 128;
            let amplitude = (f32::from(drop.depth) - 128.0) / 255.0;
            match drop.kind {
                0 => set_pressure(field, self.width, self.height, base_x, base_y, amplitude),
                1 => {
                    drop.depth = drop.depth.wrapping_add(drop.bytes[3]);
                    set_pressure(
                        field,
                        self.width,
                        self.height,
                        base_x,
                        base_y,
                        (f32::from(drop.depth) * std::f32::consts::PI / 128.0).sin(),
                    );
                }
                2 => set_pressure(
                    field,
                    self.width,
                    self.height,
                    base_x,
                    base_y,
                    amplitude * 0.25,
                ),
                3 => {
                    drop.depth = drop.depth.wrapping_add(drop.bytes[3]);
                    set_pressure(
                        field,
                        self.width,
                        self.height,
                        base_x,
                        base_y,
                        0.5 * (f32::from(drop.depth) * std::f32::consts::PI / 128.0).sin(),
                    );
                }
                6 | 7 => {
                    drop.depth = drop.depth.wrapping_add(drop.bytes[3]);
                    let angle = f32::from(drop.depth) * std::f32::consts::TAU / 256.0;
                    let radius = if drop.kind == 6 { 4.0 } else { 10.0 };
                    let x = (base_x as f32 + angle.cos() * radius).round() as isize;
                    let y = (base_y as f32 + angle.sin() * radius).round() as isize;
                    set_pressure(
                        field,
                        self.width,
                        self.height,
                        x.rem_euclid(self.width as isize) as usize,
                        y.rem_euclid(self.height as isize) as usize,
                        angle.sin() * if drop.kind == 6 { 0.5 } else { 1.0 },
                    );
                }
                12 => {
                    drop.depth = drop.depth.wrapping_add(drop.bytes[3]);
                    let pressure =
                        (f32::from(drop.depth) * std::f32::consts::PI / 128.0).sin() * 0.5;
                    let y = base_y.min(self.height - 1);
                    for x in 0..self.width {
                        field[x + y * self.width].pressure = pressure;
                    }
                }
                16 => {
                    self.random = self
                        .random
                        .wrapping_mul(1_664_525)
                        .wrapping_add(1_013_904_223);
                    if (self.random >> 24) as u8 <= drop.bytes[3] {
                        let x = (self.random as usize) % self.width;
                        let y = ((self.random >> 16) as usize) % self.height;
                        set_pressure(field, self.width, self.height, x, y, amplitude);
                    }
                }
                _ => unreachable!("validated water drop type"),
            }
        }
    }

    fn render_source(&mut self) {
        let field = &self.fields[self.current];
        for y in 0..self.height {
            for x in 0..self.width {
                let left = if x == 0 { self.width - 1 } else { x - 1 };
                let right = if x + 1 == self.width { 0 } else { x + 1 };
                let displacement = ((field[right + y * self.width].pressure
                    - field[left + y * self.width].pressure)
                    * 0.25
                    * self.width as f32) as isize;
                let source_x = (x as isize + displacement).clamp(0, self.width as isize - 1);
                self.indices[x + y * self.width] = self.source[source_x as usize + y * self.width];
            }
        }
    }
}

fn decode_water_drops(count: usize, serialized: Vec<(usize, WaterDrop)>) -> Result<Vec<WaterDrop>> {
    if count > 256 {
        return Err(Error::InvalidWaterDropCount { count });
    }
    let available = serialized
        .iter()
        .filter(|(index, _)| *index < count)
        .count();
    let mut drops = vec![None; count];
    for (index, drop) in serialized {
        if let Some(destination) = drops.get_mut(index) {
            *destination = Some(drop);
        }
    }
    drops
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::MissingWaterDrops { count, available })
}

fn set_pressure(
    field: &mut [WaterCell],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pressure: f32,
) {
    field[x.min(width - 1) + y.min(height - 1) * width].pressure = pressure;
}

fn rgba(indices: &[u8], palette: &Palette, masked: bool) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for &index in indices {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextureClass {
    Regular,
    Wet,
    Fire,
    Ice,
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
        "IceTexture" => Ok(TextureClass::Ice),
        _ => Err(Error::WrongClass {
            expected: "Texture, WetTexture, FireTexture, or IceTexture",
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
        } else if name.eq_ignore_ascii_case("bMirrored") {
            self.mirrored = value;
        }
    }
}

fn texture_poly_flag(name: &str) -> Option<u32> {
    [
        ("bInvisible", 0x0000_0001),
        ("bMasked", 0x0000_0002),
        ("bTransparent", 0x0000_0004),
        ("bNotSolid", 0x0000_0008),
        ("bEnvironment", 0x0000_0010),
        ("bSemisolid", 0x0000_0020),
        ("bModulate", 0x0000_0040),
        ("bFakeBackdrop", 0x0000_0080),
        ("bTwoSided", 0x0000_0100),
        ("bAutoUPan", 0x0000_0200),
        ("bAutoVPan", 0x0000_0400),
        ("bNoSmooth", 0x0000_0800),
        ("bBigWavy", 0x0000_1000),
        ("bHighLedge", 0x0000_1000),
        ("bSmallWavy", 0x0000_2000),
        ("bLowShadowDetail", 0x0000_8000),
        ("bNoMerge", 0x0001_0000),
        ("bCloudWavy", 0x0002_0000),
        ("bDirtyShadows", 0x0004_0000),
        ("bSpecialLit", 0x0010_0000),
        ("bGouraud", 0x0020_0000),
        ("bHighShadowDetail", 0x0080_0000),
        ("bPortal", 0x0400_0000),
        ("bMirrored", 0x0800_0000),
    ]
    .into_iter()
    .find_map(|(flag_name, flag)| flag_name.eq_ignore_ascii_case(name).then_some(flag))
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
            wet: None,
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
        flags.set("bMirrored", true);
        assert!(flags.masked);
        assert!(flags.translucent);
        assert!(flags.modulated);
        assert!(flags.two_sided);
        assert!(flags.mirrored);
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

    #[test]
    fn wet_texture_advances_at_a_stable_rate_and_displaces_its_source() {
        let wet = super::WetTexture {
            source_texture: ObjectReference::None,
            drops: vec![super::WaterDrop {
                kind: 1,
                depth: 64,
                x: 64,
                y: 64,
                bytes: [0, 0, 0, 16],
            }],
        };
        let source = (0..2048)
            .map(|index| (index % 256) as u8)
            .collect::<Vec<_>>();
        let mut animation = wet.animate(256, 8, 256, 8, &source).unwrap().unwrap();

        assert!(!animation.tick(1.0 / 60.0));
        assert!(animation.tick(1.0 / 60.0));
        assert_ne!(animation.indices, source);
        assert_eq!(animation.indices.len(), source.len());
    }

    #[test]
    fn wet_texture_expands_smaller_source_like_fire_dll() {
        let wet = super::WetTexture {
            source_texture: ObjectReference::None,
            drops: vec![],
        };
        let animation = wet.animate(4, 4, 2, 2, &[1, 2, 3, 4]).unwrap().unwrap();

        assert_eq!(
            animation.indices,
            [1, 1, 2, 2, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 4, 4]
        );
        assert!(wet.animate(2, 2, 4, 4, &[0; 16]).unwrap().is_none());
    }
}
