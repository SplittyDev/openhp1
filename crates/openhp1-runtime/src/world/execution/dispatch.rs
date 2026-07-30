use super::*;

impl ScriptRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_call(
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
        // ponytail: actor calls have a registered self identity; thread the
        // receiver UObject through hosted execution before concretizing non-Actor Self.
        let actor_context = self.actor_classes.get(&actor).is_some_and(|class| {
            class == &object_id(&actor_class.package, actor_class.export_index)
        });
        let concrete_arguments = if actor_context {
            let self_handle = self.object_handle(
                self.actor_objects
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::UnregisteredActor { actor })?,
            )?;
            Some(
                arguments
                    .iter()
                    .map(|value| concrete_self_value(value, self_handle))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let arguments = concrete_arguments.as_deref().unwrap_or(arguments);
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
    pub(super) fn dispatch_native_call(
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
    pub(super) fn dispatch_context_call(
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

    pub(super) fn dispatch_iterator_call(
        &mut self,
        current_actor: usize,
        receiver: i32,
        source: &Arc<Package>,
        call: FunctionCall,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        let receiver = (receiver != -1)
            .then(|| self.actor_for_handle(receiver))
            .transpose()?;
        let receiver_instance = receiver
            .filter(|&actor| actor != current_actor)
            .map(|actor| {
                self.instances
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::ActiveActorContext { actor })
            })
            .transpose()?;
        let iterator_actor = receiver.unwrap_or(current_actor);
        let iterator_instance = receiver_instance.as_ref().unwrap_or(current_instance);
        let FunctionCall::Native(index) = call else {
            return Err(crate::Error::Call {
                call,
                message: "iterator function is not implemented".to_owned(),
            }
            .into());
        };
        if index == TRACE_ACTORS {
            return self.trace_actors_iterator(
                iterator_actor,
                source,
                arguments,
                iterator_instance,
            );
        }
        if index == VISIBLE_ACTORS {
            return self.visible_actors_iterator(
                iterator_actor,
                source,
                arguments,
                iterator_instance,
            );
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

    fn visible_actors_iterator(
        &mut self,
        current_actor: usize,
        source: &Arc<Package>,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        let [Value::Object(base_class), Value::None, rest @ ..] = arguments else {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "VisibleActors expects a class and output actor, found {}",
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
                    "VisibleActors expects at most 4 arguments, found {}",
                    arguments.len()
                ),
            });
        }
        let base_class = self
            .resolve_class_value(source, *base_class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "VisibleActors base class is null".to_owned(),
            })?;
        // HP1 omits Radius and applies its own 512/2048-unit filters.
        let radius = match rest.first() {
            Some(Value::Float(radius)) if radius.is_finite() && *radius >= 0.0 => {
                (*radius > 0.0).then_some(*radius)
            }
            Some(Value::None) | None => None,
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("VisibleActors radius is {}", value.kind()),
                });
            }
        };
        let current_class = self.actor_classes.get(&current_actor).cloned().ok_or(
            DispatchError::UnregisteredActor {
                actor: current_actor,
            },
        )?;
        let current_class = self.resolved_object(&current_class)?;
        let location = match rest.get(1) {
            Some(Value::Vector(location)) => Vec3::from_array(*location),
            Some(Value::None) | None => Vec3::from_array(
                self.actor_vector(&current_class, current_instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            ),
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("VisibleActors location is {}", value.kind()),
                });
            }
        };
        if !location.is_finite() {
            return Err(DispatchError::UnresolvedObject {
                message: "VisibleActors location is not finite".to_owned(),
            });
        }

        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
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
            let class = self.resolved_object(&class)?;
            if !self.class_is_a(
                ResolvedObject {
                    package: Arc::clone(&class.package),
                    export_index: class.export_index,
                },
                &base_class,
            )? {
                continue;
            }
            let instance = if actor == current_actor {
                current_instance.clone()
            } else {
                self.instances
                    .get(&actor)
                    .cloned()
                    .ok_or(DispatchError::ActiveActorContext { actor })?
            };
            if self
                .actor_bool(&class, &instance, "bHidden")
                .map_err(|message| DispatchError::UnresolvedObject { message })?
            {
                continue;
            }
            let actor_location = Vec3::from_array(
                self.actor_vector(&class, &instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            );
            if !within_visible_radius(radius, location, actor_location)
                || self.collision.as_ref().is_some_and(|collision| {
                    collision
                        .sweep_aabb(location, actor_location, Vec3::ZERO)
                        .is_some()
                })
            {
                continue;
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

        let trace_start = start;
        let trace_end = end;
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
}

fn within_visible_radius(radius: Option<f32>, location: Vec3, actor_location: Vec3) -> bool {
    radius.is_none_or(|radius| actor_location.distance(location) <= radius)
}

#[cfg(test)]
mod iterator_tests {
    use super::within_visible_radius;
    use glam::Vec3;

    #[test]
    fn omitted_visible_actor_radius_is_unbounded() {
        let location = Vec3::ZERO;
        let actor = Vec3::new(512.0, 0.0, 0.0);
        assert!(within_visible_radius(None, location, actor));
        assert!(!within_visible_radius(Some(15.0), location, actor));
    }
}
