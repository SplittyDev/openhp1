use super::native::runtime_name;
use super::*;
use crate::IteratorValue;
use glam::Vec3;

mod dispatch;
mod object;

struct CallOutput {
    value: Value,
    outputs: Vec<(usize, Value)>,
}

impl CallOutput {
    fn value(value: Value) -> Self {
        Self {
            value,
            outputs: Vec::new(),
        }
    }

    fn from_arguments(value: Value, arguments: &[Value], output_arguments: Vec<Value>) -> Self {
        let outputs = arguments
            .iter()
            .zip(output_arguments)
            .enumerate()
            .filter_map(|(index, (input, output))| (input != &output).then_some((index, output)))
            .collect();
        Self { value, outputs }
    }

    fn into_response(self) -> FrameResponse {
        if self.outputs.is_empty() {
            FrameResponse::Value(self.value)
        } else {
            FrameResponse::ValueWithOutputs {
                value: self.value,
                outputs: self.outputs,
            }
        }
    }
}

impl ScriptRuntime {
    pub(super) fn execute_actor_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        self.execute_actor_function_inner(actor, actor_class, function, arguments, None)
    }

    pub(super) fn execute_actor_function_with_outputs(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        output_arguments: &mut Vec<Value>,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.execute_actor_function_inner(
            actor,
            actor_class,
            function,
            arguments,
            Some(output_arguments),
        )
    }

    fn execute_actor_function_inner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        output_arguments: Option<&mut Vec<Value>>,
    ) -> DispatchResult<Vec<ActorAction>> {
        let mut actions = Vec::new();
        let mut instance = self.instances.remove(&actor).unwrap_or_default();
        let state_revision = self.state_revision(actor);
        let result = if let Some(output_arguments) = output_arguments {
            self.execute_function_with_outputs(
                actor,
                actor_class,
                function,
                arguments,
                &mut instance,
                &mut actions,
                0,
                Some(output_arguments),
            )
        } else {
            self.execute_function(
                actor,
                actor_class,
                function,
                arguments,
                &mut instance,
                &mut actions,
                0,
            )
        };
        let state_result = if result.is_ok() && self.state_revision(actor) != state_revision {
            self.execute_ready_state(actor, actor_class, &mut instance, &mut actions)
        } else {
            Ok(())
        };
        self.instances.insert(actor, instance);
        result?;
        state_result?;
        Ok(actions)
    }

    pub(super) fn execute_ready_state(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        for _ in 0..MAX_CALL_DEPTH {
            let Some(mut state_frame) = self.state_frames.remove(&actor) else {
                return Ok(());
            };
            match state_frame.latent {
                LatentAction::Continue => {}
                LatentAction::Sleep(remaining) if remaining <= 0.0 => {
                    state_frame.latent = LatentAction::Continue;
                }
                LatentAction::Stop
                | LatentAction::Sleep(_)
                | LatentAction::FinishAnimation(_)
                | LatentAction::FinishInterpolation(_)
                | LatentAction::MoveTo(_)
                | LatentAction::MoveToward(_)
                | LatentAction::TurnTo(_)
                | LatentAction::TurnToward(_) => {
                    self.state_frames.insert(actor, state_frame);
                    return Ok(());
                }
            }

            let state = self.resolved_object(&state_frame.state)?;
            let script = self.script(&state)?;
            let mut frame = Frame::from_snapshot(&script.bytecode, state_frame.frame);
            self.bind_struct_members(&state, &script.bytecode, &mut frame)?;
            self.bind_frame_defaults(actor_class, &state.package, &script.bytecode, &mut frame)?;
            self.bind_frame_zero_values(
                &state.package,
                state.export_index,
                &script.bytecode,
                &mut frame,
            )?;
            let revision = self.state_revision(actor);
            self.state_resumes = self.state_resumes.saturating_add(1);
            self.pending_latent = None;
            let previous_state_actor = self.active_state_actor.replace(actor);
            let run = frame.resume_hosted(|request| {
                let result = match request {
                    FrameRequest::Call {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_context_call(
                            actor,
                            actor_class,
                            receiver,
                            &state.package,
                            call,
                            &arguments,
                            instance,
                            actions,
                            1,
                        )
                        .map(CallOutput::into_response),
                    FrameRequest::CallIterator {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_iterator_call(
                            actor,
                            receiver,
                            &state.package,
                            call,
                            &arguments,
                            instance,
                        )
                        .map(FrameResponse::Iterator),
                    FrameRequest::DynamicCast { class, value } => self
                        .dynamic_cast(actor_class, &state.package, class, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ObjectToString { value } => self
                        .object_to_string(actor, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ResolveObject { reference } => self
                        .object_reference_value(&state.package, reference)
                        .map(FrameResponse::Value),
                    FrameRequest::GetInstance { receiver, field } => self
                        .context_field_value(actor, receiver, &state.package, field, instance)
                        .map(FrameResponse::Value),
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    } => self
                        .set_context_field(
                            actor,
                            receiver,
                            &state.package,
                            field,
                            value,
                            instance,
                            actions,
                        )
                        .map(|()| FrameResponse::Value(Value::None)),
                };
                result
                    .map(|response| {
                        if self.pending_latent.is_some() || self.state_revision(actor) != revision {
                            match response {
                                FrameResponse::Value(value) => FrameResponse::Suspend(value),
                                response => response,
                            }
                        } else {
                            response
                        }
                    })
                    .map_err(|error| error.to_string())
            });
            self.active_state_actor = previous_state_actor;
            let snapshot = frame.into_snapshot();

            if self.state_revision(actor) != revision {
                self.pending_latent = None;
                run?;
                continue;
            }

            state_frame.frame = snapshot;
            state_frame.latent = match run? {
                FrameRun::Complete(_) | FrameRun::Stopped => LatentAction::Stop,
                FrameRun::Suspended => self.pending_latent.take().unwrap_or(LatentAction::Continue),
                FrameRun::GotoLabel(label) => {
                    let label = runtime_name(&state.package, &label)
                        .map_err(|message| DispatchError::UnresolvedObject { message })?;
                    let state_name = self
                        .actor_states
                        .get(&actor)
                        .and_then(|state| state.as_deref())
                        .unwrap_or_default()
                        .to_owned();
                    if let Some((target_state, target)) =
                        self.find_state_label(actor_class, &state_name, &label)?
                    {
                        state_frame.state =
                            object_id(&target_state.package, target_state.export_index);
                        state_frame.frame = FrameSnapshot::at(target);
                        self.state_frames.insert(actor, state_frame);
                        continue;
                    }
                    LatentAction::Stop
                }
            };
            self.pending_latent = None;
            self.state_frames.insert(actor, state_frame);
            return Ok(());
        }
        Err(DispatchError::CallDepth)
    }

    fn state_revision(&self, actor: usize) -> u64 {
        self.state_revisions.get(&actor).copied().unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_function(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
    ) -> DispatchResult<Value> {
        self.execute_function_with_outputs(
            actor,
            actor_class,
            function,
            arguments,
            instance,
            actions,
            depth,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_function_with_outputs(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        function: &ResolvedObject,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
        depth: usize,
        output_arguments: Option<&mut Vec<Value>>,
    ) -> DispatchResult<Value> {
        if depth >= MAX_CALL_DEPTH {
            return Err(DispatchError::CallDepth);
        }
        let script = self.script(function)?;
        if let ScriptMetadata::Function(metadata) = &script.metadata {
            if metadata.native_index != 0 {
                let output = self.dispatch_native_call(
                    actor,
                    actor_class,
                    &function.package,
                    metadata.native_index,
                    arguments,
                    instance,
                    actions,
                    depth,
                )?;
                if let Some(output_arguments) = output_arguments {
                    output_arguments.clear();
                    output_arguments.extend_from_slice(arguments);
                    for (index, value) in &output.outputs {
                        if let Some(argument) = output_arguments.get_mut(*index) {
                            *argument = value.clone();
                        }
                    }
                }
                return Ok(output.value);
            }
            if metadata.flags & FUNCTION_NATIVE != 0 {
                let summary = function.package.summary();
                let export = &summary.exports[function.export_index];
                let class = summary.object_name(export.outer).unwrap_or("<unknown>");
                let function_name = summary.name(export.object_name);
                if class.eq_ignore_ascii_case("Object")
                    && function_name.eq_ignore_ascii_case("Localize")
                {
                    let [
                        Value::String(section),
                        Value::String(key),
                        Value::String(package),
                    ] = arguments
                    else {
                        return Err(DispatchError::UnimplementedNamedNative {
                            class: class.to_owned(),
                            function: function_name.to_owned(),
                        });
                    };
                    return Ok(Value::String(self.packages.localize(package, section, key)));
                }
                if class.eq_ignore_ascii_case("Object")
                    && function_name.eq_ignore_ascii_case("GetLanguage")
                    && arguments.is_empty()
                {
                    return Ok(Value::String(
                        self.packages
                            .config_value("Engine.Engine", "Language")
                            .unwrap_or_else(|| "int".to_owned()),
                    ));
                }
                if class.eq_ignore_ascii_case("Object")
                    && function_name.eq_ignore_ascii_case("DynamicLoadObject")
                {
                    return self.dynamic_load_object(arguments);
                }
                if class.eq_ignore_ascii_case("Actor")
                    && function_name.eq_ignore_ascii_case("GetSoundDuration")
                {
                    return self.sound_duration(arguments).map(Value::Float);
                }
                if class.eq_ignore_ascii_case("Actor")
                    && function_name.eq_ignore_ascii_case("PlayOwnedSound")
                {
                    self.play_sound(
                        actor,
                        actor_class,
                        instance,
                        "PlayOwnedSound",
                        arguments,
                        actions,
                    )
                    .map_err(|message| DispatchError::UnresolvedObject { message })?;
                    return Ok(Value::None);
                }
                if let Some(value) = named_native(class, function_name, arguments) {
                    return Ok(value);
                }
                return Err(DispatchError::UnimplementedNamedNative {
                    class: class.to_owned(),
                    function: function_name.to_owned(),
                });
            }
        }

        let mut frame = Frame::new(&script.bytecode);
        self.bind_struct_members(function, &script.bytecode, &mut frame)?;
        self.bind_frame_defaults(actor_class, &function.package, &script.bytecode, &mut frame)?;
        let argument_bindings =
            self.bind_frame_arguments(&function.package, &script, arguments, &mut frame)?;
        let result = frame
            .execute_hosted(|request| {
                let result = match request {
                    FrameRequest::Call {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_context_call(
                            actor,
                            actor_class,
                            receiver,
                            &function.package,
                            call,
                            &arguments,
                            instance,
                            actions,
                            depth + 1,
                        )
                        .map(CallOutput::into_response),
                    FrameRequest::CallIterator {
                        receiver,
                        function: call,
                        arguments,
                    } => self
                        .dispatch_iterator_call(
                            actor,
                            receiver,
                            &function.package,
                            call,
                            &arguments,
                            instance,
                        )
                        .map(FrameResponse::Iterator),
                    FrameRequest::DynamicCast { class, value } => self
                        .dynamic_cast(actor_class, &function.package, class, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ObjectToString { value } => self
                        .object_to_string(actor, value)
                        .map(FrameResponse::Value),
                    FrameRequest::ResolveObject { reference } => self
                        .object_reference_value(&function.package, reference)
                        .map(FrameResponse::Value),
                    FrameRequest::GetInstance { receiver, field } => self
                        .context_field_value(actor, receiver, &function.package, field, instance)
                        .map(FrameResponse::Value),
                    FrameRequest::SetInstance {
                        receiver,
                        field,
                        value,
                    } => self
                        .set_context_field(
                            actor,
                            receiver,
                            &function.package,
                            field,
                            value,
                            instance,
                            actions,
                        )
                        .map(|()| FrameResponse::Value(Value::None)),
                };
                result.map_err(|error| error.to_string())
            })
            .map_err(DispatchError::from);
        if let Some(output_arguments) = output_arguments {
            copy_output_arguments(arguments, &argument_bindings, &frame, output_arguments);
        }
        result
    }

    pub(super) fn script(&mut self, object: &ResolvedObject) -> DispatchResult<Arc<ScriptExport>> {
        let id = object_id(&object.package, object.export_index);
        if let Some(script) = self.scripts.get(&id) {
            return Ok(Arc::clone(script));
        }
        let script = Arc::new(ScriptExport::decode(&object.package, object.export_index)?);
        self.scripts.insert(id, Arc::clone(&script));
        Ok(script)
    }
}

fn is_unsupported_scene_property(name: &str) -> bool {
    [
        "bCorona",
        "bLensFlare",
        "bMeshEnviroMap",
        "bNoSmooth",
        "bUnlit",
        "Fatness",
        "LightEffect",
        "LightHue",
        "LightPeriod",
        "LightPhase",
        "LightRadius",
        "LightSaturation",
        "LightType",
        "LODBias",
        "MultiSkins",
        "Texture",
        "VolumeBrightness",
    ]
    .iter()
    .any(|property| name.eq_ignore_ascii_case(property))
}

pub(super) fn named_native(class: &str, function: &str, arguments: &[Value]) -> Option<Value> {
    if class.eq_ignore_ascii_case("PlayerPawn")
        && function.eq_ignore_ascii_case("ConsoleCommand")
        && matches!(arguments, [Value::String(_)])
    {
        // ponytail: the game only probes optional console values here; add
        // command routing when an in-game console exists.
        return Some(Value::String(String::new()));
    }
    if class.eq_ignore_ascii_case("Decal")
        && function.eq_ignore_ascii_case("DetachDecal")
        && arguments.is_empty()
    {
        // ponytail: decals are not rendered yet, so there is no attachment to remove.
        return Some(Value::None);
    }
    None
}

fn concrete_self_value(value: &Value, self_handle: i32) -> Value {
    match value {
        Value::Object(-1) => Value::Object(self_handle),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| concrete_self_value(value, self_handle))
                .collect(),
        ),
        Value::Struct(values) => Value::Struct(
            values
                .iter()
                .map(|(name, value)| (name.clone(), concrete_self_value(value, self_handle)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn copy_output_arguments(
    arguments: &[Value],
    bindings: &[(i32, usize, bool)],
    frame: &Frame<'_>,
    output_arguments: &mut Vec<Value>,
) {
    output_arguments.clear();
    output_arguments.extend_from_slice(arguments);
    for &(field, argument, output) in bindings {
        if !output {
            continue;
        }
        if let Some(value) = frame.local(field)
            && let Some(output) = output_arguments.get_mut(argument)
        {
            *output = value.clone();
        }
    }
}

fn copy_native_output_arguments(
    index: u16,
    arguments: &[Value],
    output_arguments: &mut Vec<Value>,
) -> std::result::Result<(), String> {
    output_arguments.clear();
    output_arguments.extend_from_slice(arguments);
    if !matches!(index, 0xe5 | 0xe6) {
        return Ok(());
    }
    let [Value::Rotator(rotation), _, _, _] = arguments else {
        return Err(format!(
            "GetAxes expects a rotator and three vectors, found {}",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let mut axes = crate::rotator_axes(*rotation);
    if index == 0xe6 {
        axes = [
            [axes[0][0], axes[1][0], axes[2][0]],
            [axes[0][1], axes[1][1], axes[2][1]],
            [axes[0][2], axes[1][2], axes[2][2]],
        ];
    }
    for (output, axis) in output_arguments[1..].iter_mut().zip(axes) {
        *output = Value::Vector(axis);
    }
    Ok(())
}

fn closest_trace_hit(
    actor: Option<(f32, usize, Vec3)>,
    bsp: Option<(f32, Vec3)>,
) -> Option<(f32, Option<usize>, Vec3)> {
    match (actor, bsp) {
        (Some((fraction, actor, normal)), Some((bsp_fraction, _))) if fraction < bsp_fraction => {
            Some((fraction, Some(actor), normal))
        }
        (_, Some((fraction, normal))) => Some((fraction, None, normal)),
        (Some((fraction, actor, normal)), None) => Some((fraction, Some(actor), normal)),
        (None, None) => None,
    }
}

pub(super) fn local_fields(bytecode: &Bytecode) -> impl Iterator<Item = i32> + '_ {
    fields(bytecode, 0x00)
}

pub(super) fn fields(bytecode: &Bytecode, opcode: u8) -> impl Iterator<Item = i32> + '_ {
    bytecode
        .tokens
        .iter()
        .filter(move |token| token.opcode == opcode)
        .filter_map(|token| {
            bytecode
                .bytes
                .get(token.offset + 1..token.offset + 5)
                .and_then(|bytes| bytes.try_into().ok())
                .map(i32::from_le_bytes)
        })
}

fn wav_duration(data: &[u8]) -> std::result::Result<f32, String> {
    if data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") || data.len() < 12 {
        return Err("sound data is not a RIFF/WAVE stream".to_owned());
    }
    let mut offset = 12;
    let mut bytes_per_second = None;
    let mut data_size = None;
    while offset + 8 <= data.len() {
        let chunk = &data[offset..offset + 4];
        let size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let Some(payload) = data.get(offset..offset.saturating_add(size)) else {
            return Err("WAV chunk exceeds sound data".to_owned());
        };
        if chunk == b"fmt " && payload.len() >= 12 {
            bytes_per_second = Some(u32::from_le_bytes(payload[8..12].try_into().unwrap()) as f32);
        } else if chunk == b"data" {
            data_size = Some(size as f32);
        }
        offset = offset.saturating_add(size).saturating_add(size & 1);
    }
    match (data_size, bytes_per_second) {
        (Some(size), Some(rate)) if rate > 0.0 => Ok(size / rate),
        _ => Err("WAV sound is missing its format or data chunk".to_owned()),
    }
}

fn mpeg_layer_two_duration(data: &[u8]) -> std::result::Result<f32, String> {
    const MPEG1_BITRATES: [u32; 16] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
    ];
    const MPEG2_BITRATES: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];

    let mut offset = 0;
    let mut duration = 0.0;
    let mut frames = 0;
    while offset + 4 <= data.len() {
        let header = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
        let version = (header >> 19) & 0x3;
        let layer = (header >> 17) & 0x3;
        let bitrate_index = ((header >> 12) & 0xf) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;
        if header >> 21 != 0x7ff
            || version == 1
            || layer != 2
            || sample_rate_index == 3
            || bitrate_index == 0
            || bitrate_index == 15
        {
            offset += 1;
            continue;
        }
        let bitrate = if version == 3 {
            MPEG1_BITRATES[bitrate_index]
        } else {
            MPEG2_BITRATES[bitrate_index]
        } * 1_000;
        let divisor = match version {
            3 => 1,
            2 => 2,
            0 => 4,
            _ => unreachable!(),
        };
        let sample_rate = SAMPLE_RATES[sample_rate_index] / divisor;
        let padding = (header >> 9) & 1;
        let frame_size = (144 * bitrate / sample_rate + padding) as usize;
        if frame_size < 4 || offset + frame_size > data.len() {
            break;
        }
        duration += 1_152.0 / sample_rate as f32;
        frames += 1;
        offset += frame_size;
    }
    (frames > 0)
        .then_some(duration)
        .ok_or_else(|| "MP2 sound has no complete Layer II frames".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_is_concrete_for_call_arguments_and_object_comparisons() {
        let value = Value::Array(vec![Value::Object(-1), Value::Object(7)]);
        assert_eq!(
            concrete_self_value(&value, 42),
            Value::Array(vec![Value::Object(42), Value::Object(7)])
        );
        assert_eq!(
            crate::world::native::scalar_native(
                0x77,
                &[
                    Value::Object(42),
                    concrete_self_value(&Value::Object(-1), 42),
                ],
            ),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn identifies_scene_properties_that_runtime_does_not_project() {
        assert!(is_unsupported_scene_property("multiskins"));
        assert!(is_unsupported_scene_property("Texture"));
        assert!(is_unsupported_scene_property("bUnlit"));
        assert!(is_unsupported_scene_property("bMeshEnviroMap"));
        assert!(!is_unsupported_scene_property("DrawScale"));
        assert!(!is_unsupported_scene_property("Style"));
        assert!(!is_unsupported_scene_property("AmbientGlow"));
        assert!(!is_unsupported_scene_property("ScaleGlow"));
        assert!(!is_unsupported_scene_property("Skin"));
        assert!(!is_unsupported_scene_property("SkelAnim"));
        assert!(!is_unsupported_scene_property("Opacity"));
        assert!(!is_unsupported_scene_property("Mesh"));
        assert!(!is_unsupported_scene_property("DrawType"));
        assert!(!is_unsupported_scene_property("Velocity"));
    }

    #[test]
    fn effective_display_assignments_suppress_defaults_and_emit_typed_actions() {
        let object = ObjectId {
            package: Arc::from("Engine.u"),
            export_index: 42,
        };
        let runtime_object = RuntimeObject {
            package: Arc::from("Engine.u"),
            export_index: 42,
        };
        let float_default = StoredValue::Value(Value::Float(1.0));
        assert_eq!(
            object::effective_assignment(
                7,
                "ScaleGlow",
                None,
                Some(&float_default),
                None,
                &float_default,
            ),
            (false, None),
        );
        let object_default = StoredValue::Object(None);
        assert_eq!(
            object::effective_assignment(
                7,
                "Skin",
                None,
                None,
                Some(&object_default),
                &object_default,
            ),
            (false, None),
        );

        let skin = StoredValue::Object(Some(object.clone()));
        assert_eq!(
            object::effective_assignment(
                7,
                "ScaleGlow",
                None,
                Some(&float_default),
                None,
                &StoredValue::Value(Value::Float(2.0)),
            ),
            (
                true,
                Some(ActorAction::SetScaleGlow {
                    actor: 7,
                    scale_glow: 2.0,
                }),
            ),
        );
        assert_eq!(
            object::effective_assignment(7, "Skin", None, None, Some(&object_default), &skin,),
            (
                true,
                Some(ActorAction::SetSkin {
                    actor: 7,
                    skin: Some(runtime_object.clone()),
                }),
            ),
        );
        assert_eq!(
            object::effective_assignment(7, "SkelAnim", Some(&object_default), None, None, &skin,),
            (
                true,
                Some(ActorAction::SetSkelAnim {
                    actor: 7,
                    skel_anim: Some(runtime_object),
                }),
            ),
        );
        assert_eq!(
            object::effective_assignment(
                7,
                "Opacity",
                None,
                Some(&float_default),
                None,
                &StoredValue::Value(Value::Float(0.5)),
            ),
            (
                true,
                Some(ActorAction::SetOpacity {
                    actor: 7,
                    opacity: 0.5,
                }),
            ),
        );
    }

    #[test]
    fn returns_mutated_event_arguments_to_the_engine_host() {
        let bytecode = Bytecode {
            version: 76,
            raw_len: 1,
            bytes: vec![0x08],
            tokens: Vec::new(),
        };
        let mut frame = Frame::new(&bytecode);
        frame.set_local(10, Value::Vector([4.0, 5.0, 6.0]));
        frame.set_local(11, Value::Rotator([7, 8, 9]));
        let mut output = Vec::new();
        copy_output_arguments(
            &[
                Value::Object(1),
                Value::Vector([1.0, 2.0, 3.0]),
                Value::Rotator([1, 2, 3]),
            ],
            &[(10, 1, true), (11, 2, true)],
            &frame,
            &mut output,
        );
        assert_eq!(
            output,
            [
                Value::Object(1),
                Value::Vector([4.0, 5.0, 6.0]),
                Value::Rotator([7, 8, 9]),
            ]
        );
    }

    #[test]
    fn effective_light_brightness_assignments_emit_typed_actions() {
        assert!(!is_unsupported_scene_property("LightBrightness"));
        assert_eq!(
            object::effective_assignment(
                7,
                "LightBrightness",
                Some(&StoredValue::Value(Value::Byte(0))),
                None,
                None,
                &StoredValue::Value(Value::Byte(64)),
            ),
            (
                true,
                Some(ActorAction::SetLightBrightness {
                    actor: 7,
                    light_brightness: 64,
                }),
            ),
        );
    }

    #[test]
    fn trace_returns_the_closest_actor_or_bsp_hit() {
        let normal = Vec3::Z;
        assert_eq!(
            closest_trace_hit(Some((0.25, 7, normal)), Some((0.75, Vec3::X))),
            Some((0.25, Some(7), normal))
        );
        assert_eq!(
            closest_trace_hit(Some((0.75, 7, normal)), Some((0.25, Vec3::X))),
            Some((0.25, None, Vec3::X))
        );
    }

    #[test]
    fn reads_wav_duration_from_byte_rate_and_data_size() {
        let mut wav = b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x40\x1f\0\0\x01\0\x08\0data\x04\0\0\0\0\0\0\0".to_vec();
        assert_eq!(wav_duration(&wav).unwrap(), 4.0 / 8_000.0);
        wav[0] = 0;
        assert!(wav_duration(&wav).is_err());
    }

    #[test]
    fn reads_mp2_duration_from_layer_two_frames() {
        let header = (0x7ff << 21) | (3 << 19) | (2 << 17) | (1 << 16) | (8 << 12);
        let frame_size = 144 * 128_000 / 44_100;
        let mut mp2 = vec![0; frame_size * 2];
        mp2[..4].copy_from_slice(&u32::to_be_bytes(header));
        mp2[frame_size..frame_size + 4].copy_from_slice(&u32::to_be_bytes(header));
        assert!(
            (mpeg_layer_two_duration(&mp2).unwrap() - 2.0 * 1_152.0 / 44_100.0).abs()
                < f32::EPSILON
        );
    }
}
