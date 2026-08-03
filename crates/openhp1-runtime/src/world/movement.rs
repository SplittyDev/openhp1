use glam::{Mat3, Vec3};
use openhp1_physics::{boxes_overlap, cylinders_overlap, sweep_box, sweep_cylinder};

use super::*;

mod collision;
mod properties;

use collision::*;

const ACTOR_TRACE_MARGIN: f32 = 1.0;
const COLLIDE_BOX: u8 = 2;
const COLLIDE_SHAPE: u8 = 3;

#[derive(Clone)]
struct CollisionActor {
    actor: usize,
    location: Vec3,
    height: f32,
    radius: f32,
    width: f32,
    rotation: Mat3,
    collide_type: u8,
    collide_actors: bool,
    block_actors: bool,
    block_players: bool,
    player_collision: bool,
    brush: Option<Arc<BspCollision>>,
    pre_pivot: Vec3,
    main_scale: Vec3,
    shape_bounds: Option<(Vec3, Vec3)>,
}

pub(super) struct CachedCollisionActor {
    actor: CollisionActor,
    fields: CollisionFields,
}

pub(super) struct ActorSweep {
    pub(super) actor: usize,
    pub(super) fraction: f32,
    pub(super) normal: Vec3,
    blocking: bool,
}

#[derive(Clone)]
pub(super) struct CollisionFields {
    location: ObjectId,
    height: ObjectId,
    radius: ObjectId,
    width: ObjectId,
    rotation: ObjectId,
    collide_type: ObjectId,
    collide_actors: ObjectId,
    block_actors: ObjectId,
    block_players: ObjectId,
    brush: ObjectId,
    pre_pivot: ObjectId,
    main_scale: Option<ObjectId>,
    player_collision: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MovementHit {
    pub fraction: f32,
    pub normal: Vec3,
    pub actor: Option<usize>,
    pub node: Option<usize>,
}

impl ScriptRuntime {
    pub(super) fn world_collision_box(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
        visual: bool,
    ) -> std::result::Result<(Vec3, Vec3), String> {
        let actor_state = self.collision_actor(actor, class, instance)?;
        if visual && let Some(&(minimum, maximum)) = self.actor_visual_bounds.get(&actor) {
            return Ok(transform_visual_bounds(
                minimum,
                maximum,
                actor_state.location + actor_state.pre_pivot,
                actor_state.rotation,
            ));
        }
        let (center, extents) = collision_actor_world_bounds(&actor_state)
            .ok_or_else(|| format!("actor {actor} has no collision bounds"))?;
        Ok((center - extents, center + extents))
    }

