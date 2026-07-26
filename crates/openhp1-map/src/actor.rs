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
    pub pre_pivot: Option<Vec3>,
    pub draw_scale: Option<f32>,
    pub draw_type: Option<u8>,
    pub mesh: Option<ObjectReference>,
    pub skin: Option<ObjectReference>,
    pub texture: Option<ObjectReference>,
    pub multi_skins: Vec<Option<ObjectReference>>,
    pub style: Option<u8>,
    pub ambient_glow: Option<u8>,
    pub scale_glow: Option<f32>,
    pub hidden: Option<bool>,
    pub unlit: Option<bool>,
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
                ("PrePivot", PropertyKind::Struct, Some("Vector")) => {
                    properties.pre_pivot = Some(read_vec3(&mut value)?);
                }
                ("DrawScale", PropertyKind::Float, _) => {
                    properties.draw_scale = Some(value.read_f32()?);
                }
                ("DrawType", PropertyKind::Byte, _) => {
                    properties.draw_type = Some(value.read_u8()?);
                }
                ("Mesh", PropertyKind::Object, _) => {
                    properties.mesh = Some(value.read_object_reference()?);
                }
                ("Skin", PropertyKind::Object, _) => {
                    properties.skin = Some(value.read_object_reference()?);
                }
                ("Texture", PropertyKind::Object, _) => {
                    properties.texture = Some(value.read_object_reference()?);
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
                ("bHidden", PropertyKind::Bool, _) => {
                    properties.hidden = property.bool_value;
                }
                ("bUnlit", PropertyKind::Bool, _) => {
                    properties.unlit = property.bool_value;
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
