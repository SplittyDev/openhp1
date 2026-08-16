use glam::Vec3;
use openhp1_package::{ObjectReader, ObjectReference, Package, PropertyKind};

use crate::{Result, Rotator, decode::skip_object_stack};

#[derive(Clone, Debug)]
pub struct Actor {
    pub export_index: usize,
    pub properties: ActorProperties,
}

#[derive(Clone, Debug, Default)]
pub struct ActorProperties {
    pub location: Option<Vec3>,
    pub rotation: Option<Rotator>,
    pub warp_coordinates: Option<[Vec3; 4]>,
    pub pre_pivot: Option<Vec3>,
    pub collision_height: Option<f32>,
    pub collide_type: Option<u8>,
    pub collide_world: Option<bool>,
    pub align_bottom: Option<bool>,
    pub physics: Option<u8>,
    pub draw_scale: Option<f32>,
    pub draw_type: Option<u8>,
    pub brush: Option<ObjectReference>,
    pub main_scale: Option<Vec3>,
    pub mesh: Option<ObjectReference>,
    pub skeletal_animation: Option<ObjectReference>,
    pub skin: Option<ObjectReference>,
    pub texture: Option<ObjectReference>,
    pub default_texture: Option<ObjectReference>,
    pub environment_map: Option<ObjectReference>,
    pub multi_skins: Vec<Option<ObjectReference>>,
    pub style: Option<u8>,
    pub ambient_glow: Option<u8>,
    pub scale_glow: Option<f32>,
    pub opacity: Option<f32>,
    pub light_brightness: Option<u8>,
    pub light_type: Option<u8>,
    pub light_radius: Option<u8>,
    pub volume_radius: Option<u8>,
    pub anim_sequence: Option<String>,
    pub anim_frame: Option<f32>,
    pub anim_rate: Option<f32>,
    pub texture_u_pan_speed: Option<f32>,
    pub texture_v_pan_speed: Option<f32>,
    pub light_hue: Option<u8>,
    pub light_saturation: Option<u8>,
    pub corona: Option<bool>,
    pub hidden: Option<bool>,
    pub unlit: Option<bool>,
    pub mesh_environment_map: Option<bool>,
}

impl Actor {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let mut reader = package.export_reader(export_index)?;
        skip_object_stack(package, export_index, &mut reader)?;
        Ok(Self {
            export_index,
            properties: ActorProperties::decode(&mut reader)?,
        })
    }
}

