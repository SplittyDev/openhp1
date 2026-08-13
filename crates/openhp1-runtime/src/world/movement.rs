use glam::{Mat3, Vec3};
use openhp1_map::PolyFlags;
use openhp1_physics::{
    BspCollision, CollisionHit, boxes_overlap, cylinders_overlap, sweep_box, sweep_cylinder,
};

use super::physics::{PHYS_FLYING, PHYS_SWIMMING, PHYS_WALKING, two_wall_adjust};
use super::state::event_disabled;
use super::*;

mod collision;
mod properties;

use collision::*;

const ACTOR_TRACE_MARGIN: f32 = 1.0;
const MOVE_TRACE_PADDING: f32 = 2.0;
const MOVE_HIT_EPSILON: f32 = 0.0001;
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
    pub(super) node: Option<usize>,
    blocking: bool,
    bumping: bool,
}

enum MovementEvaluation<'a> {
    Real {
        actions: &'a mut Vec<ActorAction>,
        own_base_blocks: bool,
    },
    Query,
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
                node: hit.node,
                blocking: false,
                bumping: false,
            });
        }
        hits.sort_by(|left, right| {
            left.fraction
                .total_cmp(&right.fraction)
                .then_with(|| left.actor.cmp(&right.actor))
        });
        Ok(hits)
    }

    pub(super) fn floor_height_at(
        &mut self,
        actor: usize,
        instance: &InstanceState,
        location: Vec3,
        distance: f32,
        radius: f32,
    ) -> std::result::Result<Option<f32>, String> {
        let start = location + Vec3::Z * distance;
        let end = location - Vec3::Z * distance;
        let extent = Vec3::new(radius, radius, 1.0);
        let actor_hit = self
            .trace_collision_actors(start, end, extent, actor, instance)?
            .into_iter()
            .next()
            .map(|hit| (hit.fraction, hit.normal));
        let bsp_hit = self
            .collision
            .as_ref()
            .and_then(|collision| collision.sweep_aabb(start, end, extent))
            .map(|hit| (hit.fraction, hit.normal));
        let fraction = match (actor_hit, bsp_hit) {
            (Some((actor_fraction, _)), Some((bsp_fraction, _)))
                if actor_fraction < bsp_fraction =>
            {
                Some(actor_fraction)
            }
            (_, Some((bsp_fraction, _))) => Some(bsp_fraction),
            (Some((actor_fraction, _)), None) => Some(actor_fraction),
            (None, None) => None,
        };
        Ok(fraction.map(|fraction| start.lerp(end, fraction).z))
    }

    pub(in crate::world) fn single_line_check_between(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &InstanceState,
        start: Vec3,
        end: Vec3,
    ) -> std::result::Result<MovementHit, String> {
        let extent = collision_actor_local_extents(&self.collision_actor(actor, class, instance)?);
        let mut nearest = self
            .collision
            .as_ref()
            .and_then(|collision| collision.sweep_aabb(start, end, extent))
            .map_or(
                MovementHit {
                    fraction: 1.0,
                    normal: Vec3::ZERO,
                    actor: None,
                    node: None,
                },
                |hit| MovementHit {
                    fraction: hit.fraction,
                    normal: hit.normal,
                    actor: None,
                    node: Some(hit.node),
                },
            );
        self.ensure_collision_actors(actor, instance)?;
        for other_actor in 0..self.collision_actors.len() {
            if other_actor == actor
                || self.destroyed.contains(&other_actor)
                || !self.actor_is_mover(other_actor)?
            {
                continue;
            }
            let Some(other) = self.collision_actors[other_actor]
                .as_ref()
                .map(|cached| cached.actor.clone())
                .filter(|other| other.collide_actors)
            else {
                continue;
            };
            let Some(hit) = other.brush.as_ref().and_then(|brush| {
                brush.sweep_transformed_aabb(
                    start,
                    end,
                    extent,
                    other.location,
                    other.rotation,
                    other.pre_pivot,
                    other.main_scale,
                )
            }) else {
                continue;
            };
            if hit.fraction < nearest.fraction {
                nearest = MovementHit {
                    fraction: hit.fraction,
                    normal: hit.normal,
                    actor: Some(other_actor),
                    node: Some(hit.node),
                };
            }
        }
        Ok(nearest)
    }

    pub(in crate::world) fn movement_hit_has_poly_flag(
        &mut self,
        actor: usize,
        instance: &InstanceState,
        hit: MovementHit,
        start: Vec3,
        distance: f32,
        flag: PolyFlags,
    ) -> std::result::Result<bool, String> {
        let Some(fallback_node) = hit.node else {
            return Ok(false);
        };
        let end = start - hit.normal * distance;
        if let Some(other) = hit.actor {
            self.ensure_collision_actors(actor, instance)?;
            let Some(other) = self.collision_actors[other]
                .as_ref()
                .map(|cached| cached.actor.clone())
            else {
                return Ok(false);
            };
            let Some(brush) = other.brush else {
                return Ok(false);
            };
            let node = brush
                .sweep_transformed_aabb(
                    start,
                    end,
                    Vec3::ZERO,
                    other.location,
                    other.rotation,
                    other.pre_pivot,
                    other.main_scale,
                )
                .map_or(fallback_node, |surface| surface.node);
            return Ok(brush.node_has_poly_flag(node, flag));
        }
        let collision = self
            .collision
            .as_ref()
            .ok_or_else(|| "surface lookup requires a configured BSP collision model".to_owned())?;
        let node = collision
            .line_trace(start, end)
            .map_or(fallback_node, |surface| surface.node);
        Ok(collision.node_has_poly_flag(node, flag))
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

    pub fn place_actor(
        &mut self,
        actor: usize,
        location: [f32; 3],
    ) -> DispatchResult<(bool, Vec<ActorAction>)> {
        if !location.iter().all(|component| component.is_finite()) {
            return Err(DispatchError::ActorPlacement {
                actor,
                message: format!("location {location:?} is invalid"),
            });
        }
        let class_id = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class_id)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let mut actions = Vec::new();
        let result = self
            .set_actor_location_placing(
                actor,
                &class,
                &mut instance,
                Vec3::from_array(location),
                &mut actions,
            )
            .map_err(|message| DispatchError::ActorPlacement { actor, message });
        self.instances.insert(actor, instance);
        result.map(|placed| (placed, actions))
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

    pub(super) fn actor_reachable(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &InstanceState,
        target: usize,
    ) -> std::result::Result<bool, String> {
        if self.destroyed.contains(&target) {
            return Ok(false);
        }
        let target_object = self
            .actor_objects
            .get(&target)
            .cloned()
            .ok_or_else(|| format!("runtime actor {target} has no object identity"))?;
        let target_class = self
            .actor_classes
            .get(&target)
            .cloned()
            .ok_or_else(|| format!("actorReachable target actor {target} has no class"))?;
        let target_class = self
            .resolved_object(&target_class)
            .map_err(|error| error.to_string())?;
        let target_instance = if target == actor {
            instance.clone()
        } else {
            self.instances
                .get(&target)
                .cloned()
                .ok_or_else(|| format!("actorReachable target actor {target} instance is active"))?
        };
        let target_location =
            Vec3::from_array(self.actor_vector(&target_class, &target_instance, "Location")?);
        let current_location =
            Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?);
        let navpoint = if self
            .class_has_name(&target_class, "NavigationPoint")
            .map_err(|error| error.to_string())?
        {
            Some(target_object)
        } else if self
            .class_has_name(&target_class, "Inventory")
            .map_err(|error| error.to_string())?
        {
            self.other_actor_object(target, "myMarker")?
        } else {
            None
        };
        let target_is_pawn = navpoint.is_none()
            && self
                .class_has_name(&target_class, "Pawn")
                .map_err(|error| error.to_string())?;
        if !target_is_pawn && current_location.distance_squared(target_location) > 1_000_000.0 {
            return Ok(false);
        }
        let is_player = self
            .optional_actor_bool(actor_class, instance, "bIsPlayer")?
            .unwrap_or(
                self.class_has_name(actor_class, "PlayerPawn")
                    .map_err(|error| error.to_string())?,
            );
        let radius = self.actor_float(actor_class, instance, "CollisionRadius")?;
        let height = self.actor_float(actor_class, instance, "CollisionHeight")?;
        if let Some(navpoint) = navpoint {
            let player_only =
                self.object_actors
                    .get(&navpoint)
                    .copied()
                    .map(|navpoint| {
                        let class =
                            self.actor_classes.get(&navpoint).cloned().ok_or_else(|| {
                                format!("navigation point {navpoint} has no class")
                            })?;
                        let class = self
                            .resolved_object(&class)
                            .map_err(|error| error.to_string())?;
                        let instance = if navpoint == actor {
                            instance.clone()
                        } else {
                            self.instances.get(&navpoint).cloned().ok_or_else(|| {
                                format!("navigation point {navpoint} instance is active")
                            })?
                        };
                        self.optional_actor_bool(&class, &instance, "bPlayerOnly")
                    })
                    .transpose()?
                    .flatten()
                    .unwrap_or(false);
            if player_only && !is_player {
                return Ok(false);
            }
            if !self.navigation_point_reaches(&navpoint, radius, height)? {
                return Ok(false);
            }
        }

        if self.collision.is_some() {
            if self
                .zone_physics(target_location, actor, instance)?
                .is_some_and(|zone| zone.water)
                && !self.actor_bool(actor_class, instance, "bCanSwim")?
            {
                return Ok(false);
            }
            let foot = if target_is_pawn {
                target_location
                    - Vec3::Z
                        * self.actor_float(&target_class, &target_instance, "CollisionHeight")?
            } else {
                target_location
            };
            let reduced_damage_type =
                self.optional_actor_name(actor_class, instance, "ReducedDamageType")?;
            if let Some(zone) = self.zone_physics(foot, actor, instance)? {
                let damage_matches = match (&zone.damage_type, &reduced_damage_type) {
                    (Some(zone), Some(reduced)) => zone.eq_ignore_ascii_case(reduced),
                    (None, None) => true,
                    _ => false,
                };
                if zone.pain && !damage_matches {
                    return Ok(false);
                }
            }
        }
        let mut eye = current_location;
        eye.z += self.actor_float(actor_class, instance, "BaseEyeHeight")?;
        if !self.has_line_of_sight(eye, target_location) {
            return Ok(false);
        }

        let check_location = self.actor_bool(actor_class, instance, "bCollideWorld")?
            || self.actor_bool(actor_class, instance, "bCollideWhenPlacing")?;
        if !self.actor_reachable_check_location(target_location, radius, height, check_location)? {
            return Ok(false);
        }

        let mut current = self.collision_actor(actor, actor_class, instance)?;
        let mut probe_instance = instance.clone();
        let mut evaluation = MovementEvaluation::Query;
        let collide_world = self.actor_bool(actor_class, instance, "bCollideWorld")?;
        let physics = self.actor_byte(actor_class, instance, "Physics")?;
        let mut reached = false;
        for _ in 0..5 {
            let mut delta = target_location - current.location;
            if physics == PHYS_WALKING {
                delta.z = 0.0;
                if delta.length_squared() <= 1.0 {
                    reached = true;
                    break;
                }
                let step_height = self.actor_float(actor_class, instance, "MaxStepHeight")?;
                let gravity_direction = self
                    .collision
                    .is_some()
                    .then(|| self.zone_physics(current.location, actor, instance))
                    .transpose()?
                    .flatten()
                    .map_or(-1.0, |zone| if zone.gravity.z > 0.0 { 1.0 } else { -1.0 });
                let step_up = Vec3::new(0.0, 0.0, -gravity_direction * step_height);
                let (hit, _) = self.movement_hit(
                    &current,
                    step_up,
                    collide_world,
                    actor,
                    &mut probe_instance,
                    &mut evaluation,
                )?;
                current.location += step_up * hit.fraction;

                let (hit, _) = self.movement_hit(
                    &current,
                    delta,
                    collide_world,
                    actor,
                    &mut probe_instance,
                    &mut evaluation,
                )?;
                let mut moved = delta * hit.fraction;
                current.location += moved;
                if hit.fraction < 1.0 {
                    let remaining = target_location - current.location;
                    let Some(aligned) = smooth_remaining_delta(remaining, hit.normal, hit.fraction)
                    else {
                        break;
                    };
                    if aligned.length_squared() <= 0.00000001 {
                        break;
                    }
                    let (hit, _) = self.movement_hit(
                        &current,
                        aligned,
                        collide_world,
                        actor,
                        &mut probe_instance,
                        &mut evaluation,
                    )?;
                    moved = remaining * hit.fraction;
                    current.location += moved;
                }
                let (hit, _) = self.movement_hit(
                    &current,
                    -step_up,
                    collide_world,
                    actor,
                    &mut probe_instance,
                    &mut evaluation,
                )?;
                current.location -= step_up * hit.fraction;
                if moved.length_squared() <= 1.0 {
                    break;
                }
            } else if matches!(physics, PHYS_FLYING | PHYS_SWIMMING) {
                if delta.length_squared() <= 1.0 {
                    reached = true;
                    break;
                }
                let (hit, _) = self.movement_hit(
                    &current,
                    delta,
                    collide_world,
                    actor,
                    &mut probe_instance,
                    &mut evaluation,
                )?;
                let mut moved = delta * hit.fraction;
                current.location += moved;
                if hit.fraction < 1.0 {
                    let remaining = target_location - current.location;
                    let Some(aligned) = smooth_remaining_delta(remaining, hit.normal, hit.fraction)
                    else {
                        break;
                    };
                    if aligned.length_squared() <= 0.00000001 {
                        break;
                    }
                    let (hit, _) = self.movement_hit(
                        &current,
                        aligned,
                        collide_world,
                        actor,
                        &mut probe_instance,
                        &mut evaluation,
                    )?;
                    moved = remaining * hit.fraction;
                    current.location += moved;
                }
                if moved.length_squared() <= 1.0 {
                    break;
                }
            } else {
                return Ok(false);
            }
        }
        if reached && physics == PHYS_WALKING {
            let vertical = target_location.z - current.location.z;
            let gravity_direction = self
                .collision
                .is_some()
                .then(|| self.zone_physics(current.location, actor, instance))
                .transpose()?
                .flatten()
                .map_or(-1.0, |zone| if zone.gravity.z > 0.0 { 1.0 } else { -1.0 });
            if (vertical < -0.1 && gravity_direction == -1.0)
                || (vertical > 0.1 && gravity_direction == 1.0)
            {
                let (hit, _) = self.movement_hit(
                    &current,
                    Vec3::new(0.0, 0.0, vertical),
                    collide_world,
                    actor,
                    &mut probe_instance,
                    &mut evaluation,
                )?;
                current.location.z += vertical * hit.fraction;
            }
            reached = (target_location.z - current.location.z).abs() <= height;
        }
        Ok(reached)
    }

    fn navigation_point_reaches(
        &mut self,
        target: &ObjectId,
        radius: f32,
        height: f32,
    ) -> std::result::Result<bool, String> {
        let navigation_points = self
            .actor_classes
            .iter()
            .map(|(&actor, class)| (actor, class.clone()))
            .collect::<Vec<_>>();
        for (actor, class) in navigation_points {
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            if self
                .find_property(&class, "Paths", 0)
                .map_err(|error| error.to_string())?
                .is_none()
            {
                continue;
            }
            if !self
                .class_has_name(&class, "NavigationPoint")
                .map_err(|error| error.to_string())?
            {
                continue;
            }
            let instance = self
                .instances
                .get(&actor)
                .cloned()
                .ok_or_else(|| format!("navigation point {actor} instance is active"))?;
            for property in ["Paths", "PrunedPaths"] {
                let Some(StoredValue::Array(paths)) = self
                    .instance_property(&class, &instance, property)
                    .map_err(|error| error.to_string())?
                else {
                    continue;
                };
                for path in paths {
                    let StoredValue::Value(Value::Int(index)) = path else {
                        return Err(format!("navigation point {actor} {property} is not an int"));
                    };
                    let Ok(index) = usize::try_from(index) else {
                        break;
                    };
                    let Some(spec) = self.reach_specs.iter().find(|spec| spec.index == index)
                    else {
                        continue;
                    };
                    if spec.end == *target
                        && !spec.pruned
                        && spec.collision_radius as f32 >= radius
                        && spec.collision_height as f32 >= height
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn actor_reachable_check_location(
        &self,
        location: Vec3,
        radius: f32,
        height: f32,
        check: bool,
    ) -> std::result::Result<bool, String> {
        if !check {
            return Ok(true);
        }
        let collision = self
            .collision
            .as_ref()
            .ok_or_else(|| "actorReachable requires a configured BSP collision model".to_owned())?;
        let scale = radius.max(height);
        for z in [0.0, 1.0, -1.0] {
            for y in [0.0, 1.0, -1.0] {
                for x in [0.0, 1.0, -1.0] {
                    let location = location + Vec3::new(x, y, z) * scale;
                    if !collision.overlaps_cylinder(location, radius, height) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
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
        let first_normal = hit.normal;
        let Some(aligned) = smooth_remaining_delta(delta, hit.normal, hit.fraction) else {
            return Ok(Value::Bool(hit.fraction == 1.0));
        };
        let hit = self.try_move_actor(actor, actor_class, aligned.to_array(), instance, actions)?;
        if hit.fraction < 1.0 {
            let corner = two_wall_adjust(
                aligned,
                hit.normal,
                first_normal,
                delta.normalize_or_zero(),
                hit.fraction,
            );
            let hit =
                self.try_move_actor(actor, actor_class, corner.to_array(), instance, actions)?;
            return Ok(Value::Bool(hit.fraction == 1.0));
        }
        Ok(Value::Bool(hit.fraction == 1.0))
    }

    pub(super) fn find_spawn_location(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
    ) -> std::result::Result<Option<Vec3>, String> {
        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        if !self.actor_bool(class, instance, "bCollideWorld")?
            && !self.actor_bool(class, instance, "bCollideWhenPlacing")?
        {
            return Ok(Some(location));
        }
        let mut candidate = self.collision_actor(usize::MAX, class, instance)?;
        let collision = self
            .collision
            .as_ref()
            .ok_or("Spawn requires a configured BSP collision model")?;
        Ok(find_placement_location(collision, &mut candidate, location))
    }

    pub(super) fn try_move_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<MovementHit, String> {
        self.try_move_actor_inner(
            actor,
            actor_class,
            delta,
            None,
            true,
            instance,
            Some(actions),
        )
    }

    pub(super) fn try_move_actor_rotated(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        rotation: [i32; 3],
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<MovementHit, String> {
        self.try_move_actor_inner(
            actor,
            actor_class,
            [0.0; 3],
            Some(rotation),
            true,
            instance,
            Some(actions),
        )
    }

    pub(super) fn test_move_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &InstanceState,
    ) -> std::result::Result<MovementHit, String> {
        let mut instance = instance.clone();
        self.try_move_actor_inner(actor, actor_class, delta, None, true, &mut instance, None)
    }

    pub(super) fn single_line_check_actor(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        instance: &InstanceState,
    ) -> std::result::Result<MovementHit, String> {
        let mut instance = instance.clone();
        let current = self.collision_actor(actor, actor_class, &instance)?;
        let collide_world = self.actor_bool(actor_class, &instance, "bCollideWorld")?;
        let mut evaluation = MovementEvaluation::Query;
        self.movement_hit(
            &current,
            Vec3::from_array(delta),
            collide_world,
            actor,
            &mut instance,
            &mut evaluation,
        )
        .map(|(hit, _)| hit)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_move_actor_inner(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        delta: [f32; 3],
        rotation: Option<[i32; 3]>,
        own_base_blocks: bool,
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
        if delta.length_squared() < 0.00000001 && rotation.is_none() {
            return Ok(MovementHit {
                fraction: 1.0,
                normal: Vec3::ZERO,
                actor: None,
                node: None,
            });
        }
        let mut current = self.collision_actor(actor, actor_class, instance)?;
        let previous_rotation = rotation
            .map(|_| self.actor_rotator(actor_class, instance, "Rotation"))
            .transpose()?;
        if delta.length_squared() < 0.00000001 {
            let rotation = rotation.expect("zero-delta movement has a rotation");
            if previous_rotation == Some(rotation) {
                return Ok(MovementHit {
                    fraction: 1.0,
                    normal: Vec3::ZERO,
                    actor: None,
                    node: None,
                });
            }
            if current.brush.is_none() && self.based_actors(actor)?.is_empty() {
                if let Some(actions) = actions.as_deref_mut() {
                    self.set_actor_rotation(actor, actor_class, instance, rotation, actions)?;
                }
                return Ok(MovementHit {
                    fraction: 1.0,
                    normal: Vec3::ZERO,
                    actor: None,
                    node: None,
                });
            }
        }
        if let Some(rotation) = rotation {
            let axes = crate::rotator_axes(rotation);
            current.rotation = Mat3::from_cols(
                Vec3::from_array(axes[0]),
                Vec3::from_array(axes[1]),
                Vec3::from_array(axes[2]),
            );
        }
        let collide_world = self.actor_bool(actor_class, instance, "bCollideWorld")?;
        let delta_length = delta.length();
        let trace_padding = if current.brush.is_none() && delta_length > MOVE_HIT_EPSILON {
            MOVE_TRACE_PADDING
        } else {
            0.0
        };
        let trace_delta = delta + delta.normalize_or_zero() * trace_padding;
        let (mut blocking_hit, hits) = {
            let mut evaluation = match actions.as_deref_mut() {
                Some(actions) => MovementEvaluation::Real {
                    actions,
                    own_base_blocks,
                },
                None => MovementEvaluation::Query,
            };
            self.movement_hit(
                &current,
                trace_delta,
                collide_world,
                actor,
                instance,
                &mut evaluation,
            )?
        };
        if blocking_hit.fraction < 1.0 && trace_padding > 0.0 {
            blocking_hit.fraction =
                move_hit_fraction(blocking_hit.fraction, delta_length, trace_padding);
        }
        let blocking_actor = blocking_hit.actor;

        let Some(actions) = actions.as_mut() else {
            return Ok(blocking_hit);
        };
        let mut contact_bumps = Vec::new();
        for hit in hits
            .iter()
            .take_while(|hit| hit.fraction < blocking_hit.fraction)
            .filter(|hit| hit.bumping && !hit.blocking)
        {
            if self.actor_is_mover(hit.actor)? {
                self.dispatch_contact_bump(
                    hit.actor,
                    actor,
                    actor_class,
                    instance,
                    current.location + delta * hit.fraction,
                    actions,
                )?;
                contact_bumps.push(hit.actor);
            }
        }
        let location = current.location + delta * blocking_hit.fraction;
        if delta.length_squared() >= 0.00000001 {
            self.set_actor_location(actor, actor_class, instance, location, actions)?;
        }
        if blocking_hit.fraction == 1.0
            && let Some(rotation) = rotation
        {
            self.set_actor_rotation(actor, actor_class, instance, rotation, actions)?;
        }
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
            let result = self.try_move_actor_inner(
                based_actor,
                &class,
                actually_moved.to_array(),
                None,
                false,
                &mut based_instance,
                Some(actions),
            );
            self.instances.insert(based_actor, based_instance);
            result?;
        }
        if blocking_hit.fraction == 1.0
            && let (Some(rotation), Some(previous_rotation)) = (rotation, previous_rotation)
        {
            self.turn_based_actors(
                actor,
                location,
                [
                    rotation[0].wrapping_sub(previous_rotation[0]),
                    rotation[1].wrapping_sub(previous_rotation[1]),
                    rotation[2].wrapping_sub(previous_rotation[2]),
                ],
                actions,
            )?;
        }

        let mut moved = current.clone();
        moved.location = location;
        if blocking_hit.fraction == 1.0
            && moved.brush.is_some()
            && self.moving_brush_encroached(actor, actor_class, instance, &moved, actions)?
        {
            self.set_actor_location(actor, actor_class, instance, current.location, actions)?;
            if let Some(previous_rotation) = previous_rotation {
                self.set_actor_rotation(actor, actor_class, instance, previous_rotation, actions)?;
            }
            return Ok(MovementHit {
                fraction: 0.0,
                normal: Vec3::ZERO,
                actor: None,
                node: None,
            });
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
            .filter(|hit| hit.bumping)
        {
            if !contact_bumps.contains(&hit.actor) {
                self.queue_pair_event(actions, hit.actor, actor, "Bump")?;
            }
            self.queue_pair_event(actions, actor, hit.actor, "Bump")?;
        }
        for hit in hits
            .iter()
            .take_while(|hit| hit.fraction < blocking_hit.fraction)
            .filter(|hit| !hit.blocking && !hit.bumping)
        {
            let pair = actor_pair(actor, hit.actor);
            if self.touching.insert(pair) {
                self.queue_pair_event(actions, actor, hit.actor, "Touch")?;
                self.queue_pair_event(actions, hit.actor, actor, "Touch")?;
            }
        }
        self.queue_ended_touches(actor, &current, location, instance, actions)?;
        if actor_class.package.summary().exports[actor_class.export_index].serial_size != 0
            && self.collision.is_some()
            && self
                .class_has_name(actor_class, "Pawn")
                .map_err(|error| error.to_string())?
        {
            self.update_pawn_regions(actor, actor_class, instance, actions)?;
        }

        Ok(blocking_hit)
    }

    fn moving_brush_encroached(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        mover: &CollisionActor,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        if !mover.block_players && !mover.block_actors && !mover.collide_actors {
            return Ok(false);
        }
        self.ensure_collision_actors(actor, instance)?;
        let Some((center, extents)) = collision_actor_world_bounds(mover) else {
            return Ok(false);
        };
        let minimum = center - extents;
        let maximum = center + extents;
        let candidate_count = self
            .collision_actors_by_min_x
            .partition_point(|&candidate| {
                collision_actor_min_x(&self.collision_actors, candidate) <= maximum.x
            });
        let candidates = self.collision_actors_by_min_x[..candidate_count].to_vec();
        let brush = mover.brush.as_ref().expect("moving brush was checked");
        let mut overlaps = Vec::new();
        for candidate in candidates {
            if candidate == actor || self.destroyed.contains(&candidate) {
                continue;
            }
            let other = &self.collision_actors[candidate]
                .as_ref()
                .expect("indexed collision actor is missing")
                .actor;
            if other.brush.is_some() || !actors_block(mover, other) {
                continue;
            }
            let Some((other_center, other_extents)) = collision_actor_world_bounds(other) else {
                continue;
            };
            if other_center.x + other_extents.x < minimum.x
                || other_center.y + other_extents.y < minimum.y
                || other_center.y - other_extents.y > maximum.y
                || other_center.z + other_extents.z < minimum.z
                || other_center.z - other_extents.z > maximum.z
                || !brush.overlaps_transformed_aabb(
                    other_center,
                    other_extents,
                    mover.location,
                    mover.rotation,
                    mover.pre_pivot,
                    mover.main_scale,
                )
            {
                continue;
            }
            overlaps.push(candidate);
        }

        for &other in &overlaps {
            if self.mover_encroaching_on(actor, actor_class, instance, other, actions)? {
                return Ok(true);
            }
        }
        if overlaps.is_empty() {
            return Ok(false);
        }
        let mover_handle = self
            .actor_objects
            .get(&actor)
            .cloned()
            .ok_or_else(|| format!("mover {actor} has no object identity"))
            .and_then(|object| {
                self.object_handle(object)
                    .map_err(|error| error.to_string())
            })?;
        for other in overlaps {
            self.call_other_actor_event(
                other,
                "EncroachedBy",
                vec![Value::Object(mover_handle)],
                actions,
            )?;
        }
        Ok(false)
    }

    fn mover_encroaching_on(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        other: usize,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        if event_disabled(
            &self.disabled_events,
            actor,
            self.actor_states
                .get(&actor)
                .and_then(|state| state.as_deref()),
            "EncroachingOn",
        ) || self
            .state_ignores_event(actor, actor_class, "EncroachingOn")
            .map_err(|error| error.to_string())?
        {
            return Ok(false);
        }
        let Some(function) = self
            .find_actor_function(
                actor,
                ResolvedObject {
                    package: Arc::clone(&actor_class.package),
                    export_index: actor_class.export_index,
                },
                "EncroachingOn",
                0,
            )
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        let handle = self
            .actor_objects
            .get(&other)
            .cloned()
            .ok_or_else(|| format!("actor {other} has no object identity"))
            .and_then(|object| {
                self.object_handle(object)
                    .map_err(|error| error.to_string())
            })?;
        let revision = self.state_revision(actor);
        let result = self
            .execute_function(
                actor,
                actor_class,
                &function,
                &[Value::Object(handle)],
                instance,
                actions,
                0,
            )
            .map_err(|error| error.to_string())?;
        if self.state_revision(actor) != revision {
            self.execute_ready_state(actor, actor_class, instance, actions)
                .map_err(|error| error.to_string())?;
        }
        match result {
            Value::Bool(stop) => Ok(stop),
            value => Err(format!("Mover.EncroachingOn returned {}", value.kind())),
        }
    }

    pub(super) fn set_actor_rotation(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        rotation: [i32; 3],
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let field = self
            .find_property(actor_class, "Rotation", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "actor property Rotation is missing".to_owned())?;
        instance.insert(field, StoredValue::Value(Value::Rotator(rotation)));
        if let Some(Some(cached)) = self.collision_actors.get_mut(actor) {
            let axes = crate::rotator_axes(rotation);
            cached.actor.rotation = Mat3::from_cols(
                Vec3::from_array(axes[0]),
                Vec3::from_array(axes[1]),
                Vec3::from_array(axes[2]),
            );
            self.reindex_cached_collision_actor(actor);
        }
        actions.push(ActorAction::SetRotation { actor, rotation });
        Ok(())
    }

    fn turn_based_actors(
        &mut self,
        actor: usize,
        location: Vec3,
        delta: [i32; 3],
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if delta[1] & 0xffff == 0 {
            return Ok(());
        }
        let axes = crate::rotator_axes([0, delta[1], 0]);
        let yaw_rotation = Mat3::from_cols(
            Vec3::from_array(axes[0]),
            Vec3::from_array(axes[1]),
            Vec3::from_array(axes[2]),
        );
        for based_actor in self.based_actors(actor)? {
            let class = self
                .actor_classes
                .get(&based_actor)
                .cloned()
                .ok_or_else(|| format!("based actor {based_actor} has no class"))?;
            let class = self
                .resolved_object(&class)
                .map_err(|error| error.to_string())?;
            let mut based_instance = self
                .instances
                .remove(&based_actor)
                .ok_or_else(|| format!("based actor {based_actor} instance is active"))?;
            let result: std::result::Result<(), String> = (|| {
                let based_location =
                    Vec3::from_array(self.actor_vector(&class, &based_instance, "Location")?);
                self.try_move_actor_inner(
                    based_actor,
                    &class,
                    (location + yaw_rotation * (based_location - location) - based_location)
                        .to_array(),
                    None,
                    false,
                    &mut based_instance,
                    Some(actions),
                )?;
                let based_rotation = self.actor_rotator(&class, &based_instance, "Rotation")?;
                self.try_move_actor_inner(
                    based_actor,
                    &class,
                    [0.0; 3],
                    Some([
                        based_rotation[0].wrapping_add(delta[0]),
                        based_rotation[1].wrapping_add(delta[1]),
                        based_rotation[2].wrapping_add(delta[2]),
                    ]),
                    false,
                    &mut based_instance,
                    Some(actions),
                )?;
                if self
                    .class_has_name(&class, "Pawn")
                    .map_err(|error| error.to_string())?
                {
                    let view_rotation =
                        self.actor_rotator(&class, &based_instance, "ViewRotation")?;
                    self.set_actor_value(
                        &class,
                        &mut based_instance,
                        "ViewRotation",
                        Value::Rotator([
                            view_rotation[0],
                            view_rotation[1].wrapping_add(delta[1]),
                            view_rotation[2],
                        ]),
                    )?;
                }
                Ok(())
            })();
            self.instances.insert(based_actor, based_instance);
            result?;
        }
        Ok(())
    }

    fn movement_hit(
        &mut self,
        current: &CollisionActor,
        delta: Vec3,
        collide_world: bool,
        current_actor: usize,
        current_instance: &mut InstanceState,
        evaluation: &mut MovementEvaluation<'_>,
    ) -> std::result::Result<(MovementHit, Vec<ActorSweep>), String> {
        let world_hit = if collide_world {
            let collision = self
                .collision
                .as_ref()
                .ok_or_else(|| "Move requires a configured BSP collision model".to_owned())?;
            sweep_world_collision(collision, current, delta)
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
        let mut hits = Vec::new();
        if (current.collide_actors || collide_world) && current.brush.is_none() {
            hits = self.actor_sweeps(
                current,
                delta,
                collide_world,
                current_actor,
                current_instance,
                evaluation,
            )?;
            hits.sort_by(|left, right| {
                left.fraction
                    .total_cmp(&right.fraction)
                    .then_with(|| left.actor.cmp(&right.actor))
            });
            if let Some(hit) = hits
                .iter()
                .find(|hit| hit.blocking && hit.fraction < blocking_hit.fraction)
            {
                blocking_hit = MovementHit {
                    fraction: hit.fraction,
                    normal: hit.normal,
                    actor: Some(hit.actor),
                    node: hit.node,
                };
            }
        }
        Ok((blocking_hit, hits))
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

    pub(super) fn update_actor_base(
        &mut self,
        actor: usize,
        base: Option<ObjectId>,
        level: Option<ObjectId>,
    ) -> DispatchResult<()> {
        let previous = self.actor_bases.insert(actor, base.clone()).flatten();
        if previous != base {
            self.unlink_actor_base(actor, previous.as_ref(), level.as_ref())?;
            self.link_actor_base(actor, base.as_ref(), level.as_ref())?;
        }
        if let Some(object) = self
            .actor_objects
            .get(&actor)
            .filter(|object| self.base_children.contains_key(*object))
            .cloned()
        {
            self.update_standing_count(&object)?;
        }
        Ok(())
    }

    fn unlink_actor_base(
        &mut self,
        actor: usize,
        base: Option<&ObjectId>,
        level: Option<&ObjectId>,
    ) -> DispatchResult<()> {
        let Some(base) = base.filter(|base| Some(*base) != level) else {
            return Ok(());
        };
        let remove_entry = self.base_children.get_mut(base).is_some_and(|children| {
            if let Ok(index) = children.binary_search(&actor) {
                children.remove(index);
            }
            children.is_empty()
        });
        if remove_entry {
            self.base_children.remove(base);
        }
        self.update_standing_count(base)
    }

    fn link_actor_base(
        &mut self,
        actor: usize,
        base: Option<&ObjectId>,
        level: Option<&ObjectId>,
    ) -> DispatchResult<()> {
        let Some(base) = base.filter(|base| Some(*base) != level) else {
            return Ok(());
        };
        let children = self.base_children.entry(base.clone()).or_default();
        if let Err(index) = children.binary_search(&actor) {
            children.insert(index, actor);
        }
        self.update_standing_count(base)
    }

    fn update_standing_count(&mut self, object: &ObjectId) -> DispatchResult<()> {
        let Some(actor) = self.object_actors.get(object).copied() else {
            return Ok(());
        };
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let Some(field) = self.find_property(&class, "StandingCount", 0)? else {
            return Ok(());
        };
        let count = self
            .base_children
            .get(object)
            .map_or(0, |children| children.len().min(usize::from(u8::MAX)) as u8);
        if let Some(instance) = self.instances.get_mut(&actor) {
            instance.insert(field, StoredValue::Value(Value::Byte(count)));
        }
        Ok(())
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

    pub(super) fn set_actor_location_placing(
        &mut self,
        actor: usize,
        actor_class: &ResolvedObject,
        instance: &mut InstanceState,
        location: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        let mut candidate = self.collision_actor(actor, actor_class, instance)?;
        let location = if self.actor_bool(actor_class, instance, "bCollideWorld")?
            || self.actor_bool(actor_class, instance, "bCollideWhenPlacing")?
        {
            let collision = self.collision.as_ref().ok_or_else(|| {
                "SetLocation requires a configured BSP collision model".to_owned()
            })?;
            let Some(location) = find_placement_location(collision, &mut candidate, location)
            else {
                return Ok(false);
            };
            location
        } else {
            location
        };
        candidate.location = location;

        self.set_actor_location(actor, actor_class, instance, location, actions)?;
        self.queue_location_touches(actor, &candidate, instance, actions)?;
        self.queue_ended_touches(actor, &candidate, location, instance, actions)?;
        Ok(true)
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
        self.unlink_actor_base(actor, current.as_ref(), level.as_ref())
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
        self.link_actor_base(actor, base.as_ref(), level.as_ref())
            .map_err(|error| error.to_string())?;
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
        current_instance: &mut InstanceState,
        evaluation: &mut MovementEvaluation<'_>,
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
            let other = self.collision_actors[actor].as_ref().unwrap().actor.clone();
            if (other.brush.is_some() && !collide_world)
                || (other.brush.is_none() && !current.collide_actors)
            {
                continue;
            }
            let Some((other_location, other_extents)) = collision_actor_world_bounds(&other) else {
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
            let own_base_blocks = !matches!(
                evaluation,
                MovementEvaluation::Real {
                    own_base_blocks: false,
                    ..
                }
            );
            if self
                .actor_is_based_on(actor, current.actor)
                .map_err(|error| error.to_string())?
                || !own_base_blocks
                    && self
                        .actor_is_based_on(current.actor, actor)
                        .map_err(|error| error.to_string())?
            {
                continue;
            }
            let hit = if delta.length_squared() < 0.00000001 {
                collision_actors_overlap(current, &other).then_some(CollisionSweep {
                    fraction: 0.0,
                    normal: Vec3::ZERO,
                    node: None,
                })
            } else {
                sweep_collision_actors(current, &other, delta)
            };
            let Some(hit) = hit else {
                continue;
            };
            let (blocking, bumping) = self.actors_block_for_movement(
                current,
                &other,
                current_actor,
                current_instance,
                evaluation,
            )?;
            hits.push(ActorSweep {
                actor,
                fraction: hit.fraction,
                normal: hit.normal,
                node: hit.node,
                blocking,
                bumping,
            });
        }
        Ok(hits)
    }

    fn actors_block_for_movement(
        &mut self,
        first: &CollisionActor,
        second: &CollisionActor,
        current_actor: usize,
        current_instance: &mut InstanceState,
        evaluation: &mut MovementEvaluation<'_>,
    ) -> std::result::Result<(bool, bool), String> {
        let blocking = actors_block(first, second);
        let second_is_mover = self.actor_is_mover(second.actor)?;
        let first_is_mover = self.actor_is_mover(first.actor)?;
        let mover = if second_is_mover {
            Some((second.actor, first.actor))
        } else if first_is_mover {
            Some((first.actor, second.actor))
        } else {
            None
        };
        let Some((mover, other)) = mover else {
            return Ok((blocking, blocking));
        };
        let MovementEvaluation::Real { actions, .. } = evaluation else {
            return Ok((blocking, blocking));
        };

        let mover_class = self
            .actor_classes
            .get(&mover)
            .cloned()
            .ok_or_else(|| format!("mover {mover} has no class"))?;
        let mover_class = self
            .resolved_object(&mover_class)
            .map_err(|error| error.to_string())?;
        let function = self
            .find_actor_function(
                mover,
                ResolvedObject {
                    package: Arc::clone(&mover_class.package),
                    export_index: mover_class.export_index,
                },
                "IsRelevant",
                0,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("mover {mover} has no IsRelevant function"))?;
        let other_object = self
            .actor_objects
            .get(&other)
            .cloned()
            .ok_or_else(|| format!("actor {other} has no object identity"))?;
        let other_handle = self
            .object_handle(other_object)
            .map_err(|error| error.to_string())?;

        if mover != current_actor && !self.instances.contains_key(&mover) {
            return Err(format!("mover {mover} instance is active"));
        }
        let inserted_current =
            mover != current_actor && !self.instances.contains_key(&current_actor);
        if inserted_current {
            self.instances
                .insert(current_actor, current_instance.clone());
        }
        let result = if mover == current_actor {
            self.execute_function(
                mover,
                &mover_class,
                &function,
                &[Value::Object(other_handle)],
                current_instance,
                actions,
                0,
            )
        } else {
            let mut mover_instance = self
                .instances
                .remove(&mover)
                .expect("mover instance was checked before relevance evaluation");
            let result = self.execute_function(
                mover,
                &mover_class,
                &function,
                &[Value::Object(other_handle)],
                &mut mover_instance,
                actions,
                0,
            );
            self.instances.insert(mover, mover_instance);
            result
        };
        if inserted_current {
            *current_instance = self
                .instances
                .remove(&current_actor)
                .ok_or_else(|| format!("moving actor {current_actor} instance was removed"))?;
        }
        match result.map_err(|error| error.to_string())? {
            Value::Bool(relevant) => Ok((blocking, blocking || relevant)),
            value => Err(format!("Mover.IsRelevant returned {}", value.kind())),
        }
    }

    fn actor_is_mover(&mut self, actor: usize) -> std::result::Result<bool, String> {
        self.actor_classes
            .get(&actor)
            .cloned()
            .map(|class| self.resolved_object(&class))
            .transpose()
            .map_err(|error| error.to_string())?
            .map(|class| self.class_has_name(&class, "Mover"))
            .transpose()
            .map_err(|error| error.to_string())
            .map(Option::unwrap_or_default)
    }

    fn dispatch_contact_bump(
        &mut self,
        mover: usize,
        other: usize,
        other_class: &ResolvedObject,
        other_instance: &mut InstanceState,
        contact_location: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let mover_class = self
            .actor_classes
            .get(&mover)
            .cloned()
            .ok_or_else(|| format!("mover {mover} has no class"))?;
        let other_object = self
            .actor_objects
            .get(&other)
            .cloned()
            .ok_or_else(|| format!("actor {other} has no object identity"))?;
        let other_handle = self
            .object_handle(other_object)
            .map_err(|error| error.to_string())?;
        let location = self
            .find_property(other_class, "Location", 0)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("actor {other} has no Location property"))?;
        if self.instances.contains_key(&other) {
            return Err(format!("moving actor {other} instance is already stored"));
        }
        let previous_location = other_instance.insert(
            location.clone(),
            StoredValue::Value(Value::Vector(contact_location.to_array())),
        );
        self.instances.insert(other, other_instance.clone());
        let result = self.dispatch_event_with_arguments(
            mover,
            mover_class.package.as_ref(),
            mover_class.export_index,
            "Bump",
            &[Value::Object(other_handle)],
        );

        *other_instance = self
            .instances
            .remove(&other)
            .ok_or_else(|| format!("moving actor {other} instance was removed"))?;
        match previous_location {
            Some(value) => {
                other_instance.insert(location, value);
            }
            None => {
                other_instance.remove(&location);
            }
        }
        actions.extend(result.map_err(|error| error.to_string())?);
        Ok(())
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

    fn queue_location_touches(
        &mut self,
        actor: usize,
        current: &CollisionActor,
        current_instance: &InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if !current.collide_actors || current.brush.is_some() {
            return Ok(());
        }
        self.ensure_collision_actors(actor, current_instance)?;
        let extents = collision_actor_world_extents(current);
        let query_minimum = current.location - extents;
        let query_maximum = current.location + extents;
        let candidate_count = self.collision_actors_by_min_x.partition_point(|&other| {
            collision_actor_min_x(&self.collision_actors, other) <= query_maximum.x
        });
        let candidates = self.collision_actors_by_min_x[..candidate_count].to_vec();
        for other in candidates {
            if other == actor || self.destroyed.contains(&other) {
                continue;
            }
            let other_collision = &self.collision_actors[other].as_ref().unwrap().actor;
            let Some((other_location, other_extents)) =
                collision_actor_world_bounds(other_collision)
                    .filter(|_| other_collision.brush.is_none())
            else {
                continue;
            };
            if !other_collision.collide_actors
                || other_location.x + other_extents.x < query_minimum.x
                || other_location.y + other_extents.y < query_minimum.y
                || other_location.y - other_extents.y > query_maximum.y
                || other_location.z + other_extents.z < query_minimum.z
                || other_location.z - other_extents.z > query_maximum.z
                || self
                    .actors_share_base_chain(actor, other)
                    .map_err(|error| error.to_string())?
                || !collision_actors_overlap(current, other_collision)
            {
                continue;
            }
            let pair = actor_pair(actor, other);
            if self.touching.insert(pair) {
                self.queue_pair_event(actions, actor, other, "Touch")?;
                self.queue_pair_event(actions, other, actor, "Touch")?;
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

    pub(super) fn has_line_of_sight(&self, eye: Vec3, target: Vec3) -> bool {
        self.collision
            .as_ref()
            .is_none_or(|collision| collision.sweep_aabb(eye, target, Vec3::ZERO).is_none())
    }

    pub(super) fn fast_trace_native(
        &mut self,
        actor_class: &ResolvedObject,
        arguments: &[Value],
        instance: &InstanceState,
    ) -> std::result::Result<Value, String> {
        let [Value::Vector(end), rest @ ..] = arguments else {
            return Err(format!(
                "FastTrace expects a trace end and an optional trace start, found {} arguments",
                arguments.len()
            ));
        };
        if rest.len() > 1 {
            return Err(format!(
                "FastTrace expects at most 2 arguments, found {}",
                arguments.len()
            ));
        }
        let start = match rest.first() {
            Some(Value::Vector(start)) => Vec3::from_array(*start),
            Some(Value::None) | None => {
                Vec3::from_array(self.actor_vector(actor_class, instance, "Location")?)
            }
            Some(value) => return Err(format!("FastTrace start is {}", value.kind())),
        };
        let end = Vec3::from_array(*end);
        if !start.is_finite() || !end.is_finite() {
            return Err("FastTrace coordinates are not finite".to_owned());
        }
        Ok(Value::Bool(self.has_line_of_sight(start, end)))
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
            let collision_actor = if actor == current_actor {
                self.collision_actor_from_fields(actor, current_instance, &fields)?
            } else {
                let instance = self
                    .instances
                    .remove(&actor)
                    .ok_or_else(|| format!("actor {actor} instance is active"))?;
                let result = self.collision_actor_from_fields(actor, &instance, &fields);
                self.instances.insert(actor, instance);
                result?
            };
            self.collision_actors[actor] = Some(CachedCollisionActor {
                actor: collision_actor,
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
        let cached = match current_instance {
            Some(instance) => self.collision_actor_from_fields(actor, instance, &fields)?,
            None => {
                let instance = self
                    .instances
                    .remove(&actor)
                    .ok_or_else(|| format!("actor {actor} instance is active"))?;
                let result = self.collision_actor_from_fields(actor, &instance, &fields);
                self.instances.insert(actor, instance);
                result?
            }
        };
        self.collision_actors[actor] = Some(CachedCollisionActor {
            actor: cached,
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

fn sweep_world_collision(
    collision: &BspCollision,
    actor: &CollisionActor,
    delta: Vec3,
) -> Option<CollisionHit> {
    if actor.brush.is_some() {
        collision_actor_world_bounds(actor).and_then(|(center, extents)| {
            collision.sweep_aabb(
                center,
                center + delta,
                (extents - Vec3::splat(0.51)).max(Vec3::ZERO),
            )
        })
    } else {
        let center = collision_actor_center(actor);
        collision.sweep_aabb(center, center + delta, collision_actor_world_extents(actor))
    }
}

fn move_hit_fraction(trace_fraction: f32, delta_length: f32, trace_padding: f32) -> f32 {
    let fraction = ((delta_length + trace_padding) * trace_fraction - trace_padding) / delta_length;
    if fraction <= MOVE_HIT_EPSILON {
        0.0
    } else {
        fraction
    }
}

#[cfg(test)]
mod world_collision_tests {
    use super::*;
    use crate::world::tests::solid_box_collision;

    #[test]
    fn aligned_cylinder_uses_native_box_extent_against_world_bsp() {
        let collision = solid_box_collision();
        let start = Vec3::new(-20.0, -20.0, 0.0);
        let end = Vec3::ZERO;
        let actor = CollisionActor {
            actor: 0,
            location: start,
            height: 1.0,
            radius: 1.0,
            width: 1.0,
            rotation: Mat3::IDENTITY,
            collide_type: 0,
            collide_actors: false,
            block_actors: false,
            block_players: false,
            player_collision: false,
            brush: None,
            pre_pivot: Vec3::ZERO,
            main_scale: Vec3::ONE,
            shape_bounds: None,
        };

        let native_extent_hit = sweep_world_collision(&collision, &actor, end - start).unwrap();
        let rounded_cylinder_hit = collision
            .sweep_cylinder(start, end, actor.radius, actor.height)
            .unwrap();
        assert!(native_extent_hit.fraction < rounded_cylinder_hit.fraction);
    }

    #[test]
    fn native_move_trace_leaves_surface_clearance() {
        let collision = solid_box_collision();
        let start = Vec3::new(-20.0, 0.0, 0.0);
        let delta = Vec3::new(20.0, 0.0, 0.0);
        let extents = Vec3::ONE;
        let trace_delta = delta + delta.normalize() * MOVE_TRACE_PADDING;
        let hit = collision
            .sweep_aabb(start, start + trace_delta, extents)
            .unwrap();
        let fraction = move_hit_fraction(hit.fraction, delta.length(), MOVE_TRACE_PADDING);
        let stopped = start + delta * fraction;

        assert!(stopped.x + extents.x <= -12.0);
        assert!(!collision.overlaps_aabb(stopped, extents));
        assert!(
            collision
                .sweep_aabb(stopped, stopped + Vec3::Y, extents)
                .is_none()
        );
    }

    #[test]
    fn find_spot_uses_trace_fraction_instead_of_a_full_extent_step() {
        let collision = solid_box_collision();
        let location = Vec3::new(11.5, 0.0, 0.0);
        let mut actor = CollisionActor {
            actor: 0,
            location,
            height: 2.0,
            radius: 2.0,
            width: 2.0,
            rotation: Mat3::IDENTITY,
            collide_type: 0,
            collide_actors: false,
            block_actors: false,
            block_players: false,
            player_collision: false,
            brush: None,
            pre_pivot: Vec3::ZERO,
            main_scale: Vec3::ONE,
            shape_bounds: None,
        };

        let placed = find_placement_location(&collision, &mut actor, location).unwrap();
        assert!(placed.x > 12.0 && placed.x < 12.2, "{placed:?}");
    }
}

fn adjust_spawn_spot(collision: &BspCollision, spot: &mut Vec3, target: Vec3, distance: f32) {
    if let Some(hit) = collision.sweep_point(*spot, target) {
        let trace_distance = target.distance(*spot);
        let hit_fraction = (hit.fraction + ACTOR_TRACE_MARGIN / trace_distance).min(1.0);
        *spot += hit.normal * (1.05 - hit_fraction) * distance;
    }
}

fn find_placement_location(
    collision: &BspCollision,
    candidate: &mut CollisionActor,
    location: Vec3,
) -> Option<Vec3> {
    candidate.location = location;
    if !bsp_placement_blocked(collision, candidate) {
        return Some(location);
    }

    let extents = collision_actor_local_extents(candidate);
    let mut adjusted = location;
    for direction in [-1.0, 1.0] {
        for (axis, distance) in [
            (Vec3::X, extents.x),
            (Vec3::Y, extents.y),
            (Vec3::Z, extents.z),
        ] {
            let target = adjusted + axis * direction * distance;
            adjust_spawn_spot(collision, &mut adjusted, target, distance);
        }
    }
    candidate.location = adjusted;
    if !bsp_placement_blocked(collision, candidate) {
        return Some(adjusted);
    }

    let maximum = extents.length() + 2.0;
    for x in [-extents.x, extents.x] {
        for y in [-extents.y, extents.y] {
            for z in [-extents.z, extents.z] {
                let target = adjusted + Vec3::new(x, y, z);
                adjust_spawn_spot(collision, &mut adjusted, target, maximum);
            }
        }
    }
    candidate.location = adjusted;
    (adjusted.distance(location) <= maximum * 1.5 && !bsp_placement_blocked(collision, candidate))
        .then_some(adjusted)
}

fn bsp_placement_blocked(collision: &BspCollision, candidate: &CollisionActor) -> bool {
    if candidate.collide_type == COLLIDE_BOX
        || candidate.collide_type == COLLIDE_SHAPE && candidate.shape_bounds.is_some()
    {
        collision.overlaps_aabb(
            collision_actor_center(candidate),
            collision_actor_world_extents(candidate),
        )
    } else {
        collision.overlaps_cylinder(candidate.location, candidate.radius, candidate.height)
    }
}
