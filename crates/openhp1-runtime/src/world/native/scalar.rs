use glam::Vec3;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ScalarNative {
    Concat_StrStr,
    At_StrStr,
    EqualEqual_ObjectObject,
    NotEqual_ObjectObject,
    EqualEqual_StrStr,
    NotEqual_StrStr,
    ComplementEqual_StrStr,
    Len,
    InStr,
    Mid,
    Left,
    Right,
    Caps,
    Not_PreBool,
    AndAnd_BoolBool,
    XorXor_BoolBool,
    OrOr_BoolBool,
    Subtract_PreInt,
    Multiply_IntInt,
    Divide_IntInt,
    Add_IntInt,
    Subtract_IntInt,
    LessLess_IntInt,
    GreaterGreater_IntInt,
    GreaterGreaterGreater_IntInt,
    Less_IntInt,
    Greater_IntInt,
    LessEqual_IntInt,
    GreaterEqual_IntInt,
    EqualEqual_IntInt,
    NotEqual_IntInt,
    And_IntInt,
    Subtract_PreFloat,
    Multiply_FloatFloat,
    Divide_FloatFloat,
    Percent_FloatFloat,
    Add_FloatFloat,
    Subtract_FloatFloat,
    Less_FloatFloat,
    Greater_FloatFloat,
    LessEqual_FloatFloat,
    GreaterEqual_FloatFloat,
    EqualEqual_FloatFloat,
    NotEqual_FloatFloat,
    Abs,
    Sin,
    Cos,
    Tan,
    Sqrt,
    Subtract_PreVector,
    Multiply_VectorFloat,
    Multiply_FloatVector,
    Multiply_VectorVector,
    Divide_VectorFloat,
    Add_VectorVector,
    Subtract_VectorVector,
    LessLess_VectorRotator,
    GreaterGreater_VectorRotator,
    EqualEqual_VectorVector,
    NotEqual_VectorVector,
    Dot_VectorVector,
    Cross_VectorVector,
    VSize,
    Normal,
    MirrorVectorByNormal,
    FMin,
    FMax,
    FClamp,
    Min,
    Max,
    Clamp,
    EqualEqual_BoolBool,
    NotEqual_BoolBool,
    Chr,
    Asc,
    Multiply_RotatorFloat,
    Multiply_FloatRotator,
    Divide_RotatorFloat,
    Add_RotatorRotator,
    Subtract_RotatorRotator,
}

impl TryFrom<u16> for ScalarNative {
    type Error = ();

