use super::state::set_event_disabled;
use super::*;

impl ScriptRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn native(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        index: u16,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        if matches!(index, ENABLE | DISABLE) {
            let [event] = arguments else {
                return Err(format!(
                    "{} expects one name, found {}",
                    if index == ENABLE { "Enable" } else { "Disable" },
                    arguments.len()
                ));
            };
            let event = runtime_name(source, event)?;
            let state = self
                .actor_states
                .get(&actor)
                .and_then(|state| state.as_deref());
            set_event_disabled(
                &mut self.disabled_events,
                actor,
                state,
                &event,
                index == DISABLE,
            );
            if index == ENABLE && event.eq_ignore_ascii_case("Tick") {
                self.failed_ticks.remove(&actor);
            }
            return Ok(Value::None);
        }
        if index == DESTROY {
            for name in ["bStatic", "bNoDelete"] {
                let field = self
                    .find_property(actor_class, name, 0)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("Destroy property {name} is missing"))?;
                match instance.get(&field) {
                    Some(StoredValue::Value(Value::Bool(true))) => {
                        return Ok(Value::Bool(false));
                    }
                    Some(StoredValue::Value(Value::Bool(false))) | None => {}
                    Some(value) => {
                        return Err(format!("Destroy property {name} is {value:?}"));
                    }
                }
            }
            if !self.destroyed.insert(actor) {
                return Ok(Value::Bool(true));
            }
            self.tick_functions.remove(&actor);
            self.failed_ticks.remove(&actor);
            self.state_frames.remove(&actor);
            let field = self
                .find_property(actor_class, "bDeleteMe", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Destroy property bDeleteMe is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Bool(true)));
            self.timers.remove(&actor);
            actions.push(ActorAction::DestroyActor { actor });
            return Ok(Value::Bool(true));
        }
        if index == GOTO_STATE {
            if arguments.len() > 2 {
                return Err(format!(
                    "GotoState expects at most 2 arguments, found {}",
                    arguments.len()
                ));
            }
            let state = match arguments.first() {
                Some(Value::None) | None => self
                    .actor_states
                    .get(&actor)
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| "None".to_owned()),
                Some(state) => runtime_name(source, state)?,
            };
            let label = arguments
                .get(1)
                .filter(|label| !matches!(label, Value::None))
                .map(|label| runtime_name(source, label))
                .transpose()?
                .unwrap_or_default();
            let state = if state.eq_ignore_ascii_case("None") {
                None
            } else {
                self.find_state(actor_class, &state)
                    .map_err(|error| error.to_string())?
            };
            self.set_actor_state(actor, actor_class, state, &label)
                .map_err(|error| error.to_string())?;
            return Ok(Value::None);
        }
        if index == SLEEP {
            let [Value::Float(seconds)] = arguments else {
                return Err(format!(
                    "Sleep expects one float, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if self.active_state_actor != Some(actor) {
                return Err("Sleep is only valid in state code".to_owned());
            }
            if !seconds.is_finite() {
                return Err("Sleep duration is not finite".to_owned());
            }
            self.pending_latent = Some(LatentAction::Sleep(seconds.max(0.0)));
            return Ok(Value::None);
        }
        if index == PLAY_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let (rate, tween_time) = animation_parameters("PlayAnim", rest)?;
            actions.push(ActorAction::PlayAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
            });
            return Ok(Value::None);
        }
        if index == LOOP_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let (rate, tween_time) = animation_parameters("LoopAnim", rest)?;
            actions.push(ActorAction::LoopAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
            });
            return Ok(Value::None);
        }
        if index == FINISH_ANIM {
            if arguments.len() > 1 {
                return Err(format!(
                    "FinishAnim expects at most one name, found {} arguments",
                    arguments.len()
                ));
            }
            if let Some(root) = arguments.first()
                && !matches!(root, Value::None)
            {
                runtime_name(source, root)?;
            }
            if self.active_state_actor != Some(actor) {
                return Err("FinishAnim is only valid in state code".to_owned());
            }
            self.pending_latent = Some(LatentAction::FinishAnimation);
            actions.push(ActorAction::AwaitAnimation { actor });
            return Ok(Value::None);
        }
        if index == PLAY_SOUND {
            actions.push(ActorAction::DeferredCall {
                actor,
                message: "PlaySound is not audible yet".to_owned(),
            });
            return Ok(Value::None);
        }
        if matches!(index, 0xfe | 0xff)
            && let [left, right] = arguments
        {
            let equal =
                runtime_name(source, left)?.eq_ignore_ascii_case(&runtime_name(source, right)?);
            return Ok(Value::Bool(equal == (index == 0xfe)));
        }
        if index == SET_TIMER
            && let [Value::Float(rate), rest @ ..] = arguments
        {
            if !rate.is_finite() {
                return Err("SetTimer rate is not finite".to_owned());
            }
            if *rate <= 0.0 {
                self.timers.remove(&actor);
                return Ok(Value::None);
            }
            let looping = match rest.first() {
                Some(Value::Bool(looping)) => *looping,
                Some(Value::None) | None => false,
                Some(value) => return Err(format!("SetTimer loop flag is {}", value.kind())),
            };
            self.timers.insert(
                actor,
                ActorTimer {
                    remaining: *rate,
                    rate: *rate,
                    looping,
                },
            );
            return Ok(Value::None);
        }
        if index == SET_COLLISION {
            for (name, value) in ["bCollideActors", "bBlockActors", "bBlockPlayers"]
                .into_iter()
                .zip(collision_updates(arguments)?)
            {
                let Some(value) = value else {
                    continue;
                };
                let field = self
                    .find_property(actor_class, name, 0)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("SetCollision property {name} is missing"))?;
                instance.insert(field, StoredValue::Value(Value::Bool(value)));
            }
            // ponytail: these flags become collision behavior when BSP movement exists.
            return Ok(Value::None);
        }
        if index == SET_LOCATION {
            let [Value::Vector(location)] = arguments else {
                return Err(format!(
                    "SetLocation expects one vector, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !location.iter().all(|value| value.is_finite()) {
                return Err("SetLocation coordinates are not finite".to_owned());
            }
            let field = self
                .find_property(actor_class, "Location", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SetLocation property Location is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Vector(*location)));
            actions.push(ActorAction::SetLocation {
                actor,
                location: *location,
            });
            // ponytail: accept finite locations until UE1 BSP collision rejection exists.
            return Ok(Value::Bool(true));
        }
        if index == SET_ROTATION {
            let [Value::Rotator(rotation)] = arguments else {
                return Err(format!(
                    "SetRotation expects one rotator, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let field = self
                .find_property(actor_class, "Rotation", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SetRotation property Rotation is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Rotator(*rotation)));
            actions.push(ActorAction::SetRotation {
                actor,
                rotation: *rotation,
            });
            // ponytail: accept rotations until UE1 collision rejection exists.
            return Ok(Value::Bool(true));
        }
        if index == 0xa7 {
            let [Value::Int(max)] = arguments else {
                return Err(format!(
                    "Rand expects one int, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            return Ok(Value::Int(random_int(&mut self.random_state, *max)));
        }
        if index == 0xc3 {
            if !arguments.is_empty() {
                return Err(format!(
                    "FRand expects no arguments, found {}",
                    arguments.len()
                ));
            }
            return Ok(Value::Float(random_float(&mut self.random_state)));
        }
        scalar_native(index, arguments)
    }
}

pub(super) fn animation_parameters(
    name: &str,
    arguments: &[Value],
) -> std::result::Result<(f32, f32), String> {
    let parameter = |index, label, default| match arguments.get(index) {
        Some(Value::Float(value)) if value.is_finite() => Ok(*value),
        Some(Value::Float(_)) => Err(format!("{name} {label} is not finite")),
        Some(Value::None) | None => Ok(default),
        Some(value) => Err(format!("{name} {label} is {}", value.kind())),
    };
    Ok((
        parameter(0, "rate", 1.0)?,
        parameter(1, "tween time", 0.0)?.max(0.0),
    ))
}

pub(super) fn runtime_name(source: &Package, value: &Value) -> std::result::Result<String, String> {
    match value {
        Value::Name(index) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < source.summary().names.len())
            .map(|index| source.summary().name(index).to_owned())
            .ok_or_else(|| format!("invalid name index {index}")),
        Value::NameText(name) => Ok(name.clone()),
        value => Err(format!("expected name, found {}", value.kind())),
    }
}

pub(super) fn collision_updates(
    arguments: &[Value],
) -> std::result::Result<[Option<bool>; 3], String> {
    if arguments.len() > 3 {
        return Err(format!(
            "SetCollision expects at most 3 arguments, found {}",
            arguments.len()
        ));
    }
    let mut updates = [None; 3];
    for (update, argument) in updates.iter_mut().zip(arguments) {
        match argument {
            Value::Bool(value) => *update = Some(*value),
            Value::None => {}
            value => return Err(format!("SetCollision flag is {}", value.kind())),
        }
    }
    Ok(updates)
}

pub(super) fn scalar_native(index: u16, arguments: &[Value]) -> std::result::Result<Value, String> {
    if index == 0xf5 {
        let [Value::Float(left), Value::Float(right)] = arguments else {
            return Err(format!(
                "FMax expects two floats, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        return Ok(Value::Float(if left < right { *right } else { *left }));
    }
    if matches!(index, 0x72 | 0x77)
        && let [left, right] = arguments
        && let (Some(left), Some(right)) = (object_value(left), object_value(right))
    {
        return Ok(Value::Bool((left == right) == (index == 0x72)));
    }
    if index == 0x91
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return left
            .checked_div(*right)
            .map(Value::Int)
            .ok_or_else(|| "integer division by zero or overflow".to_owned());
    }
    Ok(match (index, arguments) {
        (0x81, [value]) => Value::Bool(!value.truthy().map_err(|error| error.to_string())?),
        (0x82, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                && right.truthy().map_err(|error| error.to_string())?,
        ),
        (0x84, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                || right.truthy().map_err(|error| error.to_string())?,
        ),
        (0x90, [Value::Int(left), Value::Int(right)]) => Value::Int(left * right),
        (0x92, [Value::Int(left), Value::Int(right)]) => Value::Int(left + right),
        (0x93, [Value::Int(left), Value::Int(right)]) => Value::Int(left - right),
        (0x96, [Value::Int(left), Value::Int(right)]) => Value::Bool(left < right),
        (0x97, [Value::Int(left), Value::Int(right)]) => Value::Bool(left > right),
        (0x9a, [Value::Int(left), Value::Int(right)]) => Value::Bool(left == right),
        (0x9b, [Value::Int(left), Value::Int(right)]) => Value::Bool(left != right),
        (0x9c, [Value::Int(left), Value::Int(right)]) => Value::Int(left & right),
        (0x7a, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (0x7b, [Value::String(left), Value::String(right)]) => {
            Value::Bool(!left.eq_ignore_ascii_case(right))
        }
        (0xa9, [Value::Float(value)]) => Value::Float(-value),
        (0xab, [Value::Float(left), Value::Float(right)]) => Value::Float(left * right),
        (0xac, [Value::Float(left), Value::Float(right)]) => Value::Float(left / right),
        (0xae, [Value::Float(left), Value::Float(right)]) => Value::Float(left + right),
        (0xaf, [Value::Float(left), Value::Float(right)]) => Value::Float(left - right),
        (0xb0, [Value::Float(left), Value::Float(right)]) => Value::Bool(left < right),
        (0xb1, [Value::Float(left), Value::Float(right)]) => Value::Bool(left > right),
        (0xba, [Value::Float(value)]) => Value::Float(value.abs()),
        (0xd3, [Value::Vector(value)]) => Value::Vector([-value[0], -value[1], -value[2]]),
        (0xd4, [Value::Vector(value), Value::Float(scale)])
        | (0xd5, [Value::Float(scale), Value::Vector(value)]) => {
            Value::Vector([value[0] * scale, value[1] * scale, value[2] * scale])
        }
        (0xd6, [Value::Vector(value), Value::Float(divisor)]) => {
            Value::Vector([value[0] / divisor, value[1] / divisor, value[2] / divisor])
        }
        (0xd7, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] + right[0], left[1] + right[1], left[2] + right[2]])
        }
        (0xd8, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] - right[0], left[1] - right[1], left[2] - right[2]])
        }
        (0xe1, [Value::Vector(value)]) => {
            Value::Float((value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt())
        }
        (0xe2, [Value::Vector(value)]) => {
            let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
            if length > f32::EPSILON {
                Value::Vector([value[0] / length, value[1] / length, value[2] / length])
            } else {
                Value::Vector([0.0; 3])
            }
        }
        (0xfb, [Value::Int(value), Value::Int(min), Value::Int(max)]) => {
            Value::Int((*value).min(*max).max(*min))
        }
        _ => return Err(format!("native {index:#05x} is not implemented")),
    })
}

fn next_random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

pub(super) fn random_int(state: &mut u32, max: i32) -> i32 {
    let range = max.saturating_sub(1).clamp(0, 32_767) as u32 + 1;
    ((u64::from(next_random(state)) * u64::from(range)) >> 32) as i32
}

pub(super) fn random_float(state: &mut u32) -> f32 {
    (next_random(state) >> 8) as f32 / 16_777_216.0
}

fn object_value(value: &Value) -> Option<i32> {
    match value {
        Value::None => Some(0),
        Value::Object(value) => Some(*value),
        _ => None,
    }
}
