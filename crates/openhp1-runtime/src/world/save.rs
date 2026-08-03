use std::{collections::BTreeMap, path::Path};

use super::state::set_event_disabled;
use super::*;

const MAGIC: [u8; 4] = *b"OHPS";
const VERSION: u16 = 1;
const MAX_SAVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 1_000_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SavedObject {
    package: String,
    export_index: u32,
}

#[derive(Clone, Debug)]
enum SavedValue {
    None,
    Byte(u8),
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
    Name(String),
    NameIndex(i32),
    Object(Option<SavedObject>),
    SelfObject,
    Vector([f32; 3]),
    Rotator([i32; 3]),
    Struct(Vec<(String, SavedValue)>),
    Array(Vec<SavedValue>),
    UnresolvedObject(String),
}

#[derive(Clone, Debug)]
struct SavedFrame {
    state: SavedObject,
    instruction_pointer: u32,
    locals: Vec<(i32, SavedValue)>,
    latent: SavedLatent,
}

#[derive(Clone, Copy, Debug)]
enum SavedLatent {
    Continue,
    Stop,
    Sleep(f32),
    FinishAnimation(usize),
    FinishInterpolation(usize),
    MoveTo(usize),
    MoveToward(usize),
    TurnTo(usize),
    TurnToward(usize),
}

#[derive(Clone, Debug)]
struct SavedTimer {
    remaining: f32,
    rate: f32,
    looping: bool,
}

#[derive(Clone, Debug)]
struct SavedActor {
    actor: usize,
    object: SavedObject,
    class: SavedObject,
    instance: Vec<(SavedObject, SavedValue)>,
    state: Option<String>,
    state_revision: u64,
    frame: Option<SavedFrame>,
    timer: Option<SavedTimer>,
    destroyed: bool,
    disabled_events: Vec<(Option<String>, String)>,
}

#[derive(Clone, Debug)]
struct SavedObjectInstance {
    object: SavedObject,
    class: SavedObject,
    instance: Vec<(SavedObject, SavedValue)>,
}

#[derive(Clone, Debug)]
struct SavedAnimationCommand {
    actor: usize,
    sequence: String,
    relative_rate: f32,
    tween_time: f32,
    looping: bool,
    tween_only: bool,
    root_motion: bool,
}

#[derive(Clone, Debug)]
struct SavedAnimationChannel {
    actor: usize,
    root_bone: usize,
    target: usize,
}

#[derive(Clone, Debug)]
struct SavedRuntime {
    map: String,
    random_state: u32,
    player_alt_fire_pressed: bool,
    actors: Vec<SavedActor>,
    object_instances: Vec<SavedObjectInstance>,
    animation_commands: Vec<SavedAnimationCommand>,
    animation_channels: Vec<SavedAnimationChannel>,
    animating: Vec<usize>,
    player_probe_touching: Vec<usize>,
}

impl ScriptRuntime {
    /// Encodes mutable runtime state for one map. The format is OpenHP1-owned
    /// and deliberately has no Unreal package bytes in it.
    pub fn save_game(&self, map: &str) -> DispatchResult<Vec<u8>> {
        if map.is_empty() || map.len() > 1024 || map.contains(['\r', '\n']) {
            return Err(save_error("map identifier is invalid"));
        }
        if self.active_state_actor.is_some() || self.pending_latent.is_some() {
            return Err(save_error("active script execution cannot be snapshotted"));
        }
        let mut actors = self
            .actor_objects
            .iter()
            .map(|(&actor, object)| self.save_actor(actor, object))
            .collect::<DispatchResult<Vec<_>>>()?;
        actors.sort_by_key(|actor| actor.actor);
        let mut object_instances = self
            .object_instances
            .iter()
            .map(|(object, (class, instance))| {
                let mut instance = instance
                    .iter()
                    .map(|(field, value)| Ok((saved_object(field)?, saved_stored_value(value)?)))
                    .collect::<DispatchResult<Vec<_>>>()?;
                instance.sort_by(|(left, _), (right, _)| left.cmp(right));
                Ok(SavedObjectInstance {
                    object: saved_object(object)?,
                    class: saved_object(class)?,
                    instance,
                })
            })
            .collect::<DispatchResult<Vec<_>>>()?;
        object_instances.sort_by(|left, right| left.object.cmp(&right.object));
        let mut animation_commands = self
            .animation_commands
            .iter()
            .map(|(&actor, command)| {
                if !command.relative_rate.is_finite() || !command.tween_time.is_finite() {
                    return Err(save_error("animation command contains a non-finite value"));
                }
                Ok(SavedAnimationCommand {
                    actor,
                    sequence: command.sequence.clone(),
                    relative_rate: command.relative_rate,
                    tween_time: command.tween_time,
                    looping: command.looping,
                    tween_only: command.tween_only,
                    root_motion: command.root_motion,
                })
            })
            .collect::<DispatchResult<Vec<_>>>()?;
        animation_commands.sort_by_key(|command| command.actor);
        let mut animation_channels = self
            .animation_channels
            .iter()
            .flat_map(|(&actor, channels)| {
                channels.iter().map(move |channel| SavedAnimationChannel {
                    actor,
                    root_bone: channel.root_bone,
                    target: channel.actor,
                })
            })
            .collect::<Vec<_>>();
        animation_channels
            .sort_by_key(|channel| (channel.actor, channel.root_bone, channel.target));
        let mut animating = self.animating.iter().copied().collect::<Vec<_>>();
        animating.sort_unstable();
        let mut player_probe_touching = self
            .player_probe_touching
            .iter()
            .copied()
            .collect::<Vec<_>>();
        player_probe_touching.sort_unstable();
        let snapshot = SavedRuntime {
            map: map.to_owned(),
            random_state: self.random_state,
            player_alt_fire_pressed: self.player_alt_fire_pressed,
            actors,
            object_instances,
            animation_commands,
            animation_channels,
            animating,
            player_probe_touching,
        };
        let mut writer = SaveWriter::default();
        write_runtime(&mut writer, &snapshot)?;
        if writer.bytes.len() > MAX_SAVE_BYTES {
            return Err(save_error("snapshot exceeds the 64 MiB limit"));
        }
        Ok(writer.bytes)
    }

