use super::execution::{fields, local_fields};
use super::*;

fn ini_string(value: &str) -> Option<String> {
    (!value.contains(['\r', '\n'])
        && value.trim() == value
        && !(value.starts_with('"') && value.ends_with('"')))
    .then(|| value.to_owned())
}

fn config_component<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    value
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?
        .split(',')
        .find_map(|component| {
            let (key, value) = component.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
}

struct ConfigProperty {
    declaring: ObjectId,
    field: ObjectId,
    name: String,
    metadata: PropertyMetadata,
}

impl ScriptRuntime {
    pub(super) fn load_object_instance(
        &mut self,
        object: &ResolvedObject,
    ) -> DispatchResult<(ResolvedObject, InstanceState)> {
        let export = object
            .package
            .summary()
            .exports
            .get(object.export_index)
            .ok_or(openhp1_package::Error::InvalidExportIndex {
                package: Arc::clone(&object.package.summary().source),
                index: object.export_index,
                export_count: object.package.summary().exports.len(),
            })?;
        let class = self
            .packages
            .resolve(&object.package, export.class)?
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: format!("object export {} has no class", object.export_index),
            })?;
        let mut instance = self.load_class_defaults(&class, 0)?;
        let mut reader = object.package.export_reader(object.export_index)?;
        reader.read_object_stack(export.object_flags)?;
        self.apply_properties(&class, &object.package, &mut reader, &mut instance)?;
        Ok((class, instance))
    }

    pub(super) fn instance_property(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> DispatchResult<Option<StoredValue>> {
        let Some(field) = self.find_property(class, name, 0)? else {
            return Ok(None);
        };
        if let Some(value) = instance.get(&field) {
            return Ok(Some(value.clone()));
        }
        self.default_field_value(&field)
    }

    pub(super) fn default_field_value(
        &mut self,
        field: &ObjectId,
    ) -> DispatchResult<Option<StoredValue>> {
        let field = self.resolved_object(field)?;
        self.zero_field_value(&field)?
            .map(|value| self.stored_value(&field.package, &value))
            .transpose()
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
            None => InstanceState::default(),
        };
        self.apply_properties(class, &class.package, &mut reader, &mut defaults)?;
        self.apply_config_defaults(class, &mut defaults)?;
        self.class_defaults.insert(id, defaults.clone());
        Ok(defaults)
    }

    fn apply_config_defaults(
        &mut self,
        class: &ResolvedObject,
        defaults: &mut InstanceState,
    ) -> DispatchResult<()> {
        for property in self.config_properties(class)? {
            let declaring = self.resolved_object(&property.declaring)?;
            let target = if property.metadata.flags & PROPERTY_GLOBAL_CONFIG != 0 {
                self.config_target(&declaring)?
            } else {
                self.config_target(class)?
            };
            self.apply_config_property(&property, &target, defaults)?;
        }
        Ok(())
    }

    fn apply_config_property(
        &mut self,
        property: &ConfigProperty,
        target: &(String, String),
        defaults: &mut InstanceState,
    ) -> DispatchResult<()> {
        let dimension = usize::try_from(property.metadata.array_dimension).map_err(|_| {
            DispatchError::InvalidArrayDimension {
                export_index: property.field.export_index,
                dimension: property.metadata.array_dimension,
            }
        })?;
        if dimension == 0 {
            return Err(DispatchError::InvalidArrayDimension {
                export_index: property.field.export_index,
                dimension: property.metadata.array_dimension,
            });
        }
        if dimension > 1 {
            let mut value = defaults
                .get(&property.field)
                .cloned()
                .or(self.default_field_value(&property.field)?)
                .ok_or_else(|| DispatchError::InvalidConfigValue {
                    property: property.name.clone(),
                    message: "the property has no serializable default".to_owned(),
                })?;
            let StoredValue::Array(values) = &mut value else {
                return Err(DispatchError::InvalidArrayProperty {
                    property: property.name.clone(),
                });
            };
            for (index, element) in values.iter_mut().enumerate() {
                let key = format!("{}[{index}]", property.name);
                let Some(value) = self
                    .packages
                    .config_values(&target.0, &target.1, &key)
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                let field = self.resolved_object(&property.field)?;
                let Some(parsed) = self.config_value_from_text(&field, &property.name, &value)?
                else {
                    continue;
                };
                *element = parsed;
            }
            defaults.insert(property.field.clone(), value);
            return Ok(());
        }

        let values = self
            .packages
            .config_values(&target.0, &target.1, &property.name);
        if values.is_empty() {
            return Ok(());
        }
        let field = self.resolved_object(&property.field)?;
        if field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index])
            == Some("ArrayProperty")
        {
            let Some(inner) = property.metadata.inner_type else {
                return Err(DispatchError::MissingArrayInner {
                    property: property.name.clone(),
                });
            };
            let Some(inner) = self.packages.resolve(&field.package, inner)? else {
                return Err(DispatchError::MissingArrayInner {
                    property: property.name.clone(),
                });
            };
            let mut parsed = Vec::with_capacity(values.len());
            for value in values {
                let Some(value) = self.config_value_from_text(&inner, &property.name, &value)?
                else {
                    return Ok(());
                };
                parsed.push(value);
            }
            defaults.insert(property.field.clone(), StoredValue::Array(parsed));
            return Ok(());
        }
        let Some(value) = self.config_value_from_text(&field, &property.name, &values[0])? else {
            return Ok(());
        };
        defaults.insert(property.field.clone(), value);
        Ok(())
    }

    fn config_value_from_text(
        &mut self,
        field: &ResolvedObject,
        property: &str,
        text: &str,
    ) -> DispatchResult<Option<StoredValue>> {
        let invalid = |message| DispatchError::InvalidConfigValue {
            property: property.to_owned(),
            message,
        };
        let class = field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index])
            .unwrap_or("<unknown>");
        let value = match class {
            "ByteProperty" => {
                let value = match text.trim().parse() {
                    Ok(value) => value,
                    Err(_) => self
                        .enum_names(field)?
                        .and_then(|names| {
                            names
                                .iter()
                                .position(|name| name.eq_ignore_ascii_case(text.trim()))
                                .and_then(|index| u8::try_from(index).ok())
                        })
                        .ok_or_else(|| invalid(format!("`{text}` is not a byte or enum value")))?,
                };
                StoredValue::Value(Value::Byte(value))
            }
            "IntProperty" => StoredValue::Value(Value::Int(
                text.trim()
                    .parse()
                    .map_err(|_| invalid(format!("`{text}` is not an integer")))?,
            )),
            "BoolProperty" => StoredValue::Value(Value::Bool(
                match text.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" => true,
                    "0" | "false" | "no" => false,
                    _ => return Err(invalid(format!("`{text}` is not a boolean"))),
                },
            )),
            "FloatProperty" => {
                let value: f32 = text
                    .trim()
                    .parse()
                    .map_err(|_| invalid(format!("`{text}` is not a float")))?;
                if !value.is_finite() {
                    return Err(invalid("the float is not finite".to_owned()));
                }
                StoredValue::Value(Value::Float(value))
            }
            "NameProperty" => StoredValue::Name(ini_string(text).ok_or_else(|| {
                invalid("the name cannot round-trip through an INI value".to_owned())
            })?),
            "StrProperty" | "StringProperty" => {
                StoredValue::Value(Value::String(ini_string(text).ok_or_else(|| {
                    invalid("the string cannot round-trip through an INI value".to_owned())
                })?))
            }
            "ObjectProperty" | "ClassProperty" if text.trim().eq_ignore_ascii_case("None") => {
                StoredValue::Object(None)
            }
            "ObjectProperty" | "ClassProperty" => {
                let object = self.packages.find_object_any(text.trim())?.ok_or_else(|| {
                    invalid(format!("`{text}` does not resolve to a package object"))
                })?;
                if class == "ClassProperty"
                    && object.package.summary().exports[object.export_index].class
                        != ObjectReference::None
                {
                    return Err(invalid(format!("`{text}` is not a class object")));
                }
                StoredValue::Object(Some(object_id(&object.package, object.export_index)))
            }
            "StructProperty" => {
                let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
                let Some(structure) = metadata.struct_type else {
                    return Ok(None);
                };
                let Some(structure) = self.packages.resolve(&field.package, structure)? else {
                    return Ok(None);
                };
                let name = structure
                    .package
                    .summary()
                    .name(structure.package.summary().exports[structure.export_index].object_name);
                match name {
                    "Vector" => {
                        let value = [
                            self.config_component(text, property, "X")?,
                            self.config_component(text, property, "Y")?,
                            self.config_component(text, property, "Z")?,
                        ];
                        if !value.iter().all(|value: &f32| value.is_finite()) {
                            return Err(invalid(
                                "the vector has a non-finite component".to_owned(),
                            ));
                        }
                        StoredValue::Value(Value::Vector(value))
                    }
                    "Rotator" => StoredValue::Value(Value::Rotator([
                        self.config_component(text, property, "Pitch")?,
                        self.config_component(text, property, "Yaw")?,
                        self.config_component(text, property, "Roll")?,
                    ])),
                    "Color" => {
                        let mut values = std::collections::HashMap::new();
                        for component in ["R", "G", "B", "A"] {
                            values.insert(
                                component.to_owned(),
                                Value::Byte(self.config_component(text, property, component)?),
                            );
                        }
                        StoredValue::Value(Value::Struct(values))
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn config_component<T: std::str::FromStr>(
        &self,
        text: &str,
        property: &str,
        name: &str,
    ) -> DispatchResult<T> {
        config_component(text, name)
            .ok_or_else(|| DispatchError::InvalidConfigValue {
                property: property.to_owned(),
                message: format!("the struct has no `{name}` component"),
            })?
            .parse()
            .map_err(|_| DispatchError::InvalidConfigValue {
                property: property.to_owned(),
                message: format!("the struct `{name}` component is invalid"),
            })
    }

    fn enum_names(&mut self, field: &ResolvedObject) -> DispatchResult<Option<Vec<String>>> {
        let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
        let Some(enumeration) = metadata.enum_type else {
            return Ok(None);
        };
        let Some(enumeration) = self.packages.resolve(&field.package, enumeration)? else {
            return Ok(None);
        };
        Ok(Some(enum_names(
            &enumeration.package,
            enumeration.export_index,
        )?))
    }

    pub(super) fn save_config(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
    ) -> DispatchResult<()> {
        let script = self.script(class)?;
        let ScriptMetadata::Class(metadata) = &script.metadata else {
            return Err(DispatchError::InvalidClass {
                export_index: class.export_index,
            });
        };
        if metadata.flags & CLASS_CONFIG == 0 {
            return Ok(());
        }

        let mut writes: Vec<(String, String, Vec<ConfigEntry>)> = Vec::new();
        for property in self.config_properties(class)? {
            let value = instance
                .get(&property.field)
                .cloned()
                .or(self.default_field_value(&property.field)?)
                .ok_or_else(|| DispatchError::InvalidConfigValue {
                    property: property.name.clone(),
                    message: "the property has no serializable default".to_owned(),
                })?;
            let declaring = self.resolved_object(&property.declaring)?;
            let target = if property.metadata.flags & PROPERTY_GLOBAL_CONFIG != 0 {
                self.config_target(&declaring)?
            } else {
                self.config_target(class)?
            };
            let mut entries = self.config_entries(&property, &value)?;
            for entry in &mut entries {
                entry.section.clone_from(&target.1);
            }
            if let Some((_, _, saved)) = writes.iter_mut().find(|(config, section, _)| {
                config.eq_ignore_ascii_case(&target.0) && section.eq_ignore_ascii_case(&target.1)
            }) {
                saved.extend(entries);
            } else {
                writes.push((target.0, target.1, entries));
            }
        }
        for (config, _, entries) in writes {
            self.packages.save_config(&config, &entries)?;
        }
        self.refresh_config_defaults()?;
        Ok(())
    }

    fn config_properties(&mut self, class: &ResolvedObject) -> DispatchResult<Vec<ConfigProperty>> {
        let mut properties = Vec::new();
        let mut seen = HashSet::default();
        let mut current = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        for _ in 0..MAX_CALL_DEPTH {
            let script = self.script(&current)?;
            let base = script.base_field;
            let declaring = object_id(&current.package, current.export_index);
            for (export_index, export) in current.package.summary().exports.iter().enumerate() {
                if export.outer != ObjectReference::Export(current.export_index)
                    || !current
                        .package
                        .summary()
                        .class_name(export)
                        .is_some_and(|name| name.ends_with("Property"))
                {
                    continue;
                }
                let name = current
                    .package
                    .summary()
                    .name(export.object_name)
                    .to_owned();
                if !seen.insert(name.to_ascii_lowercase()) {
                    continue;
                }
                let metadata = PropertyMetadata::decode(&current.package, export_index)?;
                if metadata.flags & (PROPERTY_CONFIG | PROPERTY_GLOBAL_CONFIG) == 0 {
                    continue;
                }
                properties.push(ConfigProperty {
                    declaring: declaring.clone(),
                    field: ObjectId {
                        package: Arc::clone(&current.package.summary().source),
                        export_index,
                    },
                    name,
                    metadata,
                });
            }
            let Some(base) = self.packages.resolve(&current.package, base)? else {
                break;
            };
            current = base;
        }
        Ok(properties)
    }

    fn refresh_config_defaults(&mut self) -> DispatchResult<()> {
        let cached = self.class_defaults.keys().cloned().collect::<Vec<_>>();
        self.class_defaults.clear();
        for class in cached {
            let class = self.resolved_object(&class)?;
            self.load_class_defaults(&class, 0)?;
        }
        Ok(())
    }

    fn config_target(&mut self, class: &ResolvedObject) -> DispatchResult<(String, String)> {
        let summary = class.package.summary();
        let config = self.config_name(class)?;
        let package = Path::new(summary.source.as_ref())
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DispatchError::InvalidConfigValue {
                property: summary
                    .name(summary.exports[class.export_index].object_name)
                    .to_owned(),
                message: "the class package has no file name".to_owned(),
            })?;
        let class_name = summary.name(summary.exports[class.export_index].object_name);
        Ok((config, format!("{package}.{class_name}")))
    }

    fn config_name(&mut self, class: &ResolvedObject) -> DispatchResult<String> {
        let mut current = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        for _ in 0..MAX_CALL_DEPTH {
            let script = self.script(&current)?;
            let ScriptMetadata::Class(metadata) = &script.metadata else {
                return Err(DispatchError::InvalidClass {
                    export_index: current.export_index,
                });
            };
            if let Some(name) = metadata
                .config_name
                .map(|index| current.package.summary().name(index))
                .filter(|name| !name.eq_ignore_ascii_case("None"))
            {
                return Ok(name.to_owned());
            }
            let Some(base) = self.packages.resolve(&current.package, script.base_field)? else {
                break;
            };
            current = base;
        }
        Ok("System".to_owned())
    }

    fn config_entries(
        &mut self,
        property: &ConfigProperty,
        value: &StoredValue,
    ) -> DispatchResult<Vec<ConfigEntry>> {
        if property.metadata.array_dimension > 1 {
            let StoredValue::Array(values) = value else {
                return Err(DispatchError::InvalidArrayProperty {
                    property: property.name.clone(),
                });
            };
            let field = self.resolved_object(&property.field)?;
            return values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    Ok(ConfigEntry {
                        section: String::new(),
                        key: format!("{}[{index}]", property.name),
                        values: vec![self.config_field_value(&field, &property.name, value)?],
                    })
                })
                .collect();
        }
        let field = self.resolved_object(&property.field)?;
        let class = field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index]);
        let values = match (class, value) {
            (Some("ArrayProperty"), StoredValue::Array(values)) => {
                let inner = property.metadata.inner_type.ok_or_else(|| {
                    DispatchError::MissingArrayInner {
                        property: property.name.clone(),
                    }
                })?;
                let inner = self
                    .packages
                    .resolve(&field.package, inner)?
                    .ok_or_else(|| DispatchError::MissingArrayInner {
                        property: property.name.clone(),
                    })?;
                values
                    .iter()
                    .map(|value| self.config_field_value(&inner, &property.name, value))
                    .collect::<DispatchResult<Vec<_>>>()?
            }
            (_, value) => vec![self.config_field_value(&field, &property.name, value)?],
        };
        Ok(vec![ConfigEntry {
            section: String::new(),
            key: property.name.clone(),
            values,
        }])
    }

    fn config_field_value(
        &mut self,
        field: &ResolvedObject,
        property: &str,
        value: &StoredValue,
    ) -> DispatchResult<String> {
        let invalid = |message| DispatchError::InvalidConfigValue {
            property: property.to_owned(),
            message,
        };
        let class = field
            .package
            .summary()
            .class_name(&field.package.summary().exports[field.export_index])
            .unwrap_or("<unknown>");
        let struct_name = if class.eq_ignore_ascii_case("StructProperty") {
            self.struct_name(field)?
        } else {
            None
        };
        match (class, value) {
            ("ByteProperty", StoredValue::Value(Value::Byte(value))) => Ok(self
                .enum_names(field)?
                .and_then(|names| names.get(usize::from(*value)).cloned())
                .unwrap_or_else(|| value.to_string())),
            ("IntProperty", StoredValue::Value(Value::Int(value))) => Ok(value.to_string()),
            ("BoolProperty", StoredValue::Value(Value::Bool(value))) => {
                Ok(if *value { "True" } else { "False" }.to_owned())
            }
            ("FloatProperty", StoredValue::Value(Value::Float(value))) if value.is_finite() => {
                Ok(value.to_string())
            }
            ("FloatProperty", StoredValue::Value(Value::Float(_))) => {
                Err(invalid("the float is not finite".to_owned()))
            }
            ("NameProperty", StoredValue::Name(value)) => ini_string(value).ok_or_else(|| {
                invalid("the name cannot round-trip through an INI value".to_owned())
            }),
            ("StrProperty" | "StringProperty", StoredValue::Value(Value::String(value))) => {
                ini_string(value).ok_or_else(|| {
                    invalid("the string cannot round-trip through an INI value".to_owned())
                })
            }
            ("ObjectProperty" | "ClassProperty", StoredValue::Object(None)) => {
                Ok("None".to_owned())
            }
            ("ObjectProperty" | "ClassProperty", StoredValue::Object(Some(object))) => {
                let object = self.resolved_object(object)?;
                if class == "ClassProperty"
                    && object.package.summary().exports[object.export_index].class
                        != ObjectReference::None
                {
                    return Err(invalid("the object is not a class".to_owned()));
                }
                PackageStore::qualified_object_name(&object).map_err(DispatchError::from)
            }
            ("StructProperty", StoredValue::Value(Value::Vector([x, y, z])))
                if struct_name.as_deref() == Some("Vector")
                    && x.is_finite()
                    && y.is_finite()
                    && z.is_finite() =>
            {
                Ok(format!("(X={x},Y={y},Z={z})"))
            }
            ("StructProperty", StoredValue::Value(Value::Rotator([pitch, yaw, roll])))
                if struct_name.as_deref() == Some("Rotator") =>
            {
                Ok(format!("(Pitch={pitch},Yaw={yaw},Roll={roll})"))
            }
            ("StructProperty", StoredValue::Value(Value::Struct(values)))
                if struct_name.as_deref() == Some("Color") =>
            {
                let component = |name| match values.get(name) {
                    Some(Value::Byte(value)) => Ok(*value),
                    _ => Err(invalid(format!(
                        "the Color struct has no byte `{name}` component"
                    ))),
                };
                Ok(format!(
                    "(R={},G={},B={},A={})",
                    component("R")?,
                    component("G")?,
                    component("B")?,
                    component("A")?
                ))
            }
            _ => Err(invalid(format!("unsupported `{class}` value"))),
        }
    }

    fn struct_name(&mut self, field: &ResolvedObject) -> DispatchResult<Option<String>> {
        let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
        let Some(structure) = metadata.struct_type else {
            return Ok(None);
        };
        let Some(structure) = self.packages.resolve(&field.package, structure)? else {
            return Ok(None);
        };
        Ok(Some(
            structure
                .package
                .summary()
                .name(structure.package.summary().exports[structure.export_index].object_name)
                .to_owned(),
        ))
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
            let Some(field) = self.find_property(class, &name, 0)? else {
                continue;
            };
            let resolved = self.resolved_object(&field)?;
            let Some(value) = self.read_property(source, reader, &property, &resolved)? else {
                continue;
            };
            let metadata = PropertyMetadata::decode(&resolved.package, resolved.export_index)?;
            let zero = self.zero_field_value(&resolved)?.unwrap_or(Value::None);
            let zero = self.stored_value(source, &zero)?;
            if metadata.array_dimension > 1 {
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
        field: &ResolvedObject,
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
            PropertyKind::Array => {
                let name = field
                    .package
                    .summary()
                    .name(field.package.summary().exports[field.export_index].object_name)
                    .to_owned();
                let length = value.read_compact_index()?;
                let count = usize::try_from(length).ok().filter(|count| {
                    // Every supported inline property consumes at least one byte.
                    *count <= value.remaining()
                });
                let Some(count) = count else {
                    return Err(DispatchError::InvalidDynamicArrayLength {
                        property: name,
                        length,
                        remaining: value.remaining(),
                    });
                };
                let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
                let inner =
                    metadata
                        .inner_type
                        .ok_or_else(|| DispatchError::MissingArrayInner {
                            property: name.clone(),
                        })?;
                let inner = self
                    .packages
                    .resolve(&field.package, inner)?
                    .ok_or_else(|| DispatchError::MissingArrayInner {
                        property: name.clone(),
                    })?;
                let inner_metadata = PropertyMetadata::decode(&inner.package, inner.export_index)?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    let element =
                        self.read_inline_property(source, &inner, &inner_metadata, &mut value, 0)?;
                    values.push(self.stored_value(source, &element)?);
                }
                StoredValue::Array(values)
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
            PropertyKind::Struct => {
                let metadata = PropertyMetadata::decode(&field.package, field.export_index)?;
                let structure =
                    metadata
                        .struct_type
                        .ok_or_else(|| DispatchError::MissingStructType {
                            property: field
                                .package
                                .summary()
                                .name(
                                    field.package.summary().exports[field.export_index].object_name,
                                )
                                .to_owned(),
                        })?;
                let structure = self
                    .packages
                    .resolve(&field.package, structure)?
                    .ok_or_else(|| DispatchError::MissingStructType {
                        property: field
                            .package
                            .summary()
                            .name(field.package.summary().exports[field.export_index].object_name)
                            .to_owned(),
                    })?;
                StoredValue::Value(self.read_struct_value(source, &structure, &mut value, 0)?)
            }
            _ => return Ok(None),
        }))
    }

    fn read_struct_value(
        &mut self,
        source: &Arc<Package>,
        structure: &ResolvedObject,
        reader: &mut ObjectReader<'_>,
        depth: usize,
    ) -> DispatchResult<Value> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let metadata = ScriptExport::decode(&structure.package, structure.export_index)?;
        let mut values = std::collections::HashMap::new();
        if let Some(base) = self
            .packages
            .resolve(&structure.package, metadata.base_field)?
        {
            let Value::Struct(base_values) =
                self.read_struct_value(source, &base, reader, depth + 1)?
            else {
                unreachable!();
            };
            values.extend(base_values);
        }
        let mut field = self
            .packages
            .resolve(&structure.package, metadata.children)?;
        while let Some(current) = field {
            let summary = current.package.summary();
            let class = summary
                .class_name(&summary.exports[current.export_index])
                .unwrap_or("<unknown>");
            if !class.ends_with("Property") {
                let metadata =
                    openhp1_script::FieldMetadata::decode(&current.package, current.export_index)?;
                field = self
                    .packages
                    .resolve(&current.package, metadata.next_field)?;
                continue;
            }
            let property = PropertyMetadata::decode(&current.package, current.export_index)?;
            let name = current
                .package
                .summary()
                .name(current.package.summary().exports[current.export_index].object_name)
                .to_owned();
            let dimension = usize::try_from(property.array_dimension).map_err(|_| {
                DispatchError::InvalidArrayDimension {
                    export_index: current.export_index,
                    dimension: property.array_dimension,
                }
            })?;
            if dimension == 0 {
                return Err(DispatchError::InvalidArrayDimension {
                    export_index: current.export_index,
                    dimension: property.array_dimension,
                });
            }
            let value = if dimension == 1 {
                self.read_inline_property(source, &current, &property, reader, depth + 1)?
            } else {
                let mut elements = Vec::with_capacity(dimension);
                for _ in 0..dimension {
                    elements.push(self.read_inline_property(
                        source,
                        &current,
                        &property,
                        reader,
                        depth + 1,
                    )?);
                }
                Value::Array(elements)
            };
            values.insert(name, value);
            field = self
                .packages
                .resolve(&current.package, property.next_field)?;
        }
        Ok(Value::Struct(values))
    }

    fn read_inline_property(
        &mut self,
        source: &Arc<Package>,
        field: &ResolvedObject,
        metadata: &PropertyMetadata,
        reader: &mut ObjectReader<'_>,
        depth: usize,
    ) -> DispatchResult<Value> {
        let export = &field.package.summary().exports[field.export_index];
        let name = field.package.summary().name(export.object_name).to_owned();
        let kind = field
            .package
            .summary()
            .class_name(export)
            .unwrap_or("<unknown>");
        Ok(match kind {
            "ByteProperty" => Value::Byte(reader.read_u8()?),
            "IntProperty" => Value::Int(reader.read_i32()?),
            "BoolProperty" => Value::Bool(reader.read_u8()? != 0),
            "FloatProperty" => Value::Float(reader.read_f32()?),
            "ObjectProperty" | "ClassProperty" => {
                let object = self
                    .packages
                    .resolve(source, reader.read_object_reference()?)?;
                Value::Object(match object {
                    Some(object) => {
                        self.object_handle(object_id(&object.package, object.export_index))?
                    }
                    None => 0,
                })
            }
            "NameProperty" => {
                let name = reader.read_name_index("runtime struct name property")?;
                Value::NameText(reader.summary().name(name).to_owned())
            }
            "StrProperty" | "StringProperty" => Value::String(reader.read_string()?),
            "StructProperty" => {
                let structure =
                    metadata
                        .struct_type
                        .ok_or_else(|| DispatchError::MissingStructType {
                            property: name.clone(),
                        })?;
                let structure = self
                    .packages
                    .resolve(&field.package, structure)?
                    .ok_or_else(|| DispatchError::MissingStructType {
                        property: name.clone(),
                    })?;
                match structure
                    .package
                    .summary()
                    .name(structure.package.summary().exports[structure.export_index].object_name)
                {
                    "Vector" => {
                        Value::Vector([reader.read_f32()?, reader.read_f32()?, reader.read_f32()?])
                    }
                    "Rotator" => {
                        Value::Rotator([reader.read_i32()?, reader.read_i32()?, reader.read_i32()?])
                    }
                    _ => self.read_struct_value(source, &structure, reader, depth + 1)?,
                }
            }
            _ => {
                return Err(DispatchError::UnsupportedStructField {
                    field: name,
                    kind: kind.to_owned(),
                });
            }
        })
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
            "ArrayProperty" => Value::Array(Vec::new()),
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
                        _ => Value::Struct(std::collections::HashMap::new()),
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

    pub(super) fn resolve_reference(
        &mut self,
        source: &Arc<Package>,
        reference: i32,
    ) -> DispatchResult<Option<ObjectId>> {
        let key = (Arc::clone(&source.summary().source), reference);
        if let Some(resolved) = self.resolved_references.get(&key) {
            return Ok(resolved.clone());
        }
        let resolved = self
            .packages
            .resolve(source, object_reference(reference))?
            .map(|object| ObjectId {
                package: Arc::clone(&object.package.summary().source),
                export_index: object.export_index,
            });
        self.resolved_references.insert(key, resolved.clone());
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
            let mut seen = HashSet::default();
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
                    _ => {
                        let Some(zero) = self.zero_field_value(&resolved)? else {
                            continue;
                        };
                        StructMember::Field {
                            name: name.to_owned(),
                            zero,
                        }
                    }
                };
                members.push((field, member));
            }
            let members = Arc::new(members);
            self.struct_members.insert(key, Arc::clone(&members));
            members
        };
        for (field, member) in members.iter() {
            frame.set_struct_member(*field, member.clone());
        }
        Ok(())
    }

    pub(super) fn bind_frame_arguments(
        &mut self,
        source: &Arc<Package>,
        function: &ScriptExport,
        arguments: &[Value],
        frame: &mut Frame<'_>,
    ) -> DispatchResult<ArgumentBindings> {
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
                let Some(id) = self.resolve_reference(source, field)? else {
                    continue;
                };
                if let Some((argument, (_, output))) = parameters
                    .iter()
                    .enumerate()
                    .find(|(_, (parameter, _))| *parameter == id)
                {
                    bindings.push((field, argument, *output));
                }
            }
            let bindings = Arc::new(bindings);
            self.frame_arguments.insert(key, Arc::clone(&bindings));
            bindings
        };
        for &(field, argument, _) in bindings.iter() {
            if let Some(value) = arguments.get(argument) {
                frame.set_local(field, value.clone());
            }
        }
        self.bind_frame_zero_values(source, function.export_index, &function.bytecode, frame)?;
        Ok(bindings)
    }

    pub(super) fn bind_frame_zero_values(
        &mut self,
        source: &Arc<Package>,
        export_index: usize,
        bytecode: &Bytecode,
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        let key = object_id(source, export_index);
        let values = if let Some(values) = self.frame_zero_values.get(&key) {
            Arc::clone(values)
        } else {
            let mut locals = Vec::new();
            let mut array_elements = Vec::new();
            for field in local_fields(bytecode) {
                let Some(field_object) = self.resolve_reference(source, field)? else {
                    continue;
                };
                let resolved = self.resolved_object(&field_object)?;
                if let Some(value) = self.zero_field_value(&resolved)? {
                    locals.push((field, value));
                }
            }
            for opcode in [0x00, 0x01, 0x02] {
                for field in fields(bytecode, opcode) {
                    let Some(field_object) = self.resolve_reference(source, field)? else {
                        continue;
                    };
                    let resolved = self.resolved_object(&field_object)?;
                    let metadata =
                        PropertyMetadata::decode(&resolved.package, resolved.export_index)?;
                    let Some(inner) = metadata.inner_type else {
                        continue;
                    };
                    let Some(inner) = self.packages.resolve(&resolved.package, inner)? else {
                        continue;
                    };
                    if let Some(value) = self.zero_field_value(&inner)? {
                        array_elements.push((field, value));
                    }
                }
            }
            let values = Arc::new(FrameZeroValues {
                locals,
                array_elements,
            });
            self.frame_zero_values.insert(key, Arc::clone(&values));
            values
        };
        for (field, value) in &values.locals {
            if frame.local(*field).is_some() {
                continue;
            }
            frame.set_local(*field, value.clone());
        }
        for (field, value) in &values.array_elements {
            frame.set_array_element_default(*field, value.clone());
        }
        Ok(())
    }

    pub(super) fn bind_frame_defaults(
        &mut self,
        class: &ResolvedObject,
        source: &Arc<Package>,
        bytecode: &Bytecode,
        frame: &mut Frame<'_>,
    ) -> DispatchResult<()> {
        let class_id = object_id(&class.package, class.export_index);
        if !self.class_defaults.contains_key(&class_id) {
            self.load_class_defaults(class, 0)?;
        }
        for field in fields(bytecode, 0x02) {
            let Some(id) = self.resolve_reference(source, field)? else {
                continue;
            };
            let value = match self
                .class_defaults
                .get(&class_id)
                .and_then(|defaults| defaults.get(&id))
                .cloned()
            {
                Some(value) => self.frame_value(&value)?,
                None => {
                    let resolved = self.resolved_object(&id)?;
                    self.zero_field_value(&resolved)?.unwrap_or(Value::None)
                }
            };
            frame.set_default(field, value);
        }
        Ok(())
    }

    fn function_parameters(
        &mut self,
        source: &Arc<Package>,
        mut field: ObjectReference,
    ) -> DispatchResult<Vec<(ObjectId, bool)>> {
        let mut parameters = Vec::new();
        let mut field_source = Arc::clone(source);
        for _ in 0..MAX_CALL_DEPTH {
            let Some(resolved) = self.packages.resolve(&field_source, field)? else {
                return Ok(parameters);
            };
            let metadata = PropertyMetadata::decode(&resolved.package, resolved.export_index)?;
            if metadata.flags & PROPERTY_PARAMETER != 0 && metadata.flags & PROPERTY_RETURN == 0 {
                parameters.push((
                    ObjectId {
                        package: Arc::clone(&resolved.package.summary().source),
                        export_index: resolved.export_index,
                    },
                    metadata.flags & PROPERTY_OUTPUT != 0,
                ));
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
