use super::*;

impl ScriptRuntime {
    pub fn dispatch_event(
        &mut self,
        actor: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
        event: &str,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.dispatch_event_with_arguments(actor, class_package, class_export, event, &[])
    }

    pub fn tick(&mut self, delta_time: f32) -> DispatchResult<Vec<ActorAction>> {
        if !delta_time.is_finite() || delta_time < 0.0 {
            return Err(DispatchError::InvalidDeltaTime { value: delta_time });
        }
        self.physics_ticked.clear();
        self.tick_sound_channels(delta_time);
        self.tick_level_time(delta_time)?;
        let mut actions = Vec::new();
        self.tick_animation_properties(delta_time, &mut actions);
        self.collision_actors.clear();
        self.collision_actors_by_min_x.clear();
        let mut actors = self
            .tick_functions
            .iter()
            .filter(|(actor, _)| !self.failed_ticks.contains(actor))
            .map(|(&actor, function)| {
                (
                    actor,
                    ResolvedObject {
                        package: Arc::clone(&function.package),
                        export_index: function.export_index,
                    },
                )
            })
            .collect::<Vec<_>>();
        actors.sort_unstable_by_key(|(actor, _)| *actor);
        let player = self.player_actor;
        let mut player_ticked = false;
        for (actor, function) in actors {
            if !player_ticked && player.is_some_and(|player| player < actor) {
                self.tick_player_events(delta_time, &mut actions)?;
                player_ticked = true;
            }
            if event_disabled(
                &self.disabled_events,
                actor,
                self.actor_states
                    .get(&actor)
                    .and_then(|state| state.as_deref()),
                "Tick",
            ) {
                if player == Some(actor) {
                    self.tick_player_events(delta_time, &mut actions)?;
                    player_ticked = true;
                }
                continue;
            }
            let Some(class) = self.actor_classes.get(&actor).cloned() else {
                continue;
            };
            let class = ResolvedObject {
                package: self.packages.load_path(Path::new(class.package.as_ref()))?,
                export_index: class.export_index,
            };
            match self.execute_actor_function(actor, &class, &function, &[Value::Float(delta_time)])
            {
                Ok(mut actor_actions) => actions.append(&mut actor_actions),
                Err(error) => {
                    // ponytail: retry deterministic Tick failures only after a state change
                    // or explicit Enable instead of failing every rendered frame.
                    self.failed_ticks.insert(actor);
                    actions.push(ActorAction::DeferredCall {
                        actor,
                        message: format!("Tick: {error}"),
                    });
                }
            }
            if player == Some(actor) {
                self.tick_player_events(delta_time, &mut actions)?;
                player_ticked = true;
            }
        }
        if player.is_some() && !player_ticked {
            self.tick_player_events(delta_time, &mut actions)?;
        }
        self.tick_lifespans(delta_time, &mut actions)?;

        let mut state_actors = self.state_frames.keys().copied().collect::<Vec<_>>();
        state_actors.sort_unstable();
        for actor in state_actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let latent = self.state_frames.get(&actor).map(|frame| frame.latent);
            if matches!(
                latent,
                Some(
                    LatentAction::FinishInterpolation(_)
                        | LatentAction::MoveTo(_)
                        | LatentAction::MoveToward(_)
                        | LatentAction::TurnTo(_)
                        | LatentAction::TurnToward(_)
                )
            ) {
                let target = match latent {
                    Some(
                        LatentAction::FinishInterpolation(target)
                        | LatentAction::MoveTo(target)
                        | LatentAction::MoveToward(target)
                        | LatentAction::TurnTo(target)
                        | LatentAction::TurnToward(target),
                    ) => target,
                    _ => unreachable!(),
                };
                let Some(class) = self.actor_classes.get(&target).cloned() else {
                    continue;
                };
                let class = self.resolved_object(&class)?;
                let mut instance = self.instances.remove(&target).unwrap_or_default();
                let result = match latent {
                    Some(LatentAction::FinishInterpolation(_)) => self
                        .actor_bool(&class, &instance, "bInterpolating")
                        .map(|interpolating| !interpolating),
                    Some(LatentAction::MoveTo(_)) => {
                        self.tick_move_to(&class, &mut instance, delta_time)
                    }
                    Some(LatentAction::MoveToward(_)) => {
                        self.tick_move_toward(&class, &mut instance, delta_time)
                    }
                    Some(LatentAction::TurnTo(_)) => self.tick_turn_to(&class, &mut instance),
                    Some(LatentAction::TurnToward(_)) => {
                        self.tick_turn_toward(&class, &mut instance)
                    }
                    _ => unreachable!(),
                };
                self.instances.insert(target, instance);
                match result {
                    Ok(true) => {
                        self.state_frames.get_mut(&actor).unwrap().latent = LatentAction::Continue;
                    }
                    Ok(false) => {}
                    Err(message) => {
                        self.state_frames.get_mut(&actor).unwrap().latent = LatentAction::Stop;
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: format!("latent movement: {message}"),
                        });
                    }
                }
            }
            let ready = match self.state_frames.get_mut(&actor) {
                Some(StateFrame {
                    latent: LatentAction::Sleep(remaining),
                    ..
                }) => {
                    *remaining = (*remaining - delta_time).max(0.0);
                    if *remaining == 0.0 {
                        self.state_frames.get_mut(&actor).unwrap().latent = LatentAction::Continue;
                        true
                    } else {
                        false
                    }
                }
                Some(StateFrame {
                    latent: LatentAction::Continue,
                    ..
                }) => true,
                _ => false,
            };
            if !ready {
                continue;
            }
            let Some(class) = self.actor_classes.get(&actor).cloned() else {
                continue;
            };
            let class = ResolvedObject {
                package: self.packages.load_path(Path::new(class.package.as_ref()))?,
                export_index: class.export_index,
            };
            let mut instance = self.instances.remove(&actor).unwrap_or_default();
            let result = self.execute_ready_state(actor, &class, &mut instance, &mut actions);
            self.instances.insert(actor, instance);
            if let Err(error) = result {
                actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("State: {error}"),
                });
            }
        }

        self.tick_physics(delta_time, &mut actions)?;

        let mut due = Vec::new();
        let actors = self.timers.keys().copied().collect::<Vec<_>>();
        for actor in actors {
            let Some(timer) = self.timers.get_mut(&actor) else {
                continue;
            };
            if !advance_timer(timer, delta_time) {
                continue;
            }
            due.push(actor);
            if !timer.looping {
                self.timers.remove(&actor);
            }
        }

        for actor in due {
            let Some(class) = self.actor_classes.get(&actor).cloned() else {
                continue;
            };
            self.timer_callbacks = self.timer_callbacks.saturating_add(1);
            match self.dispatch_event(
                actor,
                Path::new(class.package.as_ref()),
                class.export_index,
                "Timer",
            ) {
                Ok(mut actor_actions) => actions.append(&mut actor_actions),
                Err(error) => actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Timer: {error}"),
                }),
            }
        }
        Ok(actions)
    }

    pub(super) fn tick_player_events(
        &mut self,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        let arguments = [Value::Float(delta_time)];
        if std::mem::take(&mut self.player_alt_fire_pressed) {
            actions.extend(self.dispatch_player_event("AltFire", &[Value::Float(0.0)])?);
        }
        actions.extend(self.dispatch_player_event("PlayerInput", &arguments)?);
        actions.extend(self.dispatch_player_event("PlayerTick", &arguments)?);
        self.clear_player_motion_input()
    }

    fn tick_level_time(&mut self, delta_time: f32) -> DispatchResult<()> {
        let actor = self.level_info.ok_or(DispatchError::MissingLevelInfo)?;
        let time = self
            .actor_float_property(actor, "TimeSeconds")?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "LevelInfo.TimeSeconds is not a float".to_owned(),
            })?;
        let dilation = self
            .actor_float_property(actor, "TimeDilation")?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "LevelInfo.TimeDilation is not a float".to_owned(),
            })?;
        let time = advance_level_time(time, dilation, delta_time).ok_or_else(|| {
            DispatchError::UnresolvedObject {
                message: format!(
                    "LevelInfo time is invalid: time={time}, dilation={dilation}, delta={delta_time}"
                ),
            }
        })?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = self
            .set_actor_value(&class, &mut instance, "TimeSeconds", Value::Float(time))
            .map_err(|message| DispatchError::UnresolvedObject { message });
        self.instances.insert(actor, instance);
        result
    }

    fn tick_animation_properties(&mut self, delta_time: f32, actions: &mut Vec<ActorAction>) {
        let mut actors = self.animating.iter().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        for actor in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let Some(class_id) = self.actor_classes.get(&actor).cloned() else {
                actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Animation: actor {actor} is not registered"),
                });
                continue;
            };
            let class = match self.resolved_object(&class_id) {
                Ok(class) => class,
                Err(error) => {
                    actions.push(ActorAction::DeferredCall {
                        actor,
                        message: format!("Animation: {error}"),
                    });
                    continue;
                }
            };
            let mut remaining = delta_time;
            for _ in 0..10 {
                if remaining <= 0.0
                    || self.destroyed.contains(&actor)
                    || !self.animating.contains(&actor)
                {
                    break;
                }
                let mut instance = match self.instances.remove(&actor) {
                    Some(instance) => instance,
                    None => {
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: format!("Animation: animation actor {actor} is active"),
                        });
                        break;
                    }
                };
                let result = (|| {
                    let frame = self.actor_signed_float(&class, &instance, "AnimFrame")?;
                    let authored_rate = self.actor_signed_float(&class, &instance, "AnimRate")?;
                    let rate = if authored_rate >= 0.0 {
                        authored_rate
                    } else {
                        self.actor_float(&class, &instance, "AnimMinRate")?.max(
                            -authored_rate
                                * glam::Vec3::from_array(
                                    self.actor_vector(&class, &instance, "Velocity")?,
                                )
                                .length(),
                        )
                    };
                    let tween_rate = self.actor_float(&class, &instance, "TweenRate")?;
                    let last = self.actor_float(&class, &instance, "AnimLast")?;
                    let looping = self.actor_bool(&class, &instance, "bAnimLoop")?;

                    if frame < 0.0 {
                        if tween_rate == 0.0 {
                            return Ok((None, 0.0, false));
                        }
                        let (next, next_remaining) =
                            advance_animation_tween(frame, tween_rate, remaining);
                        self.set_actor_value(
                            &class,
                            &mut instance,
                            "AnimFrame",
                            Value::Float(next),
                        )?;
                        if next == 0.0 && authored_rate == 0.0 {
                            self.set_actor_value(
                                &class,
                                &mut instance,
                                "bAnimFinished",
                                Value::Bool(true),
                            )?;
                            return Ok((Some("AnimEnd".to_owned()), next_remaining, true));
                        }
                        return Ok((None, next_remaining, false));
                    }
                    if rate == 0.0 {
                        return Ok((None, 0.0, false));
                    }

                    let raw_next = frame + rate * remaining;
                    if self.actor_bool(&class, &instance, "bAnimNotify")? {
                        let sequence = match self.required_actor_property(
                            &class,
                            &instance,
                            "AnimSequence",
                        )? {
                            StoredValue::Name(sequence) => sequence,
                            value => {
                                return Err(format!("actor property AnimSequence is {value:?}"));
                            }
                        };
                        let notifications = self
                            .animation_sequences
                            .get(&actor)
                            .and_then(|sequences| sequences.get(&sequence.to_ascii_lowercase()))
                            .map(|sequence| sequence.notifications.clone())
                            .unwrap_or_default();
                        if let Some((time, function)) =
                            next_animation_notify(frame, raw_next, &notifications, |function| {
                                self.find_actor_function(
                                    actor,
                                    ResolvedObject {
                                        package: Arc::clone(&class.package),
                                        export_index: class.export_index,
                                    },
                                    function,
                                    0,
                                )
                                .map(|function| function.is_some())
                                .map_err(|error| error.to_string())
                            })?
                        {
                            self.set_actor_value(
                                &class,
                                &mut instance,
                                "AnimFrame",
                                Value::Float(time),
                            )?;
                            return Ok((
                                Some(function.to_owned()),
                                (remaining - (time - frame) / rate).max(0.0),
                                false,
                            ));
                        }
                    }

                    if looping && animation_loop_end_crossed(frame, raw_next, last) {
                        self.set_actor_value(
                            &class,
                            &mut instance,
                            "AnimFrame",
                            Value::Float(last),
                        )?;
                        return Ok((
                            Some("AnimEnd".to_owned()),
                            (remaining - (last - frame) / rate).max(0.0),
                            false,
                        ));
                    }

                    if animation_nonloop_end_crossed(looping, raw_next, last) {
                        self.set_actor_value(
                            &class,
                            &mut instance,
                            "AnimFrame",
                            Value::Float(last),
                        )?;
                        self.set_actor_value(&class, &mut instance, "AnimRate", Value::Float(0.0))?;
                        self.set_actor_value(
                            &class,
                            &mut instance,
                            "bAnimFinished",
                            Value::Bool(true),
                        )?;
                        return Ok((
                            Some("AnimEnd".to_owned()),
                            (remaining - (last - frame) / rate).max(0.0),
                            true,
                        ));
                    }

                    let end = if looping { 1.0 } else { last };
                    let (next, next_remaining) = if raw_next < frame {
                        (frame, 0.0)
                    } else if raw_next >= end {
                        if looping {
                            (0.0, (remaining - (end - frame) / rate).max(0.0))
                        } else {
                            (last, 0.0)
                        }
                    } else {
                        (raw_next, 0.0)
                    };
                    self.set_actor_value(&class, &mut instance, "AnimFrame", Value::Float(next))?;
                    Ok((None, next_remaining, false))
                })();
                self.instances.insert(actor, instance);
                match result {
                    Ok((event, next_remaining, completed)) => {
                        remaining = next_remaining;
                        if event.as_deref() == Some("AnimEnd") {
                            for frame in self.state_frames.values_mut() {
                                if frame.latent == LatentAction::FinishAnimation(actor) {
                                    frame.latent = LatentAction::Continue;
                                }
                            }
                        }
                        if completed {
                            self.animating.remove(&actor);
                            self.animation_commands.remove(&actor);
                        }
                        if let Some(event) = event {
                            match self.dispatch_event(
                                actor,
                                Path::new(class_id.package.as_ref()),
                                class_id.export_index,
                                &event,
                            ) {
                                Ok(mut event_actions) => actions.append(&mut event_actions),
                                Err(error) => actions.push(ActorAction::DeferredCall {
                                    actor,
                                    message: format!("Animation event {event}: {error}"),
                                }),
                            }
                        }
                        if remaining <= 0.0 {
                            break;
                        }
                    }
                    Err(message) => {
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: format!("Animation: {message}"),
                        });
                        break;
                    }
                }
            }
        }
    }

    pub(in crate::world) fn actor_signed_float(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<f32, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Float(value)) if value.is_finite() => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn tick_lifespans(
        &mut self,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        let mut actors = self
            .actor_classes
            .iter()
            .filter(|(actor, _)| !self.destroyed.contains(actor))
            .map(|(&actor, class)| (actor, class.clone()))
            .collect::<Vec<_>>();
        actors.sort_unstable_by_key(|(actor, _)| *actor);
        let mut expired = Vec::new();
        for (actor, class) in actors {
            let class = self.resolved_object(&class)?;
            let Some(field) = self.find_property(&class, "LifeSpan", 0)? else {
                continue;
            };
            let Some(StoredValue::Value(Value::Float(lifespan))) = self
                .instances
                .get_mut(&actor)
                .and_then(|instance| instance.get_mut(&field))
            else {
                continue;
            };
            if advance_lifespan(lifespan, delta_time) {
                expired.push((actor, class));
            }
        }

        for (actor, class) in expired {
            match self.dispatch_event(
                actor,
                Path::new(class.package.summary().source.as_ref()),
                class.export_index,
                "Expired",
            ) {
                Ok(mut expired_actions) => actions.append(&mut expired_actions),
                Err(error) => actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Expired: {error}"),
                }),
            }
            if self.destroyed.contains(&actor) {
                continue;
            }
            let mut instance = self
                .instances
                .remove(&actor)
                .ok_or(DispatchError::ActiveActorContext { actor })?;
            let result = self.destroy_actor(actor, &class, &mut instance, actions);
            self.instances.insert(actor, instance);
            result.map_err(|message| DispatchError::UnresolvedObject { message })?;
        }
        Ok(())
    }

    pub fn animation_finished(&mut self, actor: usize) -> DispatchResult<Vec<ActorAction>> {
        if self.animating.contains(&actor) {
            return Ok(Vec::new());
        }
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(Vec::new());
        };
        let resolved = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let finished = self
            .actor_bool(&resolved, &instance, "bAnimFinished")
            .map_err(|message| DispatchError::UnresolvedObject { message })?;
        if finished {
            self.instances.insert(actor, instance);
            return Ok(Vec::new());
        }
        let looping = self
            .actor_bool(&resolved, &instance, "bAnimLoop")
            .map_err(|message| DispatchError::UnresolvedObject { message })?;
        for frame in self.state_frames.values_mut() {
            if frame.latent == LatentAction::FinishAnimation(actor) {
                frame.latent = LatentAction::Continue;
            }
        }
        let result = (|| {
            let last = self.actor_float(&resolved, &instance, "AnimLast")?;
            self.set_actor_value(&resolved, &mut instance, "AnimFrame", Value::Float(last))?;
            if !looping {
                self.set_actor_value(&resolved, &mut instance, "AnimRate", Value::Float(0.0))?;
                self.set_actor_value(&resolved, &mut instance, "bAnimFinished", Value::Bool(true))?;
            }
            Ok(())
        })();
        self.instances.insert(actor, instance);
        result.map_err(|message| DispatchError::UnresolvedObject { message })?;
        if !looping {
            self.animating.remove(&actor);
            self.animation_commands.remove(&actor);
        }
        self.dispatch_event(
            actor,
            Path::new(class.package.as_ref()),
            class.export_index,
            "AnimEnd",
        )
    }

    pub fn apply_root_motion(
        &mut self,
        actor: usize,
        delta: [f32; 3],
    ) -> DispatchResult<Vec<ActorAction>> {
        if !delta.iter().all(|value| value.is_finite()) {
            return Err(DispatchError::UnresolvedObject {
                message: "root motion delta is not finite".to_owned(),
            });
        }
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let mut actions = Vec::new();
        let result = self.move_actor_smooth(actor, &class, delta, &mut instance, &mut actions);
        self.instances.insert(actor, instance);
        result.map_err(|message| DispatchError::UnresolvedObject { message })?;
        Ok(actions)
    }

    pub fn timer_callbacks(&self) -> usize {
        self.timer_callbacks
    }

    pub fn state_resumes(&self) -> usize {
        self.state_resumes
    }

    pub fn dispatch_event_with_arguments(
        &mut self,
        actor: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
        event: &str,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        if self.destroyed.contains(&actor) && !event.eq_ignore_ascii_case("Destroyed") {
            return Ok(Vec::new());
        }
        let package = self.packages.load_path(class_package)?;
        let class = ResolvedObject {
            package,
            export_index: class_export,
        };
        let actor_class = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        self.update_touching(actor, &class, event, arguments)?;
        if event_disabled(
            &self.disabled_events,
            actor,
            self.actor_states
                .get(&actor)
                .and_then(|state| state.as_deref()),
            event,
        ) || self.state_ignores_event(actor, &class, event)?
        {
            return Ok(Vec::new());
        }
        let Some(function) = self.find_actor_function(actor, class, event, 0)? else {
            return Ok(Vec::new());
        };
        self.execute_actor_function(actor, &actor_class, &function, arguments)
    }

    fn update_touching(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        event: &str,
        arguments: &[Value],
    ) -> DispatchResult<()> {
        let touching = if event.eq_ignore_ascii_case("Touch") {
            true
        } else if event.eq_ignore_ascii_case("UnTouch") {
            false
        } else {
            return Ok(());
        };
        let Some(Value::Object(handle)) = arguments.first() else {
            return Ok(());
        };
        let other = match self.stored_value(&class.package, &Value::Object(*handle))? {
            StoredValue::Object(Some(other)) => other,
            _ => return Ok(()),
        };
        let Some(field) = self.find_property(class, "Touching", 0)? else {
            return Ok(());
        };
        let resolved = self.resolved_object(&field)?;
        let zero = self.zero_field_value(&resolved)?.ok_or_else(|| {
            DispatchError::InvalidArrayProperty {
                property: "Touching".to_owned(),
            }
        })?;
        let zero = self.stored_value(&class.package, &zero)?;
        let value = self
            .instances
            .get_mut(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?
            .entry(field)
            .or_insert(zero);
        let StoredValue::Array(values) = value else {
            return Err(DispatchError::InvalidArrayProperty {
                property: "Touching".to_owned(),
            });
        };
        update_touching_array(values, other, touching);
        Ok(())
    }
}

