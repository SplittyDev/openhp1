use super::*;

impl ScriptRuntime {
    pub(in crate::world) fn add_pawn(
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

    pub(in crate::world) fn pick_target(
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

    pub(in crate::world) fn set_actor_owner(
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

    pub(in crate::world) fn destroy_actor(
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
        self.call_actor_event(
            actor,
            actor_class,
            instance,
            "Destroyed",
            Vec::new(),
            actions,
        )?;
        self.timers.remove(&actor);
        self.animating.remove(&actor);
        actions.push(ActorAction::DestroyActor { actor });
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::world) fn spawn_actor(
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
    pub(in crate::world) fn start_animation(
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

    pub(in crate::world) fn synchronize_animation_command(
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

    pub(in crate::world) fn resolve_class_value(
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
