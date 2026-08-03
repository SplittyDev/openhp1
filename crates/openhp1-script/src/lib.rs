//! Checked metadata and bytecode decoding for UnrealScript exports.

mod bytecode;
mod error;
mod metadata;

pub use bytecode::{Bytecode, CallTarget, Token, token_name};
pub use error::{Error, Result};
pub use metadata::{
    ClassDependency, ClassMetadata, FieldMetadata, FunctionMetadata, PropertyMetadata,
    ScriptExport, ScriptMetadata, StateMetadata, class_defaults_reader, enum_names,
};