pub(super) fn particle_bool(value: Option<StoredValue>) -> bool {
    matches!(value, Some(StoredValue::Value(Value::Bool(true))))
}

pub(super) fn particle_int(value: Option<StoredValue>) -> usize {
    match value {
        Some(StoredValue::Value(Value::Int(value))) => usize::try_from(value).unwrap_or(0),
        _ => 0,
    }
}

pub(super) fn particle_byte(value: Option<StoredValue>) -> u8 {
    match value {
        Some(StoredValue::Value(Value::Byte(value))) => value,
        _ => 0,
    }
}

pub(super) fn particle_scalar(value: Option<StoredValue>) -> f32 {
    match value {
        Some(StoredValue::Value(Value::Float(value))) if value.is_finite() => value,
        _ => 0.0,
    }
}

pub(super) fn particle_float(value: Option<StoredValue>) -> ParticleFloat {
    let Some(StoredValue::Value(Value::Struct(values))) = value else {
        return ParticleFloat::default();
    };
    let get = |name: &str| match values.get(name) {
        Some(Value::Float(value)) if value.is_finite() => *value,
        _ => 0.0,
    };
    ParticleFloat {
        base: get("Base"),
        random: get("Rand"),
    }
}

pub(super) fn particle_color(value: Option<StoredValue>) -> ParticleColor {
    let Some(StoredValue::Value(Value::Struct(values))) = value else {
        return ParticleColor::default();
    };
    let color = |name: &str| {
        let Some(Value::Struct(values)) = values.get(name) else {
            return [0; 4];
        };
        let component = |name: &str| match values.get(name) {
            Some(Value::Byte(value)) => *value,
            _ => 0,
        };
        [
            component("R"),
            component("G"),
            component("B"),
            component("A"),
        ]
    };
    ParticleColor {
        base: color("Base"),
        random: color("Rand"),
    }
}

