use super::*;

pub(super) fn collision_actor_brush(
    instance: &InstanceState,
    fields: &CollisionFields,
) -> std::result::Result<Option<ObjectId>, String> {
    match instance.get(&fields.brush) {
        Some(StoredValue::Object(value)) => Ok(value.clone()),
        Some(value) => Err(format!("actor property Brush is {value:?}")),
        None => Ok(None),
    }
}

pub(super) fn collision_actor_from_fields(
    actor: usize,
    instance: &InstanceState,
    fields: &CollisionFields,
    brush: Option<Arc<BspCollision>>,
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
    let byte = |field: &ObjectId, name| match instance.get(field) {
        Some(StoredValue::Value(Value::Byte(value))) => Ok(*value),
        Some(value) => Err(format!("actor property {name} is {value:?}")),
        None => Ok(0),
    };
    let rotator = |field: &ObjectId, name| match instance.get(field) {
        Some(StoredValue::Value(Value::Rotator(value))) => Ok(*value),
        Some(value) => Err(format!("actor property {name} is {value:?}")),
        None => Ok([0; 3]),
    };
    let main_scale = match fields
        .main_scale
        .as_ref()
        .and_then(|field| instance.get(field))
    {
        Some(StoredValue::Value(Value::Struct(value))) => match value
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("Scale"))
            .map(|(_, value)| value)
        {
            Some(Value::Vector(value)) if value.iter().all(|component| component.is_finite()) => {
                Vec3::from_array(*value)
            }
            Some(value) => return Err(format!("actor property MainScale.Scale is {value:?}")),
            None => Vec3::ONE,
        },
        Some(value) => return Err(format!("actor property MainScale is {value:?}")),
        None => Vec3::ONE,
    };
    let rotation = crate::rotator_axes(rotator(&fields.rotation, "Rotation")?);
    Ok(CollisionActor {
        actor,
        location: vector(&fields.location, "Location")?,
        height: float(&fields.height, "CollisionHeight")?,
        radius: float(&fields.radius, "CollisionRadius")?,
        width: float(&fields.width, "CollisionWidth")?,
        rotation: Mat3::from_cols(
            Vec3::from_array(rotation[0]),
            Vec3::from_array(rotation[1]),
            Vec3::from_array(rotation[2]),
        ),
        collide_type: byte(&fields.collide_type, "CollideType")?,
        collide_actors: boolean(&fields.collide_actors, "bCollideActors")?,
        block_actors: boolean(&fields.block_actors, "bBlockActors")?,
        block_players: boolean(&fields.block_players, "bBlockPlayers")?,
        player_collision: fields.player_collision,
        brush,
        pre_pivot: vector(&fields.pre_pivot, "PrePivot")?,
        main_scale,
    })
}

pub(super) fn collision_actor_local_extents(actor: &CollisionActor) -> Vec3 {
    if actor.collide_type == COLLIDE_BOX {
        Vec3::new(actor.radius, actor.width, actor.height)
    } else {
        Vec3::new(actor.radius, actor.radius, actor.height)
    }
}

pub(super) fn collision_actor_world_extents(actor: &CollisionActor) -> Vec3 {
    if actor.collide_type == COLLIDE_BOX {
        actor.rotation.abs() * collision_actor_local_extents(actor)
    } else {
        collision_actor_local_extents(actor)
    }
}

pub(super) fn collision_actor_world_bounds(actor: &CollisionActor) -> Option<(Vec3, Vec3)> {
    match &actor.brush {
        Some(brush) => brush.transformed_bounds(
            actor.location,
            actor.rotation,
            actor.pre_pivot,
            actor.main_scale,
        ),
        None => Some((actor.location, collision_actor_world_extents(actor))),
    }
}

pub(super) fn transform_visual_bounds(
    minimum: Vec3,
    maximum: Vec3,
    location: Vec3,
    rotation: Mat3,
) -> (Vec3, Vec3) {
    let center = location + rotation * ((minimum + maximum) * 0.5);
    let extents = rotation.abs() * ((maximum - minimum) * 0.5);
    (center - extents, center + extents)
}

