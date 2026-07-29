use super::native::runtime_name;
use super::*;
use crate::IteratorValue;
use glam::Vec3;

struct CallOutput {
    value: Value,
    outputs: Vec<(usize, Value)>,
}

impl CallOutput {
    fn value(value: Value) -> Self {
        Self {
            value,
            outputs: Vec::new(),
        }
    }

    fn from_arguments(value: Value, arguments: &[Value], output_arguments: Vec<Value>) -> Self {
        let outputs = arguments
            .iter()
            .zip(output_arguments)
            .enumerate()
            .filter_map(|(index, (input, output))| (input != &output).then_some((index, output)))
            .collect();
        Self { value, outputs }
    }

    fn into_response(self) -> FrameResponse {
        if self.outputs.is_empty() {
            FrameResponse::Value(self.value)
        } else {
            FrameResponse::ValueWithOutputs {
                value: self.value,
                outputs: self.outputs,
            }
        }
    }
}

impl ScriptRuntime {
    pub(super) fn execute_actor_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        self.execute_actor_function_inner(actor, actor_class, function, arguments, None)
    }

    pub(super) fn execute_actor_function_with_outputs(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        output_arguments: &mut Vec<Value>,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.execute_actor_function_inner(
            actor,
            actor_class,
            function,
            arguments,
            Some(output_arguments),
        )
    }

    fn execute_actor_function_inner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        output_arguments: Option<&mut Vec<Value>>,
    ) -> DispatchResult<Vec<ActorAction>> {
        let mut actions = Vec::new();
        let mut instance = self.instances.remove(&actor).unwrap_or_default();
        let state_revision = self.state_revision(actor);
        let result = if let Some(output_arguments) = output_arguments {
            self.execute_function_with_outputs(
                actor,
                actor_class,
                function,
                arguments,
                &mut instance,
                &mut actions,
                0,
                Some(output_arguments),
            )
        } else {
            self.execute_function(
                actor,
                actor_class,
                function,
                arguments,
                &mut instance,
                &mut actions,
                0,
            )
        };
        let state_result = if result.is_ok() && self.state_revision(actor) != state_revision {
            self.execute_ready_state(actor, actor_class, &mut instance, &mut actions)
        } else {
            Ok(())
        };
        self.instances.insert(actor, instance);
        result?;
        state_result?;
        Ok(actions)
    }

    pub(super) fn execute_ready_state(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        for _ in 0..MAX_CALL_DEPTH {
            let Some(mut state_frame) = self.state_frames.remove(&actor) else {
                return Ok(());
            };
            match state_frame.latent {
                LatentAction::Continue => {}
                LatentAction::Sleep(remaining) if remaining <= 0.0 => {
                    state_frame.latent = LatentAction::Continue;
                }
                LatentAction::Stop
                | LatentAction::Sleep(_)
                | LatentAction::FinishAnimation(_)
                | LatentAction::FinishInterpolation(_)
                | LatentAction::MoveTo(_)
                | LatentAction::MoveToward(_)
                | LatentAction::TurnTo(_)
                | LatentAction::TurnToward(_) => {
                    self.state_frames.insert(actor, state_frame);
                    return Ok(());
                }
            }

            let state = self.resolved_object(&state_frame.state)?;
            let script = self.script(&state)?;
            let mut frame = Frame::from_snapshot(&script.bytecode, state_frame.frame);
            self.bind_struct_members(&state, &script.bytecode, &mut frame)?;
            self.bind_frame_defaults(actor_class, &state.package, &script.bytecode, &mut frame)?;
            self.bind_frame_zero_values(&state.package, &script.bytecode, &mut frame)?;
            let revision = self.state_revision(actor);
            self.state_resumes = self.state_resumes.saturating_add(1);
            self.pending_latent = None;
            let previous_state_actor = self.active_state_actor.replace(actor);
            let run = frame.resume_hosted(|request| {
                let result = match request {
                    FrameRequest::Call {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_context_call(
                            actor,
                            actor_class,
                            receiver,
                            &state.package,
                            call,
                            &arguments,
                            instance,
                            actions,
                            1,
                        )
                        .map(CallOutput::into_response),
                    FrameRequest::CallIterator {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_iterator_call(
                            actor,
                            receiver,
                            &state.package,
                            call,
                            &arguments,
                            instance,
                        )
                        .map(FrameResponse::Iterator),
                    FrameRequest::DynamicCast { class, value } => self
                        .dynamic_cast(actor_class, &state.package, class, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ObjectToString { value } => self
                        .object_to_string(actor, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ResolveObject { reference } => self
                        .object_reference_value(&state.package, reference)
                        .map(FrameResponse::Value),
                    FrameRequest::GetInstance { receiver, field } => self
                        .context_field_value(actor, receiver, &state.package, field, instance)
                        .map(FrameResponse::Value),
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    } => self
                        .set_context_field(
                            actor,
                            receiver,
                            &state.package,
                            field,
                            value,
                            instance,
                            actions,
                        )
                        .map(|()| FrameResponse::Value(Value::None)),
                };
                result
                    .map(|response| {
                        if self.pending_latent.is_some() || self.state_revision(actor) != revision {
                            match response {
                                FrameResponse::Value(value) => FrameResponse::Suspend(value),
                                response => response,
                            }
                        } else {
                            response
                        }
                    })
                    .map_err(|error| error.to_string())
            });
            self.active_state_actor = previous_state_actor;
            let snapshot = frame.into_snapshot();

            if self.state_revision(actor) != revision {
                self.pending_latent = None;
                run?;
                continue;
            }

            state_frame.frame = snapshot;
            state_frame.latent = match run? {
                FrameRun::Complete(_) | FrameRun::Stopped => LatentAction::Stop,
                FrameRun::Suspended => self.pending_latent.take().unwrap_or(LatentAction::Continue),
                FrameRun::GotoLabel(label) => {
                    let label = runtime_name(&state.package, &label)
                        .map_err(|message| DispatchError::UnresolvedObject { message })?;
                    let state_name = self
                        .actor_states
                        .get(&actor)
                        .and_then(|state| state.as_deref())
                        .unwrap_or_default()
                        .to_owned();
                    if let Some((target_state, target)) =
                        self.find_state_label(actor_class, &state_name, &label)?
                    {
                        state_frame.state =
                            object_id(&target_state.package, target_state.export_index);
                        state_frame.frame = FrameSnapshot::at(target);
                        self.state_frames.insert(actor, state_frame);
                        continue;
                    }
                    LatentAction::Stop
                }
            };
            self.pending_latent = None;
            self.state_frames.insert(actor, state_frame);
            return Ok(());
        }
        Err(DispatchError::CallDepth)
    }

    fn state_revision(&self, actor: usize) -> u64 {
        self.state_revisions.get(&actor).copied().unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        self.execute_function_with_outputs(
            actor,
            actor_class,
            function,
            arguments,
            instance,
            actions,
            depth,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_function_with_outputs(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
        output_arguments: Option<&mut Vec<Value>>,
    ) -> DispatchResult<Value> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let script = self.script(function)?;
        if let ScriptMetadata::Function(metadata) = &script.metadata {
            if metadata.native_index != 0 {
                let output = self.dispatch_native_call(
                    actor,
                    actor_class,
                    &function.package,
                    metadata.native_index,
                    arguments,
                    instance,
                    actions,
                    depth,
                )?;
                if let Some(output_arguments) = output_arguments {
                    output_arguments.clear();
                    output_arguments.extend_from_slice(arguments);
                    for (index, value) in &output.outputs {
                        if let Some(argument) = output_arguments.get_mut(*index) {
                            *argument = value.clone();
                        }
                    }
                }
                return Ok(output.value);
            }
            if metadata.flags & FUNCTION_NATIVE != 0 {
                let summary = function.package.summary();
                let export = &summary.exports[function.export_index];
                let class = summary.object_name(export.outer).unwrap_or("<unknown>");
                let function_name = summary.name(export.object_name);
                if class.eq_ignore_ascii_case("Object")
                    && function_name.eq_ignore_ascii_case("Localize")
                {
                    let [
                        Value::String(section),
                        Value::String(key),
                        Value::String(package),
                    ] = arguments
                    else {
                        return Err(DispatchError::UnimplementedNamedNative {
                            class: class.to_owned(),
                            function: function_name.to_owned(),
                        });
                    };
                    return Ok(Value::String(self.packages.localize(package, section, key)));
                }
                if class.eq_ignore_ascii_case("Object")
                    && function_name.eq_ignore_ascii_case("DynamicLoadObject")
                {
                    return self.dynamic_load_object(arguments);
                }
                if class.eq_ignore_ascii_case("Actor")
                    && function_name.eq_ignore_ascii_case("GetSoundDuration")
                {
                    return self.sound_duration(arguments).map(Value::Float);
                }
                if class.eq_ignore_ascii_case("Actor")
                    && function_name.eq_ignore_ascii_case("PlayOwnedSound")
                {
                    self.play_sound(
                        actor,
                        actor_class,
                        instance,
                        "PlayOwnedSound",
                        arguments,
                        actions,
                    )
                    .map_err(|message| DispatchError::UnresolvedObject { message })?;
                    return Ok(Value::None);
                }
                if let Some(value) = named_native(class, function_name, arguments) {
                    return Ok(value);
                }
                return Err(DispatchError::UnimplementedNamedNative {
                    class: class.to_owned(),
                    function: function_name.to_owned(),
                });
            }
        }

        let mut frame = Frame::new(&script.bytecode);
        self.bind_struct_members(function, &script.bytecode, &mut frame)?;
        self.bind_frame_defaults(actor_class, &function.package, &script.bytecode, &mut frame)?;
        let argument_bindings =
            self.bind_frame_arguments(&function.package, &script, arguments, &mut frame)?;
        let result = frame
            .execute_hosted(|request| {
                let result = match request {
                    FrameRequest::Call {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_context_call(
                            actor,
                            actor_class,
                            receiver,
                            &function.package,
                            call,
                            &arguments,
                            instance,
                            actions,
                            depth + 1,
                        )
                        .map(CallOutput::into_response),
                    FrameRequest::CallIterator {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_iterator_call(
                            actor,
                            receiver,
                            &function.package,
                            call,
                            &arguments,
                            instance,
                        )
                        .map(FrameResponse::Iterator),
                    FrameRequest::DynamicCast { class, value } => self
                        .dynamic_cast(actor_class, &function.package, class, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ObjectToString { value } => self
                        .object_to_string(actor, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ResolveObject { reference } => self
                        .object_reference_value(&function.package, reference)
                        .map(FrameResponse::Value),
                    FrameRequest::GetInstance { receiver, field } => self
                        .context_field_value(actor, receiver, &function.package, field, instance)
                        .map(FrameResponse::Value),
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    } => self
                        .set_context_field(
                            actor,
                            receiver,
                            &function.package,
                            field,
                            value,
                            instance,
                            actions,
                        )
                        .map(|()| FrameResponse::Value(Value::None)),
                };
                result.map_err(|error| error.to_string())
            })
            .map_err(DispatchError::from);
        if let Some(output_arguments) = output_arguments {
            copy_output_arguments(arguments, &argument_bindings, &frame, output_arguments);
        }
        result
    }

    pub(super) fn script(&mut self, object: &ResolvedObject) -> DispatchResult<Arc<ScriptExport>> {
        let id = object_id(&object.package, object.export_index);
        if let Some(script) = self.scripts.get(&id) {
            return Ok(Arc::clone(script));
        }
        let script = Arc::new(ScriptExport::decode(&object.package, object.export_index)?);
        self.scripts.insert(id, Arc::clone(&script));
        Ok(script)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<CallOutput> {
        match call {
            FunctionCall::Native(index) => self.dispatch_native_call(
                actor,
                actor_class,
                source,
                index,
                arguments,
                instance,
                actions,
                depth,
            ),
            FunctionCall::Final(index) => {
                let Some(function) = self.resolve_reference(source, index)? else {
                    return Ok(CallOutput::value(Value::None));
                };
                let function = self.resolved_object(&function)?;
                let mut output_arguments = Vec::new();
                match self.execute_function_with_outputs(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                ) {
                    Ok(value) => Ok(CallOutput::from_arguments(
                        value,
                        arguments,
                        output_arguments,
                    )),
                    Err(error) => {
                        // ponytail: keep bootstrapping the subclass while the VM is
                        // incomplete; remove this deferral once the corpus executes.
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: error.to_string(),
                        });
                        Ok(CallOutput::value(Value::None))
                    }
                }
            }
            FunctionCall::Virtual(name) | FunctionCall::Global(name) => {
                let Some(name) = usize::try_from(name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .map(|name| source.summary().name(name).to_owned())
                else {
                    return Err(crate::Error::Call {
                        call,
                        message: "invalid function name".to_owned(),
                    }
                    .into());
                };
                let class = ResolvedObject {
                    package: Arc::clone(&actor_class.package),
                    export_index: actor_class.export_index,
                };
                let function = if matches!(call, FunctionCall::Virtual(_)) {
                    self.find_actor_function(actor, class, &name, depth)?
                } else {
                    self.find_function(class, &name, depth)?
                };
                let Some(function) = function else {
                    return Ok(CallOutput::value(Value::None));
                };
                let mut output_arguments = Vec::new();
                self.execute_function_with_outputs(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                )
                .map(|value| CallOutput::from_arguments(value, arguments, output_arguments))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_native_call(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        index: u16,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<CallOutput> {
        if index == TRACE {
            return self
                .trace_native(actor, actor_class, arguments, instance)
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(index),
                    message,
                })
                .map_err(DispatchError::from);
        }
        if index == PICK_TARGET {
            let (value, best_aim, best_dist) = self
                .pick_target(actor, arguments)
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(index),
                    message,
                })
                .map_err(DispatchError::from)?;
            return Ok(CallOutput {
                value,
                outputs: vec![(0, Value::Float(best_aim)), (1, Value::Float(best_dist))],
            });
        }
        let value = self
            .native(
                actor,
                actor_class,
                source,
                index,
                arguments,
                instance,
                actions,
                depth,
            )
            .map_err(|message| crate::Error::Call {
                call: FunctionCall::Native(index),
                message,
            })
            .map_err(DispatchError::from)?;
        let mut output_arguments = Vec::new();
        copy_native_output_arguments(index, arguments, &mut output_arguments).map_err(
            |message| crate::Error::Call {
                call: FunctionCall::Native(index),
                message,
            },
        )?;
        Ok(CallOutput::from_arguments(
            value,
            arguments,
            output_arguments,
        ))
    }

    fn trace_native(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        arguments: &[Value],
        instance: &InstanceState,
    ) -> std::result::Result<CallOutput, String> {
        let [_, _, Value::Vector(end), rest @ ..] = arguments else {
            return Err(format!(
                "Trace expects hit location, hit normal, and trace end, found {} arguments",
                arguments.len()
            ));
        };
        if rest.len() > 3 {
            return Err(format!(
                "Trace expects at most 6 arguments, found {}",
                arguments.len()
            ));
        }
        let start = match rest.first() {
            Some(Value::Vector(start)) => Vec3::from_array(*start),
            Some(Value::None) | None => {
                Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?)
            }
            Some(value) => return Err(format!("Trace start is {}", value.kind())),
        };
        let trace_actors = match rest.get(1) {
            Some(Value::Bool(trace_actors)) => *trace_actors,
            Some(Value::None) | None => false,
            Some(value) => return Err(format!("Trace actor flag is {}", value.kind())),
        };
        let extent = match rest.get(2) {
            Some(Value::Vector(extent)) => Vec3::from_array(*extent).abs(),
            Some(Value::None) | None => Vec3::ZERO,
            Some(value) => return Err(format!("Trace extent is {}", value.kind())),
        };
        let end = Vec3::from_array(*end);
        if !start.is_finite() || !end.is_finite() || !extent.is_finite() {
            return Err("Trace coordinates are not finite".to_owned());
        }
        let actor_hit = trace_actors
            .then(|| {
                self.trace_collision_actors(start, end, extent, actor, instance)
                    .map(|hits| {
                        hits.into_iter()
                            .next()
                            .map(|hit| (hit.fraction, hit.actor, hit.normal))
                    })
            })
            .transpose()?
            .flatten();
        let bsp_hit = self
            .collision
            .as_ref()
            .and_then(|collision| collision.sweep_aabb(start, end, extent))
            .map(|hit| (hit.fraction, hit.normal));
        let (value, location, normal) =
            if let Some((fraction, hit_actor, normal)) = closest_trace_hit(actor_hit, bsp_hit) {
                let hit_actor = hit_actor
                    .or(self.level_info)
                    .ok_or_else(|| "Trace hit BSP without a registered LevelInfo".to_owned())?;
                let object = self.actor_objects.get(&hit_actor).cloned().ok_or_else(|| {
                    format!("Trace hit actor {hit_actor} without object identity")
                })?;
                (
                    Value::Object(
                        self.object_handle(object)
                            .map_err(|error| error.to_string())?,
                    ),
                    start + (end - start) * fraction,
                    normal,
                )
            } else {
                (Value::Object(0), end, Vec3::ZERO)
            };
        let mut output_arguments = arguments.to_vec();
        output_arguments[0] = Value::Vector(location.to_array());
        output_arguments[1] = Value::Vector(normal.to_array());
        Ok(CallOutput::from_arguments(
            value,
            arguments,
            output_arguments,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_context_call(
        &mut self,
        current_actor: usize,
        current_class: &ResolvedObject,
        receiver: i32,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        current_instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<CallOutput> {
        if receiver == -1 {
            return self.dispatch_call(
                current_actor,
                current_class,
                source,
                call,
                arguments,
                current_instance,
                actions,
                depth,
            );
        }
        let object = self.object_for_handle(receiver)?;
        let Some(actor) = self.object_actors.get(&object).copied() else {
            let resolved = self.resolved_object(&object)?;
            let export = &resolved.package.summary().exports[resolved.export_index];
            if resolved
                .package
                .summary()
                .class_name(export)
                .is_some_and(|class| class.eq_ignore_ascii_case("Class"))
            {
                let self_handle =
                    self.object_handle(self.actor_objects.get(&current_actor).cloned().ok_or(
                        DispatchError::UnregisteredActor {
                            actor: current_actor,
                        },
                    )?)?;
                let arguments = arguments
                    .iter()
                    .map(|value| concrete_self_value(value, self_handle))
                    .collect::<Vec<_>>();
                return self.dispatch_class_context_call(
                    current_actor,
                    &resolved,
                    source,
                    call,
                    &arguments,
                    actions,
                    depth,
                );
            }
            let self_handle =
                self.object_handle(self.actor_objects.get(&current_actor).cloned().ok_or(
                    DispatchError::UnregisteredActor {
                        actor: current_actor,
                    },
                )?)?;
            let arguments = arguments
                .iter()
                .map(|value| concrete_self_value(value, self_handle))
                .collect::<Vec<_>>();
            let (class_id, mut instance) = match self.object_instances.remove(&object) {
                Some(instance) => instance,
                None => {
                    let (class, instance) = self.load_object_instance(&resolved)?;
                    (object_id(&class.package, class.export_index), instance)
                }
            };
            let class = self.resolved_object(&class_id)?;
            let result = self.dispatch_call(
                current_actor,
                &class,
                source,
                call,
                &arguments,
                &mut instance,
                actions,
                depth,
            );
            self.object_instances.insert(object, (class_id, instance));
            return result;
        };
        if actor == current_actor {
            return self.dispatch_call(
                current_actor,
                current_class,
                source,
                call,
                arguments,
                current_instance,
                actions,
                depth,
            );
        }
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::InvalidActorHandle { handle: receiver })?;
        let class = ResolvedObject {
            package: self.packages.load_path(Path::new(class.package.as_ref()))?,
            export_index: class.export_index,
        };
        let self_handle =
            self.object_handle(self.actor_objects.get(&current_actor).cloned().ok_or(
                DispatchError::UnregisteredActor {
                    actor: current_actor,
                },
            )?)?;
        let arguments = arguments
            .iter()
            .map(|value| concrete_self_value(value, self_handle))
            .collect::<Vec<_>>();
        if self.instances.contains_key(&current_actor) {
            return Err(DispatchError::ActiveActorContext {
                actor: current_actor,
            });
        }
        self.instances
            .insert(current_actor, std::mem::take(current_instance));
        let result = if let Some(mut instance) = self.instances.remove(&actor) {
            let result = self.dispatch_call(
                actor,
                &class,
                source,
                call,
                &arguments,
                &mut instance,
                actions,
                depth,
            );
            self.instances.insert(actor, instance);
            result
        } else {
            Err(DispatchError::ActiveActorContext { actor })
        };
        *current_instance =
            self.instances
                .remove(&current_actor)
                .ok_or(DispatchError::ActiveActorContext {
                    actor: current_actor,
                })?;
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_class_context_call(
        &mut self,
        current_actor: usize,
        class: &ResolvedObject,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<CallOutput> {
        let mut instance = self.load_class_defaults(class, 0)?;
        let result = match call {
            FunctionCall::Virtual(name) | FunctionCall::Global(name) => {
                let name = usize::try_from(name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .map(|name| source.summary().name(name).to_owned())
                    .ok_or_else(|| crate::Error::Call {
                        call,
                        message: "invalid function name".to_owned(),
                    })?;
                let Some(function) = self.find_function(
                    ResolvedObject {
                        package: Arc::clone(&class.package),
                        export_index: class.export_index,
                    },
                    &name,
                    depth,
                )?
                else {
                    return Ok(CallOutput::value(Value::None));
                };
                let mut output_arguments = Vec::new();
                self.execute_function_with_outputs(
                    current_actor,
                    class,
                    &function,
                    arguments,
                    &mut instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                )
                .map(|value| CallOutput::from_arguments(value, arguments, output_arguments))
            }
            _ => self.dispatch_call(
                current_actor,
                class,
                source,
                call,
                arguments,
                &mut instance,
                actions,
                depth,
            ),
        };
        self.class_defaults
            .insert(object_id(&class.package, class.export_index), instance);
        result
    }

    fn dispatch_iterator_call(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        if receiver != -1 {
            self.actor_for_handle(receiver)?;
        }
        let FunctionCall::Native(index) = call else {
            return Err(crate::Error::Call {
                call,
                message: "iterator function is not implemented".to_owned(),
            }
            .into());
        };
        if index == TRACE_ACTORS {
            return self.trace_actors_iterator(current_actor, source, arguments, current_instance);
        }
        if index != ALL_ACTORS {
            return Err(crate::Error::Call {
                call,
                message: "iterator function is not implemented".to_owned(),
            }
            .into());
        }
        let [Value::Object(base_class), Value::None, rest @ ..] = arguments else {
            return Err(crate::Error::Call {
                call,
                message: format!(
                    "AllActors expects a class and output actor, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
            .into());
        };
        if rest.len() > 1 {
            return Err(crate::Error::Call {
                call,
                message: format!(
                    "AllActors expects at most 3 arguments, found {}",
                    arguments.len()
                ),
            }
            .into());
        }
        let base_class = self
            .resolve_class_value(source, *base_class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "AllActors base class is null".to_owned(),
            })?;
        let match_tag = rest
            .first()
            .filter(|value| !matches!(value, Value::None))
            .map(|value| runtime_name(source, value))
            .transpose()
            .map_err(|message| DispatchError::UnresolvedObject { message })?
            .filter(|tag| !tag.eq_ignore_ascii_case("None"));

        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        let tag_field = match match_tag {
            Some(_) => Some(self.find_property(&base_class, "Tag", 0)?.ok_or_else(|| {
                DispatchError::UnresolvedObject {
                    message: "Actor.Tag is missing".to_owned(),
                }
            })?),
            None => None,
        };
        let mut values = Vec::new();
        for actor in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = ResolvedObject {
                package: self.packages.load_path(Path::new(class.package.as_ref()))?,
                export_index: class.export_index,
            };
            if !self.class_is_a(class, &base_class)? {
                continue;
            }
            if let (Some(match_tag), Some(field)) = (&match_tag, &tag_field) {
                let instance = if actor == current_actor {
                    current_instance
                } else {
                    self.instances
                        .get(&actor)
                        .ok_or(DispatchError::ActiveActorContext { actor })?
                };
                if !matches!(
                    instance.get(field),
                    Some(StoredValue::Name(tag)) if tag.eq_ignore_ascii_case(match_tag)
                ) {
                    continue;
                }
            }
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            values.push(IteratorValue {
                value: Value::Object(self.object_handle(object)?),
                outputs: Vec::new(),
            });
        }
        Ok(values)
    }

    fn trace_actors_iterator(
        &mut self,
        current_actor: usize,
        source: &Arc<Package>,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        let [
            Value::Object(base_class),
            _,
            _,
            _,
            Value::Vector(end),
            rest @ ..,
        ] = arguments
        else {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "TraceActors expects a class, output actor, hit vectors, and end point, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        };
        if rest.len() > 2 {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "TraceActors expects at most 7 arguments, found {}",
                    arguments.len()
                ),
            });
        }
        let base_class = self
            .resolve_class_value(source, *base_class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "TraceActors base class is null".to_owned(),
            })?;
        let current_class = self.actor_classes.get(&current_actor).cloned().ok_or(
            DispatchError::UnregisteredActor {
                actor: current_actor,
            },
        )?;
        let current_class = self.resolved_object(&current_class)?;
        let start = match rest.first() {
            Some(Value::Vector(start)) => Vec3::from_array(*start),
            Some(Value::None) | None => Vec3::from_array(
                self.actor_vector(&current_class, current_instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            ),
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("TraceActors start is {}", value.kind()),
                });
            }
        };
        let extent = match rest.get(1) {
            Some(Value::Vector(extent)) => Vec3::from_array(*extent).abs(),
            Some(Value::None) | None => Vec3::ZERO,
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("TraceActors extent is {}", value.kind()),
                });
            }
        };
        let end = Vec3::from_array(*end);
        if !start.is_finite() || !end.is_finite() || !extent.is_finite() {
            return Err(DispatchError::UnresolvedObject {
                message: "TraceActors coordinates are not finite".to_owned(),
            });
        }

        // UE1's iterator tests from End back toward Start.
        let trace_start = end;
        let trace_end = start;
        let mut hits = self
            .trace_collision_actors(
                trace_start,
                trace_end,
                extent,
                current_actor,
                current_instance,
            )
            .map_err(|message| DispatchError::UnresolvedObject { message })?
            .into_iter()
            .map(|hit| (hit.fraction, hit.actor, hit.normal))
            .collect::<Vec<_>>();
        if let Some(hit) = self
            .collision
            .as_ref()
            .and_then(|collision| collision.sweep_aabb(trace_start, trace_end, extent))
            && let Some(level) = self.level_info
        {
            hits.push((hit.fraction, level, hit.normal));
        }
        hits.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        });

        let delta = trace_end - trace_start;
        let mut values = Vec::new();
        for (fraction, actor, normal) in hits {
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            if !self.class_is_a(class, &base_class)? {
                continue;
            }
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            values.push(IteratorValue {
                value: Value::Object(self.object_handle(object)?),
                outputs: vec![
                    (
                        2,
                        Value::Vector((trace_start + delta * fraction).to_array()),
                    ),
                    (3, Value::Vector(normal.to_array())),
                ],
            });
        }
        Ok(values)
    }

    fn dynamic_cast(
        &mut self,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        class: i32,
        value: Value,
    ) -> DispatchResult<Value> {
        let value = match value {
            Value::None | Value::Object(0) => return Ok(Value::Object(0)),
            Value::Object(value) => value,
            value => {
                return Err(crate::Error::Type {
                    expected: "object",
                    actual: value.kind(),
                }
                .into());
            }
        };
        let target = match self.packages.resolve(source, object_reference(class)) {
            Ok(Some(target)) => target,
            Ok(None) => {
                return Err(DispatchError::UnresolvedObject {
                    message: "dynamic cast class is null".to_owned(),
                });
            }
            Err(ResolveError::MissingObject { class, path, .. })
                if class.eq_ignore_ascii_case("Class") =>
            {
                if value == -1 {
                    return Ok(Value::Object(0));
                }
                let object = self.object_for_handle(value)?;
                if self.object_actors.contains_key(&object) {
                    return Ok(Value::Object(0));
                }
                let object = self.resolved_object(&object)?;
                let export = &object.package.summary().exports[object.export_index];
                return Ok(
                    if object
                        .package
                        .summary()
                        .class_name(export)
                        .is_some_and(|class| class.eq_ignore_ascii_case(&path))
                    {
                        Value::Object(value)
                    } else {
                        Value::Object(0)
                    },
                );
            }
            Err(error) => return Err(error.into()),
        };
        let (value, class) = if value == -1 {
            (
                Value::Object(-1),
                ResolvedObject {
                    package: Arc::clone(&actor_class.package),
                    export_index: actor_class.export_index,
                },
            )
        } else {
            let index = usize::try_from(value - 1)
                .ok()
                .filter(|index| *index < self.handle_objects.len())
                .ok_or(DispatchError::InvalidObjectHandle { handle: value })?;
            let object = self.handle_objects[index].clone();
            let value = Value::Object(value);
            let class = if let Some(actor) = self.object_actors.get(&object)
                && let Some(class) = self.actor_classes.get(actor).cloned()
            {
                self.resolved_object(&class)?
            } else {
                let object = self.resolved_object(&object)?;
                let reference = object.package.summary().exports[object.export_index].class;
                let Some(class) = self.packages.resolve(&object.package, reference)? else {
                    return Ok(Value::Object(0));
                };
                class
            };
            (value, class)
        };

        Ok(if self.class_is_a(class, &target)? {
            value
        } else {
            Value::Object(0)
        })
    }

    fn object_to_string(&mut self, current_actor: usize, value: Value) -> DispatchResult<Value> {
        let object = match value {
            Value::None | Value::Object(0) => return Ok(Value::String("None".to_owned())),
            Value::Object(-1) => self.actor_objects.get(&current_actor).cloned().ok_or(
                DispatchError::UnregisteredActor {
                    actor: current_actor,
                },
            )?,
            Value::Object(handle) => {
                let index = usize::try_from(handle - 1)
                    .ok()
                    .filter(|index| *index < self.handle_objects.len())
                    .ok_or(DispatchError::InvalidObjectHandle { handle })?;
                self.handle_objects[index].clone()
            }
            value => {
                return Err(crate::Error::Type {
                    expected: "object",
                    actual: value.kind(),
                }
                .into());
            }
        };
        if object.package.as_ref() == "<runtime>" {
            let actor = object.export_index;
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            let summary = class.package.summary();
            let class_name = summary.name(summary.exports[class.export_index].object_name);
            return Ok(Value::String(format!("{class_name}{actor}")));
        }
        let object = self.resolved_object(&object)?;
        let summary = object.package.summary();
        let name = summary.name(summary.exports[object.export_index].object_name);
        let package = Path::new(summary.source.as_ref())
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(summary.source.as_ref());
        Ok(Value::String(format!("{package}.{name}")))
    }

    fn object_reference_value(
        &mut self,
        source: &Arc<Package>,
        reference: i32,
    ) -> DispatchResult<Value> {
        let object =
            match self.resolve_reference(source, reference) {
                Ok(Some(object)) => object,
                Ok(None) => return Ok(Value::Object(0)),
                Err(DispatchError::Resolve(ResolveError::MissingObject {
                    class, path, ..
                })) if class.eq_ignore_ascii_case("Class") => ObjectId {
                    package: Arc::from(format!("<native-class:{path}>")),
                    export_index: usize::MAX,
                },
                Err(error) => return Err(error),
            };
        self.object_handle(object).map(Value::Object)
    }

    fn dynamic_load_object(&mut self, arguments: &[Value]) -> DispatchResult<Value> {
        let [Value::String(name), Value::Object(class), rest @ ..] = arguments else {
            return Err(DispatchError::UnresolvedObject {
                message: "DynamicLoadObject expects an object name and class".to_owned(),
            });
        };
        if rest.len() > 1
            || rest
                .first()
                .is_some_and(|value| !matches!(value, Value::Bool(_) | Value::None))
        {
            return Err(DispatchError::UnresolvedObject {
                message: "DynamicLoadObject optional MayFail argument is not a bool".to_owned(),
            });
        }
        let class = self.object_for_handle(*class)?;
        let class_name = if class.export_index == usize::MAX {
            class
                .package
                .strip_prefix("<native-class:")
                .and_then(|name| name.strip_suffix('>'))
                .ok_or_else(|| DispatchError::UnresolvedObject {
                    message: "DynamicLoadObject class token is invalid".to_owned(),
                })?
                .to_owned()
        } else {
            let class = self.resolved_object(&class)?;
            class
                .package
                .summary()
                .name(class.package.summary().exports[class.export_index].object_name)
                .to_owned()
        };
        let Some(object) = self.packages.find_object(name, &class_name)? else {
            return Ok(Value::Object(0));
        };
        self.object_handle(object_id(&object.package, object.export_index))
            .map(Value::Object)
    }

    fn sound_duration(&mut self, arguments: &[Value]) -> DispatchResult<f32> {
        let [sound] = arguments else {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "GetSoundDuration expects one sound, found {} arguments",
                    arguments.len()
                ),
            });
        };
        let handle = match sound {
            Value::None | Value::Object(0) => return Ok(0.0),
            Value::Object(handle) => *handle,
            value => {
                return Err(crate::Error::Type {
                    expected: "sound object",
                    actual: value.kind(),
                }
                .into());
            }
        };
        let object = self.object_for_handle(handle)?;
        let object = self.resolved_object(&object)?;
        let mut reader = object.package.export_reader(object.export_index)?;
        while reader.next_property()?.is_some() {}
        let format = reader.read_name_index("sound format")?;
        let format = object.package.summary().name(format);
        if object.package.summary().header.version >= 63 {
            reader.read_u32()?;
        }
        let size = usize::try_from(reader.read_compact_index()?).map_err(|_| {
            DispatchError::UnresolvedObject {
                message: "sound data size is negative".to_owned(),
            }
        })?;
        let data = reader.read_bytes(size)?;
        let duration = if format.eq_ignore_ascii_case("wav") {
            wav_duration(data)
        } else if format.eq_ignore_ascii_case("mp2") {
            mpeg_layer_two_duration(data)
        } else {
            Err(format!(
                "GetSoundDuration does not support {format} sound data"
            ))
        };
        duration.map_err(|message| DispatchError::UnresolvedObject { message })
    }

    pub(super) fn class_is_a(
        &mut self,
        mut class: ResolvedObject,
        base: &ResolvedObject,
    ) -> DispatchResult<bool> {
        let base = object_id(&base.package, base.export_index);
        let key = (object_id(&class.package, class.export_index), base.clone());
        if let Some(result) = self.class_relations.get(&key) {
            return Ok(*result);
        }
        for _ in 0..MAX_CALL_DEPTH {
            if object_id(&class.package, class.export_index) == base {
                self.class_relations.insert(key, true);
                return Ok(true);
            }
            let Some(parent) = self.base_class(&class)? else {
                self.class_relations.insert(key, false);
                return Ok(false);
            };
            class = parent;
        }
        Err(DispatchError::CallDepth)
    }

    fn context_field_value(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        field: i32,
        current_instance: &InstanceState,
    ) -> DispatchResult<Value> {
        let (actor, context_object) = if receiver == -1 {
            (Some(current_actor), None)
        } else {
            let object = self.object_for_handle(receiver)?;
            (self.object_actors.get(&object).copied(), Some(object))
        };
        let Some(field) = self.resolve_reference(source, field)? else {
            return Ok(Value::None);
        };
        let Some(actor) = actor else {
            let Some(context_object) = context_object.as_ref() else {
                return Err(DispatchError::InvalidActorHandle { handle: receiver });
            };
            let object = self.resolved_object(context_object)?;
            let export = &object.package.summary().exports[object.export_index];
            let value = if export.class == ObjectReference::None {
                self.load_class_defaults(&object, 0)?.get(&field).cloned()
            } else {
                if !self.object_instances.contains_key(context_object) {
                    let (class, instance) = self.load_object_instance(&object)?;
                    self.object_instances.insert(
                        context_object.clone(),
                        (object_id(&class.package, class.export_index), instance),
                    );
                }
                self.object_instances
                    .get(context_object)
                    .and_then(|(_, instance)| instance.get(&field))
                    .cloned()
            };
            return match value {
                Some(value) => self.frame_value(&value),
                None => {
                    let field = self.resolved_object(&field)?;
                    Ok(self.zero_field_value(&field)?.unwrap_or(Value::None))
                }
            };
        };
        let intrinsic_name = {
            let field = self.resolved_object(&field)?;
            let summary = field.package.summary();
            let export = &summary.exports[field.export_index];
            summary
                .name(export.object_name)
                .eq_ignore_ascii_case("Name")
                && summary
                    .object_name(export.outer)
                    .is_some_and(|owner| owner.eq_ignore_ascii_case("Object"))
        };
        if intrinsic_name {
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            if object.package.as_ref() == "<runtime>" {
                let class = self
                    .actor_classes
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::UnregisteredActor { actor })?;
                let class = self.resolved_object(&class)?;
                let summary = class.package.summary();
                let class_name = summary.name(summary.exports[class.export_index].object_name);
                return Ok(Value::NameText(format!("{class_name}{actor}")));
            }
            let object = self.resolved_object(&object)?;
            let summary = object.package.summary();
            return Ok(Value::NameText(
                summary
                    .name(summary.exports[object.export_index].object_name)
                    .to_owned(),
            ));
        }
        let value = if actor == current_actor {
            current_instance.get(&field).cloned()
        } else {
            self.instances
                .get(&actor)
                .ok_or(DispatchError::ActiveActorContext { actor })?
                .get(&field)
                .cloned()
        };
        match value {
            Some(value) => self.frame_value(&value),
            None => {
                let field = self.resolved_object(&field)?;
                Ok(self.zero_field_value(&field)?.unwrap_or(Value::None))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn set_context_field(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        field: i32,
        value: Value,
        current_instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        let Some(field) = self.resolve_reference(source, field)? else {
            return Ok(());
        };
        let self_handle =
            self.object_handle(self.actor_objects.get(&current_actor).cloned().ok_or(
                DispatchError::UnregisteredActor {
                    actor: current_actor,
                },
            )?)?;
        let value = self.stored_value(source, &concrete_self_value(&value, self_handle))?;
        let actor = if receiver == -1 {
            current_actor
        } else {
            let object = self.object_for_handle(receiver)?;
            let Some(actor) = self.object_actors.get(&object).copied() else {
                let resolved = self.resolved_object(&object)?;
                let export = &resolved.package.summary().exports[resolved.export_index];
                if export.class == ObjectReference::None {
                    self.load_class_defaults(&resolved, 0)?;
                    self.class_defaults
                        .get_mut(&object)
                        .ok_or_else(|| DispatchError::UnresolvedObject {
                            message: format!("class defaults are missing for {object:?}"),
                        })?
                        .insert(field, value);
                } else {
                    if !self.object_instances.contains_key(&object) {
                        let (class, instance) = self.load_object_instance(&resolved)?;
                        self.object_instances.insert(
                            object.clone(),
                            (object_id(&class.package, class.export_index), instance),
                        );
                    }
                    self.object_instances
                        .get_mut(&object)
                        .ok_or_else(|| DispatchError::UnresolvedObject {
                            message: format!("object instance is missing for {object:?}"),
                        })?
                        .1
                        .insert(field, value);
                }
                return Ok(());
            };
            actor
        };
        let (is_base, is_hidden, is_pre_pivot, is_draw_type, unsupported_scene_property) = {
            let field = self.resolved_object(&field)?;
            let name = field
                .package
                .summary()
                .name(field.package.summary().exports[field.export_index].object_name);
            (
                name.eq_ignore_ascii_case("Base"),
                name.eq_ignore_ascii_case("bHidden"),
                name.eq_ignore_ascii_case("PrePivot"),
                name.eq_ignore_ascii_case("DrawType"),
                is_unsupported_scene_property(name).then(|| name.to_owned()),
            )
        };
        if is_base {
            let base = match &value {
                StoredValue::Object(base) => base.clone(),
                _ => None,
            };
            self.update_actor_base(actor, base);
        }
        let hidden = match (is_hidden, &value) {
            (true, StoredValue::Value(Value::Bool(hidden))) => Some(*hidden),
            _ => None,
        };
        let pre_pivot = match (is_pre_pivot, &value) {
            (true, StoredValue::Value(Value::Vector(pre_pivot))) => Some(*pre_pivot),
            _ => None,
        };
        let draw_type = match (is_draw_type, &value) {
            (true, StoredValue::Value(Value::Byte(draw_type))) => Some(*draw_type),
            _ => None,
        };
        if actor == current_actor {
            current_instance.insert(field.clone(), value);
            self.update_cached_collision_property(actor, &field, Some(current_instance))
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
        } else {
            self.instances
                .get_mut(&actor)
                .ok_or(DispatchError::ActiveActorContext { actor })?
                .insert(field.clone(), value);
            self.update_cached_collision_property(actor, &field, None)
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
        }
        if let Some(hidden) = hidden {
            actions.push(ActorAction::SetHidden { actor, hidden });
        }
        if let Some(pre_pivot) = pre_pivot {
            actions.push(ActorAction::SetPrePivot { actor, pre_pivot });
        }
        if let Some(draw_type) = draw_type {
            actions.push(ActorAction::SetDrawType { actor, draw_type });
        }
        if let Some(property) = unsupported_scene_property {
            actions.push(ActorAction::UnsupportedSceneProperty { actor, property });
        }
        Ok(())
    }

    pub(super) fn actor_for_handle(&self, handle: i32) -> DispatchResult<usize> {
        let object = self.object_for_handle(handle)?;
        self.object_actors
            .get(&object)
            .copied()
            .ok_or(DispatchError::InvalidActorHandle { handle })
    }

    pub(super) fn object_for_handle(&self, handle: i32) -> DispatchResult<ObjectId> {
        let index = usize::try_from(handle - 1)
            .ok()
            .filter(|index| *index < self.handle_objects.len())
            .ok_or(DispatchError::InvalidObjectHandle { handle })?;
        Ok(self.handle_objects[index].clone())
    }
}

fn is_unsupported_scene_property(name: &str) -> bool {
    [
        "AmbientGlow",
        "bCorona",
        "bLensFlare",
        "bMeshEnviroMap",
        "bNoSmooth",
        "bUnlit",
        "DrawScale",
        "Fatness",
        "LightBrightness",
        "LightEffect",
        "LightHue",
        "LightPeriod",
        "LightPhase",
        "LightRadius",
        "LightSaturation",
        "LightType",
        "LODBias",
        "Mesh",
        "MultiSkins",
        "ScaleGlow",
        "Skin",
        "Style",
        "Texture",
        "VolumeBrightness",
    ]
    .iter()
    .any(|property| name.eq_ignore_ascii_case(property))
}

pub(super) fn named_native(class: &str, function: &str, arguments: &[Value]) -> Option<Value> {
    if class.eq_ignore_ascii_case("PlayerPawn")
        && function.eq_ignore_ascii_case("ConsoleCommand")
        && matches!(arguments, [Value::String(_)])
    {
        // ponytail: the game only probes optional console values here; add
        // command routing when an in-game console exists.
        return Some(Value::String(String::new()));
    }
    if class.eq_ignore_ascii_case("Decal")
        && function.eq_ignore_ascii_case("DetachDecal")
        && arguments.is_empty()
    {
        // ponytail: decals are not rendered yet, so there is no attachment to remove.
        return Some(Value::None);
    }
    None
}

fn concrete_self_value(value: &Value, self_handle: i32) -> Value {
    match value {
        Value::Object(-1) => Value::Object(self_handle),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| concrete_self_value(value, self_handle))
                .collect(),
        ),
        Value::Struct(values) => Value::Struct(
            values
                .iter()
                .map(|(name, value)| (name.clone(), concrete_self_value(value, self_handle)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn copy_output_arguments(
    arguments: &[Value],
    bindings: &[(i32, usize, bool)],
    frame: &Frame<'_>,
    output_arguments: &mut Vec<Value>,
) {
    output_arguments.clear();
    output_arguments.extend_from_slice(arguments);
    for &(field, argument, output) in bindings {
        if !output {
            continue;
        }
        if let Some(value) = frame.local(field)
            && let Some(output) = output_arguments.get_mut(argument)
        {
            *output = value.clone();
        }
    }
}

fn copy_native_output_arguments(
    index: u16,
    arguments: &[Value],
    output_arguments: &mut Vec<Value>,
) -> std::result::Result<(), String> {
    output_arguments.clear();
    output_arguments.extend_from_slice(arguments);
    if !matches!(index, 0xe5 | 0xe6) {
        return Ok(());
    }
    let [Value::Rotator(rotation), _, _, _] = arguments else {
        return Err(format!(
            "GetAxes expects a rotator and three vectors, found {}",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let mut axes = crate::rotator_axes(*rotation);
    if index == 0xe6 {
        axes = [
            [axes[0][0], axes[1][0], axes[2][0]],
            [axes[0][1], axes[1][1], axes[2][1]],
            [axes[0][2], axes[1][2], axes[2][2]],
        ];
    }
    for (output, axis) in output_arguments[1..].iter_mut().zip(axes) {
        *output = Value::Vector(axis);
    }
    Ok(())
}

fn closest_trace_hit(
    actor: Option<(f32, usize, Vec3)>,
    bsp: Option<(f32, Vec3)>,
) -> Option<(f32, Option<usize>, Vec3)> {
    match (actor, bsp) {
        (Some((fraction, actor, normal)), Some((bsp_fraction, _))) if fraction < bsp_fraction => {
            Some((fraction, Some(actor), normal))
        }
        (_, Some((fraction, normal))) => Some((fraction, None, normal)),
        (Some((fraction, actor, normal)), None) => Some((fraction, Some(actor), normal)),
        (None, None) => None,
    }
}

pub(super) fn local_fields(bytecode: &Bytecode) -> impl Iterator<Item = i32> + '_ {
    fields(bytecode, 0x00)
}

pub(super) fn fields(bytecode: &Bytecode, opcode: u8) -> impl Iterator<Item = i32> + '_ {
    bytecode
        .tokens
        .iter()
        .filter(move |token| token.opcode == opcode)
        .filter_map(|token| {
            bytecode
                .bytes
                .get(token.offset + 1..token.offset + 5)
                .and_then(|bytes| bytes.try_into().ok())
                .map(i32::from_le_bytes)
        })
}

fn wav_duration(data: &[u8]) -> std::result::Result<f32, String> {
    if data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") || data.len() < 12 {
        return Err("sound data is not a RIFF/WAVE stream".to_owned());
    }
    let mut offset = 12;
    let mut bytes_per_second = None;
    let mut data_size = None;
    while offset + 8 <= data.len() {
        let chunk = &data[offset..offset + 4];
        let size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let Some(payload) = data.get(offset..offset.saturating_add(size)) else {
            return Err("WAV chunk exceeds sound data".to_owned());
        };
        if chunk == b"fmt " && payload.len() >= 12 {
            bytes_per_second = Some(u32::from_le_bytes(payload[8..12].try_into().unwrap()) as f32);
        } else if chunk == b"data" {
            data_size = Some(size as f32);
        }
        offset = offset.saturating_add(size).saturating_add(size & 1);
    }
    match (data_size, bytes_per_second) {
        (Some(size), Some(rate)) if rate > 0.0 => Ok(size / rate),
        _ => Err("WAV sound is missing its format or data chunk".to_owned()),
    }
}

fn mpeg_layer_two_duration(data: &[u8]) -> std::result::Result<f32, String> {
    const MPEG1_BITRATES: [u32; 16] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
    ];
    const MPEG2_BITRATES: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];

    let mut offset = 0;
    let mut duration = 0.0;
    let mut frames = 0;
    while offset + 4 <= data.len() {
        let header = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
        let version = (header >> 19) & 0x3;
        let layer = (header >> 17) & 0x3;
        let bitrate_index = ((header >> 12) & 0xf) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;
        if header >> 21 != 0x7ff
            || version == 1
            || layer != 2
            || sample_rate_index == 3
            || bitrate_index == 0
            || bitrate_index == 15
        {
            offset += 1;
            continue;
        }
        let bitrate = if version == 3 {
            MPEG1_BITRATES[bitrate_index]
        } else {
            MPEG2_BITRATES[bitrate_index]
        } * 1_000;
        let divisor = match version {
            3 => 1,
            2 => 2,
            0 => 4,
            _ => unreachable!(),
        };
        let sample_rate = SAMPLE_RATES[sample_rate_index] / divisor;
        let padding = (header >> 9) & 1;
        let frame_size = (144 * bitrate / sample_rate + padding) as usize;
        if frame_size < 4 || offset + frame_size > data.len() {
            break;
        }
        duration += 1_152.0 / sample_rate as f32;
        frames += 1;
        offset += frame_size;
    }
    (frames > 0)
        .then_some(duration)
        .ok_or_else(|| "MP2 sound has no complete Layer II frames".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_keeps_the_callers_identity_across_actor_contexts() {
        let value = Value::Array(vec![Value::Object(-1), Value::Object(7)]);
        assert_eq!(
            concrete_self_value(&value, 42),
            Value::Array(vec![Value::Object(42), Value::Object(7)])
        );
    }

    #[test]
    fn identifies_scene_properties_that_runtime_does_not_project() {
        assert!(is_unsupported_scene_property("DrawScale"));
        assert!(is_unsupported_scene_property("multiskins"));
        assert!(!is_unsupported_scene_property("DrawType"));
        assert!(!is_unsupported_scene_property("Velocity"));
    }

    #[test]
    fn returns_mutated_event_arguments_to_the_engine_host() {
        let bytecode = Bytecode {
            version: 76,
            raw_len: 1,
            bytes: vec![0x08],
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(10, Value::Vector([4.0, 5.0, 6.0]));
        frame.set_local(11, Value::Rotator([7, 8, 9]));
        let mut output = Vec::new();
        copy_output_arguments(
            &[
                Value::Object(1),
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Rotator([1, 2, 3]),
            ],
            &[(10, 1, true), (11, 2, true)],
            &frame,
            &mut output,
        );
        assert_eq!(
            output,
            [
                Value::Object(1),
                Value::Vector([4.0, 5.0, 6.0]),
                Value::Rotator([7, 8, 9]),
            ]
        );
    }

    #[test]
    fn trace_returns_the_closest_actor_or_bsp_hit() {
        let normal = Vec3::Z;
        assert_eq!(
            closest_trace_hit(Some((0.25, 7, normal)), Some((0.75, Vec3::X))),
            Some((0.25, Some(7), normal))
        );
        assert_eq!(
            closest_trace_hit(Some((0.75, 7, normal)), Some((0.25, Vec3::X))),
            Some((0.25, None, Vec3::X))
        );
    }

    #[test]
    fn reads_wav_duration_from_byte_rate_and_data_size() {
        let mut wav = b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x40\x1f\0\0\x01\0\x08\0data\x04\0\0\0\0\0\0\0".to_vec();
        assert_eq!(wav_duration(&wav).unwrap(), 4.0 / 8_000.0);
        wav[0] = 0;
        assert!(wav_duration(&wav).is_err());
    }

    #[test]
    fn reads_mp2_duration_from_layer_two_frames() {
        let header = (0x7ff << 21) | (3 << 19) | (2 << 17) | (1 << 16) | (8 << 12);
        let frame_size = 144 * 128_000 / 44_100;
        let mut mp2 = vec![0; frame_size * 2];
        mp2[..4].copy_from_slice(&u32::to_be_bytes(header));
        mp2[frame_size..frame_size + 4].copy_from_slice(&u32::to_be_bytes(header));
        assert!(
            (mpeg_layer_two_duration(&mp2).unwrap() - 2.0 * 1_152.0 / 44_100.0).abs()
                < f32::EPSILON
        );
    }
}