    fn try_from(index: u16) -> std::result::Result<Self, Self::Error> {
        match index {
            0x70 => Ok(Self::Concat_StrStr),
            0xa8 => Ok(Self::At_StrStr),
            0x72 => Ok(Self::EqualEqual_ObjectObject),
            0x77 => Ok(Self::NotEqual_ObjectObject),
            0x7a => Ok(Self::EqualEqual_StrStr),
            0x7b => Ok(Self::NotEqual_StrStr),
            0x7c => Ok(Self::ComplementEqual_StrStr),
            0x7d => Ok(Self::Len),
            0x7e => Ok(Self::InStr),
            0x7f => Ok(Self::Mid),
            0x80 => Ok(Self::Left),
            0x81 => Ok(Self::Not_PreBool),
            0x82 => Ok(Self::AndAnd_BoolBool),
            0x83 => Ok(Self::XorXor_BoolBool),
            0x84 => Ok(Self::OrOr_BoolBool),
            0x8f => Ok(Self::Subtract_PreInt),
            0x90 => Ok(Self::Multiply_IntInt),
            0x91 => Ok(Self::Divide_IntInt),
            0x92 => Ok(Self::Add_IntInt),
            0x93 => Ok(Self::Subtract_IntInt),
            0x94 => Ok(Self::LessLess_IntInt),
            0x95 => Ok(Self::GreaterGreater_IntInt),
            0xc4 => Ok(Self::GreaterGreaterGreater_IntInt),
            0x96 => Ok(Self::Less_IntInt),
            0x97 => Ok(Self::Greater_IntInt),
            0x98 => Ok(Self::LessEqual_IntInt),
            0x99 => Ok(Self::GreaterEqual_IntInt),
            0x9a => Ok(Self::EqualEqual_IntInt),
            0x9b => Ok(Self::NotEqual_IntInt),
            0x9c => Ok(Self::And_IntInt),
            0xa9 => Ok(Self::Subtract_PreFloat),
            0xab => Ok(Self::Multiply_FloatFloat),
            0xac => Ok(Self::Divide_FloatFloat),
            0xad => Ok(Self::Percent_FloatFloat),
            0xae => Ok(Self::Add_FloatFloat),
            0xaf => Ok(Self::Subtract_FloatFloat),
            0xb0 => Ok(Self::Less_FloatFloat),
            0xb1 => Ok(Self::Greater_FloatFloat),
            0xb2 => Ok(Self::LessEqual_FloatFloat),
            0xb3 => Ok(Self::GreaterEqual_FloatFloat),
            0xb4 => Ok(Self::EqualEqual_FloatFloat),
            0xb5 => Ok(Self::NotEqual_FloatFloat),
            0xba => Ok(Self::Abs),
            0xbb => Ok(Self::Sin),
            0xbc => Ok(Self::Cos),
            0xbd => Ok(Self::Tan),
            0xc1 => Ok(Self::Sqrt),
            0xd3 => Ok(Self::Subtract_PreVector),
            0xd4 => Ok(Self::Multiply_VectorFloat),
            0xd5 => Ok(Self::Multiply_FloatVector),
            0x128 => Ok(Self::Multiply_VectorVector),
            0xd6 => Ok(Self::Divide_VectorFloat),
            0xd7 => Ok(Self::Add_VectorVector),
            0xd8 => Ok(Self::Subtract_VectorVector),
            0x113 => Ok(Self::LessLess_VectorRotator),
            0x114 => Ok(Self::GreaterGreater_VectorRotator),
            0xd9 => Ok(Self::EqualEqual_VectorVector),
            0xda => Ok(Self::NotEqual_VectorVector),
            0xdb => Ok(Self::Dot_VectorVector),
            0xdc => Ok(Self::Cross_VectorVector),
            0xe1 => Ok(Self::VSize),
            0xe2 => Ok(Self::Normal),
            0x12c => Ok(Self::MirrorVectorByNormal),
            0xea => Ok(Self::Right),
            0xeb => Ok(Self::Caps),
            0xec => Ok(Self::Chr),
            0xed => Ok(Self::Asc),
            0xf2 => Ok(Self::EqualEqual_BoolBool),
            0xf3 => Ok(Self::NotEqual_BoolBool),
            0xf4 => Ok(Self::FMin),
            0xf5 => Ok(Self::FMax),
            0xf6 => Ok(Self::FClamp),
            0xf9 => Ok(Self::Min),
            0xfa => Ok(Self::Max),
            0xfb => Ok(Self::Clamp),
            0x11f => Ok(Self::Multiply_RotatorFloat),
            0x120 => Ok(Self::Multiply_FloatRotator),
            0x121 => Ok(Self::Divide_RotatorFloat),
            0x13c => Ok(Self::Add_RotatorRotator),
            0x13d => Ok(Self::Subtract_RotatorRotator),
            _ => Err(()),
        }
    }
}

