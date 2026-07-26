use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),
    #[error("{field} has invalid count {count}")]
    InvalidCount { field: &'static str, count: i32 },
    #[error("{field} lazy array ended at {actual:#x}, expected {expected:#x}")]
    InvalidLazyArray {
        field: &'static str,
        actual: usize,
        expected: u32,
    },
    #[error("{field} index {index} is outside 0..{length}")]
    InvalidIndex {
        field: &'static str,
        index: usize,
        length: usize,
    },
    #[error("unsupported mesh class {0}")]
    UnsupportedClass(String),
}

pub type Result<T> = std::result::Result<T, Error>;
