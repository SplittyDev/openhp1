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