pub(super) fn particle_vector(value: Option<StoredValue>) -> [f32; 3] {
    match value {
        Some(StoredValue::Value(Value::Vector(value)))
            if value.iter().all(|component| component.is_finite()) =>
        {
            value
        }
        _ => [0.0; 3],
    }
}

fn advance_animation_tween(frame: f32, rate: f32, delta_time: f32) -> (f32, f32) {
    let required = -frame / rate;
    if required <= delta_time {
        (0.0, (delta_time - required).max(0.0))
    } else {
        (frame + rate * delta_time, 0.0)
    }
}

fn animation_loop_end_crossed(frame: f32, raw_next: f32, last: f32) -> bool {
    last > frame && last <= raw_next
}

fn animation_nonloop_end_crossed(looping: bool, raw_next: f32, last: f32) -> bool {
    !looping && raw_next >= last
}

fn next_animation_notify<'a, E>(
    frame: f32,
    raw_next: f32,
    notifications: &'a [(f32, String)],
    mut callable: impl FnMut(&str) -> Result<bool, E>,
) -> Result<Option<(f32, &'a str)>, E> {
    for (time, function) in notifications {
        if time.is_finite() && *time > frame && *time <= raw_next && callable(function)? {
            return Ok(Some((*time, function)));
        }
    }
    Ok(None)
}

