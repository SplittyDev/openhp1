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
        if index == WARP {
            return self
                .warp_native(actor_class, arguments, instance)
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(index),
                    message,
                })
                .map_err(DispatchError::from);
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

    fn warp_native(
        &mut self,
        actor_class: &ResolvedObject,
        arguments: &[Value],
        instance: &InstanceState,
    ) -> std::result::Result<CallOutput, String> {
        let [
            Value::Vector(location),
            Value::Vector(velocity),
            Value::Rotator(rotation),
        ] = arguments
        else {
            return Err(format!(
                "Warp expects location, velocity, and rotation, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let StoredValue::Value(Value::Struct(coords)) =
            self.required_actor_property(actor_class, instance, "WarpCoords")?
        else {
            return Err("WarpCoords is not a coordinate struct".to_owned());
        };
        let vector = |name| match coords.get(name) {
            Some(Value::Vector(value)) if value.iter().all(|value| value.is_finite()) => {
                Ok(Vec3::from_array(*value))
            }
            Some(value) => Err(format!("WarpCoords.{name} is {}", value.kind())),
            None => Err(format!("WarpCoords.{name} is missing")),
        };
        let origin = vector("Origin")?;
        let axes = [vector("XAxis")?, vector("YAxis")?, vector("ZAxis")?];
        let rotate = |value: Vec3| {
            Vec3::new(
                axes[0].x * value.x + axes[1].x * value.y + axes[2].x * value.z,
                axes[0].y * value.x + axes[1].y * value.y + axes[2].y * value.z,
                axes[0].z * value.x + axes[1].z * value.y + axes[2].z * value.z,
            )
        };
        let inverse_rotate =
            |value: Vec3| Vec3::new(axes[0].dot(value), axes[1].dot(value), axes[2].dot(value));
        let forward = rotate(Vec3::from_array(crate::rotator_axes(*rotation)[0]));
        let units = 65_536.0 / std::f32::consts::TAU;
        let rotation = [
            ((-forward.z).atan2(forward.x.hypot(forward.y)) * units) as i32,
            (forward.y.atan2(forward.x) * units) as i32,
            rotation[2],
        ];
        Ok(CallOutput {
            value: Value::None,
            outputs: vec![
                (
                    0,
                    Value::Vector(
                        (inverse_rotate(Vec3::from_array(*location)) + origin).to_array(),
                    ),
                ),
                (
                    1,
                    Value::Vector(inverse_rotate(Vec3::from_array(*velocity)).to_array()),
                ),
                (2, Value::Rotator(rotation)),
            ],
        })
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
        if index == RADIUS_ACTORS {
            return self.radius_actors_iterator(
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
        if index == VISIBLE_COLLIDING_ACTORS {
            return self.visible_colliding_actors_iterator(
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

    fn radius_actors_iterator(
        &mut self,
        current_actor: usize,
        source: &Arc<Package>,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        let [
            Value::Object(base_class),
            Value::None,
            Value::Float(radius),
            rest @ ..,
        ] = arguments
        else {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "RadiusActors expects a class, output actor, and radius, found {}",
                    arguments
                        .iter()
                        .map(Value::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        };
        if rest.len() > 1 {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "RadiusActors expects at most 4 arguments, found {}",
                    arguments.len()
                ),
            });
        }
        let base_class = self
            .resolve_class_value(source, *base_class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "RadiusActors base class is null".to_owned(),
            })?;
        let current_class = self.actor_classes.get(&current_actor).cloned().ok_or(
            DispatchError::UnregisteredActor {
                actor: current_actor,
            },
        )?;
        let current_class = self.resolved_object(&current_class)?;
        let location = match rest.first() {
            Some(Value::Vector(location)) => Vec3::from_array(*location),
            Some(Value::None) | None => Vec3::from_array(
                self.actor_vector(&current_class, current_instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            ),
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("RadiusActors location is {}", value.kind()),
                });
            }
        };
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
            let actor_location = Vec3::from_array(
                self.actor_vector(&class, &instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            );
            if !within_radius(Some(*radius), location, actor_location) {
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

    fn visible_colliding_actors_iterator(
        &mut self,
        current_actor: usize,
        source: &Arc<Package>,
        arguments: &[Value],
        current_instance: &InstanceState,
    ) -> DispatchResult<Vec<IteratorValue>> {
        let [Value::Object(base_class), Value::None, rest @ ..] = arguments else {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "VisibleCollidingActors expects a class and output actor, found {}",
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
                    "VisibleCollidingActors expects at most 4 arguments, found {}",
                    arguments.len()
                ),
            });
        }
        let base_class = self
            .resolve_class_value(source, *base_class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "VisibleCollidingActors base class is null".to_owned(),
            })?;
        let current_class = self.actor_classes.get(&current_actor).cloned().ok_or(
            DispatchError::UnregisteredActor {
                actor: current_actor,
            },
        )?;
        let current_class = self.resolved_object(&current_class)?;
        let radius = match rest.first() {
            Some(Value::Float(radius)) => *radius,
            Some(Value::None) | None => self
                .actor_float(&current_class, current_instance, "CollisionRadius")
                .map_err(|message| DispatchError::UnresolvedObject { message })?,
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("VisibleCollidingActors radius is {}", value.kind()),
                });
            }
        };
        let location = match rest.get(1) {
            Some(Value::Vector(location)) => Vec3::from_array(*location),
            Some(Value::None) | None => Vec3::from_array(
                self.actor_vector(&current_class, current_instance, "Location")
                    .map_err(|message| DispatchError::UnresolvedObject { message })?,
            ),
            Some(value) => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("VisibleCollidingActors location is {}", value.kind()),
                });
            }
        };
        let mut values = Vec::new();
        for actor in self
            .colliding_actors(location, radius, current_actor, current_instance)
            .map_err(|message| DispatchError::UnresolvedObject { message })?
        {
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
            if !within_radius(radius, location, actor_location)
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

fn within_radius(radius: Option<f32>, location: Vec3, actor_location: Vec3) -> bool {
    radius.is_none_or(|radius| actor_location.distance(location) <= radius)
}

#[cfg(test)]
mod iterator_tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;
    use glam::Vec3;
    use openhp1_script::Bytecode;

    static NEXT_TEST_ROOT: AtomicUsize = AtomicUsize::new(0);

    struct TestRoot(PathBuf);

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn radius_actors_test_root() -> TestRoot {
        let root = std::env::temp_dir().join(format!(
            "openhp1-radius-actors-{}-{}",
            std::process::id(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("System")).unwrap();
        fs::write(
            root.join("System/Default.ini"),
            "[Core.System]\nPaths=../*.u\n",
        )
        .unwrap();
        fs::write(
            root.join("RadiusActorsTest.u"),
            radius_actors_test_package(),
        )
        .unwrap();
        TestRoot(root)
    }

    fn radius_actors_test_package() -> Vec<u8> {
        let names = ["Core", "Class", "Base", "PlayerPawn", "PlayerPawn"];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            name_table.extend(0_u32.to_le_bytes());
        }

        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let import_table = [0, 1, 0, 0, 0, 0, 1];
        let export_offset = import_offset + import_table.len();
        let mut export_table = Vec::new();
        for (object_name, class) in [(2_u8, [0x81, 0]), (3, [0, 0]), (4, [0, 0])] {
            export_table.extend(class); // Base uses the Class import; its children use None.
            export_table.extend(0_i32.to_le_bytes());
            export_table.push(object_name);
            export_table.extend(0_u32.to_le_bytes());
            export_table.push(0); // No serial data is needed for this cache-backed test.
        }

        let mut bytes = Vec::new();
        bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        for value in [
            names.len(),
            name_offset,
            3,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            bytes.extend((value as i32).to_le_bytes());
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export_table);
        bytes
    }

    #[test]
    fn omitted_visible_actor_radius_is_unbounded() {
        let location = Vec3::ZERO;
        let actor = Vec3::new(512.0, 0.0, 0.0);
        assert!(within_radius(None, location, actor));
        assert!(!within_radius(Some(15.0), location, actor));
    }

    #[test]
    fn radius_actors_filters_and_assigns_the_output_actor() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        assert_eq!(source.summary().exports[1].class, ObjectReference::None);
        let base_class = object_id(&source, 0);
        let included_class = object_id(&source, 1);
        let excluded_class = object_id(&source, 2);
        let location = ObjectId {
            package: Arc::from("<radius-actors-test>"),
            export_index: 0,
        };
        let instance = |value| {
            [(location.clone(), StoredValue::Value(Value::Vector(value)))]
                .into_iter()
                .collect::<InstanceState>()
        };

        for class in [&included_class, &excluded_class] {
            runtime.fields.insert(
                (class.clone(), "location".to_owned()),
                Some(location.clone()),
            );
        }
        runtime
            .class_relations
            .insert((included_class.clone(), base_class.clone()), true);
        runtime
            .class_relations
            .insert((excluded_class.clone(), base_class.clone()), false);
        for (actor, class) in [
            (1, excluded_class.clone()),
            (2, included_class.clone()),
            (3, included_class.clone()),
            (4, excluded_class),
        ] {
            let object = runtime_actor_id(actor);
            runtime.object_actors.insert(object.clone(), actor);
            runtime.actor_objects.insert(actor, object);
            runtime.actor_classes.insert(actor, class);
        }
        let current_instance = instance([0.0, 0.0, 0.0]);
        runtime.instances.insert(2, instance([3.0, 4.0, 0.0]));
        runtime.instances.insert(3, instance([0.0, 0.0, 6.0]));
        runtime.instances.insert(4, instance([0.0, 0.0, 1.0]));

        let base_handle = runtime.object_handle(base_class).unwrap();
        let included_handle = runtime
            .object_handle(runtime.actor_objects.get(&2).cloned().unwrap())
            .unwrap();
        let mut bytes = vec![0x2f, 0x61, 0x36, 0x20];
        bytes.extend(1_i32.to_le_bytes());
        bytes.push(0);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(5.0_f32.to_le_bytes());
        bytes.push(0x16);
        let end_offset = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x0f, 0]);
        bytes.extend(8_i32.to_le_bytes());
        bytes.push(0);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x31);
        let iterator_pop = u16::try_from(bytes.len()).unwrap();
        bytes[end_offset..end_offset + 2].copy_from_slice(&iterator_pop.to_le_bytes());
        bytes.extend([0x30, 0x04, 0]);
        bytes.extend(8_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };

        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: 1 } => {
                        Ok(FrameResponse::Value(Value::Object(base_handle)))
                    }
                    FrameRequest::CallIterator {
                        receiver,
                        function: FunctionCall::Native(RADIUS_ACTORS),
                        arguments,
                    } => runtime
                        .dispatch_iterator_call(
                            1,
                            receiver,
                            &source,
                            FunctionCall::Native(RADIUS_ACTORS),
                            &arguments,
                            &current_instance
                        )
                        .map(FrameResponse::Iterator)
                        .map_err(|error| error.to_string()),
                    _ => panic!("unexpected frame request"),
                })
                .unwrap(),
            Value::Object(included_handle)
        );
        assert_eq!(frame.local(7), Some(&Value::Object(0)));
        assert_eq!(frame.local(8), Some(&Value::Object(included_handle)));
    }

    #[test]
    fn meta_cast_keeps_derived_class_objects() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let base_class = object_id(&source, 0);
        let included_class = object_id(&source, 1);
        let excluded_class = object_id(&source, 2);
        runtime
            .class_relations
            .insert((included_class.clone(), base_class.clone()), true);
        runtime
            .class_relations
            .insert((excluded_class.clone(), base_class), false);
        let included_handle = runtime.object_handle(included_class).unwrap();

        let mut run = |reference: i32| {
            let mut bytes = vec![0x04, 0x13];
            bytes.extend(1_i32.to_le_bytes());
            bytes.push(0x20);
            bytes.extend(reference.to_le_bytes());
            let bytecode = Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            };
            Frame::new(&bytecode).execute_hosted(|request| match request {
                FrameRequest::MetaCast { class, value } => runtime
                    .meta_cast(&source, class, value)
                    .map(FrameResponse::Value)
                    .map_err(|error| error.to_string()),
                FrameRequest::ResolveObject { reference } => runtime
                    .object_reference_value(&source, reference)
                    .map(FrameResponse::Value)
                    .map_err(|error| error.to_string()),
                _ => panic!("unexpected frame request"),
            })
        };

        assert_eq!(run(2).unwrap(), Value::Object(included_handle));
        assert_eq!(run(3).unwrap(), Value::Object(0));
    }

    #[test]
    fn warp_native_transforms_all_three_output_lvalues() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let actor_class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        let warp_coords = ObjectId {
            package: Arc::from("<warp-test>"),
            export_index: 0,
        };
        runtime.fields.insert(
            (object_id(&source, 0), "warpcoords".to_owned()),
            Some(warp_coords.clone()),
        );
        let coords = std::collections::HashMap::from([
            ("Origin".to_owned(), Value::Vector([100.0, 200.0, 300.0])),
            ("XAxis".to_owned(), Value::Vector([0.0, 1.0, 0.0])),
            ("YAxis".to_owned(), Value::Vector([-1.0, 0.0, 0.0])),
            ("ZAxis".to_owned(), Value::Vector([0.0, 0.0, 1.0])),
        ]);
        let mut instance = [(warp_coords, StoredValue::Value(Value::Struct(coords)))]
            .into_iter()
            .collect::<InstanceState>();
        let mut bytes = vec![0x61, 0x3a];
        for field in [7_i32, 8, 9] {
            bytes.push(0x00);
            bytes.extend(field.to_le_bytes());
        }
        bytes.extend([0x16, 0x04, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Vector([4.0, 5.0, 6.0]));
        frame.set_local(8, Value::Vector([1.0, 2.0, 3.0]));
        frame.set_local(9, Value::Rotator([0, 0, 42]));
        let mut actions = Vec::new();

        assert_eq!(
            frame
                .execute_hosted(|request| match request {
                    FrameRequest::Call {
                        function,
                        arguments,
                        ..
                    } => runtime
                        .dispatch_call(
                            1,
                            &actor_class,
                            &source,
                            function,
                            &arguments,
                            &mut instance,
                            &mut actions,
                            0,
                        )
                        .map(CallOutput::into_response)
                        .map_err(|error| error.to_string()),
                    _ => panic!("unexpected frame request"),
                })
                .unwrap(),
            Value::Vector([105.0, 196.0, 306.0])
        );
        assert_eq!(frame.local(8), Some(&Value::Vector([2.0, -1.0, 3.0])));
        assert_eq!(frame.local(9), Some(&Value::Rotator([0, 16_384, 42])));
    }

    #[test]
    fn visible_colliding_actors_filters_cached_collisions_and_assigns_the_output_actor() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let base_class = object_id(&source, 0);
        let included_class = object_id(&source, 1);
        let excluded_class = object_id(&source, 2);
        let fields = [
            "Location",
            "CollisionHeight",
            "CollisionRadius",
            "CollisionWidth",
            "Rotation",
            "CollideType",
            "bCollideActors",
            "bBlockActors",
            "bBlockPlayers",
            "Brush",
            "PrePivot",
            "bHidden",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name,
                ObjectId {
                    package: Arc::from("<visible-colliding-actors-test>"),
                    export_index: index,
                },
            )
        })
        .collect::<HashMap<_, _>>();
        for class in [&included_class, &excluded_class] {
            for (name, field) in &fields {
                runtime.fields.insert(
                    (class.clone(), name.to_ascii_lowercase()),
                    Some(field.clone()),
                );
            }
            runtime
                .fields
                .insert((class.clone(), "mainscale".to_owned()), None);
        }
        runtime
            .class_relations
            .insert((included_class.clone(), base_class.clone()), true);
        runtime
            .class_relations
            .insert((excluded_class.clone(), base_class.clone()), false);
        for (actor, class) in [
            (1, excluded_class.clone()),
            (2, included_class.clone()),
            (3, included_class.clone()),
            (4, excluded_class.clone()),
            (5, included_class.clone()),
        ] {
            let object = runtime_actor_id(actor);
            runtime.object_actors.insert(object.clone(), actor);
            runtime.actor_objects.insert(actor, object);
            runtime.actor_classes.insert(actor, class);
        }
        runtime.next_actor = 6;
        let instance = |location, hidden, collide_actors| {
            [
                (
                    fields["Location"].clone(),
                    StoredValue::Value(Value::Vector(location)),
                ),
                (
                    fields["CollisionHeight"].clone(),
                    StoredValue::Value(Value::Float(1.0)),
                ),
                (
                    fields["CollisionRadius"].clone(),
                    StoredValue::Value(Value::Float(1.0)),
                ),
                (
                    fields["CollisionWidth"].clone(),
                    StoredValue::Value(Value::Float(0.0)),
                ),
                (
                    fields["Rotation"].clone(),
                    StoredValue::Value(Value::Rotator([0; 3])),
                ),
                (
                    fields["CollideType"].clone(),
                    StoredValue::Value(Value::Byte(0)),
                ),
                (
                    fields["bCollideActors"].clone(),
                    StoredValue::Value(Value::Bool(collide_actors)),
                ),
                (
                    fields["bBlockActors"].clone(),
                    StoredValue::Value(Value::Bool(false)),
                ),
                (
                    fields["bBlockPlayers"].clone(),
                    StoredValue::Value(Value::Bool(false)),
                ),
                (fields["Brush"].clone(), StoredValue::Object(None)),
                (
                    fields["PrePivot"].clone(),
                    StoredValue::Value(Value::Vector([0.0; 3])),
                ),
                (
                    fields["bHidden"].clone(),
                    StoredValue::Value(Value::Bool(hidden)),
                ),
            ]
            .into_iter()
            .collect::<InstanceState>()
        };
        let current_instance = instance([0.0; 3], false, true);
        runtime
            .instances
            .insert(2, instance([3.0, 0.0, 0.0], false, true));
        runtime
            .instances
            .insert(3, instance([3.0, 0.0, 0.0], true, true));
        runtime
            .instances
            .insert(4, instance([3.0, 0.0, 0.0], false, true));
        runtime
            .instances
            .insert(5, instance([3.0, 0.0, 0.0], false, false));
        let base_handle = runtime.object_handle(base_class).unwrap();
        assert!(
            runtime
                .dispatch_iterator_call(
                    1,
                    -1,
                    &source,
                    FunctionCall::Native(VISIBLE_COLLIDING_ACTORS),
                    &[Value::Object(base_handle), Value::None, Value::Float(-1.0)],
                    &current_instance,
                )
                .unwrap()
                .is_empty()
        );
        let included_handle = runtime
            .object_handle(runtime.actor_objects.get(&2).cloned().unwrap())
            .unwrap();
        let mut bytes = vec![0x2f, 0x61, 0x38, 0x20];
        bytes.extend(1_i32.to_le_bytes());
        bytes.push(0);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(5.0_f32.to_le_bytes());
        bytes.push(0x16);
        let end_offset = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x0f, 0]);
        bytes.extend(8_i32.to_le_bytes());
        bytes.push(0);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x31);
        let iterator_pop = u16::try_from(bytes.len()).unwrap();
        bytes[end_offset..end_offset + 2].copy_from_slice(&iterator_pop.to_le_bytes());
        bytes.extend([0x30, 0x04, 0]);
        bytes.extend(8_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };

        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: 1 } => {
                        Ok(FrameResponse::Value(Value::Object(base_handle)))
                    }
                    FrameRequest::CallIterator {
                        receiver,
                        function: FunctionCall::Native(VISIBLE_COLLIDING_ACTORS),
                        arguments,
                    } => runtime
                        .dispatch_iterator_call(
                            1,
                            receiver,
                            &source,
                            FunctionCall::Native(VISIBLE_COLLIDING_ACTORS),
                            &arguments,
                            &current_instance,
                        )
                        .map(FrameResponse::Iterator)
                        .map_err(|error| error.to_string()),
                    _ => panic!("unexpected frame request"),
                })
                .unwrap(),
            Value::Object(included_handle)
        );
        assert_eq!(frame.local(7), Some(&Value::Object(0)));
        assert_eq!(frame.local(8), Some(&Value::Object(included_handle)));
    }
}
