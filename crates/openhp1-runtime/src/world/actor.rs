use super::state::{event_disabled, set_event_disabled};
use super::*;

impl ScriptRuntime {
    pub fn new(game_root: impl AsRef<Path>) -> DispatchResult<Self> {
        Ok(Self {
            packages: PackageStore::scan_game_root(game_root)?,
            scripts: HashMap::default(),
            function_lookups: HashMap::default(),
            state_lookups: HashMap::default(),
            instances: HashMap::default(),
            class_defaults: HashMap::default(),
            class_relations: HashMap::default(),
            fields: HashMap::default(),
            resolved_references: HashMap::default(),
            zero_values: HashMap::default(),
            frame_arguments: HashMap::default(),
            struct_members: HashMap::default(),
            actor_classes: HashMap::default(),
            actor_states: HashMap::default(),
            state_frames: HashMap::default(),
            state_revisions: HashMap::default(),
            active_state_actor: None,
            pending_latent: None,
            state_resumes: 0,
            tick_functions: HashMap::default(),
            failed_ticks: HashSet::default(),
            disabled_events: HashMap::default(),
            object_actors: HashMap::default(),
            actor_objects: HashMap::default(),
            destroyed: HashSet::default(),
            timers: HashMap::default(),
            timer_callbacks: 0,
            random_state: 0x6d2b_79f5,
            object_handles: HashMap::default(),
            handle_objects: Vec::new(),
            next_actor: 0,
            collision: None,
            level_package: None,
            level_info: None,
            player_actor: None,
            animation_groups: HashMap::default(),
            animating: HashSet::default(),
            player_probe_touching: HashSet::default(),
            collision_fields: HashMap::default(),
            collision_actors: Vec::new(),
            collision_actors_by_min_x: Vec::new(),
            grounded_world: HashMap::default(),
            actor_bases: HashMap::default(),
            base_children: HashMap::default(),
            touching: HashSet::default(),
        })
    }

    pub fn set_collision(
        &mut self,
        collision: Arc<BspCollision>,
        level_package: impl AsRef<Path>,
    ) -> DispatchResult<()> {
        let package = self.packages.load_path(level_package)?;
        self.collision = Some(collision);
        self.level_package = Some(Arc::clone(&package.summary().source));
        Ok(())
    }

    pub fn set_actor_animation_groups(
        &mut self,
        actor: usize,
        groups: impl IntoIterator<Item = (String, String)>,
    ) {
        self.animation_groups.insert(
            actor,
            groups
                .into_iter()
                .map(|(sequence, group)| (sequence.to_ascii_lowercase(), group))
                .collect(),
        );
    }

