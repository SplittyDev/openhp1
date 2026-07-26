//! Initial UnrealScript execution core for package-backed runtime objects.

use std::collections::HashMap;

use openhp1_script::Bytecode;
use thiserror::Error;

mod world;

pub use world::{ActorAction, DispatchError, DispatchResult, ScriptRuntime};

pub type Result<T> = std::result::Result<T, Error>;
const MAX_EXPRESSION_DEPTH: usize = 64;

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
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FunctionCall {
    Native(u16),
    Virtual(i32),
    Final(i32),
    Global(i32),
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
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum Error {
    #[error("bytecode ended while reading {needed} bytes at execution offset {offset:#x}")]
    UnexpectedEnd { offset: usize, needed: usize },

    #[error("unsupported script opcode {opcode:#04x} at execution offset {offset:#x}")]
    UnsupportedOpcode { offset: usize, opcode: u8 },

    #[error("script jump target {target:#x} is outside {length} execution bytes")]
    InvalidJump { target: usize, length: usize },

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

    #[error("struct member field {field} is not initialized")]
    MissingStructMember { field: i32 },

    #[error("object context {object} is not addressable by this runtime")]
    UnsupportedContext { object: i32 },

    #[error("context operation failed: {message}")]
    Context { message: String },

    #[error("{call:?} failed: {message}")]
    Call { call: FunctionCall, message: String },
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum StructMember {
    X,
    Y,
    Z,
    Pitch,
    Yaw,
    Roll,
}

#[derive(Clone, Debug)]
enum Slot {
    Local(i32),
    Instance {
        receiver: i32,
        field: i32,
    },
    Default(i32),
    StructMember {
        target: Box<Slot>,
        member: StructMember,
    },
}

enum Expression {
    Value(Value),
    Slot(Slot),
}

pub(crate) enum FrameRequest {
    Call {
        receiver: i32,
        function: FunctionCall,
        arguments: Vec<Value>,
    },
    GetInstance {
        receiver: i32,
        field: i32,
    },
    SetInstance {
        receiver: i32,
        field: i32,
        value: Value,
    },
}

/// Mutable state for one UnrealScript frame.
pub struct Frame<'a> {
    bytecode: &'a Bytecode,
    instruction_pointer: usize,
    steps: usize,
    step_limit: usize,
    expression_depth: usize,
    current_context: i32,
    locals: HashMap<i32, Value>,
    instance: HashMap<i32, Value>,
    defaults: HashMap<i32, Value>,
    struct_members: HashMap<i32, StructMember>,
}

impl<'a> Frame<'a> {
    pub fn new(bytecode: &'a Bytecode) -> Self {
        Self {
            bytecode,
            instruction_pointer: 0,
            steps: 0,
            step_limit: 100_000,
            expression_depth: 0,
            current_context: -1,
            locals: HashMap::new(),
            instance: HashMap::new(),
            defaults: HashMap::new(),
            struct_members: HashMap::new(),
        }
    }

    pub fn set_step_limit(&mut self, limit: usize) {
        self.step_limit = limit;
    }

    pub fn set_local(&mut self, field: i32, value: Value) {
        self.locals.insert(field, value);
    }

    pub fn local(&self, field: i32) -> Option<&Value> {
        self.locals.get(&field)
    }

    pub fn set_instance(&mut self, field: i32, value: Value) {
        self.instance.insert(field, value);
    }

    pub fn instance(&self, field: i32) -> Option<&Value> {
        self.instance.get(&field)
    }

    pub(crate) fn set_struct_member(&mut self, field: i32, member: StructMember) {
        self.struct_members.insert(field, member);
    }

    pub fn execute(
        &mut self,
        mut call: impl FnMut(FunctionCall, &[Value]) -> std::result::Result<Value, String>,
    ) -> Result<Value> {
        self.execute_inner(&mut |request, _| match request {
            FrameRequest::Call {
                receiver: -1,
                function,
                arguments,
            } => call(function, &arguments),
            FrameRequest::Call { receiver, .. }
            | FrameRequest::GetInstance { receiver, .. }
            | FrameRequest::SetInstance { receiver, .. } => {
                Err(format!("object context {receiver} has no frame host"))
            }
        })
    }

    fn execute_with_instance(
        &mut self,
        instance: &mut HashMap<i32, Value>,
        mut host: impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Value> {
        std::mem::swap(&mut self.instance, instance);
        let result = self.execute_inner(&mut host);
        std::mem::swap(&mut self.instance, instance);
        result
    }

    fn execute_inner(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Value> {
        self.instruction_pointer = 0;
        self.steps = 0;
        self.expression_depth = 0;
        self.current_context = -1;
        while self.instruction_pointer < self.bytecode.bytes.len() {
            match self.peek()? {
                0x04 => {
                    self.opcode()?;
                    return if self.bytecode.version > 61 {
                        let value = self.expression(host)?;
                        self.value(value, host)
                    } else {
                        Ok(Value::None)
                    };
                }
                0x06 => {
                    self.opcode()?;
                    let target = usize::from(self.read_u16()?);
                    self.jump(target)?;
                }
                0x07 => {
                    self.opcode()?;
                    let target = usize::from(self.read_u16()?);
                    let condition = self.expression(host)?;
                    if !self.value(condition, host)?.truthy()? {
                        self.jump(target)?;
                    }
                }
                0x08 => {
                    self.opcode()?;
                    return Ok(Value::None);
                }
                _ => {
                    let expression = self.expression(host)?;
                    self.value(expression, host)?;
                }
            }
        }
        Ok(Value::None)
    }

    fn expression(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Expression> {
        if self.expression_depth >= MAX_EXPRESSION_DEPTH {
            return Err(Error::ExpressionDepth {
                offset: self.instruction_pointer,
            });
        }
        self.expression_depth += 1;
        let result = self.expression_inner(host);
        self.expression_depth -= 1;
        result
    }

    fn expression_inner(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Expression> {
        let offset = self.instruction_pointer;
        let opcode = self.opcode()?;
        Ok(match opcode {
            0x00 => Expression::Slot(Slot::Local(self.read_i32()?)),
            0x01 => Expression::Slot(Slot::Instance {
                receiver: self.current_context,
                field: self.read_i32()?,
            }),
            0x02 => Expression::Slot(Slot::Default(self.read_i32()?)),
            0x0b | 0x15 => Expression::Value(Value::None),
            0x0f | 0x14 => {
                let target = self.expression(host)?;
                let value_expression = self.expression(host)?;
                let value = self.value(value_expression, host)?;
                self.assign(target, value.clone(), host)?;
                Expression::Value(value)
            }
            0x17 => Expression::Value(Value::Object(-1)),
            0x18 => {
                self.read_u16()?;
                self.expression(host)?
            }
            0x19 => {
                let object = self.expression(host)?;
                let object = self.value(object, host)?;
                let null_skip = usize::from(self.read_u16()?);
                let zero_fill_size = self.read_u8()?;
                match object {
                    Value::None | Value::Object(0) => {
                        self.read(null_skip)?;
                        Expression::Value(if zero_fill_size == 12 {
                            Value::Vector([0.0; 3])
                        } else {
                            Value::None
                        })
                    }
                    Value::Object(object) if object != 0 => {
                        let previous = self.current_context;
                        self.current_context = object;
                        let result = self.expression(host);
                        self.current_context = previous;
                        result?
                    }
                    value => {
                        return Err(Error::Type {
                            expected: "object context",
                            actual: value.kind(),
                        });
                    }
                }
            }
            0x1b => {
                let name = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Virtual(name), host)?)
            }
            0x1c => {
                let function = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Final(function), host)?)
            }
            0x1d => Expression::Value(Value::Int(self.read_i32()?)),
            0x1e => Expression::Value(Value::Float(self.read_f32()?)),
            0x1f => Expression::Value(Value::String(self.read_ascii_z()?)),
            0x20 => Expression::Value(Value::Object(self.read_i32()?)),
            0x21 => Expression::Value(Value::Name(self.read_i32()?)),
            0x22 => Expression::Value(Value::Rotator([
                self.read_i32()?,
                self.read_i32()?,
                self.read_i32()?,
            ])),
            0x23 => Expression::Value(Value::Vector([
                self.read_f32()?,
                self.read_f32()?,
                self.read_f32()?,
            ])),
            0x24 => Expression::Value(Value::Byte(self.read_u8()?)),
            0x2c => Expression::Value(Value::Int(i32::from(self.read_u8()?))),
            0x25 => Expression::Value(Value::Int(0)),
            0x26 => Expression::Value(Value::Int(1)),
            0x27 => Expression::Value(Value::Bool(true)),
            0x28 => Expression::Value(Value::Bool(false)),
            0x2a => Expression::Value(Value::Object(0)),
            0x2b => {
                self.read_u8()?;
                self.expression(host)?
            }
            0x2d => {
                let value = self.expression(host)?;
                match value {
                    Expression::Slot(slot) => Expression::Slot(slot),
                    value => Expression::Value(Value::Bool(self.value(value, host)?.truthy()?)),
                }
            }
            0x36 => {
                let field = self.read_i32()?;
                let target = self.expression(host)?;
                let member = self
                    .struct_members
                    .get(&field)
                    .copied()
                    .ok_or(Error::MissingStructMember { field })?;
                match target {
                    Expression::Slot(target) => Expression::Slot(Slot::StructMember {
                        target: Box::new(target),
                        member,
                    }),
                    target => Expression::Value(member.get(self.value(target, host)?)?),
                }
            }
            0x38 => {
                let name = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Global(name), host)?)
            }
            0x39..=0x60 => {
                let value = self.expression(host)?;
                Expression::Value(convert(opcode, self.value(value, host)?)?)
            }
            0x61..=0x6f => {
                let low = self.read_u8()?;
                let index = (u16::from(opcode - 0x60) << 8) | u16::from(low);
                Expression::Value(self.call(FunctionCall::Native(index), host)?)
            }
            0x70..=0xff => {
                Expression::Value(self.call(FunctionCall::Native(u16::from(opcode)), host)?)
            }
            _ => return Err(Error::UnsupportedOpcode { offset, opcode }),
        })
    }

    fn call(
        &mut self,
        function: FunctionCall,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Value> {
        let mut arguments = Vec::new();
        while self.peek()? != 0x16 {
            let argument = self.expression(host)?;
            arguments.push(self.value(argument, host)?);
        }
        self.opcode()?;
        host(
            FrameRequest::Call {
                receiver: self.current_context,
                function,
                arguments,
            },
            &mut self.instance,
        )
        .map_err(|message| Error::Call {
            call: function,
            message,
        })
    }

    fn value(
        &mut self,
        expression: Expression,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Value> {
        match expression {
            Expression::Value(value) => Ok(value),
            Expression::Slot(slot) => Ok(self.slot(&slot, host)?.unwrap_or(Value::None)),
        }
    }

    fn assign(
        &mut self,
        expression: Expression,
        value: Value,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<()> {
        let Expression::Slot(slot) = expression else {
            return Err(Error::NotAssignable);
        };
        self.assign_slot(slot, value, host)
    }

    fn assign_slot(
        &mut self,
        slot: Slot,
        value: Value,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<()> {
        match slot {
            Slot::Local(field) => {
                self.locals.insert(field, value);
            }
            Slot::Instance {
                receiver: -1,
                field,
            } => {
                self.instance.insert(field, value);
            }
            Slot::Instance { receiver, field } => {
                host(
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    },
                    &mut self.instance,
                )
                .map_err(|message| Error::Context { message })?;
            }
            Slot::Default(field) => {
                self.defaults.insert(field, value);
            }
            Slot::StructMember { target, member } => {
                let mut target_value = self.slot(&target, host)?.unwrap_or(Value::None);
                member.set(&mut target_value, value)?;
                self.assign_slot(*target, target_value, host)?;
            }
        }
        Ok(())
    }

    fn slot(
        &mut self,
        slot: &Slot,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<Value, String>,
    ) -> Result<Option<Value>> {
        Ok(match slot {
            Slot::Local(field) => self.locals.get(field).cloned(),
            Slot::Instance {
                receiver: -1,
                field,
            } => self.instance.get(field).cloned(),
            Slot::Instance { receiver, field } => Some(
                host(
                    FrameRequest::GetInstance {
                        receiver: *receiver,
                        field: *field,
                    },
                    &mut self.instance,
                )
                .map_err(|message| Error::Context { message })?,
            ),
            Slot::Default(field) => self.defaults.get(field).cloned(),
            Slot::StructMember { target, member } => self
                .slot(target, host)?
                .map(|value| member.get(value))
                .transpose()?,
        })
    }

    fn opcode(&mut self) -> Result<u8> {
        if self.steps >= self.step_limit {
            return Err(Error::StepLimit {
                limit: self.step_limit,
            });
        }
        self.steps += 1;
        self.read_u8()
    }

    fn peek(&self) -> Result<u8> {
        self.bytecode
            .bytes
            .get(self.instruction_pointer)
            .copied()
            .ok_or(Error::UnexpectedEnd {
                offset: self.instruction_pointer,
                needed: 1,
            })
    }

    fn jump(&mut self, target: usize) -> Result<()> {
        if target > self.bytecode.bytes.len() {
            return Err(Error::InvalidJump {
                target,
                length: self.bytecode.bytes.len(),
            });
        }
        self.instruction_pointer = target;
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8> {
        let bytes = self.read(1)?;
        Ok(bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read(2)?.try_into().unwrap()))
    }

    fn read_i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }

    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }

    fn read_ascii_z(&mut self) -> Result<String> {
        let start = self.instruction_pointer;
        let remaining = &self.bytecode.bytes[start..];
        let Some(length) = remaining.iter().position(|byte| *byte == 0) else {
            return Err(Error::UnexpectedEnd {
                offset: start,
                needed: 1,
            });
        };
        self.instruction_pointer += length + 1;
        Ok(String::from_utf8_lossy(&remaining[..length]).into_owned())
    }

    fn read(&mut self, length: usize) -> Result<&[u8]> {
        let start = self.instruction_pointer;
        let end = start.checked_add(length).ok_or(Error::UnexpectedEnd {
            offset: start,
            needed: length,
        })?;
        let bytes = self
            .bytecode
            .bytes
            .get(start..end)
            .ok_or(Error::UnexpectedEnd {
                offset: start,
                needed: length,
            })?;
        self.instruction_pointer = end;
        Ok(bytes)
    }
}

