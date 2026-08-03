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

#[derive(Copy, Clone, Debug, PartialEq)]
pub(in crate::world) struct ModifySoundArguments {
    parameter: u8,
    value: f32,
    sound: Option<i32>,
    slot: u8,
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
        let slot = arguments.slot.unwrap_or(1);
        let object = self
            .object_for_handle(handle)
            .map_err(|error| error.to_string())?;
        let sound = object.clone();
        let object = self
            .resolved_object(&object)
            .map_err(|error| error.to_string())?;
        let clip = AudioClip::decode(&object.package, object.export_index)
            .map_err(|error| error.to_string())?;
        let duration = self
            .sound_duration(&[Value::Object(handle)])
            .map_err(|error| error.to_string())?;
        if !duration.is_finite() || duration < 0.0 {
            return Err(format!("{function} sound duration is invalid"));
        }
        let volume = match arguments.volume {
            Some(volume) => volume,
            None => self.actor_float(actor_class, instance, "TransientSoundVolume")?,
        };
        let radius = match arguments.radius {
            Some(radius) => radius,
            None => self.actor_float(actor_class, instance, "TransientSoundRadius")?,
        };
        let pitch = arguments.pitch.unwrap_or(1.0);
        if !self.start_sound(actor, slot, sound, duration, pitch, arguments.no_override) {
            return Ok(());
        }
        actions.push(ActorAction::PlaySound {
            actor,
            clip,
            location: self.actor_vector(actor_class, instance, "Location")?,
            slot,
            volume,
            no_override: arguments.no_override,
            radius,
            pitch,
        });
        Ok(())
    }

    pub(in crate::world) fn modify_sound(
        &mut self,
        actor: usize,
        arguments: &[Value],
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        let arguments = modify_sound_arguments(arguments)?;
        if !arguments.value.is_finite() {
            return Err("ModifySound value must be finite".to_owned());
        }
        if arguments.slot == 0 {
            return Ok(false);
        }
        let sound = match arguments.sound {
            None => None,
            Some(handle) => Some(
                self.object_for_handle(handle)
                    .map_err(|error| error.to_string())?,
            ),
        };
        let Some(channel) = self.sound_channels.get_mut(&(actor, arguments.slot)) else {
            return Ok(false);
        };
        if sound.as_ref().is_some_and(|sound| channel.sound != *sound) {
            return Ok(false);
        }
        if arguments.parameter == 2 {
            channel.pitch = arguments.value;
        }
        actions.push(ActorAction::ModifySound {
            actor,
            slot: arguments.slot,
            parameter: arguments.parameter,
            value: arguments.value,
        });
        Ok(true)
    }

    pub(in crate::world) fn stop_sound(
        &mut self,
        actor: usize,
        arguments: &[Value],
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if arguments.len() > 2 {
            return Err(format!(
                "StopSound expects an optional sound and slot, found {} arguments",
                arguments.len()
            ));
        }
        let (sound, clip) = match arguments.first() {
            None | Some(Value::None | Value::Object(0)) => (None, None),
            Some(Value::Object(handle)) => {
                let sound = self
                    .object_for_handle(*handle)
                    .map_err(|error| error.to_string())?;
                let object = self
                    .resolved_object(&sound)
                    .map_err(|error| error.to_string())?;
                let clip = AudioClip::decode(&object.package, object.export_index)
                    .map_err(|error| error.to_string())?;
                (Some(sound), Some(clip))
            }
            Some(value) => return Err(format!("StopSound sound is {}", value.kind())),
        };
        let slot = match arguments.get(1) {
            None | Some(Value::None) => None,
            Some(Value::Byte(slot)) => Some(*slot),
            Some(value) => return Err(format!("StopSound slot is {}", value.kind())),
        };
        self.sound_channels
            .retain(|(channel_actor, channel_slot), channel| {
                *channel_actor != actor
                    || sound.as_ref().is_some_and(|sound| channel.sound != *sound)
                    || slot.is_some_and(|slot| *channel_slot != slot)
            });
        actions.push(ActorAction::StopSound { actor, clip, slot });
        Ok(())
    }

    pub(in crate::world) fn start_sound(
        &mut self,
        actor: usize,
        slot: u8,
        sound: ObjectId,
        duration: f32,
        pitch: f32,
        no_override: bool,
    ) -> bool {
        if slot == 0 {
            return true;
        }
        if no_override && self.sound_channels.contains_key(&(actor, slot)) {
            return false;
        }
        self.sound_channels.insert(
            (actor, slot),
            SoundChannel {
                sound,
                remaining: duration,
                pitch,
            },
        );
        true
    }

    pub(in crate::world) fn tick_sound_channels(&mut self, delta_time: f32) {
        self.sound_channels.retain(|_, channel| {
            if channel.pitch > 0.0 {
                channel.remaining -= delta_time * channel.pitch;
            }
            channel.remaining > 0.0
        });
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

pub(in crate::world) fn modify_sound_arguments(
    arguments: &[Value],
) -> std::result::Result<ModifySoundArguments, String> {
    let [Value::Byte(parameter), Value::Float(value), rest @ ..] = arguments else {
        return Err(
            "ModifySound expects parameter, value, optional sound, and optional slot".to_owned(),
        );
    };
    if rest.len() > 2 {
        return Err(format!(
            "ModifySound expects parameter, value, optional sound, and optional slot, found {} arguments",
            arguments.len()
        ));
    }
    let sound = match rest.first() {
        None | Some(Value::None | Value::Object(0)) => None,
        Some(Value::Object(handle)) => Some(*handle),
        Some(value) => return Err(format!("ModifySound sound is {}", value.kind())),
    };
    let slot = match rest.get(1) {
        None | Some(Value::None) => 0,
        Some(Value::Byte(slot)) => *slot,
        Some(value) => return Err(format!("ModifySound slot is {}", value.kind())),
    };
    Ok(ModifySoundArguments {
        parameter: *parameter,
        value: *value,
        sound,
        slot,
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
