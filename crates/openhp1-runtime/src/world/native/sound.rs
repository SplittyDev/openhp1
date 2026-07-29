use super::*;

#[derive(Copy, Clone, Debug, PartialEq)]
pub(in crate::world) struct SoundArguments {
    sound: Option<i32>,
    slot: Option<u8>,
    volume: Option<f32>,
    no_override: bool,
    radius: Option<f32>,
    pitch: Option<f32>,
}

impl ScriptRuntime {
    pub(in crate::world) fn play_sound(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &InstanceState,
        function: &str,
        arguments: &[Value],
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let arguments = sound_arguments(function, arguments)?;
        let Some(handle) = arguments.sound else {
            return Ok(());
        };
        let object = self
            .object_for_handle(handle)
            .map_err(|error| error.to_string())?;
        let object = self
            .resolved_object(&object)
            .map_err(|error| error.to_string())?;
        let clip = AudioClip::decode(&object.package, object.export_index)
            .map_err(|error| error.to_string())?;
        let volume = match arguments.volume {
            Some(volume) => volume,
            None => self.actor_float(actor_class, instance, "TransientSoundVolume")?,
        };
        let radius = match arguments.radius {
            Some(radius) => radius,
            None => self.actor_float(actor_class, instance, "TransientSoundRadius")?,
        };
        actions.push(ActorAction::PlaySound {
            actor,
            clip,
            location: self.actor_vector(actor_class, instance, "Location")?,
            slot: arguments.slot.unwrap_or(1),
            volume,
            no_override: arguments.no_override,
            radius,
            pitch: arguments.pitch.unwrap_or(1.0),
        });
        Ok(())
    }
}

pub(in crate::world) fn sound_arguments(
    function: &str,
    arguments: &[Value],
) -> std::result::Result<SoundArguments, String> {
    let [sound, rest @ ..] = arguments else {
        return Err(format!("{function} expects a sound"));
    };
    if !matches!(sound, Value::Object(_) | Value::None) || rest.len() > 5 {
        return Err(format!(
            "{function} expects a sound and at most 5 optional arguments"
        ));
    }
    for (value, kind) in rest.iter().zip(["byte", "float", "bool", "float", "float"]) {
        let valid = match (value, kind) {
            (Value::None, _) | (Value::Byte(_), "byte") | (Value::Bool(_), "bool") => true,
            (Value::Float(value), "float") => value.is_finite(),
            _ => false,
        };
        if !valid {
            return Err(format!("{function} {kind} argument is {}", value.kind()));
        }
    }
    Ok(SoundArguments {
        sound: match sound {
            Value::None | Value::Object(0) => None,
            Value::Object(handle) => Some(*handle),
            _ => unreachable!(),
        },
        slot: optional_byte(rest.first()),
        volume: optional_float(rest.get(1)),
        no_override: optional_bool(rest.get(2), false),
        radius: optional_float(rest.get(3)),
        pitch: optional_float(rest.get(4)),
    })
}

fn optional_byte(value: Option<&Value>) -> Option<u8> {
    match value {
        Some(Value::Byte(value)) => Some(*value),
        _ => None,
    }
}

fn optional_float(value: Option<&Value>) -> Option<f32> {
    match value {
        Some(Value::Float(value)) => Some(*value),
        _ => None,
    }
}

fn optional_bool(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        _ => default,
    }
}

#[cfg(test)]
#[path = "sound_tests.rs"]
mod tests;
