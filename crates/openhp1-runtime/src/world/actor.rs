use super::state::{event_disabled, set_event_disabled};
use super::*;

impl ScriptRuntime {
    pub fn new(game_root: impl AsRef<Path>) -> DispatchResult<Self> {
        Ok(Self {
            packages: PackageStore::scan_game_root(game_root)?,
            scripts: HashMap::new(),
            function_lookups: HashMap::new(),
            instances: HashMap::new(),
            class_defaults: HashMap::new(),
            class_relations: HashMap::new(),
            fields: HashMap::new(),
            resolved_fields: HashMap::new(),
            zero_values: HashMap::new(),
            frame_arguments: HashMap::new(),
            struct_members: HashMap::new(),
            actor_classes: HashMap::new(),
            actor_states: HashMap::new(),
            state_frames: HashMap::new(),
            state_revisions: HashMap::new(),
            active_state_actor: None,
            pending_latent: None,
            state_resumes: 0,
            tick_functions: HashMap::new(),
            failed_ticks: HashSet::new(),
            disabled_events: HashMap::new(),
            object_actors: HashMap::new(),
            actor_objects: HashMap::new(),
            destroyed: HashSet::new(),
            timers: HashMap::new(),
            timer_callbacks: 0,
            object_handles: HashMap::new(),
            handle_objects: Vec::new(),
        })
    }

    pub fn register_actor(
        &mut self,
        actor: usize,
        actor_package: impl AsRef<Path>,
        actor_export: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
    ) -> DispatchResult<()> {
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
                    latent: match stack.latent_action {
                        0 => LatentAction::Continue,
                        0x101 => LatentAction::Sleep(0.0),
                        0x106 => LatentAction::FinishAnimation,
                        _ => LatentAction::Stop,
                    },
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
        self.instances.insert(actor, instance);
        Ok(())
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

        let mut state_actors = self.state_frames.keys().copied().collect::<Vec<_>>();
        state_actors.sort_unstable();
        for actor in state_actors {
            if self.destroyed.contains(&actor) {
                continue;
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

    pub fn animation_finished(&mut self, actor: usize) -> DispatchResult<Vec<ActorAction>> {
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
