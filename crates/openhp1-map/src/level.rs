use openhp1_package::{ObjectReference, Package};

use crate::{
    Error, Result,
    decode::{compact_count, fixed_count, require_class},
};

#[derive(Clone, Debug)]
pub struct Level {
    pub actors: Vec<ObjectReference>,
    pub model: ObjectReference,
}

impl Level {
    /// Decodes enough of `ULevel` to identify the authoritative world model.
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Level")?;
        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}

        let actor_count = fixed_count(&mut reader, "level actors")?;
        let _actor_capacity = reader.read_i32()?;
        let mut actors = Vec::with_capacity(actor_count);
        for _ in 0..actor_count {
            actors.push(reader.read_object_reference()?);
        }

        // ULevelBase serializes the URL that was used to enter the map.
        for _ in 0..4 {
            reader.read_string()?;
        }
        let option_count = compact_count(&mut reader, 1, "level URL options")?;
        for _ in 0..option_count {
            reader.read_string()?;
        }
        reader.read_i32()?; // port
        reader.read_u32()?; // legacy URL field
        let model = reader.read_object_reference()?;
        Ok(Self { actors, model })
    }
}

pub fn world_model_export(package: &Package) -> Result<usize> {
    let level_index = package
        .summary()
        .exports
        .iter()
        .position(|export| package.summary().class_name(export) == Some("Level"))
        .ok_or(Error::MissingLevel)?;
    match Level::decode(package, level_index)?.model {
        ObjectReference::Export(index) => Ok(index),
        reference => Err(Error::InvalidWorldModel { reference }),
    }
}
