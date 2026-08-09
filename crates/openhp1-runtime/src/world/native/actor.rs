use super::*;

impl ScriptRuntime {
    pub(in crate::world) fn make_noise(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        loudness: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        const NM_CLIENT: u8 = 3;

        let Some(noise_object) = self.actor_object(actor_class, instance, "Instigator")? else {
            return Ok(());
        };
        let noise_actor = self
            .object_actors
            .get(&noise_object)
            .copied()
            .ok_or_else(|| "MakeNoise Instigator is not a registered actor".to_owned())?;
        let noise_class = self
            .actor_classes
            .get(&noise_actor)
            .cloned()
            .ok_or_else(|| format!("MakeNoise Instigator {noise_actor} has no class"))?;
        let noise_class = self
            .resolved_object(&noise_class)
            .map_err(|error| error.to_string())?;
        if !self
            .class_has_name(&noise_class, "Pawn")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }

        let level = self
            .actor_object(actor_class, instance, "Level")?
            .ok_or_else(|| "MakeNoise actor has no Level".to_owned())?;
        let level_actor = self
            .object_actors
            .get(&level)
            .copied()
            .ok_or_else(|| "MakeNoise Level is not a registered actor".to_owned())?;
        let level_class = self
            .actor_classes
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("MakeNoise Level actor {level_actor} has no class"))?;
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let level_instance = self
            .instances
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("MakeNoise Level actor {level_actor} instance is active"))?;
        match self
            .instance_property(&level_class, &level_instance, "NetMode")
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Byte(NM_CLIENT))) => return Ok(()),
            Some(StoredValue::Value(Value::Byte(_))) | None => {}
            Some(value) => return Err(format!("MakeNoise Level.NetMode is {value:?}")),
        }

        let time = self.actor_float_any(&level_class, &level_instance, "TimeSeconds")?;
        let location = Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
        let recorded = if noise_actor == actor {
            self.record_noise(&noise_class, instance, time, location, loudness)?
        } else {
            let mut noise_instance = self
                .instances
                .remove(&noise_actor)
                .ok_or_else(|| format!("MakeNoise Instigator {noise_actor} instance is active"))?;
            let result =
                self.record_noise(&noise_class, &mut noise_instance, time, location, loudness);
            self.instances.insert(noise_actor, noise_instance);
            result?
        };
        if !recorded {
            return Ok(());
        }

        let source = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("MakeNoise actor {actor} has no object identity"))?;
        let source_handle = self
            .object_handle(source)
            .map_err(|error| error.to_string())?;
        let noise_instance = if noise_actor == actor {
            instance.clone()
        } else {
            self.instances
                .get(&noise_actor)
                .cloned()
                .ok_or_else(|| format!("MakeNoise Instigator {noise_actor} instance is active"))?
        };
        let mut pawn = self.actor_object(&level_class, &level_instance, "PawnList")?;
        let mut seen = HashSet::default();
        while let Some(pawn_object) = pawn {
            let pawn_actor = self
                .object_actors
                .get(&pawn_object)
                .copied()
                .ok_or_else(|| "MakeNoise PawnList has an unregistered pawn".to_owned())?;
            if !seen.insert(pawn_actor) {
                return Err("MakeNoise PawnList has a cycle".to_owned());
            }
            let pawn_class = self
                .actor_classes
                .get(&pawn_actor)
                .cloned()
                .ok_or_else(|| format!("MakeNoise pawn {pawn_actor} has no class"))?;
            let pawn_class = self
                .resolved_object(&pawn_class)
                .map_err(|error| error.to_string())?;
            let mut pawn_instance = if pawn_actor == actor {
                instance.clone()
            } else {
                self.instances
                    .get(&pawn_actor)
                    .cloned()
                    .ok_or_else(|| format!("MakeNoise pawn {pawn_actor} instance is active"))?
            };

            let should_hear = pawn_actor != noise_actor
                && !self.destroyed.contains(&pawn_actor)
                && self.can_hear_noise(
                    actor_class,
                    instance,
                    &noise_class,
                    &noise_instance,
                    &pawn_class,
                    &mut pawn_instance,
                    loudness,
                )?;
            if pawn_actor == actor {
                *instance = pawn_instance;
            } else {
                self.instances.insert(pawn_actor, pawn_instance);
            }
            if pawn_actor != noise_actor && !self.destroyed.contains(&pawn_actor) && should_hear {
                let arguments = vec![Value::Float(loudness), Value::Object(source_handle)];
                if pawn_actor == actor {
                    self.call_actor_event(
                        actor,
                        actor_class,
                        instance,
                        "HearNoise",
                        arguments,
                        actions,
                    )?;
                } else {
                    self.call_other_actor_event(pawn_actor, "HearNoise", arguments, actions)?;
                }
            }

            pawn = if pawn_actor == actor {
                self.actor_object(actor_class, instance, "nextPawn")?
            } else {
                let pawn_instance = self
                    .instances
                    .get(&pawn_actor)
                    .cloned()
                    .ok_or_else(|| format!("MakeNoise pawn {pawn_actor} instance is active"))?;
                self.actor_object(&pawn_class, &pawn_instance, "nextPawn")?
            };
        }
        Ok(())
    }

    fn record_noise(
        &mut self,
        pawn_class: &ResolvedObject,
        pawn: &mut InstanceState,
        time: f32,
        location: Vec3,
        loudness: f32,
    ) -> std::result::Result<bool, String> {
        let noise1_time = self.actor_float_any(pawn_class, pawn, "noise1time")?;
        let noise1_spot = Vec3::from_array(self.actor_vector(pawn_class, pawn, "noise1spot")?);
        let noise1_loudness = self.actor_float_any(pawn_class, pawn, "noise1loudness")?;
        let noise2_time = self.actor_float_any(pawn_class, pawn, "noise2time")?;
        let noise2_spot = Vec3::from_array(self.actor_vector(pawn_class, pawn, "noise2spot")?);
        let noise2_loudness = self.actor_float_any(pawn_class, pawn, "noise2loudness")?;

        if (noise1_time > time - 0.2
            && noise1_spot.distance_squared(location) < 2_500.0
            && noise1_loudness >= 0.9 * loudness)
            || (noise2_time > time - 0.2
                && noise2_spot.distance_squared(location) < 2_500.0
                && noise2_loudness >= 0.9 * loudness)
        {
            return Ok(false);
        }
        if noise1_time < time - 0.18 {
            self.set_noise_slot(pawn_class, pawn, "noise1", time, location, loudness)?;
        } else if noise2_time < time - 0.18 {
            self.set_noise_slot(pawn_class, pawn, "noise2", time, location, loudness)?;
        } else if noise1_spot.distance_squared(location) < 2_500.0 {
            self.set_noise_slot(pawn_class, pawn, "noise1", time, location, loudness)?;
        } else if noise2_loudness <= loudness {
            self.set_noise_slot(pawn_class, pawn, "noise2", time, location, loudness)?;
        }
        Ok(true)
    }

    fn set_noise_slot(
        &mut self,
        pawn_class: &ResolvedObject,
        pawn: &mut InstanceState,
        slot: &str,
        time: f32,
        location: Vec3,
        loudness: f32,
    ) -> std::result::Result<(), String> {
        self.set_actor_value(pawn_class, pawn, &format!("{slot}time"), Value::Float(time))?;
        self.set_actor_value(
            pawn_class,
            pawn,
            &format!("{slot}spot"),
            Value::Vector(location.to_array()),
        )?;
        self.set_actor_value(
            pawn_class,
            pawn,
            &format!("{slot}loudness"),
            Value::Float(loudness),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn can_hear_noise(
        &mut self,
        source_class: &ResolvedObject,
        source: &InstanceState,
        noise_class: &ResolvedObject,
        noise: &InstanceState,
        listener_class: &ResolvedObject,
        listener: &mut InstanceState,
        loudness: f32,
    ) -> std::result::Result<bool, String> {
        let noise_is_player = self.actor_bool(noise_class, noise, "bIsPlayer")?;
        let noise_enemy_is_player =
            self.actor_object(noise_class, noise, "Enemy")?
                .map(|enemy| {
                    let actor =
                        self.object_actors.get(&enemy).copied().ok_or_else(|| {
                            "CanHearNoise Enemy is not a registered actor".to_owned()
                        })?;
                    let class = self
                        .actor_classes
                        .get(&actor)
                        .cloned()
                        .ok_or_else(|| format!("CanHearNoise Enemy {actor} has no class"))?;
                    let class = self
                        .resolved_object(&class)
                        .map_err(|error| error.to_string())?;
                    let instance =
                        self.instances.get(&actor).cloned().ok_or_else(|| {
                            format!("CanHearNoise Enemy {actor} instance is active")
                        })?;
                    self.actor_bool(&class, &instance, "bIsPlayer")
                })
                .transpose()?
                .unwrap_or(false);
        if !noise_is_player && !noise_enemy_is_player {
            let source_name = source_class.package.summary().name(
                source_class.package.summary().exports[source_class.export_index].object_name,
            );
            let listener_name = listener_class.package.summary().name(
                listener_class.package.summary().exports[listener_class.export_index].object_name,
            );
            if !self
                .class_has_name(listener_class, source_name)
                .map_err(|error| error.to_string())?
                && !self
                    .class_has_name(source_class, listener_name)
                    .map_err(|error| error.to_string())?
            {
                return Ok(false);
            }
        } else if self
            .class_has_name(listener_class, "PlayerPawn")
            .map_err(|error| error.to_string())?
        {
            return Ok(false);
        }

        let source_location =
            Vec3::from_array(self.actor_vector(source_class, source, "Location")?);
        let listener_location =
            Vec3::from_array(self.actor_vector(listener_class, listener, "Location")?);
        let distance_squared = listener_location.distance_squared(source_location);
        let listener_is_player = self.actor_bool(listener_class, listener, "bIsPlayer")?;
        let team_mate = listener_is_player
            && noise_is_player
            && self.same_team(listener_class, listener, noise_class, noise)?;
        if distance_squared > (4_000.0 * loudness).powi(2) {
            return Ok(false);
        }
        if !team_mate {
            let perceived = (1_200_000.0 / distance_squared).min(2.0);
            let alertness = self.actor_float_any(listener_class, listener, "Alertness")?;
            let stimulus = loudness * perceived + alertness * perceived.min(0.5);
            self.set_actor_value(listener_class, listener, "Stimulus", Value::Float(stimulus))?;
            if stimulus < self.actor_float_any(listener_class, listener, "HearingThreshold")? {
                return Ok(false);
            }
        }
        Ok(self.has_line_of_sight(source_location, listener_location))
    }

    fn same_team(
        &mut self,
        listener_class: &ResolvedObject,
        listener: &InstanceState,
        noise_class: &ResolvedObject,
        noise: &InstanceState,
    ) -> std::result::Result<bool, String> {
        let Some(level) = self.actor_object(listener_class, listener, "Level")? else {
            return Ok(false);
        };
        let Some(level_actor) = self.object_actors.get(&level).copied() else {
            return Ok(false);
        };
        let Some(level_class) = self.actor_classes.get(&level_actor).cloned() else {
            return Ok(false);
        };
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let Some(level_instance) = self.instances.get(&level_actor).cloned() else {
            return Ok(false);
        };
        let Some(game) = self.actor_object(&level_class, &level_instance, "Game")? else {
            return Ok(false);
        };
        let Some(game_actor) = self.object_actors.get(&game).copied() else {
            return Ok(false);
        };
        let Some(game_class) = self.actor_classes.get(&game_actor).cloned() else {
            return Ok(false);
        };
        let game_class = self
            .resolved_object(&game_class)
            .map_err(|error| error.to_string())?;
        let Some(game_instance) = self.instances.get(&game_actor).cloned() else {
            return Ok(false);
        };
        if !matches!(
            self.instance_property(&game_class, &game_instance, "bTeamGame")
                .map_err(|error| error.to_string())?,
            Some(StoredValue::Value(Value::Bool(true)))
        ) {
            return Ok(false);
        }
        let Some(listener_pri) =
            self.actor_object(listener_class, listener, "PlayerReplicationInfo")?
        else {
            return Ok(false);
        };
        let Some(noise_pri) = self.actor_object(noise_class, noise, "PlayerReplicationInfo")?
        else {
            return Ok(false);
        };
        let Some(listener_actor) = self.object_actors.get(&listener_pri).copied() else {
            return Ok(false);
        };
        let Some(noise_actor) = self.object_actors.get(&noise_pri).copied() else {
            return Ok(false);
        };
        let listener_class = self
            .actor_classes
            .get(&listener_actor)
            .cloned()
            .ok_or_else(|| format!("CanHearNoise PRI {listener_actor} has no class"))?;
        let listener_class = self
            .resolved_object(&listener_class)
            .map_err(|error| error.to_string())?;
        let noise_class = self
            .actor_classes
            .get(&noise_actor)
            .cloned()
            .ok_or_else(|| format!("CanHearNoise PRI {noise_actor} has no class"))?;
        let noise_class = self
            .resolved_object(&noise_class)
            .map_err(|error| error.to_string())?;
        let listener_instance = self
            .instances
            .get(&listener_actor)
            .cloned()
            .ok_or_else(|| format!("CanHearNoise PRI {listener_actor} instance is active"))?;
        let noise_instance = self
            .instances
            .get(&noise_actor)
            .cloned()
            .ok_or_else(|| format!("CanHearNoise PRI {noise_actor} instance is active"))?;
        Ok(
            self.actor_byte(&listener_class, &listener_instance, "Team")?
                == self.actor_byte(&noise_class, &noise_instance, "Team")?,
        )
    }

    pub(in crate::world) fn add_pawn(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<(), String> {
        let level = self
            .actor_object(actor_class, instance, "Level")?
            .ok_or_else(|| "AddPawn actor has no Level".to_owned())?;
        let level_actor = self
            .object_actors
            .get(&level)
            .copied()
            .ok_or_else(|| "AddPawn Level is not a registered actor".to_owned())?;
        let level_class = self
            .actor_classes
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("AddPawn Level actor {level_actor} has no class"))?;
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let mut level_instance = self
            .instances
            .remove(&level_actor)
            .ok_or_else(|| format!("AddPawn Level actor {level_actor} instance is active"))?;
        let result: std::result::Result<(), String> = (|| {
            let previous =
                match self.required_actor_property(&level_class, &level_instance, "PawnList")? {
                    StoredValue::Object(value) => value,
                    value => return Err(format!("AddPawn Level.PawnList is {value:?}")),
                };
            self.set_actor_stored(
                actor_class,
                instance,
                "nextPawn",
                StoredValue::Object(previous),
            )?;
            let object = self
                .actor_objects
                .get(&actor)
                .cloned()
                .ok_or_else(|| format!("AddPawn actor {actor} has no object identity"))?;
            self.set_actor_stored(
                &level_class,
                &mut level_instance,
                "PawnList",
                StoredValue::Object(Some(object)),
            )
        })();
        self.instances.insert(level_actor, level_instance);
        result
    }

    pub(in crate::world) fn remove_pawn(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<(), String> {
        let level = self
            .actor_object(actor_class, instance, "Level")?
            .ok_or_else(|| "RemovePawn actor has no Level".to_owned())?;
        let level_actor = self
            .object_actors
            .get(&level)
            .copied()
            .ok_or_else(|| "RemovePawn Level is not a registered actor".to_owned())?;
        let level_class = self
            .actor_classes
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("RemovePawn Level actor {level_actor} has no class"))?;
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("RemovePawn actor {actor} has no object identity"))?;
        let next = self.actor_object(actor_class, instance, "nextPawn")?;
        let mut level_instance = self
            .instances
            .remove(&level_actor)
            .ok_or_else(|| format!("RemovePawn Level actor {level_actor} instance is active"))?;
        let result: std::result::Result<(), String> = (|| {
            let head = self.actor_object(&level_class, &level_instance, "PawnList")?;
            if head.as_ref() == Some(&object) {
                self.set_actor_stored(
                    &level_class,
                    &mut level_instance,
                    "PawnList",
                    StoredValue::Object(next.clone()),
                )?;
            } else {
                let mut pawns = self.actor_classes.keys().copied().collect::<Vec<_>>();
                pawns.sort_unstable();
                for pawn in pawns {
                    if pawn == actor || pawn == level_actor {
                        continue;
                    }
                    let pawn_class = self
                        .actor_classes
                        .get(&pawn)
                        .cloned()
                        .ok_or_else(|| format!("RemovePawn actor {pawn} has no class"))?;
                    let pawn_class = self
                        .resolved_object(&pawn_class)
                        .map_err(|error| error.to_string())?;
                    if !self
                        .class_has_name(&pawn_class, "Pawn")
                        .map_err(|error| error.to_string())?
                    {
                        continue;
                    }
                    let Some(mut pawn_instance) = self.instances.remove(&pawn) else {
                        continue;
                    };
                    let linked = self.actor_object(&pawn_class, &pawn_instance, "nextPawn");
                    if linked
                        .as_ref()
                        .is_ok_and(|linked| linked.as_ref() == Some(&object))
                    {
                        let update = self.set_actor_stored(
                            &pawn_class,
                            &mut pawn_instance,
                            "nextPawn",
                            StoredValue::Object(next.clone()),
                        );
                        self.instances.insert(pawn, pawn_instance);
                        update?;
                        break;
                    }
                    self.instances.insert(pawn, pawn_instance);
                    linked?;
                }
            }
            Ok(())
        })();
        self.instances.insert(level_actor, level_instance);
        result?;
        self.set_actor_stored(actor_class, instance, "nextPawn", StoredValue::Object(None))
    }

    pub(in crate::world) fn pick_target(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &InstanceState,
        arguments: &[Value],
        pawns_only: bool,
    ) -> std::result::Result<(Value, f32, f32), String> {
        let native = if pawns_only {
            "PickTarget"
        } else {
            "PickAnyTarget"
        };
        let [
            Value::Float(best_aim),
            Value::Float(best_dist),
            Value::Vector(fire_direction),
            Value::Vector(projectile_start),
        ] = arguments
        else {
            return Err(format!(
                "{native} expects best aim, best distance, fire direction, and projectile start, found {}",
                arguments
                    .iter()
                    .map(Value::kind)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let mut best_aim = *best_aim;
        let mut best_dist = *best_dist;
        let fire_direction = Vec3::from_array(*fire_direction);
        let projectile_start = Vec3::from_array(*projectile_start);
        if !best_aim.is_finite()
            || !best_dist.is_finite()
            || !fire_direction.is_finite()
            || !projectile_start.is_finite()
        {
            return Err(format!("{native} arguments are not finite"));
        }

        let mut best = None;
        let mut candidates = self.actor_classes.keys().copied().collect::<Vec<_>>();
        candidates.sort_unstable();
        for candidate in candidates {
            if candidate == actor || self.destroyed.contains(&candidate) {
                continue;
            }
            let class = self
                .actor_classes
                .get(&candidate)
                .cloned()
                .ok_or_else(|| format!("{native} actor {candidate} has no class"))?;
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            let is_pawn = self
                .class_has_name(&class, "Pawn")
                .map_err(|error| error.to_string())?;
            if is_pawn != pawns_only {
                continue;
            }
            let candidate_instance = self
                .instances
                .get(&candidate)
                .cloned()
                .ok_or_else(|| format!("{native} actor {candidate} has no instance"))?;
            if pawns_only {
                let health =
                    match self.required_actor_property(&class, &candidate_instance, "Health")? {
                        StoredValue::Value(Value::Int(health)) => health,
                        value => return Err(format!("{native} Health is {value:?}")),
                    };
                if health <= 0 {
                    continue;
                }
            } else if !self.actor_bool(&class, &candidate_instance, "bProjTarget")? {
                continue;
            }
            let location =
                Vec3::from_array(self.actor_vector(&class, &candidate_instance, "Location")?);
            let Some((aim, distance)) =
                target_score(projectile_start, fire_direction, location, best_aim)
            else {
                continue;
            };
            if !self.line_of_sight_to(actor, actor_class, instance, candidate)? {
                continue;
            }
            best_aim = aim;
            best_dist = distance;
            best = Some(candidate);
        }

        let value = match best {
            Some(candidate) => {
                let object = self
                    .actor_objects
                    .get(&candidate)
                    .cloned()
                    .ok_or_else(|| format!("{native} actor {candidate} has no object"))?;
                Value::Object(
                    self.object_handle(object)
                        .map_err(|error| error.to_string())?,
                )
            }
            None => Value::Object(0),
        };
        Ok((value, best_aim, best_dist))
    }

    pub(in crate::world) fn set_actor_owner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        owner: Option<ObjectId>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let current = self.actor_object(actor_class, instance, "Owner")?;
        if current == owner {
            return Ok(());
        }
        let actor_object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
        let actor_handle = self
            .object_handle(actor_object)
            .map_err(|error| error.to_string())?;
        if let Some(old_owner) = current
            .as_ref()
            .and_then(|owner| self.object_actors.get(owner))
            .copied()
        {
            self.call_other_actor_event(
                old_owner,
                "LostChild",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }
        self.set_actor_stored(
            actor_class,
            instance,
            "Owner",
            StoredValue::Object(owner.clone()),
        )?;
        if let Some(new_owner) = owner
            .as_ref()
            .and_then(|owner| self.object_actors.get(owner))
            .copied()
        {
            self.call_other_actor_event(
                new_owner,
                "GainedChild",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }
        Ok(())
    }

    pub(in crate::world) fn destroy_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        for name in ["bStatic", "bNoDelete"] {
            let field = self
                .find_property(actor_class, name, 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Destroy property {name} is missing"))?;
            match instance.get(&field) {
                Some(StoredValue::Value(Value::Bool(true))) => return Ok(false),
                Some(StoredValue::Value(Value::Bool(false))) | None => {}
                Some(value) => return Err(format!("Destroy property {name} is {value:?}")),
            }
        }
        if self.destroyed.contains(&actor) {
            return Ok(true);
        }
        let field = self
            .find_property(actor_class, "bDeleteMe", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Destroy property bDeleteMe is missing".to_owned())?;
        match instance.get(&field) {
            Some(StoredValue::Value(Value::Bool(true))) => return Ok(true),
            Some(StoredValue::Value(Value::Bool(false))) | None => {}
            Some(value) => return Err(format!("Destroy property bDeleteMe is {value:?}")),
        }
        instance.insert(field, StoredValue::Value(Value::Bool(true)));
        self.set_actor_base(actor, actor_class, instance, None, actions)?;
        self.tick_functions.remove(&actor);
        self.failed_ticks.remove(&actor);
        self.state_frames.remove(&actor);
        if let Some(cached) = self.collision_actors.get_mut(actor) {
            *cached = None;
            self.reindex_cached_collision_actor(actor);
        }
        self.call_actor_event(
            actor,
            actor_class,
            instance,
            "Destroyed",
            Vec::new(),
            actions,
        )?;
        let object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
        let active = std::mem::take(instance);
        if self.instances.insert(actor, active).is_some() {
            return Err(DispatchError::ActiveActorContext { actor }.to_string());
        }
        let result: std::result::Result<(), String> = (|| {
            for based_actor in self.base_children.get(&object).cloned().unwrap_or_default() {
                if self.destroyed.contains(&based_actor) {
                    continue;
                }
                let class = self
                    .actor_classes
                    .get(&based_actor)
                    .cloned()
                    .ok_or(DispatchError::UnregisteredActor { actor: based_actor })
                    .map_err(|error| error.to_string())?;
                let class = self
                    .resolved_object(&class)
                    .map_err(|error| error.to_string())?;
                let mut based_instance = self
                    .instances
                    .remove(&based_actor)
                    .ok_or_else(|| format!("based actor {based_actor} instance is active"))?;
                let result =
                    self.set_actor_base(based_actor, &class, &mut based_instance, None, actions);
                self.instances.insert(based_actor, based_instance);
                result?;
            }
            Ok(())
        })();
        *instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| DispatchError::ActiveActorContext { actor }.to_string())?;
        result?;
        self.destroyed.insert(actor);
        self.timers.remove(&actor);
        self.animating.remove(&actor);
        self.sound_channels
            .retain(|(channel_actor, _), _| *channel_actor != actor);
        actions.push(ActorAction::DestroyActor { actor });
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::world) fn spawn_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        source: &Arc<Package>,
        arguments: &[Value],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        if arguments.is_empty() || arguments.len() > 5 {
            return Err(format!(
                "Spawn expects a class and at most 4 optional arguments, found {}",
                arguments.len()
            ));
        }
        let Some(class_reference) = object_value(&arguments[0]) else {
            return Err(format!(
                "Spawn class is {}, expected object",
                arguments[0].kind()
            ));
        };
        let Some(class) = self
            .resolve_class_value(source, class_reference)
            .map_err(|error| error.to_string())?
        else {
            return Ok(Value::Object(0));
        };
        let script = self.script(&class).map_err(|error| error.to_string())?;
        let ScriptMetadata::Class(metadata) = &script.metadata else {
            return Err("Spawn object is not a class".to_owned());
        };
        let summary = class.package.summary();
        let class_name = summary.name(summary.exports[class.export_index].object_name);
        if metadata.flags & CLASS_ABSTRACT != 0 {
            actions.push(ActorAction::DeferredCall {
                actor,
                message: format!("Spawn cannot instantiate abstract class {class_name}"),
            });
            return Ok(Value::Object(0));
        }
        let class_name = class_name.to_owned();
        let default_location = match self
            .instance_property(actor_class, instance, "Location")
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Vector(value))) => value,
            Some(value) => return Err(format!("Spawn source Location is {value:?}")),
            None => [0.0; 3],
        };
        let location = match arguments.get(3) {
            Some(Value::Vector(value)) => *value,
            Some(Value::None) | None => default_location,
            Some(value) => return Err(format!("Spawn location is {}", value.kind())),
        };
        if !location.iter().all(|value| value.is_finite()) {
            return Err("Spawn location is not finite".to_owned());
        }

        let default_rotation = match self
            .instance_property(actor_class, instance, "Rotation")
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Rotator(value))) => value,
            Some(value) => return Err(format!("Spawn source Rotation is {value:?}")),
            None => [0; 3],
        };
        let rotation = match arguments.get(4) {
            Some(Value::Rotator(value)) => *value,
            Some(Value::None) | None => default_rotation,
            Some(value) => return Err(format!("Spawn rotation is {}", value.kind())),
        };

        let owner = self
            .spawn_object_value(actor, arguments.get(1).unwrap_or(&Value::None))
            .map_err(|error| error.to_string())?;
        let tag = match arguments.get(2) {
            Some(Value::None) | None => class_name.clone(),
            Some(value) => {
                let value = runtime_name(source, value)?;
                if value.eq_ignore_ascii_case("None") {
                    class_name.clone()
                } else {
                    value
                }
            }
        };

        let mut spawned_instance = self
            .load_class_defaults(&class, 0)
            .map_err(|error| error.to_string())?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "Location",
            StoredValue::Value(Value::Vector(location)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "OldLocation",
            StoredValue::Value(Value::Vector(location)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "Rotation",
            StoredValue::Value(Value::Rotator(rotation)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "DesiredRotation",
            StoredValue::Value(Value::Rotator(rotation)),
        )?;
        self.set_spawn_property(&class, &mut spawned_instance, "Tag", StoredValue::Name(tag))?;
        self.set_spawn_property(&class, &mut spawned_instance, "Owner", owner)?;
        for property in ["Instigator", "Level", "XLevel"] {
            if let Some(value) = self
                .instance_property(actor_class, instance, property)
                .map_err(|error| error.to_string())?
            {
                self.set_spawn_property(&class, &mut spawned_instance, property, value)?;
            }
        }
        let Some(location) = self.find_spawn_location(&class, &spawned_instance)? else {
            return Ok(Value::Object(0));
        };
        let location = location.to_array();
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "Location",
            StoredValue::Value(Value::Vector(location)),
        )?;
        self.set_spawn_property(
            &class,
            &mut spawned_instance,
            "OldLocation",
            StoredValue::Value(Value::Vector(location)),
        )?;

        let spawned = self.next_actor;
        self.next_actor = self
            .next_actor
            .checked_add(1)
            .ok_or_else(|| DispatchError::ObjectLimit.to_string())?;
        let object = runtime_actor_id(spawned);
        let handle = self
            .object_handle(object.clone())
            .map_err(|error| error.to_string())?;
        self.object_actors.insert(object.clone(), spawned);
        self.actor_objects.insert(spawned, object);
        self.actor_classes
            .insert(spawned, object_id(&class.package, class.export_index));
        self.track_actor_class(spawned, &class)
            .map_err(|error| error.to_string())?;
        self.actor_states.insert(spawned, None);
        self.destroyed.remove(&spawned);
        self.refresh_tick_actor(spawned, &class)
            .map_err(|error| error.to_string())?;
        self.refresh_cached_collision_actor(spawned, &class, &spawned_instance)?;
        let level = self.actor_object(&class, &spawned_instance, "Level")?;
        self.instances.insert(spawned, spawned_instance);
        self.update_actor_base(spawned, None, level)
            .map_err(|error| error.to_string())?;
        let name = format!("{class_name}{spawned}");
        actions.push(ActorAction::SpawnActor {
            actor: spawned,
            name,
            class_package: Arc::clone(&class.package.summary().source),
            class_export: class.export_index,
            class_name,
            location,
            rotation,
        });

        let parent_instance = std::mem::take(instance);
        if self.instances.insert(actor, parent_instance).is_some() {
            return Err(DispatchError::ActiveActorContext { actor }.to_string());
        }
        for event in [
            "Spawned",
            "PreBeginPlay",
            "BeginPlay",
            "PostBeginPlay",
            "SetInitialState",
        ] {
            match self.dispatch_event(
                spawned,
                Path::new(class.package.summary().source.as_ref()),
                class.export_index,
                event,
            ) {
                Ok(event_actions) => actions.extend(event_actions),
                Err(error) => actions.push(ActorAction::DeferredCall {
                    actor: spawned,
                    message: format!("{event}: {error}"),
                }),
            }
        }
        self.initialize_actor_base(spawned, actions)
            .map_err(|error| error.to_string())?;
        *instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| DispatchError::ActiveActorContext { actor }.to_string())?;

        Ok(if self.destroyed.contains(&spawned) {
            Value::Object(0)
        } else {
            Value::Object(handle)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::world) fn start_animation(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        sequence: String,
        relative_rate: f32,
        tween_time: f32,
        looping: bool,
        tween_only: bool,
        root_motion: bool,
    ) -> std::result::Result<bool, String> {
        let command = AnimationCommand {
            sequence,
            relative_rate,
            tween_time,
            looping,
            tween_only,
            root_motion,
        };
        self.animation_commands.insert(actor, command.clone());
        let configured = self.configure_animation_instance(actor, class, instance, &command)?;
        if self.animation_sequences.contains_key(&actor) && !configured {
            self.animation_commands.remove(&actor);
        }
        Ok(configured)
    }

    pub(in crate::world) fn synchronize_animation_command(
        &mut self,
        actor: usize,
    ) -> std::result::Result<(), String> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("animation actor {actor} is not registered"))?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| format!("animation actor {actor} is active"))?;
        let result = if let Some(command) = self.animation_commands.get(&actor).cloned() {
            match self.configure_animation_instance(actor, &class, &mut instance, &command) {
                Ok(configured) => {
                    if !configured {
                        self.animation_commands.remove(&actor);
                        self.animating.remove(&actor);
                    }
                    Ok(())
                }
                Err(error) => Err(error),
            }
        } else {
            let anim_rate = self.actor_signed_float(&class, &instance, "AnimRate")?;
            let anim_frame = self.actor_signed_float(&class, &instance, "AnimFrame")?;
            let tween_rate = self.actor_float(&class, &instance, "TweenRate")?;
            if anim_rate != 0.0 || anim_frame < 0.0 && tween_rate != 0.0 {
                self.animating.insert(actor);
            }
            Ok(())
        };
        self.instances.insert(actor, instance);
        result
    }

    fn configure_animation_instance(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        command: &AnimationCommand,
    ) -> std::result::Result<bool, String> {
        let Some(sequence) = self
            .animation_sequences
            .get(&actor)
            .and_then(|sequences| sequences.get(&command.sequence.to_ascii_lowercase()))
            .cloned()
        else {
            return Ok(false);
        };
        let current_sequence =
            match self.required_actor_property(class, instance, "AnimSequence")? {
                StoredValue::Name(name) => name,
                value => return Err(format!("actor property AnimSequence is {value:?}")),
            };
        let repeated_loop = command.looping
            && self.animating.contains(&actor)
            && self.actor_bool(class, instance, "bAnimLoop")?
            && current_sequence.eq_ignore_ascii_case(&command.sequence);
        self.set_actor_stored(
            class,
            instance,
            "AnimSequence",
            StoredValue::Name(command.sequence.clone()),
        )?;
        self.set_actor_value(class, instance, "bAnimLoop", Value::Bool(command.looping))?;
        self.set_actor_value(class, instance, "bAnimNotify", Value::Bool(false))?;
        self.set_actor_value(class, instance, "bAnimFinished", Value::Bool(false))?;

        self.set_actor_value(
            class,
            instance,
            "bAnimNotify",
            Value::Bool(!sequence.notifications.is_empty()),
        )?;
        let frames = sequence.frame_count.max(1) as f32;
        let tween_rate = if command.tween_time > 0.0 {
            1.0 / (command.tween_time * frames)
        } else {
            0.0
        };
        if repeated_loop && sequence.frame_count > 1 {
            let anim_rate = command.relative_rate * sequence.rate / frames;
            for (name, value) in [("AnimRate", anim_rate), ("TweenRate", tween_rate)] {
                self.set_actor_value(class, instance, name, Value::Float(value))?;
            }
            return Ok(true);
        }
        let (anim_frame, anim_last, anim_rate) = if command.tween_only {
            (
                if command.tween_time > 0.0 {
                    -1.0 / frames
                } else {
                    0.0
                },
                0.0,
                0.0,
            )
        } else if sequence.frame_count > 1 {
            (
                if command.tween_time > 0.0 {
                    -1.0 / frames
                } else {
                    0.0
                },
                1.0 - 1.0 / frames,
                command.relative_rate * sequence.rate / frames,
            )
        } else {
            (-1.0, 0.0, 0.0)
        };
        let tween_rate = if sequence.frame_count == 1 && command.tween_time == 0.0 {
            10.0
        } else {
            tween_rate
        };
        for (name, value) in [
            ("AnimFrame", anim_frame),
            ("AnimLast", anim_last),
            ("AnimRate", anim_rate),
            ("AnimMinRate", 0.0),
            ("TweenRate", tween_rate),
        ] {
            self.set_actor_value(class, instance, name, Value::Float(value))?;
        }
        if anim_rate != 0.0 || tween_rate != 0.0 {
            self.animating.insert(actor);
        } else {
            self.animating.remove(&actor);
        }
        Ok(true)
    }

    pub(in crate::world) fn resolve_class_value(
        &mut self,
        source: &Arc<Package>,
        reference: i32,
    ) -> DispatchResult<Option<ResolvedObject>> {
        if reference == 0 {
            return Ok(None);
        }
        let handle_object = usize::try_from(reference - 1)
            .ok()
            .and_then(|index| self.handle_objects.get(index))
            .cloned();
        if let Some(object) = handle_object.as_ref() {
            let object = self.resolved_object(object)?;
            if self.is_spawn_class(&object) {
                return Ok(Some(object));
            }
        }
        let reference_object = object_reference(reference);
        let reference_in_bounds = match reference_object {
            ObjectReference::None => false,
            ObjectReference::Export(index) => index < source.summary().exports.len(),
            ObjectReference::Import(index) => index < source.summary().imports.len(),
        };
        if reference_in_bounds
            && let Some(object) = self.packages.resolve(source, reference_object)?
            && self.is_spawn_class(&object)
        {
            return Ok(Some(object));
        }
        let object =
            handle_object.ok_or(DispatchError::InvalidObjectHandle { handle: reference })?;
        let object = self.resolved_object(&object)?;
        let summary = object.package.summary();
        let export = &summary.exports[object.export_index];
        Err(DispatchError::UnresolvedObject {
            message: format!(
                "Spawn object {} `{}` is not a class",
                summary.class_name(export).unwrap_or("<unknown>"),
                summary.name(export.object_name)
            ),
        })
    }

    fn is_spawn_class(&mut self, object: &ResolvedObject) -> bool {
        let summary = object.package.summary();
        match summary.class_name(&summary.exports[object.export_index]) {
            Some(name) => name.eq_ignore_ascii_case("Class"),
            None => self
                .script(object)
                .is_ok_and(|script| matches!(script.metadata, ScriptMetadata::Class(_))),
        }
    }

    fn spawn_object_value(
        &self,
        current_actor: usize,
        value: &Value,
    ) -> DispatchResult<StoredValue> {
        Ok(match value {
            Value::None | Value::Object(0) => StoredValue::Object(None),
            Value::Object(-1) => StoredValue::Object(Some(
                self.actor_objects.get(&current_actor).cloned().ok_or(
                    DispatchError::UnregisteredActor {
                        actor: current_actor,
                    },
                )?,
            )),
            Value::Object(handle) => {
                let index = usize::try_from(*handle - 1)
                    .ok()
                    .filter(|index| *index < self.handle_objects.len())
                    .ok_or(DispatchError::InvalidObjectHandle { handle: *handle })?;
                StoredValue::Object(Some(self.handle_objects[index].clone()))
            }
            value => {
                return Err(DispatchError::UnresolvedObject {
                    message: format!("Spawn owner is {}, expected object", value.kind()),
                });
            }
        })
    }

    fn set_spawn_property(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        name: &str,
        value: StoredValue,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(class, name, 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Spawn property {name} is missing"))?;
        instance.insert(field, value);
        Ok(())
    }
}