fn advance_level_time(time: f32, dilation: f32, delta_time: f32) -> Option<f32> {
    let next = time + dilation * delta_time;
    (time.is_finite() && dilation.is_finite() && next.is_finite()).then_some(next)
}

pub(in crate::world) fn decode_latent_action(index: i32, actor: usize) -> LatentAction {
    match index {
        0 => LatentAction::Continue,
        0x101 => LatentAction::Sleep(0.0),
        0x106 => LatentAction::FinishAnimation(actor),
        0x12e => LatentAction::FinishInterpolation(actor),
        501 => LatentAction::MoveTo(actor),
        503 => LatentAction::MoveToward(actor),
        509 => LatentAction::TurnTo(actor),
        511 => LatentAction::TurnToward(actor),
        _ => LatentAction::Stop,
    }
}

pub(in crate::world) fn advance_timer(timer: &mut ActorTimer, delta_time: f32) -> bool {
    timer.remaining -= delta_time;
    if timer.remaining > 0.0 {
        return false;
    }
    if timer.looping {
        // ponytail: one callback per rendered frame; add catch-up callbacks
        // if sub-frame timer fidelity becomes observable.
        timer.remaining = timer.rate - (-timer.remaining).rem_euclid(timer.rate);
    }
    true
}

pub(in crate::world) fn advance_lifespan(lifespan: &mut f32, delta_time: f32) -> bool {
    if *lifespan <= 0.0 {
        return false;
    }
    if *lifespan <= delta_time {
        *lifespan = 0.0;
        true
    } else {
        *lifespan -= delta_time;
        false
    }
}

