use thiserror::Error;

use crate::{FunctionCall, MAX_EXPRESSION_DEPTH};

#[derive(Debug, Error, PartialEq)]
pub enum Error {
    #[error("bytecode ended while reading {needed} bytes at execution offset {offset:#x}")]
    UnexpectedEnd { offset: usize, needed: usize },

    #[error("unsupported script opcode {opcode:#04x} at execution offset {offset:#x}")]
    UnsupportedOpcode { offset: usize, opcode: u8 },

    #[error("script jump target {target:#x} is outside {length} execution bytes")]
    InvalidJump { target: usize, length: usize },

    #[error("expected Case at execution offset {offset:#x}, found opcode {opcode:#04x}")]
    ExpectedCase { offset: usize, opcode: u8 },

    #[error("script exceeded its {limit}-instruction execution limit")]
    StepLimit { limit: usize },

    #[error(
        "script expression depth exceeds {MAX_EXPRESSION_DEPTH} at execution offset {offset:#x}"
    )]
    ExpressionDepth { offset: usize },

    #[error("expected {expected} value, found {actual}")]
    Type {
        expected: &'static str,
        actual: &'static str,
    },

    #[error("assignment target is not a variable")]
    NotAssignable,

    #[error("array index {index} is outside array length {length}")]
    ArrayIndex { index: i32, length: usize },

    #[error("iterator control flow has no active iterator")]
    MissingIterator,

    #[error("state-only control flow was used in a function frame")]
    UnexpectedStateControl,

    #[error("struct member field {field} is not initialized")]
    MissingStructMember { field: i32 },

    #[error("object context {object} is not addressable by this runtime")]
    UnsupportedContext { object: i32 },

    #[error("context operation failed: {message}")]
    Context { message: String },

    #[error("{call:?} failed: {message}")]
    Call { call: FunctionCall, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;
