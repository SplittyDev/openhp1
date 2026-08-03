use glam::Vec3;

use super::physics::{PHYS_FALLING, PHYS_FLYING, PHYS_SWIMMING, PHYS_WALKING};
use super::state::set_event_disabled;
use super::*;

mod actor;
mod gesture;
mod scalar;
mod sound;
mod support;

pub(super) use scalar::{
    bone_number, object_value, random_float, random_int, random_rotator, random_unit_vector,
    scalar_native, target_score,
};
#[cfg(test)]
pub(super) use sound::sound_arguments;
pub(super) use support::{
    animation_parameters, animation_root_motion, bone_position, collision_updates, log_arguments,
    next_navigation_step, noise_loudness, runtime_name, trace_texture,
};

use gesture::{gesture_native, gesture_points};

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
        if matches!(index, COMPARE_GESTURE | COMPARE_GESTURE_POINT) {
            let points = gesture_points(self, actor_class, instance)?;
            return gesture_native(index, arguments, &points);
        }
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
            if self.active_state_actor.is_none() {
                return Err("TurnTo is only valid in state code".to_owned());
            }
            self.set_actor_stored(
                actor_class,
                instance,
                "MoveTarget",
                StoredValue::Object(None),
            )?;
            self.set_actor_value(actor_class, instance, "Focus", Value::Vector(*focus))?;
            self.pending_latent = Some(LatentAction::TurnTo(actor));
            return Ok(Value::None);
        }
        if index == TURN_TOWARD {
            let [Value::Object(handle)] = arguments else {
                return Err(format!(
                    "TurnToward expects one actor, found {}",
                    arguments.len()
                ));
            };
            if self.active_state_actor.is_none() {
                return Err("TurnToward is only valid in state code".to_owned());
            }
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
            self.pending_latent = Some(LatentAction::TurnToward(actor));
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
            if self.active_state_actor.is_none() {
                return Err("FinishAnim is only valid in state code".to_owned());
            }
            self.set_actor_value(actor_class, instance, "bAnimLoop", Value::Bool(false))?;
            if let Some(command) = self.animation_commands.get_mut(&actor) {
                command.looping = false;
            }
            self.pending_latent = Some(LatentAction::FinishAnimation(actor));
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
            if self.active_state_actor.is_none() {
                return Err("FinishInterpolation is only valid in state code".to_owned());
            }
            self.pending_latent = Some(LatentAction::FinishInterpolation(actor));
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
        if index == STOP_SOUND {
            if arguments.len() > 2 {
                return Err(format!(
                    "StopSound expects an optional sound and slot, found {} arguments",
                    arguments.len()
                ));
            }
            let clip = match arguments.first() {
                None | Some(Value::None | Value::Object(0)) => None,
                Some(Value::Object(handle)) => {
                    let object = self
                        .object_for_handle(*handle)
                        .map_err(|error| error.to_string())?;
                    let object = self
                        .resolved_object(&object)
                        .map_err(|error| error.to_string())?;
                    Some(
                        AudioClip::decode(&object.package, object.export_index)
                            .map_err(|error| error.to_string())?,
                    )
                }
                Some(value) => return Err(format!("StopSound sound is {}", value.kind())),
            };
            let slot = match arguments.get(1) {
                None | Some(Value::None) => None,
                Some(Value::Byte(slot)) => Some(*slot),
                Some(value) => return Err(format!("StopSound slot is {}", value.kind())),
            };
            actions.push(ActorAction::StopSound { actor, clip, slot });
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
        if index == BONE_POS {
            let [bone] = arguments else {
                return Err(format!(
                    "BonePos expects one name, found {}",
                    arguments.len()
                ));
            };
            let bone = runtime_name(source, bone)?;
            return Ok(Value::Vector(bone_position(
                self.actor_bone_names.get(&actor).map(Vec::as_slice),
                self.actor_bone_positions.get(&actor).map(Vec::as_slice),
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
            if self.active_state_actor.is_none() {
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
            self.pending_latent = Some(LatentAction::MoveTo(actor));
            return Ok(Value::None);
        }
        if index == MOVE_TOWARD {
            let [target, rest @ ..] = arguments else {
                return Err("MoveToward expects an actor and optional speed".to_owned());
            };
            let Some(handle) = object_value(target) else {
                return Err(format!(
                    "MoveToward target is {}, expected object",
                    target.kind()
                ));
            };
            let speed = match rest {
                [] | [Value::None] => 1.0,
                [Value::Float(speed)] if speed.is_finite() => *speed,
                [value] => return Err(format!("MoveToward speed is {}", value.kind())),
                _ => {
                    return Err(format!(
                        "MoveToward expects at most 2 arguments, found {}",
                        arguments.len()
                    ));
                }
            };
            if self.active_state_actor.is_none() {
                return Err("MoveToward is only valid in state code".to_owned());
            }
            if handle == 0 {
                return Ok(Value::None);
            }
            let target_actor = if handle == -1 {
                actor
            } else {
                self.actor_for_handle(handle)
                    .map_err(|error| error.to_string())?
            };
            let target_object = self
                .actor_objects
                .get(&target_actor)
                .cloned()
                .ok_or_else(|| format!("MoveToward target actor {target_actor} is unregistered"))?;
            let destination = self.other_actor_vector(target_actor, "Location")?;
            let desired_speed = speed.clamp(
                0.0,
                self.actor_float(actor_class, instance, "MaxDesiredSpeed")?,
            );
            let target_class = self
                .actor_classes
                .get(&target_actor)
                .cloned()
                .ok_or_else(|| format!("MoveToward target actor {target_actor} has no class"))?;
            let target_class = self
                .resolved_object(&target_class)
                .map_err(|error| error.to_string())?;
            let duration = if self
                .class_has_name(&target_class, "Pawn")
                .map_err(|error| error.to_string())?
            {
                1.0
            } else {
                let location =
                    Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
                let movement_speed = match self.actor_byte(actor_class, instance, "Physics")? {
                    PHYS_WALKING | PHYS_FALLING => {
                        self.actor_float(actor_class, instance, "GroundSpeed")?
                    }
                    PHYS_SWIMMING => self.actor_float(actor_class, instance, "WaterSpeed")?,
                    PHYS_FLYING => self.actor_float(actor_class, instance, "AirSpeed")?,
                    _ => 0.0,
                };
                let scale = desired_speed * movement_speed;
                if scale > 0.0 {
                    1.0 + 1.3 * (Vec3::from_array(destination) - location).length() / scale
                } else {
                    0.5
                }
            };
            self.set_actor_stored(
                actor_class,
                instance,
                "MoveTarget",
                StoredValue::Object(Some(target_object)),
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
                Value::Vector(destination),
            )?;
            self.set_actor_value(actor_class, instance, "Focus", Value::Vector(destination))?;
            self.set_actor_value(actor_class, instance, "MoveTimer", Value::Float(duration))?;
            self.pending_latent = Some(LatentAction::MoveToward(actor));
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
        if index == GET_WORLD_COLLISION_BOX {
            let visual = match arguments {
                [] | [Value::None] => false,
                [Value::Bool(visual)] => *visual,
                _ => {
                    return Err(format!(
                        "GetWorldCollisionBox expects an optional bool, found {}",
                        arguments
                            .iter()
                            .map(Value::kind)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            };
            let (minimum, maximum) =
                self.world_collision_box(actor, actor_class, instance, visual)?;
            return Ok(Value::Struct(std::collections::HashMap::from([
                ("Min".to_owned(), Value::Vector(minimum.to_array())),
                ("Max".to_owned(), Value::Vector(maximum.to_array())),
            ])));
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
        if index == ROT_RAND {
            let roll = match arguments {
                [] | [Value::None] => false,
                [Value::Bool(roll)] => *roll,
                _ => {
                    return Err(format!(
                        "RotRand expects an optional bool, found {}",
                        arguments
                            .iter()
                            .map(Value::kind)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            };
            return Ok(Value::Rotator(random_rotator(&mut self.random_state, roll)));
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
        if index == PLAYER_CAN_SEE_ME {
            if !arguments.is_empty() {
                return Err(format!(
                    "PlayerCanSeeMe expects no arguments, found {}",
                    arguments.len()
                ));
            }
            return self
                .player_can_see_me(actor, actor_class, instance)
                .map(Value::Bool);
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
        if index == LINE_OF_SIGHT_TO {
            let [other] = arguments else {
                return Err(format!(
                    "LineOfSightTo expects one actor, found {}",
                    arguments.len()
                ));
            };
            let other = match other {
                Value::None | Value::Object(0) => return Ok(Value::Bool(false)),
                Value::Object(handle) => self
                    .actor_for_handle(*handle)
                    .map_err(|error| error.to_string())?,
                value => return Err(format!("LineOfSightTo actor is {}", value.kind())),
            };
            return self
                .line_of_sight_to(actor, actor_class, instance, other)
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
        if index == REMOVE_PAWN {
            if !arguments.is_empty() {
                return Err(format!(
                    "RemovePawn expects no arguments, found {}",
                    arguments.len()
                ));
            }
            self.remove_pawn(actor, actor_class, instance)?;
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
        if index == UPDATE_URL {
            let [
                Value::String(option),
                Value::String(value),
                save_default @ ..,
            ] = arguments
            else {
                return Err(format!(
                    "UpdateURL expects an option, value, and optional save-default flag, found {}",
                    arguments.len()
                ));
            };
            let save_default = match save_default {
                [] | [Value::None] => false,
                [Value::Bool(value)] => *value,
                [value] => {
                    return Err(format!(
                        "UpdateURL save-default flag is {}, expected bool",
                        value.kind()
                    ));
                }
                _ => {
                    return Err(format!(
                        "UpdateURL expects an option, value, and optional save-default flag, found {}",
                        arguments.len()
                    ));
                }
            };
            actions.push(ActorAction::UpdateUrl {
                actor,
                option: option.clone(),
                value: value.clone(),
                save_default,
            });
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
}