    /// Restores a snapshot only into a runtime that has registered the
    /// authored map but has not yet started game or actor events.
    pub fn restore_game(&mut self, map: &str, bytes: &[u8]) -> DispatchResult<Vec<ActorAction>> {
        let snapshot = read_runtime(bytes)?;
        if !snapshot.map.eq_ignore_ascii_case(map) {
            return Err(save_error(format!(
                "snapshot is for `{}`, not `{map}`",
                snapshot.map
            )));
        }
        self.restore_snapshot(snapshot)
    }

    /// Returns the canonical relative map identifier recorded in an OpenHP1
    /// save without resolving any original package bytes.
    pub fn saved_game_map(bytes: &[u8]) -> DispatchResult<String> {
        Ok(read_runtime(bytes)?.map)
    }

    fn save_actor(&self, actor: usize, object: &ObjectId) -> DispatchResult<SavedActor> {
        let class = self
            .actor_classes
            .get(&actor)
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let mut instance = self
            .instances
            .get(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?
            .iter()
            .map(|(field, value)| Ok((saved_object(field)?, saved_stored_value(value)?)))
            .collect::<DispatchResult<Vec<_>>>()?;
        instance.sort_by(|(left, _), (right, _)| left.cmp(right));
        let frame = match self.state_frames.get(&actor) {
            Some(frame) => Some(saved_frame(self, frame)?),
            None => None,
        };
        let mut disabled_events = self
            .disabled_events
            .iter()
            .filter_map(|((event_actor, state), events)| {
                (*event_actor == actor).then_some(
                    events
                        .iter()
                        .map(|event| ((!state.is_empty()).then_some(state.clone()), event.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect::<Vec<_>>();
        disabled_events.sort();
        Ok(SavedActor {
            actor,
            object: saved_object(object)?,
            class: saved_object(class)?,
            instance,
            state: self.actor_states.get(&actor).cloned().flatten(),
            state_revision: self
                .state_revisions
                .get(&actor)
                .copied()
                .unwrap_or_default(),
            frame,
            timer: self.timers.get(&actor).map(|timer| SavedTimer {
                remaining: timer.remaining,
                rate: timer.rate,
                looping: timer.looping,
            }),
            destroyed: self.destroyed.contains(&actor),
            disabled_events,
        })
    }

    fn restore_snapshot(&mut self, snapshot: SavedRuntime) -> DispatchResult<Vec<ActorAction>> {
        if self
            .actor_objects
            .values()
            .any(|object| object.package.as_ref() == "<runtime>")
        {
            return Err(save_error("runtime already has spawned actors"));
        }
        let mut authored = BTreeMap::new();
        for (&actor, object) in &self.actor_objects {
            authored.insert(saved_object(object)?, actor);
        }
        let mut targets = BTreeMap::new();
        let mut dynamic = Vec::new();
        for saved in &snapshot.actors {
            if targets.insert(saved.actor, 0usize).is_some() {
                return Err(save_error("snapshot has duplicate actor indices"));
            }
            if saved.object.package == "<runtime>" {
                dynamic.push(saved);
            } else {
                let actor = authored
                    .remove(&saved.object)
                    .ok_or_else(|| save_error("snapshot actor is missing from the authored map"))?;
                let class = self
                    .actor_classes
                    .get(&actor)
                    .ok_or(DispatchError::UnregisteredActor { actor })?;
                if saved_object(class)? != saved.class {
                    return Err(save_error(
                        "snapshot actor class does not match the authored map",
                    ));
                }
                targets.insert(saved.actor, actor);
            }
        }
        if !authored.is_empty() {
            return Err(save_error("snapshot is missing authored actors"));
        }
        for saved in dynamic {
            let actor = saved.actor;
            if self.actor_objects.contains_key(&actor) {
                return Err(save_error(
                    "snapshot dynamic actor conflicts with an authored actor",
                ));
            }
            let class = self.resolve_saved_object(&saved.class)?;
            let object = runtime_actor_id(actor);
            self.object_handle(object.clone())?;
            self.object_actors.insert(object.clone(), actor);
            self.actor_objects.insert(actor, object);
            self.actor_classes
                .insert(actor, object_id(&class.package, class.export_index));
            targets.insert(saved.actor, actor);
        }

        self.instances.clear();
        self.object_instances.clear();
        self.actor_states.clear();
        self.state_frames.clear();
        self.state_revisions.clear();
        self.tick_functions.clear();
        self.failed_ticks.clear();
        self.failed_physics.clear();
        self.physics_ticked.clear();
        self.disabled_events.clear();
        self.destroyed.clear();
        self.timers.clear();
        self.actor_bases.clear();
        self.base_children.clear();
        self.touching.clear();
        self.collision_actors.clear();
        self.collision_actors_by_min_x.clear();

        for saved in &snapshot.actors {
            let actor = *targets
                .get(&saved.actor)
                .ok_or_else(|| save_error("snapshot actor target is missing"))?;
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            if saved_object(&class)? != saved.class {
                return Err(save_error(
                    "snapshot dynamic actor class changed during restore",
                ));
            }
            let mut instance = InstanceState::default();
            for (field, value) in &saved.instance {
                instance.insert(
                    self.resolve_saved_object_id(field)?,
                    self.restore_stored_value(value)?,
                );
            }
            self.instances.insert(actor, instance);
            self.actor_states.insert(actor, saved.state.clone());
            self.state_revisions.insert(actor, saved.state_revision);
            if saved.destroyed {
                self.destroyed.insert(actor);
            }
            if let Some(timer) = &saved.timer {
                self.timers.insert(
                    actor,
                    ActorTimer {
                        remaining: timer.remaining,
                        rate: timer.rate,
                        looping: timer.looping,
                    },
                );
            }
            for (state, event) in &saved.disabled_events {
                set_event_disabled(
                    &mut self.disabled_events,
                    actor,
                    state.as_deref(),
                    event,
                    true,
                );
            }
        }
        for saved in &snapshot.object_instances {
            let mut instance = InstanceState::default();
            for (field, value) in &saved.instance {
                instance.insert(
                    self.resolve_saved_object_id(field)?,
                    self.restore_stored_value(value)?,
                );
            }
            let object = self.resolve_saved_object_id(&saved.object)?;
            let class = self.resolve_saved_object_id(&saved.class)?;
            self.object_instances.insert(object, (class, instance));
        }
        for saved in &snapshot.actors {
            let Some(frame) = &saved.frame else {
                continue;
            };
            let actor = *targets
                .get(&saved.actor)
                .ok_or_else(|| save_error("snapshot frame actor is missing"))?;
            let mut locals = std::collections::HashMap::new();
            for (field, value) in &frame.locals {
                locals.insert(*field, self.restore_frame_value(value)?);
            }
            let state = self.resolve_saved_object_id(&frame.state)?;
            self.state_frames.insert(
                actor,
                StateFrame {
                    state,
                    frame: FrameSnapshot::from_save_parts(
                        usize::try_from(frame.instruction_pointer)
                            .map_err(|_| save_error("frame instruction pointer is invalid"))?,
                        locals,
                    ),
                    latent: restore_latent(frame.latent, &targets)?,
                },
            );
        }
        self.random_state = snapshot.random_state;
        self.player_alt_fire_pressed = snapshot.player_alt_fire_pressed;
        self.animation_commands.clear();
        for command in &snapshot.animation_commands {
            let actor = saved_actor_target(&targets, command.actor)?;
            self.animation_commands.insert(
                actor,
                AnimationCommand {
                    sequence: command.sequence.clone(),
                    relative_rate: command.relative_rate,
                    tween_time: command.tween_time,
                    looping: command.looping,
                    tween_only: command.tween_only,
                    root_motion: command.root_motion,
                },
            );
        }
        self.animation_channels.clear();
        for channel in &snapshot.animation_channels {
            self.animation_channels
                .entry(saved_actor_target(&targets, channel.actor)?)
                .or_default()
                .push(AnimationChannel {
                    root_bone: channel.root_bone,
                    actor: saved_actor_target(&targets, channel.target)?,
                });
        }
        self.animating = snapshot
            .animating
            .iter()
            .map(|&actor| saved_actor_target(&targets, actor))
            .collect::<DispatchResult<_>>()?;
        self.player_probe_touching = snapshot
            .player_probe_touching
            .iter()
            .map(|&actor| saved_actor_target(&targets, actor))
            .collect::<DispatchResult<_>>()?;
        // Platform mixer voices are transient (and have no portable seek
        // position); the newly created game host owns an empty mixer.
        self.sound_channels.clear();
        self.next_actor = self
            .actor_objects
            .keys()
            .copied()
            .max()
            .and_then(|actor| actor.checked_add(1))
            .ok_or(DispatchError::ObjectLimit)?;

        let actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        for actor in actors {
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            self.refresh_tick_actor(actor, &class)?;
            let instance = self
                .instances
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor })?;
            self.refresh_cached_collision_actor(actor, &class, &instance)
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            let base = self
                .actor_object(&class, &instance, "Base")
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            let level = self
                .actor_object(&class, &instance, "Level")
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            self.update_actor_base(actor, base, level)?;
        }

        let mut actions = Vec::new();
        for saved in &snapshot.actors {
            let actor = *targets
                .get(&saved.actor)
                .ok_or_else(|| save_error("snapshot projection actor is missing"))?;
            if saved.object.package == "<runtime>" && !saved.destroyed {
                let class_id = self
                    .actor_classes
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::UnregisteredActor { actor })?;
                let class = self.resolved_object(&class_id)?;
                let instance = self
                    .instances
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::ActiveActorContext { actor })?;
                let location = actor_vector(self, &class, &instance, "Location")?;
                let rotation = actor_rotator(self, &class, &instance, "Rotation")?;
                actions.push(ActorAction::SpawnActor {
                    actor,
                    name: format!(
                        "{}{}",
                        class
                            .package
                            .summary()
                            .name(class.package.summary().exports[class.export_index].object_name),
                        actor
                    ),
                    class_package: Arc::clone(&class.package.summary().source),
                    class_export: class.export_index,
                    class_name: class
                        .package
                        .summary()
                        .name(class.package.summary().exports[class.export_index].object_name)
                        .to_owned(),
                    location,
                    rotation,
                });
            }
            actions.extend(self.saved_scene_actions(actor)?);
            if saved.destroyed {
                actions.push(ActorAction::DestroyActor { actor });
            }
        }
        for command in &snapshot.animation_commands {
            let actor = saved_actor_target(&targets, command.actor)?;
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            let instance = self
                .instances
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor })?;
            let phase = actor_animation_phase(self, &class, &instance)?;
            actions.push(ActorAction::RestoreAnimation {
                actor,
                sequence: command.sequence.clone(),
                rate: command.relative_rate,
                tween_time: command.tween_time,
                looping: command.looping,
                tween_only: command.tween_only,
                root_motion: command.root_motion,
                phase,
            });
        }
        Ok(actions)
    }

