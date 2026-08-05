use super::*;

impl ScriptRuntime {
    pub fn initialize_player_hud(&mut self) -> DispatchResult<Vec<ActorAction>> {
        let player = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&player)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: player })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&player)
            .ok_or(DispatchError::ActiveActorContext { actor: player })?;
        let mut actions = Vec::new();
        let result = (|| {
            if self
                .actor_object(&class, &instance, "myHUD")
                .map_err(|message| DispatchError::UnresolvedObject { message })?
                .is_some()
            {
                return Ok(());
            }
            let Some(hud_type) = self
                .actor_object(&class, &instance, "HUDType")
                .map_err(|message| DispatchError::UnresolvedObject { message })?
            else {
                return Ok(());
            };

            // UE1's renderer invokes PlayerPawn.PreRender, which lazily spawns
            // HUDType. OpenHP1's local game host calls this after Possess and
            // before the first world tick.
            let hud_type = self.object_handle(hud_type)?;
            let hud = self
                .spawn_actor(
                    player,
                    &class,
                    &class.package,
                    &[Value::Object(hud_type), Value::Object(-1)],
                    &mut instance,
                    &mut actions,
                )
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            let hud = match hud {
                Value::Object(0) | Value::None => None,
                Value::Object(handle) => Some(self.object_for_handle(handle)?),
                value => {
                    return Err(DispatchError::UnresolvedObject {
                        message: format!("HUD spawn returned {}", value.kind()),
                    });
                }
            };
            self.set_actor_stored(&class, &mut instance, "myHUD", StoredValue::Object(hud))
                .map_err(|message| DispatchError::UnresolvedObject { message })
        })();
        self.instances.insert(player, instance);
        result?;
        Ok(actions)
    }

    pub fn set_player_view_target_class(
        &mut self,
        class_name: &str,
    ) -> DispatchResult<Option<usize>> {
        let player = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        let mut target = None;
        for actor in actors {
            if actor == player || self.destroyed.contains(&actor) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?;
            let class = self.resolved_object(&class)?;
            if self.class_has_name(&class, class_name)? {
                target = Some(actor);
                break;
            }
        }
        let Some(target) = target else {
            return Ok(None);
        };
        let target_object = self
            .actor_objects
            .get(&target)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: target })?;
        self.set_player_view_target(Some(target_object))?;
        Ok(Some(target))
    }

    pub fn clear_player_view_target(&mut self) -> DispatchResult<()> {
        self.set_player_view_target(None)
    }

    pub fn player_state_name(&self) -> Option<&str> {
        self.player_actor
            .and_then(|actor| self.actor_states.get(&actor))
            .and_then(|state| state.as_deref())
    }

    pub fn take_player_music(&mut self) -> DispatchResult<Option<PlayerMusic>> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = (|| {
            let transition = self
                .actor_byte(&class, &instance, "Transition")
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            if transition == 0 {
                return Ok(None);
            }
            let song = self
                .actor_object(&class, &instance, "Song")
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            let section = self
                .actor_byte(&class, &instance, "SongSection")
                .map_err(|message| DispatchError::UnresolvedObject { message })?;
            self.set_actor_stored(
                &class,
                &mut instance,
                "Transition",
                StoredValue::Value(Value::Byte(0)),
            )
            .map_err(|message| DispatchError::UnresolvedObject { message })?;
            let clip = song
                .map(|song| {
                    let song = self.resolved_object(&song)?;
                    AudioClip::decode(&song.package, song.export_index).map_err(|error| {
                        DispatchError::UnresolvedObject {
                            message: error.to_string(),
                        }
                    })
                })
                .transpose()?;
            Ok(Some(PlayerMusic {
                clip,
                section,
                transition,
            }))
        })();
        self.instances.insert(actor, instance);
        result
    }

    fn set_player_view_target(&mut self, target: Option<ObjectId>) -> DispatchResult<()> {
        let player = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&player)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: player })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&player)
            .ok_or(DispatchError::ActiveActorContext { actor: player })?;
        let result = self
            .set_actor_stored(
                &class,
                &mut instance,
                "ViewTarget",
                StoredValue::Object(target),
            )
            .map_err(|message| DispatchError::InvalidPlayerView { message });
        self.instances.insert(player, instance);
        result
    }

    pub fn set_player_input(&mut self, input: PlayerInput) -> DispatchResult<()> {
        if ![
            input.base_x,
            input.base_y,
            input.strafe,
            input.mouse_x,
            input.mouse_y,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(DispatchError::InvalidPlayerInput {
                message: "input axes must be finite".to_owned(),
            });
        }
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = (|| {
            for (name, value) in [
                ("aBaseX", Value::Float(input.base_x)),
                ("aBaseY", Value::Float(input.base_y)),
                ("aStrafe", Value::Float(input.strafe)),
                ("aMouseX", Value::Float(input.mouse_x)),
                ("aMouseY", Value::Float(input.mouse_y)),
                ("aBroomYaw", Value::Float(input.mouse_x)),
                ("aBroomPitch", Value::Float(-input.mouse_y)),
                ("bAltFire", Value::Byte(u8::from(input.alt_fire))),
                ("bBroomYawLeft", Value::Byte(u8::from(input.base_x < 0.0))),
                ("bBroomYawRight", Value::Byte(u8::from(input.base_x > 0.0))),
                ("bBroomPitchUp", Value::Byte(u8::from(input.broom_pitch_up))),
                (
                    "bBroomPitchDown",
                    Value::Byte(u8::from(input.broom_pitch_down)),
                ),
                ("bBroomBoost", Value::Byte(u8::from(input.broom_boost))),
                ("bBroomBrake", Value::Byte(u8::from(input.broom_brake))),
                ("bBroomAction", Value::Byte(u8::from(input.jump))),
                ("bPressedJump", Value::Bool(input.jump)),
            ] {
                self.set_actor_value(&class, &mut instance, name, value)
                    .map_err(|message| DispatchError::InvalidPlayerInput { message })?;
            }
            Ok(())
        })();
        self.instances.insert(actor, instance);
        if result.is_ok() {
            self.player_alt_fire_pressed |= input.alt_fire_pressed;
            if input.space_pressed {
                self.player_space_pressed = true;
            }
            if input.space_released {
                self.player_space_pressed = false;
            }
        }
        result
    }

    pub(super) fn player_is_carrying_actor(&mut self) -> DispatchResult<bool> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        Ok(matches!(
            self.instance_property(&class, &instance, "CarryingActor")?,
            Some(StoredValue::Object(Some(_)))
        ))
    }

    pub fn dispatch_player_event(
        &mut self,
        event: &str,
        arguments: &[Value],
    ) -> DispatchResult<Vec<ActorAction>> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        self.dispatch_event_with_arguments(
            actor,
            Path::new(class.package.as_ref()),
            class.export_index,
            event,
            arguments,
        )
    }

    pub fn tick_player(
        &mut self,
        input: PlayerInput,
        delta_time: f32,
    ) -> DispatchResult<Vec<ActorAction>> {
        self.set_player_input(input)?;
        let mut actions = Vec::new();
        self.tick_player_events(delta_time, &mut actions)?;
        Ok(actions)
    }

    pub fn player_view(
        &mut self,
        location: [f32; 3],
        rotation: [i32; 3],
    ) -> DispatchResult<(PlayerView, Vec<ActorAction>)> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class_id = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class_id)?;
        let handle = self.object_handle(
            self.actor_objects
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor })?,
        )?;
        let arguments = [
            Value::Object(handle),
            Value::Vector(location),
            Value::Rotator(rotation),
        ];
        let mut output_arguments = arguments.to_vec();
        let actions =
            if let Some(function) = self.find_actor_function(actor, class, "PlayerCalcView", 0)? {
                let actor_class = self.resolved_object(&class_id)?;
                self.execute_actor_function_with_outputs(
                    actor,
                    &actor_class,
                    &function,
                    &arguments,
                    &mut output_arguments,
                )?
            } else {
                Vec::new()
            };
        let [
            Value::Object(view_handle),
            Value::Vector(location),
            Value::Rotator(rotation),
        ] = output_arguments.as_slice()
        else {
            return Err(DispatchError::InvalidPlayerView {
                message: format!("PlayerCalcView returned {output_arguments:?}"),
            });
        };
        let view_actor = if *view_handle == 0 {
            actor
        } else {
            self.actor_for_handle(*view_handle)?
        };
        let fov_degrees = self
            .actor_float_property(actor, "FovAngle")?
            .unwrap_or(90.0);
        if !fov_degrees.is_finite() || !(1.0..179.0).contains(&fov_degrees) {
            return Err(DispatchError::InvalidPlayerView {
                message: format!("FovAngle is {fov_degrees}"),
            });
        }
        Ok((
            PlayerView {
                actor: view_actor,
                location: *location,
                rotation: *rotation,
                fov_degrees,
            },
            actions,
        ))
    }

    pub(super) fn actor_float_property(
        &mut self,
        actor: usize,
        name: &str,
    ) -> DispatchResult<Option<f32>> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        Ok(match self.instance_property(&class, &instance, name)? {
            Some(StoredValue::Value(Value::Float(value))) => Some(value),
            _ => None,
        })
    }

    pub(super) fn clear_player_motion_input(&mut self) -> DispatchResult<()> {
        let actor = self.player_actor.ok_or(DispatchError::MissingPlayer)?;
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = (|| {
            for name in ["aForward", "aTurn", "aLookUp"] {
                self.set_actor_value(&class, &mut instance, name, Value::Float(0.0))
                    .map_err(|message| DispatchError::InvalidPlayerInput { message })?;
            }
            Ok(())
        })();
        self.instances.insert(actor, instance);
        result
    }
}
