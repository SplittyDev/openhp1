use std::sync::Arc;

use crate::{
    PACKAGE_MAGIC,
    archive::Archive,
    error::{Error, Result},
    summary::{
        Export, Generation, HeaderHistory, Import, NameEntry, ObjectReference, PackageHeader,
        PackageSummary,
    },
};

const MIN_SUPPORTED_VERSION: u16 = 61;
const MAX_SUPPORTED_VERSION: u16 = 76;

pub(crate) fn read_summary(bytes: &[u8], source: Arc<str>) -> Result<PackageSummary> {
    let mut archive = Archive::new(bytes, Arc::clone(&source));
    let header = read_header(&mut archive, bytes.len())?;
    let names = read_names(&mut archive, &header)?;
    let imports = read_imports(&mut archive, &header, names.len())?;
    let exports = read_exports(&mut archive, &header, names.len(), bytes.len())?;
    Ok(PackageSummary {
        source,
        header,
        names,
        imports,
        exports,
    })
}

fn read_header(archive: &mut Archive<'_>, file_len: usize) -> Result<PackageHeader> {
    let source = archive_source(archive);
    let magic = archive.read_u32()?;
    if magic != PACKAGE_MAGIC {
        return Err(Error::InvalidMagic {
            package: source,
            expected: PACKAGE_MAGIC,
            actual: magic,
        });
    }

    let version = archive.read_u16()?;
    let licensee_version = archive.read_u16()?;
    if !(MIN_SUPPORTED_VERSION..=MAX_SUPPORTED_VERSION).contains(&version) {
        return Err(Error::UnsupportedVersion {
            package: source,
            version,
        });
    }

    let package_flags = archive.read_u32()?;
    let name_count = read_count(archive, "name table")?;
    let name_offset = read_offset(archive, "name table", file_len)?;
    let export_count = read_count(archive, "export table")?;
    let export_offset = read_offset(archive, "export table", file_len)?;
    let import_count = read_count(archive, "import table")?;
    let import_offset = read_offset(archive, "import table", file_len)?;

    let history = if version < 68 {
        HeaderHistory::Heritage {
            count: read_count(archive, "heritage table")?,
            offset: read_offset(archive, "heritage table", file_len)?,
        }
    } else {
        let guid = archive.read_guid()?;
        let generation_count = read_count(archive, "generation table")?;
        let mut generations = Vec::with_capacity(generation_count);
        for _ in 0..generation_count {
            generations.push(Generation {
                export_count: read_count(archive, "generation export")?,
                name_count: read_count(archive, "generation name")?,
            });
        }
        HeaderHistory::Generations { guid, generations }
    };

    Ok(PackageHeader {
        version,
        licensee_version,
        package_flags,
        name_count,
        name_offset,
        export_count,
        export_offset,
        import_count,
        import_offset,
        history,
    })
}

fn read_names(archive: &mut Archive<'_>, header: &PackageHeader) -> Result<Vec<NameEntry>> {
    archive.seek(header.name_offset, "name table")?;
    let mut names = Vec::with_capacity(header.name_count);
    for _ in 0..header.name_count {
        let value = if header.version < 64 {
            archive.read_c_string()?
        } else {
            archive.read_unreal_string()?
        };
        names.push(NameEntry {
            value,
            flags: archive.read_u32()?,
        });
    }
    Ok(names)
}

fn read_imports(
    archive: &mut Archive<'_>,
    header: &PackageHeader,
    name_count: usize,
) -> Result<Vec<Import>> {
    archive.seek(header.import_offset, "import table")?;
    let mut imports = Vec::with_capacity(header.import_count);
    for _ in 0..header.import_count {
        let class_package = read_name_index(archive, name_count, "import class package")?;
        let class_name = read_name_index(archive, name_count, "import class name")?;
        // Package/outer indices in both UE1 index tables are fixed-width i32s.
        let outer_offset = archive.position();
        let outer = object_reference(archive.read_i32()?, archive_source(archive), outer_offset)?;
        imports.push(Import {
            class_package,
            class_name,
            outer,
            object_name: read_name_index(archive, name_count, "import object name")?,
        });
    }
    Ok(imports)
}