    fn resolve_saved_object(&mut self, object: &SavedObject) -> DispatchResult<ResolvedObject> {
        if object.package == "<runtime>" {
            return Err(save_error(
                "runtime object cannot be used as a class or field",
            ));
        }
        let package = self.packages.load(&object.package)?;
        let export_index = usize::try_from(object.export_index)
            .map_err(|_| save_error("object export index is invalid"))?;
        if export_index >= package.summary().exports.len() {
            return Err(save_error("object export index is outside its package"));
        }
        Ok(ResolvedObject {
            package,
            export_index,
        })
    }

    fn resolve_saved_object_id(&mut self, object: &SavedObject) -> DispatchResult<ObjectId> {
        if object.package == "<runtime>" {
            return Ok(runtime_actor_id(
                usize::try_from(object.export_index)
                    .map_err(|_| save_error("runtime actor index is invalid"))?,
            ));
        }
        let object = self.resolve_saved_object(object)?;
        Ok(object_id(&object.package, object.export_index))
    }

    fn restore_stored_value(&mut self, value: &SavedValue) -> DispatchResult<StoredValue> {
        Ok(match value {
            SavedValue::None => StoredValue::Value(Value::None),
            SavedValue::Byte(value) => StoredValue::Value(Value::Byte(*value)),
            SavedValue::Int(value) => StoredValue::Value(Value::Int(*value)),
            SavedValue::Float(value) => StoredValue::Value(Value::Float(*value)),
            SavedValue::Bool(value) => StoredValue::Value(Value::Bool(*value)),
            SavedValue::String(value) => StoredValue::Value(Value::String(value.clone())),
            SavedValue::Name(value) => StoredValue::Name(value.clone()),
            SavedValue::Object(value) => StoredValue::Object(
                value
                    .as_ref()
                    .map(|object| self.resolve_saved_object_id(object))
                    .transpose()?,
            ),
            SavedValue::SelfObject => StoredValue::SelfObject,
            SavedValue::Vector(value) => StoredValue::Value(Value::Vector(*value)),
            SavedValue::Rotator(value) => StoredValue::Value(Value::Rotator(*value)),
            SavedValue::Struct(values) => StoredValue::Value(Value::Struct(
                values
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.restore_plain_value(value)?)))
                    .collect::<DispatchResult<_>>()?,
            )),
            SavedValue::Array(values) => StoredValue::Array(
                values
                    .iter()
                    .map(|value| self.restore_stored_value(value))
                    .collect::<DispatchResult<_>>()?,
            ),
            SavedValue::UnresolvedObject(value) => StoredValue::UnresolvedObject(value.clone()),
            SavedValue::NameIndex(_) => {
                return Err(save_error("name index cannot be used in an instance value"));
            }
        })
    }

    fn restore_plain_value(&mut self, value: &SavedValue) -> DispatchResult<Value> {
        match value {
            SavedValue::None => Ok(Value::None),
            SavedValue::Byte(value) => Ok(Value::Byte(*value)),
            SavedValue::Int(value) => Ok(Value::Int(*value)),
            SavedValue::Float(value) => Ok(Value::Float(*value)),
            SavedValue::Bool(value) => Ok(Value::Bool(*value)),
            SavedValue::String(value) => Ok(Value::String(value.clone())),
            SavedValue::Name(value) => Ok(Value::NameText(value.clone())),
            SavedValue::NameIndex(value) => Ok(Value::Name(*value)),
            SavedValue::Object(value) => Ok(Value::Object(match value {
                Some(object) => {
                    let object = self.resolve_saved_object_id(object)?;
                    self.object_handle(object)?
                }
                None => 0,
            })),
            SavedValue::SelfObject => Ok(Value::Object(-1)),
            SavedValue::Vector(value) => Ok(Value::Vector(*value)),
            SavedValue::Rotator(value) => Ok(Value::Rotator(*value)),
            SavedValue::Struct(values) => Ok(Value::Struct(
                values
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.restore_plain_value(value)?)))
                    .collect::<DispatchResult<_>>()?,
            )),
            SavedValue::Array(values) => Ok(Value::Array(
                values
                    .iter()
                    .map(|value| self.restore_plain_value(value))
                    .collect::<DispatchResult<_>>()?,
            )),
            SavedValue::UnresolvedObject(value) => Err(save_error(format!(
                "frame contains unresolved object `{value}`"
            ))),
        }
    }

    fn restore_frame_value(&mut self, value: &SavedValue) -> DispatchResult<Value> {
        self.restore_plain_value(value)
    }

    fn saved_scene_actions(&mut self, actor: usize) -> DispatchResult<Vec<ActorAction>> {
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let mut actions = Vec::new();
        for (field, value) in &instance {
            let field = self.resolved_object(field)?;
            let name = field
                .package
                .summary()
                .name(field.package.summary().exports[field.export_index].object_name);
            actions.extend(execution::object::scene_projection_actions(
                actor, name, value,
            ));
        }
        Ok(actions)
    }
}

