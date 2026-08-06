use super::*;

impl ScriptRuntime {
    pub(super) fn tick_falling(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let Some(zone) = self.zone_physics(Vec3::from_array(old_location), actor, instance)? else {
            self.fell_out_of_world(actor, class, instance, actions)?;
            return Ok(());
        };
        let pawn = self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?;
        let decoration = self
            .class_has_name(class, "Decoration")
            .map_err(|error| error.to_string())?;
        let mut acceleration =
            Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        let old_velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);

        let mut ground_speed = 0.0;
        if pawn {
            let max_acceleration = self.actor_float(class, instance, "AirControl")?
                * self.actor_float(class, instance, "AccelRate")?;
            if acceleration.length() > max_acceleration {
                acceleration = acceleration.normalize_or_zero() * max_acceleration;
                self.set_actor_value(
                    class,
                    instance,
                    "Acceleration",
                    Value::Vector(acceleration.to_array()),
                )?;
            }
            ground_speed = self.actor_float(class, instance, "GroundSpeed")?;
        }

        let bobbing = decoration
            && self
                .optional_actor_bool(class, instance, "bBobbing")?
                .unwrap_or(false);
        let gravity_scale = if bobbing { 1.0 } else { 2.0 };
        let fluid_friction = if pawn && zone.water && old_velocity.z < 0.0 {
            zone.fluid_friction
        } else {
            0.0
        };
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let mut velocity = fall_velocity(
            old_velocity,
            acceleration,
            zone.gravity,
            gravity_scale,
            fluid_friction,
            elapsed,
        );
        let old_horizontal = old_velocity.truncate();
        let new_horizontal = velocity.truncate();
        if pawn
            && old_horizontal.length_squared() >= ground_speed * ground_speed
            && new_horizontal.length_squared() > old_horizontal.length_squared()
        {
            let horizontal = new_horizontal.normalize_or_zero() * old_horizontal.length();
            velocity.x = horizontal.x;
            velocity.y = horizontal.y;
        }
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let mut time_left = elapsed;
        for _ in 0..5 {
            if time_left <= 0.0 || self.destroyed.contains(&actor) {
                break;
            }
            if velocity.length_squared() > zone.terminal_velocity * zone.terminal_velocity {
                velocity = velocity.normalize_or_zero() * zone.terminal_velocity;
                self.set_actor_value(
                    class,
                    instance,
                    "Velocity",
                    Value::Vector(velocity.to_array()),
                )?;
            }
            let move_delta = (velocity + zone.velocity * (elapsed * 25.0)) * time_left;
            let hit =
                self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
            time_left -= time_left * hit.fraction;
            if hit.fraction == 1.0 {
                continue;
            }

            let hit_pawn = hit
                .actor
                .map(|actor| self.actor_has_class(actor, "Pawn"))
                .transpose()
                .map_err(|error| error.to_string())?
                .unwrap_or(false);
            if pawn && !hit_pawn {
                self.try_mount(actor, class, instance, hit, actions)?;
            }
            if !hit_pawn {
                self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
            }

            if self.actor_bool(class, instance, "bBounce")? {
                let reflected = move_delta.reflect(hit.normal);
                self.try_move_actor(actor, class, reflected.to_array(), instance, actions)?;
                continue;
            }
            if hit.normal.z < WALKABLE_FLOOR_Z {
                let aligned =
                    (move_delta - hit.normal * move_delta.dot(hit.normal)) * (1.0 - hit.fraction);
                if move_delta.dot(aligned) >= 0.0 {
                    let slope =
                        self.try_move_actor(actor, class, aligned.to_array(), instance, actions)?;
                    if slope.fraction < 1.0 && slope.normal.z > WALKABLE_FLOOR_Z {
                        self.phys_landed(
                            actor,
                            class,
                            instance,
                            slope.normal,
                            slope.actor,
                            actions,
                        )?;
                        return Ok(());
                    }
                }
                if !self.actor_bool(class, instance, "bJustTeleported")? {
                    let location =
                        Vec3::from_array(self.actor_vector(class, instance, "Location")?);
                    velocity = (location - Vec3::from_array(old_location)) / elapsed;
                    self.set_actor_value(
                        class,
                        instance,
                        "Velocity",
                        Value::Vector(velocity.to_array()),
                    )?;
                }
                break;
            }
            self.phys_landed(actor, class, instance, hit.normal, hit.actor, actions)?;
            break;
        }
        Ok(())
    }

    pub(in crate::world) fn try_mount(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        hit: MovementHit,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        let max_height = self.actor_float(class, instance, "MaxMountHeight")?;
        if max_height <= 0.0 || !(-0.1..0.7).contains(&hit.normal.z) {
            return Ok(false);
        }
        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        let radius = self.actor_float(class, instance, "CollisionRadius")?;
        if !self.movement_hit_has_poly_flag(
            actor,
            instance,
            hit,
            location,
            radius * 2.0 + 1.0,
            PolyFlags::HIGH_LEDGE,
        )? {
            return Ok(false);
        }
        let rotation = self.actor_rotator(class, instance, "Rotation")?;
        let forward = Vec3::from_array(crate::rotator_axes(rotation)[0]);
        if forward.dot(hit.normal) >= 0.0 {
            return Ok(false);
        }

        let up = Vec3::Z;
        let inward = Vec3::new(-hit.normal.x, -hit.normal.y, 0.0).normalize_or_zero();
        let raised = location + up * max_height;
        let far = raised + inward * (radius * 2.0 + max_height * hit.normal.z);
        let diagonal_end = far - (up + hit.normal) * max_height;
        let ledge = self.test_move_actor_between(actor, class, instance, far, diagonal_end)?;
        if ledge.fraction == 1.0 {
            return Ok(false);
        }
        let ledge_location = far + (diagonal_end - far) * ledge.fraction;
        let minimum_rise = if self.actor_byte(class, instance, "Physics")? == PHYS_FALLING {
            0.0
        } else {
            self.actor_float(class, instance, "MaxStepHeight")?
        };
        if ledge_location.z - location.z < minimum_rise {
            return Ok(false);
        }
        let destination = ledge_location + Vec3::Z * 0.51;
        if self
            .test_move_actor_between(actor, class, instance, location, raised)?
            .fraction
            < 1.0
            || self
                .test_move_actor_between(actor, class, instance, raised, destination)?
                .fraction
                < 1.0
        {
            return Ok(false);
        }

        let base = hit
            .actor
            .and_then(|actor| self.actor_objects.get(&actor).cloned())
            .or_else(|| {
                self.level_info
                    .and_then(|level| self.actor_objects.get(&level).cloned())
            });
        self.set_actor_base(actor, class, instance, base, actions)?;
        self.call_actor_event(
            actor,
            class,
            instance,
            "Mount",
            vec![Value::Vector((destination - location).to_array())],
            actions,
        )?;
        Ok(true)
    }

    pub(super) fn tick_flying(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
        swimming: bool,
    ) -> std::result::Result<(), String> {
        if !self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let old_location = self.actor_vector(class, instance, "Location")?;
        let Some(zone) = self.zone_physics(Vec3::from_array(old_location), actor, instance)? else {
            self.fell_out_of_world(actor, class, instance, actions)?;
            return Ok(());
        };
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let mut acceleration =
            Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        if acceleration.length_squared() > 0.0001 {
            let acceleration_rate =
                self.actor_float(class, instance, "AccelRate")? * if swimming { 0.3 } else { 1.0 };
            let acceleration_speed = acceleration.length();
            let acceleration_direction = acceleration / acceleration_speed;
            if acceleration_speed > acceleration_rate {
                acceleration = acceleration_direction * acceleration_rate;
                self.set_actor_value(
                    class,
                    instance,
                    "Acceleration",
                    Value::Vector(acceleration.to_array()),
                )?;
            }
            let speed = velocity.length();
            velocity -=
                (velocity - acceleration_direction * speed) * (zone.fluid_friction * elapsed);
        } else {
            let speed = velocity.length();
            if speed > 0.0 {
                let new_speed = (speed - speed * zone.fluid_friction * 2.0 * elapsed).max(0.0);
                velocity *= new_speed / speed;
            }
        }
        velocity += acceleration * elapsed;

        let speed_property = if swimming { "WaterSpeed" } else { "AirSpeed" };
        let mut maximum_speed = self.actor_float(class, instance, speed_property)?;
        if !self
            .class_has_name(class, "PlayerPawn")
            .map_err(|error| error.to_string())?
        {
            maximum_speed *= self.actor_float(class, instance, "DesiredSpeed")?;
        }
        let speed = velocity.length();
        if speed > maximum_speed && speed > 0.0 {
            velocity *= maximum_speed / speed;
        }
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let movement_velocity = velocity + zone.velocity * (elapsed * 25.0);
        if has_movement(movement_velocity) {
            let mut time_left = elapsed;
            for _ in 0..5 {
                if time_left <= 0.0 {
                    break;
                }
                let mut move_delta = movement_velocity * time_left;
                let hit =
                    self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
                time_left -= time_left * hit.fraction;
                move_delta = movement_velocity * time_left;
                if hit.fraction >= 1.0 {
                    continue;
                }

                let pushable = if swimming
                    && self
                        .class_has_name(class, "PlayerPawn")
                        .map_err(|error| error.to_string())?
                    && hit.normal.dot(move_delta) < -0.9
                    && let Some(other) = hit.actor
                {
                    self.actor_has_class(other, "Decoration")
                        .map_err(|error| error.to_string())?
                        && self.other_actor_bool(other, "bPushable")?
                } else {
                    false
                };
                if pushable {
                    let other = hit.actor.unwrap();
                    let mass = self.actor_float(class, instance, "Mass")?;
                    let other_mass = self.other_actor_float(other, "Mass")?;
                    self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(true))?;
                    velocity *= mass / (mass + other_mass);
                    self.set_actor_value(
                        class,
                        instance,
                        "Velocity",
                        Value::Vector(velocity.to_array()),
                    )?;
                    self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                    break;
                }

                self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                let aligned =
                    (move_delta - hit.normal * move_delta.dot(hit.normal)) * (1.0 - hit.fraction);
                if move_delta.dot(aligned) < 0.0 {
                    break;
                }
                let slide =
                    self.try_move_actor(actor, class, aligned.to_array(), instance, actions)?;
                time_left -= time_left * slide.fraction;
                if slide.fraction < 1.0 {
                    self.call_hit_wall(actor, class, instance, slide.normal, slide.actor, actions)?;
                }
            }
        }

        if !self.actor_bool(class, instance, "bJustTeleported")? {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            velocity = (location - Vec3::from_array(old_location)) / elapsed;
        }
        if swimming {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            let new_zone = self.zone_physics(location, actor, instance)?;
            if !new_zone.is_some_and(|zone| zone.water) {
                if velocity.z > 0.0 {
                    velocity.z = velocity.z.max((100.0 + velocity.truncate().length()) * 0.5);
                }
                if self.actor_byte(class, instance, "Physics")? == PHYS_SWIMMING {
                    self.set_actor_value(class, instance, "Physics", Value::Byte(PHYS_FALLING))?;
                }
            }
        }
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;
        Ok(())
    }

    pub(super) fn tick_projectile(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let Some(zone) = self.zone_physics(Vec3::from_array(old_location), actor, instance)? else {
            self.native(
                actor,
                class,
                &Arc::clone(&class.package),
                DESTROY,
                &[],
                instance,
                actions,
                0,
            )?;
            return Ok(());
        };
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let acceleration = Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        if zone.water {
            velocity *= (1.0 - zone.fluid_friction * 0.2 * elapsed).max(0.0);
        }
        velocity += acceleration * elapsed;
        if self
            .class_has_name(class, "Projectile")
            .map_err(|error| error.to_string())?
            && let Some(max_speed) = self.optional_actor_float(class, instance, "MaxSpeed")?
            && velocity.length_squared() > max_speed * max_speed
        {
            velocity = velocity.normalize_or_zero() * max_speed;
        }
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let hit = self.try_move_actor(
            actor,
            class,
            (velocity * elapsed).to_array(),
            instance,
            actions,
        )?;
        let just_teleported = self.actor_bool(class, instance, "bJustTeleported")?;
        if hit.fraction < 1.0
            && hit.actor.is_none()
            && !self.destroyed.contains(&actor)
            && !just_teleported
        {
            let level = self.actor_object(class, instance, "Level")?;
            let level = match level {
                Some(level) => self
                    .object_handle(level)
                    .map_err(|error| error.to_string())?,
                None => 0,
            };
            actions.push(ActorAction::DispatchEvent {
                actor,
                event: "HitWall",
                arguments: vec![Value::Vector(hit.normal.to_array()), Value::Object(level)],
            });
        }
        if !self.actor_bool(class, instance, "bBounce")? && !just_teleported {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            velocity = (location - Vec3::from_array(old_location)) / elapsed;
            self.set_actor_value(
                class,
                instance,
                "Velocity",
                Value::Vector(velocity.to_array()),
            )?;
        }
        Ok(())
    }

    pub(super) fn tick_rolling(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let Some(zone) = self.zone_physics(Vec3::from_array(old_location), actor, instance)? else {
            self.fell_out_of_world(actor, class, instance, actions)?;
            return Ok(());
        };
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let acceleration = Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        let speed = velocity.length();
        velocity -= speed
            * (velocity.normalize_or_zero() - acceleration.normalize_or_zero())
            * zone.ground_friction
            * elapsed;
        velocity = velocity * (1.0 - zone.fluid_friction * elapsed) + acceleration * elapsed;
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let move_delta = (velocity + zone.velocity * (elapsed * 25.0)) * elapsed;
        let hit = self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
        if hit.fraction < 1.0 && hit.normal.z < WALKABLE_FLOOR_Z {
            self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
            let aligned =
                (move_delta - hit.normal * move_delta.dot(hit.normal)) * (1.0 - hit.fraction);
            if move_delta.dot(aligned) >= 0.0 {
                self.try_move_actor(actor, class, aligned.to_array(), instance, actions)?;
            }
        }
        if self.actor_byte(class, instance, "Physics")? != PHYS_ROLLING {
            return Ok(());
        }

        let gravity_direction = if zone.gravity.z > 0.0 { 1.0 } else { -1.0 };
        let height = self.actor_float(class, instance, "CollisionHeight")?;
        let step_down = Vec3::new(
            0.0,
            0.0,
            gravity_direction * (25.0 / 47.5) * height * STEP_DOWN_FACTOR,
        );
        let floor = self.test_move_actor(actor, class, step_down.to_array(), instance)?;
        if floor.fraction == 1.0 || floor.normal.z < WALKABLE_FLOOR_Z {
            self.set_actor_value(class, instance, "Physics", Value::Byte(PHYS_FALLING))?;
            self.set_actor_base(actor, class, instance, None, actions)?;
        } else {
            let floor =
                self.try_move_actor(actor, class, step_down.to_array(), instance, actions)?;
            if floor.fraction != 1.0 {
                let base = floor
                    .actor
                    .and_then(|actor| self.actor_objects.get(&actor).cloned());
                self.set_actor_base(actor, class, instance, base, actions)?;
            }
        }
        if !self.actor_bool(class, instance, "bJustTeleported")? {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            velocity = (location - Vec3::from_array(old_location)) / elapsed;
            self.set_actor_value(
                class,
                instance,
                "Velocity",
                Value::Vector(velocity.to_array()),
            )?;
        }
        Ok(())
    }

    pub(super) fn tick_trailer(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let Some(owner) = self.actor_object(class, instance, "Owner")? else {
            return Ok(());
        };
        let Some(owner) = self.object_actors.get(&owner).copied() else {
            return Ok(());
        };
        let location = Vec3::from_array(self.other_actor_vector(owner, "Location")?);
        self.set_actor_location(actor, class, instance, location, actions)?;
        if self.actor_byte(class, instance, "DrawType")? != 1 {
            let rotation = self.other_actor_rotator(owner, "Rotation")?;
            self.set_actor_value(class, instance, "Rotation", Value::Rotator(rotation))?;
            actions.push(ActorAction::SetRotation { actor, rotation });
        }
        Ok(())
    }

    pub(super) fn tick_interpolating(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;

        let mut time_left = elapsed;
        while time_left > 0.0 {
            let rate = self.actor_float_any(class, instance, "PhysRate")?;
            if rate == 0.0 || !self.actor_bool(class, instance, "bInterpolating")? {
                break;
            }
            let Some(target) = self.actor_object(class, instance, "Target")? else {
                break;
            };
            let Some(target_actor) = self.object_actors.get(&target).copied() else {
                break;
            };
            let Some(next) = self.other_actor_object(target_actor, "Next")? else {
                break;
            };
            let Some(next_actor) = self.object_actors.get(&next).copied() else {
                break;
            };

            let mut alpha = self.actor_float_any(class, instance, "PhysAlpha")?;
            let rate_modifier = self.other_actor_float(target_actor, "RateModifier")?;
            let adjusted_rate = rate * rate_modifier;
            if adjusted_rate == 0.0 {
                break;
            }
            let mut reached_start = false;
            let mut reached_end = false;
            alpha += adjusted_rate * time_left;
            if adjusted_rate < 0.0 && alpha < 0.0 {
                time_left = alpha / adjusted_rate;
                alpha = 0.0;
                reached_start = true;
            } else if adjusted_rate > 0.0 && alpha > 1.0 {
                time_left = (alpha - 1.0) / adjusted_rate;
                alpha = 1.0;
                reached_end = true;
            } else {
                time_left = 0.0;
            }

            let target_location =
                Vec3::from_array(self.other_actor_vector(target_actor, "Location")?);
            let next_location = Vec3::from_array(self.other_actor_vector(next_actor, "Location")?);
            let target_rotation = self.other_actor_rotator(target_actor, "Rotation")?;
            let next_rotation = self.other_actor_rotator(next_actor, "Rotation")?;
            let previous = self
                .other_actor_object(target_actor, "Prev")?
                .and_then(|object| self.object_actors.get(&object).copied());
            let next_next = self
                .other_actor_object(next_actor, "Next")?
                .and_then(|object| self.object_actors.get(&object).copied());
            let (location, rotation) =
                if let (Some(previous), Some(next_next)) = (previous, next_next) {
                    (
                        spline_vector(
                            Vec3::from_array(self.other_actor_vector(previous, "Location")?),
                            target_location,
                            next_location,
                            Vec3::from_array(self.other_actor_vector(next_next, "Location")?),
                            alpha,
                        ),
                        spline_rotator(
                            self.other_actor_rotator(previous, "Rotation")?,
                            target_rotation,
                            next_rotation,
                            self.other_actor_rotator(next_next, "Rotation")?,
                            alpha,
                        ),
                    )
                } else {
                    (
                        target_location.lerp(next_location, alpha),
                        lerp_rotator(target_rotation, next_rotation, alpha),
                    )
                };

            self.set_actor_value(class, instance, "PhysAlpha", Value::Float(alpha))?;
            let current = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            self.try_move_actor(
                actor,
                class,
                (location - current).to_array(),
                instance,
                actions,
            )?;
            self.set_actor_value(class, instance, "Rotation", Value::Rotator(rotation))?;
            actions.push(ActorAction::SetRotation { actor, rotation });
            if self
                .class_has_name(class, "PlayerPawn")
                .map_err(|error| error.to_string())?
            {
                self.set_actor_value(class, instance, "ViewRotation", Value::Rotator(rotation))?;
            }

            if reached_start || reached_end {
                let current_object = self
                    .actor_objects
                    .get(&actor)
                    .cloned()
                    .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
                let current_handle = self
                    .object_handle(current_object)
                    .map_err(|error| error.to_string())?;
                self.call_other_actor_event(
                    target_actor,
                    "InterpolateEnd",
                    vec![Value::Object(current_handle)],
                    actions,
                )?;
                let target_handle = self
                    .object_handle(target.clone())
                    .map_err(|error| error.to_string())?;
                self.call_actor_event(
                    actor,
                    class,
                    instance,
                    "InterpolateEnd",
                    vec![Value::Object(target_handle)],
                    actions,
                )?;

                let property = if reached_start { "Prev" } else { "Next" };
                let target = self.other_actor_object(target_actor, property)?;
                self.set_actor_stored(class, instance, "Target", StoredValue::Object(target))?;
                self.set_actor_value(
                    class,
                    instance,
                    "PhysAlpha",
                    Value::Float(if reached_start { 1.0 } else { 0.0 }),
                )?;
            }
        }

        if elapsed > 0.0 {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            self.set_actor_value(
                class,
                instance,
                "Velocity",
                Value::Vector(((location - Vec3::from_array(old_location)) / elapsed).to_array()),
            )?;
        }
        Ok(())
    }

    pub(in crate::world) fn tick_moving_brush(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        if !self
            .class_has_name(class, "Mover")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }

        if elapsed > 0.0 && self.actor_bool(class, instance, "bCollideWorld")? {
            let location = Vec3::from_array(old_location);
            if let Some(zone) = self.zone_physics(location, actor, instance)?
                && zone.gravity.length_squared() > 0.0
            {
                let velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
                let parallel_velocity =
                    zone.gravity * velocity.dot(zone.gravity) / zone.gravity.length_squared();
                let gravity_delta =
                    parallel_velocity * elapsed + zone.gravity * (0.5 * elapsed * elapsed);
                self.set_actor_value(
                    class,
                    instance,
                    "Velocity",
                    Value::Vector((velocity + zone.gravity * elapsed).to_array()),
                )?;

                let hit =
                    self.try_move_actor(actor, class, gravity_delta.to_array(), instance, actions)?;
                let moved =
                    Vec3::from_array(self.actor_vector(class, instance, "Location")?) - location;
                if hit.fraction > 0.0 && moved.length_squared() > 0.0 {
                    let key = usize::from(self.actor_byte(class, instance, "KeyNum")?).min(7);
                    let mut key_positions =
                        self.required_actor_property(class, instance, "KeyPos")?;
                    let StoredValue::Array(values) = &mut key_positions else {
                        return Err(format!("actor property KeyPos is {key_positions:?}"));
                    };
                    let Some(StoredValue::Value(Value::Vector(key_position))) = values.get_mut(key)
                    else {
                        return Err(format!(
                            "actor property KeyPos[{key}] is missing or invalid"
                        ));
                    };
                    *key_position = (Vec3::from_array(*key_position) + moved).to_array();
                    self.set_actor_stored(class, instance, "KeyPos", key_positions)?;

                    let old_position =
                        Vec3::from_array(self.actor_vector(class, instance, "OldPos")?);
                    self.set_actor_value(
                        class,
                        instance,
                        "OldPos",
                        Value::Vector((old_position + moved).to_array()),
                    )?;
                } else if hit.fraction == 0.0 && hit.normal.length_squared() > 0.0 {
                    let base = hit
                        .actor
                        .and_then(|actor| self.actor_objects.get(&actor).cloned())
                        .or_else(|| {
                            self.level_info
                                .and_then(|level| self.actor_objects.get(&level).cloned())
                        });
                    self.set_actor_base(actor, class, instance, base, actions)?;
                }
            }
        }

        let mut time_left = elapsed;
        while time_left > 0.0 {
            if !self.actor_bool(class, instance, "bInterpolating")? {
                break;
            }
            let rate = self.actor_float_any(class, instance, "PhysRate")?;
            if rate <= 0.0 {
                break;
            }

            let previous_alpha = self.actor_float_any(class, instance, "PhysAlpha")?;
            let mut alpha = previous_alpha;
            alpha += rate * time_left;
            if alpha > 1.0 {
                time_left = (alpha - 1.0) / rate;
                alpha = 1.0;
            } else {
                time_left = 0.0;
            }
            let blend = if self.actor_byte(class, instance, "MoverGlideType")? == 1 {
                alpha * alpha * (3.0 - 2.0 * alpha)
            } else {
                alpha
            };
            let key = usize::from(self.actor_byte(class, instance, "KeyNum")?).min(7);
            let old_position = Vec3::from_array(self.actor_vector(class, instance, "OldPos")?);
            let base_position = Vec3::from_array(self.actor_vector(class, instance, "BasePos")?);
            let key_position =
                Vec3::from_array(self.actor_array_vector(class, instance, "KeyPos", key)?);
            let old_rotation = self.actor_rotator(class, instance, "OldRot")?;
            let base_rotation = self.actor_rotator(class, instance, "BaseRot")?;
            let key_rotation = self.actor_array_rotator(class, instance, "KeyRot", key)?;
            let target_position =
                old_position + (base_position + key_position - old_position) * blend;
            let target_rotation = mover_rotation(old_rotation, base_rotation, key_rotation, blend);
            let current = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            let hit = self.try_move_actor(
                actor,
                class,
                (target_position - current).to_array(),
                instance,
                actions,
            )?;
            if hit.fraction == 1.0 {
                self.set_actor_value(class, instance, "Rotation", Value::Rotator(target_rotation))?;
                actions.push(ActorAction::SetRotation {
                    actor,
                    rotation: target_rotation,
                });
                self.set_actor_value(class, instance, "PhysAlpha", Value::Float(alpha))?;
                if alpha == 1.0 {
                    self.set_actor_value(class, instance, "bInterpolating", Value::Bool(false))?;
                    self.call_actor_event(
                        actor,
                        class,
                        instance,
                        "InterpolateEnd",
                        vec![Value::Object(0)],
                        actions,
                    )?;
                }
            } else {
                self.set_actor_value(
                    class,
                    instance,
                    "PhysAlpha",
                    Value::Float(previous_alpha + (alpha - previous_alpha) * hit.fraction),
                )?;
                self.set_actor_value(class, instance, "bInterpolating", Value::Bool(false))?;
                break;
            }
        }
        if elapsed > 0.0 {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            self.set_actor_value(
                class,
                instance,
                "Velocity",
                Value::Vector(((location - Vec3::from_array(old_location)) / elapsed).to_array()),
            )?;
        }
        Ok(())
    }

    pub(super) fn tick_rotating(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let mut rotation = self.actor_rotator(class, instance, "Rotation")?;
        let rate = self.actor_rotator(class, instance, "RotationRate")?;
        let before = rotation;
        let pawn = self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?;
        let player_pawn = self
            .class_has_name(class, "PlayerPawn")
            .map_err(|error| error.to_string())?;
        // Keep normal PlayerPawn rotation script-controlled. HP1's blocking
        // latent turns still need Pawn rotation to resume their authored state.
        if player_rotation_is_script_controlled(player_pawn, actor, &self.state_frames) {
            return Ok(());
        }
        if pawn {
            self.set_actor_value(class, instance, "bRotateToDesired", Value::Bool(true))?;
            self.set_actor_value(class, instance, "bFixedRotationDir", Value::Bool(false))?;
            let desired = self.actor_rotator(class, instance, "DesiredRotation")?;
            if !rotators_equal(rotation, desired) {
                rotation[1] = turn_to_shortest(
                    rotation[1],
                    desired[1],
                    (rate[1] as f32 * elapsed).abs() as i32,
                );
                rotation[0] = clamp_pawn_pitch(desired[0], rate[0]);
                if rotators_equal(rotation, desired) {
                    actions.push(ActorAction::DispatchEvent {
                        actor,
                        event: "EndedRotation",
                        arguments: Vec::new(),
                    });
                }
            }
        } else {
            let rotate_to_desired = self.actor_bool(class, instance, "bRotateToDesired")?;
            let fixed_direction = self.actor_bool(class, instance, "bFixedRotationDir")?;
            if rotate_to_desired {
                let desired = self.actor_rotator(class, instance, "DesiredRotation")?;
                if !rotators_equal(rotation, desired) {
                    for index in 0..3 {
                        let step = (rate[index] as f32 * elapsed) as i32;
                        rotation[index] = if fixed_direction {
                            turn_to_fixed(rotation[index], desired[index], step)
                        } else {
                            turn_to_shortest(rotation[index], desired[index], step.abs())
                        };
                    }
                    if rotators_equal(rotation, desired) {
                        actions.push(ActorAction::DispatchEvent {
                            actor,
                            event: "EndedRotation",
                            arguments: Vec::new(),
                        });
                    }
                }
            } else if fixed_direction {
                for index in 0..3 {
                    rotation[index] =
                        rotation[index].wrapping_add((rate[index] as f32 * elapsed) as i32);
                }
            }
        }

        if rotation != before {
            self.set_actor_value(class, instance, "Rotation", Value::Rotator(rotation))?;
            actions.push(ActorAction::SetRotation { actor, rotation });
        }
        Ok(())
    }
}
