use super::execution::{fields, local_fields};
use super::*;

impl ScriptRuntime {
    pub(super) fn instance_property(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> DispatchResult<Option<StoredValue>> {
        let Some(field) = self.find_property(class, name, 0)? else {
            return Ok(None);
        };
        Ok(instance.get(&field).cloned())
    }

    pub(super) fn load_class_defaults(
        &mut self,
        class: &ResolvedObject,
        depth: usize,
    ) -> DispatchResult<InstanceState> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let id = object_id(&class.package, class.export_index);
        if let Some(defaults) = self.class_defaults.get(&id) {
            return Ok(defaults.clone());
        }
        let (metadata, mut reader) = class_defaults_reader(&class.package, class.export_index)?;
        let mut defaults = match self.packages.resolve(&class.package, metadata.base_field)? {
            Some(base) => self.load_class_defaults(&base, depth + 1)?,
            None => InstanceState::new(),
        };
        self.apply_properties(class, &class.package, &mut reader, &mut defaults)?;
        self.class_defaults.insert(id, defaults.clone());
        Ok(defaults)
    }

    pub(super) fn apply_properties(
        &mut self,
        class: &ResolvedObject,
        source: &Arc<Package>,
        reader: &mut ObjectReader<'_>,
        instance: &mut InstanceState,
    ) -> DispatchResult<()> {
        while let Some(property) = reader.next_property()? {
            let name = reader.summary().name(property.name).to_owned();
            let Some(value) = self.read_property(source, reader, &property)? else {
                continue;
            };
            let Some(field) = self.find_property(class, &name, 0)? else {
                continue;
            };
            let resolved = self.resolved_object(&field)?;
            let zero = self.zero_field_value(&resolved)?.unwrap_or(Value::None);
            let zero = self.stored_value(source, &zero)?;
            if matches!(zero, StoredValue::Array(_)) {
                let index = property.array_index.unwrap_or(0);
                let stored = instance.entry(field).or_insert(zero);
                let StoredValue::Array(values) = stored else {
                    return Err(DispatchError::InvalidArrayProperty { property: name });
                };
                let length = values.len();
                let element = values
                    .get_mut(index)
                    .ok_or(DispatchError::ArrayPropertyIndex {
                        property: name,
                        index,
                        length,
                    })?;
                *element = value;
            } else {
                instance.insert(field, value);
            }
        }
        Ok(())
    }

    fn read_property(
        &mut self,
        source: &Arc<Package>,
        reader: &ObjectReader<'_>,
        property: &openhp1_package::PropertyTag,
    ) -> DispatchResult<Option<StoredValue>> {
        let mut value = reader.property_reader(property);
        Ok(Some(match property.kind {
            PropertyKind::Byte => StoredValue::Value(Value::Byte(value.read_u8()?)),
            PropertyKind::Int => StoredValue::Value(Value::Int(value.read_i32()?)),
            PropertyKind::Bool => {
                StoredValue::Value(Value::Bool(property.bool_value.unwrap_or(false)))
            }
            PropertyKind::Float => StoredValue::Value(Value::Float(value.read_f32()?)),
            PropertyKind::Object | PropertyKind::Class => {
                let reference = value.read_object_reference()?;
                match self.packages.resolve(source, reference) {
                    Ok(object) => StoredValue::Object(
                        object.map(|object| object_id(&object.package, object.export_index)),
                    ),
                    Err(error) => StoredValue::UnresolvedObject(error.to_string()),
                }
            }
            PropertyKind::Name => {
                let name = value.read_name_index("runtime name property")?;
                StoredValue::Name(value.summary().name(name).to_owned())
            }
            PropertyKind::String | PropertyKind::Str => {
                StoredValue::Value(Value::String(value.read_string()?))
            }
            PropertyKind::Vector => StoredValue::Value(Value::Vector([
                value.read_f32()?,
                value.read_f32()?,
                value.read_f32()?,
            ])),
            PropertyKind::Rotator => StoredValue::Value(Value::Rotator([
                value.read_i32()?,
                value.read_i32()?,
                value.read_i32()?,
            ])),
            PropertyKind::Struct
                if property
                    .struct_name
                    .is_some_and(|name| value.summary().name(name) == "Vector") =>
            {
                StoredValue::Value(Value::Vector([
                    value.read_f32()?,
                    value.read_f32()?,
                    value.read_f32()?,
                ]))
            }
            PropertyKind::Struct
                if property
                    .struct_name
                    .is_some_and(|name| value.summary().name(name) == "Rotator") =>
            {
                StoredValue::Value(Value::Rotator([
                    value.read_i32()?,
                    value.read_i32()?,
                    value.read_i32()?,
                ]))
            }
            _ => return Ok(None),
        }))
    }