    pub fn register_actor(
        &mut self,
        actor: usize,
        actor_package: impl AsRef<Path>,
        actor_export: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
    ) -> DispatchResult<()> {
        self.next_actor = self.next_actor.max(actor.saturating_add(1));
        let actor_package = self.packages.load_path(actor_package)?;
        let actor_entry = actor_package.summary().exports.get(actor_export).ok_or(
            openhp1_package::Error::InvalidExportIndex {
                package: Arc::clone(&actor_package.summary().source),
                index: actor_export,
                export_count: actor_package.summary().exports.len(),
            },
        )?;
        let class = ResolvedObject {
            package: self.packages.load_path(class_package)?,
            export_index: class_export,
        };
        let object = object_id(&actor_package, actor_export);
        self.object_handle(object.clone())?;
        self.object_actors.insert(object.clone(), actor);
        self.actor_objects.insert(actor, object.clone());
        self.actor_classes
            .insert(actor, object_id(&class.package, class.export_index));
        if self.level_info.is_none() && self.class_has_name(&class, "LevelInfo")? {
            self.level_info = Some(actor);
        }
        if self.player_actor.is_none() && self.class_has_name(&class, "PlayerPawn")? {
            self.player_actor = Some(actor);
        }

        let mut instance = self.load_class_defaults(&class, 0)?;
        let mut reader = actor_package.export_reader(actor_export)?;
        let stack = reader.read_object_stack(actor_entry.object_flags)?;
        let state = stack
            .and_then(|stack| {
                (stack.function != ObjectReference::None)
                    .then_some(stack.function)
                    .or((stack.state != ObjectReference::None).then_some(stack.state))
            })
            .map(|state| self.packages.resolve(&actor_package, state))
            .transpose()?
            .flatten();
        let state_name = state.as_ref().map(|state| {
            state
                .package
                .summary()
                .name(state.package.summary().exports[state.export_index].object_name)
                .to_owned()
        });
        if let (Some(stack), Some(state)) = (stack, state.as_ref())
            && stack.function != ObjectReference::None
            && matches!(self.script(state)?.metadata, ScriptMetadata::State(_))
        {
            let script = self.script(state)?;
            let offset = stack
                .bytecode_offset
                .and_then(|offset| usize::try_from(offset).ok())
                .filter(|offset| *offset <= script.bytecode.bytes.len())
                .ok_or_else(|| DispatchError::InvalidStateLabel {
                    state: state
                        .package
                        .summary()
                        .name(state.package.summary().exports[state.export_index].object_name)
                        .to_owned(),
                    label: format!("#{}", stack.bytecode_offset.unwrap_or(-1)),
                    length: script.bytecode.bytes.len(),
                })?;
            self.state_frames.insert(
                actor,
                StateFrame {
                    state: object_id(&state.package, state.export_index),
                    frame: FrameSnapshot::at(offset),
                    latent: decode_latent_action(stack.latent_action),
                },
            );
        }
        if let Some(stack) = stack {
            for (index, event) in PROBE_EVENTS.iter().enumerate() {
                if stack.probe_mask & (1_u64 << index) != 0 {
                    set_event_disabled(
                        &mut self.disabled_events,
                        actor,
                        state_name.as_deref(),
                        event,
                        true,
                    );
                }
            }
        }
        self.actor_states.insert(actor, state_name);
        self.refresh_tick_actor(actor, &class)?;
        self.apply_properties(&class, &actor_package, &mut reader, &mut instance)?;
        // UE1 starts desired turning from the actor's spawn orientation.
        if let Some(rotation) = self
            .find_property(&class, "Rotation", 0)?
            .and_then(|field| instance.get(&field))
            .cloned()
            && let Some(desired) = self.find_property(&class, "DesiredRotation", 0)?
        {
            instance.insert(desired, rotation);
        }
        let base =
            self.find_property(&class, "Base", 0)?
                .and_then(|field| match instance.get(&field) {
                    Some(StoredValue::Object(base)) => base.clone(),
                    _ => None,
                });
        self.update_actor_base(actor, base);
        self.instances.insert(actor, instance);
        Ok(())
    }

    pub fn player_actor(&self) -> Option<usize> {
        self.player_actor
    }

    pub fn active_actor_count(&self) -> usize {
        self.actor_classes
            .len()
            .saturating_sub(self.destroyed.len())
    }

