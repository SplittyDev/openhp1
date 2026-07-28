use glam::Vec3;
use openhp1_package::{ObjectReference, Package};

use crate::{
    PolyFlags, Result,
    decode::{compact_count, fixed_count, require_class, skip_object_stack},
};

#[derive(Clone, Debug)]
pub struct BrushPolygon {
    pub base: Vec3,
    pub normal: Vec3,
    pub texture_u: Vec3,
    pub texture_v: Vec3,
    pub vertices: Vec<Vec3>,
    pub poly_flags: PolyFlags,
    pub texture: ObjectReference,
    pub pan_u: i16,
    pub pan_v: i16,
}

#[derive(Clone, Debug)]
pub struct BrushPolys {
    pub polygons: Vec<BrushPolygon>,
}

impl BrushPolys {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Polys")?;
        let mut reader = package.export_reader(export_index)?;
        skip_object_stack(package, export_index, &mut reader)?;
        while reader.next_property()?.is_some() {}
        let count = fixed_count(&mut reader, "brush polygons")?;
        let _capacity = fixed_count(&mut reader, "brush polygon capacity")?;
        let mut polygons = Vec::with_capacity(count);
        for _ in 0..count {
            let vertex_count = compact_count(&mut reader, 12, "brush polygon vertices")?;
            let base = read_vec3(&mut reader)?;
            let normal = read_vec3(&mut reader)?;
            let texture_u = read_vec3(&mut reader)?;
            let texture_v = read_vec3(&mut reader)?;
            let mut vertices = Vec::with_capacity(vertex_count);
            for _ in 0..vertex_count {
                vertices.push(read_vec3(&mut reader)?);
            }
            let poly_flags = PolyFlags::from_bits(reader.read_u32()?);
            let _actor = reader.read_object_reference()?;
            let texture = reader.read_object_reference()?;
            let _item_name = reader.read_name_index("brush polygon item name")?;
            let _link_index = reader.read_compact_index()?;
            let _brush_poly_index = reader.read_compact_index()?;
            let pan_u = reader.read_i16()?;
            let pan_v = reader.read_i16()?;
            polygons.push(BrushPolygon {
                base,
                normal,
                texture_u,
                texture_v,
                vertices,
                poly_flags,
                texture,
                pan_u,
                pan_v,
            });
        }
        Ok(Self { polygons })
    }
}

fn read_vec3(reader: &mut openhp1_package::ObjectReader<'_>) -> Result<Vec3> {
    Ok(Vec3::new(
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
    ))
}