    pub(super) fn find_property(
        &mut self,
        class: &ResolvedObject,
        name: &str,
        mut depth: usize,
    ) -> DispatchResult<Option<ObjectId>> {
        let key = (
            object_id(&class.package, class.export_index),
            name.to_ascii_lowercase(),
        );
        if let Some(field) = self.fields.get(&key) {
            return Ok(field.clone());
        }
        let mut current = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        let field = loop {
            if depth >= MAX_CALL_DEPTH {
                return Err(DispatchError::CallDepth);
            }
            if let Some(export_index) =
                current.package.summary().exports.iter().position(|export| {
                    export.outer == ObjectReference::Export(current.export_index)
                        && current
                            .package
                            .summary()
                            .class_name(export)
                            .is_some_and(|class| class.ends_with("Property"))
                        && current
                            .package
                            .summary()
                            .name(export.object_name)
                            .eq_ignore_ascii_case(name)
                })
            {
                break Some(object_id(&current.package, export_index));
            }
            let metadata = ScriptExport::decode(&current.package, current.export_index)?;
            let Some(base) = self
                .packages
                .resolve(&current.package, metadata.base_field)?
            else {
                break None;
            };
            current = base;
            depth += 1;
        };
        self.fields.insert(key, field.clone());
        Ok(field)
    }

    pub(super) fn frame_value(&mut self, value: &StoredValue) -> DispatchResult<Value> {
        Ok(match value {
            StoredValue::Value(value) => value.clone(),
            StoredValue::Array(values) => {
                let mut result = Vec::with_capacity(values.len());
                for value in values {
                    result.push(self.frame_value(value)?);
                }
                Value::Array(result)
            }
            StoredValue::Name(name) => Value::NameText(name.clone()),
            StoredValue::Object(None) => Value::Object(0),
            StoredValue::Object(Some(object)) => Value::Object(self.object_handle(object.clone())?),
            StoredValue::UnresolvedObject(message) => {
                return Err(DispatchError::UnresolvedObject {
                    message: message.clone(),
                });
            }
            StoredValue::SelfObject => Value::Object(-1),
        })
    }

    pub(super) fn stored_value(
        &self,
        source: &Arc<Package>,
        value: &Value,
    ) -> DispatchResult<StoredValue> {
        Ok(match value {
            Value::Array(values) => {
                let mut result = Vec::with_capacity(values.len());
                for value in values {
                    result.push(self.stored_value(source, value)?);
                }
                StoredValue::Array(result)
            }
            Value::Name(name) => {
                let name = usize::try_from(*name)
                    .ok()
                    .filter(|name| *name < source.summary().names.len())
                    .ok_or_else(|| DispatchError::MissingName {
                        package: Arc::clone(&source.summary().source),
                        name: format!("#{name}"),
                    })?;
                StoredValue::Name(source.summary().name(name).to_owned())
            }
            Value::NameText(name) => StoredValue::Name(name.clone()),
            Value::Object(0) => StoredValue::Object(None),
            Value::Object(-1) => StoredValue::SelfObject,
            Value::Object(handle) => {
                let index = usize::try_from(*handle - 1)
                    .ok()
                    .filter(|index| *index < self.handle_objects.len())
                    .ok_or(DispatchError::InvalidObjectHandle { handle: *handle })?;
                StoredValue::Object(Some(self.handle_objects[index].clone()))
            }
            value => StoredValue::Value(value.clone()),
        })
    }

