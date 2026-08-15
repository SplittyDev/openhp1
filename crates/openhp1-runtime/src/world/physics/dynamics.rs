use super::*;

fn falling_calls_hit_wall(bounce: bool, hit_pawn: bool, hit_normal: Vec3) -> bool {
    bounce || (!hit_pawn && hit_normal.z < WALKABLE_FLOOR_Z)
}

impl ScriptRuntime {
    pub(super) fn tick_interpolation_manager(
        &mut self,
        manager: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let Some(owner) = self.actor_object(class, instance, "Owner")? else {
            return Ok(());
        };
        let Some(owner) = self.object_actors.get(&owner).copied() else {
            return Ok(());
        };
        if self.destroyed.contains(&owner) {
            return Ok(());
        }

        let mut time_left = elapsed;
        while time_left > 0.0 && !self.destroyed.contains(&manager) {
            let pause = self.actor_float_any(class, instance, "RemainingPause")?;
            if pause > 0.0 {
                let consumed = time_left.min(pause);
                let remaining = pause - consumed;
                self.set_actor_value(class, instance, "RemainingPause", Value::Float(remaining))?;
                time_left -= consumed;
                if remaining > 0.0 {
                    break;
                }
                self.finish_interpolation_manager_segment(manager, class, instance, true, actions)?;
                continue;
            }

            let rate = self.actor_float_any(class, instance, "PhysRate")?;
            if rate == 0.0 {
                break;
            }
            let Some(destination) = self.actor_object(class, instance, "Dest")? else {
                break;
            };
            let Some(destination) = self.object_actors.get(&destination).copied() else {
                break;
            };
            let Some(previous) = self.other_actor_object(destination, "Prev")? else {
                break;
            };
            let Some(previous) = self.object_actors.get(&previous).copied() else {
                break;
            };

            let alpha = self.actor_float_any(class, instance, "PhysAlpha")?;
            let owner_class = self
                .actor_classes
                .get(&owner)
                .cloned()
                .ok_or_else(|| format!("interpolation owner {owner} has no class"))?;
            let owner_class = self
                .resolved_object(&owner_class)
                .map_err(|error| error.to_string())?;
            let owner_instance = self
                .instances
                .get(&owner)
                .cloned()
                .ok_or_else(|| format!("interpolation owner {owner} instance is active"))?;
            if !self.actor_bool(&owner_class, &owner_instance, "bInterpolating")? {
                break;
            }
            let path_distance = self.other_actor_float(destination, "PathDist")?;
            let speed = self.interpolation_manager_speed(
                &owner_class,
                &owner_instance,
                previous,
                destination,
                alpha,
            )?;
            let adjusted_rate = if path_distance > 0.0 {
                rate * speed / path_distance
            } else {
                rate
            };
            if adjusted_rate == 0.0 {
                break;
            }

            let forward = adjusted_rate > 0.0;
            let boundary = if forward { 1.0 } else { 0.0 };
            let time_to_boundary = (boundary - alpha) / adjusted_rate;
            let reached_boundary = time_to_boundary >= 0.0 && time_to_boundary <= time_left;
            let consumed = if reached_boundary {
                time_to_boundary
            } else {
                time_left
            };
            let next_alpha = if reached_boundary {
                boundary
            } else {
                alpha + adjusted_rate * consumed
            };
            self.set_actor_value(class, instance, "PhysAlpha", Value::Float(next_alpha))?;
            self.call_actor_event(
                manager,
                class,
                instance,
                "UpdateCamera",
                vec![Value::Float(next_alpha)],
                actions,
            )?;
            self.move_interpolation_manager_owner(
                manager,
                instance,
                owner,
                &owner_class,
                previous,
                destination,
                next_alpha,
                consumed,
                actions,
            )?;
            time_left -= consumed;

            if !reached_boundary {
                break;
            }
            self.set_actor_value(
                class,
                instance,
                "PhysAlpha",
                Value::Float(if forward { 0.0 } else { 1.0 }),
            )?;
            self.finish_interpolation_manager_segment(manager, class, instance, forward, actions)?;
            if self.actor_object(class, instance, "Dest")?
                == self.actor_objects.get(&destination).cloned()
                && self.actor_float_any(class, instance, "RemainingPause")? == 0.0
            {
                break;
            }
        }
        Ok(())
    }

