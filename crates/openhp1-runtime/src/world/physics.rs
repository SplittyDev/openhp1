use glam::Vec3;

use super::*;

const PHYSICS_STEP: f32 = 0.02;
const PHYS_NONE: u8 = 0;
const PHYS_WALKING: u8 = 1;
const PHYS_FALLING: u8 = 2;
const PHYS_SWIMMING: u8 = 3;
const PHYS_FLYING: u8 = 4;
const PHYS_PROJECTILE: u8 = 6;
const PHYS_ROLLING: u8 = 7;
const PHYS_INTERPOLATING: u8 = 8;
const PHYS_MOVING_BRUSH: u8 = 9;
const PHYS_TRAILER: u8 = 11;
const STEP_DOWN_FACTOR: f32 = 1.3;
const WALKABLE_FLOOR_Z: f32 = 7071.0 / 10_000.0;

struct ZonePhysics {
    number: usize,
    gravity: Vec3,
    velocity: Vec3,
    ground_friction: f32,
    fluid_friction: f32,
    terminal_velocity: f32,
    water: bool,
}

impl ScriptRuntime {
    pub(super) fn tick_physics(
        &mut self,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        for actor in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
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
            let result = self.tick_actor_physics(actor, &class, &mut instance, delta_time, actions);
            self.instances.insert(actor, instance);
            if let Err(message) = result {
                actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Physics: {message}"),
                });
            }
        }
        Ok(())
    }

    fn tick_actor_physics(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let mut time_left = delta_time;
        while time_left > 0.0 && !self.destroyed.contains(&actor) {
            let elapsed = time_left.min(PHYSICS_STEP);
            time_left -= PHYSICS_STEP;
            let mode = self.actor_byte(class, instance, "Physics")?;
            if mode == PHYS_NONE {
                continue;
            }
            match mode {
                PHYS_WALKING => {
                    self.tick_walking(actor, class, instance, elapsed, actions)?;
                }
                PHYS_FALLING => {
                    self.tick_falling(actor, class, instance, elapsed, actions)?;
                }
                PHYS_SWIMMING => {
                    self.tick_flying(actor, class, instance, elapsed, actions, true)?;
                }
                PHYS_FLYING => {
                    self.tick_flying(actor, class, instance, elapsed, actions, false)?;
                }
                PHYS_PROJECTILE => {
                    self.tick_projectile(actor, class, instance, elapsed, actions)?;
                }
                PHYS_ROLLING => {
                    self.tick_rolling(actor, class, instance, elapsed, actions)?;
                }
                PHYS_INTERPOLATING => {
                    self.tick_interpolating(actor, class, instance, elapsed, actions)?;
                }
                PHYS_MOVING_BRUSH => {
                    self.tick_moving_brush(actor, class, instance, elapsed, actions)?;
                }
                PHYS_TRAILER => {
                    self.tick_trailer(actor, class, instance, actions)?;
                }
                _ => {}
            }
            self.tick_rotating(actor, class, instance, elapsed, actions)?;
        }
        Ok(())
    }

    fn tick_walking(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if !self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let old_location = self.actor_vector(class, instance, "Location")?;
        let zone = self.zone_physics(Vec3::from_array(old_location), actor, instance)?;
        if zone.number == 0 {
            self.call_actor_event(
                actor,
                class,
                instance,
                "FellOutOfWorld",
                Vec::new(),
                actions,
            )?;
            return Ok(());
        }
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let player = self
            .class_has_name(class, "PlayerPawn")
            .map_err(|error| error.to_string())?;
        let player_walking = player
            && self
                .optional_actor_bool(class, instance, "bIsWalking")?
                .unwrap_or(false);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        velocity.z = 0.0;
        let mut acceleration =
            Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        if acceleration.length_squared() > 0.0001 {
            let mut acceleration_rate = self.actor_float(class, instance, "AccelRate")?;
            if player_walking {
                acceleration_rate *= 0.3;
            }
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
                (velocity - acceleration_direction * speed) * (zone.ground_friction * elapsed);
        } else {
            let speed = velocity.length();
            if speed > 0.0 {
                let new_speed = (speed - speed * zone.ground_friction * 2.0 * elapsed).max(0.0);
                velocity *= new_speed / speed;
            }
        }
        velocity += acceleration * elapsed;

        let mut maximum_speed = self.actor_float(class, instance, "GroundSpeed")?;
        if !player {
            maximum_speed *= self.actor_float(class, instance, "DesiredSpeed")?;
        } else if player_walking {
            maximum_speed *= 0.3;
        }
        let speed = velocity.length();
        if speed > maximum_speed && speed > 0.0 {
            velocity *= maximum_speed / speed;
        }
        velocity.z = 0.0;
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let gravity_direction = if zone.gravity.z > 0.0 { 1.0 } else { -1.0 };
        let step_height = self.actor_float(class, instance, "MaxStepHeight")?;
        let step_up = Vec3::new(0.0, 0.0, -gravity_direction * step_height);
        let step_down = Vec3::new(0.0, 0.0, gravity_direction * step_height * STEP_DOWN_FACTOR);
        let movement_velocity = velocity + zone.velocity * (elapsed * 25.0);
        if movement_velocity.x != 0.0 && movement_velocity.y != 0.0 {
            let mut time_left = elapsed;
            for _ in 0..5 {
                if time_left <= 0.0 {
                    break;
                }
                let mut move_delta = movement_velocity * time_left;
                self.try_move_actor(actor, class, step_up.to_array(), instance, actions)?;
                let mut hit =
                    self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
                time_left -= time_left * hit.fraction;
                move_delta = movement_velocity * time_left;
                self.try_move_actor(actor, class, (-step_up).to_array(), instance, actions)?;

                if hit.fraction < f32::EPSILON {
                    hit = self.try_move_actor(
                        actor,
                        class,
                        move_delta.to_array(),
                        instance,
                        actions,
                    )?;
                    time_left -= time_left * hit.fraction;
                }
                if hit.fraction < 1.0 {
                    if let (true, Some(other)) = (player, hit.actor) {
                        if self
                            .actor_has_class(other, "Decoration")
                            .map_err(|error| error.to_string())?
                            && self.other_actor_bool(other, "bPushable")?
                            && hit.normal.dot(move_delta) < -0.9
                        {
                            let mass = self.actor_float(class, instance, "Mass")?;
                            let other_mass = self.other_actor_float(other, "Mass")?;
                            self.set_actor_value(
                                class,
                                instance,
                                "bJustTeleported",
                                Value::Bool(true),
                            )?;
                            velocity *= mass / (mass + other_mass);
                            self.set_actor_value(
                                class,
                                instance,
                                "Velocity",
                                Value::Vector(velocity.to_array()),
                            )?;
                            self.call_hit_wall(
                                actor, class, instance, hit.normal, hit.actor, actions,
                            )?;
                            time_left = 0.0;
                        }
                    } else if hit.normal.z < 0.2 && hit.normal.z > -0.2 {
                        self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                        let aligned = (move_delta - hit.normal * move_delta.dot(hit.normal))
                            * (1.0 - hit.fraction);
                        if move_delta.dot(aligned) >= 0.0 {
                            hit = self.try_move_actor(
                                actor,
                                class,
                                aligned.to_array(),
                                instance,
                                actions,
                            )?;
                            time_left -= time_left * hit.fraction;
                            if hit.fraction < 1.0 {
                                self.call_hit_wall(
                                    actor, class, instance, hit.normal, hit.actor, actions,
                                )?;
                            }
                        } else {
                            time_left = 0.0;
                        }
                    }
                }

                if self.actor_byte(class, instance, "Physics")? != PHYS_WALKING {
                    return Ok(());
                }
                if !self.walk_to_floor(actor, class, instance, step_down, actions)? {
                    return Ok(());
                }
            }
        } else {
            // ponytail: the BSP is static, so recheck an idle world-supported pawn
            // only after its location changes; moving bases will invalidate this.
            if self.grounded_world.get(&actor) != Some(&old_location) {
                let floor = self.test_move_actor(actor, class, step_down.to_array(), instance)?;
                if floor.fraction == 1.0 || floor.normal.z < WALKABLE_FLOOR_Z {
                    self.grounded_world.remove(&actor);
                    self.start_falling(class, instance)?;
                } else if floor.actor.is_none() {
                    self.grounded_world.insert(actor, old_location);
                }
            }
        }

        if !self.actor_bool(class, instance, "bJustTeleported")? {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            velocity = (location - Vec3::from_array(old_location)) / elapsed;
        }
        velocity.z = 0.0;
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;
        Ok(())
    }

    fn walk_to_floor(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        step_down: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        let floor = self.test_move_actor(actor, class, step_down.to_array(), instance)?;
        if floor.fraction == 1.0 || floor.normal.z < WALKABLE_FLOOR_Z {
            self.grounded_world.remove(&actor);
            self.start_falling(class, instance)?;
            return Ok(false);
        }
        let floor = self.try_move_actor(actor, class, step_down.to_array(), instance, actions)?;
        if floor.fraction != 1.0 {
            let base = floor
                .actor
                .and_then(|actor| self.actor_objects.get(&actor).cloned());
            self.set_actor_stored(class, instance, "Base", StoredValue::Object(base))?;
        }
        Ok(true)
    }

    fn start_falling(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<(), String> {
        self.set_actor_value(class, instance, "Physics", Value::Byte(PHYS_FALLING))?;
        self.set_actor_stored(class, instance, "Base", StoredValue::Object(None))
    }

    fn tick_falling(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let zone = self.zone_physics(Vec3::from_array(old_location), actor, instance)?;
        if zone.number == 0 {
            self.call_actor_event(
                actor,
                class,
                instance,
                "FellOutOfWorld",
                Vec::new(),
                actions,
            )?;
            return Ok(());
        }
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

    fn tick_flying(
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
        let zone = self.zone_physics(Vec3::from_array(old_location), actor, instance)?;
        if zone.number == 0 {
            self.call_actor_event(
                actor,
                class,
                instance,
                "FellOutOfWorld",
                Vec::new(),
                actions,
            )?;
            return Ok(());
        }
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
        if movement_velocity.x != 0.0 && movement_velocity.y != 0.0 {
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
            if !new_zone.water {
                if velocity.z > 0.0 {
                    velocity.z = velocity.z.max((100.0 + velocity.truncate().length()) * 0.5);
                }
                if self.actor_byte(class, instance, "Physics")? == PHYS_SWIMMING {
                    self.set_actor_value(class, instance, "Physics", Value::Byte(PHYS_FALLING))?;
                }
            }
        } else {
            velocity.z = 0.0;
        }
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;
        Ok(())
    }

    fn tick_projectile(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let zone = self.zone_physics(Vec3::from_array(old_location), actor, instance)?;
        if zone.number == 0 {
            self.native(
                actor,
                class,
                &Arc::clone(&class.package),
                DESTROY,
                &[],
                instance,
                actions,
            )?;
            return Ok(());
        }
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

    fn tick_rolling(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        let zone = self.zone_physics(Vec3::from_array(old_location), actor, instance)?;
        if zone.number == 0 {
            self.call_actor_event(
                actor,
                class,
                instance,
                "FellOutOfWorld",
                Vec::new(),
                actions,
            )?;
            return Ok(());
        }
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
            self.set_actor_stored(class, instance, "Base", StoredValue::Object(None))?;
        } else {
            let floor =
                self.try_move_actor(actor, class, step_down.to_array(), instance, actions)?;
            if floor.fraction != 1.0 {
                let base = floor
                    .actor
                    .and_then(|actor| self.actor_objects.get(&actor).cloned());
                self.set_actor_stored(class, instance, "Base", StoredValue::Object(base))?;
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

    fn tick_trailer(
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

    fn tick_interpolating(
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

    fn tick_moving_brush(
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

        let mut time_left = elapsed;
        while time_left > 0.0 {
            if !self.actor_bool(class, instance, "bInterpolating")? {
                break;
            }
            let rate = self.actor_float_any(class, instance, "PhysRate")?;
            if rate <= 0.0 {
                break;
            }

            let mut alpha = self.actor_float_any(class, instance, "PhysAlpha")?;
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

    fn tick_rotating(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let rotate_to_desired = self.actor_bool(class, instance, "bRotateToDesired")?;
        let fixed_direction = self.actor_bool(class, instance, "bFixedRotationDir")?;
        let mut rotation = self.actor_rotator(class, instance, "Rotation")?;
        let rate = self.actor_rotator(class, instance, "RotationRate")?;
        let before = rotation;

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

        if rotation != before {
            self.set_actor_value(class, instance, "Rotation", Value::Rotator(rotation))?;
            actions.push(ActorAction::SetRotation { actor, rotation });
        }
        Ok(())
    }

    fn phys_landed(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        normal: Vec3,
        hit_actor: Option<usize>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        self.call_actor_event(
            actor,
            class,
            instance,
            "Landed",
            vec![Value::Vector(normal.to_array())],
            actions,
        )?;
        if self.actor_byte(class, instance, "Physics")? != PHYS_FALLING {
            return Ok(());
        }
        let pawn = self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?;
        self.set_actor_value(
            class,
            instance,
            "Physics",
            Value::Byte(if pawn { PHYS_WALKING } else { PHYS_NONE }),
        )?;
        let base = hit_actor.and_then(|actor| self.actor_objects.get(&actor).cloned());
        self.set_actor_stored(class, instance, "Base", StoredValue::Object(base))?;
        if pawn && hit_actor.is_none() {
            let location = self.actor_vector(class, instance, "Location")?;
            self.grounded_world.insert(actor, location);
        }
        if !pawn {
            self.set_actor_value(class, instance, "Velocity", Value::Vector([0.0; 3]))?;
        }
        Ok(())
    }

    fn call_hit_wall(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        normal: Vec3,
        hit_actor: Option<usize>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let object = match hit_actor {
            Some(actor) => self.actor_objects.get(&actor).cloned(),
            None => self.actor_object(class, instance, "Level")?,
        };
        let handle = match object {
            Some(object) => self
                .object_handle(object)
                .map_err(|error| error.to_string())?,
            None => 0,
        };
        self.call_actor_event(
            actor,
            class,
            instance,
            "HitWall",
            vec![Value::Vector(normal.to_array()), Value::Object(handle)],
            actions,
        )
    }

    fn call_actor_event(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        event: &str,
        arguments: Vec<Value>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if self.instances.contains_key(&actor) {
            return Err(DispatchError::ActiveActorContext { actor }.to_string());
        }
        let active = std::mem::take(instance);
        self.instances.insert(actor, active);
        let result = self.dispatch_event_with_arguments(
            actor,
            Path::new(class.package.summary().source.as_ref()),
            class.export_index,
            event,
            &arguments,
        );
        *instance = self
            .instances
            .remove(&actor)
            .ok_or_else(|| DispatchError::ActiveActorContext { actor }.to_string())?;
        match result {
            Ok(event_actions) => actions.extend(event_actions),
            Err(error) => actions.push(ActorAction::DeferredCall {
                actor,
                message: format!("{event}: {error}"),
            }),
        }
        Ok(())
    }

    fn zone_physics(
        &mut self,
        location: Vec3,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<ZonePhysics, String> {
        let collision = self
            .collision
            .as_ref()
            .ok_or_else(|| "physics requires a configured BSP collision model".to_owned())?;
        let zone = collision.zone_at(location);
        let zone_actor = collision
            .zone_actor_export(zone)
            .and_then(|export_index| {
                self.level_package.as_ref().and_then(|package| {
                    self.object_actors
                        .get(&ObjectId {
                            package: Arc::clone(package),
                            export_index,
                        })
                        .copied()
                })
            })
            .or(self.level_info)
            .ok_or_else(|| format!("zone {zone} has no registered ZoneInfo or LevelInfo"))?;
        let class = self
            .actor_classes
            .get(&zone_actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: zone_actor })
            .map_err(|error| error.to_string())?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let instance = if zone_actor == current_actor {
            current_instance.clone()
        } else {
            self.instances
                .get(&zone_actor)
                .cloned()
                .ok_or_else(|| format!("zone actor {zone_actor} instance is active"))?
        };
        Ok(ZonePhysics {
            number: zone,
            gravity: Vec3::from_array(self.actor_vector(&class, &instance, "ZoneGravity")?),
            velocity: Vec3::from_array(self.actor_vector(&class, &instance, "ZoneVelocity")?),
            ground_friction: self.actor_float(&class, &instance, "ZoneGroundFriction")?,
            fluid_friction: self.actor_float(&class, &instance, "ZoneFluidFriction")?,
            terminal_velocity: self.actor_float(&class, &instance, "ZoneTerminalVelocity")?,
            water: self.actor_bool(&class, &instance, "bWaterZone")?,
        })
    }

    fn actor_has_class(&mut self, actor: usize, name: &str) -> DispatchResult<bool> {
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(false);
        };
        let class = self.resolved_object(&class)?;
        self.class_has_name(&class, name)
    }

    fn other_actor_property(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<StoredValue, String> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })
            .map_err(|error| error.to_string())?;
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("actor {actor} instance is active"))?;
        self.required_actor_property(&class, &instance, name)
    }

    fn other_actor_bool(&mut self, actor: usize, name: &str) -> std::result::Result<bool, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Bool(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn other_actor_float(&mut self, actor: usize, name: &str) -> std::result::Result<f32, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Float(value)) if value.is_finite() && value >= 0.0 => {
                Ok(value)
            }
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn actor_float_any(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<f32, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Float(value)) if value.is_finite() => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn other_actor_object(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<Option<ObjectId>, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Object(value) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn other_actor_vector(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<[f32; 3], String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Vector(value))
                if value.iter().all(|component| component.is_finite()) =>
            {
                Ok(value)
            }
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn other_actor_rotator(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<[i32; 3], String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Rotator(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn set_actor_value(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        name: &str,
        value: Value,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(class, name, 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor property {name} is missing"))?;
        instance.insert(field, StoredValue::Value(value));
        Ok(())
    }

    fn set_actor_stored(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        name: &str,
        value: StoredValue,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(class, name, 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor property {name} is missing"))?;
        instance.insert(field, value);
        Ok(())
    }

    fn actor_array_vector(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
        index: usize,
    ) -> std::result::Result<[f32; 3], String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Array(values) => match values.get(index) {
                Some(StoredValue::Value(Value::Vector(value)))
                    if value.iter().all(|component| component.is_finite()) =>
                {
                    Ok(*value)
                }
                Some(value) => Err(format!("actor property {name}[{index}] is {value:?}")),
                None => Err(format!(
                    "actor property {name} has {} elements, requested {index}",
                    values.len()
                )),
            },
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn actor_array_rotator(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
        index: usize,
    ) -> std::result::Result<[i32; 3], String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Array(values) => match values.get(index) {
                Some(StoredValue::Value(Value::Rotator(value))) => Ok(*value),
                Some(value) => Err(format!("actor property {name}[{index}] is {value:?}")),
                None => Err(format!(
                    "actor property {name} has {} elements, requested {index}",
                    values.len()
                )),
            },
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    fn optional_actor_float(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<Option<f32>, String> {
        match self
            .instance_property(class, instance, name)
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Float(value))) if value.is_finite() && value >= 0.0 => {
                Ok(Some(value))
            }
            Some(value) => Err(format!("actor property {name} is {value:?}")),
            None => Ok(None),
        }
    }

    fn optional_actor_bool(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<Option<bool>, String> {
        match self
            .instance_property(class, instance, name)
            .map_err(|error| error.to_string())?
        {
            Some(StoredValue::Value(Value::Bool(value))) => Ok(Some(value)),
            Some(value) => Err(format!("actor property {name} is {value:?}")),
            None => Ok(None),
        }
    }

    fn call_other_actor_event(
        &mut self,
        actor: usize,
        event: &str,
        arguments: Vec<Value>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })
            .map_err(|error| error.to_string())?;
        match self.dispatch_event_with_arguments(
            actor,
            Path::new(class.package.as_ref()),
            class.export_index,
            event,
            &arguments,
        ) {
            Ok(event_actions) => actions.extend(event_actions),
            Err(error) => actions.push(ActorAction::DeferredCall {
                actor,
                message: format!("{event}: {error}"),
            }),
        }
        Ok(())
    }
}

fn spline_weight(value: f32) -> f32 {
    let squared = value * value;
    squared * squared * (1.0 / 16.0) - squared * 0.5 + 1.0
}

fn spline_weights(alpha: f32) -> [f32; 4] {
    let weights = [
        spline_weight(alpha + 1.0),
        spline_weight(alpha),
        spline_weight(alpha - 1.0),
        spline_weight(alpha - 2.0),
    ];
    let inverse = weights.iter().sum::<f32>().recip();
    weights.map(|weight| weight * inverse)
}

fn spline_vector(first: Vec3, second: Vec3, third: Vec3, fourth: Vec3, alpha: f32) -> Vec3 {
    let [first_weight, second_weight, third_weight, fourth_weight] = spline_weights(alpha);
    first * first_weight + second * second_weight + third * third_weight + fourth * fourth_weight
}

fn lerp_rotator(first: [i32; 3], second: [i32; 3], alpha: f32) -> [i32; 3] {
    std::array::from_fn(|index| {
        (first[index] as f32 * (1.0 - alpha)) as i32 + (second[index] as f32 * alpha) as i32
    })
}

fn spline_rotator(
    first: [i32; 3],
    second: [i32; 3],
    third: [i32; 3],
    fourth: [i32; 3],
    alpha: f32,
) -> [i32; 3] {
    let weights = [
        spline_weight(alpha + 1.0),
        spline_weight(alpha),
        spline_weight(alpha - 1.0),
        spline_weight(alpha - 2.0),
    ];
    let inverse = weights.iter().sum::<f32>().recip();
    std::array::from_fn(|index| {
        let weighted = (first[index] as f32 * weights[0]) as i32
            + (second[index] as f32 * weights[1]) as i32
            + (third[index] as f32 * weights[2]) as i32
            + (fourth[index] as f32 * weights[3]) as i32;
        (weighted as f32 * inverse) as i32
    })
}

fn mover_rotation(old: [i32; 3], base: [i32; 3], key: [i32; 3], blend: f32) -> [i32; 3] {
    std::array::from_fn(|index| {
        let delta = base[index]
            .wrapping_add(key[index])
            .wrapping_sub(old[index]);
        old[index].wrapping_add((delta as f32 * blend) as i32)
    })
}

fn fall_velocity(
    old_velocity: Vec3,
    acceleration: Vec3,
    gravity: Vec3,
    gravity_scale: f32,
    fluid_friction: f32,
    elapsed: f32,
) -> Vec3 {
    old_velocity * (1.0 - fluid_friction * elapsed)
        + (acceleration * 1.5 + gravity * gravity_scale) * (0.5 * elapsed)
}

fn rotators_equal(left: [i32; 3], right: [i32; 3]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left as u16 == right as u16)
}

fn turn_to_shortest(from: i32, to: i32, speed: i32) -> i32 {
    let from = from & 0xffff;
    let to = to & 0xffff;
    if from > to {
        if from - to < 0x8000 {
            (from - (from - to).min(speed)) & 0xffff
        } else {
            (from + (to + 0x10000 - from).min(speed)) & 0xffff
        }
    } else if to - from < 0x8000 {
        (from + (to - from).min(speed)) & 0xffff
    } else {
        (from - (from + 0x10000 - to).min(speed)) & 0xffff
    }
}

fn turn_to_fixed(from: i32, to: i32, direction: i32) -> i32 {
    let from = from & 0xffff;
    let to = to & 0xffff;
    if direction > 0 {
        if from > to {
            (from + direction.min(to - from + 0x10000)) & 0xffff
        } else {
            (from + direction.min(to - from)) & 0xffff
        }
    } else if from < to {
        (from + direction.max(to - from - 0x10000)) & 0xffff
    } else {
        (from + direction.max(to - from)) & 0xffff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotator_turning_wraps_like_ue1() {
        assert_eq!(turn_to_shortest(65_000, 100, 200), 65_200);
        assert_eq!(turn_to_shortest(100, 65_000, 200), 65_436);
        assert_eq!(turn_to_fixed(65_000, 100, 1_000), 100);
        assert_eq!(turn_to_fixed(100, 65_000, -1_000), 65_000);
        assert!(rotators_equal([-1, 0, 65_536], [65_535, 0, 0]));
    }

    #[test]
    fn falling_applies_ue1_half_step_gravity_and_fluid_drag() {
        assert_eq!(
            fall_velocity(
                Vec3::new(10.0, 0.0, -10.0),
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, -512.0),
                2.0,
                1.0,
                0.02,
            ),
            Vec3::new(9.8, 0.0, -20.04)
        );
    }

    #[test]
    fn interpolation_spline_matches_ue1_weighting() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
        assert!(
            spline_vector(points[0], points[1], points[2], points[3], 0.0)
                .abs_diff_eq(Vec3::new(8.0 / 17.0, 4.5 / 17.0, 0.0), 0.0001)
        );
        assert!(
            spline_vector(points[0], points[1], points[2], points[3], 1.0)
                .abs_diff_eq(Vec3::new(4.5 / 17.0, 8.0 / 17.0, 4.5 / 17.0), 0.0001)
        );
    }

    #[test]
    fn mover_rotation_uses_wrapping_ue1_rotator_math() {
        assert_eq!(
            mover_rotation([100, -100, i32::MAX], [300, 100, i32::MIN], [0; 3], 0.5),
            [200, 0, i32::MAX]
        );
    }
}