pub(in crate::world) fn update_touching_array(
    values: &mut [StoredValue],
    other: ObjectId,
    touching: bool,
) {
    let current = values
        .iter()
        .position(|value| matches!(value, StoredValue::Object(Some(value)) if value == &other));
    if touching {
        if current.is_none()
            && let Some(slot) = values
                .iter_mut()
                .find(|value| matches!(value, StoredValue::Object(None)))
        {
            *slot = StoredValue::Object(Some(other));
        }
    } else if let Some(index) = current {
        values[index] = StoredValue::Object(None);
    }
}

#[cfg(test)]
mod animation_tests {
    use std::collections::HashMap;

    use super::{
        LatentAction, ParticleColor, ParticleEmitter, StoredValue, Value, advance_animation_tween,
        advance_level_time, animation_loop_end_crossed, animation_nonloop_end_crossed,
        decode_latent_action, next_animation_notify, particle_color, particle_scalar,
    };

    #[test]
    fn animation_notifications_follow_callable_authored_events_and_raw_time() {
        let notifications = [
            (0.05, "Missing".to_owned()),
            (0.1, "Cast".to_owned()),
            (0.2, "AfterCast".to_owned()),
            (1.0, "PastLastFrame".to_owned()),
        ];
        assert_eq!(
            next_animation_notify(0.0, 0.5, &notifications, |function| {
                Ok::<_, ()>(function != "Missing")
            }),
            Ok(Some((0.1, "Cast")))
        );
        assert_eq!(
            next_animation_notify(0.1, 0.2, &notifications, |_| Ok::<_, ()>(true)),
            Ok(Some((0.2, "AfterCast")))
        );
        assert_eq!(
            next_animation_notify(0.9, 1.2, &notifications, |_| Ok::<_, ()>(true)),
            Ok(Some((1.0, "PastLastFrame")))
        );
    }

