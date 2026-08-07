mod lightmap;
mod vertex;

use glam::Vec3;
use openhp1_package::{ObjectReference, Package, PropertyKind};

use crate::{Error, Level, Result, Rotator, decode::skip_object_stack};

pub use lightmap::{AuthoredLight, AuthoredLightmap, LightVisibility, LightmapImage};
pub use vertex::{ActorVertexLighting, VertexLighting, bsp_zone_at, bsp_zone_at_checked};

#[derive(Clone, Copy, Debug, PartialEq)]
struct LightActor {
    location: Vec3,
    rotation: Rotator,
    light_type: u8,
    effect: u8,
    brightness: u8,
    hue: u8,
    saturation: u8,
    radius: u8,
    cone: u8,
    volume_brightness: u8,
    volume_fog: u8,
    volume_radius: u8,
    corona: bool,
    special_lit: bool,
}

impl Default for LightActor {
    fn default() -> Self {
        Self {
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            light_type: 1,
            effect: 0,
            brightness: 64,
            hue: 0,
            saturation: 255,
            radius: 64,
            cone: 128,
            volume_brightness: 64,
            volume_fog: 0,
            volume_radius: 0,
            corona: false,
            special_lit: false,
        }
    }
}

#[derive(Clone, Copy)]
struct AmbientLight {
    brightness: u8,
    hue: u8,
    saturation: u8,
}

impl Default for AmbientLight {
    fn default() -> Self {
        Self {
            brightness: 0,
            hue: 0,
            saturation: 255,
        }
    }
}

fn decode_level_ambient(package: &Package) -> Result<AmbientLight> {
    let Some(level_index) = package
        .summary()
        .exports
        .iter()
        .position(|export| package.summary().class_name(export) == Some("Level"))
    else {
        return Err(Error::MissingLevel);
    };
    let level = Level::decode(package, level_index)?;
    match level.actors.into_iter().find_map(|actor| match actor {
        ObjectReference::Export(index)
            if package
                .summary()
                .exports
                .get(index)
                .and_then(|export| package.summary().class_name(export))
                == Some("LevelInfo") =>
        {
            Some(index)
        }
        _ => None,
    }) {
        Some(index) => decode_ambient(package, index),
        None => Ok(AmbientLight::default()),
    }
}

fn decode_light(package: &Package, export_index: usize) -> Result<LightActor> {
    let mut reader = package.export_reader(export_index)?;
    skip_object_stack(package, export_index, &mut reader)?;
    let mut light = LightActor::default();
    while let Some(property) = reader.next_property()? {
        let name = reader.summary().name(property.name);
        let struct_name = property
            .struct_name
            .map(|index| reader.summary().name(index));
        let mut value = reader.property_reader(&property);
        match (name, property.kind, struct_name) {
            ("Location", PropertyKind::Struct, Some("Vector")) => {
                light.location = Vec3::new(value.read_f32()?, value.read_f32()?, value.read_f32()?);
            }
            ("Rotation", PropertyKind::Struct, Some("Rotator")) => {
                light.rotation = Rotator {
                    pitch: value.read_i32()?,
                    yaw: value.read_i32()?,
                    roll: value.read_i32()?,
                };
            }
            ("LightType", PropertyKind::Byte, _) => light.light_type = value.read_u8()?,
            ("LightEffect", PropertyKind::Byte, _) => light.effect = value.read_u8()?,
            ("LightBrightness", PropertyKind::Byte, _) => light.brightness = value.read_u8()?,
            ("LightHue", PropertyKind::Byte, _) => light.hue = value.read_u8()?,
            ("LightSaturation", PropertyKind::Byte, _) => light.saturation = value.read_u8()?,
            ("LightRadius", PropertyKind::Byte, _) => light.radius = value.read_u8()?,
            ("LightCone", PropertyKind::Byte, _) => light.cone = value.read_u8()?,
            ("VolumeBrightness", PropertyKind::Byte, _) => {
                light.volume_brightness = value.read_u8()?;
            }
            ("VolumeFog", PropertyKind::Byte, _) => light.volume_fog = value.read_u8()?,
            ("VolumeRadius", PropertyKind::Byte, _) => light.volume_radius = value.read_u8()?,
            ("bCorona", PropertyKind::Bool, _) => {
                light.corona = property.bool_value.unwrap_or(false);
            }
            ("bSpecialLit", PropertyKind::Bool, _) => {
                light.special_lit = property.bool_value.unwrap_or(false);
            }
            _ => {}
        }
    }
    Ok(light)
}

fn decode_ambient(package: &Package, export_index: usize) -> Result<AmbientLight> {
    let mut reader = package.export_reader(export_index)?;
    skip_object_stack(package, export_index, &mut reader)?;
    let mut ambient = AmbientLight::default();
    while let Some(property) = reader.next_property()? {
        let name = reader.summary().name(property.name);
        let mut value = reader.property_reader(&property);
        match (name, property.kind) {
            ("AmbientBrightness", PropertyKind::Byte) => {
                ambient.brightness = value.read_u8()?;
            }
            ("AmbientHue", PropertyKind::Byte) => ambient.hue = value.read_u8()?,
            ("AmbientSaturation", PropertyKind::Byte) => {
                ambient.saturation = value.read_u8()?;
            }
            _ => {}
        }
    }
    Ok(ambient)
}

pub fn hsb_to_rgb(hue: u8, saturation: u8, brightness: u8) -> Vec3 {
    let value = 6.512_735 * f32::from(brightness).sqrt();
    if saturation >= 250 {
        return Vec3::splat(value / 255.0);
    }
    if brightness == 0 {
        return Vec3::ZERO;
    }
    let mut saturation = f32::from(saturation) / 2.5;
    if saturation > 32.0 {
        saturation += 2.0;
    }
    let sector = f32::from(hue) / 85.0;
    let fraction = sector.fract();
    let low = saturation * value / 104.0;
    let falling = (1.0 - fraction) * value + low * fraction;
    let rising = fraction * value + low * (1.0 - fraction);
    let rgb = if hue < 85 {
        Vec3::new(falling, rising, low)
    } else if hue < 170 {
        Vec3::new(low, falling, rising)
    } else {
        Vec3::new(rising, low, falling)
    };
    rgb / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_unreal_saturation_produces_grey_light() {
        let color = hsb_to_rgb(123, 255, 64);
        assert_eq!(color.x, color.y);
        assert_eq!(color.y, color.z);
        assert!((color.x - 0.204_321_1).abs() < 0.000_001);
    }
}