impl StructMember {
    fn get(self, value: Value) -> Result<Value> {
        Ok(match (self, value) {
            (Self::X, Value::Vector(value)) => Value::Float(value[0]),
            (Self::Y, Value::Vector(value)) => Value::Float(value[1]),
            (Self::Z, Value::Vector(value)) => Value::Float(value[2]),
            (Self::Pitch, Value::Rotator(value)) => Value::Int(value[0]),
            (Self::Yaw, Value::Rotator(value)) => Value::Int(value[1]),
            (Self::Roll, Value::Rotator(value)) => Value::Int(value[2]),
            (_, value) => {
                return Err(Error::Type {
                    expected: "matching vector or rotator",
                    actual: value.kind(),
                });
            }
        })
    }

    fn set(self, target: &mut Value, value: Value) -> Result<()> {
        match (self, target, value) {
            (Self::X, Value::Vector(target), Value::Float(value)) => target[0] = value,
            (Self::Y, Value::Vector(target), Value::Float(value)) => target[1] = value,
            (Self::Z, Value::Vector(target), Value::Float(value)) => target[2] = value,
            (Self::Pitch, Value::Rotator(target), Value::Int(value)) => target[0] = value,
            (Self::Yaw, Value::Rotator(target), Value::Int(value)) => target[1] = value,
            (Self::Roll, Value::Rotator(target), Value::Int(value)) => target[2] = value,
            (_, target, value) => {
                return Err(Error::Type {
                    expected: match target {
                        Value::Vector(_) => "float",
                        Value::Rotator(_) => "int",
                        _ => "matching vector or rotator",
                    },
                    actual: value.kind(),
                });
            }
        }
        Ok(())
    }
}

