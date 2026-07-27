use glam::Vec3;
use openhp1_physics::{cylinders_overlap, sweep_cylinder};

use super::*;

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

struct ActorSweep {
    actor: usize,
    fraction: f32,
    blocking: bool,
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
        if self.actor_bool(actor_class, instance, "bStatic")?
            || !self.actor_bool(actor_class, instance, "bMovable")?
        {
            return Ok(Value::Bool(false));
        }

        let delta = Vec3::from_array(delta);
        if delta.length_squared() < 0.00000001 {
            return Ok(Value::Bool(true));
        }
        let current = self.collision_actor(actor, actor_class, instance)?;
        let collide_world = self.actor_bool(actor_class, instance, "bCollideWorld")?;
        let mut blocking_fraction = if collide_world && !current.has_brush {
            self.collision
                .as_ref()
                .ok_or_else(|| "Move requires a configured BSP collision model".to_owned())?
                .sweep_aabb(
                    current.location,
                    current.location + delta,
                    Vec3::new(current.radius, current.radius, current.height),
                )
                .map_or(1.0, |hit| hit.fraction)
        } else {
            1.0
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
            .find(|hit| hit.blocking && hit.fraction < blocking_fraction)
            .map(|hit| {
                blocking_fraction = hit.fraction;
                hit.actor
            });

        let location = current.location + delta * blocking_fraction;
        let field = self
            .find_property(actor_class, "Location", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Move property Location is missing".to_owned())?;
        instance.insert(
            field,
            StoredValue::Value(Value::Vector(location.to_array())),
        );
        actions.push(ActorAction::SetLocation {
            actor,
            location: location.to_array(),
        });

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
            .take_while(|hit| hit.fraction < blocking_fraction)
            .filter(|hit| !hit.blocking)
        {
            let pair = actor_pair(actor, hit.actor);
            if self.touching.insert(pair) {
                self.queue_pair_event(actions, actor, hit.actor, "Touch")?;
                self.queue_pair_event(actions, hit.actor, actor, "Touch")?;
            }
        }
        self.queue_ended_touches(actor, &current, location, instance, actions)?;

        Ok(Value::Bool(blocking_fraction == 1.0))
    }

    fn actor_sweeps(
        &mut self,
        current: &CollisionActor,
        delta: Vec3,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Vec<ActorSweep>, String> {
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        let mut hits = Vec::new();
        for actor in actors {
            if actor == current.actor || self.destroyed.contains(&actor) {
                continue;
            }
            let Some(other) =
                self.collision_actor_by_index(actor, current_actor, current_instance)?
            else {
                continue;
            };
            if !other.collide_actors
                || other.has_brush
                || self
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
        Ok(CollisionActor {
            actor,
            location: Vec3::from_array(self.actor_vector(class, instance, "Location")?),
            height: self.actor_float(class, instance, "CollisionHeight")?,
            radius: self.actor_float(class, instance, "CollisionRadius")?,
            collide_actors: self.actor_bool(class, instance, "bCollideActors")?,
            block_actors: self.actor_bool(class, instance, "bBlockActors")?,
            block_players: self.actor_bool(class, instance, "bBlockPlayers")?,
            player_collision: self
                .class_has_name(class, "PlayerPawn")
                .map_err(|error| error.to_string())?
                || self
                    .class_has_name(class, "Projectile")
                    .map_err(|error| error.to_string())?,
            has_brush: self.actor_object(class, instance, "Brush")?.is_some(),
        })
    }

    fn collision_actor_by_index(
        &mut self,
        actor: usize,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Option<CollisionActor>, String> {
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(None);
        };
        let class = self
            .resolved_object(&class)
            .map_err(|error| error.to_string())?;
        if actor == current_actor {
            return self
                .collision_actor(actor, &class, current_instance)
                .map(Some);
        }
        let instance = self
            .instances
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("actor {actor} instance is active"))?;
        self.collision_actor(actor, &class, &instance).map(Some)
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

    fn class_has_name(&mut self, class: &ResolvedObject, name: &str) -> DispatchResult<bool> {
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

    fn required_actor_property(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        name: &str,
    ) -> std::result::Result<StoredValue, String> {
        self.instance_property(class, instance, name)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor property {name} is missing"))
    }

    fn actor_bool(
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

    fn actor_float(
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

    fn actor_vector(
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

    fn actor_object(
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
}

fn actor_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}
