use openhp1_package::{ObjectReference, Package};

use crate::{
    Error, Result,
    decode::{compact_count, fixed_count, require_class},
};

#[derive(Clone, Debug)]
pub struct Level {
    pub actors: Vec<ObjectReference>,
    pub model: ObjectReference,
    pub reach_specs: Vec<ReachSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReachSpec {
    pub distance: i32,
    pub start_actor: ObjectReference,
    pub end_actor: ObjectReference,
    pub collision_radius: i32,
    pub collision_height: i32,
    pub reach_flags: i32,
    pub pruned: bool,
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
        let reach_spec_count = compact_count(&mut reader, 19, "level reach specs")?;
        let mut reach_specs = Vec::with_capacity(reach_spec_count);
        for _ in 0..reach_spec_count {
            reach_specs.push(ReachSpec {
                distance: reader.read_i32()?,
                start_actor: reader.read_object_reference()?,
                end_actor: reader.read_object_reference()?,
                collision_radius: reader.read_i32()?,
                collision_height: reader.read_i32()?,
                reach_flags: reader.read_i32()?,
                pruned: reader.read_u8()? != 0,
            });
        }
        Ok(Self {
            actors,
            model,
            reach_specs,
        })
    }

    /// Returns the active `LevelInfo` stored in `Level.Actors(0)` by UE1.
    pub fn level_info_export(&self) -> Option<usize> {
        match self.actors.first() {
            Some(ObjectReference::Export(index)) => Some(*index),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use openhp1_package::ObjectReference;

    use super::Level;

    #[test]
    fn level_info_is_actor_zero_not_a_later_export() {
        let level = Level {
            actors: vec![ObjectReference::Export(7), ObjectReference::Export(3)],
            model: ObjectReference::None,
            reach_specs: Vec::new(),
        };

        assert_eq!(level.level_info_export(), Some(7));
    }
}