    pub fn initialize_game(&mut self) -> DispatchResult<Vec<ActorAction>> {
        let level = self.level_info.ok_or(DispatchError::MissingLevelInfo)?;
        let level_class = self
            .actor_classes
            .get(&level)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: level })?;
        let level_class = self.resolved_object(&level_class)?;
        let game_package = self.packages.load("Engine")?;
        let game_export = game_package
            .summary()
            .exports
            .iter()
            .position(|export| {
                export.class == ObjectReference::None
                    && game_package
                        .summary()
                        .name(export.object_name)
                        .eq_ignore_ascii_case("GameInfo")
            })
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "Engine.GameInfo class is missing".to_owned(),
            })?;
        let mut level_instance = self
            .instances
            .remove(&level)
            .ok_or(DispatchError::ActiveActorContext { actor: level })?;
        let mut actions = Vec::new();
        let result = self
            .spawn_actor(
                level,
                &level_class,
                &game_package,
                &[Value::Object(
                    i32::try_from(game_export + 1).map_err(|_| DispatchError::ObjectLimit)?,
                )],
                &mut level_instance,
                &mut actions,
            )
            .map_err(|message| DispatchError::UnresolvedObject { message });
        self.instances.insert(level, level_instance);
        let Value::Object(game_handle) = result? else {
            return Err(DispatchError::UnresolvedObject {
                message: "GameInfo spawn returned no actor".to_owned(),
            });
        };
        let game = self.actor_for_handle(game_handle)?;
        let game_object = self
            .actor_objects
            .get(&game)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: game })?;
        let mut level_instance = self
            .instances
            .remove(&level)
            .ok_or(DispatchError::ActiveActorContext { actor: level })?;
        let result = self.set_actor_stored(
            &level_class,
            &mut level_instance,
            "Game",
            StoredValue::Object(Some(game_object)),
        );
        self.instances.insert(level, level_instance);
        result.map_err(|message| DispatchError::UnresolvedObject { message })?;
        actions.extend(self.dispatch_event_with_arguments(
            game,
            Path::new(game_package.summary().source.as_ref()),
            game_export,
            "InitGame",
            &[Value::String(String::new()), Value::String(String::new())],
        )?);
        Ok(actions)
    }

    pub fn set_player_view_target_class(
        &mut self,
        class_name: &str,
    ) -> DispatchResult<Option<usize>> {
        let player = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        let mut target = None;
        for actor in actors {
            if actor == player || self.destroyed.contains(&actor) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            if self.class_has_name(&class, class_name)? {
                target = Some(actor);
                break;
            }
        }
        let Some(target) = target else {
            return Ok(None);
        };
        let target_object = self
            .actor_objects
            .get(&target)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: target })?;
        self.set_player_view_target(Some(target_object))?;
        Ok(Some(target))
    }

    pub fn clear_player_view_target(&mut self) -> DispatchResult<()> {
        self.set_player_view_target(None)
    }

    pub fn player_state_name(&self) -> Option<&str> {
        self.player_actor
            .and_then(|actor| self.actor_states.get(&actor))
            .and_then(|state| state.as_deref())
    }

    fn set_player_view_target(&mut self, target: Option<ObjectId>) -> DispatchResult<()> {
        let player = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&player)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: player })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&player)
            .ok_or(DispatchError::ActiveActorContext { actor: player })?;
        let result = self
            .set_actor_stored(
                &class,
                &mut instance,
                "ViewTarget",
                StoredValue::Object(target),
            )
            .map_err(|message| DispatchError::InvalidPlayerView { message });
        self.instances.insert(player, instance);
        result
    }

    pub fn set_player_input(&mut self, input: PlayerInput) -> DispatchResult<()> {
        if ![
            input.base_x,
            input.base_y,
            input.strafe,
            input.mouse_x,
            input.mouse_y,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(DispatchError::InvalidPlayerInput {
                message: "input axes must be finite".to_owned(),
            });
        }
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
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
        let result = (|| {
            for (name, value) in [
                ("aBaseX", Value::Float(input.base_x)),
                ("aBaseY", Value::Float(input.base_y)),
                ("aStrafe", Value::Float(input.strafe)),
                ("aMouseX", Value::Float(input.mouse_x)),
                ("aMouseY", Value::Float(input.mouse_y)),
                ("bAltFire", Value::Byte(u8::from(input.alt_fire))),
                ("bBroomAction", Value::Bool(input.jump)),
                ("bPressedJump", Value::Bool(input.jump)),
            ] {
                self.set_actor_value(&class, &mut instance, name, value)
                    .map_err(|message| DispatchError::InvalidPlayerInput { message })?;
            }
            Ok(())
        })();
        self.instances.insert(actor, instance);
        result
    }

    pub fn dispatch_player_event(
        &mut self,
        event: &str,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        self.dispatch_event_with_arguments(
            actor,
            Path::new(class.package.as_ref()),
            class.export_index,
            event,
            arguments,
        )
    }

    pub fn tick_player(
        &mut self,
        input: PlayerInput,
        delta_time: f32,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.set_player_input(input)?;
        let arguments = [Value::Float(delta_time)];
        let result = (|| {
            let mut actions = self.dispatch_player_event("PlayerInput", &arguments)?;
            actions.extend(self.dispatch_player_event("PlayerTick", &arguments)?);
            Ok(actions)
        })();
        let cleared = self.clear_player_motion_input();
        match (result, cleared) {
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            (Ok(actions), Ok(())) => Ok(actions),
        }
    }

    pub fn player_view(
        &mut self,
        location: [f32; 3],
        rotation: [i32; 3],
    ) -> DispatchResult<(PlayerView, Vec<ActorAction>)> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class_id = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class_id)?;
        let handle = self.object_handle(
            self.actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?,
        )?;
        let arguments = [
            Value::Object(handle),
            Value::Vector(location),
            Value::Rotator(rotation),
        ];
        let mut output_arguments = arguments.to_vec();
        let actions =
            if let Some(function) = self.find_actor_function(actor, class, "PlayerCalcView", 0)? {
                let actor_class = self.resolved_object(&class_id)?;
                self.execute_actor_function_with_outputs(
                    actor,
                    &actor_class,
                    &function,
                    &arguments,
                    &mut output_arguments,
                )?
            } else {
                Vec::new()
            };
        let [
            Value::Object(view_handle),
            Value::Vector(location),
            Value::Rotator(rotation),
        ] = output_arguments.as_slice()
        else {
            return Err(DispatchError::InvalidPlayerView {
                message: format!("PlayerCalcView returned {output_arguments:?}"),
            });
        };
        let view_actor = if *view_handle == 0 {
            actor
        } else {
            self.actor_for_handle(*view_handle)?
        };
        let fov_degrees = self
            .actor_float_property(actor, "FovAngle")?
            .unwrap_or(90.0);
        if !fov_degrees.is_finite() || !(1.0..179.0).contains(&fov_degrees) {
            return Err(DispatchError::InvalidPlayerView {
                message: format!("FovAngle is {fov_degrees}"),
            });
        }
        Ok((
            PlayerView {
                actor: view_actor,
                location: *location,
                rotation: *rotation,
                fov_degrees,
            },
            actions,
        ))
    }

    fn actor_float_property(&mut self, actor: usize, name: &str) -> DispatchResult<Option<f32>> {
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
        Ok(match self.instance_property(&class, &instance, name)? {
            Some(StoredValue::Value(Value::Float(value))) => Some(value),
            _ => None,
        })
    }

    fn clear_player_motion_input(&mut self) -> DispatchResult<()> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
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
        let result = (|| {
            for name in ["aForward", "aTurn", "aLookUp"] {
                self.set_actor_value(&class, &mut instance, name, Value::Float(0.0))
                    .map_err(|message| DispatchError::InvalidPlayerInput { message })?;
            }
            Ok(())
        })();
        self.instances.insert(actor, instance);
        result
    }

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
        let mut actions = Vec::new();
        for (actor, function) in actors {
            if event_disabled(
                &self.disabled_events,
                actor,
                self.actor_states
                    .get(&actor)
                    .and_then(|state| state.as_deref()),
                "Tick",
            ) {
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
                    LatentAction::FinishInterpolation | LatentAction::MoveTo | LatentAction::TurnTo
                )
            ) {
                let Some(class) = self.actor_classes.get(&actor).cloned() else {
                    continue;
                };
                let class = self.resolved_object(&class)?;
                let mut instance = self.instances.remove(&actor).unwrap_or_default();
                let result = match latent {
                    Some(LatentAction::FinishInterpolation) => self
                        .actor_bool(&class, &instance, "bInterpolating")
                        .map(|interpolating| !interpolating),
                    Some(LatentAction::MoveTo) => {
                        self.tick_move_to(&class, &mut instance, delta_time)
                    }
                    Some(LatentAction::TurnTo) => self.tick_turn_to(&class, &mut instance),
                    _ => unreachable!(),
                };
                self.instances.insert(actor, instance);
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
        self.animating.remove(&actor);
        if let Some(frame) = self.state_frames.get_mut(&actor)
            && frame.latent == LatentAction::FinishAnimation
        {
            frame.latent = LatentAction::Continue;
        }
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(Vec::new());
        };
        self.dispatch_event(
            actor,
            Path::new(class.package.as_ref()),
            class.export_index,
            "AnimEnd",
        )
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

pub(super) fn decode_latent_action(index: i32) -> LatentAction {
    match index {
        0 => LatentAction::Continue,
        0x101 => LatentAction::Sleep(0.0),
        0x106 => LatentAction::FinishAnimation,
        0x12e => LatentAction::FinishInterpolation,
        501 => LatentAction::MoveTo,
        509 => LatentAction::TurnTo,
        _ => LatentAction::Stop,
    }
}

pub(super) fn advance_timer(timer: &mut ActorTimer, delta_time: f32) -> bool {
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

pub(super) fn advance_lifespan(lifespan: &mut f32, delta_time: f32) -> bool {
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

pub(super) fn update_touching_array(values: &mut [StoredValue], other: ObjectId, touching: bool) {
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