fn saved_object(object: &ObjectId) -> DispatchResult<SavedObject> {
    let package = if object.package.as_ref() == "<runtime>" {
        "<runtime>".to_owned()
    } else {
        Path::new(object.package.as_ref())
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| save_error("object package has no package name"))?
    };
    Ok(SavedObject {
        package,
        export_index: u32::try_from(object.export_index)
            .map_err(|_| save_error("object export index exceeds save format"))?,
    })
}

fn saved_stored_value(value: &StoredValue) -> DispatchResult<SavedValue> {
    Ok(match value {
        StoredValue::Value(value) => saved_plain_value(value)?,
        StoredValue::Array(values) => SavedValue::Array(
            values
                .iter()
                .map(saved_stored_value)
                .collect::<DispatchResult<_>>()?,
        ),
        StoredValue::Name(value) => SavedValue::Name(value.clone()),
        StoredValue::Object(value) => {
            SavedValue::Object(value.as_ref().map(saved_object).transpose()?)
        }
        StoredValue::UnresolvedObject(value) => SavedValue::UnresolvedObject(value.clone()),
        StoredValue::SelfObject => SavedValue::SelfObject,
    })
}

fn saved_plain_value(value: &Value) -> DispatchResult<SavedValue> {
    Ok(match value {
        Value::None => SavedValue::None,
        Value::Byte(value) => SavedValue::Byte(*value),
        Value::Int(value) => SavedValue::Int(*value),
        Value::Float(value) if value.is_finite() => SavedValue::Float(*value),
        Value::Float(_) => return Err(save_error("snapshot contains a non-finite float")),
        Value::Bool(value) => SavedValue::Bool(*value),
        Value::String(value) => SavedValue::String(value.clone()),
        Value::Name(value) => SavedValue::NameIndex(*value),
        Value::NameText(value) => SavedValue::Name(value.clone()),
        Value::Object(0) => SavedValue::Object(None),
        Value::Object(-1) => SavedValue::SelfObject,
        Value::Object(value) => {
            return Err(save_error(format!(
                "instance contains unresolved object handle {value}"
            )));
        }
        Value::Vector(value) if value.iter().all(|value| value.is_finite()) => {
            SavedValue::Vector(*value)
        }
        Value::Vector(_) => return Err(save_error("snapshot contains a non-finite vector")),
        Value::Rotator(value) => SavedValue::Rotator(*value),
        Value::Struct(values) => {
            let mut fields = values
                .iter()
                .map(|(name, value)| Ok((name.clone(), saved_plain_value(value)?)))
                .collect::<DispatchResult<Vec<_>>>()?;
            fields.sort_by(|(left, _), (right, _)| left.cmp(right));
            SavedValue::Struct(fields)
        }
        Value::Array(values) => SavedValue::Array(
            values
                .iter()
                .map(saved_plain_value)
                .collect::<DispatchResult<_>>()?,
        ),
    })
}

