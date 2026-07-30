use super::*;

impl ScriptRuntime {
    pub(in crate::world) fn phys_landed(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        normal: Vec3,
        hit_actor: Option<usize>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if let Err(message) = self.call_actor_event(
            actor,
            class,
            instance,
            "Landed",
            vec![Value::Vector(normal.to_array())],
            actions,
        ) {
            actions.push(ActorAction::DeferredCall {
                actor,
                message: format!("Landed: {message}"),
            });
        }
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
        self.set_actor_base(actor, class, instance, base, actions)?;
        if !pawn {
            self.set_actor_value(class, instance, "Velocity", Value::Vector([0.0; 3]))?;
        }
        Ok(())
    }

    pub(super) fn call_hit_wall(
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

    pub(in crate::world) fn call_actor_event(
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

    pub(in crate::world) fn zone_physics(
        &mut self,
        location: Vec3,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Option<ZonePhysics>, String> {
        let collision = self
            .collision
            .as_ref()
            .ok_or_else(|| "physics requires a configured BSP collision model".to_owned())?;
        let zone_actor = zone_actor_at(
            collision,
            location,
            self.level_package.as_ref(),
            &self.object_actors,
            self.level_info,
        )
        .ok_or_else(|| "location has no registered ZoneInfo or LevelInfo".to_owned())?;
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
        Ok(Some(ZonePhysics {
            gravity: Vec3::from_array(self.actor_vector(&class, &instance, "ZoneGravity")?),
            velocity: Vec3::from_array(self.actor_vector(&class, &instance, "ZoneVelocity")?),
            ground_friction: self.actor_float(&class, &instance, "ZoneGroundFriction")?,
            fluid_friction: self.actor_float(&class, &instance, "ZoneFluidFriction")?,
            terminal_velocity: self.actor_float(&class, &instance, "ZoneTerminalVelocity")?,
            water: self.actor_bool(&class, &instance, "bWaterZone")?,
        }))
    }

    pub(super) fn actor_has_class(&mut self, actor: usize, name: &str) -> DispatchResult<bool> {
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(false);
        };
        let class = self.resolved_object(&class)?;
        self.class_has_name(&class, name)
    }

    pub(super) fn other_actor_property(
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

    pub(super) fn other_actor_bool(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<bool, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Bool(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(super) fn other_actor_float(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<f32, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Float(value)) if value.is_finite() && value >= 0.0 => {
                Ok(value)
            }
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(super) fn actor_float_any(
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

    pub(super) fn other_actor_object(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<Option<ObjectId>, String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Object(value) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn other_actor_vector(
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

    pub(super) fn other_actor_rotator(
        &mut self,
        actor: usize,
        name: &str,
    ) -> std::result::Result<[i32; 3], String> {
        match self.other_actor_property(actor, name)? {
            StoredValue::Value(Value::Rotator(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn set_actor_value(
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

    pub(in crate::world) fn set_actor_stored(
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

    pub(super) fn actor_array_vector(
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

    pub(super) fn actor_array_rotator(
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

    pub(super) fn optional_actor_float(
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

    pub(super) fn optional_actor_bool(
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

    pub(in crate::world) fn call_other_actor_event(
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