    fn interpolation_manager_speed(
        &mut self,
        owner_class: &ResolvedObject,
        owner_instance: &InstanceState,
        previous: usize,
        destination: usize,
        alpha: f32,
    ) -> std::result::Result<f32, String> {
        if let Some(speed) = self.optional_actor_float(owner_class, owner_instance, "IPSpeed")?
            && speed > 0.0
        {
            return Ok(speed);
        }
        let destination_speed = self.other_actor_float(destination, "DesiredSpeed")?;
        if destination_speed == 0.0 {
            return Ok(Vec3::from_array(self.actor_vector(
                owner_class,
                owner_instance,
                "Velocity",
            )?)
            .length());
        }
        let previous_speed = self.other_actor_float(previous, "DesiredSpeed")?;
        Ok(previous_speed + (destination_speed - previous_speed) * alpha)
    }

    #[allow(clippy::too_many_arguments)]
    fn move_interpolation_manager_owner(
        &mut self,
        manager: usize,
        manager_instance: &mut InstanceState,
        owner: usize,
        owner_class: &ResolvedObject,
        previous: usize,
        destination: usize,
        alpha: f32,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let start = Vec3::from_array(self.other_actor_vector(previous, "Location")?);
        let end = Vec3::from_array(self.other_actor_vector(destination, "Location")?);
        let start_control =
            start + Vec3::from_array(self.other_actor_vector(previous, "StartControlPoint")?);
        let end_control =
            end + Vec3::from_array(self.other_actor_vector(destination, "EndControlPoint")?);
        let location = bezier_vector(start, start_control, end_control, end, alpha);
        let rotation = if self.other_actor_bool(destination, "bFaceMoveDirection")? {
            direction_rotator(bezier_tangent(
                start,
                start_control,
                end_control,
                end,
                alpha,
            ))
        } else {
            lerp_rotator(
                self.other_actor_rotator(previous, "Rotation")?,
                self.other_actor_rotator(destination, "Rotation")?,
                alpha,
            )
        };

        let active_manager = std::mem::take(manager_instance);
        self.instances.insert(manager, active_manager);
        let mut owner_instance = self
            .instances
            .remove(&owner)
            .ok_or_else(|| format!("interpolation owner {owner} instance is active"))?;
        let result = (|| {
            let old_location =
                Vec3::from_array(self.actor_vector(owner_class, &owner_instance, "Location")?);
            self.set_actor_value(
                owner_class,
                &mut owner_instance,
                "OldLocation",
                Value::Vector(old_location.to_array()),
            )?;
            let hit = self.try_move_actor(
                owner,
                owner_class,
                (location - old_location).to_array(),
                &mut owner_instance,
                actions,
            )?;
            if hit.fraction == 1.0 {
                self.try_move_actor_rotated(
                    owner,
                    owner_class,
                    rotation,
                    &mut owner_instance,
                    actions,
                )?;
            }
            if elapsed > 0.0 {
                let location = Vec3::from_array(self.actor_vector(
                    owner_class,
                    &owner_instance,
                    "Location",
                )?);
                self.set_actor_value(
                    owner_class,
                    &mut owner_instance,
                    "Velocity",
                    Value::Vector(((location - old_location) / elapsed).to_array()),
                )?;
            }
            Ok(())
        })();
        self.instances.insert(owner, owner_instance);
        *manager_instance = self
            .instances
            .remove(&manager)
            .ok_or_else(|| format!("interpolation manager {manager} instance is active"))?;
        result
    }