fn saved_frame(runtime: &ScriptRuntime, frame: &StateFrame) -> DispatchResult<SavedFrame> {
    let Some((instruction_pointer, locals)) = frame.frame.save_parts() else {
        return Err(save_error("active state iterator cannot round-trip"));
    };
    let mut locals = locals
        .iter()
        .map(|(&field, value)| Ok((field, saved_frame_value(runtime, value)?)))
        .collect::<DispatchResult<Vec<_>>>()?;
    locals.sort_by_key(|(field, _)| *field);
    Ok(SavedFrame {
        state: saved_object(&frame.state)?,
        instruction_pointer: u32::try_from(instruction_pointer)
            .map_err(|_| save_error("frame instruction pointer exceeds save format"))?,
        locals,
        latent: saved_latent(frame.latent),
    })
}

fn saved_frame_value(runtime: &ScriptRuntime, value: &Value) -> DispatchResult<SavedValue> {
    match value {
        Value::Object(handle) if *handle > 0 => {
            let index = usize::try_from(*handle - 1)
                .ok()
                .filter(|index| *index < runtime.handle_objects.len())
                .ok_or(DispatchError::InvalidObjectHandle { handle: *handle })?;
            Ok(SavedValue::Object(Some(saved_object(
                &runtime.handle_objects[index],
            )?)))
        }
        Value::Struct(values) => {
            let mut fields = values
                .iter()
                .map(|(name, value)| Ok((name.clone(), saved_frame_value(runtime, value)?)))
                .collect::<DispatchResult<Vec<_>>>()?;
            fields.sort_by(|(left, _), (right, _)| left.cmp(right));
            Ok(SavedValue::Struct(fields))
        }
        Value::Array(values) => Ok(SavedValue::Array(
            values
                .iter()
                .map(|value| saved_frame_value(runtime, value))
                .collect::<DispatchResult<_>>()?,
        )),
        value => saved_plain_value(value),
    }
}

fn saved_latent(latent: LatentAction) -> SavedLatent {
    match latent {
        LatentAction::Continue => SavedLatent::Continue,
        LatentAction::Stop => SavedLatent::Stop,
        LatentAction::Sleep(value) => SavedLatent::Sleep(value),
        LatentAction::FinishAnimation(actor) => SavedLatent::FinishAnimation(actor),
        LatentAction::FinishInterpolation(actor) => SavedLatent::FinishInterpolation(actor),
        LatentAction::MoveTo(actor) => SavedLatent::MoveTo(actor),
        LatentAction::MoveToward(actor) => SavedLatent::MoveToward(actor),
        LatentAction::TurnTo(actor) => SavedLatent::TurnTo(actor),
        LatentAction::TurnToward(actor) => SavedLatent::TurnToward(actor),
    }
}

fn restore_latent(
    latent: SavedLatent,
    actors: &BTreeMap<usize, usize>,
) -> DispatchResult<LatentAction> {
    let actor = |actor| {
        actors
            .get(&actor)
            .copied()
            .ok_or_else(|| save_error("latent action refers to an absent actor"))
    };
    Ok(match latent {
        SavedLatent::Continue => LatentAction::Continue,
        SavedLatent::Stop => LatentAction::Stop,
        SavedLatent::Sleep(value) if value.is_finite() => LatentAction::Sleep(value),
        SavedLatent::Sleep(_) => return Err(save_error("latent sleep duration is not finite")),
        SavedLatent::FinishAnimation(value) => LatentAction::FinishAnimation(actor(value)?),
        SavedLatent::FinishInterpolation(value) => LatentAction::FinishInterpolation(actor(value)?),
        SavedLatent::MoveTo(value) => LatentAction::MoveTo(actor(value)?),
        SavedLatent::MoveToward(value) => LatentAction::MoveToward(actor(value)?),
        SavedLatent::TurnTo(value) => LatentAction::TurnTo(actor(value)?),
        SavedLatent::TurnToward(value) => LatentAction::TurnToward(actor(value)?),
    })
}

fn actor_vector(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &InstanceState,
    name: &str,
) -> DispatchResult<[f32; 3]> {
    match runtime.instance_property(class, instance, name)? {
        Some(StoredValue::Value(Value::Vector(value)))
            if value.iter().all(|value| value.is_finite()) =>
        {
            Ok(value)
        }
        Some(value) => Err(save_error(format!("actor {name} is {value:?}"))),
        None => Err(save_error(format!("actor has no {name} property"))),
    }
}

fn actor_rotator(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &InstanceState,
    name: &str,
) -> DispatchResult<[i32; 3]> {
    match runtime.instance_property(class, instance, name)? {
        Some(StoredValue::Value(Value::Rotator(value))) => Ok(value),
        Some(value) => Err(save_error(format!("actor {name} is {value:?}"))),
        None => Err(save_error(format!("actor has no {name} property"))),
    }
}

fn actor_animation_phase(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &InstanceState,
) -> DispatchResult<f32> {
    match runtime.instance_property(class, instance, "AnimFrame")? {
        Some(StoredValue::Value(Value::Float(value))) if value.is_finite() => Ok(value),
        Some(value) => Err(save_error(format!("actor AnimFrame is {value:?}"))),
        None => Err(save_error("animated actor has no AnimFrame property")),
    }
}

#[derive(Default)]
struct SaveWriter {
    bytes: Vec<u8>,
}

