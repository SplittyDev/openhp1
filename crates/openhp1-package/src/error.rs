use std::{io, string::FromUtf16Error, sync::Arc};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not read Unreal package `{path}`")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error(
        "`{package}` ended at byte {file_len:#x}; reading {needed} bytes at {offset:#x} would exceed it"
    )]
    UnexpectedEnd {
        package: Arc<str>,
        offset: usize,
        needed: usize,
        file_len: usize,
    },

    #[error("`{package}` has package magic {actual:#010x}, expected {expected:#010x}")]
    InvalidMagic {
        package: Arc<str>,
        expected: u32,
        actual: u32,
    },

    #[error("`{package}` uses unsupported Unreal package version {version}")]
    UnsupportedVersion { package: Arc<str>, version: u16 },

    #[error("`{package}` has invalid {field} count {count} at byte {offset:#x}")]
    InvalidCount {
        package: Arc<str>,
        field: &'static str,
        count: i32,
        offset: usize,
    },

    #[error("`{package}` has invalid {field} offset {offset:#x}; package length is {file_len:#x}")]
    InvalidOffset {
        package: Arc<str>,
        field: &'static str,
        offset: usize,
        file_len: usize,
    },

    #[error("`{package}` has an invalid compact index at byte {offset:#x}")]
    InvalidCompactIndex { package: Arc<str>, offset: usize },

    #[error("`{package}` has invalid string length {length} at byte {offset:#x}")]
    InvalidStringLength {
        package: Arc<str>,
        offset: usize,
        length: i32,
    },

    #[error("`{package}` has an unterminated string at byte {offset:#x}")]
    UnterminatedString { package: Arc<str>, offset: usize },

    #[error("`{package}` has invalid UTF-16 at byte {offset:#x}")]
    InvalidUtf16 {
        package: Arc<str>,
        offset: usize,
        #[source]
        source: FromUtf16Error,
    },

    #[error(
        "`{package}` references name index {index} from {field} at byte {offset:#x}, but only {name_count} names exist"
    )]
    InvalidNameIndex {
        package: Arc<str>,
        field: &'static str,
        index: i32,
        name_count: usize,
        offset: usize,
    },

    #[error("`{package}` has invalid object reference {index} at byte {offset:#x}")]
    InvalidObjectReference {
        package: Arc<str>,
        index: i32,
        offset: usize,
    },

    #[error(
        "`{package}` export {export_index} spans {offset:#x}..{end:#x}, outside package length {file_len:#x}"
    )]
    InvalidExportRange {
        package: Arc<str>,
        export_index: usize,
        offset: usize,
        end: usize,
        file_len: usize,
    },

    #[error("`{package}` has no export at index {index}; it contains {export_count} exports")]
    InvalidExportIndex {
        package: Arc<str>,
        index: usize,
        export_count: usize,
    },

    #[error("`{package}` export {index} has no serialized payload")]
    ExportHasNoData { package: Arc<str>, index: usize },

    #[error("`{package}` has unknown property type {kind} at byte {offset:#x}")]
    InvalidPropertyType {
        package: Arc<str>,
        offset: usize,
        kind: u8,
    },
}
