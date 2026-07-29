use std::collections::HashMap;

use openhp1_script::Bytecode;

use crate::{
    Error, MAX_EXPRESSION_DEPTH, Result, Value,
    opcode::{ConversionOpcode, Opcode},
};

mod execute;
mod operations;

pub(crate) use operations::rotator_axes;
use operations::*;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FunctionCall {
    Native(u16),
    Virtual(i32),
    Final(i32),
    Global(i32),
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
    Default {
        receiver: i32,
        field: i32,
    },
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
    pub(crate) value: Value,
    pub(crate) outputs: Vec<(usize, Value)>,
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
    array_element_defaults: HashMap<i32, Value>,
    struct_members: HashMap<i32, StructMember>,
    hosted_instance: bool,
    creating_iterator: bool,
    suspend_requested: bool,
    pending_iterator: Option<PendingIterator>,
    iterators: Vec<ActiveIterator>,
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;

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
            array_element_defaults: HashMap::new(),
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

    pub(crate) fn set_array_element_default(&mut self, field: i32, value: Value) {
        self.array_element_defaults.insert(field, value);
    }

    pub fn instance(&self, field: i32) -> Option<&Value> {
        self.instance.get(&field)
    }

    pub(crate) fn set_struct_member(&mut self, field: i32, member: StructMember) {
        self.struct_members.insert(field, member);
    }
}