impl SaveWriter {
    fn bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) -> DispatchResult<()> {
        if !value.is_finite() {
            return Err(save_error("snapshot contains a non-finite float"));
        }
        self.bytes(&value.to_le_bytes());
        Ok(())
    }

    fn count(&mut self, value: usize) -> DispatchResult<()> {
        if value > MAX_ITEMS {
            return Err(save_error("snapshot collection exceeds its item limit"));
        }
        self.u32(u32::try_from(value).map_err(|_| save_error("snapshot count is too large"))?);
        Ok(())
    }

    fn string(&mut self, value: &str) -> DispatchResult<()> {
        if value.len() > MAX_SAVE_BYTES {
            return Err(save_error("snapshot string is too large"));
        }
        self.count(value.len())?;
        self.bytes(value.as_bytes());
        Ok(())
    }
}

struct SaveReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SaveReader<'a> {
    fn new(bytes: &'a [u8]) -> DispatchResult<Self> {
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(save_error("save file exceeds the 64 MiB limit"));
        }
        Ok(Self { bytes, position: 0 })
    }

    fn take(&mut self, count: usize) -> DispatchResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| save_error("save file ends unexpectedly"))?;
        let bytes = &self.bytes[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> DispatchResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> DispatchResult<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(save_error("boolean has an invalid value")),
        }
    }

    fn u16(&mut self) -> DispatchResult<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("two bytes"),
        ))
    }

    fn u32(&mut self) -> DispatchResult<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    fn i32(&mut self) -> DispatchResult<i32> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    fn u64(&mut self) -> DispatchResult<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight bytes"),
        ))
    }

    fn f32(&mut self) -> DispatchResult<f32> {
        let value = f32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"));
        value
            .is_finite()
            .then_some(value)
            .ok_or_else(|| save_error("save file contains a non-finite float"))
    }

    fn count(&mut self) -> DispatchResult<usize> {
        usize::try_from(self.u32()?)
            .ok()
            .filter(|count| *count <= MAX_ITEMS)
            .ok_or_else(|| save_error("save file collection count is invalid"))
    }

    fn string(&mut self) -> DispatchResult<String> {
        let length = self.count()?;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| save_error("save file string is not UTF-8"))
    }

    fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn write_runtime(writer: &mut SaveWriter, snapshot: &SavedRuntime) -> DispatchResult<()> {
    writer.bytes(&MAGIC);
    writer.u16(VERSION);
    writer.string(&snapshot.map)?;
    writer.u32(snapshot.random_state);
    writer.bool(snapshot.player_alt_fire_pressed);
    writer.count(snapshot.actors.len())?;
    for actor in &snapshot.actors {
        write_actor(writer, actor)?;
    }
    writer.count(snapshot.object_instances.len())?;
    for instance in &snapshot.object_instances {
        write_object_instance(writer, instance)?;
    }
    writer.count(snapshot.animation_commands.len())?;
    for command in &snapshot.animation_commands {
        write_animation_command(writer, command)?;
    }
    writer.count(snapshot.animation_channels.len())?;
    for channel in &snapshot.animation_channels {
        writer
            .u32(u32::try_from(channel.actor).map_err(|_| save_error("actor index is too large"))?);
        writer.u32(
            u32::try_from(channel.root_bone).map_err(|_| save_error("bone index is too large"))?,
        );
        writer.u32(
            u32::try_from(channel.target).map_err(|_| save_error("actor index is too large"))?,
        );
    }
    write_actor_list(writer, &snapshot.animating)?;
    write_actor_list(writer, &snapshot.player_probe_touching)?;
    Ok(())
}

fn read_runtime(bytes: &[u8]) -> DispatchResult<SavedRuntime> {
    let mut reader = SaveReader::new(bytes)?;
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(save_error(
            "save file magic does not identify OpenHP1 state",
        ));
    }
    if reader.u16()? != VERSION {
        return Err(save_error("save file version is unsupported"));
    }
    let map = reader.string()?;
    let random_state = reader.u32()?;
    let player_alt_fire_pressed = reader.bool()?;
    let actors = (0..reader.count()?)
        .map(|_| read_actor(&mut reader))
        .collect::<DispatchResult<Vec<_>>>()?;
    let object_instances = (0..reader.count()?)
        .map(|_| read_object_instance(&mut reader))
        .collect::<DispatchResult<Vec<_>>>()?;
    let animation_commands = (0..reader.count()?)
        .map(|_| read_animation_command(&mut reader))
        .collect::<DispatchResult<Vec<_>>>()?;
    let animation_channels = (0..reader.count()?)
        .map(|_| {
            Ok(SavedAnimationChannel {
                actor: read_actor_index(&mut reader)?,
                root_bone: read_actor_index(&mut reader)?,
                target: read_actor_index(&mut reader)?,
            })
        })
        .collect::<DispatchResult<Vec<_>>>()?;
    let animating = read_actor_list(&mut reader)?;
    let player_probe_touching = read_actor_list(&mut reader)?;
    if !reader.finished() {
        return Err(save_error("save file has trailing bytes"));
    }
    Ok(SavedRuntime {
        map,
        random_state,
        player_alt_fire_pressed,
        actors,
        object_instances,
        animation_commands,
        animation_channels,
        animating,
        player_probe_touching,
    })
}

fn write_object_instance(
    writer: &mut SaveWriter,
    instance: &SavedObjectInstance,
) -> DispatchResult<()> {
    write_object(writer, &instance.object)?;
    write_object(writer, &instance.class)?;
    writer.count(instance.instance.len())?;
    for (field, value) in &instance.instance {
        write_object(writer, field)?;
        write_value(writer, value, 0)?;
    }
    Ok(())
}

fn read_object_instance(reader: &mut SaveReader<'_>) -> DispatchResult<SavedObjectInstance> {
    let object = read_object(reader)?;
    let class = read_object(reader)?;
    let instance = (0..reader.count()?)
        .map(|_| Ok((read_object(reader)?, read_value(reader, 0)?)))
        .collect::<DispatchResult<Vec<_>>>()?;
    Ok(SavedObjectInstance {
        object,
        class,
        instance,
    })
}

