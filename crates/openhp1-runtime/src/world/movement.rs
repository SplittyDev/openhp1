use glam::Vec3;
use openhp1_physics::{cylinders_overlap, sweep_cylinder};

use super::*;

#[derive(Clone)]
struct CollisionActor {
    actor: usize,
    location: Vec3,
    height: f32,
    radius: f32,
    collide_actors: bool,
    block_actors: bool,
    block_players: bool,
    player_collision: bool,
    has_brush: bool,
}

pub(super) struct CachedCollisionActor {
    actor: CollisionActor,
    fields: CollisionFields,
}

struct ActorSweep {
    actor: usize,
    fraction: f32,
    normal: Vec3,
    blocking: bool,
}

#[derive(Clone)]
pub(super) struct CollisionFields {
    location: ObjectId,
    height: ObjectId,
    radius: ObjectId,
    collide_actors: ObjectId,
    block_actors: ObjectId,
    block_players: ObjectId,
    brush: ObjectId,
    player_collision: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MovementHit {
    pub fraction: f32,
    pub normal: Vec3,
    pub actor: Option<usize>,
}

impl ScriptRuntime {
    pub(super) fn move_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        let hit = self.try_move_actor(actor, actor_class, delta, instance, actions)?;
        Ok(Value::Bool(hit.fraction == 1.0))
    }

    pub(super) fn try_move_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<MovementHit, String> {
        self.try_move_actor_inner(actor, actor_class, delta, instance, Some(actions))
    }

