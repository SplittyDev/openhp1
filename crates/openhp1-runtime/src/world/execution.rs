use super::native::runtime_name;
use super::*;

impl ScriptRuntime {
    pub(super) fn execute_actor_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        let mut actions = Vec::new();
        let mut instance = self.instances.remove(&actor).unwrap_or_default();
        let state_revision = self.state_revision(actor);
        let result = self.execute_function(
            actor,
            actor_class,
            function,
            arguments,
            &mut instance,
            &mut actions,
            0,
        );
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
                | LatentAction::FinishAnimation
                | LatentAction::MoveTo => {
                    self.state_frames.insert(actor, state_frame);
                    return Ok(());
                }
            }

            let state = self.resolved_object(&state_frame.state)?;
            let script = self.script(&state)?;
            let mut frame = Frame::from_snapshot(&script.bytecode, state_frame.frame);
            self.bind_struct_members(&state, &script.bytecode, &mut frame)?;
            self.bind_frame_defaults(actor_class, &state.package, &script.bytecode, &mut frame)?;
            self.bind_frame_arrays(&state.package, &script.bytecode, &mut frame)?;
            let revision = self.state_revision(actor);
            self.state_resumes = self.state_resumes.saturating_add(1);
            self.pending_latent = None;
            self.active_state_actor = Some(actor);
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
                        .map(FrameResponse::Value),
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
                    FrameRequest::GetInstance { receiver, field } => self
                        .context_field_value(actor, receiver, &state.package, field, instance)
                        .map(FrameResponse::Value),
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    } => self
                        .set_context_field(actor, receiver, &state.package, field, value, instance)
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
            self.active_state_actor = None;
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
    fn execute_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let script = self.script(function)?;
        if let ScriptMetadata::Function(metadata) = &script.metadata {
            if metadata.native_index != 0 {
                return self
                    .native(
                        actor,
                        actor_class,
                        &function.package,
                        metadata.native_index,
                        arguments,
                        instance,
                        actions,
                    )
                    .map_err(|message| crate::Error::Call {
                        call: FunctionCall::Native(metadata.native_index),
                        message,
                    })
                    .map_err(Into::into);
            }
            if metadata.flags & FUNCTION_NATIVE != 0 {
                let summary = function.package.summary();
                let export = &summary.exports[function.export_index];
                return Err(DispatchError::UnimplementedNamedNative {
                    class: summary
                        .object_name(export.outer)
                        .unwrap_or("<unknown>")
                        .to_owned(),
                    function: summary.name(export.object_name).to_owned(),
                });
            }
        }

        let mut frame = Frame::new(&script.bytecode);
        self.bind_struct_members(function, &script.bytecode, &mut frame)?;
        self.bind_frame_defaults(actor_class, &function.package, &script.bytecode, &mut frame)?;
        self.bind_frame_arguments(&function.package, &script, arguments, &mut frame)?;
        frame
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
                        .map(FrameResponse::Value),
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
                        )
                        .map(|()| FrameResponse::Value(Value::None)),
                };
                result.map_err(|error| error.to_string())
            })
            .map_err(Into::into)
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
    ) -> DispatchResult<Value> {
        match call {
            FunctionCall::Native(index) => self
                .native(
                    actor,
                    actor_class,
                    source,
                    index,
                    arguments,
                    instance,
                    actions,
                )
                .map_err(|message| crate::Error::Call { call, message }.into()),
            FunctionCall::Final(index) => {
                let reference = object_reference(index);
                let Some(function) = self.packages.resolve(source, reference)? else {
                    return Ok(Value::None);
                };
                match self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                ) {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        // ponytail: keep bootstrapping the subclass while the VM is
                        // incomplete; remove this deferral once the corpus executes.
                        actions.push(ActorAction::DeferredCall {
                            actor,
                            message: error.to_string(),
                        });
                        Ok(Value::None)
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
                    return Ok(Value::None);
                };
                self.execute_function(
                    actor,
                    actor_class,
                    &function,
                    arguments,
                    instance,
                    actions,
                    depth,
                )
            }
        }
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
    ) -> DispatchResult<Value> {
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
        let actor = self.actor_for_handle(receiver)?;
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
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = self.dispatch_call(
            actor,
            &class,
            source,
            call,
            arguments,
            &mut instance,
            actions,
            depth,
        );
        self.instances.insert(actor, instance);
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
    ) -> DispatchResult<Vec<Value>> {
        if receiver != -1 {
            self.actor_for_handle(receiver)?;
        }
        let FunctionCall::Native(ALL_ACTORS) = call else {
            return Err(crate::Error::Call {
                call,
                message: "iterator function is not implemented".to_owned(),
            }
            .into());
        };
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
            .packages
            .resolve(source, object_reference(*base_class))?
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
            values.push(Value::Object(self.object_handle(object)?));
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
        let target = self
            .packages
            .resolve(source, object_reference(class))?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "dynamic cast class is null".to_owned(),
            })?;
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

    fn class_is_a(
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
        let actor = if receiver == -1 {
            current_actor
        } else {
            self.actor_for_handle(receiver)?
        };
        let Some(field) = self.resolve_field(source, field)? else {
            return Ok(Value::None);
        };
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

    fn set_context_field(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        field: i32,
        value: Value,
        current_instance: &mut InstanceState,
    ) -> DispatchResult<()> {
        let actor = if receiver == -1 {
            current_actor
        } else {
            self.actor_for_handle(receiver)?
        };
        let Some(field) = self.resolve_field(source, field)? else {
            return Ok(());
        };
        let is_base = {
            let field = self.resolved_object(&field)?;
            field
                .package
                .summary()
                .name(field.package.summary().exports[field.export_index].object_name)
                .eq_ignore_ascii_case("Base")
        };
        let value = self.stored_value(source, &value)?;
        if is_base {
            let base = match &value {
                StoredValue::Object(base) => base.clone(),
                _ => None,
            };
            self.update_actor_base(actor, base);
        }
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
        Ok(())
    }

    fn actor_for_handle(&self, handle: i32) -> DispatchResult<usize> {
        let index = usize::try_from(handle - 1)
            .ok()
            .filter(|index| *index < self.handle_objects.len())
            .ok_or(DispatchError::InvalidObjectHandle { handle })?;
        self.object_actors
            .get(&self.handle_objects[index])
            .copied()
            .ok_or(DispatchError::InvalidActorHandle { handle })
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
