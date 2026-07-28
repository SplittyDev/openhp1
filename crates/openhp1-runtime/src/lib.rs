//! Initial UnrealScript execution core for package-backed runtime objects.

use std::collections::HashMap;

use openhp1_script::Bytecode;
use thiserror::Error;

mod world;

pub use world::{ActorAction, DispatchError, DispatchResult, PlayerMusic, ScriptRuntime};

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
    Struct(HashMap<String, Value>),
    Array(Vec<Value>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    pub base_x: f32,
    pub base_y: f32,
    pub strafe: f32,
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub alt_fire: bool,
    pub jump: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub actor: usize,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub fov_degrees: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FunctionCall {
    Native(u16),
    Virtual(i32),
    Final(i32),
    Global(i32),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Opcode {
    LocalVariable,
    InstanceVariable,
    DefaultVariable,
    Return,
    Switch,
    Jump,
    JumpIfNot,
    Stop,
    Case,
    Nothing,
    LabelTable,
    GotoLabel,
    Let,
    LetBool,
    DynArrayElement,
    DynArrayToInt,
    Unknown0x15,
    EndFunctionParms,
    SelfObject,
    Skip,
    ClassContext,
    Context,
    ArrayElement,
    VirtualFunction,
    FinalFunction,
    IntConst,
    FloatConst,
    StringConst,
    ObjectConst,
    NameConst,
    RotationConst,
    VectorConst,
    ByteConst,
    IntZero,
    IntOne,
    True,
    False,
    NoObject,
    Unknown0x2b,
    IntConstByte,
    BoolVariable,
    DynamicCast,
    Iterator,
    IteratorPop,
    IteratorNext,
    StructMember,
    GlobalFunction,
    Conversion(ConversionOpcode),
    ExtendedNative(u16),
    Native(u16),
    Unsupported,
}

impl From<u8> for Opcode {
    fn from(opcode: u8) -> Self {
        match opcode {
            0x00 => Self::LocalVariable,
            0x01 => Self::InstanceVariable,
            0x02 => Self::DefaultVariable,
            0x04 => Self::Return,
            0x05 => Self::Switch,
            0x06 => Self::Jump,
            0x07 => Self::JumpIfNot,
            0x08 => Self::Stop,
            0x0a => Self::Case,
            0x0b => Self::Nothing,
            0x0c => Self::LabelTable,
            0x0d => Self::GotoLabel,
            0x0f => Self::Let,
            0x10 => Self::DynArrayElement,
            0x14 => Self::LetBool,
            0x15 => Self::Unknown0x15,
            0x16 => Self::EndFunctionParms,
            0x17 => Self::SelfObject,
            0x18 => Self::Skip,
            0x12 => Self::ClassContext,
            0x19 => Self::Context,
            0x1a => Self::ArrayElement,
            0x1b => Self::VirtualFunction,
            0x1c => Self::FinalFunction,
            0x1d => Self::IntConst,
            0x1e => Self::FloatConst,
            0x1f => Self::StringConst,
            0x20 => Self::ObjectConst,
            0x21 => Self::NameConst,
            0x22 => Self::RotationConst,
            0x23 => Self::VectorConst,
            0x24 => Self::ByteConst,
            0x25 => Self::IntZero,
            0x26 => Self::IntOne,
            0x27 => Self::True,
            0x28 => Self::False,
            0x2a => Self::NoObject,
            0x2b => Self::Unknown0x2b,
            0x2c => Self::IntConstByte,
            0x2d => Self::BoolVariable,
            0x2e => Self::DynamicCast,
            0x2f => Self::Iterator,
            0x30 => Self::IteratorPop,
            0x31 => Self::IteratorNext,
            0x36 => Self::StructMember,
            0x37 => Self::DynArrayToInt,
            0x38 => Self::GlobalFunction,
            0x39..=0x60 => Self::Conversion(opcode.into()),
            0x61..=0x6f => Self::ExtendedNative(u16::from(opcode - 0x60) << 8),
            0x70..=0xff => Self::Native(u16::from(opcode)),
            _ => Self::Unsupported,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ConversionOpcode {
    RotatorToVector,
    ByteToInt,
    ByteToBool,
    ByteToFloat,
    IntToByte,
    IntToBool,
    IntToFloat,
    BoolToByte,
    BoolToInt,
    BoolToFloat,
    FloatToByte,
    FloatToInt,
    FloatToBool,
    ObjectToBool,
    NameToBool,
    StringToByte,
    StringToInt,
    StringToBool,
    StringToFloat,
    StringToVector,
    StringToRotator,
    VectorToBool,
    VectorToRotator,
    RotatorToBool,
    ByteToString,
    IntToString,
    BoolToString,
    FloatToString,
    ObjectToString,
    NameToString,
    VectorToString,
    RotatorToString,
    StringToName,
    Unsupported,
}

impl From<u8> for ConversionOpcode {
    fn from(opcode: u8) -> Self {
        match opcode {
            0x39 => Self::RotatorToVector,
            0x3a => Self::ByteToInt,
            0x3b => Self::ByteToBool,
            0x3c => Self::ByteToFloat,
            0x3d => Self::IntToByte,
            0x3e => Self::IntToBool,
            0x3f => Self::IntToFloat,
            0x40 => Self::BoolToByte,
            0x41 => Self::BoolToInt,
            0x42 => Self::BoolToFloat,
            0x43 => Self::FloatToByte,
            0x44 => Self::FloatToInt,
            0x45 => Self::FloatToBool,
            0x47 => Self::ObjectToBool,
            0x48 => Self::NameToBool,
            0x49 => Self::StringToByte,
            0x4a => Self::StringToInt,
            0x4b => Self::StringToBool,
            0x4c => Self::StringToFloat,
            0x4d => Self::StringToVector,
            0x4e => Self::StringToRotator,
            0x4f => Self::VectorToBool,
            0x50 => Self::VectorToRotator,
            0x51 => Self::RotatorToBool,
            0x52 => Self::ByteToString,
            0x53 => Self::IntToString,
            0x54 => Self::BoolToString,
            0x55 => Self::FloatToString,
            0x56 => Self::ObjectToString,
            0x57 => Self::NameToString,
            0x58 => Self::VectorToString,
            0x59 => Self::RotatorToString,
            0x5a => Self::StringToName,
            _ => Self::Unsupported,
        }
    }
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

#[derive(Clone, Debug)]
pub(crate) enum StructMember {
    X,
    Y,
    Z,
    Pitch,
    Yaw,
    Roll,
    Field { name: String, zero: Value },
}

#[derive(Clone, Debug)]
enum Slot {
    Discard(Value),
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
    ArrayElement {
        target: Box<Slot>,
        index: i32,
    },
    DynArrayElement {
        target: Box<Slot>,
        index: i32,
    },
}

#[derive(Clone)]
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
    CallIterator {
        receiver: i32,
        function: FunctionCall,
        arguments: Vec<Value>,
    },
    DynamicCast {
        class: i32,
        value: Value,
    },
    ObjectToString {
        value: Value,
    },
    ResolveObject {
        reference: i32,
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

pub(crate) enum FrameResponse {
    Value(Value),
    ValueWithOutputs {
        value: Value,
        outputs: Vec<(usize, Value)>,
    },
    Iterator(Vec<IteratorValue>),
    Suspend(Value),
}

#[derive(Clone)]
pub(crate) struct IteratorValue {
    value: Value,
    outputs: Vec<(usize, Value)>,
}

impl FrameResponse {
    fn into_value(self) -> std::result::Result<Value, String> {
        match self {
            Self::Value(value) | Self::ValueWithOutputs { value, .. } | Self::Suspend(value) => {
                Ok(value)
            }
            Self::Iterator(_) => Err("regular call returned an iterator".to_owned()),
        }
    }

    fn into_iterator(self) -> std::result::Result<Vec<IteratorValue>, String> {
        match self {
            Self::Iterator(values) => Ok(values),
            Self::Value(_) | Self::ValueWithOutputs { .. } | Self::Suspend(_) => {
                Err("iterator call returned a regular value".to_owned())
            }
        }
    }
}

struct PendingIterator {
    target: Slot,
    output_targets: Vec<(usize, Slot)>,
    values: Vec<IteratorValue>,
}

struct ActiveIterator {
    target: Slot,
    output_targets: Vec<(usize, Slot)>,
    values: std::vec::IntoIter<IteratorValue>,
    start: usize,
    end: usize,
}

pub(crate) struct FrameSnapshot {
    instruction_pointer: usize,
    locals: HashMap<i32, Value>,
    iterators: Vec<ActiveIterator>,
}

impl FrameSnapshot {
    pub(crate) fn at(instruction_pointer: usize) -> Self {
        Self {
            instruction_pointer,
            locals: HashMap::new(),
            iterators: Vec::new(),
        }
    }
}

pub(crate) enum FrameRun {
    Complete(Value),
    Stopped,
    Suspended,
    GotoLabel(Value),
}

/// Mutable state for one UnrealScript frame.
pub struct Frame<'a> {
    bytecode: &'a Bytecode,
    instruction_pointer: usize,
    steps: usize,
    step_limit: usize,
    expression_depth: usize,
    current_context: i32,
    context_parents: Vec<i32>,
    locals: HashMap<i32, Value>,
    instance: HashMap<i32, Value>,
    defaults: HashMap<i32, Value>,
    struct_members: HashMap<i32, StructMember>,
    hosted_instance: bool,
    creating_iterator: bool,
    suspend_requested: bool,
    pending_iterator: Option<PendingIterator>,
    iterators: Vec<ActiveIterator>,
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
            context_parents: Vec::new(),
            locals: HashMap::new(),
            instance: HashMap::new(),
            defaults: HashMap::new(),
            struct_members: HashMap::new(),
            hosted_instance: false,
            creating_iterator: false,
            suspend_requested: false,
            pending_iterator: None,
            iterators: Vec::new(),
        }
    }

    pub(crate) fn from_snapshot(bytecode: &'a Bytecode, snapshot: FrameSnapshot) -> Self {
        let mut frame = Self::new(bytecode);
        frame.instruction_pointer = snapshot.instruction_pointer;
        frame.locals = snapshot.locals;
        frame.iterators = snapshot.iterators;
        frame
    }

    pub(crate) fn into_snapshot(self) -> FrameSnapshot {
        FrameSnapshot {
            instruction_pointer: self.instruction_pointer,
            locals: self.locals,
            iterators: self.iterators,
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

    pub(crate) fn set_default(&mut self, field: i32, value: Value) {
        self.defaults.insert(field, value);
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
            } => call(function, &arguments).map(FrameResponse::Value),
            FrameRequest::CallIterator { .. } => {
                Err("standalone frames do not host iterators".to_owned())
            }
            FrameRequest::DynamicCast { .. } => {
                Err("standalone frames do not host dynamic casts".to_owned())
            }
            FrameRequest::ObjectToString { .. } => {
                Err("standalone frames do not host object conversions".to_owned())
            }
            FrameRequest::ResolveObject { reference } => {
                Ok(FrameResponse::Value(Value::Object(reference)))
            }
            FrameRequest::Call { receiver, .. }
            | FrameRequest::GetInstance { receiver, .. }
            | FrameRequest::SetInstance { receiver, .. } => {
                Err(format!("object context {receiver} has no frame host"))
            }
        })
    }

    #[cfg(test)]
    fn execute_with_instance(
        &mut self,
        instance: &mut HashMap<i32, Value>,
        mut host: impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Value> {
        std::mem::swap(&mut self.instance, instance);
        let result = self.execute_inner(&mut host);
        std::mem::swap(&mut self.instance, instance);
        result
    }

    pub(crate) fn execute_hosted(
        &mut self,
        mut host: impl FnMut(FrameRequest) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Value> {
        self.hosted_instance = true;
        let result = self.execute_inner(&mut |request, _| host(request));
        self.hosted_instance = false;
        result
    }

    pub(crate) fn resume_hosted(
        &mut self,
        mut host: impl FnMut(FrameRequest) -> std::result::Result<FrameResponse, String>,
    ) -> Result<FrameRun> {
        self.hosted_instance = true;
        let result = self.resume_inner(&mut |request, _| host(request));
        self.hosted_instance = false;
        result
    }

    fn execute_inner(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Value> {
        self.instruction_pointer = 0;
        self.steps = 0;
        self.expression_depth = 0;
        self.current_context = -1;
        self.context_parents.clear();
        self.creating_iterator = false;
        self.suspend_requested = false;
        self.pending_iterator = None;
        self.iterators.clear();
        match self.run_inner(host)? {
            FrameRun::Complete(value) => Ok(value),
            FrameRun::Stopped => Ok(Value::None),
            FrameRun::Suspended | FrameRun::GotoLabel(_) => Err(Error::UnexpectedStateControl),
        }
    }

    fn resume_inner(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<FrameRun> {
        self.steps = 0;
        self.expression_depth = 0;
        self.current_context = -1;
        self.context_parents.clear();
        self.creating_iterator = false;
        self.suspend_requested = false;
        self.pending_iterator = None;
        self.run_inner(host)
    }

    fn run_inner(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<FrameRun> {
        while self.instruction_pointer < self.bytecode.bytes.len() {
            match Opcode::from(self.peek()?) {
                Opcode::Return => {
                    self.opcode()?;
                    let value = if self.bytecode.version > 61 {
                        let value = self.expression(host)?;
                        self.value(value, host)
                    } else {
                        Ok(Value::None)
                    }?;
                    return Ok(FrameRun::Complete(value));
                }
                Opcode::Switch => {
                    self.opcode()?;
                    self.read_u8()?;
                    let condition = self.expression(host)?;
                    let condition = self.value(condition, host)?;
                    loop {
                        let offset = self.instruction_pointer;
                        let opcode = self.opcode()?;
                        if Opcode::from(opcode) != Opcode::Case {
                            return Err(Error::ExpectedCase { offset, opcode });
                        }
                        let next = self.read_u16()?;
                        if next == u16::MAX {
                            break;
                        }
                        let case = self.expression(host)?;
                        let case = self.value(case, host)?;
                        if switch_values_equal(&condition, &case)? {
                            break;
                        }
                        self.jump(usize::from(next))?;
                    }
                }
                Opcode::Case => {
                    self.opcode()?;
                    if self.read_u16()? != u16::MAX {
                        let value = self.expression(host)?;
                        self.value(value, host)?;
                    }
                }
                Opcode::Jump => {
                    self.opcode()?;
                    let target = usize::from(self.read_u16()?);
                    self.jump(target)?;
                }
                Opcode::JumpIfNot => {
                    self.opcode()?;
                    let target = usize::from(self.read_u16()?);
                    let condition = self.expression(host)?;
                    if !self.value(condition, host)?.truthy()? {
                        self.jump(target)?;
                    }
                }
                Opcode::Stop => {
                    self.opcode()?;
                    return Ok(FrameRun::Stopped);
                }
                Opcode::LabelTable => {
                    return Ok(FrameRun::Stopped);
                }
                Opcode::GotoLabel => {
                    self.opcode()?;
                    let label = self.expression(host)?;
                    return Ok(FrameRun::GotoLabel(self.value(label, host)?));
                }
                Opcode::Iterator => {
                    self.opcode()?;
                    self.creating_iterator = true;
                    let result = self.expression(host);
                    self.creating_iterator = false;
                    self.value(result?, host)?;
                    let end = usize::from(self.read_u16()?);
                    let iterator = self.pending_iterator.take().ok_or(Error::MissingIterator)?;
                    self.iterators.push(ActiveIterator {
                        target: iterator.target,
                        output_targets: iterator.output_targets,
                        values: iterator.values.into_iter(),
                        start: self.instruction_pointer,
                        end,
                    });
                    self.next_iterator(host)?;
                }
                Opcode::IteratorPop => {
                    self.opcode()?;
                    self.iterators.pop().ok_or(Error::MissingIterator)?;
                }
                Opcode::IteratorNext => {
                    self.opcode()?;
                    self.next_iterator(host)?;
                }
                _ => {
                    let expression = self.expression(host)?;
                    self.value(expression, host)?;
                }
            }
            if self.suspend_requested {
                return Ok(FrameRun::Suspended);
            }
        }
        Ok(FrameRun::Complete(Value::None))
    }

    fn expression(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
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
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Expression> {
        let offset = self.instruction_pointer;
        let raw_opcode = self.opcode()?;
        let opcode = Opcode::from(raw_opcode);
        Ok(match opcode {
            Opcode::LocalVariable => Expression::Slot(Slot::Local(self.read_i32()?)),
            Opcode::InstanceVariable => Expression::Slot(Slot::Instance {
                receiver: self.current_context,
                field: self.read_i32()?,
            }),
            Opcode::DefaultVariable => Expression::Slot(Slot::Default(self.read_i32()?)),
            Opcode::Nothing | Opcode::Unknown0x15 => Expression::Value(Value::None),
            Opcode::Let | Opcode::LetBool => {
                let target = self.expression(host)?;
                let value_expression = self.expression(host)?;
                let value = self.value(value_expression, host)?;
                self.assign(target, value.clone(), host)?;
                Expression::Value(value)
            }
            Opcode::SelfObject => Expression::Value(Value::Object(-1)),
            Opcode::Skip => {
                self.read_u16()?;
                self.expression(host)?
            }
            Opcode::ClassContext | Opcode::Context => {
                let object = self.expression(host)?;
                let object = self.value(object, host)?;
                let null_skip = usize::from(self.read_u16()?);
                let zero_fill_size = self.read_u8()?;
                match object {
                    Value::None | Value::Object(0) => {
                        self.read(null_skip)?;
                        Expression::Slot(Slot::Discard(if zero_fill_size == 12 {
                            Value::Vector([0.0; 3])
                        } else {
                            Value::None
                        }))
                    }
                    Value::Object(object) if object != 0 => {
                        let previous = self.current_context;
                        self.current_context = object;
                        self.context_parents.push(previous);
                        let result = self.expression(host);
                        self.context_parents.pop();
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
            Opcode::ArrayElement => {
                let index = self.expression(host)?;
                let index = match self.value(index, host)? {
                    Value::Byte(index) => i32::from(index),
                    Value::Int(index) => index,
                    value => {
                        return Err(Error::Type {
                            expected: "integer array index",
                            actual: value.kind(),
                        });
                    }
                };
                let target = self.expression(host)?;
                match target {
                    Expression::Slot(target) => Expression::Slot(Slot::ArrayElement {
                        target: Box::new(target),
                        index,
                    }),
                    target => Expression::Value(array_element(&self.value(target, host)?, index)?),
                }
            }
            Opcode::DynArrayElement => {
                let index = self.expression(host)?;
                let index = match self.value(index, host)? {
                    Value::Byte(index) => i32::from(index),
                    Value::Int(index) => index,
                    value => {
                        return Err(Error::Type {
                            expected: "integer array index",
                            actual: value.kind(),
                        });
                    }
                };
                let Expression::Slot(target) = self.expression(host)? else {
                    return Err(Error::NotAssignable);
                };
                Expression::Slot(Slot::DynArrayElement {
                    target: Box::new(target),
                    index,
                })
            }
            Opcode::VirtualFunction => {
                let name = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Virtual(name), host)?)
            }
            Opcode::FinalFunction => {
                let function = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Final(function), host)?)
            }
            Opcode::IntConst => Expression::Value(Value::Int(self.read_i32()?)),
            Opcode::FloatConst => Expression::Value(Value::Float(self.read_f32()?)),
            Opcode::StringConst => Expression::Value(Value::String(self.read_ascii_z()?)),
            Opcode::ObjectConst => {
                let reference = self.read_i32()?;
                Expression::Value(
                    host(
                        FrameRequest::ResolveObject { reference },
                        &mut self.instance,
                    )
                    .and_then(FrameResponse::into_value)
                    .map_err(|message| Error::Context { message })?,
                )
            }
            Opcode::NameConst => Expression::Value(Value::Name(self.read_i32()?)),
            Opcode::RotationConst => Expression::Value(Value::Rotator([
                self.read_i32()?,
                self.read_i32()?,
                self.read_i32()?,
            ])),
            Opcode::VectorConst => Expression::Value(Value::Vector([
                self.read_f32()?,
                self.read_f32()?,
                self.read_f32()?,
            ])),
            Opcode::ByteConst => Expression::Value(Value::Byte(self.read_u8()?)),
            Opcode::IntConstByte => Expression::Value(Value::Int(i32::from(self.read_u8()?))),
            Opcode::IntZero => Expression::Value(Value::Int(0)),
            Opcode::IntOne => Expression::Value(Value::Int(1)),
            Opcode::True => Expression::Value(Value::Bool(true)),
            Opcode::False => Expression::Value(Value::Bool(false)),
            Opcode::NoObject => Expression::Value(Value::Object(0)),
            Opcode::Unknown0x2b => {
                self.read_u8()?;
                self.expression(host)?
            }
            Opcode::BoolVariable => {
                let value = self.expression(host)?;
                match value {
                    Expression::Slot(slot) => Expression::Slot(slot),
                    value => Expression::Value(Value::Bool(self.value(value, host)?.truthy()?)),
                }
            }
            Opcode::DynamicCast => {
                let class = self.read_i32()?;
                let value = self.expression(host)?;
                let value = self.value(value, host)?;
                Expression::Value(
                    host(
                        FrameRequest::DynamicCast { class, value },
                        &mut self.instance,
                    )
                    .and_then(FrameResponse::into_value)
                    .map_err(|message| Error::Context { message })?,
                )
            }
            Opcode::StructMember => {
                let field = self.read_i32()?;
                let target = self.expression(host)?;
                let member = self
                    .struct_members
                    .get(&field)
                    .cloned()
                    .ok_or(Error::MissingStructMember { field })?;
                match target {
                    Expression::Slot(target) => Expression::Slot(Slot::StructMember {
                        target: Box::new(target),
                        member,
                    }),
                    target => Expression::Value(member.get(self.value(target, host)?)?),
                }
            }
            Opcode::DynArrayToInt => {
                let value = self.expression(host)?;
                let value = self.value(value, host)?;
                let length = match value {
                    Value::Array(values) => values.len(),
                    Value::None => 0,
                    value => {
                        return Err(Error::Type {
                            expected: "array",
                            actual: value.kind(),
                        });
                    }
                };
                Expression::Value(Value::Int(length as i32))
            }
            Opcode::GlobalFunction => {
                let name = self.read_i32()?;
                Expression::Value(self.call(FunctionCall::Global(name), host)?)
            }
            Opcode::Conversion(conversion) => {
                let value = self.expression(host)?;
                let value = self.value(value, host)?;
                if conversion == ConversionOpcode::ObjectToString {
                    Expression::Value(
                        host(FrameRequest::ObjectToString { value }, &mut self.instance)
                            .and_then(FrameResponse::into_value)
                            .map_err(|message| Error::Context { message })?,
                    )
                } else {
                    Expression::Value(convert(conversion, value)?)
                }
            }
            Opcode::ExtendedNative(high) => {
                let low = self.read_u8()?;
                Expression::Value(self.call(FunctionCall::Native(high | u16::from(low)), host)?)
            }
            Opcode::Native(index) => {
                Expression::Value(self.call(FunctionCall::Native(index), host)?)
            }
            _ => {
                return Err(Error::UnsupportedOpcode {
                    offset,
                    opcode: raw_opcode,
                });
            }
        })
    }

    fn call(
        &mut self,
        function: FunctionCall,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Value> {
        let creating_iterator = std::mem::take(&mut self.creating_iterator);
        let receiver = self.current_context;
        let assignment_native = matches!(
            function,
            FunctionCall::Native(index)
                if IncrementDecrement::try_from(index).is_ok()
                    || CompoundAssignment::try_from(index).is_ok()
        );
        if !assignment_native {
            self.current_context = self.context_parents.last().copied().unwrap_or(-1);
        }
        let mut arguments = Vec::new();
        while Opcode::from(self.peek()?) != Opcode::EndFunctionParms {
            arguments.push(self.expression(host)?);
        }
        self.opcode()?;
        self.current_context = receiver;
        if let FunctionCall::Native(index) = function
            && let Ok(operation) = IncrementDecrement::try_from(index)
        {
            let [target] = arguments
                .try_into()
                .map_err(|arguments: Vec<_>| Error::Call {
                    call: function,
                    message: format!(
                        "increment or decrement expects 1 argument, found {}",
                        arguments.len()
                    ),
                })?;
            let current = match &target {
                Expression::Value(value) => value.clone(),
                Expression::Slot(slot) => self.slot(slot, host)?.unwrap_or(Value::None),
            };
            let (stored, returned) = increment_decrement(operation, &current)?;
            self.assign(target, stored, host)?;
            return Ok(returned);
        }
        if let FunctionCall::Native(index) = function
            && let Ok(assignment) = CompoundAssignment::try_from(index)
        {
            let [target, operand] =
                arguments
                    .try_into()
                    .map_err(|arguments: Vec<_>| Error::Call {
                        call: function,
                        message: format!(
                            "compound assignment expects 2 arguments, found {}",
                            arguments.len()
                        ),
                    })?;
            let current = match &target {
                Expression::Value(value) => value.clone(),
                Expression::Slot(slot) => self.slot(slot, host)?.unwrap_or(Value::None),
            };
            let operand = self.value(operand, host)?;
            let value = compound_assignment(assignment, &current, &operand)?;
            self.assign(target, value.clone(), host)?;
            return Ok(value);
        }
        if let FunctionCall::Native(index @ (0xe5 | 0xe6)) = function {
            let [rotation, x, y, z] =
                arguments
                    .try_into()
                    .map_err(|arguments: Vec<_>| Error::Call {
                        call: function,
                        message: format!("GetAxes expects 4 arguments, found {}", arguments.len()),
                    })?;
            let rotation = self.value(rotation, host)?;
            let Value::Rotator(rotation) = rotation else {
                return Err(Error::Type {
                    expected: "rotator",
                    actual: rotation.kind(),
                });
            };
            let mut axes = rotator_axes(rotation);
            if index == 0xe6 {
                axes = [
                    [axes[0][0], axes[1][0], axes[2][0]],
                    [axes[0][1], axes[1][1], axes[2][1]],
                    [axes[0][2], axes[1][2], axes[2][2]],
                ];
            }
            self.assign(x, Value::Vector(axes[0]), host)?;
            self.assign(y, Value::Vector(axes[1]), host)?;
            self.assign(z, Value::Vector(axes[2]), host)?;
            return Ok(Value::None);
        }
        if creating_iterator {
            let target = match arguments.get(1) {
                Some(Expression::Slot(target)) => target.clone(),
                _ => return Err(Error::NotAssignable),
            };
            let output_targets = arguments
                .iter()
                .enumerate()
                .filter_map(|(index, argument)| match argument {
                    Expression::Slot(target) if index != 1 => Some((index, target.clone())),
                    _ => None,
                })
                .collect();
            let mut values = Vec::with_capacity(arguments.len());
            for (index, argument) in arguments.into_iter().enumerate() {
                values.push(if index == 1 {
                    Value::None
                } else {
                    self.value(argument, host)?
                });
            }
            let iterator = host(
                FrameRequest::CallIterator {
                    receiver,
                    function,
                    arguments: values,
                },
                &mut self.instance,
            )
            .and_then(FrameResponse::into_iterator)
            .map_err(|message| Error::Call {
                call: function,
                message,
            })?;
            self.pending_iterator = Some(PendingIterator {
                target,
                output_targets,
                values: iterator,
            });
            return Ok(Value::None);
        }
        let mut values = Vec::with_capacity(arguments.len());
        for argument in &arguments {
            values.push(self.value(argument.clone(), host)?);
        }
        let response = host(
            FrameRequest::Call {
                receiver,
                function,
                arguments: values,
            },
            &mut self.instance,
        )
        .map_err(|message| Error::Call {
            call: function,
            message,
        })?;
        match response {
            FrameResponse::Value(value) => Ok(value),
            FrameResponse::ValueWithOutputs { value, outputs } => {
                for (argument, output) in outputs {
                    let target = arguments
                        .get(argument)
                        .cloned()
                        .ok_or_else(|| Error::Call {
                            call: function,
                            message: format!("output argument {argument} is out of range"),
                        })?;
                    self.assign(target, output, host)?;
                }
                Ok(value)
            }
            FrameResponse::Suspend(value) => {
                self.suspend_requested = true;
                Ok(value)
            }
            FrameResponse::Iterator(_) => Err(Error::Call {
                call: function,
                message: "regular call returned an iterator".to_owned(),
            }),
        }
    }

    fn value(
        &mut self,
        expression: Expression,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
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
        ) -> std::result::Result<FrameResponse, String>,
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
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<()> {
        match slot {
            Slot::Discard(_) => {}
            Slot::Local(field) => {
                self.locals.insert(field, value);
            }
            Slot::Instance {
                receiver: -1,
                field,
            } if self.hosted_instance => {
                host(
                    FrameRequest::SetInstance {
                        receiver: -1,
                        field,
                        value,
                    },
                    &mut self.instance,
                )
                .and_then(FrameResponse::into_value)
                .map_err(|message| Error::Context { message })?;
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
                .and_then(FrameResponse::into_value)
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
            Slot::ArrayElement { target, index } => {
                let mut target_value = self.slot(&target, host)?.unwrap_or(Value::None);
                let Value::Array(values) = &mut target_value else {
                    return Err(Error::Type {
                        expected: "array",
                        actual: target_value.kind(),
                    });
                };
                let length = values.len();
                let element = usize::try_from(index)
                    .ok()
                    .and_then(|index| values.get_mut(index))
                    .ok_or(Error::ArrayIndex { index, length })?;
                *element = value;
                self.assign_slot(*target, target_value, host)?;
            }
            Slot::DynArrayElement { target, index } => {
                let mut target_value = self.slot(&target, host)?.unwrap_or(Value::None);
                if matches!(target_value, Value::None) {
                    target_value = Value::Array(Vec::new());
                }
                let Value::Array(values) = &mut target_value else {
                    return Err(Error::Type {
                        expected: "array",
                        actual: target_value.kind(),
                    });
                };
                let index = usize::try_from(index).map_err(|_| Error::ArrayIndex {
                    index,
                    length: values.len(),
                })?;
                values.resize(index.saturating_add(1), Value::None);
                values[index] = value;
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
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<Option<Value>> {
        Ok(match slot {
            Slot::Discard(value) => Some(value.clone()),
            Slot::Local(field) => self.locals.get(field).cloned(),
            Slot::Instance {
                receiver: -1,
                field,
            } if self.hosted_instance => Some(
                host(
                    FrameRequest::GetInstance {
                        receiver: -1,
                        field: *field,
                    },
                    &mut self.instance,
                )
                .and_then(FrameResponse::into_value)
                .map_err(|message| Error::Context { message })?,
            ),
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
                .and_then(FrameResponse::into_value)
                .map_err(|message| Error::Context { message })?,
            ),
            Slot::Default(field) => self.defaults.get(field).cloned(),
            Slot::StructMember { target, member } => self
                .slot(target, host)?
                .map(|value| member.get(value))
                .transpose()?,
            Slot::ArrayElement { target, index } => self
                .slot(target, host)?
                .map(|value| array_element(&value, *index))
                .transpose()?,
            Slot::DynArrayElement { target, index } => {
                let mut target_value = self.slot(target, host)?.unwrap_or(Value::None);
                if matches!(target_value, Value::None) {
                    target_value = Value::Array(Vec::new());
                }
                let Value::Array(values) = &mut target_value else {
                    return Err(Error::Type {
                        expected: "array",
                        actual: target_value.kind(),
                    });
                };
                let index = usize::try_from(*index).map_err(|_| Error::ArrayIndex {
                    index: *index,
                    length: values.len(),
                })?;
                values.resize(index.saturating_add(1), Value::None);
                let value = values[index].clone();
                self.assign_slot((**target).clone(), target_value, host)?;
                Some(value)
            }
        })
    }

    fn next_iterator(
        &mut self,
        host: &mut impl FnMut(
            FrameRequest,
            &mut HashMap<i32, Value>,
        ) -> std::result::Result<FrameResponse, String>,
    ) -> Result<()> {
        let (target, value, outputs, jump) = {
            let iterator = self.iterators.last_mut().ok_or(Error::MissingIterator)?;
            let value = iterator.values.next();
            let has_value = value.is_some();
            let (value, outputs) = value.map_or_else(
                || (Value::Object(0), Vec::new()),
                |value| {
                    let outputs = value
                        .outputs
                        .into_iter()
                        .filter_map(|(index, value)| {
                            iterator
                                .output_targets
                                .iter()
                                .find(|(target_index, _)| *target_index == index)
                                .map(|(_, target)| (target.clone(), value))
                        })
                        .collect();
                    (value.value, outputs)
                },
            );
            (
                iterator.target.clone(),
                value,
                outputs,
                if has_value {
                    iterator.start
                } else {
                    iterator.end
                },
            )
        };
        self.assign_slot(target, value, host)?;
        for (target, value) in outputs {
            self.assign_slot(target, value, host)?;
        }
        self.jump(jump)
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

fn array_element(value: &Value, index: i32) -> Result<Value> {
    let Value::Array(values) = value else {
        return Err(Error::Type {
            expected: "array",
            actual: value.kind(),
        });
    };
    values
        .get(usize::try_from(index).unwrap_or(usize::MAX))
        .cloned()
        .ok_or(Error::ArrayIndex {
            index,
            length: values.len(),
        })
}

impl StructMember {
    fn get(&self, value: Value) -> Result<Value> {
        Ok(match (self, value) {
            (Self::X, Value::Vector(value)) => Value::Float(value[0]),
            (Self::Y, Value::Vector(value)) => Value::Float(value[1]),
            (Self::Z, Value::Vector(value)) => Value::Float(value[2]),
            (Self::X, Value::Struct(mut values)) => values.remove("X").unwrap_or(Value::Float(0.0)),
            (Self::Y, Value::Struct(mut values)) => values.remove("Y").unwrap_or(Value::Float(0.0)),
            (Self::Z, Value::Struct(mut values)) => values.remove("Z").unwrap_or(Value::Float(0.0)),
            (Self::Pitch, Value::Rotator(value)) => Value::Int(value[0]),
            (Self::Yaw, Value::Rotator(value)) => Value::Int(value[1]),
            (Self::Roll, Value::Rotator(value)) => Value::Int(value[2]),
            (Self::X | Self::Y | Self::Z, Value::None) => Value::Float(0.0),
            (Self::Pitch | Self::Yaw | Self::Roll, Value::None) => Value::Int(0),
            (Self::Field { name, zero }, Value::Struct(mut values)) => {
                values.remove(name).unwrap_or_else(|| zero.clone())
            }
            (Self::Field { zero, .. }, Value::None) => zero.clone(),
            (_, value) => {
                return Err(Error::Type {
                    expected: "matching struct, vector, or rotator",
                    actual: value.kind(),
                });
            }
        })
    }

    fn set(&self, target: &mut Value, value: Value) -> Result<()> {
        if matches!(target, Value::None) {
            *target = match self {
                Self::X | Self::Y | Self::Z => Value::Vector([0.0; 3]),
                Self::Pitch | Self::Yaw | Self::Roll => Value::Rotator([0; 3]),
                Self::Field { .. } => Value::Struct(HashMap::new()),
            };
        }
        if let Self::Field { name, .. } = self {
            if let Value::Struct(values) = target {
                values.insert(name.clone(), value);
                return Ok(());
            }
            return Err(Error::Type {
                expected: "struct",
                actual: target.kind(),
            });
        }
        match (self, target, value) {
            (Self::X, Value::Vector(target), Value::Float(value)) => target[0] = value,
            (Self::Y, Value::Vector(target), Value::Float(value)) => target[1] = value,
            (Self::Z, Value::Vector(target), Value::Float(value)) => target[2] = value,
            (Self::X, Value::Struct(target), Value::Float(value)) => {
                target.insert("X".to_owned(), Value::Float(value));
            }
            (Self::Y, Value::Struct(target), Value::Float(value)) => {
                target.insert("Y".to_owned(), Value::Float(value));
            }
            (Self::Z, Value::Struct(target), Value::Float(value)) => {
                target.insert("Z".to_owned(), Value::Float(value));
            }
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

fn switch_values_equal(condition: &Value, case: &Value) -> Result<bool> {
    Ok(match (condition, case) {
        (Value::None, Value::None | Value::Object(0))
        | (Value::Object(0), Value::None)
        | (Value::Object(0), Value::Object(0)) => true,
        (Value::None, Value::Byte(value)) => *value == 0,
        (Value::None, Value::Int(value)) => *value == 0,
        (Value::None, Value::Bool(value)) => !value,
        (Value::None, Value::Float(value)) => *value == 0.0,
        (Value::None, Value::Name(value)) => *value == 0,
        (Value::None, Value::NameText(value)) => value.eq_ignore_ascii_case("None"),
        (Value::Byte(left), Value::Byte(right)) => left == right,
        (Value::Byte(left), Value::Int(right)) => i32::from(*left) == *right,
        (Value::Int(left), Value::Byte(right)) => *left == i32::from(*right),
        (Value::Int(left), Value::Int(right)) => left == right,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Name(left), Value::Name(right)) => left == right,
        (Value::NameText(left), Value::NameText(right)) => left.eq_ignore_ascii_case(right),
        (Value::Object(left), Value::Object(right)) => left == right,
        (Value::Vector(left), Value::Vector(right)) => left == right,
        (Value::Rotator(left), Value::Rotator(right)) => left == right,
        (_, case) => {
            return Err(Error::Type {
                expected: condition.kind(),
                actual: case.kind(),
            });
        }
    })
}

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum IncrementDecrement {
    AddAdd_PreByte,
    SubtractSubtract_PreByte,
    AddAdd_Byte,
    SubtractSubtract_Byte,
    AddAdd_PreInt,
    SubtractSubtract_PreInt,
    AddAdd_Int,
    SubtractSubtract_Int,
}

impl TryFrom<u16> for IncrementDecrement {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0x89 => Ok(Self::AddAdd_PreByte),
            0x8a => Ok(Self::SubtractSubtract_PreByte),
            0x8b => Ok(Self::AddAdd_Byte),
            0x8c => Ok(Self::SubtractSubtract_Byte),
            0xa3 => Ok(Self::AddAdd_PreInt),
            0xa4 => Ok(Self::SubtractSubtract_PreInt),
            0xa5 => Ok(Self::AddAdd_Int),
            0xa6 => Ok(Self::SubtractSubtract_Int),
            _ => Err(()),
        }
    }
}

fn increment_decrement(operation: IncrementDecrement, current: &Value) -> Result<(Value, Value)> {
    let (stored, prefix) = match (operation, current) {
        (IncrementDecrement::AddAdd_PreByte, Value::Byte(value)) => {
            (Value::Byte(value.wrapping_add(1)), true)
        }
        (IncrementDecrement::SubtractSubtract_PreByte, Value::Byte(value)) => {
            (Value::Byte(value.wrapping_sub(1)), true)
        }
        (IncrementDecrement::AddAdd_Byte, Value::Byte(value)) => {
            (Value::Byte(value.wrapping_add(1)), false)
        }
        (IncrementDecrement::SubtractSubtract_Byte, Value::Byte(value)) => {
            (Value::Byte(value.wrapping_sub(1)), false)
        }
        (IncrementDecrement::AddAdd_PreInt, Value::Int(value)) => {
            (Value::Int(value.wrapping_add(1)), true)
        }
        (IncrementDecrement::SubtractSubtract_PreInt, Value::Int(value)) => {
            (Value::Int(value.wrapping_sub(1)), true)
        }
        (IncrementDecrement::AddAdd_Int, Value::Int(value)) => {
            (Value::Int(value.wrapping_add(1)), false)
        }
        (IncrementDecrement::SubtractSubtract_Int, Value::Int(value)) => {
            (Value::Int(value.wrapping_sub(1)), false)
        }
        (_, value) => {
            return Err(Error::Type {
                expected: "matching byte or int increment operand",
                actual: value.kind(),
            });
        }
    };
    let returned = if prefix {
        stored.clone()
    } else {
        current.clone()
    };
    Ok((stored, returned))
}

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum CompoundAssignment {
    AddEqual_IntInt,
    MultiplyEqual_FloatFloat,
    DivideEqual_FloatFloat,
    AddEqual_FloatFloat,
    SubtractEqual_FloatFloat,
    MultiplyEqual_VectorFloat,
    DivideEqual_VectorFloat,
    AddEqual_VectorVector,
    SubtractEqual_VectorVector,
}

impl TryFrom<u16> for CompoundAssignment {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0xa1 => Ok(Self::AddEqual_IntInt),
            0xb6 => Ok(Self::MultiplyEqual_FloatFloat),
            0xb7 => Ok(Self::DivideEqual_FloatFloat),
            0xb8 => Ok(Self::AddEqual_FloatFloat),
            0xb9 => Ok(Self::SubtractEqual_FloatFloat),
            0xdd => Ok(Self::MultiplyEqual_VectorFloat),
            0xde => Ok(Self::DivideEqual_VectorFloat),
            0xdf => Ok(Self::AddEqual_VectorVector),
            0xe0 => Ok(Self::SubtractEqual_VectorVector),
            _ => Err(()),
        }
    }
}

fn compound_assignment(
    assignment: CompoundAssignment,
    left: &Value,
    right: &Value,
) -> Result<Value> {
    if matches!(left, Value::None) {
        let zero = match assignment {
            CompoundAssignment::AddEqual_IntInt => Value::Int(0),
            CompoundAssignment::MultiplyEqual_FloatFloat
            | CompoundAssignment::DivideEqual_FloatFloat
            | CompoundAssignment::AddEqual_FloatFloat
            | CompoundAssignment::SubtractEqual_FloatFloat => Value::Float(0.0),
            _ => Value::Vector([0.0; 3]),
        };
        return compound_assignment(assignment, &zero, right);
    }
    Ok(match (assignment, left, right) {
        (CompoundAssignment::AddEqual_IntInt, Value::Int(left), Value::Int(right)) => {
            Value::Int(left.wrapping_add(*right))
        }
        (CompoundAssignment::MultiplyEqual_FloatFloat, Value::Float(left), Value::Float(right)) => {
            Value::Float(left * right)
        }
        (CompoundAssignment::DivideEqual_FloatFloat, Value::Float(left), Value::Float(right)) => {
            Value::Float(left / right)
        }
        (CompoundAssignment::AddEqual_FloatFloat, Value::Float(left), Value::Float(right)) => {
            Value::Float(left + right)
        }
        (CompoundAssignment::SubtractEqual_FloatFloat, Value::Float(left), Value::Float(right)) => {
            Value::Float(left - right)
        }
        (
            CompoundAssignment::MultiplyEqual_VectorFloat,
            Value::Vector(left),
            Value::Float(right),
        ) => Value::Vector([left[0] * right, left[1] * right, left[2] * right]),
        (CompoundAssignment::DivideEqual_VectorFloat, Value::Vector(left), Value::Float(right)) => {
            Value::Vector([left[0] / right, left[1] / right, left[2] / right])
        }
        (CompoundAssignment::AddEqual_VectorVector, Value::Vector(left), Value::Vector(right)) => {
            Value::Vector([left[0] + right[0], left[1] + right[1], left[2] + right[2]])
        }
        (
            CompoundAssignment::SubtractEqual_VectorVector,
            Value::Vector(left),
            Value::Vector(right),
        ) => Value::Vector([left[0] - right[0], left[1] - right[1], left[2] - right[2]]),
        (_, left, right) => {
            return Err(Error::Type {
                expected: "matching compound arithmetic operands",
                actual: if left.kind() != "none" {
                    left.kind()
                } else {
                    right.kind()
                },
            });
        }
    })
}

fn convert(opcode: ConversionOpcode, value: Value) -> Result<Value> {
    let value = if matches!(value, Value::None) {
        match opcode {
            ConversionOpcode::RotatorToVector
            | ConversionOpcode::RotatorToBool
            | ConversionOpcode::RotatorToString => Value::Rotator([0; 3]),
            ConversionOpcode::ByteToInt
            | ConversionOpcode::ByteToBool
            | ConversionOpcode::ByteToFloat
            | ConversionOpcode::ByteToString => Value::Byte(0),
            ConversionOpcode::IntToByte
            | ConversionOpcode::IntToBool
            | ConversionOpcode::IntToFloat
            | ConversionOpcode::IntToString => Value::Int(0),
            ConversionOpcode::BoolToByte
            | ConversionOpcode::BoolToInt
            | ConversionOpcode::BoolToFloat
            | ConversionOpcode::BoolToString => Value::Bool(false),
            ConversionOpcode::FloatToByte
            | ConversionOpcode::FloatToInt
            | ConversionOpcode::FloatToBool
            | ConversionOpcode::FloatToString => Value::Float(0.0),
            ConversionOpcode::ObjectToBool => Value::Object(0),
            ConversionOpcode::NameToBool | ConversionOpcode::NameToString => {
                Value::NameText("None".to_owned())
            }
            ConversionOpcode::StringToByte
            | ConversionOpcode::StringToInt
            | ConversionOpcode::StringToBool
            | ConversionOpcode::StringToFloat
            | ConversionOpcode::StringToVector
            | ConversionOpcode::StringToRotator
            | ConversionOpcode::StringToName => Value::String(String::new()),
            ConversionOpcode::VectorToBool
            | ConversionOpcode::VectorToRotator
            | ConversionOpcode::VectorToString => Value::Vector([0.0; 3]),
            ConversionOpcode::ObjectToString | ConversionOpcode::Unsupported => Value::None,
        }
    } else {
        value
    };
    Ok(match (opcode, value) {
        (ConversionOpcode::RotatorToVector, Value::Rotator([pitch, yaw, _])) => {
            let units_to_radians = std::f32::consts::TAU / 65_536.0;
            let (pitch_sin, pitch_cos) = ((pitch as f32) * units_to_radians).sin_cos();
            let (yaw_sin, yaw_cos) = ((yaw as f32) * units_to_radians).sin_cos();
            Value::Vector([pitch_cos * yaw_cos, pitch_cos * yaw_sin, -pitch_sin])
        }
        (ConversionOpcode::ByteToInt, Value::Byte(value)) => Value::Int(i32::from(value)),
        (ConversionOpcode::ByteToBool, Value::Byte(value)) => Value::Bool(value != 0),
        (ConversionOpcode::ByteToFloat, Value::Byte(value)) => Value::Float(f32::from(value)),
        (ConversionOpcode::IntToByte, Value::Int(value)) => Value::Byte(value as u8),
        (ConversionOpcode::IntToBool, Value::Int(value)) => Value::Bool(value != 0),
        (ConversionOpcode::IntToFloat, Value::Int(value)) => Value::Float(value as f32),
        (ConversionOpcode::BoolToByte, Value::Bool(value)) => Value::Byte(u8::from(value)),
        (ConversionOpcode::BoolToInt, Value::Bool(value)) => Value::Int(i32::from(value)),
        (ConversionOpcode::BoolToFloat, Value::Bool(value)) => Value::Float(f32::from(value)),
        (ConversionOpcode::FloatToByte, Value::Float(value)) => Value::Byte(value as u8),
        (ConversionOpcode::FloatToInt, Value::Float(value)) => Value::Int(value as i32),
        (ConversionOpcode::FloatToBool, Value::Float(value)) => Value::Bool(value != 0.0),
        (ConversionOpcode::ObjectToBool, Value::Object(value)) => Value::Bool(value != 0),
        (ConversionOpcode::NameToBool, Value::Name(value)) => Value::Bool(value != 0),
        (ConversionOpcode::NameToBool, Value::NameText(value)) => {
            Value::Bool(!value.eq_ignore_ascii_case("None"))
        }
        (ConversionOpcode::StringToByte, Value::String(value)) => {
            Value::Byte(value.trim().parse().unwrap_or_default())
        }
        (ConversionOpcode::StringToInt, Value::String(value)) => {
            Value::Int(value.trim().parse().unwrap_or_default())
        }
        (ConversionOpcode::StringToBool, Value::String(value)) => {
            Value::Bool(value.trim().parse::<i32>().unwrap_or_default() != 0)
        }
        (ConversionOpcode::StringToFloat, Value::String(value)) => {
            Value::Float(value.trim().parse().unwrap_or_default())
        }
        (ConversionOpcode::StringToVector, Value::String(value)) => {
            Value::Vector(parse_string_triplet(&value).unwrap_or([0.0; 3]))
        }
        (ConversionOpcode::StringToRotator, Value::String(value)) => Value::Rotator(
            parse_string_triplet(&value)
                .map(|value| [value[0] as i32, value[1] as i32, value[2] as i32])
                .unwrap_or([0; 3]),
        ),
        (ConversionOpcode::VectorToBool, Value::Vector(value)) => Value::Bool(value != [0.0; 3]),
        (ConversionOpcode::VectorToRotator, Value::Vector([x, y, z])) => {
            let units = 65_536.0 / std::f32::consts::TAU;
            Value::Rotator([
                ((-z).atan2((x * x + y * y).sqrt()) * units) as i32,
                (y.atan2(x) * units) as i32,
                0,
            ])
        }
        (ConversionOpcode::RotatorToBool, Value::Rotator(value)) => Value::Bool(value != [0; 3]),
        (ConversionOpcode::ByteToString, Value::Byte(value)) => Value::String(value.to_string()),
        (ConversionOpcode::IntToString, Value::Int(value)) => Value::String(value.to_string()),
        (ConversionOpcode::BoolToString, Value::Bool(value)) => {
            Value::String(if value { "True" } else { "False" }.to_owned())
        }
        (ConversionOpcode::FloatToString, Value::Float(value)) => Value::String(value.to_string()),
        (ConversionOpcode::NameToString, Value::NameText(value)) => Value::String(value),
        (ConversionOpcode::VectorToString, Value::Vector(value)) => {
            Value::String(format!("{},{},{}", value[0], value[1], value[2]))
        }
        (ConversionOpcode::RotatorToString, Value::Rotator(value)) => {
            Value::String(format!("{},{},{}", value[0], value[1], value[2]))
        }
        (ConversionOpcode::StringToName, Value::String(value)) => Value::NameText(value),
        (_, value) => {
            return Err(Error::Type {
                expected: "supported conversion input",
                actual: value.kind(),
            });
        }
    })
}

fn parse_string_triplet(value: &str) -> Option<[f32; 3]> {
    let mut values = value.split(',').map(|value| value.trim().parse::<f32>());
    Some([
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    ])
}

fn rotator_axes([pitch, yaw, roll]: [i32; 3]) -> [[f32; 3]; 3] {
    let units_to_radians = std::f32::consts::TAU / 65_536.0;
    let (pitch_sin, pitch_cos) = ((pitch as f32) * units_to_radians).sin_cos();
    let (yaw_sin, yaw_cos) = ((yaw as f32) * units_to_radians).sin_cos();
    let (roll_sin, roll_cos) = ((roll as f32) * units_to_radians).sin_cos();
    [
        [pitch_cos * yaw_cos, pitch_cos * yaw_sin, -pitch_sin],
        [
            -roll_sin * pitch_sin * yaw_cos - roll_cos * yaw_sin,
            -roll_sin * pitch_sin * yaw_sin + roll_cos * yaw_cos,
            -roll_sin * pitch_cos,
        ],
        [
            roll_cos * pitch_sin * yaw_cos - roll_sin * yaw_sin,
            roll_cos * pitch_sin * yaw_sin + roll_sin * yaw_cos,
            roll_cos * pitch_cos,
        ],
    ]
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
    fn switch_selects_matching_and_default_cases() {
        let mut bytes = vec![0x05, 3, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x0a);
        let next_case = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x2c, 2, 0x0f, 0x00]);
        bytes.extend(8_i32.to_le_bytes());
        bytes.extend([0x2c, 20, 0x06]);
        let end_jump = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        let default_case = u16::try_from(bytes.len()).unwrap();
        bytes[next_case..next_case + 2].copy_from_slice(&default_case.to_le_bytes());
        bytes.push(0x0a);
        bytes.extend(u16::MAX.to_le_bytes());
        bytes.extend([0x0f, 0x00]);
        bytes.extend(8_i32.to_le_bytes());
        bytes.extend([0x2c, 30]);
        let end = u16::try_from(bytes.len()).unwrap();
        bytes[end_jump..end_jump + 2].copy_from_slice(&end.to_le_bytes());
        bytes.extend([0x04, 0x00]);
        bytes.extend(8_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let run = |condition| {
            let mut frame = Frame::new(&bytecode);
            frame.set_local(7, Value::Int(condition));
            frame.execute(|_, _| unreachable!()).unwrap()
        };

        assert_eq!(run(2), Value::Int(20));
        assert_eq!(run(3), Value::Int(30));
        assert_eq!(
            switch_values_equal(&Value::Byte(2), &Value::Int(2)),
            Ok(true)
        );
        assert_eq!(switch_values_equal(&Value::None, &Value::Byte(0)), Ok(true));
        assert_eq!(
            switch_values_equal(&Value::None, &Value::Byte(1)),
            Ok(false)
        );
    }

    #[test]
    fn switch_allows_adjacent_fallthrough_cases() {
        let mut bytes = vec![0x05, 3, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x0a);
        let second_case = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x2c, 2, 0x0a]);
        let default_case = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x2c, 3, 0x04, 0x2c, 42]);
        let default = u16::try_from(bytes.len()).unwrap();
        bytes[second_case..second_case + 2]
            .copy_from_slice(&u16::try_from(default_case - 1).unwrap().to_le_bytes());
        bytes[default_case..default_case + 2].copy_from_slice(&default.to_le_bytes());
        bytes.extend([0x0a, 0xff, 0xff, 0x04, 0x2c, 0]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let run = |condition| {
            let mut frame = Frame::new(&bytecode);
            frame.set_local(7, Value::Int(condition));
            frame.execute(|_, _| unreachable!()).unwrap()
        };
        assert_eq!(run(2), Value::Int(42));
        assert_eq!(run(3), Value::Int(42));
        assert_eq!(run(4), Value::Int(0));
    }

    #[test]
    fn resumes_state_frames_after_latent_calls() {
        let mut bytes = vec![0x61, 0x00, 0x1e];
        bytes.extend(0.5_f32.to_le_bytes());
        bytes.push(0x16);
        bytes.extend([0x0f, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(42_i32.to_le_bytes());
        bytes.push(0x08);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        assert!(matches!(
            frame
                .resume_hosted(|request| match request {
                    FrameRequest::Call {
                        function: FunctionCall::Native(0x100),
                        arguments,
                        ..
                    } if arguments == [Value::Float(0.5)] => {
                        Ok(FrameResponse::Suspend(Value::None))
                    }
                    _ => unreachable!(),
                })
                .unwrap(),
            FrameRun::Suspended
        ));

        let mut frame = Frame::from_snapshot(&bytecode, frame.into_snapshot());
        assert!(matches!(
            frame.resume_hosted(|_| unreachable!()).unwrap(),
            FrameRun::Stopped
        ));
        assert_eq!(frame.local(7), Some(&Value::Int(42)));
    }

    #[test]
    fn iterates_values_and_clears_the_output_slot() {
        let mut bytes = vec![0x2f, 0x61, 0x30, 0x20];
        bytes.extend(1_i32.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(9_i32.to_le_bytes());
        bytes.push(0x16);
        let end_offset = bytes.len();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend([0x0f, 0x00]);
        bytes.extend(8_i32.to_le_bytes());
        bytes.push(0x00);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x31);
        let iterator_pop = u16::try_from(bytes.len()).unwrap();
        bytes[end_offset..end_offset + 2].copy_from_slice(&iterator_pop.to_le_bytes());
        bytes.extend([0x30, 0x04, 0x00]);
        bytes.extend(8_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut instance = HashMap::new();
        let mut frame = Frame::new(&bytecode);
        let result = frame
            .execute_with_instance(&mut instance, |request, _| match request {
                FrameRequest::ResolveObject { reference: 1 } => {
                    Ok(FrameResponse::Value(Value::Object(1)))
                }
                FrameRequest::CallIterator {
                    function: FunctionCall::Native(0x130),
                    arguments,
                    ..
                } => {
                    assert_eq!(arguments, vec![Value::Object(1), Value::None, Value::None]);
                    Ok(FrameResponse::Iterator(vec![
                        IteratorValue {
                            value: Value::Object(11),
                            outputs: vec![(2, Value::Int(101))],
                        },
                        IteratorValue {
                            value: Value::Object(22),
                            outputs: vec![(2, Value::Int(202))],
                        },
                    ]))
                }
                _ => unreachable!(),
            })
            .unwrap();
        assert_eq!(result, Value::Object(22));
        assert_eq!(frame.local(7), Some(&Value::Object(0)));
        assert_eq!(frame.local(9), Some(&Value::Int(202)));
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
    fn converts_unreal_rotators_to_direction_vectors() {
        let direction = |rotation| {
            let Value::Vector(direction) =
                convert(ConversionOpcode::RotatorToVector, Value::Rotator(rotation)).unwrap()
            else {
                unreachable!()
            };
            direction
        };
        let close = |left: [f32; 3], right: [f32; 3]| {
            left.into_iter()
                .zip(right)
                .all(|(left, right)| (left - right).abs() < 1.0e-6)
        };

        assert!(close(direction([0, 0, 0]), [1.0, 0.0, 0.0]));
        assert!(close(direction([0, 16_384, 0]), [0.0, 1.0, 0.0]));
        assert!(close(direction([16_384, 0, 0]), [0.0, 0.0, -1.0]));
    }

    #[test]
    fn converts_missing_typed_defaults_used_by_scripts() {
        assert_eq!(
            convert(ConversionOpcode::ObjectToBool, Value::None).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            convert(ConversionOpcode::ByteToInt, Value::None).unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            convert(ConversionOpcode::RotatorToVector, Value::None).unwrap(),
            Value::Vector([1.0, 0.0, -0.0])
        );
        assert_eq!(
            convert(ConversionOpcode::FloatToInt, Value::None).unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            convert(
                ConversionOpcode::StringToVector,
                Value::String("1.5, -2, 3".to_owned())
            )
            .unwrap(),
            Value::Vector([1.5, -2.0, 3.0])
        );
    }

    #[test]
    fn get_axes_writes_its_vector_outputs() {
        let mut bytes = vec![0xe5, 0x22];
        for component in [0_i32, 16_384, 0] {
            bytes.extend(component.to_le_bytes());
        }
        for field in [7_i32, 8, 9] {
            bytes.push(0x00);
            bytes.extend(field.to_le_bytes());
        }
        bytes.extend([0x16, 0x04, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        let close = |left: [f32; 3], right: [f32; 3]| {
            left.into_iter()
                .zip(right)
                .all(|(left, right)| (left - right).abs() < 1.0e-6)
        };
        let Value::Vector(x) = frame.execute(|_, _| unreachable!()).unwrap() else {
            unreachable!()
        };
        assert!(close(x, [0.0, 1.0, 0.0]));
        assert!(matches!(
            frame.local(8),
            Some(Value::Vector(y)) if close(*y, [-1.0, 0.0, 0.0])
        ));
        assert!(matches!(
            frame.local(9),
            Some(Value::Vector(z)) if close(*z, [0.0, 0.0, 1.0])
        ));
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

        let mut plane = Value::Struct(HashMap::new());
        StructMember::Z.set(&mut plane, Value::Float(42.0)).unwrap();
        assert_eq!(StructMember::Z.get(plane).unwrap(), Value::Float(42.0));

        let mut uninitialized = Value::None;
        StructMember::Z
            .set(&mut uninitialized, Value::Float(42.0))
            .unwrap();
        assert_eq!(
            uninitialized,
            Value::Vector([0.0, 0.0, 42.0]),
            "dynamic arrays zero-initialize vector elements"
        );
    }

    #[test]
    fn reads_and_writes_generic_struct_members() {
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
        frame.set_local(7, Value::Struct(HashMap::new()));
        frame.set_struct_member(
            9,
            StructMember::Field {
                name: "Base".to_owned(),
                zero: Value::Float(0.0),
            },
        );
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Float(42.0)
        );
        assert_eq!(
            frame.local(7),
            Some(&Value::Struct(HashMap::from([(
                "Base".to_owned(),
                Value::Float(42.0)
            )])))
        );
    }

    #[test]
    fn reads_and_writes_fixed_array_elements() {
        for opcode in [0x1a, 0x10] {
            let mut bytes = vec![0x0f, opcode, 0x26, 0x00];
            bytes.extend(7_i32.to_le_bytes());
            bytes.push(0x1d);
            bytes.extend(42_i32.to_le_bytes());
            bytes.extend([0x04, opcode, 0x26, 0x00]);
            bytes.extend(7_i32.to_le_bytes());
            let bytecode = Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            };
            let mut frame = Frame::new(&bytecode);
            frame.set_local(7, Value::Array(vec![Value::Int(1), Value::Int(2)]));

            assert_eq!(
                frame.execute(|_, _| unreachable!()).unwrap(),
                Value::Int(42)
            );
            assert_eq!(
                frame.local(7),
                Some(&Value::Array(vec![Value::Int(1), Value::Int(42)]))
            );
        }
    }

    #[test]
    fn dynamic_arrays_grow_on_access_and_report_their_length() {
        let mut bytes = vec![0x0f, 0x10, 0x2c, 3, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1d);
        bytes.extend(42_i32.to_le_bytes());
        bytes.extend([0x04, 0x37, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Array(Vec::new()));

        assert_eq!(frame.execute(|_, _| unreachable!()).unwrap(), Value::Int(4));
        assert_eq!(
            frame.local(7),
            Some(&Value::Array(vec![
                Value::None,
                Value::None,
                Value::None,
                Value::Int(42)
            ]))
        );
    }

    #[test]
    fn compound_native_assignment_preserves_the_target_slot() {
        let mut bytes = vec![0xb8, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(2.5_f32.to_le_bytes());
        bytes.extend([0x16, 0x04, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(7, Value::Float(1.5));
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Float(4.0)
        );
        assert_eq!(frame.local(7), Some(&Value::Float(4.0)));
        assert_eq!(
            compound_assignment(
                CompoundAssignment::AddEqual_IntInt,
                &Value::Int(i32::MAX),
                &Value::Int(1),
            )
            .unwrap(),
            Value::Int(i32::MIN)
        );
        assert_eq!(
            compound_assignment(
                CompoundAssignment::SubtractEqual_FloatFloat,
                &Value::None,
                &Value::Float(1.0),
            )
            .unwrap(),
            Value::Float(-1.0)
        );
    }

    #[test]
    fn increment_natives_preserve_prefix_and_postfix_results() {
        let run = |native, initial| {
            let mut bytes = vec![0x0f, 0x00];
            bytes.extend(8_i32.to_le_bytes());
            bytes.extend([native, 0x00]);
            bytes.extend(7_i32.to_le_bytes());
            bytes.extend([0x16, 0x04, 0x00]);
            bytes.extend(8_i32.to_le_bytes());
            let bytecode = Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            };
            let mut frame = Frame::new(&bytecode);
            frame.set_local(7, Value::Int(initial));
            let returned = frame.execute(|_, _| unreachable!()).unwrap();
            (returned, frame.local(7).cloned())
        };

        assert_eq!(run(0xa3, 41), (Value::Int(42), Some(Value::Int(42))));
        assert_eq!(run(0xa5, 41), (Value::Int(41), Some(Value::Int(42))));
        assert_eq!(
            increment_decrement(IncrementDecrement::AddAdd_Byte, &Value::Byte(u8::MAX)).unwrap(),
            (Value::Byte(0), Value::Byte(u8::MAX))
        );
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
    fn default_variable_reads_bound_class_default() {
        let mut bytes = vec![0x04, 0x02];
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_default(7, Value::Vector([0.0, 0.0, -512.0]));
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Vector([0.0, 0.0, -512.0])
        );
    }

    #[test]
    fn dynamic_cast_uses_frame_host() {
        let run = |result: Value| {
            let mut bytes = vec![0x04, 0x2e];
            bytes.extend(7_i32.to_le_bytes());
            bytes.push(0x20);
            bytes.extend(11_i32.to_le_bytes());
            let bytecode = Bytecode {
                version: 76,
                raw_len: bytes.len(),
                bytes,
                tokens: Vec::new(),
            };
            Frame::new(&bytecode)
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: 11 } => {
                        Ok(FrameResponse::Value(Value::Object(11)))
                    }
                    FrameRequest::DynamicCast {
                        class: 7,
                        value: Value::Object(11),
                    } => Ok(FrameResponse::Value(result.clone())),
                    _ => unreachable!(),
                })
                .unwrap()
        };

        assert_eq!(run(Value::Object(11)), Value::Object(11));
        assert_eq!(run(Value::Object(0)), Value::Object(0));
    }

    #[test]
    fn object_to_string_uses_frame_host() {
        let mut bytes = vec![0x04, 0x56, 0x20];
        bytes.extend(11_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        assert_eq!(
            Frame::new(&bytecode)
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: 11 } => {
                        Ok(FrameResponse::Value(Value::Object(11)))
                    }
                    FrameRequest::ObjectToString {
                        value: Value::Object(11),
                    } => Ok(FrameResponse::Value(Value::String(
                        "Hog2.Snail1".to_owned(),
                    ))),
                    _ => unreachable!(),
                })
                .unwrap(),
            Value::String("Hog2.Snail1".to_owned())
        );
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
                FrameRequest::ResolveObject { reference: 1 } => {
                    Ok(FrameResponse::Value(Value::Object(1)))
                }
                FrameRequest::GetInstance { receiver: 1, field } => Ok(FrameResponse::Value(
                    remote.get(&field).cloned().unwrap_or(Value::None),
                )),
                FrameRequest::SetInstance {
                    receiver: 1,
                    field,
                    value,
                } => {
                    remote.insert(field, value);
                    Ok(FrameResponse::Value(Value::None))
                }
                _ => unreachable!(),
            })
            .unwrap();
        assert_eq!(result, Value::Int(42));
        assert_eq!(remote.get(&7), Some(&Value::Int(42)));
    }

    #[test]
    fn class_context_reads_the_resolved_default_object() {
        let mut bytes = vec![0x04, 0x12, 0x20];
        bytes.extend((-149_i32).to_le_bytes());
        bytes.extend(5_u16.to_le_bytes());
        bytes.push(4);
        bytes.push(0x01);
        bytes.extend(7_i32.to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        assert_eq!(
            Frame::new(&bytecode)
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: -149 } => {
                        Ok(FrameResponse::Value(Value::Object(23)))
                    }
                    FrameRequest::GetInstance {
                        receiver: 23,
                        field: 7,
                    } => Ok(FrameResponse::Value(Value::Int(42))),
                    _ => unreachable!(),
                })
                .unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn context_reads_and_writes_remote_struct_members() {
        let mut context = vec![0x19, 0x20];
        context.extend(1_i32.to_le_bytes());
        context.extend(5_u16.to_le_bytes());
        context.push(0);
        context.push(0x01);
        context.extend(7_i32.to_le_bytes());
        let mut member = vec![0x36];
        member.extend(9_i32.to_le_bytes());
        member.extend(&context);
        let mut bytes = vec![0x0f];
        bytes.extend(&member);
        bytes.push(0x1e);
        bytes.extend(42.0_f32.to_le_bytes());
        bytes.push(0x04);
        bytes.extend(member);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut remote = HashMap::new();
        let mut frame = Frame::new(&bytecode);
        frame.set_struct_member(
            9,
            StructMember::Field {
                name: "Base".to_owned(),
                zero: Value::Float(0.0),
            },
        );
        let result = frame
            .execute_hosted(|request| match request {
                FrameRequest::ResolveObject { reference: 1 } => {
                    Ok(FrameResponse::Value(Value::Object(1)))
                }
                FrameRequest::GetInstance { receiver: 1, field } => Ok(FrameResponse::Value(
                    remote
                        .get(&field)
                        .cloned()
                        .unwrap_or_else(|| Value::Struct(HashMap::new())),
                )),
                FrameRequest::SetInstance {
                    receiver: 1,
                    field,
                    value,
                } => {
                    remote.insert(field, value);
                    Ok(FrameResponse::Value(Value::None))
                }
                _ => unreachable!(),
            })
            .unwrap();
        assert_eq!(result, Value::Float(42.0));
        assert_eq!(
            remote.get(&7),
            Some(&Value::Struct(HashMap::from([(
                "Base".to_owned(),
                Value::Float(42.0)
            )])))
        );
    }

    #[test]
    fn null_context_discards_struct_member_writes() {
        let mut bytes = vec![0x0f, 0x36];
        bytes.extend(9_i32.to_le_bytes());
        bytes.extend([0x19, 0x2a]);
        bytes.extend(5_u16.to_le_bytes());
        bytes.extend([8, 0x01]);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(42.0_f32.to_le_bytes());
        bytes.extend([0x04, 0x27]);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_struct_member(
            9,
            StructMember::Field {
                name: "Base".to_owned(),
                zero: Value::Float(0.0),
            },
        );
        assert_eq!(
            frame.execute(|_, _| unreachable!()).unwrap(),
            Value::Bool(true)
        );
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

    #[test]
    fn hosted_instance_reads_and_writes_without_a_frame_copy() {
        let mut bytes = vec![0x0f, 0x01];
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
        let result = frame
            .execute_hosted(|request| match request {
                FrameRequest::GetInstance {
                    receiver: -1,
                    field,
                } => Ok(FrameResponse::Value(
                    instance.get(&field).cloned().unwrap_or(Value::None),
                )),
                FrameRequest::SetInstance {
                    receiver: -1,
                    field,
                    value,
                } => {
                    instance.insert(field, value);
                    Ok(FrameResponse::Value(Value::None))
                }
                _ => unreachable!(),
            })
            .unwrap();

        assert_eq!(result, Value::Int(42));
        assert_eq!(instance.get(&7), Some(&Value::Int(42)));
    }

    #[test]
    fn context_call_arguments_use_the_callers_instance() {
        let mut bytes = vec![0x04, 0x19, 0x20];
        bytes.extend(11_i32.to_le_bytes());
        bytes.extend([0, 0, 0, 0x1b]);
        bytes.extend(5_i32.to_le_bytes());
        bytes.push(0x01);
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x16);
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        let result = frame
            .execute_hosted(|request| match request {
                FrameRequest::ResolveObject { reference: 11 } => {
                    Ok(FrameResponse::Value(Value::Object(11)))
                }
                FrameRequest::GetInstance {
                    receiver: -1,
                    field: 7,
                } => Ok(FrameResponse::Value(Value::String("cue".to_owned()))),
                FrameRequest::Call {
                    receiver: 11,
                    function: FunctionCall::Virtual(5),
                    arguments,
                } => {
                    assert_eq!(arguments, [Value::String("cue".to_owned())]);
                    Ok(FrameResponse::Value(Value::Int(42)))
                }
                _ => unreachable!(),
            })
            .unwrap();
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn object_constants_resolve_signed_package_references() {
        let mut bytes = vec![0x04, 0x20];
        bytes.extend((-149_i32).to_le_bytes());
        let bytecode = Bytecode {
            version: 76,
            raw_len: bytes.len(),
            bytes,
            tokens: Vec::new(),
        };
        assert_eq!(
            Frame::new(&bytecode)
                .execute_hosted(|request| match request {
                    FrameRequest::ResolveObject { reference: -149 } => {
                        Ok(FrameResponse::Value(Value::Object(23)))
                    }
                    _ => unreachable!(),
                })
                .unwrap(),
            Value::Object(23)
        );
    }
}