    pub(super) fn zero_field_value(
        &mut self,
        field: &ResolvedObject,
    ) -> DispatchResult<Option<Value>> {
        let id = object_id(&field.package, field.export_index);
        if let Some(value) = self.zero_values.get(&id) {
            return Ok(value.clone());
        }
        let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
        let class = field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index])
            .unwrap_or("<unknown>");
        let value = match class {
            "ByteProperty" => Value::Byte(0),
            "IntProperty" => Value::Int(0),
            "BoolProperty" => Value::Bool(false),
            "FloatProperty" => Value::Float(0.0),
            "ObjectProperty" | "ClassProperty" => Value::Object(0),
            "NameProperty" => Value::NameText("None".to_owned()),
            "StrProperty" | "StringProperty" => Value::String(String::new()),
            "StructProperty" => {
                if let Some(struct_type) = metadata.struct_type
                    && let Some(struct_type) = self.packages.resolve(&field.package, struct_type)?
                {
                    let name = struct_type.package.summary().name(
                        struct_type.package.summary().exports[struct_type.export_index].object_name,
                    );
                    match name {
                        "Vector" => Value::Vector([0.0; 3]),
                        "Rotator" => Value::Rotator([0; 3]),
                        _ => {
                            self.zero_values.insert(id, None);
                            return Ok(None);
                        }
                    }
                } else {
                    self.zero_values.insert(id, None);
                    return Ok(None);
                }
            }
            _ => {
                self.zero_values.insert(id, None);
                return Ok(None);
            }
        };
        let dimension = usize::try_from(metadata.array_dimension).map_err(|_| {
            DispatchError::InvalidArrayDimension {
                export_index: field.export_index,
                dimension: metadata.array_dimension,
            }
        })?;
        if dimension == 0 {
            return Err(DispatchError::InvalidArrayDimension {
                export_index: field.export_index,
                dimension: metadata.array_dimension,
            });
        }
        let value = if dimension > 1 {
            Value::Array(vec![value; dimension])
        } else {
            value
        };
        self.zero_values.insert(id, Some(value.clone()));
        Ok(Some(value))
    }

    pub(super) fn object_handle(&mut self, object: ObjectId) -> DispatchResult<i32> {
        if let Some(handle) = self.object_handles.get(&object) {
            return Ok(*handle);
        }
        let handle =
            i32::try_from(self.handle_objects.len() + 1).map_err(|_| DispatchError::ObjectLimit)?;
        self.handle_objects.push(object.clone());
        self.object_handles.insert(object, handle);
        Ok(handle)
    }

    pub(super) fn resolved_object(&mut self, object: &ObjectId) -> DispatchResult<ResolvedObject> {
        Ok(ResolvedObject {
            package: self
                .packages
                .load_path(Path::new(object.package.as_ref()))?,
            export_index: object.export_index,
        })
    }

    pub(super) fn resolve_field(
        &mut self,
        source: &Arc<Package>,
        field: i32,
    ) -> DispatchResult<Option<ObjectId>> {
        let key = (Arc::clone(&source.summary().source), field);
        if let Some(resolved) = self.resolved_fields.get(&key) {
            return Ok(resolved.clone());
        }
        let resolved = self
            .packages
            .resolve(source, object_reference(field))?
            .map(|field| ObjectId {
                package: Arc::clone(&field.package.summary().source),
                export_index: field.export_index,
            });
        self.resolved_fields.insert(key, resolved.clone());
        Ok(resolved)
    }

    pub(super) fn bind_struct_members(
        &mut self,
        function: &ResolvedObject,
        bytecode: &Bytecode,
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        let key = object_id(&function.package, function.export_index);
        let members = if let Some(members) = self.struct_members.get(&key) {
            Arc::clone(members)
        } else {
            let mut members = Vec::new();
            let mut seen = HashSet::new();
            for field in fields(bytecode, 0x36) {
                if !seen.insert(field) {
                    continue;
                }
                let Some(resolved) = self
                    .packages
                    .resolve(&function.package, object_reference(field))?
                else {
                    continue;
                };
                let summary = resolved.package.summary();
                let export = &summary.exports[resolved.export_index];
                let Some(owner) = summary.object_name(export.outer) else {
                    continue;
                };
                let name = summary.name(export.object_name);
                let member = match (owner, name) {
                    ("Vector", "X") => StructMember::X,
                    ("Vector", "Y") => StructMember::Y,
                    ("Vector", "Z") => StructMember::Z,
                    ("Rotator", "Pitch") => StructMember::Pitch,
                    ("Rotator", "Yaw") => StructMember::Yaw,
                    ("Rotator", "Roll") => StructMember::Roll,
                    _ => continue,
                };
                members.push((field, member));
            }
            let members = Arc::new(members);
            self.struct_members.insert(key, Arc::clone(&members));
            members
        };
        for &(field, member) in members.iter() {
            frame.set_struct_member(field, member);
        }
        Ok(())
    }

    pub(super) fn bind_frame_arguments(
        &mut self,
        source: &Arc<Package>,
        function: &ScriptExport,
        arguments: &[Value],
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        let key = ObjectId {
            package: Arc::clone(&source.summary().source),
            export_index: function.export_index,
        };
        let bindings = if let Some(bindings) = self.frame_arguments.get(&key) {
            Arc::clone(bindings)
        } else {
            let parameters = self.function_parameters(source, function.children)?;
            let mut bindings = Vec::new();
            for field in local_fields(&function.bytecode) {
                let Some(id) = self.resolve_field(source, field)? else {
                    continue;
                };
                if let Some(argument) = parameters.iter().position(|parameter| *parameter == id) {
                    bindings.push((field, argument));
                }
            }
            let bindings = Arc::new(bindings);
            self.frame_arguments.insert(key, Arc::clone(&bindings));
            bindings
        };
        for &(field, argument) in bindings.iter() {
            if let Some(value) = arguments.get(argument) {
                frame.set_local(field, value.clone());
            }
        }
        self.bind_frame_arrays(source, &function.bytecode, frame)?;
        Ok(())
    }

    pub(super) fn bind_frame_arrays(
        &mut self,
        source: &Arc<Package>,
        bytecode: &Bytecode,
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        for field in local_fields(bytecode) {
            if frame.local(field).is_some() {
                continue;
            }
            let Some(field_object) = self.resolve_field(source, field)? else {
                continue;
            };
            let resolved = self.resolved_object(&field_object)?;
            if let Some(value @ Value::Array(_)) = self.zero_field_value(&resolved)? {
                frame.set_local(field, value);
            }
        }
        Ok(())
    }

    fn function_parameters(
        &mut self,
        source: &Arc<Package>,
        mut field: ObjectReference,
    ) -> DispatchResult<Vec<ObjectId>> {
        let mut parameters = Vec::new();
        let mut field_source = Arc::clone(source);
        for _ in 0..MAX_CALL_DEPTH {
            let Some(resolved) = self.packages.resolve(&field_source, field)? else {
                return Ok(parameters);
            };
            let metadata = PropertyMetadata::decode(&resolved.package, resolved.export_index)?;
            if metadata.flags & PROPERTY_PARAMETER != 0 && metadata.flags & PROPERTY_RETURN == 0 {
                parameters.push(ObjectId {
                    package: Arc::clone(&resolved.package.summary().source),
                    export_index: resolved.export_index,
                });
            }
            field = metadata.next_field;
            field_source = resolved.package;
            if field == ObjectReference::None {
                return Ok(parameters);
            }
        }
        Err(DispatchError::CallDepth)
    }
}
