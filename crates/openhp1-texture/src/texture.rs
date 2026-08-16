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

#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub palette: ObjectReference,
    pub anim_next: ObjectReference,
    pub prime_count: u8,
    pub min_frame_rate: f32,
    pub max_frame_rate: f32,
    pub declared_width: Option<u32>,
    pub declared_height: Option<u32>,
    pub render_flags: TextureRenderFlags,
    pub mips: Vec<MipLevel>,
    pub wet: Option<WetTexture>,
    pub ice: Option<IceTexture>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IcePanningStyle {
    Linear,
    Circular,
    Gestation,
    WavyX,
    WavyY,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IceTimeMethod {
    FrameRateSync,
    Realtime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IceTexture {
    pub glass_texture: ObjectReference,
    pub source_texture: ObjectReference,
    pub panning_style: IcePanningStyle,
    pub time_method: IceTimeMethod,
    pub horiz_pan_speed: u8,
    pub vert_pan_speed: u8,
    pub frequency: u8,
    pub amplitude: u8,
    pub move_ice: bool,
    pub master_count: f32,
    pub u_displace: f32,
    pub v_displace: f32,
    pub u_position: f32,
    pub v_position: f32,
}

#[derive(Clone, Debug)]
pub struct IceAnimation {
    config: IceTexture,
    width: usize,
    height: usize,
    source: Vec<u8>,
    glass: Vec<u8>,
    indices: Vec<u8>,
    accumulator: f32,
    prime_count: u8,
    prime_current: u8,
    min_frame_rate: f32,
    max_frame_rate: f32,
    last_position: [i32; 2],
    force_refresh: bool,
    dependency_refresh: bool,
    local_source: bool,
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
        let mut anim_next = ObjectReference::None;
        let mut prime_count = 0;
        let mut min_frame_rate = 0.0;
        let mut max_frame_rate = 0.0;
        let mut declared_width = None;
        let mut declared_height = None;
        let mut clamp_width = None;
        let mut clamp_height = None;
        let mut source_texture = ObjectReference::None;
        let mut glass_texture = ObjectReference::None;
        let mut panning_style = IcePanningStyle::Linear;
        let mut time_method = IceTimeMethod::FrameRateSync;
        let mut horiz_pan_speed = 128;
        let mut vert_pan_speed = 128;
        let mut frequency = 11;
        let mut amplitude = 44;
        let mut move_ice = true;
        let mut master_count = 0.0;
        let mut u_displace = 0.0;
        let mut v_displace = 0.0;
        let mut u_position = 0.0;
        let mut v_position = 0.0;
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
                "AnimNext" if property.kind == PropertyKind::Object => {
                    anim_next = reader.property_reader(&property).read_object_reference()?;
                }
                "PrimeCount" if property.kind == PropertyKind::Byte => {
                    prime_count = reader.property_reader(&property).read_u8()?;
                }
                "MinFrameRate" if property.kind == PropertyKind::Float => {
                    min_frame_rate = reader.property_reader(&property).read_f32()?;
                }
                "MaxFrameRate" if property.kind == PropertyKind::Float => {
                    max_frame_rate = reader.property_reader(&property).read_f32()?;
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
                "GlassTexture" if property.kind == PropertyKind::Object => {
                    glass_texture = reader.property_reader(&property).read_object_reference()?;
                }
                "PanningStyle"
                    if class == TextureClass::Ice && property.kind == PropertyKind::Byte =>
                {
                    panning_style = match reader.property_reader(&property).read_u8()? {
                        0 => IcePanningStyle::Linear,
                        1 => IcePanningStyle::Circular,
                        2 => IcePanningStyle::Gestation,
                        3 => IcePanningStyle::WavyX,
                        4 => IcePanningStyle::WavyY,
                        value => return Err(Error::UnsupportedIcePanningStyle(value)),
                    };
                }
                "TimeMethod" if property.kind == PropertyKind::Byte => {
                    time_method = if reader.property_reader(&property).read_u8()? == 0 {
                        IceTimeMethod::FrameRateSync
                    } else {
                        IceTimeMethod::Realtime
                    };
                }
                "HorizPanSpeed" if property.kind == PropertyKind::Byte => {
                    horiz_pan_speed = reader.property_reader(&property).read_u8()?;
                }
                "VertPanSpeed" if property.kind == PropertyKind::Byte => {
                    vert_pan_speed = reader.property_reader(&property).read_u8()?;
                }
                "Frequency" if property.kind == PropertyKind::Byte => {
                    frequency = reader.property_reader(&property).read_u8()?;
                }
                "Amplitude" if property.kind == PropertyKind::Byte => {
                    amplitude = reader.property_reader(&property).read_u8()?;
                }
                "MoveIce" if property.kind == PropertyKind::Bool => {
                    move_ice = property.bool_value.unwrap_or(false);
                }
                "MasterCount" if property.kind == PropertyKind::Float => {
                    master_count = reader.property_reader(&property).read_f32()?;
                }
                "UDisplace" if property.kind == PropertyKind::Float => {
                    u_displace = reader.property_reader(&property).read_f32()?;
                }
                "VDisplace" if property.kind == PropertyKind::Float => {
                    v_displace = reader.property_reader(&property).read_f32()?;
                }
                "UPosition" if property.kind == PropertyKind::Float => {
                    u_position = reader.property_reader(&property).read_f32()?;
                }
                "VPosition" if property.kind == PropertyKind::Float => {
                    v_position = reader.property_reader(&property).read_f32()?;
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
        let ice = (class == TextureClass::Ice).then_some(IceTexture {
            glass_texture,
            source_texture,
            panning_style,
            time_method,
            horiz_pan_speed,
            vert_pan_speed,
            frequency,
            amplitude,
            move_ice,
            master_count,
            u_displace,
            v_displace,
            u_position,
            v_position,
        });

        Ok(Self {
            palette,
            anim_next,
            prime_count,
            min_frame_rate,
            max_frame_rate,
            declared_width,
            declared_height,
            render_flags,
            mips,
            wet,
            ice,
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

impl IceTexture {
    #[allow(clippy::too_many_arguments)]
    pub fn animate(
        &self,
        width: u32,
        height: u32,
        source_width: u32,
        source_height: u32,
        source: &[u8],
        glass_width: u32,
        glass_height: u32,
        glass: &[u8],
        min_frame_rate: f32,
        max_frame_rate: f32,
        prime_count: u8,
    ) -> Result<IceAnimation> {
        validate_ice_pixels("texture", width, height, pixel_count(width, height)?)?;
        validate_ice_pixels("source", source_width, source_height, source.len())?;
        validate_ice_pixels("glass", glass_width, glass_height, glass.len())?;
        require_ice_dimensions("glass", width, height, glass_width, glass_height)?;
        if source_width > width || source_height > height {
            return Err(Error::IceDimensionMismatch {
                field: "source",
                expected_width: width,
                expected_height: height,
                actual_width: source_width,
                actual_height: source_height,
            });
        }
        let local_source = (source_width, source_height) != (width, height);
        let mut animation = IceAnimation {
            config: self.clone(),
            width: width as usize,
            height: height as usize,
            source: source.to_vec(),
            glass: glass.to_vec(),
            indices: if local_source {
                expand_ice_source(width, height, source_width, source_height, source)
            } else {
                vec![0; pixel_count(width, height)?]
            },
            accumulator: 0.0,
            prime_count,
            prime_current: 0,
            min_frame_rate,
            max_frame_rate,
            last_position: [-1; 2],
            force_refresh: true,
            dependency_refresh: false,
            local_source,
        };
        animation.render(0.0);
        Ok(animation)
    }
}

impl IceAnimation {
    pub fn tick(&mut self, delta_time: f32) -> bool {
        if !delta_time.is_finite() || delta_time == 0.0 {
            return false;
        }
        if self.config.time_method == IceTimeMethod::Realtime {
            return self.render(delta_time);
        }
        let mut changed = false;
        while self.prime_current < self.prime_count {
            self.prime_current += 1;
            changed |= self.render(1.0 / 120.0);
        }
        self.accumulator += delta_time;
        if self.max_frame_rate != 0.0 {
            let maximum = texture_frame_rate(self.max_frame_rate);
            if self.accumulator < 1.0 / maximum {
                return if self.dependency_refresh {
                    self.render(0.0) || changed
                } else {
                    changed
                };
            }
            let minimum_period = 1.0 / texture_frame_rate(self.min_frame_rate);
            if self.accumulator < minimum_period {
                self.accumulator = 0.0;
            } else {
                self.accumulator -= minimum_period;
                if self.accumulator > minimum_period {
                    self.accumulator = minimum_period;
                }
            }
        } else {
            self.accumulator = 0.0;
        }
        self.render(1.0 / 120.0) || changed
    }

    pub fn indices(&self) -> &[u8] {
        &self.indices
    }

    pub fn rgba(&self, palette: &Palette, masked: bool) -> Result<Vec<u8>> {
        rgba(&self.indices, palette, masked)
    }

    pub fn update_dependencies(
        &mut self,
        source_width: u32,
        source_height: u32,
        source: &[u8],
        glass_width: u32,
        glass_height: u32,
        glass: &[u8],
    ) -> Result<bool> {
        validate_ice_pixels("source", source_width, source_height, source.len())?;
        validate_ice_pixels("glass", glass_width, glass_height, glass.len())?;
        require_ice_dimensions(
            "glass",
            self.width as u32,
            self.height as u32,
            glass_width,
            glass_height,
        )?;
        if source_width as usize > self.width || source_height as usize > self.height {
            return Err(Error::IceDimensionMismatch {
                field: "source",
                expected_width: self.width as u32,
                expected_height: self.height as u32,
                actual_width: source_width,
                actual_height: source_height,
            });
        }
        let local_source =
            (source_width as usize, source_height as usize) != (self.width, self.height);
        let expanded_source = local_source.then(|| {
            expand_ice_source(
                self.width as u32,
                self.height as u32,
                source_width,
                source_height,
                source,
            )
        });
        let changed = self.source != source
            || self.glass != glass
            || self.local_source != local_source
            || expanded_source
                .as_ref()
                .is_some_and(|expanded| self.indices != *expanded);
        self.source.clear();
        self.source.extend_from_slice(source);
        self.glass.clear();
        self.glass.extend_from_slice(glass);
        self.local_source = local_source;
        if let Some(expanded_source) = expanded_source {
            self.indices = expanded_source;
        }
        self.force_refresh |= changed;
        self.dependency_refresh |= changed;
        Ok(changed)
    }

    fn render(&mut self, delta_time: f32) -> bool {
        self.move_position(delta_time);
        let position = [
            self.config.u_position.round() as i32,
            self.config.v_position.round() as i32,
        ];
        if position == self.last_position && !self.force_refresh {
            return false;
        }
        self.last_position = position;
        if self.local_source {
            let changed = self.dependency_refresh;
            self.force_refresh = false;
            self.dependency_refresh = false;
            return changed;
        }
        let u_mask = self.width - 1;
        let v_mask = self.height - 1;
        for y in 0..self.height {
            for x in 0..self.width {
                let destination = x + y * self.width;
                self.indices[destination] = if self.config.move_ice {
                    let glass = self.glass[wrap(x as i32 + position[0], u_mask)
                        + wrap(y as i32 + position[1], v_mask) * self.width];
                    self.source[wrap(x as i32 + i32::from(glass), u_mask) + y * self.width]
                } else {
                    let glass = self.glass[destination];
                    self.source[wrap(x as i32 + position[0] + i32::from(glass), u_mask)
                        + wrap(y as i32 + position[1], v_mask) * self.width]
                };
            }
        }
        self.force_refresh = false;
        self.dependency_refresh = false;
        true
    }

    fn move_position(&mut self, delta_time: f32) {
        self.config.master_count += 120.0 * delta_time;
        self.config.u_displace -=
            2.0 * f32::from(i16::from(self.config.horiz_pan_speed) - 128) * delta_time;
        self.config.v_displace +=
            2.0 * f32::from(i16::from(self.config.vert_pan_speed) - 128) * delta_time;
        let q = f32::from(u16::from(self.config.frequency) + 1) * self.config.master_count;
        let amplitude = f32::from(u16::from(self.config.amplitude) + 1);
        let sine = (q * 0.0012).sin();
        let cosine = (q * if self.config.panning_style == IcePanningStyle::Gestation {
            0.0011
        } else {
            0.0012
        })
        .cos();
        let (u, v) = match self.config.panning_style {
            IcePanningStyle::Linear => (0.0, 0.0),
            IcePanningStyle::Circular | IcePanningStyle::Gestation => {
                ((amplitude * sine).round(), (amplitude * cosine).round())
            }
            IcePanningStyle::WavyX => ((amplitude * 0.5 * sine).round(), 0.0),
            IcePanningStyle::WavyY => (0.0, (amplitude * 0.5 * cosine).round()),
        };
        self.config.u_position = self.config.u_displace + u;
        self.config.v_position = self.config.v_displace + v;
    }
}

fn wrap(value: i32, mask: usize) -> usize {
    value as usize & mask
}

fn expand_ice_source(
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    source: &[u8],
) -> Vec<u8> {
    let u_shift = width.ilog2() - source_width.ilog2();
    let v_shift = height.ilog2() - source_height.ilog2();
    let mut expanded = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            expanded.push(source[((x >> u_shift) + (y >> v_shift) * source_width) as usize]);
        }
    }
    expanded
}

fn validate_ice_pixels(field: &'static str, width: u32, height: u32, actual: usize) -> Result<()> {
    if width == 0 || height == 0 || !width.is_power_of_two() || !height.is_power_of_two() {
        return Err(Error::InvalidIceDimensions {
            field,
            width,
            height,
        });
    }
    let expected = pixel_count(width, height)?;
    if actual != expected {
        return Err(Error::InvalidIcePixels {
            field,
            width,
            height,
            actual,
        });
    }
    Ok(())
}

fn require_ice_dimensions(
    field: &'static str,
    expected_width: u32,
    expected_height: u32,
    actual_width: u32,
    actual_height: u32,
) -> Result<()> {
    if (actual_width, actual_height) != (expected_width, expected_height) {
        return Err(Error::IceDimensionMismatch {
            field,
            expected_width,
            expected_height,
            actual_width,
            actual_height,
        });
    }
    Ok(())
}

fn texture_frame_rate(value: f32) -> f32 {
    if 0.1 <= value { value.min(100.0) } else { 0.1 }
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

    pub fn indices(&self) -> &[u8] {
        &self.indices
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
    use std::sync::Arc;

    use openhp1_package::{ObjectReference, PACKAGE_MAGIC, Package};

    use crate::{
        Color, IcePanningStyle, IceTexture, IceTimeMethod, MipLevel, Palette, Texture,
        TextureRenderFlags,
    };

    #[test]
    fn masked_palette_index_zero_becomes_transparent_black() {
        let texture = Texture {
            palette: ObjectReference::None,
            anim_next: ObjectReference::None,
            prime_count: 0,
            min_frame_rate: 0.0,
            max_frame_rate: 0.0,
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
            ice: None,
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
    fn decodes_generic_texture_animation_properties() {
        let texture = Texture::decode(&synthetic_animated_texture(), 0).unwrap();

        assert_eq!(texture.anim_next, ObjectReference::Export(0));
        assert_eq!(texture.prime_count, 3);
        assert_eq!(texture.min_frame_rate, 12.0);
        assert_eq!(texture.max_frame_rate, 24.0);
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

    #[test]
    fn decodes_ice_texture_properties_and_runtime_state() {
        let texture = Texture::decode(&synthetic_ice_texture(), 0).unwrap();
        let ice = texture.ice.unwrap();

        assert_eq!(ice.glass_texture, ObjectReference::Export(0));
        assert_eq!(ice.source_texture, ObjectReference::Export(0));
        assert_eq!(ice.panning_style, IcePanningStyle::WavyY);
        assert_eq!(ice.time_method, IceTimeMethod::Realtime);
        assert_eq!(ice.horiz_pan_speed, 129);
        assert_eq!(ice.vert_pan_speed, 127);
        assert_eq!(ice.frequency, 12);
        assert_eq!(ice.amplitude, 45);
        assert!(!ice.move_ice);
        assert_eq!(
            [
                ice.master_count,
                ice.u_displace,
                ice.v_displace,
                ice.u_position,
                ice.v_position,
            ],
            [10.0, 1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn ice_blits_every_8x8_pixel_like_fire_dll() {
        let source = (0..64).collect::<Vec<_>>();
        let glass = (0..64).map(|index| index % 8).collect::<Vec<_>>();
        let not_moving = [
            17, 19, 21, 23, 17, 19, 21, 23, 25, 27, 29, 31, 25, 27, 29, 31, 33, 35, 37, 39, 33, 35,
            37, 39, 41, 43, 45, 47, 41, 43, 45, 47, 49, 51, 53, 55, 49, 51, 53, 55, 57, 59, 61, 63,
            57, 59, 61, 63, 1, 3, 5, 7, 1, 3, 5, 7, 9, 11, 13, 15, 9, 11, 13, 15,
        ];
        let moving = [
            1, 3, 5, 7, 1, 3, 5, 7, 9, 11, 13, 15, 9, 11, 13, 15, 17, 19, 21, 23, 17, 19, 21, 23,
            25, 27, 29, 31, 25, 27, 29, 31, 33, 35, 37, 39, 33, 35, 37, 39, 41, 43, 45, 47, 41, 43,
            45, 47, 49, 51, 53, 55, 49, 51, 53, 55, 57, 59, 61, 63, 57, 59, 61, 63,
        ];

        for (move_ice, expected) in [(false, not_moving), (true, moving)] {
            let mut config = ice_config(IcePanningStyle::Linear);
            config.move_ice = move_ice;
            config.u_displace = 1.0;
            config.v_displace = 2.0;
            let animation = config
                .animate(8, 8, 8, 8, &source, 8, 8, &glass, 0.0, 0.0, 0)
                .unwrap();
            assert_eq!(animation.indices(), expected);
        }
    }

    #[test]
    fn ice_movement_covers_speeds_panning_and_time_modes() {
        let pixels = [0; 64];
        let mut speed = ice_config(IcePanningStyle::Linear)
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 0)
            .unwrap();
        speed.config.horiz_pan_speed = 129;
        speed.config.vert_pan_speed = 127;
        assert!(!speed.tick(1.0 / 120.0));
        assert!((speed.config.master_count - 1.0).abs() < 1.0e-6);
        assert!((speed.config.u_displace + 1.0 / 60.0).abs() < 1.0e-6);
        assert!((speed.config.v_displace + 1.0 / 60.0).abs() < 1.0e-6);

        let master = std::f32::consts::PI / (4.0 * 0.0012) - 1.0;
        for (style, expected) in [
            (IcePanningStyle::Linear, [0.0, 0.0]),
            (IcePanningStyle::Circular, [7.0, 7.0]),
            (IcePanningStyle::Gestation, [7.0, 8.0]),
            (IcePanningStyle::WavyX, [4.0, 0.0]),
            (IcePanningStyle::WavyY, [0.0, 4.0]),
        ] {
            let mut config = ice_config(style);
            config.master_count = master;
            config.amplitude = 9;
            let mut animation = config
                .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 0)
                .unwrap();
            animation.tick(1.0 / 120.0);
            assert_eq!(
                [animation.config.u_position, animation.config.v_position],
                expected
            );
        }

        let mut synced = ice_config(IcePanningStyle::Linear)
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 20.0, 30.0, 0)
            .unwrap();
        synced.config.time_method = IceTimeMethod::FrameRateSync;
        assert!(!synced.tick(1.0 / 60.0));
        assert!(!synced.tick(1.0 / 60.0));
        assert_eq!(synced.config.master_count, 1.0);
        let mut realtime = ice_config(IcePanningStyle::Linear);
        realtime.time_method = IceTimeMethod::Realtime;
        let mut realtime = realtime
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 20.0, 30.0, 0)
            .unwrap();
        assert!(!realtime.tick(1.0 / 60.0));
        assert_eq!(realtime.config.master_count, 2.0);
    }

    #[test]
    fn ice_frame_sync_primes_before_scheduling_and_time_accepts_negative_nonzero_only() {
        let pixels = [0; 64];
        let mut synced = ice_config(IcePanningStyle::Linear);
        synced.time_method = IceTimeMethod::FrameRateSync;
        let mut synced = synced
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 2)
            .unwrap();
        assert!(!synced.tick(0.0));
        assert_eq!((synced.prime_current, synced.config.master_count), (0, 0.0));
        assert!(!synced.tick(1.0 / 60.0));
        assert_eq!((synced.prime_current, synced.config.master_count), (2, 3.0));

        let mut realtime = ice_config(IcePanningStyle::Linear)
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 2)
            .unwrap();
        assert!(!realtime.tick(0.0));
        assert!(!realtime.tick(-1.0 / 120.0));
        assert_eq!(
            (realtime.prime_current, realtime.config.master_count),
            (0, -1.0)
        );

        let mut negative_sync = ice_config(IcePanningStyle::Linear);
        negative_sync.time_method = IceTimeMethod::FrameRateSync;
        let mut negative_sync = negative_sync
            .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 0)
            .unwrap();
        assert!(!negative_sync.tick(-1.0));
        assert_eq!(negative_sync.config.master_count, 1.0);
    }

    #[test]
    fn ice_cache_force_local_source_and_dependency_replacement_match_native_guards() {
        let source = (0..64).collect::<Vec<_>>();
        let glass = [0; 64];
        let mut animation = ice_config(IcePanningStyle::Linear)
            .animate(8, 8, 8, 8, &source, 8, 8, &glass, 0.0, 0.0, 0)
            .unwrap();
        assert!(!animation.tick(1.0 / 120.0));
        assert!(!animation.tick(1.0 / 120.0));
        animation.force_refresh = true;
        assert!(animation.tick(1.0 / 120.0));
        let before = animation.indices.clone();
        let replacement = source.iter().map(|value| 63 - value).collect::<Vec<_>>();
        animation
            .update_dependencies(8, 8, &replacement, 8, 8, &glass)
            .unwrap();
        assert!(animation.tick(1.0 / 120.0));
        assert_ne!(animation.indices, before);
        let source_output = animation.indices.clone();
        animation
            .update_dependencies(8, 8, &replacement, 8, 8, &[1; 64])
            .unwrap();
        assert!(animation.tick(1.0 / 120.0));
        assert_ne!(animation.indices, source_output);

        let smaller_source = (0..16).collect::<Vec<_>>();
        let mut local_source = ice_config(IcePanningStyle::Linear)
            .animate(8, 8, 4, 4, &smaller_source, 8, 8, &glass, 0.0, 0.0, 0)
            .unwrap();
        assert_eq!(
            local_source.indices(),
            &[
                0, 0, 1, 1, 2, 2, 3, 3, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 4, 4, 5, 5,
                6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
                14, 14, 15, 15, 12, 12, 13, 13, 14, 14, 15, 15,
            ]
        );
        assert!(!local_source.tick(1.0));
        let smaller_replacement = (0..16).rev().collect::<Vec<_>>();
        local_source
            .update_dependencies(4, 4, &smaller_replacement, 8, 8, &glass)
            .unwrap();
        assert!(local_source.tick(1.0));
        assert_eq!(
            local_source.indices(),
            &[
                15, 15, 14, 14, 13, 13, 12, 12, 15, 15, 14, 14, 13, 13, 12, 12, 11, 11, 10, 10, 9,
                9, 8, 8, 11, 11, 10, 10, 9, 9, 8, 8, 7, 7, 6, 6, 5, 5, 4, 4, 7, 7, 6, 6, 5, 5, 4,
                4, 3, 3, 2, 2, 1, 1, 0, 0, 3, 3, 2, 2, 1, 1, 0, 0,
            ]
        );

        let mut synced = ice_config(IcePanningStyle::Linear);
        synced.time_method = IceTimeMethod::FrameRateSync;
        let mut synced = synced
            .animate(8, 8, 8, 8, &source, 8, 8, &glass, 20.0, 30.0, 0)
            .unwrap();
        assert!(!synced.tick(1.0 / 30.0));
        let master_count = synced.config.master_count;
        synced
            .update_dependencies(8, 8, &replacement, 8, 8, &glass)
            .unwrap();
        assert!(synced.tick(1.0 / 120.0));
        assert_eq!(synced.config.master_count, master_count);
    }

    fn ice_config(panning_style: IcePanningStyle) -> IceTexture {
        IceTexture {
            glass_texture: ObjectReference::None,
            source_texture: ObjectReference::None,
            panning_style,
            time_method: IceTimeMethod::Realtime,
            horiz_pan_speed: 128,
            vert_pan_speed: 128,
            frequency: 0,
            amplitude: 0,
            move_ice: false,
            master_count: 0.0,
            u_displace: 0.0,
            v_displace: 0.0,
            u_position: 0.0,
            v_position: 0.0,
        }
    }

    fn synthetic_ice_texture() -> Package {
        let names = [
            "None",
            "Core",
            "Class",
            "IceTexture",
            "Frozen",
            "Palette",
            "GlassTexture",
            "SourceTexture",
            "PanningStyle",
            "TimeMethod",
            "HorizPanSpeed",
            "VertPanSpeed",
            "Frequency",
            "Amplitude",
            "MoveIce",
            "MasterCount",
            "UDisplace",
            "VDisplace",
            "UPosition",
            "VPosition",
        ];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            push_u32(&mut name_table, 0);
        }
        let mut import_table = vec![1, 2];
        push_i32(&mut import_table, 0);
        import_table.push(3);

        let mut payload = vec![5, 0x05, 1, 6, 0x05, 1, 7, 0x05, 1];
        for (name, value) in [(8, 4), (9, 1), (10, 129), (11, 127), (12, 12), (13, 45)] {
            payload.extend([name, 0x01, value]);
        }
        payload.extend([14, 0x03]);
        for (name, value) in [(15, 10.0_f32), (16, 1.0), (17, 2.0), (18, 3.0), (19, 4.0)] {
            payload.extend([name, 0x24]);
            payload.extend(value.to_le_bytes());
        }
        payload.extend([0, 1, 0]);
        payload.extend(8_u32.to_le_bytes());
        payload.extend(8_u32.to_le_bytes());
        payload.extend([3, 3]);

        const HEADER_SIZE: usize = 48;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        push_i32(&mut export, 0);
        export.push(4);
        push_u32(&mut export, 0);
        export.extend(compact_index(payload.len() as i32));
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let encoded = compact_index(payload_offset as i32);
            let next = export_offset + export.len() + encoded.len();
            if next == payload_offset {
                export.extend(encoded);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        push_u32(&mut bytes, PACKAGE_MAGIC);
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        push_u32(&mut bytes, 0);
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
            0,
        ] {
            push_i32(&mut bytes, value as i32);
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        Package::parse("synthetic ice texture", Arc::from(bytes)).unwrap()
    }

    fn synthetic_animated_texture() -> Package {
        let names = [
            "None",
            "Core",
            "Class",
            "Texture",
            "Animated",
            "Palette",
            "AnimNext",
            "PrimeCount",
            "MinFrameRate",
            "MaxFrameRate",
        ];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            push_u32(&mut name_table, 0);
        }
        let mut import_table = vec![1, 2];
        push_i32(&mut import_table, 0);
        import_table.push(3);

        let mut payload = vec![5, 0x05, 0, 6, 0x05, 1, 7, 0x01, 3, 8, 0x24];
        payload.extend(12.0_f32.to_le_bytes());
        payload.extend([9, 0x24]);
        payload.extend(24.0_f32.to_le_bytes());
        payload.extend([0, 1, 1, 7]);
        payload.extend(1_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.extend([0, 0]);

        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        push_i32(&mut export, 0);
        export.push(4);
        push_u32(&mut export, 0);
        export.extend(compact_index(payload.len() as i32));
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let encoded = compact_index(payload_offset as i32);
            let next = export_offset + export.len() + encoded.len();
            if next == payload_offset {
                export.extend(encoded);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        push_u32(&mut bytes, PACKAGE_MAGIC);
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        push_u32(&mut bytes, 0);
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            push_i32(&mut bytes, value as i32);
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        Package::parse("synthetic animated texture", Arc::from(bytes)).unwrap()
    }

    fn compact_index(value: i32) -> Vec<u8> {
        let mut value = value as u32;
        let mut bytes = vec![(value as u8) & 0x3f];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = (value as u8) & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }
}
