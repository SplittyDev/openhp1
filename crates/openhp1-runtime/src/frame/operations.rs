use super::*;

pub(super) fn array_element(value: &Value, index: i32) -> Result<Value> {
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
    pub(super) fn get(&self, value: Value) -> Result<Value> {
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

    pub(super) fn set(&self, target: &mut Value, value: Value) -> Result<()> {
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

pub(super) fn switch_values_equal(condition: &Value, case: &Value) -> Result<bool> {
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
pub(super) enum IncrementDecrement {
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

pub(super) fn increment_decrement(
    operation: IncrementDecrement,
    current: &Value,
) -> Result<(Value, Value)> {
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
pub(super) enum CompoundAssignment {
    AddEqual_IntInt,
    SubtractEqual_IntInt,
    MultiplyEqual_IntFloat,
    DivideEqual_IntFloat,
    MultiplyEqual_FloatFloat,
    DivideEqual_FloatFloat,
    AddEqual_FloatFloat,
    SubtractEqual_FloatFloat,
    MultiplyEqual_VectorFloat,
    DivideEqual_VectorFloat,
    AddEqual_VectorVector,
    SubtractEqual_VectorVector,
    MultiplyEqual_VectorVector,
}

impl TryFrom<u16> for CompoundAssignment {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0xa1 => Ok(Self::AddEqual_IntInt),
            0xa2 => Ok(Self::SubtractEqual_IntInt),
            0x9f => Ok(Self::MultiplyEqual_IntFloat),
            0xa0 => Ok(Self::DivideEqual_IntFloat),
            0xb6 => Ok(Self::MultiplyEqual_FloatFloat),
            0xb7 => Ok(Self::DivideEqual_FloatFloat),
            0xb8 => Ok(Self::AddEqual_FloatFloat),
            0xb9 => Ok(Self::SubtractEqual_FloatFloat),
            0xdd => Ok(Self::MultiplyEqual_VectorFloat),
            0xde => Ok(Self::DivideEqual_VectorFloat),
            0xdf => Ok(Self::AddEqual_VectorVector),
            0xe0 => Ok(Self::SubtractEqual_VectorVector),
            0x129 => Ok(Self::MultiplyEqual_VectorVector),
            _ => Err(()),
        }
    }
}

pub(super) fn compound_assignment(
    assignment: CompoundAssignment,
    left: &Value,
    right: &Value,
) -> Result<Value> {
    if matches!(left, Value::None) {
        let zero = match assignment {
            CompoundAssignment::AddEqual_IntInt
            | CompoundAssignment::SubtractEqual_IntInt
            | CompoundAssignment::MultiplyEqual_IntFloat
            | CompoundAssignment::DivideEqual_IntFloat => Value::Int(0),
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
        (CompoundAssignment::SubtractEqual_IntInt, Value::Int(left), Value::Int(right)) => {
            Value::Int(left.wrapping_sub(*right))
        }
        (CompoundAssignment::MultiplyEqual_IntFloat, Value::Int(left), Value::Float(right)) => {
            Value::Int(compound_int_from_float(f64::from((*left as f32) * *right)))
        }
        (CompoundAssignment::DivideEqual_IntFloat, Value::Int(left), Value::Float(right)) => {
            Value::Int(if *right == 0.0 {
                0
            } else {
                compound_int_from_float(f64::from(*left) / f64::from(*right))
            })
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
        (
            CompoundAssignment::MultiplyEqual_VectorVector,
            Value::Vector(left),
            Value::Vector(right),
        ) => Value::Vector([left[0] * right[0], left[1] * right[1], left[2] * right[2]]),
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

fn compound_int_from_float(value: f64) -> i32 {
    if value.is_finite() && value > f64::from(i32::MIN) - 1.0 && value < f64::from(i32::MAX) + 1.0 {
        value as i32
    } else {
        i32::MIN
    }
}

pub(super) fn convert(opcode: ConversionOpcode, value: Value) -> Result<Value> {
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
            Value::Vector([pitch_cos * yaw_cos, pitch_cos * yaw_sin, pitch_sin])
        }
        (ConversionOpcode::ByteToInt, Value::Byte(value)) => Value::Int(i32::from(value)),
        (ConversionOpcode::ByteToInt, Value::Bool(value)) => Value::Int(i32::from(value)),
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
                (z.atan2((x * x + y * y).sqrt()) * units) as i32,
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

pub(super) fn parse_string_triplet(value: &str) -> Option<[f32; 3]> {
    let mut values = value.split(',').map(|value| value.trim().parse::<f32>());
    Some([
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    ])
}

pub(crate) fn rotator_axes([pitch, yaw, roll]: [i32; 3]) -> [[f32; 3]; 3] {
    let units_to_radians = std::f32::consts::TAU / 65_536.0;
    let (pitch_sin, pitch_cos) = ((pitch as f32) * units_to_radians).sin_cos();
    let (yaw_sin, yaw_cos) = ((yaw as f32) * units_to_radians).sin_cos();
    let (roll_sin, roll_cos) = ((roll as f32) * units_to_radians).sin_cos();
    [
        [pitch_cos * yaw_cos, pitch_cos * yaw_sin, pitch_sin],
        [
            roll_sin * pitch_sin * yaw_cos - roll_cos * yaw_sin,
            roll_sin * pitch_sin * yaw_sin + roll_cos * yaw_cos,
            -roll_sin * pitch_cos,
        ],
        [
            -roll_cos * pitch_sin * yaw_cos - roll_sin * yaw_sin,
            -roll_cos * pitch_sin * yaw_sin + roll_sin * yaw_cos,
            roll_cos * pitch_cos,
        ],
    ]
}
