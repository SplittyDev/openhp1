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
fn byte_to_int_converts_a_boolean_instance_value() {
    let mut bytes = vec![0x04, 0x3a, 0x01];
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
        Value::Int(1)
    );
}

#[test]
fn eat_string_evaluates_its_child_and_discards_its_value() {
    let mut bytes = vec![0x0f, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.extend([0x0e, 0x92, 0x16, 0x04, 0x00]);
    bytes.extend(7_i32.to_le_bytes());
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    let mut calls = 0;
    let result = frame
        .execute(|call, arguments| {
            assert_eq!(call, FunctionCall::Native(0x92));
            assert!(arguments.is_empty());
            calls += 1;
            Ok(Value::String("discarded".to_owned()))
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(result, Value::None);
    assert_eq!(frame.local(7), Some(&Value::None));
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
    assert!(close(rotator_axes([16_384, 0, 0])[0], [0.0, 0.0, -1.0]));
    assert_eq!(
        convert(
            ConversionOpcode::VectorToRotator,
            Value::Vector([0.0, 0.0, 1.0])
        )
        .unwrap(),
        Value::Rotator([-16_384, 0, 0])
    );
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
    frame.set_array_element_default(7, Value::Int(0));

    assert_eq!(frame.execute(|_, _| unreachable!()).unwrap(), Value::Int(4));
    assert_eq!(
        frame.local(7),
        Some(&Value::Array(vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(42)
        ]))
    );
}

#[test]
fn dynamic_array_access_does_not_shrink_existing_values() {
    let mut bytes = vec![0x10, 0x2c, 0, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.extend([0x0f, 0x10, 0x2c, 1, 0x00]);
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
    frame.set_local(
        7,
        Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
    );
    frame.set_array_element_default(7, Value::Int(0));

    assert_eq!(frame.execute(|_, _| unreachable!()).unwrap(), Value::Int(3));
    assert_eq!(
        frame.local(7),
        Some(&Value::Array(vec![
            Value::Int(1),
            Value::Int(42),
            Value::Int(3)
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
    assert_eq!(
        compound_assignment(
            CompoundAssignment::MultiplyEqual_VectorVector,
            &Value::Vector([2.0, 3.0, 4.0]),
            &Value::Vector([5.0, 6.0, 7.0]),
        )
        .unwrap(),
        Value::Vector([10.0, 18.0, 28.0])
    );
}

#[test]
fn subtract_equal_int_int_dispatches_and_stores_the_wrapped_difference() {
    let mut bytes = vec![0xa2, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.push(0x1d);
    bytes.extend(1_i32.to_le_bytes());
    bytes.extend([0x16, 0x04, 0x00]);
    bytes.extend(7_i32.to_le_bytes());
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    frame.set_local(7, Value::Int(i32::MIN));

    assert_eq!(
        frame.execute(|_, _| unreachable!()).unwrap(),
        Value::Int(i32::MAX)
    );
    assert_eq!(frame.local(7), Some(&Value::Int(i32::MAX)));
}

#[test]
fn multiply_equal_int_float_dispatches_and_stores_the_truncated_product() {
    let mut bytes = vec![0x9f, 0x00];
    bytes.extend(7_i32.to_le_bytes());
    bytes.push(0x1e);
    bytes.extend(1.5_f32.to_le_bytes());
    bytes.extend([0x16, 0x04, 0x00]);
    bytes.extend(7_i32.to_le_bytes());
    let bytecode = Bytecode {
        version: 76,
        raw_len: bytes.len(),
        bytes,
        tokens: Vec::new(),
    };
    let mut frame = Frame::new(&bytecode);
    frame.set_local(7, Value::Int(-3));

    assert_eq!(
        frame.execute(|_, _| unreachable!()).unwrap(),
        Value::Int(-4)
    );
    assert_eq!(frame.local(7), Some(&Value::Int(-4)));
}

#[test]
fn divide_equal_int_float_dispatches_with_ue1_precision_and_edges() {
    let run = |initial: i32, operand: f32| {
        let mut bytes = vec![0xa0, 0x00];
        bytes.extend(7_i32.to_le_bytes());
        bytes.push(0x1e);
        bytes.extend(operand.to_le_bytes());
        bytes.extend([0x16, 0x04, 0x00]);
        bytes.extend(7_i32.to_le_bytes());
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

    assert_eq!(run(-3, 1.5), (Value::Int(-2), Some(Value::Int(-2))));
    assert_eq!(
        run(i32::MAX, 2.0),
        (Value::Int(1_073_741_823), Some(Value::Int(1_073_741_823)))
    );
    assert_eq!(run(7, 0.0), (Value::Int(0), Some(Value::Int(0))));
    assert_eq!(run(7, -0.0), (Value::Int(0), Some(Value::Int(0))));
    assert_eq!(
        run(7, f32::NAN),
        (Value::Int(i32::MIN), Some(Value::Int(i32::MIN)))
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
fn name_to_string_resolves_numeric_names_through_the_frame_host() {
    let mut bytes = vec![0x04, 0x57, 0x21];
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
                FrameRequest::NameToString {
                    value: Value::Name(7),
                } => Ok(FrameResponse::Value(Value::String("Run".to_owned()))),
                _ => unreachable!(),
            })
            .unwrap(),
        Value::String("Run".to_owned())
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
fn class_context_reads_the_resolved_class_default() {
    let mut bytes = vec![0x04, 0x12, 0x20];
    bytes.extend((-149_i32).to_le_bytes());
    bytes.extend(5_u16.to_le_bytes());
    bytes.push(4);
    bytes.push(0x02);
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
                FrameRequest::GetDefault {
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
fn object_context_reads_the_receivers_class_default() {
    let mut context = vec![0x19, 0x20];
    context.extend(1_i32.to_le_bytes());
    context.extend(5_u16.to_le_bytes());
    context.push(4);
    context.push(0x02);
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
    let mut defaults = HashMap::new();
    let result = Frame::new(&bytecode)
        .execute_hosted(|request| match request {
            FrameRequest::ResolveObject { reference: 1 } => {
                Ok(FrameResponse::Value(Value::Object(23)))
            }
            FrameRequest::GetDefault {
                receiver: 23,
                field,
            } => Ok(FrameResponse::Value(
                defaults.get(&field).cloned().unwrap_or(Value::None),
            )),
            FrameRequest::SetDefault {
                receiver: 23,
                field,
                value,
            } => {
                defaults.insert(field, value);
                Ok(FrameResponse::Value(Value::None))
            }
            _ => unreachable!(),
        })
        .unwrap();
    assert_eq!(result, Value::Int(42));
    assert_eq!(defaults.get(&7), Some(&Value::Int(42)));
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