    pub(super) fn test_move_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &InstanceState,
    ) -> std::result::Result<MovementHit, String> {
        let mut instance = instance.clone();
        self.try_move_actor_inner(actor, actor_class, delta, &mut instance, None)
    }

    fn try_move_actor_inner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &mut InstanceState,
        mut actions: Option<&mut Vec<ActorAction>>,
    ) -> std::result::Result<MovementHit, String> {
        if self.actor_bool(actor_class, instance, "bStatic")?
            || !self.actor_bool(actor_class, instance, "bMovable")?
        {
            return Ok(MovementHit {
                fraction: 0.0,
                normal: Vec3::ZERO,
                actor: None,
            });
        }

        let delta = Vec3::from_array(delta);
        if delta.length_squared() < 0.00000001 {
            return Ok(MovementHit {
                fraction: 1.0,
                normal: Vec3::ZERO,
                actor: None,
            });
        }
        let current = self.collision_actor(actor, actor_class, instance)?;
        let collide_world = self.actor_bool(actor_class, instance, "bCollideWorld")?;
        let world_hit = if collide_world && !current.has_brush {
            self.collision
                .as_ref()
                .ok_or_else(|| "Move requires a configured BSP collision model".to_owned())?
                .sweep_aabb(
                    current.location,
                    current.location + delta,
                    Vec3::new(current.radius, current.radius, current.height),
                )
        } else {
            None
        };
        let mut blocking_hit = if let Some(hit) = world_hit {
            MovementHit {
                fraction: hit.fraction,
                normal: hit.normal,
                actor: None,
            }
        } else {
            MovementHit {
                fraction: 1.0,
                normal: Vec3::ZERO,
                actor: None,
            }
        };

        let mut hits = if current.collide_actors && !current.has_brush {
            self.actor_sweeps(&current, delta, actor, instance)?
        } else {
            Vec::new()
        };
        hits.sort_by(|left, right| {
            left.fraction
                .total_cmp(&right.fraction)
                .then_with(|| left.actor.cmp(&right.actor))
        });
        let blocking_actor = hits
            .iter()
            .find(|hit| hit.blocking && hit.fraction < blocking_hit.fraction)
            .map(|hit| {
                blocking_hit = MovementHit {
                    fraction: hit.fraction,
                    normal: hit.normal,
                    actor: Some(hit.actor),
                };
                hit.actor
            });

        let Some(actions) = actions.as_mut() else {
            return Ok(blocking_hit);
        };
        let location = current.location + delta * blocking_hit.fraction;
        self.set_actor_location(actor, actor_class, instance, location, actions)?;
        let actually_moved = delta * blocking_hit.fraction;
        for based_actor in self.based_actors(actor)? {
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
            let result = self.try_move_actor(
                based_actor,
                &class,
                actually_moved.to_array(),
                &mut based_instance,
                actions,
            );
            self.instances.insert(based_actor, based_instance);
            result?;
        }

        if let Some(other) = blocking_actor
            && !self
                .actors_share_base_chain(actor, other, actor, instance)
                .map_err(|error| error.to_string())?
        {
            self.queue_pair_event(actions, other, actor, "Bump")?;
            self.queue_pair_event(actions, actor, other, "Bump")?;
        }
        for hit in hits
            .iter()
            .take_while(|hit| hit.fraction < blocking_hit.fraction)
            .filter(|hit| !hit.blocking)
        {
            let pair = actor_pair(actor, hit.actor);
            if self.touching.insert(pair) {
                self.queue_pair_event(actions, actor, hit.actor, "Touch")?;
                self.queue_pair_event(actions, hit.actor, actor, "Touch")?;
            }
        }
        self.queue_ended_touches(actor, &current, location, instance, actions)?;

        Ok(blocking_hit)
    }

    fn based_actors(&self, actor: usize) -> std::result::Result<Vec<usize>, String> {
        let object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
        let mut based = self
            .actor_bases
            .iter()
            .filter(|(candidate, base)| {
                **candidate != actor
                    && !self.destroyed.contains(candidate)
                    && base.as_ref() == Some(&object)
            })
            .map(|(&candidate, _)| candidate)
            .collect::<Vec<_>>();
        based.sort_unstable();
        Ok(based)
    }

    pub(super) fn set_actor_location(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        location: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(actor_class, "Location", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "actor property Location is missing".to_owned())?;
        instance.insert(
            field,
            StoredValue::Value(Value::Vector(location.to_array())),
        );
        if let Some(Some(cached)) = self.collision_actors.get_mut(actor) {
            cached.actor.location = location;
        }
        actions.push(ActorAction::SetLocation {
            actor,
            location: location.to_array(),
        });
        Ok(())
    }

    pub(super) fn set_actor_base(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        base: Option<ObjectId>,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(actor_class, "Base", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "actor property Base is missing".to_owned())?;
        let current = match instance.get(&field) {
            Some(StoredValue::Object(value)) => value.clone(),
            Some(value) => return Err(format!("actor property Base is {value:?}")),
            None => None,
        };
        if current == base {
            return Ok(());
        }
        if let Some(base_actor) = base
            .as_ref()
            .and_then(|object| self.object_actors.get(object))
            .copied()
            && (base_actor == actor
                || self
                    .actor_is_based_on(base_actor, actor, actor, instance)
                    .map_err(|error| error.to_string())?)
        {
            return Ok(());
        }

        let level = self.actor_object(actor_class, instance, "Level")?;
        let actor_object = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("runtime actor {actor} has no object identity"))?;
        let actor_handle = self
            .object_handle(actor_object)
            .map_err(|error| error.to_string())?;
        if let Some(old_base) = current
            .as_ref()
            .filter(|old_base| Some(*old_base) != level.as_ref())
            .and_then(|object| self.object_actors.get(object))
            .copied()
        {
            self.call_other_actor_event(
                old_base,
                "Detach",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }

        instance.insert(field, StoredValue::Object(base.clone()));
        self.actor_bases.insert(actor, base.clone());
        // ponytail: derive direct based actors from this compact index; add
        // StandingCount bookkeeping when scripts consume it.
        if let Some(new_base) = base
            .as_ref()
            .filter(|new_base| Some(*new_base) != level.as_ref())
            .and_then(|object| self.object_actors.get(object))
            .copied()
        {
            self.call_other_actor_event(
                new_base,
                "Attach",
                vec![Value::Object(actor_handle)],
                actions,
            )?;
        }
        self.call_actor_event(
            actor,
            actor_class,
            instance,
            "BaseChange",
            Vec::new(),
            actions,
        )
    }

    fn actor_sweeps(
        &mut self,
        current: &CollisionActor,
        delta: Vec3,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Vec<ActorSweep>, String> {
        self.ensure_collision_actors(current_actor, current_instance)?;
        let actors = self
            .collision_actors
            .iter()
            .filter_map(|cached| cached.as_ref())
            .filter(|cached| cached.actor.collide_actors && !cached.actor.has_brush)
            .map(|cached| cached.actor.clone())
            .collect::<Vec<_>>();
        let mut hits = Vec::new();
        for other in actors {
            let actor = other.actor;
            if actor == current.actor || self.destroyed.contains(&actor) {
                continue;
            }
            if self
                .actors_share_base_chain(current.actor, actor, current_actor, current_instance)
                .map_err(|error| error.to_string())?
            {
                continue;
            }
            let Some(hit) = sweep_cylinder(
                current.location,
                current.location + delta,
                current.height,
                current.radius,
                other.location,
                other.height,
                other.radius,
            ) else {
                continue;
            };
            let use_block_players = current.player_collision || other.player_collision;
            let blocking = if use_block_players {
                current.block_players && other.block_players
            } else {
                current.block_actors && other.block_actors
            };
            hits.push(ActorSweep {
                actor,
                fraction: hit.fraction,
                normal: hit.normal,
                blocking,
            });
        }
        Ok(hits)
    }

    fn queue_ended_touches(
        &mut self,
        actor: usize,
        current: &CollisionActor,
        location: Vec3,
        current_instance: &InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let pairs = self
            .touching
            .iter()
            .copied()
            .filter(|(left, right)| *left == actor || *right == actor)
            .collect::<Vec<_>>();
        for pair in pairs {
            let other = if pair.0 == actor { pair.1 } else { pair.0 };
            let overlaps = self
                .collision_actor_by_index(other, actor, current_instance)?
                .is_some_and(|other| {
                    cylinders_overlap(
                        location,
                        current.height,
                        current.radius,
                        other.location,
                        other.height,
                        other.radius,
                    )
                });
            if !overlaps {
                self.touching.remove(&pair);
                self.queue_pair_event(actions, actor, other, "UnTouch")?;
                self.queue_pair_event(actions, other, actor, "UnTouch")?;
            }
        }
        Ok(())
    }

    fn queue_pair_event(
        &mut self,
        actions: &mut Vec<ActorAction>,
        actor: usize,
        other: usize,
        event: &'static str,
    ) -> std::result::Result<(), String> {
        let object = self
            .actor_objects
            .get(&other)
            .cloned()
            .ok_or_else(|| format!("runtime actor {other} has no object identity"))?;
        let handle = self
            .object_handle(object)
            .map_err(|error| error.to_string())?;
        actions.push(ActorAction::DispatchEvent {
            actor,
            event,
            arguments: vec![Value::Object(handle)],
        });
        Ok(())
    }

    fn collision_actor(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
    ) -> std::result::Result<CollisionActor, String> {
        let fields = self.collision_fields(class)?;
        collision_actor_from_fields(actor, instance, &fields)
    }

    fn collision_actor_by_index(
        &mut self,
        actor: usize,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Option<CollisionActor>, String> {
        self.ensure_collision_actors(current_actor, current_instance)?;
        Ok(self
            .collision_actors
            .get(actor)
            .and_then(|cached| cached.as_ref())
            .map(|cached| cached.actor.clone()))
    }

    fn ensure_collision_actors(
        &mut self,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<(), String> {
        if !self.collision_actors.is_empty() {
            return Ok(());
        }
        self.collision_actors.resize_with(self.next_actor, || None);
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        for actor in actors {
            let class_id = self.actor_classes[&actor].clone();
            let fields = if let Some(fields) = self.collision_fields.get(&class_id) {
                fields.clone()
            } else {
                let class = self
                    .resolved_object(&class_id)
                    .map_err(|error| error.to_string())?;
                self.collision_fields(&class)?
            };
            let instance = if actor == current_actor {
                current_instance
            } else {
                self.instances
                    .get(&actor)
                    .ok_or_else(|| format!("actor {actor} instance is active"))?
            };
            self.collision_actors[actor] = Some(CachedCollisionActor {
                actor: collision_actor_from_fields(actor, instance, &fields)?,
                fields,
            });
        }
        Ok(())
    }

    pub(super) fn refresh_cached_collision_actor(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
    ) -> std::result::Result<(), String> {
        if self.collision_actors.is_empty() {
            return Ok(());
        }
        let fields = self.collision_fields(class)?;
        if actor >= self.collision_actors.len() {
            self.collision_actors.resize_with(actor + 1, || None);
        }
        self.collision_actors[actor] = Some(CachedCollisionActor {
            actor: collision_actor_from_fields(actor, instance, &fields)?,
            fields,
        });
        Ok(())
    }

    pub(super) fn update_cached_collision_property(
        &mut self,
        actor: usize,
        field: &ObjectId,
        current_instance: Option<&InstanceState>,
    ) -> std::result::Result<(), String> {
        let Some(Some(cached)) = self.collision_actors.get(actor) else {
            return Ok(());
        };
        let fields = cached.fields.clone();
        if !fields.contains(field) {
            return Ok(());
        }
        let instance = match current_instance {
            Some(instance) => instance,
            None => self
                .instances
                .get(&actor)
                .ok_or_else(|| format!("actor {actor} instance is active"))?,
        };
        self.collision_actors[actor] = Some(CachedCollisionActor {
            actor: collision_actor_from_fields(actor, instance, &fields)?,
            fields,
        });
        Ok(())
    }

    fn collision_fields(
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
            collide_actors: field("bCollideActors")?,
            block_actors: field("bBlockActors")?,
            block_players: field("bBlockPlayers")?,
            brush: field("Brush")?,
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

    fn actors_share_base_chain(
        &mut self,
        first: usize,
        second: usize,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> DispatchResult<bool> {
        Ok(
            self.actor_is_based_on(first, second, current_actor, current_instance)?
                || self.actor_is_based_on(second, first, current_actor, current_instance)?,
        )
    }

    fn actor_is_based_on(
        &mut self,
        mut actor: usize,
        base: usize,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> DispatchResult<bool> {
        for _ in 0..MAX_CALL_DEPTH {
            let Some(class) = self.actor_classes.get(&actor).cloned() else {
                return Ok(false);
            };
            let class = self.resolved_object(&class)?;
            let value = if actor == current_actor {
                self.instance_property(&class, current_instance, "Base")?
            } else {
                let Some(field) = self.find_property(&class, "Base", 0)? else {
                    return Ok(false);
                };
                self.instances
                    .get(&actor)
                    .and_then(|instance| instance.get(&field))
                    .cloned()
            };
            let Some(StoredValue::Object(Some(object))) = value else {
                return Ok(false);
            };
            let Some(next) = self.object_actors.get(&object).copied() else {
                return Ok(false);
            };
            if next == base {
                return Ok(true);
            }
            actor = next;
        }
        Err(DispatchError::CallDepth)
    }

    pub(super) fn class_has_name(
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

    pub(super) fn required_actor_property(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<StoredValue, String> {
        self.instance_property(class, instance, name)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor property {name} is missing"))
    }

    pub(super) fn actor_bool(
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

    pub(super) fn actor_float(
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

    pub(super) fn actor_vector(
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

    pub(super) fn actor_object(
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

    pub(super) fn actor_byte(
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

    pub(super) fn actor_rotator(
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
    fn contains(&self, field: &ObjectId) -> bool {
        [
            &self.location,
            &self.height,
            &self.radius,
            &self.collide_actors,
            &self.block_actors,
            &self.block_players,
            &self.brush,
        ]
        .contains(&field)
    }
}

fn collision_actor_from_fields(
    actor: usize,
    instance: &InstanceState,
    fields: &CollisionFields,
) -> std::result::Result<CollisionActor, String> {
    let vector = |field: &ObjectId, name| match instance.get(field) {
        Some(StoredValue::Value(Value::Vector(value)))
            if value.iter().all(|component| component.is_finite()) =>
        {
            Ok(Vec3::from_array(*value))
        }
        Some(value) => Err(format!("actor property {name} is {value:?}")),
        None => Ok(Vec3::ZERO),
    };
    let float = |field: &ObjectId, name| match instance.get(field) {
        Some(StoredValue::Value(Value::Float(value))) if value.is_finite() && *value >= 0.0 => {
            Ok(*value)
        }
        Some(value) => Err(format!("actor property {name} is {value:?}")),
        None => Ok(0.0),
    };
    let boolean = |field: &ObjectId, name| match instance.get(field) {
        Some(StoredValue::Value(Value::Bool(value))) => Ok(*value),
        Some(value) => Err(format!("actor property {name} is {value:?}")),
        None => Ok(false),
    };
    let has_brush = match instance.get(&fields.brush) {
        Some(StoredValue::Object(value)) => value.is_some(),
        Some(value) => return Err(format!("actor property Brush is {value:?}")),
        None => false,
    };
    Ok(CollisionActor {
        actor,
        location: vector(&fields.location, "Location")?,
        height: float(&fields.height, "CollisionHeight")?,
        radius: float(&fields.radius, "CollisionRadius")?,
        collide_actors: boolean(&fields.collide_actors, "bCollideActors")?,
        block_actors: boolean(&fields.block_actors, "bBlockActors")?,
        block_players: boolean(&fields.block_players, "bBlockPlayers")?,
        player_collision: fields.player_collision,
        has_brush,
    })
}

fn actor_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}
