use glam::Vec3;

use super::physics::{PHYS_FALLING, PHYS_FLYING, PHYS_SWIMMING, PHYS_WALKING};
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
        depth: usize,
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
        if index == IS_A {
            let [name] = arguments else {
                return Err(format!("IsA expects one name, found {}", arguments.len()));
            };
            let name = runtime_name(source, name)?;
            return self
                .class_has_name(actor_class, &name)
                .map(Value::Bool)
                .map_err(|error| error.to_string());
        }
        if index == DESTROY {
            return self
                .destroy_actor(actor, actor_class, instance, actions)
                .map(Value::Bool);
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
            let old_state = self
                .actor_states
                .get(&actor)
                .and_then(Clone::clone)
                .unwrap_or_else(|| "None".to_owned());
            let state = if state.eq_ignore_ascii_case("None") {
                None
            } else {
                self.find_state(actor_class, &state)
                    .map_err(|error| error.to_string())?
            };
            let new_state = state
                .as_ref()
                .map(|state| {
                    state
                        .package
                        .summary()
                        .name(state.package.summary().exports[state.export_index].object_name)
                        .to_owned()
                })
                .unwrap_or_else(|| "None".to_owned());
            if !old_state.eq_ignore_ascii_case(&new_state)
                && let Some(function) = self
                    .find_actor_function(
                        actor,
                        ResolvedObject {
                            package: Arc::clone(&actor_class.package),
                            export_index: actor_class.export_index,
                        },
                        "EndState",
                        0,
                    )
                    .map_err(|error| error.to_string())?
            {
                self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    &[],
                    instance,
                    actions,
                    depth + 1,
                )
                .map_err(|error| error.to_string())?;
            }
            self.set_actor_state(actor, actor_class, state, &label)
                .map_err(|error| error.to_string())?;
            if !old_state.eq_ignore_ascii_case(&new_state)
                && let Some(function) = self
                    .find_actor_function(
                        actor,
                        ResolvedObject {
                            package: Arc::clone(&actor_class.package),
                            export_index: actor_class.export_index,
                        },
                        "BeginState",
                        0,
                    )
                    .map_err(|error| error.to_string())?
            {
                self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    &[],
                    instance,
                    actions,
                    depth + 1,
                )
                .map_err(|error| error.to_string())?;
            }
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
            self.animating.insert(actor);
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
            self.animating.insert(actor);
            return Ok(Value::None);
        }
        if index == IS_ANIMATING {
            if arguments.len() > 1 {
                return Err(format!(
                    "IsAnimating expects at most one name, found {} arguments",
                    arguments.len()
                ));
            }
            if let Some(root) = arguments.first()
                && !matches!(root, Value::None)
            {
                // ponytail: runtime animation has one channel per actor; distinguish
                // root-bone channels when the sampler supports them.
                runtime_name(source, root)?;
            }
            return Ok(Value::Bool(self.animating.contains(&actor)));
        }
        if index == TURN_TO {
            let [Value::Vector(focus)] = arguments else {
                return Err(format!(
                    "TurnTo expects one vector, found {}",
                    arguments.len()
                ));
            };
            self.set_actor_stored(
                actor_class,
                instance,
                "MoveTarget",
                StoredValue::Object(None),
            )?;
            self.set_actor_value(actor_class, instance, "Focus", Value::Vector(*focus))?;
            self.pending_latent = Some(LatentAction::TurnTo);
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
        if index == GET_ANIM_GROUP {
            let [sequence] = arguments else {
                return Err(format!(
                    "GetAnimGroup expects one name, found {}",
                    arguments.len()
                ));
            };
            let sequence = runtime_name(source, sequence)?.to_ascii_lowercase();
            return Ok(Value::NameText(
                self.animation_groups
                    .get(&actor)
                    .and_then(|groups| groups.get(&sequence))
                    .cloned()
                    .unwrap_or_else(|| "None".to_owned()),
            ));
        }
        if index == SPAWN {
            return self.spawn_actor(actor, actor_class, source, arguments, instance, actions);
        }
        if index == MOVE_TO {
            let [Value::Vector(destination), rest @ ..] = arguments else {
                return Err(format!(
                    "MoveTo expects a vector and optional speed, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let speed = match rest {
                [] | [Value::None] => 1.0,
                [Value::Float(speed)] if speed.is_finite() => *speed,
                [value] => return Err(format!("MoveTo speed is {}", value.kind())),
                _ => {
                    return Err(format!(
                        "MoveTo expects at most 2 arguments, found {}",
                        arguments.len()
                    ));
                }
            };
            if self.active_state_actor != Some(actor) {
                return Err("MoveTo is only valid in state code".to_owned());
            }
            let desired_speed = speed.clamp(
                0.0,
                self.actor_float(actor_class, instance, "MaxDesiredSpeed")?,
            );
            let location =
                Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
            let speed = match self.actor_byte(actor_class, instance, "Physics")? {
                PHYS_WALKING | PHYS_FALLING => {
                    self.actor_float(actor_class, instance, "GroundSpeed")?
                }
                PHYS_SWIMMING => self.actor_float(actor_class, instance, "WaterSpeed")?,
                PHYS_FLYING => self.actor_float(actor_class, instance, "AirSpeed")?,
                _ => 0.0,
            };
            let scale = desired_speed * speed;
            let duration = if scale > 0.0 {
                1.0 + 1.3 * (Vec3::from_array(*destination) - location).length() / scale
            } else {
                0.5
            };
            self.set_actor_stored(
                actor_class,
                instance,
                "MoveTarget",
                StoredValue::Object(None),
            )?;
            self.set_actor_value(actor_class, instance, "bReducedSpeed", Value::Bool(false))?;
            self.set_actor_value(
                actor_class,
                instance,
                "DesiredSpeed",
                Value::Float(desired_speed),
            )?;
            self.set_actor_value(
                actor_class,
                instance,
                "Destination",
                Value::Vector(*destination),
            )?;
            self.set_actor_value(actor_class, instance, "Focus", Value::Vector(*destination))?;
            self.set_actor_value(actor_class, instance, "MoveTimer", Value::Float(duration))?;
            self.pending_latent = Some(LatentAction::MoveTo);
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
        if index == IS_IN_STATE {
            let [state] = arguments else {
                return Err(format!(
                    "IsInState expects one name, found {}",
                    arguments.len()
                ));
            };
            let state = runtime_name(source, state)?;
            return Ok(Value::Bool(
                self.actor_states
                    .get(&actor)
                    .and_then(|current| current.as_deref())
                    .is_some_and(|current| current.eq_ignore_ascii_case(&state)),
            ));
        }
        if index == GET_STATE_NAME {
            if !arguments.is_empty() {
                return Err(format!(
                    "GetStateName expects no arguments, found {}",
                    arguments.len()
                ));
            }
            return Ok(Value::NameText(
                self.actor_states
                    .get(&actor)
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| "None".to_owned()),
            ));
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
            self.refresh_cached_collision_actor(actor, actor_class, instance)?;
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
        if matches!(index, MOVE | MOVE_SMOOTH) {
            let [Value::Vector(delta)] = arguments else {
                return Err(format!(
                    "{} expects one vector, found {}",
                    if index == MOVE_SMOOTH {
                        "MoveSmooth"
                    } else {
                        "Move"
                    },
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !delta.iter().all(|value| value.is_finite()) {
                return Err("Move delta is not finite".to_owned());
            }
            return if index == MOVE_SMOOTH {
                self.move_actor_smooth(actor, actor_class, *delta, instance, actions)
            } else {
                self.move_actor(actor, actor_class, *delta, instance, actions)
            };
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
            self.set_actor_location(
                actor,
                actor_class,
                instance,
                Vec3::from_array(*location),
                actions,
            )?;
            // ponytail: accept finite locations until UE1 BSP collision rejection exists.
            return Ok(Value::Bool(true));
        }
        if index == SET_BASE {
            let [base] = arguments else {
                return Err(format!(
                    "SetBase expects one object, found {}",
                    arguments.len()
                ));
            };
            let base = match self
                .stored_value(source, base)
                .map_err(|error| error.to_string())?
            {
                StoredValue::Object(base) => base,
                value => return Err(format!("SetBase object is {value:?}")),
            };
            self.set_actor_base(actor, actor_class, instance, base, actions)?;
            return Ok(Value::None);
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
        if index == RAND_RANGE {
            let [Value::Float(min), Value::Float(max)] = arguments else {
                return Err(format!(
                    "RandRange expects two floats, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !min.is_finite() || !max.is_finite() {
                return Err("RandRange bounds are not finite".to_owned());
            }
            return Ok(Value::Float(
                min + random_float(&mut self.random_state) * (max - min),
            ));
        }
        if index == CAN_SEE {
            let [other] = arguments else {
                return Err(format!(
                    "CanSee expects one actor, found {}",
                    arguments.len()
                ));
            };
            let other = match other {
                Value::None | Value::Object(0) => return Ok(Value::Bool(false)),
                Value::Object(handle) => self
                    .actor_for_handle(*handle)
                    .map_err(|error| error.to_string())?,
                value => return Err(format!("CanSee actor is {}", value.kind())),
            };
            return self
                .can_see(actor, actor_class, instance, other)
                .map(Value::Bool);
        }
        if index == ADD_PAWN {
            if !arguments.is_empty() {
                return Err(format!(
                    "AddPawn expects no arguments, found {}",
                    arguments.len()
                ));
            }
            self.add_pawn(actor, actor_class, instance)?;
            return Ok(Value::None);
        }
        scalar_native(index, arguments)
    }

    fn add_pawn(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<(), String> {
        let level = self
            .actor_object(actor_class, instance, "Level")?
            .ok_or_else(|| "AddPawn actor has no Level".to_owned())?;
        let level_actor = self
            .object_actors
            .get(&level)
            .copied()
            .ok_or_else(|| "AddPawn Level is not a registered actor".to_owned())?;
        let level_class = self
            .actor_classes
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("AddPawn Level actor {level_actor} has no class"))?;
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let mut level_instance = self
            .instances
            .remove(&level_actor)
            .ok_or_else(|| format!("AddPawn Level actor {level_actor} instance is active"))?;
        let result = (|| {
            let previous =
                match self.required_actor_property(&level_class, &level_instance, "PawnList")? {
                    StoredValue::Object(value) => value,
                    value => return Err(format!("AddPawn Level.PawnList is {value:?}")),
                };
            self.set_actor_stored(
                actor_class,
                instance,
                "nextPawn",
                StoredValue::Object(previous),
            )?;
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or_else(|| format!("AddPawn actor {actor} has no object identity"))?;
            self.set_actor_stored(
                &level_class,
                &mut level_instance,
                "PawnList",
                StoredValue::Object(Some(object)),
            )
        })();
        self.instances.insert(level_actor, level_instance);
        result
    }

    pub(super) fn destroy_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        for name in ["bStatic", "bNoDelete"] {
            let field = self
                .find_property(actor_class, name, 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Destroy property {name} is missing"))?;
            match instance.get(&field) {
                Some(StoredValue::Value(Value::Bool(true))) => return Ok(false),
                Some(StoredValue::Value(Value::Bool(false))) | None => {}
                Some(value) => return Err(format!("Destroy property {name} is {value:?}")),
            }
        }
        if !self.destroyed.insert(actor) {
            return Ok(true);
        }
        self.tick_functions.remove(&actor);
        self.failed_ticks.remove(&actor);
        self.state_frames.remove(&actor);
        self.update_actor_base(actor, None);
        if let Some(cached) = self.collision_actors.get_mut(actor) {
            *cached = None;
            self.reindex_cached_collision_actor(actor);
        }
        let field = self
            .find_property(actor_class, "bDeleteMe", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Destroy property bDeleteMe is missing".to_owned())?;
        instance.insert(field, StoredValue::Value(Value::Bool(true)));
        self.timers.remove(&actor);
        self.animating.remove(&actor);
        actions.push(ActorAction::DestroyActor { actor });
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        if arguments.is_empty() || arguments.len() > 5 {
            return Err(format!(
                "Spawn expects a class and at most 4 optional arguments, found {}",
                arguments.len()
            ));
        }
        let Some(class_reference) = object_value(&arguments[0]) else {
            return Err(format!(
                "Spawn class is {}, expected object",
                arguments[0].kind()
            ));
        };
        let Some(class) = self
            .resolve_spawn_class(source, class_reference)
            .map_err(|error| error.to_string())?
        else {
            return Ok(Value::Object(0));
        };
        let script = self.script(&class).map_err(|error| error.to_string())?;
        let ScriptMetadata::Class(metadata) = &script.metadata else {
            return Err("Spawn object is not a class".to_owned());
        };
        let summary = class.package.summary();
        let class_name = summary.name(summary.exports[class.export_index].object_name);
        if metadata.flags & CLASS_ABSTRACT != 0 {
            actions.push(ActorAction::DeferredCall {
                actor,
                message: format!("Spawn cannot instantiate abstract class {class_name}"),
            });
            return Ok(Value::Object(0));
        }
        let class_name = class_name.to_owned();

        let default_location = match self
            .instance_property(actor_class, instance, "Location")
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Vector(value))) => value,
            Some(value) => return Err(format!("Spawn source Location is {value:?}")),
            None => [0.0; 3],
        };
        let location = match arguments.get(3) {
            Some(Value::Vector(value)) => *value,
            Some(Value::None) | None => default_location,
            Some(value) => return Err(format!("Spawn location is {}", value.kind())),
        };
        if !location.iter().all(|value| value.is_finite()) {
            return Err("Spawn location is not finite".to_owned());
        }

        let default_rotation = match self
            .instance_property(actor_class, instance, "Rotation")
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Rotator(value))) => value,
            Some(value) => return Err(format!("Spawn source Rotation is {value:?}")),
            None => [0; 3],
        };
        let rotation = match arguments.get(4) {
            Some(Value::Rotator(value)) => *value,
            Some(Value::None) | None => default_rotation,
            Some(value) => return Err(format!("Spawn rotation is {}", value.kind())),
        };

        let owner = self
            .spawn_object_value(actor, arguments.get(1).unwrap_or(&Value::None))
            .map_err(|error| error.to_string())?;
        let tag = match arguments.get(2) {
            Some(Value::None) | None => class_name.clone(),
            Some(value) => {
                let value = runtime_name(source, value)?;
                if value.eq_ignore_ascii_case("None") {
                    class_name.clone()
                } else {
                    value
                }
            }
        };

        let spawned = self.next_actor;
        self.next_actor = self
            .next_actor
            .checked_add(1)
            .ok_or_else(|| DispatchError::ObjectLimit.to_string())?;
        let object = runtime_actor_id(spawned);
        let handle = self
            .object_handle(object.clone())
            .map_err(|error| error.to_string())?;
        self.object_actors.insert(object.clone(), spawned);
        self.actor_objects.insert(spawned, object);
        self.actor_classes
            .insert(spawned, object_id(&class.package, class.export_index));
        self.actor_states.insert(spawned, None);
        self.destroyed.remove(&spawned);

        let mut spawned_instance = self
            .load_class_defaults(&class, 0)
            .map_err(|error| error.to_string())?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "Location",
            StoredValue::Value(Value::Vector(location)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "OldLocation",
            StoredValue::Value(Value::Vector(location)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "Rotation",
            StoredValue::Value(Value::Rotator(rotation)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "DesiredRotation",
            StoredValue::Value(Value::Rotator(rotation)),
        )?;
        self.set_spawn_property(&class, &mut spawned_instance, "Tag", StoredValue::Name(tag))?;
        self.set_spawn_property(&class, &mut spawned_instance, "Owner", owner)?;
        for property in ["Instigator", "Level", "XLevel"] {
            if let Some(value) = self
                .instance_property(actor_class, instance, property)
                .map_err(|error| error.to_string())?
            {
                self.set_spawn_property(&class, &mut spawned_instance, property, value)?;
            }
        }

        self.refresh_tick_actor(spawned, &class)
            .map_err(|error| error.to_string())?;
        self.update_actor_base(spawned, None);
        self.refresh_cached_collision_actor(spawned, &class, &spawned_instance)?;
        self.instances.insert(spawned, spawned_instance);
        let name = format!("{class_name}{spawned}");
        actions.push(ActorAction::SpawnActor {
            actor: spawned,
            name,
            class_package: Arc::clone(&class.package.summary().source),
            class_export: class.export_index,
            class_name,
            location,
            rotation,
        });

        let parent_instance = std::mem::take(instance);
        if self.instances.insert(actor, parent_instance).is_some() {
            return Err(DispatchError::ActiveActorContext { actor }.to_string());
        }
        for event in [
            "Spawned",
            "PreBeginPlay",
            "BeginPlay",
            "PostBeginPlay",
            "SetInitialState",
        ] {
            match self.dispatch_event(
                spawned,
                Path::new(class.package.summary().source.as_ref()),
                class.export_index,
                event,
            ) {
                Ok(event_actions) => actions.extend(event_actions),
                Err(error) => actions.push(ActorAction::DeferredCall {
                    actor: spawned,
                    message: format!("{event}: {error}"),
                }),
            }
        }
        *instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| DispatchError::ActiveActorContext { actor }.to_string())?;

        Ok(if self.destroyed.contains(&spawned) {
            Value::Object(0)
        } else {
            Value::Object(handle)
        })
    }

    fn resolve_spawn_class(
        &mut self,
        source: &Arc<Package>,
        reference: i32,
    ) -> DispatchResult<Option<ResolvedObject>> {
        if reference == 0 {
            return Ok(None);
        }
        if let Some(object) = self.packages.resolve(source, object_reference(reference))?
            && self.is_spawn_class(&object)
        {
            return Ok(Some(object));
        }
        let index = usize::try_from(reference - 1)
            .ok()
            .filter(|index| *index < self.handle_objects.len())
            .ok_or(DispatchError::InvalidObjectHandle { handle: reference })?;
        let object = self.handle_objects[index].clone();
        let object = self.resolved_object(&object)?;
        if self.is_spawn_class(&object) {
            Ok(Some(object))
        } else {
            let summary = object.package.summary();
            let export = &summary.exports[object.export_index];
            Err(DispatchError::UnresolvedObject {
                message: format!(
                    "Spawn object {} `{}` is not a class",
                    summary.class_name(export).unwrap_or("<unknown>"),
                    summary.name(export.object_name)
                ),
            })
        }
    }

    fn is_spawn_class(&mut self, object: &ResolvedObject) -> bool {
        let summary = object.package.summary();
        match summary.class_name(&summary.exports[object.export_index]) {
            Some(name) => name.eq_ignore_ascii_case("Class"),
            None => self
                .script(object)
                .is_ok_and(|script| matches!(script.metadata, ScriptMetadata::Class(_))),
        }
    }

    fn spawn_object_value(
        &self,
        current_actor: usize,
        value: &Value,
    ) -> DispatchResult<StoredValue> {
        Ok(match value {
            Value::None | Value::Object(0) => StoredValue::Object(None),
            Value::Object(-1) => StoredValue::Object(Some(
                self.actor_objects.get(&current_actor).cloned().ok_or(
                    DispatchError::UnregisteredActor {
                        actor: current_actor,
                    },
                )?,
            )),
            Value::Object(handle) => {
                let index = usize::try_from(*handle - 1)
                    .ok()
                    .filter(|index| *index < self.handle_objects.len())
                    .ok_or(DispatchError::InvalidObjectHandle { handle: *handle })?;
                StoredValue::Object(Some(self.handle_objects[index].clone()))
            }
            value => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("Spawn owner is {}, expected object", value.kind()),
                });
            }
        })
    }

    fn set_spawn_property(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        name: &str,
        value: StoredValue,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(class, name, 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Spawn property {name} is missing"))?;
        instance.insert(field, value);
        Ok(())
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
    At_StrStr,
    EqualEqual_ObjectObject,
    NotEqual_ObjectObject,
    EqualEqual_StrStr,
    NotEqual_StrStr,
    ComplementEqual_StrStr,
    Len,
    InStr,
    Mid,
    Left,
    Right,
    Caps,
    Not_PreBool,
    AndAnd_BoolBool,
    XorXor_BoolBool,
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
    Sqrt,
    Subtract_PreVector,
    Multiply_VectorFloat,
    Multiply_FloatVector,
    Divide_VectorFloat,
    Add_VectorVector,
    Subtract_VectorVector,
    LessLess_VectorRotator,
    GreaterGreater_VectorRotator,
    EqualEqual_VectorVector,
    NotEqual_VectorVector,
    Dot_VectorVector,
    VSize,
    Normal,
    FMin,
    FMax,
    FClamp,
    Min,
    Max,
    Clamp,
    EqualEqual_BoolBool,
    NotEqual_BoolBool,
    Chr,
    Asc,
    Multiply_RotatorFloat,
    Multiply_FloatRotator,
    Divide_RotatorFloat,
    Add_RotatorRotator,
    Subtract_RotatorRotator,
}

impl TryFrom<u16> for ScalarNative {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0x70 => Ok(Self::Concat_StrStr),
            0xa8 => Ok(Self::At_StrStr),
            0x72 => Ok(Self::EqualEqual_ObjectObject),
            0x77 => Ok(Self::NotEqual_ObjectObject),
            0x7a => Ok(Self::EqualEqual_StrStr),
            0x7b => Ok(Self::NotEqual_StrStr),
            0x7c => Ok(Self::ComplementEqual_StrStr),
            0x7d => Ok(Self::Len),
            0x7e => Ok(Self::InStr),
            0x7f => Ok(Self::Mid),
            0x80 => Ok(Self::Left),
            0x81 => Ok(Self::Not_PreBool),
            0x82 => Ok(Self::AndAnd_BoolBool),
            0x83 => Ok(Self::XorXor_BoolBool),
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
            0xc1 => Ok(Self::Sqrt),
            0xd3 => Ok(Self::Subtract_PreVector),
            0xd4 => Ok(Self::Multiply_VectorFloat),
            0xd5 => Ok(Self::Multiply_FloatVector),
            0xd6 => Ok(Self::Divide_VectorFloat),
            0xd7 => Ok(Self::Add_VectorVector),
            0xd8 => Ok(Self::Subtract_VectorVector),
            0x113 => Ok(Self::LessLess_VectorRotator),
            0x114 => Ok(Self::GreaterGreater_VectorRotator),
            0xd9 => Ok(Self::EqualEqual_VectorVector),
            0xda => Ok(Self::NotEqual_VectorVector),
            0xdb => Ok(Self::Dot_VectorVector),
            0xe1 => Ok(Self::VSize),
            0xe2 => Ok(Self::Normal),
            0xea => Ok(Self::Right),
            0xeb => Ok(Self::Caps),
            0xec => Ok(Self::Chr),
            0xed => Ok(Self::Asc),
            0xf2 => Ok(Self::EqualEqual_BoolBool),
            0xf3 => Ok(Self::NotEqual_BoolBool),
            0xf4 => Ok(Self::FMin),
            0xf5 => Ok(Self::FMax),
            0xf6 => Ok(Self::FClamp),
            0xf9 => Ok(Self::Min),
            0xfa => Ok(Self::Max),
            0xfb => Ok(Self::Clamp),
            0x11f => Ok(Self::Multiply_RotatorFloat),
            0x120 => Ok(Self::Multiply_FloatRotator),
            0x121 => Ok(Self::Divide_RotatorFloat),
            0x13c => Ok(Self::Add_RotatorRotator),
            0x13d => Ok(Self::Subtract_RotatorRotator),
            _ => Err(()),
        }
    }
}

pub(super) fn scalar_native(index: u16, arguments: &[Value]) -> std::result::Result<Value, String> {
    let native = ScalarNative::try_from(index)
        .map_err(|()| format!("native {index:#05x} is not implemented"))?;
    if matches!(native, ScalarNative::FMin | ScalarNative::FMax) {
        let [Value::Float(left), Value::Float(right)] = arguments else {
            return Err(format!(
                "{native:?} expects two floats, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let value = match native {
            ScalarNative::FMin if right < left => *right,
            ScalarNative::FMax if left < right => *right,
            _ => *left,
        };
        return Ok(Value::Float(value));
    }
    if native == ScalarNative::FClamp
        && let [Value::Float(value), Value::Float(min), Value::Float(max)] = arguments
    {
        return Ok(Value::Float(value.min(*max).max(*min)));
    }
    if matches!(native, ScalarNative::Min | ScalarNative::Max)
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return Ok(Value::Int(if native == ScalarNative::Min {
            (*left).min(*right)
        } else {
            (*left).max(*right)
        }));
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
        (ScalarNative::At_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::String(format!("{left} {right}"))
        }
        (ScalarNative::Not_PreBool, [value]) => {
            Value::Bool(!value.truthy().map_err(|error| error.to_string())?)
        }
        (ScalarNative::AndAnd_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                && right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::XorXor_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                != right.truthy().map_err(|error| error.to_string())?,
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
        (ScalarNative::ComplementEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::Len, [Value::String(value)]) => Value::Int(value.chars().count() as i32),
        (ScalarNative::InStr, [Value::String(value), Value::String(needle)]) => Value::Int(
            value
                .find(needle)
                .map_or(-1, |index| value[..index].chars().count() as i32),
        ),
        (
            ScalarNative::Mid,
            [Value::String(value), Value::Int(start)]
            | [Value::String(value), Value::Int(start), Value::None],
        ) => {
            let start = usize::try_from(*start).unwrap_or_default();
            Value::String(value.chars().skip(start).collect())
        }
        (ScalarNative::Mid, [Value::String(value), Value::Int(start), Value::Int(length)]) => {
            let start = usize::try_from(*start).unwrap_or_default();
            let length = usize::try_from(*length).unwrap_or_default();
            Value::String(value.chars().skip(start).take(length).collect())
        }
        (ScalarNative::Left, [Value::String(value), Value::Int(length)]) => {
            let length = usize::try_from(*length).unwrap_or_default();
            Value::String(value.chars().take(length).collect())
        }
        (ScalarNative::Right, [Value::String(value), Value::Int(length)]) => {
            let length = usize::try_from(*length).unwrap_or_default();
            let skip = value.chars().count().saturating_sub(length);
            Value::String(value.chars().skip(skip).collect())
        }
        (ScalarNative::Caps, [Value::String(value)]) => Value::String(value.to_uppercase()),
        (ScalarNative::Asc, [Value::String(value)]) => Value::Int(i32::from(
            value.as_bytes().first().copied().unwrap_or_default(),
        )),
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
        (ScalarNative::Sqrt, [Value::Float(value)]) => Value::Float(value.sqrt()),
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
        (
            ScalarNative::LessLess_VectorRotator,
            [Value::Vector(vector), Value::Rotator(rotation)],
        ) => {
            let [x, y, z] = crate::rotator_axes(*rotation);
            Value::Vector([
                x[0] * vector[0] + y[0] * vector[1] + z[0] * vector[2],
                x[1] * vector[0] + y[1] * vector[1] + z[1] * vector[2],
                x[2] * vector[0] + y[2] * vector[1] + z[2] * vector[2],
            ])
        }
        (
            ScalarNative::GreaterGreater_VectorRotator,
            [Value::Vector(vector), Value::Rotator(rotation)],
        ) => {
            let [x, y, z] = crate::rotator_axes(*rotation);
            Value::Vector([
                x[0] * vector[0] + x[1] * vector[1] + x[2] * vector[2],
                y[0] * vector[0] + y[1] * vector[1] + y[2] * vector[2],
                z[0] * vector[0] + z[1] * vector[1] + z[2] * vector[2],
            ])
        }
        (ScalarNative::EqualEqual_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::Dot_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Float(left[0] * right[0] + left[1] * right[1] + left[2] * right[2])
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
        (ScalarNative::Multiply_RotatorFloat, [Value::Rotator(value), Value::Float(scale)])
        | (ScalarNative::Multiply_FloatRotator, [Value::Float(scale), Value::Rotator(value)]) => {
            Value::Rotator([
                (value[0] as f32 * scale) as i32,
                (value[1] as f32 * scale) as i32,
                (value[2] as f32 * scale) as i32,
            ])
        }
        (ScalarNative::Divide_RotatorFloat, [Value::Rotator(value), Value::Float(scale)]) => {
            Value::Rotator([
                (value[0] as f32 / scale) as i32,
                (value[1] as f32 / scale) as i32,
                (value[2] as f32 / scale) as i32,
            ])
        }
        (ScalarNative::Add_RotatorRotator, [Value::Rotator(left), Value::Rotator(right)]) => {
            Value::Rotator([
                left[0].wrapping_add(right[0]),
                left[1].wrapping_add(right[1]),
                left[2].wrapping_add(right[2]),
            ])
        }
        (ScalarNative::Subtract_RotatorRotator, [Value::Rotator(left), Value::Rotator(right)]) => {
            Value::Rotator([
                left[0].wrapping_sub(right[0]),
                left[1].wrapping_sub(right[1]),
                left[2].wrapping_sub(right[2]),
            ])
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
