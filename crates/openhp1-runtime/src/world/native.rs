use glam::Vec3;

use super::physics::{PHYS_FALLING, PHYS_FLYING, PHYS_SWIMMING, PHYS_WALKING};
use super::state::{event_disabled, set_event_disabled};
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
    next_navigation_step, noise_loudness, runtime_name, trace_texture_arguments,
};

use gesture::{gesture_native, gesture_points};

impl ScriptRuntime {
    pub(super) fn begin_latent_action(&mut self, actor: usize, action: LatentAction) {
        if self.active_state_actor == Some(actor) {
            self.pending_latent = Some(action);
        } else if let Some(frame) = self.state_frames.get_mut(&actor) {
            frame.latent = action;
        }
    }

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
        if index == FIND_STAIR_ROTATION {
            let [Value::Float(delta_time)] = arguments else {
                return Err(format!(
                    "FindStairRotation expects one float, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            return self
                .find_stair_rotation(actor, actor_class, instance, *delta_time)
                .map(Value::Int);
        }
        if index == DESTROY {
            return self
                .destroy_actor(actor, actor_class, instance, actions)
                .map(Value::Bool);
        }
        if index == AUTONOMOUS_PHYSICS {
            let [Value::Float(delta_time)] = arguments else {
                return Err(format!(
                    "AutonomousPhysics expects one float, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            self.tick_actor_physics(actor, actor_class, instance, *delta_time, actions)?;
            self.failed_physics.remove(&actor);
            self.physics_ticked.insert(actor);
            return Ok(Value::None);
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
            let state_actor = self.active_state_actor.unwrap_or(actor);
            let latent = self
                .state_frames
                .get(&state_actor)
                .map(|frame| frame.latent);
            self.finish_latent_movement(actor, actor_class, instance, latent)?;
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
            let sequence_metadata_pending = !self.animation_sequences.contains_key(&actor);
            let configured = self.start_animation(
                actor,
                actor_class,
                instance,
                name.clone(),
                rate,
                tween_time,
                false,
                false,
                root_motion,
            )?;
            actions.push(ActorAction::PlayAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
                root_motion,
            });
            if configured || sequence_metadata_pending {
                self.animating.insert(actor);
            }
            return Ok(Value::None);
        }
        if index == LOOP_ANIM
            && let [name, rest @ ..] = arguments
        {
            let name = runtime_name(source, name)?;
            let (rate, tween_time) = animation_parameters("LoopAnim", rest)?;
            let root_motion = animation_root_motion("LoopAnim", source, rest, 4)?;
            let sequence_metadata_pending = !self.animation_sequences.contains_key(&actor);
            let configured = self.start_animation(
                actor,
                actor_class,
                instance,
                name.clone(),
                rate,
                tween_time,
                true,
                false,
                root_motion,
            )?;
            actions.push(ActorAction::LoopAnimation {
                actor,
                sequence: name,
                rate,
                tween_time,
                root_motion,
            });
            if configured || sequence_metadata_pending {
                self.animating.insert(actor);
            }
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
                false,
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
            let channel = match arguments.first() {
                None | Some(Value::None) => actor,
                Some(root) => {
                    let root = runtime_name(source, root)?;
                    if root.eq_ignore_ascii_case("None") {
                        actor
                    } else {
                        let Some(root_bone) = self.actor_bone_names.get(&actor).and_then(|bones| {
                            bones
                                .iter()
                                .position(|bone| bone.eq_ignore_ascii_case(&root))
                        }) else {
                            return Ok(Value::Bool(false));
                        };
                        let Some(channel) =
                            self.animation_channels.get(&actor).and_then(|channels| {
                                channels
                                    .iter()
                                    .find(|channel| channel.root_bone == root_bone)
                            })
                        else {
                            return Ok(Value::Bool(false));
                        };
                        channel.actor
                    }
                }
            };
            return Ok(Value::Bool(self.animating.contains(&channel)));
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
            self.tick_turn_to(actor_class, instance)?;
            self.begin_latent_action(actor, LatentAction::TurnTo(actor));
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
            self.tick_turn_to(actor_class, instance)?;
            self.begin_latent_action(actor, LatentAction::TurnToward(actor));
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
            if self.actor_bool(actor_class, instance, "bAnimLoop")? {
                self.set_actor_value(actor_class, instance, "bAnimLoop", Value::Bool(false))?;
                self.set_actor_value(actor_class, instance, "bAnimFinished", Value::Bool(false))?;
            }
            if let Some(command) = self.animation_commands.get_mut(&actor) {
                command.looping = false;
            }
            let sequence =
                match self.required_actor_property(actor_class, instance, "AnimSequence")? {
                    StoredValue::Name(sequence) => sequence,
                    value => return Err(format!("actor property AnimSequence is {value:?}")),
                };
            let animating = !sequence.eq_ignore_ascii_case("None")
                && (self.actor_signed_float(actor_class, instance, "AnimRate")? != 0.0
                    || self.actor_float(actor_class, instance, "TweenRate")? != 0.0)
                && self.actor_signed_float(actor_class, instance, "AnimFrame")?
                    < self.actor_float(actor_class, instance, "AnimLast")?;
            if animating {
                self.pending_latent = Some(LatentAction::FinishAnimation(actor));
            }
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
        if index == MODIFY_SOUND {
            return self
                .modify_sound(actor, arguments, actions)
                .map(Value::Bool);
        }
        if index == STOP_SOUND {
            self.stop_sound(actor, arguments, actions)?;
            return Ok(Value::None);
        }
        if index == MAKE_NOISE {
            self.make_noise(
                actor,
                actor_class,
                instance,
                noise_loudness(arguments)?,
                actions,
            )?;
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
            let root_bone = runtime_name(source, root_bone)?;
            let channel = self.spawn_actor(
                actor,
                actor_class,
                source,
                &[class.clone(), Value::Object(-1)],
                instance,
                actions,
            )?;
            if let Value::Object(handle) = &channel
                && *handle != 0
                && let Some(root_bone) = self.actor_bone_names.get(&actor).and_then(|bones| {
                    bones
                        .iter()
                        .position(|bone| bone.eq_ignore_ascii_case(&root_bone))
                })
            {
                let channel_actor = self
                    .actor_for_handle(*handle)
                    .map_err(|error| error.to_string())?;
                self.animation_channels
                    .entry(actor)
                    .or_default()
                    .push(AnimationChannel {
                        root_bone,
                        actor: channel_actor,
                    });
                if let Some(sequences) = self.animation_sequences.get(&actor).cloned() {
                    self.animation_sequences.insert(channel_actor, sequences);
                    if self.animation_commands.contains_key(&channel_actor) {
                        self.synchronize_animation_command(channel_actor)?;
                    }
                }
                return Ok(Value::Object(*handle));
            }
            return Ok(channel);
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
            self.set_actor_physics(actor, actor_class, instance, *physics, actions)?;
            if matches!(*physics, physics::PHYS_NONE | physics::PHYS_ROTATING) {
                for property in ["Velocity", "Acceleration"] {
                    self.set_actor_value(actor_class, instance, property, Value::Vector([0.0; 3]))?;
                }
            }
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
            return self
                .set_actor_location_placing(
                    actor,
                    actor_class,
                    instance,
                    Vec3::from_array(*location),
                    actions,
                )
                .map(Value::Bool);
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
            return self
                .try_move_actor_rotated(actor, actor_class, *rotation, instance, actions)
                .map(|hit| Value::Bool(hit.fraction == 1.0));
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
        if index == ACTOR_REACHABLE {
            let [other] = arguments else {
                return Err(format!(
                    "actorReachable expects one actor, found {}",
                    arguments.len()
                ));
            };
            let other = match other {
                Value::None | Value::Object(0) => return Ok(Value::Bool(false)),
                Value::Object(handle) => self
                    .actor_for_handle(*handle)
                    .map_err(|error| error.to_string())?,
                value => return Err(format!("actorReachable actor is {}", value.kind())),
            };
            return self
                .actor_reachable(actor, actor_class, instance, other)
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
            self.save_config(actor_class, instance)
                .map_err(|error| error.to_string())?;
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
        if index == FIND_PATH_TO {
            let [Value::Vector(destination), options @ ..] = arguments else {
                return Err(format!(
                    "FindPathTo expects a destination vector and up to two optional bools, found {}",
                    arguments.len()
                ));
            };
            if options.len() > 2 || !destination.iter().all(|value| value.is_finite()) {
                return Err("FindPathTo arguments are invalid".to_owned());
            }
            let optional_flag = |index, default| match options.get(index) {
                Some(Value::Bool(value)) => Ok(*value),
                Some(Value::None) | None => Ok(default),
                Some(value) => Err(format!("FindPathTo optional flag is {}", value.kind())),
            };
            let _single_path = optional_flag(0, false)?;
            let clear_paths = optional_flag(1, true)?;
            let destination = Vec3::from_array(*destination);
            let location =
                Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
            let base_eye_height = self.actor_float(actor_class, instance, "BaseEyeHeight")?;
            let radius = self.actor_float(actor_class, instance, "CollisionRadius")?;
            let height = self.actor_float(actor_class, instance, "CollisionHeight")?;
            let is_player = self
                .optional_actor_bool(actor_class, instance, "bIsPlayer")?
                .unwrap_or(
                    self.class_has_name(actor_class, "PlayerPawn")
                        .map_err(|error| error.to_string())?,
                );
            if clear_paths {
                self.clear_navigation_paths(actor, actions, depth)?;
            }
            let mut target_candidates = self.navigation_points(is_player)?;
            target_candidates
                .retain(|(_, _, point)| point.distance_squared(destination) <= 500.0_f32.powi(2));
            target_candidates.sort_by(|(_, _, left), (_, _, right)| {
                left.distance_squared(destination)
                    .total_cmp(&right.distance_squared(destination))
            });
            let target = target_candidates
                .into_iter()
                .take(4)
                .find(|(_, _, point)| {
                    matches!(
                        self.fast_trace_native(
                            actor_class,
                            &[
                                Value::Vector(destination.to_array()),
                                Value::Vector((*point + Vec3::Z * base_eye_height).to_array()),
                            ],
                            instance,
                        ),
                        Ok(Value::Bool(true))
                    )
                })
                .map(|(_, point, _)| point);
            let Some(target) = target else {
                self.set_route_cache(actor_class, instance, &[])?;
                return Ok(Value::Object(0));
            };
            let endpoints = self.mark_reachable_navigation_endpoints(
                actor,
                actor_class,
                instance,
                location,
                is_player,
            )?;
            let Some(route) = self.find_path_to_endpoint(
                &endpoints,
                &target,
                radius as i32,
                height as i32,
                is_player,
                location,
            )?
            else {
                self.set_route_cache(actor_class, instance, &[])?;
                return Ok(Value::Object(0));
            };
            let Some(next) =
                self.path_special_handling(actor, actor_class, instance, &route, actions, depth)?
            else {
                return Ok(Value::Object(0));
            };
            return self
                .object_handle(next)
                .map(Value::Object)
                .map_err(|error| error.to_string());
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
            let Some(next) = next_navigation_step(&self.reach_specs, &start, &target) else {
                return Ok(Value::Object(0));
            };
            return self
                .object_handle(next)
                .map(Value::Object)
                .map_err(|error| error.to_string());
        }
        scalar_native(index, arguments)
    }

    fn navigation_points(
        &mut self,
        include_player_only: bool,
    ) -> std::result::Result<Vec<(usize, ObjectId, Vec3)>, String> {
        let mut actors = self
            .actor_classes
            .iter()
            .map(|(&actor, class)| (actor, class.clone()))
            .collect::<Vec<_>>();
        actors.sort_unstable_by_key(|(actor, _)| *actor);
        let mut points = Vec::new();
        for (actor, class) in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            if !self
                .class_has_name(&class, "NavigationPoint")
                .map_err(|error| error.to_string())?
            {
                continue;
            }
            let Some(point) = self.actor_objects.get(&actor).cloned() else {
                continue;
            };
            let Some(instance) = self.instances.get(&actor).cloned() else {
                continue;
            };
            if self
                .optional_actor_bool(&class, &instance, "bPlayerOnly")?
                .unwrap_or(false)
                && !include_player_only
            {
                continue;
            }
            let point_location =
                Vec3::from_array(self.actor_vector(&class, &instance, "Location")?);
            points.push((actor, point, point_location));
        }
        Ok(points)
    }

    fn clear_navigation_paths(
        &mut self,
        pawn: usize,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> std::result::Result<(), String> {
        let points = self.navigation_points(true)?;
        let pawn = self
            .actor_objects
            .get(&pawn)
            .cloned()
            .ok_or_else(|| format!("FindPathTo pawn {pawn} has no object identity"))?;
        let pawn = Value::Object(
            self.object_handle(pawn)
                .map_err(|error| error.to_string())?,
        );
        for (actor, _, _) in points {
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or_else(|| format!("navigation point {actor} has no class"))?;
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            let mut instance = self
                .instances
                .remove(&actor)
                .ok_or_else(|| format!("navigation point {actor} instance is active"))?;
            let result = (|| {
                self.set_optional_actor_value(
                    &class,
                    &mut instance,
                    "bEndPoint",
                    Value::Bool(false),
                )?;
                let cost = if self
                    .optional_actor_bool(&class, &instance, "bSpecialCost")?
                    .unwrap_or(false)
                {
                    match self.call_navigation_event(
                        actor,
                        &class,
                        &mut instance,
                        "SpecialCost",
                        &[pawn.clone()],
                        actions,
                        depth,
                    )? {
                        Some(Value::Int(cost)) => cost,
                        Some(value) => {
                            return Err(format!("SpecialCost returned {}", value.kind()));
                        }
                        None => 0,
                    }
                } else {
                    match self
                        .instance_property(&class, &instance, "ExtraCost")
                        .map_err(|error| error.to_string())?
                    {
                        Some(StoredValue::Value(Value::Int(cost))) => cost,
                        Some(value) => return Err(format!("ExtraCost is {value:?}")),
                        None => 0,
                    }
                };
                self.set_optional_actor_value(&class, &mut instance, "cost", Value::Int(cost))
            })();
            self.instances.insert(actor, instance);
            result?;
        }
        Ok(())
    }

    fn mark_reachable_navigation_endpoints(
        &mut self,
        pawn: usize,
        pawn_class: &ResolvedObject,
        pawn_instance: &InstanceState,
        location: Vec3,
        is_player: bool,
    ) -> std::result::Result<Vec<ObjectId>, String> {
        let points = self.navigation_points(true)?;
        for (actor, _, _) in &points {
            self.set_navigation_endpoint(*actor, false)?;
        }
        let mut endpoints = Vec::new();
        for (point_actor, point, point_location) in points {
            if endpoints.len() == 8 {
                break;
            }
            if !self.navigation_point_allowed(&point, is_player)? {
                continue;
            }
            if point_location.distance_squared(location) > 1_000_000.0 {
                continue;
            }
            if self.actor_reachable(pawn, pawn_class, pawn_instance, point_actor)? {
                self.set_navigation_endpoint(point_actor, true)?;
                endpoints.push(point);
            }
        }
        Ok(endpoints)
    }

    fn find_path_to_endpoint(
        &mut self,
        endpoints: &[ObjectId],
        target: &ObjectId,
        radius: i32,
        height: i32,
        is_player: bool,
        location: Vec3,
    ) -> std::result::Result<Option<Vec<ObjectId>>, String> {
        if !self.navigation_point_allowed(target, is_player)? {
            return Ok(None);
        }
        if endpoints.contains(target) {
            return Ok(Some(vec![target.clone()]));
        }
        let specs = self.reach_specs.clone();
        let mut distances = HashMap::default();
        let mut next = HashMap::default();
        let mut pending = vec![target.clone()];
        distances.insert(target.clone(), 0_i32);
        for _ in 0..1000 {
            let Some((index, _)) = pending
                .iter()
                .enumerate()
                .min_by_key(|(_, point)| distances.get(*point).copied().unwrap_or(i32::MAX))
            else {
                break;
            };
            let current = pending.swap_remove(index);
            if endpoints.contains(&current) {
                let mut route = vec![current.clone()];
                while route.last().is_some_and(|point| point != target) {
                    route.push(
                        next.get(route.last().unwrap())
                            .cloned()
                            .ok_or_else(|| "FindPathTo route ended before its target".to_owned())?,
                    );
                }
                let mut route_without_touched_points = Vec::new();
                for point in route {
                    let point_location = self.navigation_point_location(&point)?;
                    let delta = point_location - location;
                    if delta.length_squared() < (radius as f32).powi(2)
                        && delta.z.abs() < height as f32
                    {
                        route_without_touched_points.clear();
                    } else {
                        route_without_touched_points.push(point);
                    }
                }
                return Ok((!route_without_touched_points.is_empty())
                    .then_some(route_without_touched_points));
            }
            let distance = distances[&current];
            for spec in specs.iter().filter(|spec| {
                spec.end == current
                    && !spec.pruned
                    && spec.collision_radius >= radius
                    && spec.collision_height >= height
            }) {
                if !self.navigation_point_allowed(&spec.start, is_player)? {
                    continue;
                }
                let candidate = distance.saturating_add(spec.distance.max(0));
                if distances
                    .get(&spec.start)
                    .is_some_and(|known| *known <= candidate)
                {
                    continue;
                }
                distances.insert(spec.start.clone(), candidate);
                next.insert(spec.start.clone(), current.clone());
                if !pending.contains(&spec.start) {
                    pending.push(spec.start.clone());
                }
            }
        }
        Ok(None)
    }

    fn path_special_handling(
        &mut self,
        pawn: usize,
        pawn_class: &ResolvedObject,
        pawn_instance: &mut InstanceState,
        route: &[ObjectId],
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> std::result::Result<Option<ObjectId>, String> {
        self.set_route_cache(pawn_class, pawn_instance, route)?;
        let Some(old_best_point) = route.first().cloned() else {
            return Ok(None);
        };
        let Some(point_actor) = self.object_actors.get(&old_best_point).copied() else {
            return Ok(None);
        };
        let point_class = self
            .actor_classes
            .get(&point_actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {point_actor} has no class"))?;
        let point_class = self
            .resolved_object(&point_class)
            .map_err(|error| error.to_string())?;
        let mut point_instance = self
            .instances
            .remove(&point_actor)
            .ok_or_else(|| format!("navigation point {point_actor} instance is active"))?;
        let pawn_object = self
            .actor_objects
            .get(&pawn)
            .cloned()
            .ok_or_else(|| format!("FindPathTo pawn {pawn} has no object identity"))?;
        let pawn_handle = self
            .object_handle(pawn_object)
            .map_err(|error| error.to_string())?;
        let special = self.call_navigation_event(
            point_actor,
            &point_class,
            &mut point_instance,
            "SpecialHandling",
            &[Value::Object(pawn_handle)],
            actions,
            depth,
        );
        self.instances.insert(point_actor, point_instance);
        let Some(special) = special? else {
            if self.actor_object(pawn_class, pawn_instance, "SpecialGoal")?
                == Some(old_best_point.clone())
            {
                self.set_actor_stored(
                    pawn_class,
                    pawn_instance,
                    "SpecialGoal",
                    StoredValue::Object(None),
                )?;
            }
            return Ok(Some(old_best_point));
        };
        let best_point = match special {
            Value::Object(handle) if handle != 0 => self
                .object_for_handle(handle)
                .map_err(|error| error.to_string())
                .ok()
                .filter(|point| self.object_actors.contains_key(point)),
            Value::None | Value::Object(_) => None,
            _ => None,
        };
        let best_point = self
            .actor_bool(pawn_class, pawn_instance, "bCanDoSpecial")?
            .then_some(best_point)
            .flatten();
        self.set_actor_stored(
            pawn_class,
            pawn_instance,
            "SpecialGoal",
            StoredValue::Object(best_point.clone()),
        )?;
        let Some(best_point) = best_point else {
            return Ok(None);
        };
        if best_point != old_best_point {
            let Some(best_actor) = self.object_actors.get(&best_point).copied() else {
                return Ok(None);
            };
            if !self.actor_reachable(pawn, pawn_class, pawn_instance, best_actor)? {
                self.set_route_cache(pawn_class, pawn_instance, &[])?;
                return Ok(None);
            }
        }
        Ok(Some(best_point))
    }

    fn call_navigation_event(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        event: &str,
        arguments: &[Value],
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> std::result::Result<Option<Value>, String> {
        if event_disabled(
            &self.disabled_events,
            actor,
            self.actor_states
                .get(&actor)
                .and_then(|state| state.as_deref()),
            event,
        ) || self
            .state_ignores_event(actor, class, event)
            .map_err(|error| error.to_string())?
        {
            return Ok(None);
        }
        let Some(function) = self
            .find_actor_function(
                actor,
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                event,
                0,
            )
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let state_revision = self.state_revision(actor);
        let value = self
            .execute_function(
                actor,
                class,
                &function,
                arguments,
                instance,
                actions,
                depth + 1,
            )
            .map_err(|error| error.to_string())?;
        if self.state_revision(actor) != state_revision {
            self.execute_ready_state(actor, class, instance, actions)
                .map_err(|error| error.to_string())?;
        }
        Ok(Some(value))
    }

    fn set_route_cache(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        route: &[ObjectId],
    ) -> std::result::Result<(), String> {
        let Some(field) = self
            .find_property(class, "RouteCache", 0)
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        let Some(StoredValue::Array(cache)) = self
            .instance_property(class, instance, "RouteCache")
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        let cache = cache
            .into_iter()
            .enumerate()
            .map(|(index, _)| StoredValue::Object(route.get(index).cloned()))
            .collect();
        instance.insert(field, StoredValue::Array(cache));
        Ok(())
    }

    fn set_navigation_endpoint(
        &mut self,
        actor: usize,
        endpoint: bool,
    ) -> std::result::Result<(), String> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {actor} has no class"))?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| format!("navigation point {actor} instance is active"))?;
        let result = self.set_optional_actor_value(
            &class,
            &mut instance,
            "bEndPoint",
            Value::Bool(endpoint),
        );
        self.instances.insert(actor, instance);
        result
    }

    fn navigation_point_allowed(
        &mut self,
        point: &ObjectId,
        is_player: bool,
    ) -> std::result::Result<bool, String> {
        let Some(actor) = self.object_actors.get(point).copied() else {
            return Ok(false);
        };
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {actor} has no class"))?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        if !self
            .class_has_name(&class, "NavigationPoint")
            .map_err(|error| error.to_string())?
        {
            return Ok(false);
        }
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {actor} instance is active"))?;
        Ok(!self
            .optional_actor_bool(&class, &instance, "bPlayerOnly")?
            .unwrap_or(false)
            || is_player)
    }

    fn navigation_point_location(&mut self, point: &ObjectId) -> std::result::Result<Vec3, String> {
        let Some(actor) = self.object_actors.get(point).copied() else {
            return Err("FindPathTo route has an unregistered navigation point".to_owned());
        };
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {actor} has no class"))?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("navigation point {actor} instance is active"))?;
        Ok(Vec3::from_array(
            self.actor_vector(&class, &instance, "Location")?,
        ))
    }

    fn set_optional_actor_value(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        name: &str,
        value: Value,
    ) -> std::result::Result<(), String> {
        let Some(field) = self
            .find_property(class, name, 0)
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        instance.insert(field, StoredValue::Value(value));
        Ok(())
    }

    fn find_stair_rotation(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        delta_time: f32,
    ) -> std::result::Result<i32, String> {
        if !delta_time.is_finite() {
            return Err("FindStairRotation delta time is not finite".to_owned());
        }
        let mut rotation = self.actor_rotator(class, instance, "Rotation")?;
        if rotation[0] > 0x8000 {
            rotation[0] = (rotation[0] & 0xffff) - 0x10000;
            self.set_actor_value(class, instance, "Rotation", Value::Rotator(rotation))?;
        }
        let current = rotation[0];
        if delta_time > 0.33 {
            return Ok(current);
        }
        if self.collision.is_none() {
            return Ok(interpolate_stair_rotation(current, 0, delta_time));
        }

        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        let radius = self.actor_float(class, instance, "CollisionRadius")?;
        let height = self.actor_float(class, instance, "CollisionHeight")?;
        let eye_height = self.actor_float(class, instance, "EyeHeight")?;
        let distance = height + eye_height;
        if distance <= f32::EPSILON {
            return Ok(interpolate_stair_rotation(current, 0, delta_time));
        }
        let rotation = [0, rotation[1], rotation[2]];
        let forward = Vec3::from_array(crate::rotator_axes(rotation)[0]);
        let floor = self.floor_height_at(actor, instance, location, distance, radius)?;
        let ahead = self.floor_height_at(
            actor,
            instance,
            location + forward * distance,
            distance,
            radius,
        )?;
        let target = match (floor, ahead) {
            (Some(floor), Some(ahead)) if ahead > floor + 6.0 => 5_400,
            (Some(floor), Some(ahead)) if ahead < floor - 6.0 => -5_000,
            _ => 0,
        };
        Ok(interpolate_stair_rotation(current, target, delta_time))
    }
}

fn interpolate_stair_rotation(current: i32, target: i32, delta_time: f32) -> i32 {
    let difference = current.abs_diff(target);
    if difference == 0 {
        return target;
    }
    let rate = if difference < 1_000 {
        8_000.0 / difference as f32
    } else {
        8.0
    };
    let alpha = (rate * delta_time).min(1.0);
    (current as f32 * (1.0 - alpha) + target as f32 * alpha).round_ties_even() as i32
}
