use super::*;

impl ScriptRuntime {
    pub(super) fn dynamic_cast(
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

    pub(super) fn object_to_string(
        &mut self,
        current_actor: usize,
        value: Value,
    ) -> DispatchResult<Value> {
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

    pub(super) fn object_reference_value(
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

    pub(super) fn dynamic_load_object(&mut self, arguments: &[Value]) -> DispatchResult<Value> {
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

    pub(super) fn sound_duration(&mut self, arguments: &[Value]) -> DispatchResult<f32> {
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

    pub(in crate::world) fn class_is_a(
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

    pub(super) fn context_field_value(
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
    pub(super) fn set_context_field(
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

    pub(in crate::world) fn actor_for_handle(&self, handle: i32) -> DispatchResult<usize> {
        let object = self.object_for_handle(handle)?;
        self.object_actors
            .get(&object)
            .copied()
            .ok_or(DispatchError::InvalidActorHandle { handle })
    }

    pub(in crate::world) fn object_for_handle(&self, handle: i32) -> DispatchResult<ObjectId> {
        let index = usize::try_from(handle - 1)
            .ok()
            .filter(|index| *index < self.handle_objects.len())
            .ok_or(DispatchError::InvalidObjectHandle { handle })?;
        Ok(self.handle_objects[index].clone())
    }
}