pub(in crate::world) fn scalar_native(
    index: u16,
    arguments: &[Value],
) -> std::result::Result<Value, String> {
    let native = ScalarNative::try_from(index)
        .map_err(|()| format!("native {index:#05x} is not implemented"))?;
    let normalized = null_numeric_value(native).and_then(|zero| {
        arguments
            .iter()
            .any(|value| matches!(value, Value::None))
            .then(|| {
                arguments
                    .iter()
                    .map(|value| {
                        if matches!(value, Value::None) {
                            zero.clone()
                        } else {
                            value.clone()
                        }
                    })
                    .collect::<Vec<_>>()
            })
    });
    let arguments = normalized.as_deref().unwrap_or(arguments);
    if matches!(native, ScalarNative::FMin | ScalarNative::FMax) {
        let [Value::Float(left), Value::Float(right)] = arguments else {
            return Err(format!(
                "{native:?} expects two floats, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let value = match native {
            ScalarNative::FMin if right < left => *right,
            ScalarNative::FMax if left < right => *right,
            _ => *left,
        };
        return Ok(Value::Float(value));
    }
    if native == ScalarNative::FClamp
        && let [Value::Float(value), Value::Float(min), Value::Float(max)] = arguments
    {
        return Ok(Value::Float(value.min(*max).max(*min)));
    }
    if matches!(native, ScalarNative::Min | ScalarNative::Max)
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return Ok(Value::Int(if native == ScalarNative::Min {
            (*left).min(*right)
        } else {
            (*left).max(*right)
        }));
    }
    if matches!(
        native,
        ScalarNative::EqualEqual_ObjectObject | ScalarNative::NotEqual_ObjectObject
    ) && let [left, right] = arguments
        && let (Some(left), Some(right)) = (object_value(left), object_value(right))
    {
        return Ok(Value::Bool(
            (left == right) == (native == ScalarNative::EqualEqual_ObjectObject),
        ));
    }
    if matches!(
        native,
        ScalarNative::EqualEqual_BoolBool | ScalarNative::NotEqual_BoolBool
    ) && let [left, right] = arguments
    {
        let equal = left.truthy().map_err(|error| error.to_string())?
            == right.truthy().map_err(|error| error.to_string())?;
        return Ok(Value::Bool(
            equal == (native == ScalarNative::EqualEqual_BoolBool),
        ));
    }
    if native == ScalarNative::Divide_IntInt
        && let [Value::Int(left), Value::Int(right)] = arguments
    {
        return left
            .checked_div(*right)
            .map(Value::Int)
            .ok_or_else(|| "integer division by zero or overflow".to_owned());
    }
    Ok(match (native, arguments) {
        (ScalarNative::Concat_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::String(left.clone() + right)
        }
        (ScalarNative::At_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::String(format!("{left} {right}"))
        }
        (ScalarNative::Not_PreBool, [value]) => {
            Value::Bool(!value.truthy().map_err(|error| error.to_string())?)
        }
        (ScalarNative::AndAnd_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                && right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::XorXor_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                != right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::OrOr_BoolBool, [left, right]) => Value::Bool(
            left.truthy().map_err(|error| error.to_string())?
                || right.truthy().map_err(|error| error.to_string())?,
        ),
        (ScalarNative::Subtract_PreInt, [Value::Int(value)]) => Value::Int(value.wrapping_neg()),
        (ScalarNative::Multiply_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left * right)
        }
        (ScalarNative::Add_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left + right)
        }
        (ScalarNative::Subtract_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left - right)
        }
        (ScalarNative::LessLess_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left.wrapping_shl(*right as u32))
        }
        (ScalarNative::GreaterGreater_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left.wrapping_shr(*right as u32))
        }
        (ScalarNative::GreaterGreaterGreater_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(((*left as u32).wrapping_shr(*right as u32)) as i32)
        }
        (ScalarNative::Less_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left < right)
        }
        (ScalarNative::Greater_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left > right)
        }
        (ScalarNative::LessEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left <= right)
        }
        (ScalarNative::GreaterEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left >= right)
        }
        (ScalarNative::EqualEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::And_IntInt, [Value::Int(left), Value::Int(right)]) => {
            Value::Int(left & right)
        }
        (ScalarNative::EqualEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::NotEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(!left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::ComplementEqual_StrStr, [Value::String(left), Value::String(right)]) => {
            Value::Bool(left.eq_ignore_ascii_case(right))
        }
        (ScalarNative::Len, [Value::String(value)]) => Value::Int(value.chars().count() as i32),
        (ScalarNative::InStr, [Value::String(value), Value::String(needle)]) => Value::Int(
            value
                .find(needle)
                .map_or(-1, |index| value[..index].chars().count() as i32),
        ),
        (
            ScalarNative::Mid,
            [Value::String(value), Value::Int(start)]
            | [Value::String(value), Value::Int(start), Value::None],
        ) => {
            let start = usize::try_from(*start).unwrap_or_default();
            Value::String(value.chars().skip(start).collect())
        }
        (ScalarNative::Mid, [Value::String(value), Value::Int(start), Value::Int(length)]) => {
            let start = usize::try_from(*start).unwrap_or_default();
            let length = usize::try_from(*length).unwrap_or_default();
            Value::String(value.chars().skip(start).take(length).collect())
        }
        (ScalarNative::Left, [Value::String(value), Value::Int(length)]) => {
            let length = usize::try_from(*length).unwrap_or_default();
            Value::String(value.chars().take(length).collect())
        }
        (ScalarNative::Right, [Value::String(value), Value::Int(length)]) => {
            let length = usize::try_from(*length).unwrap_or_default();
            let skip = value.chars().count().saturating_sub(length);
            Value::String(value.chars().skip(skip).collect())
        }
        (ScalarNative::Caps, [Value::String(value)]) => Value::String(value.to_uppercase()),
        (ScalarNative::Asc, [Value::String(value)]) => Value::Int(i32::from(
            value.as_bytes().first().copied().unwrap_or_default(),
        )),
        (ScalarNative::Subtract_PreFloat, [Value::Float(value)]) => Value::Float(-value),
        (ScalarNative::Multiply_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left * right)
        }
        (ScalarNative::Divide_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left / right)
        }
        (ScalarNative::Percent_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left % right)
        }
        (ScalarNative::Add_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left + right)
        }
        (ScalarNative::Subtract_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Float(left - right)
        }
        (ScalarNative::Less_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left < right)
        }
        (ScalarNative::Greater_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left > right)
        }
        (ScalarNative::LessEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left <= right)
        }
        (ScalarNative::GreaterEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left >= right)
        }
        (ScalarNative::EqualEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_FloatFloat, [Value::Float(left), Value::Float(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::Abs, [Value::Float(value)]) => Value::Float(value.abs()),
        (ScalarNative::Sin, [Value::Float(value)]) => Value::Float(value.sin()),
        (ScalarNative::Cos, [Value::Float(value)]) => Value::Float(value.cos()),
        (ScalarNative::Tan, [Value::Float(value)]) => Value::Float(value.tan()),
        (ScalarNative::Sqrt, [Value::Float(value)]) => Value::Float(value.sqrt()),
        (ScalarNative::Subtract_PreVector, [Value::Vector(value)]) => {
            Value::Vector([-value[0], -value[1], -value[2]])
        }
        (ScalarNative::Multiply_VectorFloat, [Value::Vector(value), Value::Float(scale)])
        | (ScalarNative::Multiply_FloatVector, [Value::Float(scale), Value::Vector(value)]) => {
            Value::Vector([value[0] * scale, value[1] * scale, value[2] * scale])
        }
        (ScalarNative::Multiply_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] * right[0], left[1] * right[1], left[2] * right[2]])
        }
        (ScalarNative::Divide_VectorFloat, [Value::Vector(value), Value::Float(divisor)]) => {
            Value::Vector([value[0] / divisor, value[1] / divisor, value[2] / divisor])
        }
        (ScalarNative::Add_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] + right[0], left[1] + right[1], left[2] + right[2]])
        }
        (ScalarNative::Subtract_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector([left[0] - right[0], left[1] - right[1], left[2] - right[2]])
        }
        (
            ScalarNative::LessLess_VectorRotator,
            [Value::Vector(vector), Value::Rotator(rotation)],
        ) => {
            let [x, y, z] = crate::rotator_axes(*rotation);
            Value::Vector([
                x[0] * vector[0] + x[1] * vector[1] + x[2] * vector[2],
                y[0] * vector[0] + y[1] * vector[1] + y[2] * vector[2],
                z[0] * vector[0] + z[1] * vector[1] + z[2] * vector[2],
            ])
        }
        (
            ScalarNative::GreaterGreater_VectorRotator,
            [Value::Vector(vector), Value::Rotator(rotation)],
        ) => {
            let [x, y, z] = crate::rotator_axes(*rotation);
            Value::Vector([
                x[0] * vector[0] + y[0] * vector[1] + z[0] * vector[2],
                x[1] * vector[0] + y[1] * vector[1] + z[1] * vector[2],
                x[2] * vector[0] + y[2] * vector[1] + z[2] * vector[2],
            ])
        }
        (ScalarNative::EqualEqual_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Bool(left == right)
        }
        (ScalarNative::NotEqual_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Bool(left != right)
        }
        (ScalarNative::Dot_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Float(left[0] * right[0] + left[1] * right[1] + left[2] * right[2])
        }
        (ScalarNative::Cross_VectorVector, [Value::Vector(left), Value::Vector(right)]) => {
            Value::Vector(
                Vec3::from_array(*left)
                    .cross(Vec3::from_array(*right))
                    .to_array(),
            )
        }
        (ScalarNative::VSize, [Value::Vector(value)]) => {
            Value::Float((value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt())
        }
        (ScalarNative::Normal, [Value::Vector(value)]) => {
            let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
            if length > f32::EPSILON {
                Value::Vector([value[0] / length, value[1] / length, value[2] / length])
            } else {
                Value::Vector([0.0; 3])
            }
        }
        (ScalarNative::MirrorVectorByNormal, [Value::Vector(vector), Value::Vector(normal)]) => {
            let scale =
                2.0 * (vector[0] * normal[0] + vector[1] * normal[1] + vector[2] * normal[2]);
            Value::Vector([
                vector[0] - scale * normal[0],
                vector[1] - scale * normal[1],
                vector[2] - scale * normal[2],
            ])
        }
        (ScalarNative::Clamp, [Value::Int(value), Value::Int(min), Value::Int(max)]) => {
            Value::Int((*value).min(*max).max(*min))
        }
        (ScalarNative::Chr, [Value::Int(value)]) => {
            Value::String(char::from(*value as u8).to_string())
        }
        (ScalarNative::Multiply_RotatorFloat, [Value::Rotator(value), Value::Float(scale)])
        | (ScalarNative::Multiply_FloatRotator, [Value::Float(scale), Value::Rotator(value)]) => {
            Value::Rotator([
                (value[0] as f32 * scale) as i32,
                (value[1] as f32 * scale) as i32,
                (value[2] as f32 * scale) as i32,
            ])
        }
        (ScalarNative::Divide_RotatorFloat, [Value::Rotator(value), Value::Float(scale)]) => {
            Value::Rotator([
                (value[0] as f32 / scale) as i32,
                (value[1] as f32 / scale) as i32,
                (value[2] as f32 / scale) as i32,
            ])
        }
        (ScalarNative::Add_RotatorRotator, [Value::Rotator(left), Value::Rotator(right)]) => {
            Value::Rotator([
                left[0].wrapping_add(right[0]),
                left[1].wrapping_add(right[1]),
                left[2].wrapping_add(right[2]),
            ])
        }
        (ScalarNative::Subtract_RotatorRotator, [Value::Rotator(left), Value::Rotator(right)]) => {
            Value::Rotator([
                left[0].wrapping_sub(right[0]),
                left[1].wrapping_sub(right[1]),
                left[2].wrapping_sub(right[2]),
            ])
        }
        _ => {
            return Err(format!(
                "{native:?} does not accept operands ({})",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    })
}

fn null_numeric_value(native: ScalarNative) -> Option<Value> {
    Some(match native {
        ScalarNative::Subtract_PreInt
        | ScalarNative::Multiply_IntInt
        | ScalarNative::Divide_IntInt
        | ScalarNative::Add_IntInt
        | ScalarNative::Subtract_IntInt
        | ScalarNative::LessLess_IntInt
        | ScalarNative::GreaterGreaterGreater_IntInt
        | ScalarNative::Less_IntInt
        | ScalarNative::Greater_IntInt
        | ScalarNative::LessEqual_IntInt
        | ScalarNative::GreaterEqual_IntInt
        | ScalarNative::EqualEqual_IntInt
        | ScalarNative::NotEqual_IntInt
        | ScalarNative::And_IntInt
        | ScalarNative::Min
        | ScalarNative::Max
        | ScalarNative::Clamp
        | ScalarNative::Chr => Value::Int(0),
        ScalarNative::Subtract_PreFloat
        | ScalarNative::Multiply_FloatFloat
        | ScalarNative::Divide_FloatFloat
        | ScalarNative::Add_FloatFloat
        | ScalarNative::Subtract_FloatFloat
        | ScalarNative::Less_FloatFloat
        | ScalarNative::Greater_FloatFloat
        | ScalarNative::LessEqual_FloatFloat
        | ScalarNative::GreaterEqual_FloatFloat
        | ScalarNative::EqualEqual_FloatFloat
        | ScalarNative::NotEqual_FloatFloat
        | ScalarNative::Abs
        | ScalarNative::Sqrt
        | ScalarNative::FMin
        | ScalarNative::FMax
        | ScalarNative::FClamp => Value::Float(0.0),
        _ => return None,
    })
}

pub(in crate::world) fn target_score(
    start: Vec3,
    direction: Vec3,
    target: Vec3,
    best_aim: f32,
) -> Option<(f32, f32)> {
    let delta = target - start;
    let distance = delta.length();
    if distance == 0.0 || distance > 2_500.0 {
        return None;
    }
    let aim = direction.dot(delta) / distance;
    (aim >= best_aim && aim >= 0.0).then_some((aim, distance))
}

pub(in crate::world) fn bone_number(bones: Option<&[String]>, name: &str) -> i32 {
    bones
        .and_then(|bones| {
            bones
                .iter()
                .position(|bone| bone.eq_ignore_ascii_case(name))
        })
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0)
}

