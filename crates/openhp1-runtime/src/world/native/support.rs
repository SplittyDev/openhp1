use super::*;

pub(in crate::world) fn next_navigation_step(
    specs: &[NavigationReachSpec],
    start: &ObjectId,
    target: &ObjectId,
    radius: i32,
    height: i32,
) -> Option<ObjectId> {
    if start == target {
        return Some(target.clone());
    }
    let mut distances = HashMap::default();
    let mut previous = HashMap::default();
    let mut pending = vec![start.clone()];
    distances.insert(start.clone(), 0_i32);
    while !pending.is_empty() {
        let index = pending
            .iter()
            .enumerate()
            .min_by_key(|(_, node)| distances.get(*node).copied().unwrap_or(i32::MAX))?
            .0;
        let current = pending.swap_remove(index);
        let distance = distances[&current];
        if current == *target {
            break;
        }
        for spec in specs.iter().filter(|spec| {
            spec.start == current
                && !spec.pruned
                && spec.collision_radius >= radius
                && spec.collision_height >= height
        }) {
            let candidate = distance.saturating_add(spec.distance.max(0));
            if distances
                .get(&spec.end)
                .is_some_and(|known| *known <= candidate)
            {
                continue;
            }
            distances.insert(spec.end.clone(), candidate);
            previous.insert(spec.end.clone(), current.clone());
            if !pending.contains(&spec.end) {
                pending.push(spec.end.clone());
            }
        }
    }
    let mut step = target.clone();
    while let Some(parent) = previous.get(&step) {
        if parent == start {
            return Some(step);
        }
        step = parent.clone();
    }
    None
}

pub(in crate::world) fn log_arguments(
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

pub(in crate::world) fn noise_loudness(arguments: &[Value]) -> std::result::Result<f32, String> {
    let [Value::Float(loudness)] = arguments else {
        return Err(format!(
            "MakeNoise expects one float, found {}",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if !loudness.is_finite() {
        return Err("MakeNoise loudness is not finite".to_owned());
    }
    Ok(*loudness)
}

pub(in crate::world) fn trace_texture(arguments: &[Value]) -> std::result::Result<Value, String> {
    let [
        Value::Vector(start),
        Value::Vector(end),
        Value::Int(_),
        rest @ ..,
    ] = arguments
    else {
        return Err(format!(
            "TraceTexture expects start, end, flags, and an optional decal flag, found {}",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if rest.len() > 1
        || rest
            .first()
            .is_some_and(|value| !matches!(value, Value::Bool(_) | Value::None))
        || !start.iter().chain(end).all(|value| value.is_finite())
    {
        return Err("TraceTexture arguments are invalid".to_owned());
    }
    // ponytail: BSP collision does not retain surface texture identities yet;
    // return no texture until material-aware traces have a gameplay consumer.
    Ok(Value::Object(0))
}

pub(in crate::world) fn animation_parameters(
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

pub(in crate::world) fn animation_root_motion(
    name: &str,
    source: &Package,
    arguments: &[Value],
    root_index: usize,
) -> std::result::Result<bool, String> {
    if arguments.len() > root_index + 1 {
        return Err(format!(
            "{name} expects at most {} optional arguments, found {}",
            root_index + 1,
            arguments.len()
        ));
    }
    arguments
        .get(root_index)
        .filter(|value| !matches!(value, Value::None))
        .map(|value| runtime_name(source, value).map(|root| root.eq_ignore_ascii_case("Move")))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(in crate::world) fn runtime_name(
    source: &Package,
    value: &Value,
) -> std::result::Result<String, String> {
    match value {
        Value::Name(index) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < source.summary().names.len())
            .map(|index| source.summary().name(index).to_owned())
            .ok_or_else(|| format!("invalid name index {index}")),
        Value::NameText(name) => Ok(name.clone()),
        Value::None => Ok("None".to_owned()),
        value => Err(format!("expected name, found {}", value.kind())),
    }
}

pub(in crate::world) fn bone_position(
    bones: Option<&[String]>,
    positions: Option<&[[f32; 3]]>,
    bone: &str,
) -> [f32; 3] {
    bones
        .and_then(|bones| {
            bones
                .iter()
                .position(|name| name.eq_ignore_ascii_case(bone))
        })
        .and_then(|index| positions.and_then(|positions| positions.get(index)))
        .copied()
        .unwrap_or([0.0; 3])
}

pub(in crate::world) fn collision_updates(
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
