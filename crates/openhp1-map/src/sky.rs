use std::f32::consts::TAU;

use glam::Vec3;
use openhp1_package::{ObjectReference, Package, PropertyKind};

use crate::{Model, Result, decode::skip_object_stack};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rotator {
    pub pitch: i32,
    pub yaw: i32,
    pub roll: i32,
}

impl Rotator {
    pub fn radians(self) -> Vec3 {
        Vec3::new(self.pitch as f32, self.yaw as f32, self.roll as f32) * (TAU / 65_536.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyZone {
    pub location: Vec3,
    pub rotation: Rotator,
}

impl Model {
    /// Returns the first BSP `SkyZoneInfo`. HP1 maps with fake backdrops use
    /// one such actor as their fixed sky-box viewpoint.
    pub fn sky_zone(&self, package: &Package) -> Result<Option<SkyZone>> {
        for zone in &self.zones {
            let ObjectReference::Export(export_index) = zone.actor else {
                continue;
            };
            let export = &package.summary().exports[export_index];
            if package.summary().class_name(export) != Some("SkyZoneInfo") {
                continue;
            }
            return decode_sky_zone(package, export_index).map(Some);
        }
        Ok(None)
    }
}

fn decode_sky_zone(package: &Package, export_index: usize) -> Result<SkyZone> {
    let mut reader = package.export_reader(export_index)?;
    skip_object_stack(package, export_index, &mut reader)?;

    let mut sky = SkyZone::default();
    while let Some(property) = reader.next_property()? {
        let name = reader.summary().name(property.name);
        let struct_name = property
            .struct_name
            .map(|index| reader.summary().name(index));
        let mut value = reader.property_reader(&property);
        match (name, property.kind, struct_name) {
            ("Location", PropertyKind::Struct, Some("Vector")) => {
                sky.location = Vec3::new(value.read_f32()?, value.read_f32()?, value.read_f32()?);
            }
            ("Rotation", PropertyKind::Struct, Some("Rotator")) => {
                sky.rotation = Rotator {
                    pitch: value.read_i32()?,
                    yaw: value.read_i32()?,
                    roll: value.read_i32()?,
                };
            }
            _ => {}
        }
    }
    Ok(sky)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use super::*;

    #[test]
    fn converts_unreal_rotation_units_to_radians() {
        let radians = Rotator {
            pitch: 16_384,
            yaw: -32_768,
            roll: 0,
        }
        .radians();
        assert!((radians.x - FRAC_PI_2).abs() < 0.000_001);
        assert!((radians.y + PI).abs() < 0.000_001);
        assert_eq!(radians.z, 0.0);
    }
}
