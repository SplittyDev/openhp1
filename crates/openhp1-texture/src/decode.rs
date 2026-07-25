use std::sync::Arc;

use openhp1_package::Package;

use crate::{Error, Result};

pub(crate) fn require_class(
    package: &Package,
    export_index: usize,
    expected: &'static str,
) -> Result<()> {
    let summary = package.summary();
    let export = summary.exports.get(export_index).ok_or_else(|| {
        openhp1_package::Error::InvalidExportIndex {
            package: Arc::clone(&summary.source),
            index: export_index,
            export_count: summary.exports.len(),
        }
    })?;
    let actual = summary.class_name(export).unwrap_or("<class>");
    if actual != expected {
        return Err(Error::WrongClass {
            expected,
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn nonnegative(value: i32, offset: usize, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::NegativeCount {
        field,
        value,
        offset,
    })
}