    fn finish_interpolation_manager_segment(
        &mut self,
        manager: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        forward: bool,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let Some(destination) = self.actor_object(class, instance, "Dest")? else {
            return Ok(());
        };
        let Some(destination) = self.object_actors.get(&destination).copied() else {
            return Ok(());
        };
        let manager_object = self
            .actor_objects
            .get(&manager)
            .cloned()
            .ok_or_else(|| format!("interpolation manager {manager} has no object identity"))?;
        let manager_handle = self
            .object_handle(manager_object)
            .map_err(|error| error.to_string())?;
        let active_manager = std::mem::take(instance);
        self.instances.insert(manager, active_manager);
        let result = self.call_other_actor_event(
            destination,
            "InterpolateEnd",
            vec![Value::Object(manager_handle), Value::Bool(forward)],
            actions,
        );
        *instance = self
            .instances
            .remove(&manager)
            .ok_or_else(|| format!("interpolation manager {manager} instance is active"))?;
        result
    }

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
            let bounce = self.actor_bool(class, instance, "bBounce")?;
            let calls_hit_wall = falling_calls_hit_wall(bounce, hit_pawn, hit.normal);
            if bounce {
                if calls_hit_wall {
                    self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                }
                if self.destroyed.contains(&actor)
                    || self.actor_byte(class, instance, "Physics")? == PHYS_NONE
                {
                    return Ok(());
                }
                let reflected = move_delta.reflect(hit.normal);
                self.try_move_actor(actor, class, reflected.to_array(), instance, actions)?;
                continue;
            }
            if pawn && !hit_pawn {
                self.try_mount(actor, class, instance, hit, actions)?;
            }
            if calls_hit_wall {
                self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
            }
            if hit.normal.z < WALKABLE_FLOOR_Z {
                let mut aligned =
                    (move_delta - hit.normal * move_delta.dot(hit.normal)) * (1.0 - hit.fraction);
                if move_delta.dot(aligned) >= 0.0 {
                    let slope =
                        self.try_move_actor(actor, class, aligned.to_array(), instance, actions)?;
                    if slope.fraction < 1.0 {
                        if slope.normal.z > WALKABLE_FLOOR_Z {
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
                        self.call_hit_wall(
                            actor,
                            class,
                            instance,
                            slope.normal,
                            slope.actor,
                            actions,
                        )?;
                        aligned = two_wall_adjust(
                            aligned,
                            slope.normal,
                            hit.normal,
                            move_delta.normalize_or_zero(),
                            slope.fraction,
                        );
                        let corner = self.try_move_actor(
                            actor,
                            class,
                            aligned.to_array(),
                            instance,
                            actions,
                        )?;
                        if corner.fraction < 1.0 && corner.normal.z > WALKABLE_FLOOR_Z {
                            self.phys_landed(
                                actor,
                                class,
                                instance,
                                corner.normal,
                                corner.actor,
                                actions,
                            )?;
                            return Ok(());
                        }
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

        let (raised, far, diagonal_end) =
            mount_trace_points(location, hit.normal, radius, max_height);
        let ledge = self.single_line_check_between(actor, class, instance, far, diagonal_end)?;
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
        let destination = ledge_location + Vec3::Z * 2.0;
        if self
            .single_line_check_between(actor, class, instance, location, raised)?
            .fraction
            < 1.0
            || self
                .single_line_check_between(actor, class, instance, raised, destination)?
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
            let move_delta = movement_velocity * elapsed;
            let hit =
                self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
            if hit.fraction < 1.0 {
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
                } else {
                    self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                    let aligned = (move_delta - hit.normal * move_delta.dot(hit.normal))
                        * (1.0 - hit.fraction);
                    if move_delta.dot(aligned) >= 0.0 {
                        let slide = self.try_move_actor(
                            actor,
                            class,
                            aligned.to_array(),
                            instance,
                            actions,
                        )?;
                        if slide.fraction < 1.0 {
                            self.call_hit_wall(
                                actor,
                                class,
                                instance,
                                slide.normal,
                                slide.actor,
                                actions,
                            )?;
                            let corner = two_wall_adjust(
                                aligned,
                                slide.normal,
                                hit.normal,
                                move_delta.normalize_or_zero(),
                                slide.fraction,
                            );
                            self.try_move_actor(
                                actor,
                                class,
                                corner.to_array(),
                                instance,
                                actions,
                            )?;
                        }
                    }
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
                    self.set_actor_physics(actor, class, instance, PHYS_FALLING, actions)?;
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
            self.set_actor_physics(actor, class, instance, PHYS_FALLING, actions)?;
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
        if !self
            .class_has_name(class, "Mover")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let old_location = self.actor_vector(class, instance, "Location")?;
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;

        if !self.actor_bool(class, instance, "bInterpolating")? {
            if elapsed > 0.0 {
                self.set_actor_value(class, instance, "Velocity", Value::Vector([0.0; 3]))?;
            }
            return Ok(());
        }

        let mut time_left = elapsed;
        loop {
            if time_left <= 0.0 || !self.actor_bool(class, instance, "bInterpolating")? {
                break;
            }
            let mut gravity_moved = false;
            if self.actor_bool(class, instance, "bCollideWorld")? {
                let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
                if let Some(zone) = self.zone_physics(location, actor, instance)?
                    && zone.gravity.length_squared() > 0.0
                {
                    let velocity =
                        Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
                    let parallel_velocity =
                        zone.gravity * velocity.dot(zone.gravity) / zone.gravity.length_squared();
                    let gravity_delta = parallel_velocity * time_left
                        + zone.gravity * (0.5 * time_left * time_left);
                    self.set_actor_value(
                        class,
                        instance,
                        "Velocity",
                        Value::Vector((velocity + zone.gravity * time_left).to_array()),
                    )?;

                    let hit = self.try_move_actor(
                        actor,
                        class,
                        gravity_delta.to_array(),
                        instance,
                        actions,
                    )?;
                    let moved = Vec3::from_array(self.actor_vector(class, instance, "Location")?)
                        - location;
                    if hit.fraction > 0.0 && moved.length_squared() > 0.0 {
                        gravity_moved = true;
                        let key = usize::from(self.actor_byte(class, instance, "KeyNum")?).min(7);
                        let mut key_positions =
                            self.required_actor_property(class, instance, "KeyPos")?;
                        let StoredValue::Array(values) = &mut key_positions else {
                            return Err(format!("actor property KeyPos is {key_positions:?}"));
                        };
                        let Some(StoredValue::Value(Value::Vector(key_position))) =
                            values.get_mut(key)
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

            let rate = self.actor_float_any(class, instance, "PhysRate")?;
            if rate <= 0.0 {
                break;
            }

            let previous_alpha = self.actor_float_any(class, instance, "PhysAlpha")?;
            let mut alpha = previous_alpha + rate * time_left;
            if alpha > 1.0 {
                time_left = if previous_alpha < 1.0 {
                    (alpha - 1.0) / rate
                } else {
                    0.0
                };
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
            self.record_mover_trace(format!(
                "interpolate actor=#{actor} key={key} alpha={previous_alpha:.6}->{alpha:.6} current={current:?} target={target_position:?} delta={:?}",
                target_position - current,
            ));
            let hit = self.try_move_actor(
                actor,
                class,
                (target_position - current).to_array(),
                instance,
                actions,
            )?;
            self.record_mover_trace(format!(
                "interpolate-result actor=#{actor} fraction={:.6} normal={:?} actor={:?} node={:?}",
                hit.fraction, hit.normal, hit.actor, hit.node,
            ));
            if hit.fraction == 1.0 {
                self.try_move_actor_rotated(actor, class, target_rotation, instance, actions)?;
                self.set_actor_value(class, instance, "PhysAlpha", Value::Float(alpha))?;
                if alpha == 1.0 && !gravity_moved {
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
                if !gravity_moved {
                    self.set_actor_value(class, instance, "bInterpolating", Value::Bool(false))?;
                }
            }
            if !gravity_moved {
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

    pub(in crate::world) fn tick_rotating(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        old_velocity: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let mut rotation = self.actor_rotator(class, instance, "Rotation")?;
        let rate = self.actor_rotator(class, instance, "RotationRate")?;
        let before = rotation;
        let pawn = self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?;
        let physics = self.actor_byte(class, instance, "Physics")?;
        if pawn && rate[2] > 0 {
            let velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
            let acceleration = if physics == PHYS_WALKING && velocity.length_squared() < 40_000.0 {
                Vec3::ZERO
            } else {
                (velocity - old_velocity) / elapsed
            };
            rotation[2] = pawn_roll(
                rotation[2],
                acceleration,
                rotation,
                self.actor_float(class, instance, "AccelRate")?,
                rate[2],
                elapsed,
            );
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

fn mount_trace_points(
    location: Vec3,
    hit_normal: Vec3,
    radius: f32,
    max_height: f32,
) -> (Vec3, Vec3, Vec3) {
    let up = Vec3::Z;
    let inward = Vec3::new(-hit_normal.x, -hit_normal.y, 0.0).normalize_or_zero();
    let raised = location + up * max_height;
    let far = raised + inward * (radius * 2.0 + max_height * hit_normal.z);
    let diagonal_end = far - (up + inward * hit_normal.z) * max_height;
    (raised, far, diagonal_end)
}

#[cfg(test)]
mod tests {
    use super::{falling_calls_hit_wall, mount_trace_points};
    use glam::Vec3;

    #[test]
    fn native_mount_trace_ends_two_radii_inward() {
        let (raised, far, end) =
            mount_trace_points(Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0), 15.0, 96.5);

        assert_eq!(raised, Vec3::new(0.0, 0.0, 96.5));
        assert_eq!(far, Vec3::new(0.0, 30.0, 96.5));
        assert_eq!(end, Vec3::new(0.0, 30.0, 0.0));
    }

    #[test]
    fn falling_hit_wall_matches_native_wall_and_landing_callbacks() {
        assert!(falling_calls_hit_wall(true, true, Vec3::Z));
        assert!(!falling_calls_hit_wall(false, true, Vec3::X));
        assert!(falling_calls_hit_wall(false, false, Vec3::X));
        assert!(!falling_calls_hit_wall(false, false, Vec3::Z));
    }
}