fn read_exports(
    archive: &mut Archive<'_>,
    header: &PackageHeader,
    name_count: usize,
    file_len: usize,
) -> Result<Vec<Export>> {
    archive.seek(header.export_offset, "export table")?;
    let mut exports = Vec::with_capacity(header.export_count);
    for export_index in 0..header.export_count {
        let class = read_object_reference(archive)?;
        let super_class = read_object_reference(archive)?;
        // UE1 stores the export's outer/package index as a fixed-width value.
        let outer_offset = archive.position();
        let outer = object_reference(archive.read_i32()?, archive_source(archive), outer_offset)?;
        let object_name = read_name_index(archive, name_count, "export object name")?;
        let object_flags = archive.read_u32()?;
        let size_offset = archive.position();
        let serial_size = nonnegative_compact(archive.read_compact_index()?, archive, size_offset)?;
        let serial_offset = if serial_size == 0 {
            None
        } else {
            let offset_position = archive.position();
            Some(nonnegative_compact(
                archive.read_compact_index()?,
                archive,
                offset_position,
            )?)
        };

        if let Some(offset) = serial_offset {
            let end = offset
                .checked_add(serial_size)
                .filter(|end| *end <= file_len)
                .ok_or_else(|| Error::InvalidExportRange {
                    package: archive_source(archive),
                    export_index,
                    offset,
                    end: offset.saturating_add(serial_size),
                    file_len,
                })?;
            debug_assert!(end <= file_len);
        }

        exports.push(Export {
            class,
            super_class,
            outer,
            object_name,
            object_flags,
            serial_size,
            serial_offset,
        });
    }
    Ok(exports)
}

fn read_count(archive: &mut Archive<'_>, field: &'static str) -> Result<usize> {
    let offset = archive.position();
    let count = archive.read_i32()?;
    usize::try_from(count).map_err(|_| Error::InvalidCount {
        package: archive_source(archive),
        field,
        count,
        offset,
    })
}

fn read_offset(archive: &mut Archive<'_>, field: &'static str, file_len: usize) -> Result<usize> {
    let offset = usize::try_from(archive.read_i32()?).map_err(|_| Error::InvalidOffset {
        package: archive_source(archive),
        field,
        offset: usize::MAX,
        file_len,
    })?;
    if offset > file_len {
        return Err(Error::InvalidOffset {
            package: archive_source(archive),
            field,
            offset,
            file_len,
        });
    }
    Ok(offset)
}

fn read_name_index(
    archive: &mut Archive<'_>,
    name_count: usize,
    field: &'static str,
) -> Result<usize> {
    let offset = archive.position();
    let index = archive.read_compact_index()?;
    usize::try_from(index)
        .ok()
        .filter(|index| *index < name_count)
        .ok_or_else(|| Error::InvalidNameIndex {
            package: archive_source(archive),
            field,
            index,
            name_count,
            offset,
        })
}

fn read_object_reference(archive: &mut Archive<'_>) -> Result<ObjectReference> {
    let offset = archive.position();
    let index = archive.read_compact_index()?;
    object_reference(index, archive_source(archive), offset)
}

pub(crate) fn object_reference(
    index: i32,
    package: Arc<str>,
    offset: usize,
) -> Result<ObjectReference> {
    if index == 0 {
        return Ok(ObjectReference::None);
    }
    if index > 0 {
        return Ok(ObjectReference::Export(index as usize - 1));
    }
    let absolute = index
        .checked_abs()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(Error::InvalidObjectReference {
            package,
            index,
            offset,
        })?;
    Ok(ObjectReference::Import(absolute - 1))
}

fn nonnegative_compact(value: i32, archive: &Archive<'_>, offset: usize) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidCount {
        package: archive_source(archive),
        field: "serialized export",
        count: value,
        offset,
    })
}

fn archive_source(archive: &Archive<'_>) -> Arc<str> {
    archive.source()
}

#[cfg(test)]
mod tests {
    use crate::ObjectReference;

    #[test]
    fn object_reference_sign_selects_table() {
        assert_eq!(
            super::object_reference(0, "test".into(), 0).unwrap(),
            ObjectReference::None
        );
        assert_eq!(
            super::object_reference(1, "test".into(), 0).unwrap(),
            ObjectReference::Export(0)
        );
        assert_eq!(
            super::object_reference(-1, "test".into(), 0).unwrap(),
            ObjectReference::Import(0)
        );
    }
}
