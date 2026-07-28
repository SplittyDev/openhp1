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
        if index == CLASS_IS_CHILD_OF {
            let [test, parent] = arguments else {
                return Err(format!(
                    "ClassIsChildOf expects two classes, found {}",
                    arguments.len()
                ));
            };
            let Some(test) = object_value(test) else {
                return Err(format!(
                    "ClassIsChildOf test class is {}, expected object",
                    test.kind()
                ));
            };
            let Some(parent) = object_value(parent) else {
                return Err(format!(
                    "ClassIsChildOf parent class is {}, expected object",
                    parent.kind()
                ));
            };
            let Some(test) = self
                .resolve_class_value(source, test)
                .map_err(|error| error.to_string())?
            else {
                return Ok(Value::Bool(false));
            };
            let Some(parent) = self
                .resolve_class_value(source, parent)
                .map_err(|error| error.to_string())?
            else {
                return Ok(Value::Bool(false));
            };
            return self
                .class_is_a(test, &parent)
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
            let root_motion = animation_root_motion("PlayAnim", source, rest, 3)?;
            self.start_animation(
                actor,
                actor_class,
                instance,
                name.clone(),
                rate,
                tween_time,
                false,
                false,
            )?;
            actions.push(ActorAction::PlayAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
                root_motion,
            });
            self.animating.insert(actor);
            return Ok(Value::None);
        }
        if index == LOOP_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let (rate, tween_time) = animation_parameters("LoopAnim", rest)?;
            let root_motion = animation_root_motion("LoopAnim", source, rest, 4)?;
            self.start_animation(
                actor,
                actor_class,
                instance,
                name.clone(),
                rate,
                tween_time,
                true,
                false,
            )?;
            actions.push(ActorAction::LoopAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
                root_motion,
            });
            self.animating.insert(actor);
            return Ok(Value::None);
        }
        if index == TWEEN_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let tween_time = match rest {
                [] | [Value::None] => 0.0,
                [Value::Float(value)] if value.is_finite() => value.max(0.0),
                [Value::Float(_)] => return Err("TweenAnim time is not finite".to_owned()),
                [value] => return Err(format!("TweenAnim time is {}", value.kind())),
                _ => {
                    return Err(format!(
                        "TweenAnim expects a name and time, found {} arguments",
                        arguments.len()
                    ));
                }
            };
            self.start_animation(
                actor,
                actor_class,
                instance,
                name.clone(),
                0.0,
                tween_time,
                false,
                true,
            )?;
            actions.push(ActorAction::PlayAnimation {
                actor,
                sequence: name,
                rate: 0.0,
                tween_time,
                root_motion: false,
            });
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
        if index == TURN_TOWARD {
            let [Value::Object(handle)] = arguments else {
                return Err(format!(
                    "TurnToward expects one actor, found {}",
                    arguments.len()
                ));
            };
            if *handle == 0 {
                return Ok(Value::None);
            }
            let target_actor = if *handle == -1 {
                actor
            } else {
                self.actor_for_handle(*handle)
                    .map_err(|error| error.to_string())?
            };
            let target = self
                .actor_objects
                .get(&target_actor)
                .cloned()
                .ok_or_else(|| format!("TurnToward target actor {target_actor} is unregistered"))?;
            let focus = self.other_actor_vector(target_actor, "Location")?;
            self.set_actor_stored(
                actor_class,
                instance,
                "FaceTarget",
                StoredValue::Object(Some(target)),
            )?;
            self.set_actor_value(actor_class, instance, "Focus", Value::Vector(focus))?;
            self.pending_latent = Some(LatentAction::TurnToward);
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
            self.set_actor_value(actor_class, instance, "bAnimLoop", Value::Bool(false))?;
            if let Some(command) = self.animation_commands.get_mut(&actor) {
                command.looping = false;
            }
            self.pending_latent = Some(LatentAction::FinishAnimation);
            actions.push(ActorAction::AwaitAnimation { actor });
            return Ok(Value::None);
        }
        if index == FINISH_INTERPOLATION {
            if !arguments.is_empty() {
                return Err(format!(
                    "FinishInterpolation expects no arguments, found {}",
                    arguments.len()
                ));
            }
            if self.active_state_actor != Some(actor) {
                return Err("FinishInterpolation is only valid in state code".to_owned());
            }
            self.pending_latent = Some(LatentAction::FinishInterpolation);
            return Ok(Value::None);
        }
        if index == PLAY_SOUND {
            self.play_sound(
                actor,
                actor_class,
                instance,
                "PlaySound",
                arguments,
                actions,
            )?;
            return Ok(Value::None);
        }
        if index == TRACE_TEXTURE {
            return trace_texture(arguments);
        }
        if index == MAKE_NOISE {
            noise_loudness(arguments)?;
            // ponytail: movement only needs MakeNoise not to abort; populate pawn
            // noise slots and dispatch HearNoise when AI hearing uses them.
            return Ok(Value::None);
        }
        if index == CREATE_ANIM_CHANNEL {
            let [class, Value::Byte(_), root_bone, rest @ ..] = arguments else {
                return Err(format!(
                    "CreateAnimChannel expects a class, type, root bone, and optional transient flag, found {}",
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
            {
                return Err("CreateAnimChannel transient flag is not a bool".to_owned());
            }
            runtime_name(source, root_bone)?;
            return self.spawn_actor(
                actor,
                actor_class,
                source,
                std::slice::from_ref(class),
                instance,
                actions,
            );
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
                self.animation_sequences
                    .get(&actor)
                    .and_then(|sequences| sequences.get(&sequence))
                    .map(|sequence| sequence.group.clone())
                    .unwrap_or_else(|| "None".to_owned()),
            ));
        }
        if index == BONE_NUMBER {
            let [bone] = arguments else {
                return Err(format!(
                    "BoneNumber expects one name, found {}",
                    arguments.len()
                ));
            };
            let bone = runtime_name(source, bone)?;
            return Ok(Value::Int(bone_number(
                self.actor_bone_names.get(&actor).map(Vec::as_slice),
                &bone,
            )));
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
        if index == SET_COLLISION_SIZE {
            let [Value::Float(radius), Value::Float(height), rest @ ..] = arguments else {
                return Err(format!(
                    "SetCollisionSize expects radius, height, and optional width floats, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let width = match rest {
                [] => None,
                [Value::Float(width)] => Some(*width),
                _ => {
                    return Err(format!(
                        "SetCollisionSize expects radius, height, and optional width floats, found {}",
                        arguments
                            .iter()
                            .map(Value::kind)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            };
            if !radius.is_finite()
                || !height.is_finite()
                || width.is_some_and(|width| !width.is_finite())
                || *radius < 0.0
                || *height < 0.0
                || width.is_some_and(|width| width < 0.0)
            {
                return Err("SetCollisionSize dimensions are invalid".to_owned());
            }
            self.set_actor_value(
                actor_class,
                instance,
                "CollisionRadius",
                Value::Float(*radius),
            )?;
            self.set_actor_value(
                actor_class,
                instance,
                "CollisionHeight",
                Value::Float(*height),
            )?;
            if let Some(width) = width {
                self.set_actor_value(actor_class, instance, "CollisionWidth", Value::Float(width))?;
            }
            self.refresh_cached_collision_actor(actor, actor_class, instance)?;
            return Ok(Value::Bool(true));
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
        if index == SET_OWNER {
            let [owner] = arguments else {
                return Err(format!(
                    "SetOwner expects one object, found {}",
                    arguments.len()
                ));
            };
            let owner = match self
                .stored_value(source, owner)
                .map_err(|error| error.to_string())?
            {
                StoredValue::Object(owner) => owner,
                value => return Err(format!("SetOwner object is {value:?}")),
            };
            self.set_actor_owner(actor, actor_class, instance, owner, actions)?;
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
        if index == V_RAND {
            if !arguments.is_empty() {
                return Err(format!(
                    "VRand expects no arguments, found {}",
                    arguments.len()
                ));
            }
            return Ok(Value::Vector(
                random_unit_vector(&mut self.random_state).to_array(),
            ));
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
        if index == SAVE_CONFIG {
            if !arguments.is_empty() {
                return Err(format!(
                    "SaveConfig expects no arguments, found {}",
                    arguments.len()
                ));
            }
            // ponytail: runtime configuration is read-only; persist UObject
            // config properties when settings need to survive process exit.
            return Ok(Value::None);
        }
        if index == FIND_PATH {
            let [start, destination] = arguments else {
                return Err(format!(
                    "FindPath expects a start point and destination name, found {}",
                    arguments.len()
                ));
            };
            let start = match start {
                Value::None | Value::Object(0) => return Ok(Value::Object(0)),
                Value::Object(handle) => self
                    .object_for_handle(*handle)
                    .map_err(|error| error.to_string())?,
                value => return Err(format!("FindPath start point is {}", value.kind())),
            };
            let destination = runtime_name(source, destination)?;
            let objects = self
                .reach_specs
                .iter()
                .flat_map(|spec| [&spec.start, &spec.end])
                .cloned()
                .collect::<Vec<_>>();
            let mut target = None;
            for object in objects {
                let resolved = self
                    .resolved_object(&object)
                    .map_err(|error| error.to_string())?;
                let summary = resolved.package.summary();
                if summary
                    .name(summary.exports[resolved.export_index].object_name)
                    .eq_ignore_ascii_case(&destination)
                {
                    target = Some(object);
                    break;
                }
            }
            let Some(target) = target else {
                return Ok(Value::Object(0));
            };
            let radius = self.actor_float(actor_class, instance, "CollisionRadius")? as i32;
            let height = self.actor_float(actor_class, instance, "CollisionHeight")? as i32;
            let Some(next) =
                next_navigation_step(&self.reach_specs, &start, &target, radius, height)
            else {
                return Ok(Value::Object(0));
            };
            return self
                .object_handle(next)
                .map(Value::Object)
                .map_err(|error| error.to_string());
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

    pub(super) fn pick_target(
        &mut self,
        actor: usize,
        arguments: &[Value],
    ) -> std::result::Result<(Value, f32, f32), String> {
        let [
            Value::Float(best_aim),
            Value::Float(best_dist),
            Value::Vector(fire_direction),
            Value::Vector(projectile_start),
        ] = arguments
        else {
            return Err(format!(
                "PickTarget expects best aim, best distance, fire direction, and projectile start, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let mut best_aim = *best_aim;
        let mut best_dist = *best_dist;
        let fire_direction = Vec3::from_array(*fire_direction);
        let projectile_start = Vec3::from_array(*projectile_start);
        if !best_aim.is_finite()
            || !best_dist.is_finite()
            || !fire_direction.is_finite()
            || !projectile_start.is_finite()
        {
            return Err("PickTarget arguments are not finite".to_owned());
        }

        let mut best = None;
        let mut candidates = self.actor_classes.keys().copied().collect::<Vec<_>>();
        candidates.sort_unstable();
        for candidate in candidates {
            if candidate == actor || self.destroyed.contains(&candidate) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&candidate)
                .cloned()
                .ok_or_else(|| format!("PickTarget actor {candidate} has no class"))?;
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            if !self
                .class_has_name(&class, "Pawn")
                .map_err(|error| error.to_string())?
            {
                continue;
            }
            let candidate_instance = self
                .instances
                .get(&candidate)
                .cloned()
                .ok_or_else(|| format!("PickTarget actor {candidate} has no instance"))?;
            let health =
                match self.required_actor_property(&class, &candidate_instance, "Health")? {
                    StoredValue::Value(Value::Int(health)) => health,
                    value => return Err(format!("PickTarget Health is {value:?}")),
                };
            if health <= 0 {
                continue;
            }
            let location =
                Vec3::from_array(self.actor_vector(&class, &candidate_instance, "Location")?);
            let Some((aim, distance)) =
                target_score(projectile_start, fire_direction, location, best_aim)
            else {
                continue;
            };
            if self.collision.as_ref().is_some_and(|collision| {
                collision
                    .sweep_aabb(projectile_start, location, Vec3::ZERO)
                    .is_some()
            }) {
                continue;
            }
            best_aim = aim;
            best_dist = distance;
            best = Some(candidate);
        }

        let value = match best {
            Some(candidate) => {
                let object = self
                    .actor_objects
                    .get(&candidate)
                    .cloned()
                    .ok_or_else(|| format!("PickTarget actor {candidate} has no object"))?;
                Value::Object(
                    self.object_handle(object)
                        .map_err(|error| error.to_string())?,
                )
            }
            None => Value::Object(0),
        };
        Ok((value, best_aim, best_dist))
    }

    fn set_actor_owner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        owner: Option<ObjectId>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let current = self.actor_object(actor_class, instance, "Owner")?;
        if current == owner {
            return Ok(());
        }
        let actor_object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
        let actor_handle = self
            .object_handle(actor_object)
            .map_err(|error| error.to_string())?;
        if let Some(old_owner) = current
            .as_ref()
            .and_then(|owner| self.object_actors.get(owner))
            .copied()
        {
            self.call_other_actor_event(
                old_owner,
                "LostChild",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }
        self.set_actor_stored(
            actor_class,
            instance,
            "Owner",
            StoredValue::Object(owner.clone()),
        )?;
        if let Some(new_owner) = owner
            .as_ref()
            .and_then(|owner| self.object_actors.get(owner))
            .copied()
        {
            self.call_other_actor_event(
                new_owner,
                "GainedChild",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }
        Ok(())
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
            .resolve_class_value(source, class_reference)
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
        if !self.spawn_location_is_clear(&class, &spawned_instance, actor, instance)? {
            return Ok(Value::Object(0));
        }

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

    #[allow(clippy::too_many_arguments)]
    fn start_animation(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        sequence: String,
        relative_rate: f32,
        tween_time: f32,
        looping: bool,
        tween_only: bool,
    ) -> std::result::Result<(), String> {
        let command = AnimationCommand {
            sequence,
            relative_rate,
            tween_time,
            looping,
            tween_only,
        };
        self.animation_commands.insert(actor, command.clone());
        self.configure_animation_instance(actor, class, instance, &command)
    }

    pub(super) fn synchronize_animation_command(
        &mut self,
        actor: usize,
    ) -> std::result::Result<(), String> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("animation actor {actor} is not registered"))?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| format!("animation actor {actor} is active"))?;
        let result = if let Some(command) = self.animation_commands.get(&actor).cloned() {
            self.configure_animation_instance(actor, &class, &mut instance, &command)
        } else {
            let anim_rate = self.actor_signed_float(&class, &instance, "AnimRate")?;
            let anim_frame = self.actor_signed_float(&class, &instance, "AnimFrame")?;
            let tween_rate = self.actor_float(&class, &instance, "TweenRate")?;
            if anim_rate != 0.0 || anim_frame < 0.0 && tween_rate != 0.0 {
                self.animating.insert(actor);
            }
            Ok(())
        };
        self.instances.insert(actor, instance);
        result
    }

    fn configure_animation_instance(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        command: &AnimationCommand,
    ) -> std::result::Result<(), String> {
        let current_sequence =
            match self.required_actor_property(class, instance, "AnimSequence")? {
                StoredValue::Name(name) => name,
                value => return Err(format!("actor property AnimSequence is {value:?}")),
            };
        let repeated_loop = command.looping
            && self.animating.contains(&actor)
            && self.actor_bool(class, instance, "bAnimLoop")?
            && current_sequence.eq_ignore_ascii_case(&command.sequence);
        self.set_actor_stored(
            class,
            instance,
            "AnimSequence",
            StoredValue::Name(command.sequence.clone()),
        )?;
        self.set_actor_value(class, instance, "bAnimLoop", Value::Bool(command.looping))?;
        self.set_actor_value(class, instance, "bAnimNotify", Value::Bool(false))?;
        self.set_actor_value(class, instance, "bAnimFinished", Value::Bool(false))?;

        let Some(sequence) = self
            .animation_sequences
            .get(&actor)
            .and_then(|sequences| sequences.get(&command.sequence.to_ascii_lowercase()))
            .cloned()
        else {
            return Ok(());
        };
        let frames = sequence.frame_count.max(1) as f32;
        let tween_rate = if command.tween_time > 0.0 {
            1.0 / (command.tween_time * frames)
        } else {
            0.0
        };
        if repeated_loop && sequence.frame_count > 1 {
            let anim_rate = command.relative_rate * sequence.rate / frames;
            for (name, value) in [("AnimRate", anim_rate), ("TweenRate", tween_rate)] {
                self.set_actor_value(class, instance, name, Value::Float(value))?;
            }
            return Ok(());
        }
        let (anim_frame, anim_last, anim_rate) = if command.tween_only {
            (
                if command.tween_time > 0.0 {
                    -1.0 / frames
                } else {
                    0.0
                },
                0.0,
                0.0,
            )
        } else if sequence.frame_count > 1 {
            (
                if command.tween_time > 0.0 {
                    -1.0 / frames
                } else {
                    0.0
                },
                1.0 - 1.0 / frames,
                command.relative_rate * sequence.rate / frames,
            )
        } else {
            (-1.0, 0.0, 0.0)
        };
        let tween_rate = if sequence.frame_count == 1 && command.tween_time == 0.0 {
            10.0
        } else {
            tween_rate
        };
        for (name, value) in [
            ("AnimFrame", anim_frame),
            ("AnimLast", anim_last),
            ("AnimRate", anim_rate),
            ("AnimMinRate", 0.0),
            ("TweenRate", tween_rate),
        ] {
            self.set_actor_value(class, instance, name, Value::Float(value))?;
        }
        if anim_rate != 0.0 || tween_rate != 0.0 {
            self.animating.insert(actor);
        } else {
            self.animating.remove(&actor);
        }
        Ok(())
    }

    pub(super) fn resolve_class_value(
        &mut self,
        source: &Arc<Package>,
        reference: i32,
    ) -> DispatchResult<Option<ResolvedObject>> {
        if reference == 0 {
            return Ok(None);
        }
        let handle_object = usize::try_from(reference - 1)
            .ok()
            .and_then(|index| self.handle_objects.get(index))
            .cloned();
        if let Some(object) = handle_object.as_ref() {
            let object = self.resolved_object(object)?;
            if self.is_spawn_class(&object) {
                return Ok(Some(object));
            }
        }
        let reference_object = object_reference(reference);
        let reference_in_bounds = match reference_object {
            ObjectReference::None => false,
            ObjectReference::Export(index) => index < source.summary().exports.len(),
            ObjectReference::Import(index) => index < source.summary().imports.len(),
        };
        if reference_in_bounds
            && let Some(object) = self.packages.resolve(source, reference_object)?
            && self.is_spawn_class(&object)
        {
            return Ok(Some(object));
        }
        let object =
            handle_object.ok_or(DispatchError::InvalidObjectHandle { handle: reference })?;
        let object = self.resolved_object(&object)?;
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

pub(super) fn next_navigation_step(
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

pub(super) fn noise_loudness(arguments: &[Value]) -> std::result::Result<f32, String> {
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

#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) struct SoundArguments {
    sound: Option<i32>,
    slot: Option<u8>,
    volume: Option<f32>,
    no_override: bool,
    radius: Option<f32>,
    pitch: Option<f32>,
}

impl ScriptRuntime {
    pub(super) fn play_sound(
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

pub(super) fn sound_arguments(
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
mod sound_tests {
    use super::*;

    #[test]
    fn omitted_sound_arguments_remain_authored_defaults() {
        assert_eq!(
            sound_arguments("PlaySound", &[Value::Object(1)]).unwrap(),
            SoundArguments {
                sound: Some(1),
                slot: None,
                volume: None,
                no_override: false,
                radius: None,
                pitch: None,
            }
        );
    }
}

pub(super) fn trace_texture(arguments: &[Value]) -> std::result::Result<Value, String> {
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

fn animation_root_motion(
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

pub(super) fn runtime_name(source: &Package, value: &Value) -> std::result::Result<String, String> {
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
    Subtract_PreInt,
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
    Percent_FloatFloat,
    Add_FloatFloat,
    Subtract_FloatFloat,
    Less_FloatFloat,
    Greater_FloatFloat,
    LessEqual_FloatFloat,
    GreaterEqual_FloatFloat,
    EqualEqual_FloatFloat,
    NotEqual_FloatFloat,
    Abs,
    Tan,
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
    MirrorVectorByNormal,
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
            0x8f => Ok(Self::Subtract_PreInt),
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
            0xad => Ok(Self::Percent_FloatFloat),
            0xae => Ok(Self::Add_FloatFloat),
            0xaf => Ok(Self::Subtract_FloatFloat),
            0xb0 => Ok(Self::Less_FloatFloat),
            0xb1 => Ok(Self::Greater_FloatFloat),
            0xb2 => Ok(Self::LessEqual_FloatFloat),
            0xb3 => Ok(Self::GreaterEqual_FloatFloat),
            0xb4 => Ok(Self::EqualEqual_FloatFloat),
            0xb5 => Ok(Self::NotEqual_FloatFloat),
            0xba => Ok(Self::Abs),
            0xbd => Ok(Self::Tan),
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
            0x12c => Ok(Self::MirrorVectorByNormal),
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
    let normalized = null_numeric_value(native).and_then(|zero| {
        arguments
            .iter()
            .any(|value| matches!(value, Value::None))
            .then(|| {
                arguments
                    .iter()
                    .map(|value| {
                        if matches!(value, Value::None) {
                            zero.clone()
                        } else {
                            value.clone()
                        }
                    })
                    .collect::<Vec<_>>()
            })
    });
    let arguments = normalized.as_deref().unwrap_or(arguments);
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
        (ScalarNative::Subtract_PreInt, [Value::Int(value)]) => Value::Int(value.wrapping_neg()),
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
        (ScalarNative::Percent_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left % right)
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
        (ScalarNative::Tan, [Value::Float(value)]) => Value::Float(value.tan()),
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
                x[0] * vector[0] + x[1] * vector[1] + x[2] * vector[2],
                y[0] * vector[0] + y[1] * vector[1] + y[2] * vector[2],
                z[0] * vector[0] + z[1] * vector[1] + z[2] * vector[2],
            ])
        }
        (
            ScalarNative::GreaterGreater_VectorRotator,
            [Value::Vector(vector), Value::Rotator(rotation)],
        ) => {
            let [x, y, z] = crate::rotator_axes(*rotation);
            Value::Vector([
                x[0] * vector[0] + y[0] * vector[1] + z[0] * vector[2],
                x[1] * vector[0] + y[1] * vector[1] + z[1] * vector[2],
                x[2] * vector[0] + y[2] * vector[1] + z[2] * vector[2],
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
        (ScalarNative::MirrorVectorByNormal, [Value::Vector(vector), Value::Vector(normal)]) => {
            let scale =
                2.0 * (vector[0] * normal[0] + vector[1] * normal[1] + vector[2] * normal[2]);
            Value::Vector([
                vector[0] - scale * normal[0],
                vector[1] - scale * normal[1],
                vector[2] - scale * normal[2],
            ])
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

fn null_numeric_value(native: ScalarNative) -> Option<Value> {
    Some(match native {
        ScalarNative::Subtract_PreInt
        | ScalarNative::Multiply_IntInt
        | ScalarNative::Divide_IntInt
        | ScalarNative::Add_IntInt
        | ScalarNative::Subtract_IntInt
        | ScalarNative::Less_IntInt
        | ScalarNative::Greater_IntInt
        | ScalarNative::LessEqual_IntInt
        | ScalarNative::GreaterEqual_IntInt
        | ScalarNative::EqualEqual_IntInt
        | ScalarNative::NotEqual_IntInt
        | ScalarNative::And_IntInt
        | ScalarNative::Min
        | ScalarNative::Max
        | ScalarNative::Clamp
        | ScalarNative::Chr => Value::Int(0),
        ScalarNative::Subtract_PreFloat
        | ScalarNative::Multiply_FloatFloat
        | ScalarNative::Divide_FloatFloat
        | ScalarNative::Add_FloatFloat
        | ScalarNative::Subtract_FloatFloat
        | ScalarNative::Less_FloatFloat
        | ScalarNative::Greater_FloatFloat
        | ScalarNative::LessEqual_FloatFloat
        | ScalarNative::GreaterEqual_FloatFloat
        | ScalarNative::EqualEqual_FloatFloat
        | ScalarNative::NotEqual_FloatFloat
        | ScalarNative::Abs
        | ScalarNative::Sqrt
        | ScalarNative::FMin
        | ScalarNative::FMax
        | ScalarNative::FClamp => Value::Float(0.0),
        _ => return None,
    })
}

pub(super) fn target_score(
    start: Vec3,
    direction: Vec3,
    target: Vec3,
    best_aim: f32,
) -> Option<(f32, f32)> {
    let delta = target - start;
    let distance = delta.length();
    if distance == 0.0 || distance > 2_500.0 {
        return None;
    }
    let aim = direction.dot(delta) / distance;
    (aim >= best_aim && aim >= 0.0).then_some((aim, distance))
}

pub(super) fn bone_number(bones: Option<&[String]>, name: &str) -> i32 {
    bones
        .and_then(|bones| {
            bones
                .iter()
                .position(|bone| bone.eq_ignore_ascii_case(name))
        })
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0)
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

pub(super) fn random_unit_vector(state: &mut u32) -> Vec3 {
    loop {
        let vector = Vec3::new(
            random_float(state) * 2.0 - 1.0,
            random_float(state) * 2.0 - 1.0,
            random_float(state) * 2.0 - 1.0,
        );
        let length_squared = vector.length_squared();
        if length_squared > f32::EPSILON && length_squared <= 1.0 {
            return vector / length_squared.sqrt();
        }
    }
}

fn object_value(value: &Value) -> Option<i32> {
    match value {
        Value::None => Some(0),
        Value::Object(value) => Some(*value),
        _ => None,
    }
}