fn write_animation_command(
    writer: &mut SaveWriter,
    command: &SavedAnimationCommand,
) -> DispatchResult<()> {
    writer.u32(u32::try_from(command.actor).map_err(|_| save_error("actor index is too large"))?);
    writer.string(&command.sequence)?;
    writer.f32(command.relative_rate)?;
    writer.f32(command.tween_time)?;
    writer.bool(command.looping);
    writer.bool(command.tween_only);
    writer.bool(command.root_motion);
    Ok(())
}

fn read_animation_command(reader: &mut SaveReader<'_>) -> DispatchResult<SavedAnimationCommand> {
    Ok(SavedAnimationCommand {
        actor: read_actor_index(reader)?,
        sequence: reader.string()?,
        relative_rate: reader.f32()?,
        tween_time: reader.f32()?,
        looping: reader.bool()?,
        tween_only: reader.bool()?,
        root_motion: reader.bool()?,
    })
}

fn write_actor_list(writer: &mut SaveWriter, actors: &[usize]) -> DispatchResult<()> {
    writer.count(actors.len())?;
    for &actor in actors {
        writer.u32(u32::try_from(actor).map_err(|_| save_error("actor index is too large"))?);
    }
    Ok(())
}

fn read_actor_list(reader: &mut SaveReader<'_>) -> DispatchResult<Vec<usize>> {
    (0..reader.count()?)
        .map(|_| read_actor_index(reader))
        .collect()
}

fn read_actor_index(reader: &mut SaveReader<'_>) -> DispatchResult<usize> {
    usize::try_from(reader.u32()?).map_err(|_| save_error("actor index is invalid"))
}

fn saved_actor_target(targets: &BTreeMap<usize, usize>, actor: usize) -> DispatchResult<usize> {
    targets
        .get(&actor)
        .copied()
        .ok_or_else(|| save_error("saved state refers to an absent actor"))
}

fn write_actor(writer: &mut SaveWriter, actor: &SavedActor) -> DispatchResult<()> {
    writer.u32(u32::try_from(actor.actor).map_err(|_| save_error("actor index is too large"))?);
    write_object(writer, &actor.object)?;
    write_object(writer, &actor.class)?;
    writer.count(actor.instance.len())?;
    for (field, value) in &actor.instance {
        write_object(writer, field)?;
        write_value(writer, value, 0)?;
    }
    write_option_string(writer, actor.state.as_deref())?;
    writer.u64(actor.state_revision);
    match &actor.frame {
        Some(frame) => {
            writer.bool(true);
            write_frame(writer, frame)?;
        }
        None => writer.bool(false),
    }
    match &actor.timer {
        Some(timer) => {
            writer.bool(true);
            writer.f32(timer.remaining)?;
            writer.f32(timer.rate)?;
            writer.bool(timer.looping);
        }
        None => writer.bool(false),
    }
    writer.bool(actor.destroyed);
    writer.count(actor.disabled_events.len())?;
    for (state, event) in &actor.disabled_events {
        write_option_string(writer, state.as_deref())?;
        writer.string(event)?;
    }
    Ok(())
}

fn read_actor(reader: &mut SaveReader<'_>) -> DispatchResult<SavedActor> {
    let actor = usize::try_from(reader.u32()?).map_err(|_| save_error("actor index is invalid"))?;
    let object = read_object(reader)?;
    let class = read_object(reader)?;
    let instance = (0..reader.count()?)
        .map(|_| Ok((read_object(reader)?, read_value(reader, 0)?)))
        .collect::<DispatchResult<Vec<_>>>()?;
    let state = read_option_string(reader)?;
    let state_revision = reader.u64()?;
    let frame = reader.bool()?.then(|| read_frame(reader)).transpose()?;
    let timer = reader
        .bool()?
        .then(|| -> DispatchResult<SavedTimer> {
            Ok(SavedTimer {
                remaining: reader.f32()?,
                rate: reader.f32()?,
                looping: reader.bool()?,
            })
        })
        .transpose()?;
    let destroyed = reader.bool()?;
    let disabled_events = (0..reader.count()?)
        .map(|_| Ok((read_option_string(reader)?, reader.string()?)))
        .collect::<DispatchResult<Vec<_>>>()?;
    Ok(SavedActor {
        actor,
        object,
        class,
        instance,
        state,
        state_revision,
        frame,
        timer,
        destroyed,
        disabled_events,
    })
}

fn write_frame(writer: &mut SaveWriter, frame: &SavedFrame) -> DispatchResult<()> {
    write_object(writer, &frame.state)?;
    writer.u32(frame.instruction_pointer);
    writer.count(frame.locals.len())?;
    for (field, value) in &frame.locals {
        writer.i32(*field);
        write_value(writer, value, 0)?;
    }
    write_latent(writer, frame.latent)
}

fn read_frame(reader: &mut SaveReader<'_>) -> DispatchResult<SavedFrame> {
    let state = read_object(reader)?;
    let instruction_pointer = reader.u32()?;
    let locals = (0..reader.count()?)
        .map(|_| Ok((reader.i32()?, read_value(reader, 0)?)))
        .collect::<DispatchResult<Vec<_>>>()?;
    let latent = read_latent(reader)?;
    Ok(SavedFrame {
        state,
        instruction_pointer,
        locals,
        latent,
    })
}

fn write_latent(writer: &mut SaveWriter, latent: SavedLatent) -> DispatchResult<()> {
    match latent {
        SavedLatent::Continue => writer.u8(0),
        SavedLatent::Stop => writer.u8(1),
        SavedLatent::Sleep(value) => {
            writer.u8(2);
            writer.f32(value)?;
        }
        SavedLatent::FinishAnimation(actor) => write_latent_actor(writer, 3, actor)?,
        SavedLatent::FinishInterpolation(actor) => write_latent_actor(writer, 4, actor)?,
        SavedLatent::MoveTo(actor) => write_latent_actor(writer, 5, actor)?,
        SavedLatent::MoveToward(actor) => write_latent_actor(writer, 6, actor)?,
        SavedLatent::TurnTo(actor) => write_latent_actor(writer, 7, actor)?,
        SavedLatent::TurnToward(actor) => write_latent_actor(writer, 8, actor)?,
    }
    Ok(())
}

