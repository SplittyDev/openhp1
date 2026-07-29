use super::*;
use super::{
    actor::advance_lifespan,
    actor::advance_timer,
    actor::decode_latent_action,
    actor::update_touching_array,
    native::{
        animation_parameters, bone_number, collision_updates, log_arguments, next_navigation_step,
        noise_loudness, random_float, random_int, random_unit_vector, scalar_native,
        sound_arguments, target_score, trace_texture,
    },
    state::{event_disabled, probe_event_index, set_event_disabled},
};

#[test]
fn looping_timer_keeps_fractional_overshoot() {
    let mut timer = ActorTimer {
        remaining: 0.1,
        rate: 0.1,
        looping: true,
    };
    assert!(advance_timer(&mut timer, 0.15));
    assert!((timer.remaining - 0.05).abs() < 1.0e-6);
    assert!(!advance_timer(&mut timer, 0.04));
    assert!(advance_timer(&mut timer, 0.01));
    assert!((timer.remaining - 0.1).abs() < 1.0e-6);
}

#[test]
fn positive_lifespans_expire_once_at_zero() {
    let mut lifespan = 0.1;
    assert!(!advance_lifespan(&mut lifespan, 0.04));
    assert!(advance_lifespan(&mut lifespan, 0.07));
    assert_eq!(lifespan, 0.0);
    assert!(!advance_lifespan(&mut lifespan, 0.1));
}

#[test]
fn decodes_finish_interpolation_latent_state() {
    assert_eq!(
        decode_latent_action(0x12e, 7),
        LatentAction::FinishInterpolation(7)
    );
}

#[test]
fn touch_events_keep_the_engine_touching_array_in_sync() {
    let first = runtime_actor_id(1);
    let second = runtime_actor_id(2);
    let mut values = vec![
        StoredValue::Object(None),
        StoredValue::Object(None),
        StoredValue::Object(None),
        StoredValue::Object(None),
    ];

    update_touching_array(&mut values, first.clone(), true);
    update_touching_array(&mut values, first.clone(), true);
    update_touching_array(&mut values, second.clone(), true);
    assert!(matches!(
        &values[..],
        [
            StoredValue::Object(Some(value)),
            StoredValue::Object(Some(other)),
            StoredValue::Object(None),
            StoredValue::Object(None),
        ] if value == &first && other == &second
    ));

    update_touching_array(&mut values, first, false);
    assert!(matches!(values[0], StoredValue::Object(None)));
}

#[test]
fn integer_division_is_checked() {
    assert_eq!(
        scalar_native(0x91, &[Value::Int(7), Value::Int(2)]),
        Ok(Value::Int(3))
    );
    assert!(scalar_native(0x91, &[Value::Int(1), Value::Int(0)]).is_err());
    assert_eq!(
        scalar_native(0x9c, &[Value::Int(0x1_ffff), Value::Int(0xffff)]),
        Ok(Value::Int(0xffff))
    );
}

#[test]
fn float_remainder_uses_unreal_dividend_sign() {
    assert_eq!(
        scalar_native(0xad, &[Value::Float(-7.5), Value::Float(2.0)]),
        Ok(Value::Float(-1.5))
    );
}

#[test]
fn tangent_uses_radians() {
    let Value::Float(value) =
        scalar_native(0xbd, &[Value::Float(std::f32::consts::FRAC_PI_4)]).unwrap()
    else {
        panic!("expected float");
    };
    assert!((value - 1.0).abs() < 1.0e-6);
}

#[test]
fn bone_numbers_follow_case_insensitive_skeletal_order() {
    let bones = vec!["Root".to_owned(), "Head".to_owned()];
    assert_eq!(bone_number(Some(&bones), "head"), 1);
    assert_eq!(bone_number(Some(&bones), "missing"), 0);
}

#[test]
fn scalar_natives_distinguish_bad_operands_from_unknown_indices() {
    assert_eq!(
        scalar_native(0x97, &[Value::Byte(1), Value::Int(0)]),
        Err("Greater_IntInt does not accept operands (byte, int)".to_owned())
    );
    assert_eq!(
        scalar_native(0xffff, &[]),
        Err("native 0xffff is not implemented".to_owned())
    );
}

