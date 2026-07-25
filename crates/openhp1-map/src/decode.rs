use std::sync::Arc;

use glam::Vec3;
use openhp1_package::{ObjectReader, Package};

use crate::{Error, PrimitiveBounds, Result};

pub(crate) fn read_primitive_bounds(
    reader: &mut ObjectReader<'_>,
    sphere_radius: bool,
) -> Result<PrimitiveBounds> {
    let mut bounds = read_box(reader)?;
    bounds.sphere = [
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
        if sphere_radius {
            reader.read_f32()?
        } else {
            0.0
        },
    ];
    Ok(bounds)
}

pub(crate) fn read_box(reader: &mut ObjectReader<'_>) -> Result<PrimitiveBounds> {
    Ok(PrimitiveBounds {
        minimum: read_vec3(reader)?,
        maximum: read_vec3(reader)?,
        valid: reader.read_u8()? != 0,
        sphere: [0.0; 4],
    })
}

pub(crate) fn read_vec3(reader: &mut ObjectReader<'_>) -> Result<Vec3> {
    Ok(Vec3::new(
        reader.read_f32()?,
        reader.read_f32()?,
        reader.read_f32()?,
    ))
}

pub(crate) fn compact_count(
    reader: &mut ObjectReader<'_>,
    minimum_item_size: usize,
    field: &'static str,
) -> Result<usize> {
    let offset = reader.absolute_position();
    let value = reader.read_compact_index()?;
    let count = usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        value,
        offset,
    })?;
    if minimum_item_size != 0 && count > reader.remaining() / minimum_item_size {
        return Err(invalid_count(reader, field, count));
    }
    Ok(count)
}

pub(crate) fn fixed_count(reader: &mut ObjectReader<'_>, field: &'static str) -> Result<usize> {
    let offset = reader.absolute_position();
    let value = reader.read_i32()?;
    usize::try_from(value).map_err(|_| Error::InvalidCount {
        field,
        value,
        offset,
    })
}

pub(crate) fn invalid_count(reader: &ObjectReader<'_>, field: &'static str, count: usize) -> Error {
    Error::CountExceedsPayload {
        field,
        count,
        remaining: reader.remaining(),
        offset: reader.absolute_position(),
    }
}

pub(crate) fn index(value: i32, length: usize, field: &'static str) -> Result<usize> {
    usize::try_from(value)
        .ok()
        .filter(|index| *index < length)
        .ok_or(Error::InvalidIndex {
            field,
            value,
            length,
        })
}

pub(crate) fn point_index(value: i32, length: usize) -> Result<u32> {
    let index = index(value, length, "BSP vertex point")?;
    u32::try_from(index).map_err(|_| Error::MeshTooLarge {
        point_count: length,
    })
}

pub(crate) fn require_class(
    package: &Package,
    export_index: usize,
    expected: &'static str,
) -> Result<()> {
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