fn write_latent_actor(writer: &mut SaveWriter, tag: u8, actor: usize) -> DispatchResult<()> {
    writer.u8(tag);
    writer.u32(u32::try_from(actor).map_err(|_| save_error("latent actor is too large"))?);
    Ok(())
}

fn read_latent(reader: &mut SaveReader<'_>) -> DispatchResult<SavedLatent> {
    let actor = |reader: &mut SaveReader<'_>| {
        usize::try_from(reader.u32()?).map_err(|_| save_error("latent actor is invalid"))
    };
    match reader.u8()? {
        0 => Ok(SavedLatent::Continue),
        1 => Ok(SavedLatent::Stop),
        2 => Ok(SavedLatent::Sleep(reader.f32()?)),
        3 => Ok(SavedLatent::FinishAnimation(actor(reader)?)),
        4 => Ok(SavedLatent::FinishInterpolation(actor(reader)?)),
        5 => Ok(SavedLatent::MoveTo(actor(reader)?)),
        6 => Ok(SavedLatent::MoveToward(actor(reader)?)),
        7 => Ok(SavedLatent::TurnTo(actor(reader)?)),
        8 => Ok(SavedLatent::TurnToward(actor(reader)?)),
        _ => Err(save_error("latent action has an invalid tag")),
    }
}

fn write_object(writer: &mut SaveWriter, object: &SavedObject) -> DispatchResult<()> {
    writer.string(&object.package)?;
    writer.u32(object.export_index);
    Ok(())
}

fn read_object(reader: &mut SaveReader<'_>) -> DispatchResult<SavedObject> {
    let package = reader.string()?.to_ascii_lowercase();
    if package.is_empty() || package.contains(['/', '\\']) || package == "." || package == ".." {
        return Err(save_error("object package name is invalid"));
    }
    Ok(SavedObject {
        package,
        export_index: reader.u32()?,
    })
}

fn write_option_string(writer: &mut SaveWriter, value: Option<&str>) -> DispatchResult<()> {
    writer.bool(value.is_some());
    if let Some(value) = value {
        writer.string(value)?;
    }
    Ok(())
}

fn read_option_string(reader: &mut SaveReader<'_>) -> DispatchResult<Option<String>> {
    reader.bool()?.then(|| reader.string()).transpose()
}

fn write_value(writer: &mut SaveWriter, value: &SavedValue, depth: usize) -> DispatchResult<()> {
    if depth >= MAX_DEPTH {
        return Err(save_error("snapshot value nesting exceeds its limit"));
    }
    match value {
        SavedValue::None => writer.u8(0),
        SavedValue::Byte(value) => {
            writer.u8(1);
            writer.u8(*value);
        }
        SavedValue::Int(value) => {
            writer.u8(2);
            writer.i32(*value);
        }
        SavedValue::Float(value) => {
            writer.u8(3);
            writer.f32(*value)?;
        }
        SavedValue::Bool(value) => {
            writer.u8(4);
            writer.bool(*value);
        }
        SavedValue::String(value) => {
            writer.u8(5);
            writer.string(value)?;
        }
        SavedValue::Name(value) => {
            writer.u8(6);
            writer.string(value)?;
        }
        SavedValue::NameIndex(value) => {
            writer.u8(7);
            writer.i32(*value);
        }
        SavedValue::Object(value) => {
            writer.u8(8);
            writer.bool(value.is_some());
            if let Some(value) = value {
                write_object(writer, value)?;
            }
        }
        SavedValue::SelfObject => writer.u8(9),
        SavedValue::Vector(value) => {
            writer.u8(10);
            for value in value {
                writer.f32(*value)?;
            }
        }
        SavedValue::Rotator(value) => {
            writer.u8(11);
            for value in value {
                writer.i32(*value);
            }
        }
        SavedValue::Struct(values) => {
            writer.u8(12);
            writer.count(values.len())?;
            for (name, value) in values {
                writer.string(name)?;
                write_value(writer, value, depth + 1)?;
            }
        }
        SavedValue::Array(values) => {
            writer.u8(13);
            writer.count(values.len())?;
            for value in values {
                write_value(writer, value, depth + 1)?;
            }
        }
        SavedValue::UnresolvedObject(value) => {
            writer.u8(14);
            writer.string(value)?;
        }
    }
    Ok(())
}

fn read_value(reader: &mut SaveReader<'_>, depth: usize) -> DispatchResult<SavedValue> {
    if depth >= MAX_DEPTH {
        return Err(save_error("save value nesting exceeds its limit"));
    }
    Ok(match reader.u8()? {
        0 => SavedValue::None,
        1 => SavedValue::Byte(reader.u8()?),
        2 => SavedValue::Int(reader.i32()?),
        3 => SavedValue::Float(reader.f32()?),
        4 => SavedValue::Bool(reader.bool()?),
        5 => SavedValue::String(reader.string()?),
        6 => SavedValue::Name(reader.string()?),
        7 => SavedValue::NameIndex(reader.i32()?),
        8 => SavedValue::Object(reader.bool()?.then(|| read_object(reader)).transpose()?),
        9 => SavedValue::SelfObject,
        10 => SavedValue::Vector([reader.f32()?, reader.f32()?, reader.f32()?]),
        11 => SavedValue::Rotator([reader.i32()?, reader.i32()?, reader.i32()?]),
        12 => SavedValue::Struct(
            (0..reader.count()?)
                .map(|_| Ok((reader.string()?, read_value(reader, depth + 1)?)))
                .collect::<DispatchResult<_>>()?,
        ),
        13 => SavedValue::Array(
            (0..reader.count()?)
                .map(|_| read_value(reader, depth + 1))
                .collect::<DispatchResult<_>>()?,
        ),
        14 => SavedValue::UnresolvedObject(reader.string()?),
        _ => return Err(save_error("save value has an invalid tag")),
    })
}

fn save_error(message: impl Into<String>) -> DispatchError {
    DispatchError::SaveState {
        message: message.into(),
    }
}