    pub(super) fn trace_collision_actors(
        &mut self,
        start: Vec3,
        end: Vec3,
        extent: Vec3,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Vec<ActorSweep>, String> {
        self.ensure_collision_actors(current_actor, current_instance)?;
        let trace = CollisionActor {
            actor: current_actor,
            location: start,
            height: extent.z,
            radius: extent.x,
            width: extent.y,
            rotation: Mat3::IDENTITY,
            collide_type: 0,
            collide_actors: true,
            block_actors: false,
            block_players: false,
            player_collision: false,
            brush: None,
            pre_pivot: Vec3::ZERO,
            main_scale: Vec3::ONE,
            shape_bounds: None,
        };
        let delta = end - start;
        let mut hits = Vec::new();
        for actor in 0..self.collision_actors.len() {
            if actor == current_actor || self.destroyed.contains(&actor) {
                continue;
            }
            let Some(other) = self.collision_actors[actor]
                .as_ref()
                .map(|cached| &cached.actor)
            else {
                continue;
            };
            if !other.collide_actors {
                continue;
            }
            let Some(hit) = sweep_collision_actors(&trace, other, delta) else {
                continue;
            };
            hits.push(ActorSweep {
                actor,
                fraction: hit.fraction,
                normal: hit.normal,
                blocking: false,
            });
        }
        hits.sort_by(|left, right| {
            left.fraction
                .total_cmp(&right.fraction)
                .then_with(|| left.actor.cmp(&right.actor))
        });
        Ok(hits)
    }

    pub(super) fn colliding_actors(
        &mut self,
        location: Vec3,
        radius: f32,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Vec<usize>, String> {
        self.ensure_collision_actors(current_actor, current_instance)?;
        let minimum_x = location.x - radius;
        let maximum_x = location.x + radius;
        let candidate_count = self.collision_actors_by_min_x.partition_point(|&actor| {
            collision_actor_min_x(&self.collision_actors, actor) <= maximum_x
        });
        let candidates = self.collision_actors_by_min_x[..candidate_count].to_vec();
        let mut actors = Vec::new();
        for actor in candidates {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let other = &self.collision_actors[actor].as_ref().unwrap().actor;
            let Some((other_location, other_extents)) = collision_actor_world_bounds(other) else {
                continue;
            };
            if other_location.x + other_extents.x < minimum_x
                || !sphere_collision_actor_overlap(location, radius, other)
            {
                continue;
            }
            actors.push(actor);
        }
        actors.sort_unstable();
        Ok(actors)
    }

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
        if current.collide_actors && current.brush.is_none() {
            let current_extents = collision_actor_world_extents(&current);
            let query_minimum = current.location - current_extents;
            let query_maximum = current.location + current_extents;
            let candidate_count = self.collision_actors_by_min_x.partition_point(|&actor| {
                collision_actor_min_x(&self.collision_actors, actor) <= query_maximum.x
            });
            for &actor in &self.collision_actors_by_min_x[..candidate_count] {
                if actor == player || self.destroyed.contains(&actor) {
                    continue;
                }
                let other = &self.collision_actors[actor].as_ref().unwrap().actor;
                let Some((other_location, other_extents)) =
                    collision_actor_world_bounds(other).filter(|_| other.brush.is_none())
                else {
                    continue;
                };
                if other_location.x + other_extents.x < query_minimum.x
                    || other_location.y + other_extents.y < query_minimum.y
                    || other_location.y - other_extents.y > query_maximum.y
                    || other_location.z + other_extents.z < query_minimum.z
                    || other_location.z - other_extents.z > query_maximum.z
                    || actors_block(&current, other)
                {
                    continue;
                }
                if collision_actors_overlap(&current, other) {
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

    pub(super) fn move_actor_smooth(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<Value, String> {
        let delta = Vec3::from_array(delta);
        let hit = self.try_move_actor(actor, actor_class, delta.to_array(), instance, actions)?;
        let Some(aligned) = smooth_remaining_delta(delta, hit.normal, hit.fraction) else {
            return Ok(Value::Bool(hit.fraction == 1.0));
        };
        let hit = self.try_move_actor(actor, actor_class, aligned.to_array(), instance, actions)?;
        Ok(Value::Bool(hit.fraction == 1.0))
    }

    pub(super) fn spawn_location_is_clear(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
        active_actor: usize,
        active_instance: &InstanceState,
    ) -> std::result::Result<bool, String> {
        if !self.actor_bool(class, instance, "bCollideWorld")?
            && !self.actor_bool(class, instance, "bCollideWhenPlacing")?
        {
            return Ok(true);
        }
        let candidate = self.collision_actor(usize::MAX, class, instance)?;
        self.ensure_collision_actors(active_actor, active_instance)?;
        // ponytail: authored spawn points are world-valid; add nearby BSP-aware
        // placement search when arbitrary runtime spawn locations need it.
        Ok(self
            .collision_actors
            .iter()
            .flatten()
            .filter(|other| !self.destroyed.contains(&other.actor.actor))
            .all(|other| !placement_blocked(&candidate, &other.actor)))
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
                node: None,
            });
        }

        let delta = Vec3::from_array(delta);
        if delta.length_squared() < 0.00000001 {
            return Ok(MovementHit {
                fraction: 1.0,
                normal: Vec3::ZERO,
                actor: None,
                node: None,
            });
        }
        let current = self.collision_actor(actor, actor_class, instance)?;
        let collide_world = self.actor_bool(actor_class, instance, "bCollideWorld")?;
        let world_hit = if collide_world && current.brush.is_none() {
            let collision = self
                .collision
                .as_ref()
                .ok_or_else(|| "Move requires a configured BSP collision model".to_owned())?;
            if current.collide_type == COLLIDE_BOX {
                collision.sweep_aabb(
                    current.location,
                    current.location + delta,
                    collision_actor_world_extents(&current),
                )
            } else {
                collision.sweep_cylinder(
                    current.location,
                    current.location + delta,
                    current.radius,
                    current.height,
                )
            }
        } else {
            None
        };
        let mut blocking_hit = if let Some(hit) = world_hit {
            MovementHit {
                fraction: hit.fraction,
                normal: hit.normal,
                actor: None,
                node: Some(hit.node),
            }
        } else {
            MovementHit {
                fraction: 1.0,
                normal: Vec3::ZERO,
                actor: None,
                node: None,
            }
        };

        let mut hits = if (current.collide_actors || collide_world) && current.brush.is_none() {
            self.actor_sweeps(&current, delta, collide_world, actor, instance)?
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
                    node: None,
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
        collide_world: bool,
        current_actor: usize,
        current_instance: &InstanceState,
    ) -> std::result::Result<Vec<ActorSweep>, String> {
        self.ensure_collision_actors(current_actor, current_instance)?;
        let end = current.location + delta;
        let current_extents = collision_actor_world_extents(current);
        let query_minimum =
            current.location.min(end) - current_extents - Vec3::splat(ACTOR_TRACE_MARGIN);
        let query_maximum =
            current.location.max(end) + current_extents + Vec3::splat(ACTOR_TRACE_MARGIN);
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
            if (other.brush.is_some() && !collide_world)
                || (other.brush.is_none() && !current.collide_actors)
            {
                continue;
            }
            let Some((other_location, other_extents)) = collision_actor_world_bounds(other) else {
                continue;
            };
            if other_location.x + other_extents.x < query_minimum.x
                || other_location.y + other_extents.y < query_minimum.y
                || other_location.y - other_extents.y > query_maximum.y
                || other_location.z + other_extents.z < query_minimum.z
                || other_location.z - other_extents.z > query_maximum.z
            {
                continue;
            }
            if self
                .actors_share_base_chain(current.actor, actor)
                .map_err(|error| error.to_string())?
            {
                continue;
            }
            let Some(hit) = sweep_collision_actors(current, other, delta) else {
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
                    let mut current = current.clone();
                    current.location = location;
                    collision_actors_overlap(&current, &other)
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
        self.collision_actor_from_fields(actor, instance, &fields)
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
        Ok(self.actor_is_visible(eye, &other))
    }

    pub(super) fn line_of_sight_to(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
        other: usize,
    ) -> std::result::Result<bool, String> {
        let Some(other) = self.collision_actor_by_index(other, actor, instance)? else {
            return Ok(false);
        };
        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        if location.distance(other.location) > self.actor_float(class, instance, "SightRadius")? {
            return Ok(false);
        }
        let mut eye = location;
        eye.z += self.actor_float(class, instance, "BaseEyeHeight")?;
        Ok(self.actor_is_visible(eye, &other))
    }

    pub(super) fn player_can_see_me(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &InstanceState,
    ) -> std::result::Result<bool, String> {
        let target = Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
        let level = self
            .actor_object(actor_class, instance, "Level")?
            .ok_or_else(|| "PlayerCanSeeMe actor has no Level".to_owned())?;
        let level_actor = self
            .object_actors
            .get(&level)
            .copied()
            .ok_or_else(|| "PlayerCanSeeMe Level is not a registered actor".to_owned())?;
        let level_class = self
            .actor_classes
            .get(&level_actor)
            .cloned()
            .ok_or_else(|| format!("PlayerCanSeeMe Level actor {level_actor} has no class"))?;
        let level_class = self
            .resolved_object(&level_class)
            .map_err(|error| error.to_string())?;
        let level_instance = self.instances.get(&level_actor).cloned().ok_or_else(|| {
            format!("PlayerCanSeeMe Level actor {level_actor} instance is active")
        })?;
        let mut pawn = self.actor_object(&level_class, &level_instance, "PawnList")?;
        let mut seen = HashSet::default();

        while let Some(pawn_object) = pawn {
            let pawn_actor = self
                .object_actors
                .get(&pawn_object)
                .copied()
                .ok_or_else(|| "PlayerCanSeeMe PawnList has an unregistered pawn".to_owned())?;
            if !seen.insert(pawn_actor) {
                return Err("PlayerCanSeeMe PawnList has a cycle".to_owned());
            }
            if pawn_actor == actor {
                pawn = self.actor_object(actor_class, instance, "nextPawn")?;
                continue;
            }
            let destroyed = self.destroyed.contains(&pawn_actor);
            let pawn_class = self
                .actor_classes
                .get(&pawn_actor)
                .cloned()
                .ok_or_else(|| format!("PlayerCanSeeMe pawn {pawn_actor} has no class"))?;
            let pawn_class = self
                .resolved_object(&pawn_class)
                .map_err(|error| error.to_string())?;
            let pawn_instance =
                self.instances.get(&pawn_actor).cloned().ok_or_else(|| {
                    format!("PlayerCanSeeMe pawn {pawn_actor} instance is active")
                })?;
            pawn = self.actor_object(&pawn_class, &pawn_instance, "nextPawn")?;

            if destroyed {
                continue;
            }
            let location =
                Vec3::from_array(self.actor_vector(&pawn_class, &pawn_instance, "Location")?);
            let forward = Vec3::from_array(
                crate::rotator_axes(self.actor_rotator(
                    &pawn_class,
                    &pawn_instance,
                    "ViewRotation",
                )?)[0],
            );
            let behind_view = self.actor_bool(&pawn_class, &pawn_instance, "bBehindView")?;
            if !player_can_see_me_candidate(location, target, forward, behind_view) {
                continue;
            }
            let mut eye = location;
            eye.z += self.actor_float(&pawn_class, &pawn_instance, "BaseEyeHeight")?;
            if self.has_line_of_sight(eye, target) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn has_line_of_sight(&self, eye: Vec3, target: Vec3) -> bool {
        self.collision
            .as_ref()
            .is_none_or(|collision| collision.sweep_aabb(eye, target, Vec3::ZERO).is_none())
    }

    fn actor_is_visible(&self, eye: Vec3, other: &CollisionActor) -> bool {
        [
            other.location,
            other.location + Vec3::Z * (other.height * 0.5),
            other.location - Vec3::Z * (other.height * 0.5),
        ]
        .into_iter()
        .any(|target| self.has_line_of_sight(eye, target))
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
                current_instance.clone()
            } else {
                self.instances
                    .get(&actor)
                    .ok_or_else(|| format!("actor {actor} instance is active"))?
                    .clone()
            };
            self.collision_actors[actor] = Some(CachedCollisionActor {
                actor: self.collision_actor_from_fields(actor, &instance, &fields)?,
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
                    .is_some_and(|cached| {
                        cached.actor.collide_actors
                            && collision_actor_world_bounds(&cached.actor).is_some()
                    })
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
            actor: self.collision_actor_from_fields(actor, instance, &fields)?,
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
            Some(instance) => instance.clone(),
            None => self
                .instances
                .get(&actor)
                .ok_or_else(|| format!("actor {actor} instance is active"))?
                .clone(),
        };
        self.collision_actors[actor] = Some(CachedCollisionActor {
            actor: self.collision_actor_from_fields(actor, &instance, &fields)?,
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
        if !cached.actor.collide_actors {
            return;
        }
        let Some((location, extents)) = collision_actor_world_bounds(&cached.actor) else {
            return;
        };
        let minimum = location.x - extents.x;
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

    pub(super) fn update_cached_collision_shape_bounds(
        &mut self,
        actor: usize,
        bounds: Option<(Vec3, Vec3)>,
    ) {
        if let Some(Some(cached)) = self.collision_actors.get_mut(actor) {
            cached.actor.shape_bounds = bounds;
            self.reindex_cached_collision_actor(actor);
        }
    }
}