    #[test]
    fn tween_and_loop_boundaries_leave_time_for_the_next_step() {
        assert_eq!(advance_animation_tween(-0.1, 2.0, 0.025), (-0.05, 0.0));
        assert_eq!(advance_animation_tween(-0.1, 2.0, 0.1), (0.0, 0.05));
        assert!(animation_loop_end_crossed(0.9, 1.1, 0.95));
        assert!(!animation_loop_end_crossed(0.95, 1.1, 0.95));
        assert!(animation_nonloop_end_crossed(false, 0.95, 0.95));
        assert!(!animation_nonloop_end_crossed(true, 0.95, 0.95));
        let notifications = [
            (0.05, "AfterWrap".to_owned()),
            (0.95, "BeforeWrap".to_owned()),
        ];
        assert_eq!(
            next_animation_notify(0.9, 1.1, &notifications, |_| Ok::<_, ()>(true)),
            Ok(Some((0.95, "BeforeWrap")))
        );
        assert_eq!(
            next_animation_notify(0.0, 0.1, &notifications, |_| Ok::<_, ()>(true)),
            Ok(Some((0.05, "AfterWrap")))
        );
    }

    #[test]
    fn level_time_advances_with_time_dilation() {
        assert_eq!(advance_level_time(10.0, 0.5, 2.0), Some(11.0));
        assert_eq!(advance_level_time(f32::NAN, 1.0, 1.0), None);
    }

