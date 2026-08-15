use super::*;

impl<'a> Frame<'a> {
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
            FrameRequest::MetaCast { .. } => {
                Err("standalone frames do not host meta casts".to_owned())
            }
            FrameRequest::ObjectToString { .. } => {
                Err("standalone frames do not host object conversions".to_owned())
            }
            FrameRequest::NameToString { .. } => {
                Err("standalone frames do not host name conversions".to_owned())
            }
            FrameRequest::ResolveObject { reference } => {
                Ok(FrameResponse::Value(Value::Object(reference)))
            }
            FrameRequest::Call { receiver, .. }
            | FrameRequest::GetInstance { receiver, .. }
            | FrameRequest::GetDefault { receiver, .. }
            | FrameRequest::SetInstance { receiver, .. }
            | FrameRequest::SetDefault { receiver, .. } => {
                Err(format!("object context {receiver} has no frame host"))
            }
        })
    }

    #[cfg(test)]
    pub(super) fn execute_with_instance(
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
            if self.steps >= self.step_limit {
                return Err(Error::StepLimit {
                    limit: self.step_limit,
                });
            }
            self.steps += 1;
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
            Opcode::DefaultVariable => Expression::Slot(Slot::Default {
                receiver: self.current_context,
                field: self.read_i32()?,
            }),
            Opcode::Nothing | Opcode::Unknown0x15 => Expression::Value(Value::None),
            Opcode::EatString => {
                let value = self.expression(host)?;
                self.value(value, host)?;
                Expression::Value(Value::None)
            }
            Opcode::Let | Opcode::LetBool => {
                let target = self.expression(host)?;
                let value_expression = self.expression(host)?;
                let value = if matches!(
                    &value_expression,
                    Expression::Slot(Slot::Discard(Value::Vector(_)))
                ) && matches!(
                    &target,
                    Expression::Slot(slot)
                        if matches!(self.slot(slot, host)?, Some(Value::Rotator(_)))
                ) {
                    Value::Rotator([0; 3])
                } else {
                    self.value(value_expression, host)?
                };
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
            Opcode::MetaCast => {
                let class = self.read_i32()?;
                let value = self.expression(host)?;
                let value = self.value(value, host)?;
                Expression::Value(
                    host(FrameRequest::MetaCast { class, value }, &mut self.instance)
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
                match (conversion, value) {
                    (ConversionOpcode::ObjectToString, value) => Expression::Value(
                        host(FrameRequest::ObjectToString { value }, &mut self.instance)
                            .and_then(FrameResponse::into_value)
                            .map_err(|message| Error::Context { message })?,
                    ),
                    (ConversionOpcode::NameToString, value @ Value::Name(_)) => Expression::Value(
                        host(FrameRequest::NameToString { value }, &mut self.instance)
                            .and_then(FrameResponse::into_value)
                            .map_err(|message| Error::Context { message })?,
                    ),
                    (conversion, value) => Expression::Value(convert(conversion, value)?),
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
        if let FunctionCall::Native(index @ (0x82 | 0x84)) = function {
            let left = self.expression(host)?;
            let left = self.value(left, host)?.truthy()?;
            let opcode = self.opcode()?;
            if Opcode::from(opcode) != Opcode::Skip {
                return Err(Error::Call {
                    call: function,
                    message: format!("logical native expects Skip, found opcode {opcode:#04x}"),
                });
            }
            let skip = usize::from(self.read_u16()?);
            let short_circuit = (index == 0x82 && !left) || (index == 0x84 && left);
            if short_circuit {
                self.jump(self.instruction_pointer.saturating_add(skip))?;
                self.current_context = receiver;
                return Ok(Value::Bool(left));
            }
            let right = self.expression(host)?;
            let value = self.value(right, host)?.truthy()?;
            let opcode = self.opcode()?;
            if Opcode::from(opcode) != Opcode::EndFunctionParms {
                return Err(Error::Call {
                    call: function,
                    message: format!(
                        "logical native expects EndFunctionParms, found opcode {opcode:#04x}"
                    ),
                });
            }
            self.current_context = receiver;
            return Ok(Value::Bool(value));
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
            Slot::Default {
                receiver: -1,
                field,
            } => {
                self.defaults.insert(field, value);
            }
            Slot::Default { receiver, field } => {
                host(
                    FrameRequest::SetDefault {
                        receiver,
                        field,
                        value,
                    },
                    &mut self.instance,
                )
                .and_then(FrameResponse::into_value)
                .map_err(|message| Error::Context { message })?;
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
                let default = self.array_element_default(&target);
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
                if index >= values.len() {
                    values.resize(index.saturating_add(1), default);
                }
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
            Slot::Default {
                receiver: -1,
                field,
            } => self.defaults.get(field).cloned(),
            Slot::Default { receiver, field } => Some(
                host(
                    FrameRequest::GetDefault {
                        receiver: *receiver,
                        field: *field,
                    },
                    &mut self.instance,
                )
                .and_then(FrameResponse::into_value)
                .map_err(|message| Error::Context { message })?,
            ),
            Slot::StructMember { target, member } => self
                .slot(target, host)?
                .map(|value| member.get(value))
                .transpose()?,
            Slot::ArrayElement { target, index } => self
                .slot(target, host)?
                .map(|value| array_element(&value, *index))
                .transpose()?,
            Slot::DynArrayElement { target, index } => {
                let default = self.array_element_default(target);
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
                if index >= values.len() {
                    values.resize(index.saturating_add(1), default);
                }
                let value = values[index].clone();
                self.assign_slot((**target).clone(), target_value, host)?;
                Some(value)
            }
        })
    }

    fn array_element_default(&self, target: &Slot) -> Value {
        let field = match target {
            Slot::Local(field) | Slot::Instance { field, .. } | Slot::Default { field, .. } => {
                Some(field)
            }
            _ => None,
        };
        field
            .and_then(|field| self.array_element_defaults.get(field))
            .cloned()
            .unwrap_or(Value::None)
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