#[test]
fn named_native_shims_validate_their_engine_calls() {
    assert_eq!(
        execution::named_native(
            "PlayerPawn",
            "ConsoleCommand",
            &[Value::String("GETPING".to_owned())]
        ),
        Some(Value::String(String::new()))
    );
    assert_eq!(
        execution::named_native("Decal", "DetachDecal", &[]),
        Some(Value::None)
    );
    assert_eq!(
        execution::named_native("PlayerPawn", "ConsoleCommand", &[]),
        None
    );
}

#[test]
fn scalar_comparisons_cover_bool_int_and_float_families() {
    assert_eq!(
        scalar_native(0x98, &[Value::None, Value::Int(0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x83, &[Value::Bool(true), Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf2, &[Value::Bool(true), Value::Bool(true)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf3, &[Value::Bool(true), Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xf2, &[Value::None, Value::Bool(false)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x98, &[Value::Int(2), Value::Int(2)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0x99, &[Value::Int(3), Value::Int(2)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb2, &[Value::Float(2.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb3, &[Value::Float(3.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb4, &[Value::Float(2.0), Value::Float(2.0)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(0xb5, &[Value::Float(f32::NAN), Value::Float(f32::NAN)]),
        Ok(Value::Bool(true))
    );
}

#[test]
fn pick_target_score_rejects_targets_behind_or_beyond_range() {
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(100.0, 0.0, 0.0),
            0.5
        ),
        Some((1.0, 100.0))
    );
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(-100.0, 0.0, 0.0),
            0.0
        ),
        None
    );
    assert_eq!(
        target_score(
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::new(2_501.0, 0.0, 0.0),
            0.0
        ),
        None
    );
}

#[test]
fn observed_string_natives_match_unreal_semantics() {
    assert_eq!(
        scalar_native(
            0x70,
            &[
                Value::String("Harry".to_owned()),
                Value::String(" Potter".to_owned())
            ]
        ),
        Ok(Value::String("Harry Potter".to_owned()))
    );
    assert_eq!(
        scalar_native(
            0xa8,
            &[
                Value::String("Hello".to_owned()),
                Value::String("world".to_owned())
            ]
        ),
        Ok(Value::String("Hello world".to_owned()))
    );
    assert_eq!(
        scalar_native(
            0x7f,
            &[
                Value::String("Hogwarts".to_owned()),
                Value::Int(3),
                Value::Int(4)
            ]
        ),
        Ok(Value::String("wart".to_owned()))
    );
    assert_eq!(
        scalar_native(0xea, &[Value::String("Hogwarts".to_owned()), Value::Int(4)]),
        Ok(Value::String("arts".to_owned()))
    );
    assert_eq!(
        scalar_native(0xec, &[Value::Int(0x141)]),
        Ok(Value::String("A".to_owned()))
    );
    assert_eq!(
        scalar_native(0xed, &[Value::String("Alohomora".to_owned())]),
        Ok(Value::Int(65))
    );
}

#[test]
fn collision_updates_preserve_omitted_flags() {
    assert_eq!(
        collision_updates(&[Value::Bool(true), Value::None]),
        Ok([Some(true), None, None])
    );
    assert!(collision_updates(&[Value::Float(1.0)]).is_err());
}

#[test]
fn log_arguments_preserve_optional_tags() {
    assert_eq!(
        log_arguments(&[Value::String("hello".to_owned()), Value::Name(7)]),
        Ok(("hello", Some(&Value::Name(7))))
    );
    assert!(log_arguments(&[Value::Int(1)]).is_err());
    assert!(log_arguments(&[Value::String("hello".to_owned()), Value::None, Value::None]).is_err());
}

#[test]
fn noise_loudness_must_be_a_finite_float() {
    assert_eq!(noise_loudness(&[Value::Float(0.5)]), Ok(0.5));
    assert!(noise_loudness(&[Value::Float(f32::NAN)]).is_err());
    assert!(noise_loudness(&[]).is_err());
}

#[test]
fn landing_surface_and_sound_natives_validate_calls() {
    assert_eq!(
        trace_texture(&[
            Value::Vector([0.0; 3]),
            Value::Vector([0.0, 0.0, -16.0]),
            Value::Int(0),
            Value::Bool(false),
        ]),
        Ok(Value::Object(0))
    );
    assert!(trace_texture(&[Value::Int(0)]).is_err());
    assert!(
        sound_arguments(
            "PlayOwnedSound",
            &[
                Value::Object(1),
                Value::Byte(0),
                Value::Float(1.0),
                Value::Bool(false),
            ],
        )
        .is_ok()
    );
    assert!(sound_arguments("PlayOwnedSound", &[Value::Int(1)]).is_err());
}

#[test]
fn disabled_events_are_case_insensitive_and_scoped_to_actor_state() {
    let mut disabled = HashMap::default();
    set_event_disabled(&mut disabled, 2, Some("Beano"), "Tick", true);

    assert!(event_disabled(&disabled, 2, Some("beano"), "TICK"));
    assert!(!event_disabled(&disabled, 2, Some("KillBean"), "Tick"));
    assert!(!event_disabled(&disabled, 3, Some("Beano"), "Tick"));

    set_event_disabled(&mut disabled, 2, Some("BEANO"), "tick", false);
    assert!(!event_disabled(&disabled, 2, Some("Beano"), "Tick"));
    assert_eq!(probe_event_index("tick"), Some(36));
}

#[test]
fn float_min_max_match_unreal_native_ordering() {
    assert_eq!(
        scalar_native(0xf4, &[Value::Float(2.0), Value::Float(3.0)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(0xf4, &[Value::Float(2.0), Value::Float(f32::NAN)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(0xf5, &[Value::Float(2.0), Value::Float(3.0)]),
        Ok(Value::Float(3.0))
    );
    assert_eq!(
        scalar_native(0xf5, &[Value::Float(2.0), Value::Float(f32::NAN)]),
        Ok(Value::Float(2.0))
    );
    assert_eq!(
        scalar_native(
            0xf6,
            &[Value::Float(5.0), Value::Float(1.0), Value::Float(3.0)]
        ),
        Ok(Value::Float(3.0))
    );
}

#[test]
fn basic_vector_arithmetic_matches_unreal_natives() {
    assert_eq!(scalar_native(0x8f, &[Value::Int(7)]), Ok(Value::Int(-7)));
    assert_eq!(
        scalar_native(
            0xd9,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([1.0, 2.0, 3.0])
            ]
        ),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        scalar_native(
            0xd7,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([4.0, 5.0, 6.0])
            ]
        ),
        Ok(Value::Vector([5.0, 7.0, 9.0]))
    );
    assert_eq!(
        scalar_native(0xd4, &[Value::Vector([1.0, 2.0, 3.0]), Value::Float(2.0)]),
        Ok(Value::Vector([2.0, 4.0, 6.0]))
    );
    assert_eq!(
        scalar_native(
            0xdb,
            &[
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Vector([4.0, 5.0, 6.0])
            ]
        ),
        Ok(Value::Float(32.0))
    );
    let quarter_yaw = Value::Rotator([0, 16_384, 0]);
    let Value::Vector(rotated) = scalar_native(
        0x114,
        &[Value::Vector([1.0, 0.0, 0.0]), quarter_yaw.clone()],
    )
    .unwrap() else {
        panic!("expected vector rotation");
    };
    assert!(glam::Vec3::from_array(rotated).abs_diff_eq(glam::Vec3::Y, 1.0e-6));
    let Value::Vector(unrotated) =
        scalar_native(0x113, &[Value::Vector(rotated), quarter_yaw]).unwrap()
    else {
        panic!("expected inverse vector rotation");
    };
    assert!(glam::Vec3::from_array(unrotated).abs_diff_eq(glam::Vec3::X, 1.0e-6));
    assert_eq!(
        scalar_native(
            0x12c,
            &[
                Value::Vector([1.0, -2.0, 3.0]),
                Value::Vector([0.0, 1.0, 0.0])
            ],
        ),
        Ok(Value::Vector([1.0, 2.0, 3.0]))
    );
}

#[test]
fn navigation_uses_the_shortest_unpruned_reachable_step() {
    let start = runtime_actor_id(1);
    let short = runtime_actor_id(2);
    let long = runtime_actor_id(3);
    let target = runtime_actor_id(4);
    let spec = |start, end, distance| NavigationReachSpec {
        distance,
        start,
        end,
        collision_radius: 40,
        collision_height: 40,
        pruned: false,
    };
    let specs = [
        spec(start.clone(), long.clone(), 10),
        spec(long, target.clone(), 10),
        spec(start.clone(), short.clone(), 3),
        spec(short.clone(), target.clone(), 3),
    ];
    assert_eq!(
        next_navigation_step(&specs, &start, &target, 20, 20),
        Some(short)
    );
    assert_eq!(next_navigation_step(&specs, &start, &target, 50, 20), None);
}

#[test]
fn rotator_addition_wraps_each_ue1_component() {
    assert_eq!(
        scalar_native(
            0x13c,
            &[
                Value::Rotator([i32::MAX, 2, -4]),
                Value::Rotator([1, 3, -5])
            ]
        ),
        Ok(Value::Rotator([i32::MIN, 5, -9]))
    );
    assert_eq!(
        scalar_native(
            0x13d,
            &[
                Value::Rotator([i32::MIN, i32::MAX, 1]),
                Value::Rotator([1, -1, 2])
            ]
        ),
        Ok(Value::Rotator([i32::MAX, i32::MIN, -1]))
    );
}

#[test]
fn requested_core_math_and_random_natives_match_unreal_semantics() {
    assert_eq!(
        scalar_native(0xc1, &[Value::Float(9.0)]),
        Ok(Value::Float(3.0))
    );
    assert_eq!(
        scalar_native(0xfb, &[Value::Int(12), Value::Int(-5), Value::Int(10)]),
        Ok(Value::Int(10))
    );
    assert_eq!(
        scalar_native(0xba, &[Value::Float(-2.5)]),
        Ok(Value::Float(2.5))
    );
    assert_eq!(
        scalar_native(0xe2, &[Value::Vector([3.0, 0.0, 4.0])]),
        Ok(Value::Vector([0.6, 0.0, 0.8]))
    );
    assert_eq!(
        scalar_native(0xe2, &[Value::Vector([0.0; 3])]),
        Ok(Value::Vector([0.0; 3]))
    );

    let mut state = 0x6d2b_79f5;
    for _ in 0..100 {
        assert!((0..7).contains(&random_int(&mut state, 7)));
        assert!((0.0..1.0).contains(&random_float(&mut state)));
    }
    assert_eq!(random_int(&mut state, 0), 0);
}

#[test]
fn random_vectors_are_normalized_and_deterministic() {
    let mut first = 0x6d2b_79f5;
    let mut second = first;
    let vector = random_unit_vector(&mut first);
    assert!((vector.length() - 1.0).abs() < 1.0e-6);
    assert_eq!(vector, random_unit_vector(&mut second));
}

#[test]
fn animation_parameters_preserve_optional_tween_time() {
    assert_eq!(
        animation_parameters("LoopAnim", &[Value::None, Value::Float(0.5)]),
        Ok((1.0, 0.5))
    );
    assert_eq!(animation_parameters("PlayAnim", &[]), Ok((1.0, 0.0)));
}

#[test]
fn function_lookups_are_case_insensitive_and_state_scoped() {
    let class = ObjectId {
        package: Arc::from("Test.u"),
        export_index: 7,
    };
    let lower = FunctionLookup::new(class.clone(), Some("patrol"), "tick", 2);
    let upper = FunctionLookup::new(class.clone(), Some("PATROL"), "TICK", 2);
    let other_state = FunctionLookup::new(class, Some("waiting"), "tick", 2);

    assert_eq!(lower, upper);
    assert_ne!(lower, other_state);
}

#[test]
fn state_lookups_are_case_insensitive() {
    let class = ObjectId {
        package: Arc::from("Test.u"),
        export_index: 7,
    };
    assert_eq!(
        StateLookup::new(class.clone(), "patrol"),
        StateLookup::new(class, "PATROL")
    );
}