    #[test]
    fn decodes_turn_toward_latent_state() {
        assert_eq!(decode_latent_action(503, 7), LatentAction::MoveToward(7));
        assert_eq!(decode_latent_action(511, 7), LatentAction::TurnToward(7));
    }

    #[test]
    fn decodes_authored_particle_color_ranges() {
        let color = |red, green, blue| {
            Value::Struct(HashMap::from([
                ("R".to_owned(), Value::Byte(red)),
                ("G".to_owned(), Value::Byte(green)),
                ("B".to_owned(), Value::Byte(blue)),
                ("A".to_owned(), Value::Byte(0)),
            ]))
        };
        assert_eq!(
            particle_color(Some(StoredValue::Value(Value::Struct(HashMap::from([
                ("Base".to_owned(), color(1, 2, 3)),
                ("Rand".to_owned(), color(4, 5, 6)),
            ]))))),
            ParticleColor {
                base: [1, 2, 3, 0],
                random: [4, 5, 6, 0],
            }
        );
    }

    #[test]
    fn projects_authored_chaos_delay_and_defaults_missing_values_to_zero() {
        let emitter = |value| ParticleEmitter {
            chaos_delay: particle_scalar(value),
            ..Default::default()
        };
        assert_eq!(
            emitter(Some(StoredValue::Value(Value::Float(0.5)))).chaos_delay,
            0.5,
        );
        assert_eq!(emitter(None).chaos_delay, 0.0);
    }
}
