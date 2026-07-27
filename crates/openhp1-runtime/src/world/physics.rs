use glam::Vec3;

use super::*;

const PHYSICS_STEP: f32 = 0.02;
const PHYS_NONE: u8 = 0;
const PHYS_WALKING: u8 = 1;
const PHYS_FALLING: u8 = 2;
const PHYS_PROJECTILE: u8 = 6;

struct ZonePhysics {
    number: usize,
    gravity: Vec3,
    velocity: Vec3,
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
            if mode == PHYS_PROJECTILE {
                self.tick_projectile(actor, class, instance, elapsed, actions)?;
            } else if mode == PHYS_FALLING {
                self.tick_falling(actor, class, instance, elapsed, actions)?;
            }
            self.tick_rotating(actor, class, instance, elapsed, actions)?;
        }
        Ok(())
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
                self.call_hit_event(
                    actor, class, instance, "HitWall", hit.normal, hit.actor, actions,
                )?;
            }

            if self.actor_bool(class, instance, "bBounce")? {
                let reflected = move_delta.reflect(hit.normal);
                self.try_move_actor(actor, class, reflected.to_array(), instance, actions)?;
                continue;
            }
            if hit.normal.z < 0.7071 {
                let aligned =
                    (move_delta - hit.normal * move_delta.dot(hit.normal)) * (1.0 - hit.fraction);
                if move_delta.dot(aligned) >= 0.0 {
                    let slope =
                        self.try_move_actor(actor, class, aligned.to_array(), instance, actions)?;
                    if slope.fraction < 1.0 && slope.normal.z > 0.7071 {
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

    fn tick_projectile(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let old_location = self.actor_vector(class, instance, "Location")?;
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let acceleration = Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?)
            + acceleration * elapsed;
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
        if !pawn {
            self.set_actor_value(class, instance, "Velocity", Value::Vector([0.0; 3]))?;
        }
        Ok(())
    }

    fn call_hit_event(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        event: &str,
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
            event,
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
}
