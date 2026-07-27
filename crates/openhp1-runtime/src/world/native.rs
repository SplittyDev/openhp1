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
        if index == SET_PHYSICS {
            let [Value::Byte(physics)] = arguments else {
                return Err(format!(
                    "SetPhysics expects one byte, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let field = self
                .find_property(actor_class, "Physics", 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "SetPhysics property Physics is missing".to_owned())?;
            instance.insert(field, StoredValue::Value(Value::Byte(*physics)));
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
        if index == LOG {
            let (message, tag) = log_arguments(arguments)?;
            let tag = tag.map(|tag| runtime_name(source, tag)).transpose()?;
            actions.push(ActorAction::Log {
                actor,
                message: message.to_owned(),
                tag,
            });
            return Ok(Value::None);
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

pub(super) fn log_arguments(
    arguments: &[Value],
) -> std::result::Result<(&str, Option<&Value>), String> {
    match arguments {
        [Value::String(message)] | [Value::String(message), Value::None] => Ok((message, None)),
        [Value::String(message), tag] => Ok((message, Some(tag))),
        _ => Err(format!(
            "Log expects a string and optional name, found {}",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        )),
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

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ScalarNative {
    Concat_StrStr,
    EqualEqual_ObjectObject,
    NotEqual_ObjectObject,
    EqualEqual_StrStr,
    NotEqual_StrStr,
    Not_PreBool,
    AndAnd_BoolBool,
    OrOr_BoolBool,
    Multiply_IntInt,
    Divide_IntInt,
    Add_IntInt,
    Subtract_IntInt,
    Less_IntInt,
    Greater_IntInt,
    LessEqual_IntInt,
    GreaterEqual_IntInt,
    EqualEqual_IntInt,
    NotEqual_IntInt,
    And_IntInt,
    Subtract_PreFloat,
    Multiply_FloatFloat,
    Divide_FloatFloat,
    Add_FloatFloat,
    Subtract_FloatFloat,
    Less_FloatFloat,
    Greater_FloatFloat,
    LessEqual_FloatFloat,
    GreaterEqual_FloatFloat,
    EqualEqual_FloatFloat,
    NotEqual_FloatFloat,
    Abs,
    Subtract_PreVector,
    Multiply_VectorFloat,
    Multiply_FloatVector,
    Divide_VectorFloat,
    Add_VectorVector,
    Subtract_VectorVector,
    VSize,
    Normal,
    FMax,
    Clamp,
    EqualEqual_BoolBool,
    NotEqual_BoolBool,
    Chr,
}

impl TryFrom<u16> for ScalarNative {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0x70 => Ok(Self::Concat_StrStr),
            0x72 => Ok(Self::EqualEqual_ObjectObject),
            0x77 => Ok(Self::NotEqual_ObjectObject),
            0x7a => Ok(Self::EqualEqual_StrStr),
            0x7b => Ok(Self::NotEqual_StrStr),
            0x81 => Ok(Self::Not_PreBool),
            0x82 => Ok(Self::AndAnd_BoolBool),
            0x84 => Ok(Self::OrOr_BoolBool),
            0x90 => Ok(Self::Multiply_IntInt),
            0x91 => Ok(Self::Divide_IntInt),
            0x92 => Ok(Self::Add_IntInt),
            0x93 => Ok(Self::Subtract_IntInt),
            0x96 => Ok(Self::Less_IntInt),
            0x97 => Ok(Self::Greater_IntInt),
            0x98 => Ok(Self::LessEqual_IntInt),
            0x99 => Ok(Self::GreaterEqual_IntInt),
            0x9a => Ok(Self::EqualEqual_IntInt),
            0x9b => Ok(Self::NotEqual_IntInt),
            0x9c => Ok(Self::And_IntInt),
            0xa9 => Ok(Self::Subtract_PreFloat),
            0xab => Ok(Self::Multiply_FloatFloat),
            0xac => Ok(Self::Divide_FloatFloat),
            0xae => Ok(Self::Add_FloatFloat),
            0xaf => Ok(Self::Subtract_FloatFloat),
            0xb0 => Ok(Self::Less_FloatFloat),
            0xb1 => Ok(Self::Greater_FloatFloat),
            0xb2 => Ok(Self::LessEqual_FloatFloat),
            0xb3 => Ok(Self::GreaterEqual_FloatFloat),
            0xb4 => Ok(Self::EqualEqual_FloatFloat),
            0xb5 => Ok(Self::NotEqual_FloatFloat),
            0xba => Ok(Self::Abs),
            0xd3 => Ok(Self::Subtract_PreVector),
            0xd4 => Ok(Self::Multiply_VectorFloat),
            0xd5 => Ok(Self::Multiply_FloatVector),
            0xd6 => Ok(Self::Divide_VectorFloat),
            0xd7 => Ok(Self::Add_VectorVector),
            0xd8 => Ok(Self::Subtract_VectorVector),
            0xe1 => Ok(Self::VSize),
            0xe2 => Ok(Self::Normal),
            0xec => Ok(Self::Chr),
            0xf2 => Ok(Self::EqualEqual_BoolBool),
            0xf3 => Ok(Self::NotEqual_BoolBool),
            0xf5 => Ok(Self::FMax),
            0xfb => Ok(Self::Clamp),
            _ => Err(()),
        }
    }
}

pub(super) fn scalar_native(index: u16, arguments: &[Value]) -> std::result::Result<Value, String> {
    let native = ScalarNative::try_from(index)
        .map_err(|()| format!("native {index:#05x} is not implemented"))?;
    if native == ScalarNative::FMax {
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
    if matches!(
        native,
        ScalarNative::EqualEqual_ObjectObject | ScalarNative::NotEqual_ObjectObject
    ) && let [left, right] = arguments
        && let (Some(left), Some(right)) = (object_value(left), object_value(right))
    {
        return Ok(Value::Bool(
            (left == right) == (native == ScalarNative::EqualEqual_ObjectObject),
        ));
    }
    if native == ScalarNative::Divide_IntInt
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return left
            .checked_div(*right)
            .map(Value::Int)
            .ok_or_else(|| "integer division by zero or overflow".to_owned());
    }
    Ok(match (native, arguments) {
        (ScalarNative::Concat_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::String(left.clone() + right)
        }
        (ScalarNative::Not_PreBool, [value]) => {
            Value::Bool(!value.truthy().map_err(|error| error.to_string())?)
        }
        (ScalarNative::AndAnd_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                && right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::OrOr_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                || right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::Multiply_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left * right)
        }
        (ScalarNative::Add_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left + right)
        }
        (ScalarNative::Subtract_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left - right)
        }
        (ScalarNative::Less_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left < right)
        }
        (ScalarNative::Greater_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left > right)
        }
        (ScalarNative::LessEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left <= right)
        }
        (ScalarNative::GreaterEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left >= right)
        }
        (ScalarNative::EqualEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::And_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left & right)
        }
        (ScalarNative::EqualEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::NotEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(!left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::Subtract_PreFloat, [Value::Float(value)]) => Value::Float(-value),
        (ScalarNative::Multiply_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left * right)
        }
        (ScalarNative::Divide_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left / right)
        }
        (ScalarNative::Add_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left + right)
        }
        (ScalarNative::Subtract_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left - right)
        }
        (ScalarNative::Less_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left < right)
        }
        (ScalarNative::Greater_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left > right)
        }
        (ScalarNative::LessEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left <= right)
        }
        (ScalarNative::GreaterEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left >= right)
        }
        (ScalarNative::EqualEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::Abs, [Value::Float(value)]) => Value::Float(value.abs()),
        (ScalarNative::Subtract_PreVector, [Value::Vector(value)]) => {
            Value::Vector([-value[0], -value[1], -value[2]])
        }
        (ScalarNative::Multiply_VectorFloat, [Value::Vector(value), Value::Float(scale)])
        | (ScalarNative::Multiply_FloatVector, [Value::Float(scale), Value::Vector(value)]) => {
            Value::Vector([value[0] * scale, value[1] * scale, value[2] * scale])
        }
        (ScalarNative::Divide_VectorFloat, [Value::Vector(value), Value::Float(divisor)]) => {
            Value::Vector([value[0] / divisor, value[1] / divisor, value[2] / divisor])
        }
        (ScalarNative::Add_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] + right[0], left[1] + right[1], left[2] + right[2]])
        }
        (ScalarNative::Subtract_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] - right[0], left[1] - right[1], left[2] - right[2]])
        }
        (ScalarNative::VSize, [Value::Vector(value)]) => {
            Value::Float((value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt())
        }
        (ScalarNative::Normal, [Value::Vector(value)]) => {
            let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
            if length > f32::EPSILON {
                Value::Vector([value[0] / length, value[1] / length, value[2] / length])
            } else {
                Value::Vector([0.0; 3])
            }
        }
        (ScalarNative::Clamp, [Value::Int(value), Value::Int(min), Value::Int(max)]) => {
            Value::Int((*value).min(*max).max(*min))
        }
        (ScalarNative::EqualEqual_BoolBool, [Value::Bool(left), Value::Bool(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_BoolBool, [Value::Bool(left), Value::Bool(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::Chr, [Value::Int(value)]) => {
            Value::String(char::from(*value as u8).to_string())
        }
        _ => {
            return Err(format!(
                "{native:?} does not accept operands ({})",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
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
