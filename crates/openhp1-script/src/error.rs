use std::sync::Arc;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error("`{package}` export {export_index} is unsupported script metadata `{class_name}`")]
    UnsupportedExportClass {
        package: Arc<str>,
        export_index: usize,
        class_name: String,
    },

    #[error(
        "`{package}` bytecode at {raw_offset:#x} decoded to {actual} bytes, expected {expected}"
    )]
    BytecodeSize {
        package: Arc<str>,
        raw_offset: usize,
        expected: usize,
        actual: usize,
    },

    #[error(
        "`{package}` has unknown script token {token:#04x} at raw byte {raw_offset:#x} (decoded offset {decoded_offset:#x})"
    )]
    UnknownToken {
        package: Arc<str>,
        raw_offset: usize,
        decoded_offset: usize,
        token: u8,
    },

    #[error(
        "`{package}` script nesting exceeds 64 expressions at raw byte {raw_offset:#x} (decoded offset {decoded_offset:#x})"
    )]
    RecursionLimit {
        package: Arc<str>,
        raw_offset: usize,
        decoded_offset: usize,
    },

    #[error("`{package}` has invalid {field} count {count} at byte {offset:#x}")]
    InvalidCount {
        package: Arc<str>,
        field: &'static str,
        count: i32,
        offset: usize,
    },
}
