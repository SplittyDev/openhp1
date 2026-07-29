use openhp1_package::{ObjectReader, ObjectReference, Package};

use crate::{Bytecode, Error, Result};

const FUNCTION_NET: u32 = 0x0000_0040;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionMetadata {
    pub parameter_size: Option<u16>,
    pub native_index: u16,
    pub parameter_count: Option<u8>,
    pub operator_precedence: u8,
    pub return_value_offset: Option<u16>,
    pub flags: u32,
    pub replication_offset: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateMetadata {
    pub probe_mask: u64,
    pub ignore_mask: u64,
    pub label_table_offset: u16,
    pub flags: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDependency {
    pub class: ObjectReference,
    pub deep: u32,
    pub script_text_crc: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassMetadata {
    pub state: StateMetadata,
    pub old_record_size: Option<u32>,
    pub flags: u32,
    pub guid: [u8; 16],
    pub dependencies: Vec<ClassDependency>,
    pub package_imports: Vec<i32>,
    pub within: Option<i32>,
    pub config_name: Option<usize>,
    pub defaults_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyMetadata {
    pub base_field: ObjectReference,
    pub next_field: ObjectReference,
    pub array_dimension: i32,
    pub flags: u32,
    pub category: usize,
    pub replication_offset: Option<u16>,
    pub struct_type: Option<ObjectReference>,
    pub inner_type: Option<ObjectReference>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMetadata {
    pub base_field: ObjectReference,
    pub next_field: ObjectReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptMetadata {
    Struct,
    Function(FunctionMetadata),
    State(StateMetadata),
    Class(ClassMetadata),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptExport {
    pub export_index: usize,
    pub class_name: String,
    pub base_field: ObjectReference,
    pub next_field: ObjectReference,
    pub script_text: ObjectReference,
    pub children: ObjectReference,
    pub friendly_name: usize,
    pub line: u32,
    pub text_position: u32,
    pub bytecode: Bytecode,
    pub metadata: ScriptMetadata,
}

impl ScriptExport {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let mut reader = package.export_reader(export_index)?;
        decode_reader(package, export_index, &mut reader)
    }
}

impl PropertyMetadata {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let summary = package.summary();
        let export = summary.exports.get(export_index).ok_or_else(|| {
            openhp1_package::Error::InvalidExportIndex {
                package: summary.source.clone(),
                index: export_index,
                export_count: summary.exports.len(),
            }
        })?;
        let class_name = summary.class_name(export).unwrap_or("<unknown>");
        if !class_name.ends_with("Property") {
            return Err(unsupported(package, export_index, class_name));
        }
        let mut reader = package.export_reader(export_index)?;
        let field = read_field_metadata(export.object_flags, &mut reader)?;
        let array_dimension = reader.read_i32()?;
        let flags = reader.read_u32()?;
        let category = reader.read_name_index("property category")?;
        let replication_offset = (flags & 0x20 != 0).then(|| reader.read_u16()).transpose()?;
        let struct_type = class_name
            .eq_ignore_ascii_case("StructProperty")
            .then(|| reader.read_object_reference())
            .transpose()?;
        let inner_type = class_name
            .eq_ignore_ascii_case("ArrayProperty")
            .then(|| reader.read_object_reference())
            .transpose()?;
        Ok(Self {
            base_field: field.base_field,
            next_field: field.next_field,
            array_dimension,
            flags,
            category,
            replication_offset,
            struct_type,
            inner_type,
        })
    }
}

impl FieldMetadata {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let summary = package.summary();
        let export = summary.exports.get(export_index).ok_or_else(|| {
            openhp1_package::Error::InvalidExportIndex {
                package: summary.source.clone(),
                index: export_index,
                export_count: summary.exports.len(),
            }
        })?;
        let mut reader = package.export_reader(export_index)?;
        read_field_metadata(export.object_flags, &mut reader)
    }
}

pub fn class_defaults_reader(
    package: &Package,
    export_index: usize,
) -> Result<(ScriptExport, ObjectReader<'_>)> {
    let mut reader = package.export_reader(export_index)?;
    let decoded = decode_reader(package, export_index, &mut reader)?;
    if !matches!(decoded.metadata, ScriptMetadata::Class(_)) {
        return Err(unsupported(package, export_index, &decoded.class_name));
    }
    Ok((decoded, reader))
}

fn decode_reader(
    package: &Package,
    export_index: usize,
    reader: &mut ObjectReader<'_>,
) -> Result<ScriptExport> {
    let summary = package.summary();
    let export = summary.exports.get(export_index).ok_or_else(|| {
        openhp1_package::Error::InvalidExportIndex {
            package: summary.source.clone(),
            index: export_index,
            export_count: summary.exports.len(),
        }
    })?;
    let class_name = if export.class == ObjectReference::None {
        "Class"
    } else {
        summary.class_name(export).unwrap_or("<unknown>")
    }
    .to_owned();
    if !matches_ignore_ascii_case(&class_name, &["Struct", "Function", "State", "Class"]) {
        return Err(unsupported(package, export_index, &class_name));
    }

    reader.read_object_stack(export.object_flags)?;
    if !class_name.eq_ignore_ascii_case("Class") {
        while reader.next_property()?.is_some() {}
    }

    let base_field = reader.read_object_reference()?;
    let next_field = reader.read_object_reference()?;
    let script_text = reader.read_object_reference()?;
    let children = reader.read_object_reference()?;
    let friendly_name = reader.read_name_index("script friendly name")?;
    let line = reader.read_u32()?;
    let text_position = reader.read_u32()?;
    let decoded_size = reader.read_u32()?;
    let bytecode = Bytecode::decode(reader, decoded_size)?;

    let metadata = if class_name.eq_ignore_ascii_case("Function") {
        ScriptMetadata::Function(read_function(reader)?)
    } else if class_name.eq_ignore_ascii_case("State") {
        ScriptMetadata::State(read_state(reader)?)
    } else if class_name.eq_ignore_ascii_case("Class") {
        ScriptMetadata::Class(read_class(reader)?)
    } else {
        ScriptMetadata::Struct
    };

    Ok(ScriptExport {
        export_index,
        class_name,
        base_field,
        next_field,
        script_text,
        children,
        friendly_name,
        line,
        text_position,
        bytecode,
        metadata,
    })
}

fn read_field_metadata(object_flags: u32, reader: &mut ObjectReader<'_>) -> Result<FieldMetadata> {
    reader.read_object_stack(object_flags)?;
    while reader.next_property()?.is_some() {}
    Ok(FieldMetadata {
        base_field: reader.read_object_reference()?,
        next_field: reader.read_object_reference()?,
    })
}

fn read_function(reader: &mut ObjectReader<'_>) -> Result<FunctionMetadata> {
    let legacy = reader.summary().header.version <= 63;
    let parameter_size = legacy.then(|| reader.read_u16()).transpose()?;
    let native_index = reader.read_u16()?;
    let parameter_count = legacy.then(|| reader.read_u8()).transpose()?;
    let operator_precedence = reader.read_u8()?;
    let return_value_offset = legacy.then(|| reader.read_u16()).transpose()?;
    let flags = reader.read_u32()?;
    let replication_offset = (flags & FUNCTION_NET != 0)
        .then(|| reader.read_u16())
        .transpose()?;
    Ok(FunctionMetadata {
        parameter_size,
        native_index,
        parameter_count,
        operator_precedence,
        return_value_offset,
        flags,
        replication_offset,
    })
}

fn read_state(reader: &mut ObjectReader<'_>) -> Result<StateMetadata> {
    Ok(StateMetadata {
        probe_mask: reader.read_u64()?,
        ignore_mask: reader.read_u64()?,
        label_table_offset: reader.read_u16()?,
        flags: reader.read_u32()?,
    })
}

fn read_class(reader: &mut ObjectReader<'_>) -> Result<ClassMetadata> {
    let state = read_state(reader)?;
    let old_record_size = (reader.summary().header.version <= 61)
        .then(|| reader.read_u32())
        .transpose()?;
    let flags = reader.read_u32()?;
    let guid = reader.read_bytes(16)?.try_into().unwrap();
    let dependency_count = read_count(reader, "class dependencies", 9)?;
    let mut dependencies = Vec::with_capacity(dependency_count);
    for _ in 0..dependency_count {
        dependencies.push(ClassDependency {
            class: reader.read_object_reference()?,
            deep: reader.read_u32()?,
            script_text_crc: reader.read_u32()?,
        });
    }
    let import_count = read_count(reader, "class package imports", 1)?;
    let mut package_imports = Vec::with_capacity(import_count);
    for _ in 0..import_count {
        package_imports.push(reader.read_compact_index()?);
    }
    let (within, config_name) = if reader.summary().header.version >= 62 {
        (
            Some(reader.read_compact_index()?),
            Some(reader.read_name_index("class config name")?),
        )
    } else {
        (None, None)
    };
    Ok(ClassMetadata {
        state,
        old_record_size,
        flags,
        guid,
        dependencies,
        package_imports,
        within,
        config_name,
        defaults_offset: reader.position(),
    })
}

fn read_count(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    minimum_size: usize,
) -> Result<usize> {
    let offset = reader.absolute_position();
    let count = reader.read_compact_index()?;
    let count = usize::try_from(count).map_err(|_| Error::InvalidCount {
        package: reader.summary().source.clone(),
        field,
        count,
        offset,
    })?;
    if count > reader.remaining() / minimum_size {
        return Err(Error::InvalidCount {
            package: reader.summary().source.clone(),
            field,
            count: i32::try_from(count).unwrap_or(i32::MAX),
            offset,
        });
    }
    Ok(count)
}

fn unsupported(package: &Package, export_index: usize, class_name: &str) -> Error {
    Error::UnsupportedExportClass {
        package: package.summary().source.clone(),
        export_index,
        class_name: class_name.to_owned(),
    }
}

fn matches_ignore_ascii_case(value: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openhp1_package::{PACKAGE_MAGIC, Package};

    use super::*;
    use crate::CallTarget;

    #[test]
    fn decodes_legacy_function_and_expands_compact_bytecode_indices() {
        let bytecode = [0x1b, 6, 0x1d, 42, 0, 0, 0, 0x16, 0x04];
        let mut payload = vec![0, 0, 0, 0, 0, 5];
        payload.extend(10_u32.to_le_bytes());
        payload.extend(20_u32.to_le_bytes());
        payload.extend(12_u32.to_le_bytes());
        payload.extend(bytecode);
        payload.extend(8_u16.to_le_bytes());
        payload.extend(0_u16.to_le_bytes());
        payload.push(1);
        payload.push(0);
        payload.extend(4_u16.to_le_bytes());
        payload.extend(2_u32.to_le_bytes());

        let package = synthetic_package("Function", "TestFunction", payload);
        let decoded = ScriptExport::decode(&package, 0).unwrap();
        let ScriptMetadata::Function(function) = decoded.metadata else {
            panic!("expected function metadata");
        };
        assert_eq!(function.parameter_size, Some(8));
        assert_eq!(function.parameter_count, Some(1));
        assert_eq!(decoded.bytecode.raw_len, bytecode.len());
        assert_eq!(decoded.bytecode.bytes.len(), 12);
        assert_eq!(&decoded.bytecode.bytes[1..5], &6_i32.to_le_bytes());
        assert_eq!(
            decoded.bytecode.tokens[0].call,
            Some(CallTarget::Virtual(6))
        );
        assert_eq!(
            decoded
                .bytecode
                .tokens
                .iter()
                .map(|token| token.opcode)
                .collect::<Vec<_>>(),
            [0x1b, 0x1d, 0x16, 0x04]
        );
    }

    #[test]
    fn positions_reader_at_class_defaults_after_nonempty_bytecode() {
        let mut payload = vec![0, 0, 0, 0, 5];
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.push(0x04);
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u64.to_le_bytes());
        payload.extend(0_u16.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(2_u32.to_le_bytes());
        payload.extend([0; 16]);
        payload.push(0);
        payload.push(0);
        payload.push(0);

        let package = synthetic_package("Class", "TestClass", payload);
        let (decoded, mut defaults) = class_defaults_reader(&package, 0).unwrap();
        let ScriptMetadata::Class(class) = decoded.metadata else {
            panic!("expected class metadata");
        };
        assert_eq!(decoded.bytecode.bytes, [0x04]);
        assert_eq!(class.defaults_offset, defaults.position());
        assert!(defaults.next_property().unwrap().is_none());
        assert_eq!(defaults.remaining(), 0);
    }

    #[test]
    fn rejects_unknown_bytecode_token_with_offsets() {
        let mut payload = vec![0, 0, 0, 0, 0, 5];
        payload.extend(0_u32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend(1_u32.to_le_bytes());
        payload.push(0x03);
        let package = synthetic_package("Function", "BadFunction", payload);
        assert!(matches!(
            ScriptExport::decode(&package, 0),
            Err(Error::UnknownToken {
                token: 0x03,
                decoded_offset: 0,
                ..
            })
        ));
    }

    #[test]
    fn decodes_function_parameter_property_metadata() {
        let mut payload = vec![0, 0, 0];
        payload.extend(1_i32.to_le_bytes());
        payload.extend(0xa0_u32.to_le_bytes());
        payload.push(5);
        payload.extend(77_u16.to_le_bytes());
        let package = synthetic_package("FloatProperty", "DeltaTime", payload);
        let property = PropertyMetadata::decode(&package, 0).unwrap();
        assert_eq!(property.array_dimension, 1);
        assert_eq!(property.flags, 0xa0);
        assert_eq!(property.category, 5);
        assert_eq!(property.replication_offset, Some(77));
        assert_eq!(property.struct_type, None);
        assert_eq!(property.inner_type, None);
    }

    #[test]
    fn decodes_struct_property_type_reference() {
        let mut payload = vec![0, 0, 0];
        payload.extend(1_i32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.push(5);
        payload.push(0);
        let package = synthetic_package("StructProperty", "Value", payload);
        let property = PropertyMetadata::decode(&package, 0).unwrap();
        assert_eq!(property.struct_type, Some(ObjectReference::None));
        assert_eq!(property.inner_type, None);
    }

    #[test]
    fn decodes_array_property_inner_reference() {
        let mut payload = vec![0, 0, 0];
        payload.extend(1_i32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend([5, 0]);
        let package = synthetic_package("ArrayProperty", "Values", payload);
        let property = PropertyMetadata::decode(&package, 0).unwrap();
        assert_eq!(property.struct_type, None);
        assert_eq!(property.inner_type, Some(ObjectReference::None));
    }

    #[test]
    fn decodes_non_property_field_links() {
        let package = synthetic_package("Enum", "Values", vec![0, 0, 0]);
        assert_eq!(
            FieldMetadata::decode(&package, 0).unwrap(),
            FieldMetadata {
                base_field: ObjectReference::None,
                next_field: ObjectReference::None,
            }
        );
    }

    fn synthetic_package(class_name: &str, object_name: &str, payload: Vec<u8>) -> Package {
        let names = [
            "None",
            "Core",
            "Class",
            class_name,
            object_name,
            "Friendly",
            "CalledFunction",
        ];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            push_u32(&mut name_table, 0);
        }

        let mut import_table = vec![1, 2];
        push_i32(&mut import_table, 0);
        import_table.push(3);

        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![
            if class_name.eq_ignore_ascii_case("Class") {
                0
            } else {
                0x81
            },
            0,
        ];
        push_i32(&mut export, 0);
        export.push(4);
        push_u32(&mut export, 0);
        export.extend(compact_index(payload.len() as i32));
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let encoded = compact_index(payload_offset as i32);
            let next = export_offset + export.len() + encoded.len();
            if next == payload_offset {
                export.extend(encoded);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        push_u32(&mut bytes, PACKAGE_MAGIC);
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        push_u32(&mut bytes, 0);
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            push_i32(&mut bytes, value as i32);
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        Package::parse("synthetic script", Arc::from(bytes)).unwrap()
    }

    fn compact_index(value: i32) -> Vec<u8> {
        let negative = value < 0;
        let mut value = value.unsigned_abs();
        let mut bytes = vec![(value as u8 & 0x3f) | if negative { 0x80 } else { 0 }];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = value as u8 & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }
}
