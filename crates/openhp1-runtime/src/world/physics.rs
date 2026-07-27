use glam::Vec3;

use super::*;

const PHYSICS_STEP: f32 = 0.02;
const PHYS_NONE: u8 = 0;
const PHYS_PROJECTILE: u8 = 6;

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
            }
            self.tick_rotating(actor, class, instance, elapsed, actions)?;
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
}
