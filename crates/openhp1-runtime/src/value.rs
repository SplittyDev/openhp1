use std::collections::HashMap;

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Byte(u8),
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
    Name(i32),
    NameText(String),
    Object(i32),
    Vector([f32; 3]),
    Rotator([i32; 3]),
    Struct(HashMap<String, Value>),
    Array(Vec<Value>),
}

impl Value {
    pub(crate) fn truthy(&self) -> Result<bool> {
        match self {
            Self::None | Self::Object(0) => Ok(false),
            Self::Bool(value) => Ok(*value),
            Self::Byte(value) => Ok(*value != 0),
            Self::Int(value) | Self::Name(value) | Self::Object(value) => Ok(*value != 0),
            Self::NameText(value) => Ok(!value.eq_ignore_ascii_case("None")),
            Self::Float(value) => Ok(*value != 0.0),
            Self::String(value) => Ok(!value.is_empty()),
            value => Err(Error::Type {
                expected: "boolean-compatible",
                actual: value.kind(),
            }),
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Byte(_) => "byte",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Bool(_) => "bool",
            Self::String(_) => "string",
            Self::Name(_) | Self::NameText(_) => "name",
            Self::Object(_) => "object",
            Self::Vector(_) => "vector",
            Self::Rotator(_) => "rotator",
            Self::Struct(_) => "struct",
            Self::Array(_) => "array",
        }
    }
}
