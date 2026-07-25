use std::{fs, path::Path, sync::Arc};

use crate::{
    error::{Error, Result},
    object::ObjectReader,
    summary::PackageSummary,
    tables::read_summary,
};

pub struct Package {
    summary: PackageSummary,
    bytes: Arc<[u8]>,
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(path.display().to_string(), bytes.into())
    }

    pub fn parse(source: impl Into<Arc<str>>, bytes: Arc<[u8]>) -> Result<Self> {
        let summary = read_summary(&bytes, source.into())?;
        Ok(Self { summary, bytes })
    }

    pub fn summary(&self) -> &PackageSummary {
        &self.summary
    }

    pub fn export_data(&self, index: usize) -> Option<&[u8]> {
        let export = self.summary.exports.get(index)?;
        let offset = export.serial_offset?;
        self.bytes.get(offset..offset + export.serial_size)
    }

    pub fn export_reader(&self, index: usize) -> Result<ObjectReader<'_>> {
        let export = self
            .summary
            .exports
            .get(index)
            .ok_or_else(|| Error::InvalidExportIndex {
                package: Arc::clone(&self.summary.source),
                index,
                export_count: self.summary.exports.len(),
            })?;
        let offset = export.serial_offset.ok_or_else(|| Error::ExportHasNoData {
            package: Arc::clone(&self.summary.source),
            index,
        })?;
        Ok(ObjectReader::new(
            &self.bytes[offset..offset + export.serial_size],
            &self.summary,
            offset,
        ))
    }
}