impl ActorProperties {
    pub fn decode(reader: &mut ObjectReader<'_>) -> Result<Self> {
        let mut properties = Self::default();
        while let Some(property) = reader.next_property()? {
            let name = reader.summary().name(property.name);
            let struct_name = property
                .struct_name
                .map(|index| reader.summary().name(index));
            let mut value = reader.property_reader(&property);
            match (name, property.kind, struct_name) {
                ("Location", PropertyKind::Struct, Some("Vector")) => {
                    properties.location = Some(read_vec3(&mut value)?);
                }
                ("Rotation", PropertyKind::Struct, Some("Rotator")) => {
                    properties.rotation = Some(Rotator {
                        pitch: value.read_i32()?,
                        yaw: value.read_i32()?,
                        roll: value.read_i32()?,
                    });
                }
                ("WarpCoords", PropertyKind::Struct, Some("Coords")) => {
                    properties.warp_coordinates = Some([
                        read_vec3(&mut value)?,
                        read_vec3(&mut value)?,
                        read_vec3(&mut value)?,
                        read_vec3(&mut value)?,
                    ]);
                }
                ("PrePivot", PropertyKind::Struct, Some("Vector")) => {
                    properties.pre_pivot = Some(read_vec3(&mut value)?);
                }
                ("CollisionHeight", PropertyKind::Float, _) => {
                    properties.collision_height = Some(value.read_f32()?);
                }
                ("CollideType", PropertyKind::Byte, _) => {
                    properties.collide_type = Some(value.read_u8()?);
                }
                ("bCollideWorld", PropertyKind::Bool, _) => {
                    properties.collide_world = property.bool_value;
                }
                ("bAlignBottom", PropertyKind::Bool, _) => {
                    properties.align_bottom = property.bool_value;
                }
                ("Physics", PropertyKind::Byte, _) => {
                    properties.physics = Some(value.read_u8()?);
                }
                ("DrawScale", PropertyKind::Float, _) => {
                    properties.draw_scale = Some(value.read_f32()?);
                }
                ("DrawType", PropertyKind::Byte, _) => {
                    properties.draw_type = Some(value.read_u8()?);
                }
                ("Brush", PropertyKind::Object, _) => {
                    properties.brush = Some(value.read_object_reference()?);
                }
                ("MainScale", PropertyKind::Struct, Some("Scale")) => {
                    properties.main_scale = Some(read_vec3(&mut value)?);
                }
                ("Mesh", PropertyKind::Object, _) => {
                    properties.mesh = Some(value.read_object_reference()?);
                }
                ("SkelAnim", PropertyKind::Object, _) => {
                    properties.skeletal_animation = Some(value.read_object_reference()?);
                }
                ("Skin", PropertyKind::Object, _) => {
                    properties.skin = Some(value.read_object_reference()?);
                }
                ("Texture", PropertyKind::Object, _) => {
                    properties.texture = Some(value.read_object_reference()?);
                }
                ("DefaultTexture", PropertyKind::Object, _) => {
                    properties.default_texture = Some(value.read_object_reference()?);
                }
                ("EnvironmentMap", PropertyKind::Object, _) => {
                    properties.environment_map = Some(value.read_object_reference()?);
                }
                ("MultiSkins", PropertyKind::Object, _) => {
                    let index = property.array_index.unwrap_or_default();
                    properties.multi_skins.resize(index + 1, None);
                    properties.multi_skins[index] = Some(value.read_object_reference()?);
                }
                ("Style", PropertyKind::Byte, _) => {
                    properties.style = Some(value.read_u8()?);
                }
                ("AmbientGlow", PropertyKind::Byte, _) => {
                    properties.ambient_glow = Some(value.read_u8()?);
                }
                ("ScaleGlow", PropertyKind::Float, _) => {
                    properties.scale_glow = Some(value.read_f32()?);
                }
                ("Opacity", PropertyKind::Float, _) => {
                    properties.opacity = Some(value.read_f32()?);
                }
                ("LightBrightness", PropertyKind::Byte, _) => {
                    properties.light_brightness = Some(value.read_u8()?);
                }
                ("LightType", PropertyKind::Byte, _) => {
                    properties.light_type = Some(value.read_u8()?);
                }
                ("LightRadius", PropertyKind::Byte, _) => {
                    properties.light_radius = Some(value.read_u8()?);
                }
                ("VolumeRadius", PropertyKind::Byte, _) => {
                    properties.volume_radius = Some(value.read_u8()?);
                }
                ("AnimSequence", PropertyKind::Name, _) => {
                    let index = value.read_name_index("actor animation sequence")?;
                    properties.anim_sequence = Some(value.summary().name(index).to_owned());
                }
                ("AnimFrame", PropertyKind::Float, _) => {
                    properties.anim_frame = Some(value.read_f32()?);
                }
                ("AnimRate", PropertyKind::Float, _) => {
                    properties.anim_rate = Some(value.read_f32()?);
                }
                ("TexUPanSpeed", PropertyKind::Float, _) => {
                    properties.texture_u_pan_speed = Some(value.read_f32()?);
                }
                ("TexVPanSpeed", PropertyKind::Float, _) => {
                    properties.texture_v_pan_speed = Some(value.read_f32()?);
                }
                ("LightHue", PropertyKind::Byte, _) => {
                    properties.light_hue = Some(value.read_u8()?);
                }
                ("LightSaturation", PropertyKind::Byte, _) => {
                    properties.light_saturation = Some(value.read_u8()?);
                }
                ("bCorona", PropertyKind::Bool, _) => {
                    properties.corona = property.bool_value;
                }
                ("bHidden", PropertyKind::Bool, _) => {
                    properties.hidden = property.bool_value;
                }
                ("bUnlit", PropertyKind::Bool, _) => {
                    properties.unlit = property.bool_value;
                }
                ("bMeshEnviroMap", PropertyKind::Bool, _) => {
                    properties.mesh_environment_map = property.bool_value;
                }
                _ => {}
            }
        }
        Ok(properties)
    }
}

fn read_vec3(reader: &mut ObjectReader<'_>) -> Result<Vec3> {
    Ok(Vec3::new(
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openhp1_package::{ObjectReference, PACKAGE_MAGIC, Package};

    #[test]
    fn decodes_zone_environment_map_object_property() {
        let names = ["EnvironmentMap", "None", "Core", "Class", "Actor", "Zone"];
        let package = synthetic_actor_package(&names, vec![0, 0x05, 1, 1]);

        assert_eq!(
            super::Actor::decode(&package, 0)
                .unwrap()
                .properties
                .environment_map,
            Some(ObjectReference::Export(0))
        );
    }

    #[test]
    fn decodes_level_default_texture_object_property() {
        let names = [
            "DefaultTexture",
            "None",
            "Core",
            "Class",
            "Actor",
            "LevelInfo",
        ];
        let package = synthetic_actor_package(&names, vec![0, 0x05, 1, 1]);

        assert_eq!(
            super::Actor::decode(&package, 0)
                .unwrap()
                .properties
                .default_texture,
            Some(ObjectReference::Export(0))
        );
    }

    fn synthetic_actor_package(names: &[&str], payload: Vec<u8>) -> Package {
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            name_table.extend(0_u32.to_le_bytes());
        }
        let mut import_table = vec![2, 3];
        import_table.extend(compact_index(0));
        import_table.extend(compact_index(4));
        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        export.extend(0_i32.to_le_bytes());
        export.extend(compact_index(5));
        export.extend(0_u32.to_le_bytes());
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
        bytes.extend(PACKAGE_MAGIC.to_le_bytes());
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
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
            bytes.extend((value as i32).to_le_bytes());
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        Package::parse("synthetic zone", Arc::from(bytes)).unwrap()
    }

    fn compact_index(value: i32) -> Vec<u8> {
        let negative = value < 0;
        let mut value = value.unsigned_abs();
        let mut bytes = vec![(value as u8 & 0x3f) | if negative { 0x80 } else { 0 }];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }
}
