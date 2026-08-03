use super::*;

#[test]
fn omitted_sound_arguments_remain_authored_defaults() {
    assert_eq!(
        sound_arguments("PlaySound", &[Value::Object(1)]).unwrap(),
        SoundArguments {
            sound: Some(1),
            slot: None,
            volume: None,
            no_override: false,
            radius: None,
            pitch: None,
        }
    );
}

#[test]
fn modify_sound_uses_the_shipped_parameter_value_sound_slot_order() {
    assert_eq!(
        modify_sound_arguments(&[Value::Byte(2), Value::Float(0.75)]).unwrap(),
        ModifySoundArguments {
            parameter: 2,
            value: 0.75,
            sound: None,
            slot: 0,
        }
    );
    assert_eq!(
        modify_sound_arguments(&[
            Value::Byte(1),
            Value::Float(0.5),
            Value::Object(7),
            Value::Byte(3),
        ])
        .unwrap(),
        ModifySoundArguments {
            parameter: 1,
            value: 0.5,
            sound: Some(7),
            slot: 3,
        }
    );
}
