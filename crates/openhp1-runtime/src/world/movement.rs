use glam::Vec3;
use openhp1_physics::{cylinders_overlap, sweep_cylinder};

use super::*;

const ACTOR_TRACE_MARGIN: f32 = 1.0;

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
    pub fn update_player_touches(
        &mut self,
        location: [f32; 3],
    ) -> DispatchResult<Vec<ActorAction>> {
        if !location.iter().all(|component| component.is_finite()) {
            return Err(DispatchError::InvalidPlayerLocation { location });
        }
        let Some(player) = self.player_actor else {
            return Ok(Vec::new());
        };
        let class = self
            .actor_classes
            .get(&player)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: player })?;
        let class = self.resolved_object(&class)?;
        let instance = self
            .instances
            .get(&player)
            .cloned()
            .ok_or(DispatchError::ActiveActorContext { actor: player })?;
        let mut current = self
            .collision_actor(player, &class, &instance)
            .map_err(DispatchError::PlayerTouchCollision)?;
        current.location = Vec3::from_array(location);
        self.ensure_collision_actors(player, &instance)
            .map_err(DispatchError::PlayerTouchCollision)?;

        let mut touching = HashSet::default();
        if current.collide_actors && !current.has_brush {
            let query_minimum =
                current.location - Vec3::new(current.radius, current.radius, current.height);
            let query_maximum =
                current.location + Vec3::new(current.radius, current.radius, current.height);
            let candidate_count = self.collision_actors_by_min_x.partition_point(|&actor| {
                collision_actor_min_x(&self.collision_actors, actor) <= query_maximum.x
            });
            for &actor in &self.collision_actors_by_min_x[..candidate_count] {
                if actor == player || self.destroyed.contains(&actor) {
                    continue;
                }
                let other = &self.collision_actors[actor].as_ref().unwrap().actor;
                if other.location.x + other.radius < query_minimum.x
                    || other.location.y + other.radius < query_minimum.y
                    || other.location.y - other.radius > query_maximum.y
                    || other.location.z + other.height < query_minimum.z
                    || other.location.z - other.height > query_maximum.z
                    || actors_block(&current, other)
                {
                    continue;
                }
                if cylinders_overlap(
                    current.location,
                    current.height,
                    current.radius,
                    other.location,
                    other.height,
                    other.radius,
                ) {
                    touching.insert(actor);
                }
            }
        }

        let previous = std::mem::replace(&mut self.player_probe_touching, touching);
        let mut entered = self
            .player_probe_touching
            .difference(&previous)
            .copied()
            .collect::<Vec<_>>();
        let mut exited = previous
            .difference(&self.player_probe_touching)
            .copied()
            .collect::<Vec<_>>();
        entered.sort_unstable();
        exited.sort_unstable();

        let mut actions = Vec::with_capacity((entered.len() + exited.len()) * 2);
        for actor in entered {
            self.queue_pair_event(&mut actions, player, actor, "Touch")
                .map_err(DispatchError::PlayerTouchCollision)?;
            self.queue_pair_event(&mut actions, actor, player, "Touch")
                .map_err(DispatchError::PlayerTouchCollision)?;
        }
        for actor in exited {
            self.queue_pair_event(&mut actions, player, actor, "UnTouch")
                .map_err(DispatchError::PlayerTouchCollision)?;
            self.queue_pair_event(&mut actions, actor, player, "UnTouch")
                .map_err(DispatchError::PlayerTouchCollision)?;
        }
        Ok(actions)
    }

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
                .actors_share_base_chain(actor, other)
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
        let mut based = self.base_children.get(&object).cloned().unwrap_or_default();
        based.retain(|candidate| !self.destroyed.contains(candidate));
        Ok(based)
    }

    pub(super) fn update_actor_base(&mut self, actor: usize, base: Option<ObjectId>) {
        let previous = self.actor_bases.insert(actor, base.clone()).flatten();
        if previous == base {
            return;
        }
        if let Some(previous) = previous {
            let remove_entry = self
                .base_children
                .get_mut(&previous)
                .is_some_and(|children| {
                    if let Ok(index) = children.binary_search(&actor) {
                        children.remove(index);
                    }
                    children.is_empty()
                });
            if remove_entry {
                self.base_children.remove(&previous);
            }
        }
        if let Some(base) = base {
            let children = self.base_children.entry(base).or_default();
            if let Err(index) = children.binary_search(&actor) {
                children.insert(index, actor);
            }
        }
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
            self.reindex_cached_collision_actor(actor);
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
                    .actor_is_based_on(base_actor, actor)
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
        self.update_actor_base(actor, base.clone());
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
        let end = current.location + delta;
        let query_minimum = current.location.min(end)
            - Vec3::new(current.radius, current.radius, current.height)
            - Vec3::splat(ACTOR_TRACE_MARGIN);
        let query_maximum = current.location.max(end)
            + Vec3::new(current.radius, current.radius, current.height)
            + Vec3::splat(ACTOR_TRACE_MARGIN);
        let candidate_count = self.collision_actors_by_min_x.partition_point(|&actor| {
            collision_actor_min_x(&self.collision_actors, actor) <= query_maximum.x
        });
        let actors = self.collision_actors_by_min_x[..candidate_count].to_vec();
        let mut hits = Vec::new();
        for actor in actors {
            if actor == current.actor || self.destroyed.contains(&actor) {
                continue;
            }
            let other = &self.collision_actors[actor].as_ref().unwrap().actor;
            if other.location.x + other.radius < query_minimum.x
                || other.location.y + other.radius < query_minimum.y
                || other.location.y - other.radius > query_maximum.y
                || other.location.z + other.height < query_minimum.z
                || other.location.z - other.height > query_maximum.z
            {
                continue;
            }
            if self
                .actors_share_base_chain(current.actor, actor)
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
            hits.push(ActorSweep {
                actor,
                fraction: hit.fraction,
                normal: hit.normal,
                blocking: actors_block(current, other),
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

    pub(super) fn can_see(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
        other: usize,
    ) -> std::result::Result<bool, String> {
        let Some(other) = self.collision_actor_by_index(other, actor, instance)? else {
            return Ok(false);
        };
        let mut eye = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        eye.z += self.actor_float(class, instance, "BaseEyeHeight")?;
        let sight_radius = self.actor_float(class, instance, "SightRadius")?;
        let peripheral_vision =
            match self.required_actor_property(class, instance, "PeripheralVision")? {
                StoredValue::Value(Value::Float(value)) if value.is_finite() => value,
                value => return Err(format!("actor property PeripheralVision is {value:?}")),
            };
        let forward = Vec3::from_array(
            crate::rotator_axes(self.actor_rotator(class, instance, "Rotation")?)[0],
        );
        if !within_sight(
            eye,
            other.location,
            forward,
            sight_radius,
            peripheral_vision,
        ) {
            return Ok(false);
        }
        Ok([
            other.location,
            other.location + Vec3::Z * (other.height * 0.5),
            other.location - Vec3::Z * (other.height * 0.5),
        ]
        .into_iter()
        .any(|target| {
            self.collision
                .as_ref()
                .is_none_or(|collision| collision.sweep_aabb(eye, target, Vec3::ZERO).is_none())
        }))
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
        self.collision_actors_by_min_x = self
            .collision_actors
            .iter()
            .enumerate()
            .filter_map(|(actor, cached)| {
                cached
                    .as_ref()
                    .is_some_and(|cached| cached.actor.collide_actors && !cached.actor.has_brush)
                    .then_some(actor)
            })
            .collect();
        self.collision_actors_by_min_x
            .sort_unstable_by(|&left, &right| {
                collision_actor_min_x(&self.collision_actors, left)
                    .total_cmp(&collision_actor_min_x(&self.collision_actors, right))
                    .then_with(|| left.cmp(&right))
            });
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
        self.reindex_cached_collision_actor(actor);
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
        self.reindex_cached_collision_actor(actor);
        Ok(())
    }

    pub(super) fn reindex_cached_collision_actor(&mut self, actor: usize) {
        if let Some(index) = self
            .collision_actors_by_min_x
            .iter()
            .position(|&candidate| candidate == actor)
        {
            self.collision_actors_by_min_x.remove(index);
        }
        let Some(Some(cached)) = self.collision_actors.get(actor) else {
            return;
        };
        if !cached.actor.collide_actors || cached.actor.has_brush {
            return;
        }
        let minimum = cached.actor.location.x - cached.actor.radius;
        let index = self
            .collision_actors_by_min_x
            .binary_search_by(|&candidate| {
                collision_actor_min_x(&self.collision_actors, candidate)
                    .total_cmp(&minimum)
                    .then_with(|| candidate.cmp(&actor))
            })
            .unwrap_or_else(|index| index);
        self.collision_actors_by_min_x.insert(index, actor);
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

    fn actors_share_base_chain(&self, first: usize, second: usize) -> DispatchResult<bool> {
        Ok(self.actor_is_based_on(first, second)? || self.actor_is_based_on(second, first)?)
    }

    fn actor_is_based_on(&self, mut actor: usize, base: usize) -> DispatchResult<bool> {
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

fn collision_actor_min_x(actors: &[Option<CachedCollisionActor>], actor: usize) -> f32 {
    let actor = &actors[actor].as_ref().unwrap().actor;
    actor.location.x - actor.radius
}

fn actors_block(first: &CollisionActor, second: &CollisionActor) -> bool {
    if first.player_collision || second.player_collision {
        first.block_players && second.block_players
    } else {
        first.block_actors && second.block_actors
    }
}

fn actor_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

fn within_sight(
    eye: Vec3,
    target: Vec3,
    forward: Vec3,
    sight_radius: f32,
    peripheral_vision: f32,
) -> bool {
    let direction = target - eye;
    direction.length() <= sight_radius
        && (peripheral_vision <= 0.0
            || direction
                .try_normalize()
                .is_some_and(|direction| forward.dot(direction) >= peripheral_vision))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collision_actor(actor: usize, location: Vec3, block_players: bool) -> CollisionActor {
        CollisionActor {
            actor,
            location,
            height: 10.0,
            radius: 10.0,
            collide_actors: true,
            block_actors: false,
            block_players,
            player_collision: actor == 0,
            has_brush: false,
        }
    }

    #[test]
    fn player_probe_accepts_trigger_overlaps_but_not_blocking_actors() {
        let player = collision_actor(0, Vec3::ZERO, true);
        let trigger = collision_actor(1, Vec3::X * 5.0, false);
        let wall = collision_actor(2, Vec3::X * 5.0, true);

        assert!(!actors_block(&player, &trigger));
        assert!(cylinders_overlap(
            player.location,
            player.height,
            player.radius,
            trigger.location,
            trigger.height,
            trigger.radius,
        ));
        assert!(actors_block(&player, &wall));
    }

    #[test]
    fn sight_rejects_targets_outside_radius_or_view_cone() {
        assert!(within_sight(Vec3::ZERO, Vec3::X * 10.0, Vec3::X, 20.0, 0.5));
        assert!(!within_sight(
            Vec3::ZERO,
            Vec3::Y * 10.0,
            Vec3::X,
            20.0,
            0.5
        ));
        assert!(!within_sight(
            Vec3::ZERO,
            Vec3::X * 30.0,
            Vec3::X,
            20.0,
            0.5
        ));
    }
}
