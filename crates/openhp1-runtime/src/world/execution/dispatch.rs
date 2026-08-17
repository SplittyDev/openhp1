use std::path::Path;

use openhp1_texture::texture_poly_flags;

use super::super::native::trace_texture_arguments;
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
                let arguments = portable_call_arguments(source, &function.package, arguments)
                    .map_err(|message| DispatchError::UnresolvedObject { message })?;
                let mut output_arguments = Vec::new();
                let value = self.execute_function_with_outputs(
                    actor,
                    actor_class,
                    &function,
                    &arguments,
                    instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                )?;
                Ok(CallOutput::from_arguments(
                    value,
                    &arguments,
                    output_arguments,
                ))
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
                let arguments = portable_call_arguments(source, &function.package, arguments)
                    .map_err(|message| DispatchError::UnresolvedObject { message })?;
                let mut output_arguments = Vec::new();
                self.execute_function_with_outputs(
                    actor,
                    actor_class,
                    &function,
                    &arguments,
                    instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                )
                .map(|value| CallOutput::from_arguments(value, &arguments, output_arguments))
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
        if index == FAST_TRACE {
            return self
                .fast_trace_native(actor_class, arguments, instance)
                .map(CallOutput::value)
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(index),
                    message,
                })
                .map_err(DispatchError::from);
        }
        if index == TRACE_TEXTURE {
            return self
                .trace_texture_native(arguments)
                .map_err(|message| crate::Error::Call {
                    call: FunctionCall::Native(index),
                    message,
                })
                .map_err(DispatchError::from);
        }
        if matches!(index, PICK_TARGET | PICK_ANY_TARGET) {
            let (value, best_aim, best_dist) = self
                .pick_target(
                    actor,
                    actor_class,
                    instance,
                    arguments,
                    index == PICK_TARGET,
                )
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
        if matches!(index, WARP | UNWARP) {
            return self
                .warp_native(actor_class, arguments, instance, index == UNWARP)
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
        unwarp: bool,
    ) -> std::result::Result<CallOutput, String> {
        let [
            Value::Vector(location),
            Value::Vector(velocity),
            Value::Rotator(rotation),
        ] = arguments
        else {
            return Err(format!(
                "{} expects location, velocity, and rotation, found {}",
                if unwarp { "UnWarp" } else { "Warp" },
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
        let (location, velocity, forward) = if unwarp {
            (
                rotate(Vec3::from_array(*location) - origin),
                rotate(Vec3::from_array(*velocity)),
                inverse_rotate(Vec3::from_array(crate::rotator_axes(*rotation)[0])),
            )
        } else {
            (
                inverse_rotate(Vec3::from_array(*location)) + origin,
                inverse_rotate(Vec3::from_array(*velocity)),
                rotate(Vec3::from_array(crate::rotator_axes(*rotation)[0])),
            )
        };
        let units = 65_536.0 / std::f32::consts::TAU;
        let rotation = [
            ((-forward.z).atan2(forward.x.hypot(forward.y)) * units) as i32,
            (forward.y.atan2(forward.x) * units) as i32,
            rotation[2],
        ];
        Ok(CallOutput {
            value: Value::None,
            outputs: vec![
                (0, Value::Vector(location.to_array())),
                (1, Value::Vector(velocity.to_array())),
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

    fn trace_texture_native(
        &mut self,
        arguments: &[Value],
    ) -> std::result::Result<CallOutput, String> {
        let (start, end, _trace_decals) = trace_texture_arguments(arguments)?;
        let surface = self.collision.as_ref().and_then(|collision| {
            collision
                .line_trace(Vec3::from_array(start), Vec3::from_array(end))
                .and_then(|hit| collision.surface_hit(hit.node))
        });
        let mut flags = 0;
        let value = if let Some(surface) = surface {
            flags = surface.poly_flags.bits();
            let level_package = self
                .level_package
                .clone()
                .ok_or_else(|| "TraceTexture hit BSP without a level package".to_owned())?;
            let package = self
                .packages
                .load_path(Path::new(level_package.as_ref()))
                .map_err(|error| error.to_string())?;
            match self
                .packages
                .resolve(&package, surface.texture)
                .map_err(|error| error.to_string())?
            {
                None => Value::Object(0),
                Some(texture) => {
                    flags |= texture_poly_flags(&texture.package, texture.export_index)
                        .map_err(|error| error.to_string())?;
                    Value::Object(
                        self.object_handle(object_id(&texture.package, texture.export_index))
                            .map_err(|error| error.to_string())?,
                    )
                }
            }
        } else {
            Value::Object(0)
        };
        Ok(CallOutput {
            value,
            outputs: vec![(2, Value::Int(flags as i32))],
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::world) fn dispatch_context_call(
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
        if object == host_console_id() || object == host_menu_book_id() {
            let name = match call {
                FunctionCall::Virtual(name) | FunctionCall::Global(name) => usize::try_from(name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .map(|name| source.summary().name(name)),
                _ => None,
            };
            match (name, arguments) {
                (Some(name), []) if name.eq_ignore_ascii_case("SaveSelectedSlot") => {
                    if let Some(host) = self.console_command_host.as_deref_mut() {
                        host.console_command(current_actor, "HPConsole", "SaveGame 99");
                    }
                }
                (Some(name), []) if name.eq_ignore_ascii_case("LoadSelectedSlot") => {
                    if let Some(host) = self.console_command_host.as_deref_mut() {
                        host.console_command(current_actor, "HPConsole", "open save99.usa");
                    }
                }
                (Some(name), [Value::String(url), Value::Bool(transfer_items)])
                    if object == host_console_id() && name.eq_ignore_ascii_case("ChangeLevel") =>
                {
                    actions.push(ActorAction::ClientTravel {
                        actor: current_actor,
                        url: url.clone(),
                        travel_type: 0,
                        transfer_items: *transfer_items,
                    });
                }
                (Some(name), [Value::Int(story), event_when_done])
                    if object == host_menu_book_id()
                        && name.eq_ignore_ascii_case("DoStoryBookInterlude") =>
                {
                    let event_when_done = runtime_name(source, event_when_done)
                        .map_err(|message| DispatchError::UnresolvedObject { message })?;
                    actions.push(ActorAction::StoryBookInterlude {
                        actor: current_actor,
                        story: *story,
                        event_when_done,
                    });
                }
                _ => {}
            }
            return Ok(CallOutput::value(Value::None));
        }
        if object == host_quidditch_page_id() {
            let name = match call {
                FunctionCall::Virtual(name) | FunctionCall::Global(name) => usize::try_from(name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .map(|name| source.summary().name(name)),
                _ => None,
            };
            if let (Some(name), [Value::String(unlock)]) = (name, arguments)
                && name.eq_ignore_ascii_case("UnlockQuidditch")
            {
                let level = if unlock.eq_ignore_ascii_case("Broom") {
                    Some(1)
                } else if unlock.eq_ignore_ascii_case("League") {
                    Some(2)
                } else {
                    None
                };
                if let Some(level) = level {
                    actions.push(ActorAction::UnlockQuidditch {
                        actor: current_actor,
                        level,
                    });
                }
            } else if let (Some(name), [Value::Int(team0_score), Value::Int(opponent_score)]) =
                (name, arguments)
                && name.eq_ignore_ascii_case("FinishGame")
            {
                actions.push(ActorAction::FinishQuidditchMatch {
                    actor: current_actor,
                    team0_score: *team0_score,
                    opponent_score: *opponent_score,
                });
            }
            return Ok(CallOutput::value(Value::None));
        }
        let Some(actor) = self.object_actors.get(&object).copied() else {
            let resolved = self.resolved_object(&object)?;
            let export = &resolved.package.summary().exports[resolved.export_index];
            if export.class == ObjectReference::None {
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
        let server_travel = match (call, arguments) {
            (
                FunctionCall::Virtual(name) | FunctionCall::Global(name),
                [Value::String(url), Value::Bool(transfer_items)],
            ) if usize::try_from(name)
                .ok()
                .filter(|name| *name < source.summary().names.len())
                .is_some_and(|name| {
                    source
                        .summary()
                        .name(name)
                        .eq_ignore_ascii_case("ServerTravel")
                })
                && self.class_has_name(&class, "LevelInfo")? =>
            {
                Some((url.clone(), *transfer_items))
            }
            _ => None,
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
        if result.is_ok()
            && let Some((url, transfer_items)) = server_travel
        {
            actions.push(ActorAction::ClientTravel {
                actor,
                url,
                travel_type: 0,
                transfer_items,
            });
        }
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
                let arguments = portable_call_arguments(source, &function.package, arguments)
                    .map_err(|message| DispatchError::UnresolvedObject { message })?;
                let mut output_arguments = Vec::new();
                self.execute_function_with_outputs(
                    current_actor,
                    class,
                    &function,
                    &arguments,
                    &mut instance,
                    actions,
                    depth,
                    Some(&mut output_arguments),
                )
                .map(|value| CallOutput::from_arguments(value, &arguments, output_arguments))
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
    use openhp1_map::{BspNode, BspSurface, BspVertex, Model, PolyFlags, PrimitiveBounds};
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
        let names = [
            "Core",
            "Class",
            "Base",
            "PlayerPawn",
            "PlayerPawn",
            "Texture",
            "bMasked",
            "bNotSolid",
            "USize",
            "VSize",
            "None",
            "Decal",
            "AttachDecal",
            "DetachDecal",
        ];
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
        let mut property_data = vec![6, 0x83, 7, 0x83, 8, 0x22];
        property_data.extend(32_u32.to_le_bytes());
        property_data.extend([9, 0x22]);
        property_data.extend(128_u32.to_le_bytes());
        property_data.push(10);
        let property_offset = import_offset + import_table.len();
        let export_offset = property_offset + property_data.len();
        let mut export_table = Vec::new();
        for (object_name, class) in [(2_u8, [0x81, 0]), (3, [0, 0]), (4, [0, 0])] {
            export_table.extend(class); // Base uses the Class import; its children use None.
            export_table.extend(0_i32.to_le_bytes());
            export_table.push(object_name);
            export_table.extend(0_u32.to_le_bytes());
            export_table.push(0); // No serial data is needed for this cache-backed test.
        }
        export_table.extend([0, 0]); // The synthetic texture has no class or superclass.
        export_table.extend(0_i32.to_le_bytes());
        export_table.push(5);
        export_table.extend(0_u32.to_le_bytes());
        export_table.push(property_data.len() as u8);
        export_table.extend([
            0x40 | (property_offset as u8 & 0x3f),
            (property_offset >> 6) as u8,
        ]);
        for (object_name, outer) in [(11_u8, 0_i32), (12, 5), (13, 5)] {
            export_table.extend([0, 0]);
            export_table.extend(outer.to_le_bytes());
            export_table.push(object_name);
            export_table.extend(0_u32.to_le_bytes());
            export_table.push(0);
        }

        let mut bytes = Vec::new();
        bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        for value in [
            names.len(),
            name_offset,
            7,
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
        bytes.extend(property_data);
        bytes.extend(export_table);
        bytes
    }

    fn plane_model(texture: ObjectReference, poly_flags: PolyFlags) -> Model {
        Model {
            bounds: PrimitiveBounds {
                minimum: Vec3::ZERO,
                maximum: Vec3::ZERO,
                valid: false,
                sphere: [0.0; 4],
            },
            vectors: vec![Vec3::X],
            points: vec![
                Vec3::new(0.0, -10.0, -10.0),
                Vec3::new(0.0, 10.0, -10.0),
                Vec3::new(0.0, 10.0, 10.0),
                Vec3::new(0.0, -10.0, 10.0),
            ],
            nodes: vec![BspNode {
                plane: [1.0, 0.0, 0.0, 0.0],
                zone_mask: 0,
                flags: 0,
                vertex_pool: 0,
                surface: 0,
                back: -1,
                front: -1,
                coplanar: -1,
                collision_bound: -1,
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 4,
                leaves: [0; 2],
            }],
            surfaces: vec![BspSurface {
                texture,
                poly_flags,
                base_point: 0,
                normal: 0,
                texture_u: 0,
                texture_v: 0,
                light_map: -1,
                brush_poly: -1,
                pan_u: 0,
                pan_v: 0,
                brush_actor: ObjectReference::None,
            }],
            vertices: (0..4).map(|point| BspVertex { point, side: -1 }).collect(),
            shared_side_count: 0,
            zones: Vec::new(),
            polys: ObjectReference::None,
            light_maps: Vec::new(),
            light_bits: Vec::new(),
            collision_bounds: Vec::new(),
            leaf_hulls: Vec::new(),
            leaves: Vec::new(),
            lights: Vec::new(),
            root_outside: true,
            linked: false,
        }
    }

    fn adjacent_decal_model() -> Model {
        let mut model = plane_model(ObjectReference::Export(3), PolyFlags::default());
        model.vectors.push(Vec3::new(0.8, -0.6, 0.0));
        model.points = vec![
            Vec3::new(0.0, -10.0, -5.0),
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(0.0, -10.0, 10.0),
            Vec3::new(0.0, -10.0, -15.0),
            Vec3::new(0.0, 0.0, -15.0),
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(0.0, -10.0, -5.0),
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::new(11.25, 15.0, -10.0),
            Vec3::new(11.25, 15.0, 10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ];
        model.vertices = (0..12).map(|point| BspVertex { point, side: -1 }).collect();
        model.nodes = vec![
            BspNode {
                plane: [1.0, 0.0, 0.0, 0.0],
                zone_mask: 0,
                flags: 0,
                vertex_pool: 0,
                surface: 0,
                back: -1,
                front: -1,
                coplanar: 1,
                collision_bound: -1,
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 4,
                leaves: [0; 2],
            },
            BspNode {
                plane: [1.0, 0.0, 0.0, 0.0],
                zone_mask: 0,
                flags: 0,
                vertex_pool: 4,
                surface: 1,
                back: -1,
                front: -1,
                coplanar: 2,
                collision_bound: -1,
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 4,
                leaves: [0; 2],
            },
            BspNode {
                plane: [0.8, -0.6, 0.0, 0.0],
                zone_mask: 0,
                flags: 0,
                vertex_pool: 8,
                surface: 2,
                back: -1,
                front: -1,
                coplanar: -1,
                collision_bound: -1,
                render_bound: -1,
                zones: [0; 2],
                vertex_count: 4,
                leaves: [0; 2],
            },
        ];
        model.surfaces = [0, 4, 8]
            .into_iter()
            .enumerate()
            .map(|(surface, base_point)| BspSurface {
                texture: ObjectReference::Export(3),
                poly_flags: PolyFlags::default(),
                base_point,
                normal: i32::from(surface == 2),
                texture_u: 0,
                texture_v: 0,
                light_map: -1,
                brush_poly: -1,
                pan_u: 0,
                pan_v: 0,
                brush_actor: ObjectReference::None,
            })
            .collect();
        model
    }

    fn decal_instance(
        runtime: &mut ScriptRuntime,
        source: &Arc<Package>,
        multi_decal_level: i32,
        texture: Option<ObjectId>,
    ) -> (ResolvedObject, InstanceState) {
        let class = ResolvedObject {
            package: Arc::clone(source),
            export_index: 0,
        };
        let fields = [
            "Texture",
            "MultiDecalLevel",
            "Location",
            "Rotation",
            "DrawScale",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let field = ObjectId {
                package: Arc::from("<decal-fields>"),
                export_index: index,
            };
            runtime.fields.insert(
                (object_id(source, 0), name.to_ascii_lowercase()),
                Some(field.clone()),
            );
            (name, field)
        })
        .collect::<HashMap<_, _>>();
        let instance = [
            (fields["Texture"].clone(), StoredValue::Object(texture)),
            (
                fields["MultiDecalLevel"].clone(),
                StoredValue::Value(Value::Int(multi_decal_level)),
            ),
            (
                fields["Location"].clone(),
                StoredValue::Value(Value::Vector([10.0, 0.0, 0.0])),
            ),
            (
                fields["Rotation"].clone(),
                StoredValue::Value(Value::Rotator([0, 0, 0])),
            ),
            (
                fields["DrawScale"].clone(),
                StoredValue::Value(Value::Float(1.0)),
            ),
        ]
        .into_iter()
        .collect();
        (class, instance)
    }

    fn named_decal_script(export_index: usize) -> Arc<ScriptExport> {
        Arc::new(ScriptExport {
            export_index,
            class_name: "Function".to_owned(),
            base_field: ObjectReference::None,
            next_field: ObjectReference::None,
            script_text: ObjectReference::None,
            children: ObjectReference::None,
            friendly_name: export_index,
            line: 0,
            text_position: 0,
            bytecode: Bytecode {
                version: 61,
                bytes: Vec::new(),
                raw_len: 0,
                tokens: Vec::new(),
            },
            metadata: ScriptMetadata::Function(openhp1_script::FunctionMetadata {
                parameter_size: None,
                native_index: 0,
                parameter_count: None,
                operator_precedence: 0,
                return_value_offset: None,
                flags: FUNCTION_NATIVE,
                replication_offset: None,
            }),
        })
    }

    #[test]
    fn omitted_visible_actor_radius_is_unbounded() {
        let location = Vec3::ZERO;
        let actor = Vec3::new(512.0, 0.0, 0.0);
        assert!(within_radius(None, location, actor));
        assert!(!within_radius(Some(15.0), location, actor));
    }

    #[test]
    fn class_object_context_uses_class_defaults() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let current_class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        let class = object_id(&source, 1);
        assert_eq!(source.summary().exports[1].class, ObjectReference::None);
        runtime
            .class_defaults
            .insert(class.clone(), InstanceState::default());
        let current_object = runtime_actor_id(7);
        runtime.actor_objects.insert(7, current_object.clone());
        runtime.object_actors.insert(current_object, 7);
        runtime.actor_classes.insert(7, object_id(&source, 0));
        let receiver = runtime.object_handle(class).unwrap();

        assert_eq!(
            runtime
                .dispatch_context_call(
                    7,
                    &current_class,
                    receiver,
                    &source,
                    FunctionCall::Native(RAND_RANGE),
                    &[Value::Float(2.0), Value::Float(2.0)],
                    &mut InstanceState::default(),
                    &mut Vec::new(),
                    0,
                )
                .unwrap()
                .value,
            Value::Float(2.0),
        );
    }

    #[test]
    fn destroyed_actor_context_still_answers_is_a() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        for actor in [7, 8] {
            let object = runtime_actor_id(actor);
            runtime.object_actors.insert(object.clone(), actor);
            runtime.actor_objects.insert(actor, object);
            runtime.actor_classes.insert(actor, object_id(&source, 0));
        }
        runtime.instances.insert(8, InstanceState::default());
        runtime.destroyed.insert(8);
        let receiver = runtime
            .object_handle(runtime.actor_objects.get(&8).cloned().unwrap())
            .unwrap();

        assert_eq!(
            runtime
                .dispatch_context_call(
                    7,
                    &class,
                    receiver,
                    &source,
                    FunctionCall::Native(IS_A),
                    &[Value::NameText("Base".to_owned())],
                    &mut InstanceState::default(),
                    &mut Vec::new(),
                    0,
                )
                .unwrap()
                .value,
            Value::Bool(true),
        );
    }

    #[test]
    fn fast_trace_dispatches_numeric_native_with_optional_start() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        let location = ObjectId {
            package: Arc::from("<fast-trace-test>"),
            export_index: 0,
        };
        runtime.fields.insert(
            (object_id(&source, 0), "location".to_owned()),
            Some(location.clone()),
        );
        let mut instance = [(
            location,
            StoredValue::Value(Value::Vector([10.0, 0.0, 0.0])),
        )]
        .into_iter()
        .collect::<InstanceState>();
        let model = plane_model(ObjectReference::None, PolyFlags::default());
        runtime.collision = Some(Arc::new(BspCollision::from_model(&model).unwrap()));

        assert_eq!(
            runtime
                .dispatch_native_call(
                    1,
                    &class,
                    &source,
                    FAST_TRACE,
                    &[Value::Vector([5.0, 0.0, 0.0]), Value::None],
                    &mut instance,
                    &mut Vec::new(),
                    0,
                )
                .unwrap()
                .value,
            Value::Bool(true),
        );
        assert_eq!(
            runtime
                .dispatch_native_call(
                    1,
                    &class,
                    &source,
                    FAST_TRACE,
                    &[
                        Value::Vector([5.0, 0.0, 0.0]),
                        Value::Vector([-10.0, 0.0, 0.0]),
                    ],
                    &mut instance,
                    &mut Vec::new(),
                    0,
                )
                .unwrap()
                .value,
            Value::Bool(false),
        );
    }

    #[test]
    fn trace_texture_dispatches_through_bytecode_and_writes_surface_and_texture_flags() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        let model = plane_model(ObjectReference::Export(3), PolyFlags::HIGH_LEDGE);
        runtime.collision = Some(Arc::new(BspCollision::from_model(&model).unwrap()));
        runtime.level_package = Some(Arc::clone(&source.summary().source));
        let texture = runtime.object_handle(object_id(&source, 3)).unwrap();

        let mut bytes = vec![0x04, 0x61, 0x1d, 0x23];
        for value in [-10.0_f32, 0.0, 0.0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.push(0x23);
        for value in [10.0_f32, 0.0, 0.0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.push(0x00);
        bytes.extend(7_i32.to_le_bytes());
        bytes.extend([0x27, 0x16]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Int(-1));
        let mut instance = InstanceState::default();
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
                            &class,
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
            Value::Object(texture)
        );
        assert_eq!(
            frame.local(7),
            Some(&Value::Int(
                (PolyFlags::HIGH_LEDGE.bits()
                    | PolyFlags::MASKED.bits()
                    | PolyFlags::NOT_SOLID.bits()) as i32
            ))
        );
    }

    #[test]
    fn attach_decal_retains_surface_geometry_groups_texture_and_detaches_backward() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let collision = Arc::new(
            BspCollision::from_model(&plane_model(
                ObjectReference::Export(3),
                PolyFlags::default(),
            ))
            .unwrap(),
        );
        runtime.collision = Some(Arc::clone(&collision));
        runtime.level_package = Some(Arc::clone(&source.summary().source));
        let texture = object_id(&source, 3);
        let expected = runtime.object_handle(texture.clone()).unwrap();
        let (class, mut instance) = decal_instance(&mut runtime, &source, 0, Some(texture.clone()));
        let mut actions = Vec::new();

        runtime
            .scripts
            .insert(object_id(&source, 5), named_decal_script(5));
        let attach = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 5,
        };

        assert_eq!(
            runtime
                .execute_function(
                    1,
                    &class,
                    &attach,
                    &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                    &mut instance,
                    &mut actions,
                    0,
                )
                .unwrap(),
            Value::Object(expected),
        );
        let attachment = &runtime.bsp_decal_attachments[&0][0];
        assert_eq!(attachment.actor, 1);
        assert_eq!(attachment.surface, 0);
        assert_eq!(attachment.saved_nodes, [0]);
        assert_eq!(runtime.decal_surface_list(1), [0]);
        let base = collision.surface_geometry(0).unwrap().base;
        let center = Vec3::ZERO;
        for corner in attachment.corners {
            let corner = base + Vec3::from_array(corner);
            assert!((corner.distance(center) - 32.0).abs() < 1.0e-5);
        }

        let (_, mut second_instance) =
            decal_instance(&mut runtime, &source, 0, Some(texture.clone()));
        assert_eq!(
            runtime
                .attach_decal_native(
                    2,
                    &class,
                    &mut second_instance,
                    &[Value::Float(20.0), Value::Vector(Vec3::ZERO.to_array())],
                    &mut actions,
                )
                .unwrap(),
            Value::Object(expected),
        );
        assert_eq!(runtime.bsp_decal_attachments[&0].len(), 2);
        assert_eq!(
            runtime.bsp_decal_attachments[&0]
                .iter()
                .map(|attachment| attachment.actor)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(runtime.decal_surface_list(2), [0]);

        runtime.attach_decal_to_surface(
            2,
            RuntimeObject {
                package: Arc::clone(&texture.package),
                export_index: texture.export_index,
            },
            &collision,
            collision.surface_geometry(0).unwrap(),
            [Vec3::ZERO; 4],
        );
        assert_eq!(runtime.decal_surface_list(2), [0, 0]);
        runtime
            .scripts
            .insert(object_id(&source, 6), named_decal_script(6));
        let detach = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 6,
        };
        runtime
            .execute_function(
                2,
                &class,
                &detach,
                &[],
                &mut second_instance,
                &mut actions,
                0,
            )
            .unwrap();
        assert_eq!(runtime.bsp_decal_attachments[&0].len(), 1);
        assert_eq!(runtime.bsp_decal_attachments[&0][0].actor, 1);
        assert!(runtime.decal_surface_list(2).is_empty());
        assert!(actions.is_empty());
        runtime.detach_decal_native(1);
        assert!(runtime.bsp_decal_attachments.is_empty());

        let (_, mut negative_instance) =
            decal_instance(&mut runtime, &source, -2, Some(texture.clone()));
        runtime
            .attach_decal_native(
                3,
                &class,
                &mut negative_instance,
                &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                &mut Vec::new(),
            )
            .unwrap();
        assert!(
            negative_instance
                .values()
                .any(|value| *value == StoredValue::Value(Value::Int(-2)))
        );
        assert_eq!(runtime.decal_surface_list(3), [0]);
        runtime.detach_decal_native(3);

        let (_, mut clamped_instance) = decal_instance(&mut runtime, &source, 9, Some(texture));
        runtime
            .attach_decal_native(
                4,
                &class,
                &mut clamped_instance,
                &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                &mut Vec::new(),
            )
            .unwrap();
        assert!(
            clamped_instance
                .values()
                .any(|value| *value == StoredValue::Value(Value::Int(4)))
        );
    }

    #[test]
    fn attach_decal_logs_missing_texture_and_preserves_probe_rules() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        runtime.collision = Some(Arc::new(
            BspCollision::from_model(&plane_model(
                ObjectReference::Export(3),
                PolyFlags::default(),
            ))
            .unwrap(),
        ));
        runtime.level_package = Some(Arc::clone(&source.summary().source));
        let (class, mut instance) = decal_instance(&mut runtime, &source, -2, None);
        let mut actions = Vec::new();
        assert_eq!(
            runtime
                .attach_decal_native(
                    9,
                    &class,
                    &mut instance,
                    &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                    &mut actions,
                )
                .unwrap(),
            Value::Object(0),
        );
        assert!(matches!(
            actions.as_slice(),
            [ActorAction::Log { actor: 9, message, tag: None }]
                if message == "AttachDecal: No Texture"
        ));

        for flags in [PolyFlags::AUTO_U_PAN, PolyFlags::AUTO_V_PAN] {
            runtime.collision = Some(Arc::new(
                BspCollision::from_model(&plane_model(ObjectReference::Export(3), flags)).unwrap(),
            ));
            let (_, mut instance) =
                decal_instance(&mut runtime, &source, 0, Some(object_id(&source, 3)));
            assert_eq!(
                runtime
                    .attach_decal_native(
                        10,
                        &class,
                        &mut instance,
                        &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                        &mut Vec::new(),
                    )
                    .unwrap(),
                Value::Object(0),
            );
            assert!(runtime.decal_surface_list(10).is_empty());
        }

        let corners = [Vec3::X, Vec3::Y, Vec3::NEG_X, Vec3::NEG_Y];
        for (level, expected) in [(-1, 0), (0, 0), (1, 1), (2, 4), (4, 16), (8, 16)] {
            assert_eq!(decal_probe_points(corners, level).len(), expected);
        }
        let boundary = Vec3::new(0.7, (1.0_f32 - 0.49).sqrt(), 0.0);
        assert!(!decal_neighbor_allowed(
            Vec3::X,
            boundary,
            PolyFlags::default()
        ));
        assert!(decal_neighbor_allowed(
            Vec3::X,
            Vec3::new(0.700_001, (1.0_f32 - 0.700_001_f32.powi(2)).sqrt(), 0.0),
            PolyFlags::default()
        ));
        assert!(!decal_neighbor_allowed(
            Vec3::X,
            Vec3::X,
            PolyFlags::AUTO_U_PAN
        ));
        assert!(!decal_neighbor_allowed(
            Vec3::X,
            Vec3::X,
            PolyFlags::AUTO_V_PAN
        ));

        let (random_first, random_second) = decal_basis(Vec3::X, Vec3::Y, false).unwrap();
        assert!(random_first.abs_diff_eq(Vec3::Y, 1.0e-6));
        assert!(random_second.abs_diff_eq(Vec3::NEG_Z, 1.0e-6));
        let (provided_first, provided_second) = decal_basis(Vec3::X, Vec3::Y, true).unwrap();
        assert!(provided_first.abs_diff_eq(Vec3::new(0.0, 1.0, -1.0), 1.0e-6));
        assert!(provided_second.abs_diff_eq(Vec3::new(0.0, 1.0, 1.0), 1.0e-6));
        assert!((provided_first.length() - std::f32::consts::SQRT_2).abs() < 1.0e-6);
        assert!((provided_second.length() - std::f32::consts::SQRT_2).abs() < 1.0e-6);
        let half_size = 512.0_f32.sqrt();
        let draw_scale_times_u_size = 32.0;
        for axis in [random_first, random_second] {
            assert!(
                ((axis * half_size).length() - draw_scale_times_u_size / std::f32::consts::SQRT_2)
                    .abs()
                    < 1.0e-5
            );
        }
        for axis in [provided_first, provided_second] {
            assert!(((axis * half_size).length() - draw_scale_times_u_size).abs() < 1.0e-5);
        }
    }

    #[test]
    fn attach_decal_collects_unique_adjacent_and_tilted_surfaces() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let collision = Arc::new(BspCollision::from_model(&adjacent_decal_model()).unwrap());
        runtime.collision = Some(Arc::clone(&collision));
        runtime.level_package = Some(Arc::clone(&source.summary().source));
        let texture = object_id(&source, 3);
        let (class, mut instance) = decal_instance(&mut runtime, &source, 4, Some(texture));

        runtime
            .attach_decal_native(
                5,
                &class,
                &mut instance,
                &[Value::Float(20.0), Value::Vector([0.0, 1.0, 0.0])],
                &mut Vec::new(),
            )
            .unwrap();

        let surfaces = runtime.decal_surface_list(5);
        assert_eq!(surfaces.first(), Some(&0));
        assert!(surfaces.contains(&1));
        assert!(surfaces.contains(&2), "{surfaces:?}");
        assert_eq!(
            surfaces.iter().copied().collect::<HashSet<_>>().len(),
            surfaces.len()
        );
        for &surface in surfaces {
            let geometry = collision.surface_geometry(surface).unwrap();
            let normal = geometry.normal.normalize();
            let attachment = &runtime.bsp_decal_attachments[&surface][0];
            assert!(!attachment.saved_nodes.is_empty());
            for corner in attachment.corners {
                assert!(normal.dot(Vec3::from_array(corner)).abs() < 1.0e-4);
            }
        }
        runtime.detach_decal_native(5);
        assert!(runtime.bsp_decal_attachments.is_empty());
        assert!(runtime.decal_surface_list(5).is_empty());
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
    fn high_native_zero_dispatches_through_the_runtime_host() {
        let root = radius_actors_test_root();
        let mut runtime = ScriptRuntime::new(&root.0).unwrap();
        let source = runtime.packages.load("RadiusActorsTest").unwrap();
        let actor_class = ResolvedObject {
            package: Arc::clone(&source),
            export_index: 0,
        };
        let bytecode = Bytecode {
            version: 76,
            raw_len: 8,
            bytes: vec![0x04, 0x60, 0x92, 0x2c, 20, 0x2c, 22, 0x16],
            tokens: Vec::new(),
        };
        let mut instance = InstanceState::default();
        let mut actions = Vec::new();

        assert_eq!(
            Frame::new(&bytecode)
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
            Value::Int(42)
        );
    }

    #[test]
    fn warp_and_unwarp_round_trip_all_three_output_lvalues() {
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
        let mut execute = |frame: &mut Frame| {
            frame.execute_hosted(|request| match request {
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
        };

        assert_eq!(
            execute(&mut frame).unwrap(),
            Value::Vector([105.0, 196.0, 306.0])
        );
        assert_eq!(frame.local(8), Some(&Value::Vector([2.0, -1.0, 3.0])));
        assert_eq!(frame.local(9), Some(&Value::Rotator([0, 16_384, 42])));

        let mut unwarp_bytecode = bytecode.clone();
        unwarp_bytecode.bytes[1] = 0x3b;
        let mut frame = Frame::new(&unwarp_bytecode);
        frame.set_local(7, Value::Vector([105.0, 196.0, 306.0]));
        frame.set_local(8, Value::Vector([2.0, -1.0, 3.0]));
        frame.set_local(9, Value::Rotator([0, 16_384, 42]));
        assert_eq!(execute(&mut frame).unwrap(), Value::Vector([4.0, 5.0, 6.0]));
        assert_eq!(frame.local(8), Some(&Value::Vector([1.0, 2.0, 3.0])));
        assert_eq!(frame.local(9), Some(&Value::Rotator([0, 0, 42])));
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
            "bStatic",
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
                    fields["bStatic"].clone(),
                    StoredValue::Value(Value::Bool(false)),
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