fn convert(opcode: u8, value: Value) -> Result<Value> {
    Ok(match (opcode, value) {
        (0x3a, Value::Byte(value)) => Value::Int(i32::from(value)),
        (0x3b, Value::Byte(value)) => Value::Bool(value != 0),
        (0x3c, Value::Byte(value)) => Value::Float(f32::from(value)),
        (0x3d, Value::Int(value)) => Value::Byte(value as u8),
        (0x3e, Value::Int(value)) => Value::Bool(value != 0),
        (0x3f, Value::Int(value)) => Value::Float(value as f32),
        (0x40, Value::Bool(value)) => Value::Byte(u8::from(value)),
        (0x41, Value::Bool(value)) => Value::Int(i32::from(value)),
        (0x42, Value::Bool(value)) => Value::Float(f32::from(value)),
        (0x43, Value::Float(value)) => Value::Byte(value as u8),
        (0x44, Value::Float(value)) => Value::Int(value as i32),
        (0x45, Value::Float(value)) => Value::Bool(value != 0.0),
        (0x47, Value::Object(value)) => Value::Bool(value != 0),
        (0x48, Value::Name(value)) => Value::Bool(value != 0),
        (0x48, Value::NameText(value)) => Value::Bool(!value.eq_ignore_ascii_case("None")),
        (_, value) => {
            return Err(Error::Type {
                expected: "supported conversion input",
                actual: value.kind(),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use openhp1_script::Bytecode;

    use super::*;

    #[test]
    fn executes_assignment_native_call_and_return() {
        let mut bytes = vec![0x0f, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(41_i32.to_le_bytes());
        bytes.extend([0x04, 0x92, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        bytes.extend([0x26, 0x16]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        let result = frame
            .execute(|call, arguments| match (call, arguments) {
                (FunctionCall::Native(0x92), [Value::Int(left), Value::Int(right)]) => {
                    Ok(Value::Int(left + right))
                }
                _ => Err("unexpected native call".to_owned()),
            })
            .unwrap();
        assert_eq!(result, Value::Int(42));
        assert_eq!(frame.local(7), Some(&Value::Int(41)));
    }

    #[test]
    fn takes_conditional_jump_using_canonical_offsets() {
        let bytecode = Bytecode {
            version: 76,
            bytes: vec![
                0x07, 0x08, 0x00, 0x28, 0x04, 0x26, 0x04, 0x25, 0x04, 0x2c, 42,
            ],
            raw_len: 11,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn converts_compact_int_constant_to_float() {
        let bytecode = Bytecode {
            version: 76,
            bytes: vec![0x04, 0x3f, 0x2c, 100],
            raw_len: 4,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Float(100.0)
        );
    }

    #[test]
    fn reads_and_writes_vector_struct_members() {
        let mut bytes = vec![0x0f, 0x36];
        bytes.extend(9_i32.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(42.0_f32.to_le_bytes());
        bytes.extend([0x04, 0x36]);
        bytes.extend(9_i32.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Vector([1.0, 2.0, 3.0]));
        frame.set_struct_member(9, StructMember::Z);
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Float(42.0)
        );
        assert_eq!(frame.local(7), Some(&Value::Vector([1.0, 2.0, 42.0])));
    }

    #[test]
    fn bool_variable_remains_assignable() {
        let mut bytes = vec![0x14, 0x2d, 0x01];
        bytes.extend(7_i32.to_le_bytes());
        bytes.extend([0x28, 0x04, 0x2d, 0x01]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut instance = HashMap::from([(7, Value::Bool(true))]);
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame
                .execute_with_instance(&mut instance, |_, _| unreachable!())
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(instance.get(&7), Some(&Value::Bool(false)));
    }

    #[test]
    fn context_reads_and_writes_remote_instance_fields() {
        let mut context = vec![0x19, 0x20];
        context.extend(1_i32.to_le_bytes());
        context.extend(5_u16.to_le_bytes());
        context.push(4);
        context.push(0x01);
        context.extend(7_i32.to_le_bytes());
        let mut bytes = vec![0x0f];
        bytes.extend(&context);
        bytes.push(0x1d);
        bytes.extend(42_i32.to_le_bytes());
        bytes.push(0x04);
        bytes.extend(context);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut instance = HashMap::new();
        let mut remote = HashMap::new();
        let mut frame = Frame::new(&bytecode);
        let result = frame
            .execute_with_instance(&mut instance, |request, _| match request {
                FrameRequest::GetInstance { receiver: 1, field } => {
                    Ok(remote.get(&field).cloned().unwrap_or(Value::None))
                }
                FrameRequest::SetInstance {
                    receiver: 1,
                    field,
                    value,
                } => {
                    remote.insert(field, value);
                    Ok(Value::None)
                }
                _ => unreachable!(),
            })
            .unwrap();
        assert_eq!(result, Value::Int(42));
        assert_eq!(remote.get(&7), Some(&Value::Int(42)));
    }

    #[test]
    fn stops_runaway_script() {
        let bytecode = Bytecode {
            version: 76,
            bytes: vec![0x06, 0, 0],
            raw_len: 3,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_step_limit(3);
        assert_eq!(
            frame.execute(|_, _| unreachable!()),
            Err(Error::StepLimit { limit: 3 })
        );
    }

    #[test]
    fn stops_runaway_expression_nesting() {
        let mut bytes = vec![0x3f; MAX_EXPRESSION_DEPTH + 1];
        bytes.push(0x25);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame.execute(|_, _| unreachable!()),
            Err(Error::ExpressionDepth {
                offset: MAX_EXPRESSION_DEPTH
            })
        );
    }

    #[test]
    fn self_context_persists_instance_state_and_null_context_short_circuits() {
        let mut bytes = vec![0x0f, 0x19, 0x17, 0, 0, 4, 0x01];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(42_i32.to_le_bytes());
        bytes.extend([0x04, 0x01]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut instance = HashMap::new();
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame
                .execute_with_instance(&mut instance, |_, _| unreachable!())
                .unwrap(),
            Value::Int(42)
        );
        assert_eq!(instance.get(&7), Some(&Value::Int(42)));

        let bytecode = Bytecode {
            version: 76,
            raw_len: 11,
            bytes: vec![0x04, 0x19, 0x2a, 5, 0, 4, 0x1d, 42, 0, 0, 0],
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        assert_eq!(
            frame
                .execute_with_instance(&mut instance, |_, _| unreachable!())
                .unwrap(),
            Value::None
        );
        assert_eq!(instance.get(&7), Some(&Value::Int(42)));
    }
}