fn next_random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

pub(in crate::world) fn random_int(state: &mut u32, max: i32) -> i32 {
    let range = max.saturating_sub(1).clamp(0, 32_767) as u32 + 1;
    ((u64::from(next_random(state)) * u64::from(range)) >> 32) as i32
}

pub(in crate::world) fn random_float(state: &mut u32) -> f32 {
    (next_random(state) >> 8) as f32 / 16_777_216.0
}

pub(in crate::world) fn random_rotator(state: &mut u32, roll: bool) -> [i32; 3] {
    let yaw = (next_random(state) >> 16) as i32;
    let pitch = (next_random(state) >> 16) as i32;
    let roll = roll.then(|| (next_random(state) >> 16) as i32).unwrap_or(0);
    [pitch, yaw, roll]
}

pub(in crate::world) fn random_unit_vector(state: &mut u32) -> Vec3 {
    loop {
        let vector = Vec3::new(
            random_float(state) * 2.0 - 1.0,
            random_float(state) * 2.0 - 1.0,
            random_float(state) * 2.0 - 1.0,
        );
        let length_squared = vector.length_squared();
        if length_squared > f32::EPSILON && length_squared <= 1.0 {
            return vector / length_squared.sqrt();
        }
    }
}

pub(in crate::world) fn object_value(value: &Value) -> Option<i32> {
    match value {
        Value::None => Some(0),
        Value::Object(value) => Some(*value),
        _ => None,
    }
}
use super::*;
