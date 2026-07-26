use std::sync::Arc;

use openhp1_package::Package;

pub(crate) fn package(
    source: &str,
    names: &[&str],
    class_name: usize,
    object_name: usize,
    payload: Vec<u8>,
) -> Package {
    let mut name_table = Vec::new();
    for name in names {
        name_table.extend(name.as_bytes());
        name_table.push(0);
        push_u32(&mut name_table, 0);
    }

    let mut import_table = vec![1, 2];
    push_i32(&mut import_table, 0);
    import_table.extend(compact_index(class_name as i32));

    const HEADER_SIZE: usize = 44;
    let name_offset = HEADER_SIZE;
    let import_offset = name_offset + name_table.len();
    let export_offset = import_offset + import_table.len();
    let mut export_prefix = vec![0x81, 0];
    push_i32(&mut export_prefix, 0);
    export_prefix.extend(compact_index(object_name as i32));
    push_u32(&mut export_prefix, 0);
    export_prefix.extend(compact_index(payload.len() as i32));
    let mut payload_offset = export_offset + export_prefix.len() + 1;
    loop {
        let encoded = compact_index(payload_offset as i32);
        let next = export_offset + export_prefix.len() + encoded.len();
        if next == payload_offset {
            export_prefix.extend(encoded);
            break;
        }
        payload_offset = next;
    }

    let mut bytes = Vec::new();
    push_u32(&mut bytes, openhp1_package::PACKAGE_MAGIC);
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
    bytes.extend(export_prefix);
    assert_eq!(bytes.len(), payload_offset);
    bytes.extend(payload);
    Package::parse(source, Arc::from(bytes)).unwrap()
}

pub(crate) fn compact_index(value: i32) -> Vec<u8> {
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

pub(crate) fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend(value.to_le_bytes());
}

pub(crate) fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

pub(crate) fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend(value.to_le_bytes());
}