pub(super) fn sweep_collision_actors(
    current: &CollisionActor,
    other: &CollisionActor,
    delta: Vec3,
) -> Option<openhp1_physics::ActorCollisionHit> {
    if let Some(brush) = &other.brush {
        brush
            .sweep_transformed_aabb(
                current.location,
                current.location + delta,
                collision_actor_world_extents(current),
                other.location,
                other.rotation,
                other.pre_pivot,
                other.main_scale,
            )
            .map(|hit| openhp1_physics::ActorCollisionHit {
                fraction: hit.fraction,
                normal: hit.normal,
            })
    } else if other.collide_type == COLLIDE_BOX {
        sweep_box(
            current.location,
            current.location + delta,
            collision_actor_world_extents(current),
            other.location,
            collision_actor_local_extents(other),
            other.rotation,
        )
    } else if current.collide_type == COLLIDE_BOX {
        sweep_box(
            current.location,
            current.location + delta,
            collision_actor_world_extents(current),
            other.location,
            collision_actor_local_extents(other),
            Mat3::IDENTITY,
        )
    } else {
        sweep_cylinder(
            current.location,
            current.location + delta,
            current.height,
            current.radius,
            other.location,
            other.height,
            other.radius,
        )
    }
}

pub(super) fn collision_actors_overlap(first: &CollisionActor, second: &CollisionActor) -> bool {
    if first.brush.is_some() || second.brush.is_some() {
        false
    } else if second.collide_type == COLLIDE_BOX {
        boxes_overlap(
            first.location,
            collision_actor_world_extents(first),
            second.location,
            collision_actor_local_extents(second),
            second.rotation,
        )
    } else if first.collide_type == COLLIDE_BOX {
        boxes_overlap(
            second.location,
            collision_actor_world_extents(second),
            first.location,
            collision_actor_local_extents(first),
            first.rotation,
        )
    } else {
        cylinders_overlap(
            first.location,
            first.height,
            first.radius,
            second.location,
            second.height,
            second.radius,
        )
    }
}

pub(super) fn collision_actor_min_x(actors: &[Option<CachedCollisionActor>], actor: usize) -> f32 {
    let actor = &actors[actor].as_ref().unwrap().actor;
    let (location, extents) = collision_actor_world_bounds(actor).unwrap();
    location.x - extents.x
}

pub(super) fn actors_block(first: &CollisionActor, second: &CollisionActor) -> bool {
    if first.player_collision || second.player_collision {
        first.block_players && second.block_players
    } else {
        first.block_actors && second.block_actors
    }
}

pub(super) fn placement_blocked(first: &CollisionActor, second: &CollisionActor) -> bool {
    actors_block(first, second) && collision_actors_overlap(first, second)
}

pub(super) fn actor_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

pub(super) fn within_sight(
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

pub(super) fn smooth_remaining_delta(delta: Vec3, normal: Vec3, fraction: f32) -> Option<Vec3> {
    if fraction >= 1.0 {
        return None;
    }
    let aligned = (delta - normal * delta.dot(normal)) * (1.0 - fraction);
    (delta.dot(aligned) >= 0.0).then_some(aligned)
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
            width: 10.0,
            rotation: Mat3::IDENTITY,
            collide_type: 0,
            collide_actors: true,
            block_actors: false,
            block_players,
            player_collision: actor == 0,
            brush: None,
            pre_pivot: Vec3::ZERO,
            main_scale: Vec3::ONE,
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
        assert!(!placement_blocked(&player, &trigger));
        assert!(placement_blocked(&player, &wall));
    }

    #[test]
    fn hp1_box_collision_uses_width_and_actor_rotation() {
        let pawn = collision_actor(0, Vec3::new(20.0, -30.0, 0.0), true);
        let mut wall = collision_actor(1, Vec3::ZERO, true);
        wall.radius = 10.0;
        wall.width = 2.0;
        wall.rotation = Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
        wall.collide_type = COLLIDE_BOX;

        assert!(sweep_collision_actors(&pawn, &wall, Vec3::Y * 60.0).is_none());
        let mut pawn = pawn;
        pawn.location.x = 0.0;
        assert!(sweep_collision_actors(&pawn, &wall, Vec3::Y * 60.0).is_some());
    }

    #[test]
    fn visual_bounds_follow_actor_translation_and_rotation() {
        let (minimum, maximum) = transform_visual_bounds(
            Vec3::new(-1.0, -2.0, -3.0),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(10.0, 20.0, 30.0),
            Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2),
        );
        assert!(minimum.abs_diff_eq(Vec3::new(8.0, 19.0, 27.0), 1.0e-5));
        assert!(maximum.abs_diff_eq(Vec3::new(12.0, 21.0, 33.0), 1.0e-5));
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

    #[test]
    fn smooth_move_slides_the_untraveled_distance_along_a_wall() {
        assert_eq!(
            smooth_remaining_delta(Vec3::new(2.0, 4.0, 0.0), -Vec3::X, 0.25),
            Some(Vec3::new(0.0, 3.0, 0.0))
        );
        assert_eq!(smooth_remaining_delta(Vec3::X, -Vec3::X, 1.0), None);
    }
}
