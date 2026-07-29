use super::*;

impl ScriptRuntime {
    pub(super) fn collision_fields(
        &mut self,
        class: &ResolvedObject,
    ) -> std::result::Result<CollisionFields, String> {
        let class_id = object_id(&class.package, class.export_index);
        if let Some(fields) = self.collision_fields.get(&class_id) {
            return Ok(fields.clone());
        }
        let mut field = |name| {
            self.find_property(class, name, 0)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("actor property {name} is missing"))
        };
        let fields = CollisionFields {
            location: field("Location")?,
            height: field("CollisionHeight")?,
            radius: field("CollisionRadius")?,
            width: field("CollisionWidth")?,
            rotation: field("Rotation")?,
            collide_type: field("CollideType")?,
            collide_actors: field("bCollideActors")?,
            block_actors: field("bBlockActors")?,
            block_players: field("bBlockPlayers")?,
            brush: field("Brush")?,
            pre_pivot: field("PrePivot")?,
            main_scale: self
                .find_property(class, "MainScale", 0)
                .map_err(|error| error.to_string())?,
            player_collision: self
                .class_has_name(class, "PlayerPawn")
                .map_err(|error| error.to_string())?
                || self
                    .class_has_name(class, "Projectile")
                    .map_err(|error| error.to_string())?,
        };
        self.collision_fields.insert(class_id, fields.clone());
        Ok(fields)
    }

    pub(super) fn collision_actor_from_fields(
        &mut self,
        actor: usize,
        instance: &InstanceState,
        fields: &CollisionFields,
    ) -> std::result::Result<CollisionActor, String> {
        let brush = collision_actor_brush(instance, fields)?
            .map(|object| self.brush_collision(object))
            .transpose()?;
        collision_actor_from_fields(
            actor,
            instance,
            fields,
            brush,
            self.actor_visual_bounds.get(&actor).copied(),
        )
    }

    pub(super) fn brush_collision(
        &mut self,
        object: ObjectId,
    ) -> std::result::Result<Arc<BspCollision>, String> {
        if let Some(collision) = self.brush_collisions.get(&object) {
            return Ok(Arc::clone(collision));
        }
        let brush = self
            .resolved_object(&object)
            .map_err(|error| error.to_string())?;
        let model = Model::decode(&brush.package, brush.export_index)
            .map_err(|error| format!("could not decode brush model: {error}"))?;
        let collision = Arc::new(
            BspCollision::from_model(&model)
                .map_err(|error| format!("could not build brush collision: {error}"))?,
        );
        self.brush_collisions.insert(object, Arc::clone(&collision));
        Ok(collision)
    }

    pub(super) fn actors_share_base_chain(
        &self,
        first: usize,
        second: usize,
    ) -> DispatchResult<bool> {
        Ok(self.actor_is_based_on(first, second)? || self.actor_is_based_on(second, first)?)
    }

    pub(super) fn actor_is_based_on(&self, mut actor: usize, base: usize) -> DispatchResult<bool> {
        for _ in 0..MAX_CALL_DEPTH {
            let Some(Some(object)) = self.actor_bases.get(&actor) else {
                return Ok(false);
            };
            let Some(next) = self.object_actors.get(object).copied() else {
                return Ok(false);
            };
            if next == base {
                return Ok(true);
            }
            actor = next;
        }
        Err(DispatchError::CallDepth)
    }

    pub(in crate::world) fn class_has_name(
        &mut self,
        class: &ResolvedObject,
        name: &str,
    ) -> DispatchResult<bool> {
        let mut class = ResolvedObject {
            package: Arc::clone(&class.package),
            export_index: class.export_index,
        };
        for _ in 0..MAX_CALL_DEPTH {
            let summary = class.package.summary();
            if summary
                .name(summary.exports[class.export_index].object_name)
                .eq_ignore_ascii_case(name)
            {
                return Ok(true);
            }
            let Some(base) = self.base_class(&class)? else {
                return Ok(false);
            };
            class = base;
        }
        Err(DispatchError::CallDepth)
    }

    pub(in crate::world) fn required_actor_property(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<StoredValue, String> {
        self.instance_property(class, instance, name)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor property {name} is missing"))
    }

    pub(in crate::world) fn actor_bool(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<bool, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Bool(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn actor_float(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<f32, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Float(value)) if value.is_finite() && value >= 0.0 => {
                Ok(value)
            }
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn actor_vector(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<[f32; 3], String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Vector(value))
                if value.iter().all(|component| component.is_finite()) =>
            {
                Ok(value)
            }
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn actor_object(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<Option<ObjectId>, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Object(value) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn actor_byte(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<u8, String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Byte(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }

    pub(in crate::world) fn actor_rotator(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<[i32; 3], String> {
        match self.required_actor_property(class, instance, name)? {
            StoredValue::Value(Value::Rotator(value)) => Ok(value),
            value => Err(format!("actor property {name} is {value:?}")),
        }
    }
}

impl CollisionFields {
    pub(super) fn contains(&self, field: &ObjectId) -> bool {
        [
            &self.location,
            &self.height,
            &self.radius,
            &self.width,
            &self.rotation,
            &self.collide_type,
            &self.collide_actors,
            &self.block_actors,
            &self.block_players,
            &self.brush,
            &self.pre_pivot,
        ]
        .contains(&field)
            || self.main_scale.as_ref() == Some(field)
    }
}
