use glam::Vec3;
use openhp1_map::PolyFlags;

use super::movement::MovementHit;
use super::*;

mod dynamics;
mod events;
mod kinematics;

use kinematics::*;

const PHYSICS_STEP: f32 = 0.02;
pub(super) const PHYS_NONE: u8 = 0;
pub(super) const PHYS_WALKING: u8 = 1;
pub(super) const PHYS_FALLING: u8 = 2;
pub(super) const PHYS_SWIMMING: u8 = 3;
pub(super) const PHYS_FLYING: u8 = 4;
pub(super) const PHYS_ROTATING: u8 = 5;
const PHYS_PROJECTILE: u8 = 6;
const PHYS_ROLLING: u8 = 7;
const PHYS_INTERPOLATING: u8 = 8;
const PHYS_MOVING_BRUSH: u8 = 9;
const PHYS_TRAILER: u8 = 11;
const STEP_DOWN_FACTOR: f32 = 1.3;
const WALKABLE_FLOOR_Z: f32 = 7071.0 / 10_000.0;

fn direction_pitch(direction: Vec3) -> i32 {
    (direction.z.atan2(direction.x.hypot(direction.y)) * (65_536.0 / std::f32::consts::TAU)) as i32
}

fn should_slide_walking_collision(pushable: bool, normal: Vec3) -> bool {
    !pushable && normal.z.abs() < 0.2
}

pub(super) fn two_wall_adjust(
    delta: Vec3,
    hit_normal: Vec3,
    old_hit_normal: Vec3,
    desired_direction: Vec3,
    hit_fraction: f32,
) -> Vec3 {
    if old_hit_normal.dot(hit_normal) <= 0.0 {
        let direction = hit_normal.cross(old_hit_normal).normalize_or_zero();
        let adjusted = direction * delta.dot(direction) * (1.0 - hit_fraction);
        if desired_direction.dot(adjusted) < 0.0 {
            -adjusted
        } else {
            adjusted
        }
    } else {
        let adjusted = (delta - hit_normal * delta.dot(hit_normal)) * (1.0 - hit_fraction);
        if adjusted.dot(desired_direction) > 0.0 {
            adjusted
        } else {
            Vec3::ZERO
        }
    }
}

pub(super) struct ZonePhysics {
    pub(super) gravity: Vec3,
    velocity: Vec3,
    ground_friction: f32,
    fluid_friction: f32,
    terminal_velocity: f32,
    pub(in crate::world) water: bool,
    pub(in crate::world) pain: bool,
    pub(in crate::world) damage_type: Option<String>,
}

impl ScriptRuntime {
    pub(super) fn tick_turn_to(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<bool, String> {
        let physics = self.actor_byte(class, instance, "Physics")?;
        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        let focus = Vec3::from_array(self.actor_vector(class, instance, "Focus")?);
        let direction = focus - location;
        let yaw = (direction.y.atan2(direction.x) * (65_536.0 / std::f32::consts::TAU)) as i32;
        let pitch = if physics == PHYS_WALKING {
            0
        } else {
            direction_pitch(direction)
        };
        self.set_actor_value(
            class,
            instance,
            "DesiredRotation",
            Value::Rotator([pitch, yaw, 0]),
        )?;
        let rotation = self.actor_rotator(class, instance, "Rotation")?;
        let difference = yaw.wrapping_sub(rotation[1]) & 0xffff;
        Ok(!(2_000..=0x1_0000 - 2_000).contains(&difference))
    }

    pub(super) fn tick_turn_toward(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
    ) -> std::result::Result<bool, String> {
        let Some(target) = self.actor_object(class, instance, "FaceTarget")? else {
            return Ok(true);
        };
        let Some(target) = self.object_actors.get(&target).copied() else {
            return Ok(true);
        };
        let focus = self.other_actor_vector(target, "Location")?;
        self.set_actor_value(class, instance, "Focus", Value::Vector(focus))?;
        self.tick_turn_to(class, instance)
    }

    pub(super) fn tick_move_to(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
    ) -> std::result::Result<bool, String> {
        let timer = self.actor_float_any(class, instance, "MoveTimer")? - elapsed;
        self.set_actor_value(class, instance, "MoveTimer", Value::Float(timer))?;
        let physics = self.actor_byte(class, instance, "Physics")?;
        let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
        let destination = Vec3::from_array(self.actor_vector(class, instance, "Destination")?);
        let velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        let delta = destination - location;
        let yaw = (delta.y.atan2(delta.x) * (65_536.0 / std::f32::consts::TAU)) as i32;
        let pitch = if physics == PHYS_WALKING {
            0
        } else {
            direction_pitch(delta)
        };
        self.set_actor_value(
            class,
            instance,
            "DesiredRotation",
            Value::Rotator([pitch, yaw, 0]),
        )?;
        let Some(direction) = move_to_direction(physics, delta, velocity, timer) else {
            return Ok(true);
        };
        let acceleration_direction = if matches!(physics, PHYS_SWIMMING | PHYS_FLYING)
            && !self.actor_bool(class, instance, "bCanStrafe")?
        {
            Vec3::from_array(
                crate::rotator_axes(self.actor_rotator(class, instance, "Rotation")?)[0],
            )
        } else {
            direction
        };
        let acceleration =
            acceleration_direction * self.actor_float(class, instance, "AccelRate")?;
        self.set_actor_value(
            class,
            instance,
            "Acceleration",
            Value::Vector(acceleration.to_array()),
        )?;
        Ok(false)
    }

    pub(super) fn tick_move_toward(
        &mut self,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
    ) -> std::result::Result<bool, String> {
        let Some(target) = self.actor_object(class, instance, "MoveTarget")? else {
            return Ok(true);
        };
        let Some(target) = self.object_actors.get(&target).copied() else {
            return Ok(true);
        };
        let destination = self.other_actor_vector(target, "Location")?;
        self.set_actor_value(class, instance, "Destination", Value::Vector(destination))?;
        self.set_actor_value(class, instance, "Focus", Value::Vector(destination))?;
        self.tick_move_to(class, instance, elapsed)
    }

    pub(super) fn tick_physics(
        &mut self,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        let mut actors = self.actor_classes.keys().copied().collect::<Vec<_>>();
        actors.sort_unstable();
        for actor in actors {
            self.tick_actor_physics_by_id(actor, delta_time, actions)?;
        }
        Ok(())
    }

    pub(super) fn tick_actor_physics_by_id(
        &mut self,
        actor: usize,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> DispatchResult<()> {
        if self.destroyed.contains(&actor) || self.physics_ticked.contains(&actor) {
            return Ok(());
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
        let mode = self
            .actor_byte(&class, &instance, "Physics")
            .map_err(|message| DispatchError::UnresolvedObject { message })?;
        let result = self.tick_actor_physics(actor, &class, &mut instance, delta_time, actions);
        self.instances.insert(actor, instance);
        self.physics_ticked.insert(actor);
        match result {
            Ok(()) => {
                self.failed_physics.remove(&actor);
            }
            Err(message) if self.failed_physics.insert(actor, mode) != Some(mode) => {
                actions.push(ActorAction::DeferredCall {
                    actor,
                    message: format!("Physics: {message}"),
                });
            }
            Err(_) => {}
        }
        Ok(())
    }

    pub(super) fn tick_actor_physics(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        delta_time: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        let summary = class.package.summary();
        let interpolation_manager = summary
            .name(summary.exports[class.export_index].object_name)
            .eq_ignore_ascii_case("InterpolationManager");
        if !interpolation_manager && self.actor_byte(class, instance, "Physics")? == PHYS_NONE {
            return Ok(());
        }
        let pawn = self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?;
        let maximum_step = if pawn { delta_time } else { PHYSICS_STEP };
        let mut time_left = delta_time;
        while time_left > 0.0 && !self.destroyed.contains(&actor) {
            let elapsed = time_left.min(maximum_step);
            time_left -= maximum_step;
            if interpolation_manager {
                self.tick_interpolation_manager(actor, class, instance, elapsed, actions)?;
                continue;
            }
            let mode = self.actor_byte(class, instance, "Physics")?;
            if mode == PHYS_NONE {
                continue;
            }
            let old_velocity = if pawn && matches!(mode, PHYS_FALLING | PHYS_SWIMMING | PHYS_FLYING)
            {
                Vec3::from_array(self.actor_vector(class, instance, "Velocity")?)
            } else {
                Vec3::ZERO
            };
            match mode {
                PHYS_WALKING => {
                    self.tick_walking(actor, class, instance, elapsed, actions)?;
                }
                PHYS_FALLING => {
                    self.tick_falling(actor, class, instance, elapsed, actions)?;
                }
                PHYS_SWIMMING => {
                    self.tick_flying(actor, class, instance, elapsed, actions, true)?;
                }
                PHYS_FLYING => {
                    self.tick_flying(actor, class, instance, elapsed, actions, false)?;
                }
                PHYS_ROTATING => {}
                PHYS_PROJECTILE => {
                    self.tick_projectile(actor, class, instance, elapsed, actions)?;
                }
                PHYS_ROLLING => {
                    self.tick_rolling(actor, class, instance, elapsed, actions)?;
                }
                PHYS_INTERPOLATING => {
                    self.tick_interpolating(actor, class, instance, elapsed, actions)?;
                }
                PHYS_MOVING_BRUSH => {
                    self.tick_moving_brush(actor, class, instance, elapsed, actions)?;
                }
                PHYS_TRAILER => {
                    self.tick_trailer(actor, class, instance, actions)?;
                }
                _ => return Err(format!("physics mode {mode} is not implemented")),
            }
            self.tick_rotating(actor, class, instance, elapsed, old_velocity, actions)?;
        }
        Ok(())
    }

    fn tick_walking(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        elapsed: f32,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        if !self
            .class_has_name(class, "Pawn")
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let old_location = self.actor_vector(class, instance, "Location")?;
        let Some(zone) = self.zone_physics(Vec3::from_array(old_location), actor, instance)? else {
            self.fell_out_of_world(actor, class, instance, actions)?;
            return Ok(());
        };
        self.set_actor_value(class, instance, "OldLocation", Value::Vector(old_location))?;
        self.set_actor_value(class, instance, "bJustTeleported", Value::Bool(false))?;

        let player = self
            .class_has_name(class, "PlayerPawn")
            .map_err(|error| error.to_string())?;
        let player_walking = player
            && self
                .optional_actor_bool(class, instance, "bIsWalking")?
                .unwrap_or(false);
        let mut velocity = Vec3::from_array(self.actor_vector(class, instance, "Velocity")?);
        velocity.z = 0.0;
        let mut acceleration =
            Vec3::from_array(self.actor_vector(class, instance, "Acceleration")?);
        if acceleration.length_squared() > 0.0001 {
            let mut acceleration_rate = self.actor_float(class, instance, "AccelRate")?;
            if player_walking {
                acceleration_rate *= 0.3;
            }
            let acceleration_speed = acceleration.length();
            let acceleration_direction = acceleration / acceleration_speed;
            if acceleration_speed > acceleration_rate {
                acceleration = acceleration_direction * acceleration_rate;
                self.set_actor_value(
                    class,
                    instance,
                    "Acceleration",
                    Value::Vector(acceleration.to_array()),
                )?;
            }
            let speed = velocity.length();
            velocity -=
                (velocity - acceleration_direction * speed) * (zone.ground_friction * elapsed);
        } else {
            let speed = velocity.length();
            if speed > 0.0 {
                let new_speed = (speed - speed * zone.ground_friction * 2.0 * elapsed).max(0.0);
                velocity *= new_speed / speed;
            }
        }
        velocity += acceleration * elapsed;

        let mut maximum_speed = self.actor_float(class, instance, "GroundSpeed")?;
        if !player {
            maximum_speed *= self.actor_float(class, instance, "DesiredSpeed")?;
        } else if player_walking {
            maximum_speed *= 0.3;
        }
        let speed = velocity.length();
        if speed > maximum_speed && speed > 0.0 {
            velocity *= maximum_speed / speed;
        }
        velocity.z = 0.0;
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;

        let gravity_direction = if zone.gravity.z > 0.0 { 1.0 } else { -1.0 };
        let step_height = self.actor_float(class, instance, "MaxStepHeight")?;
        let step_up = Vec3::new(0.0, 0.0, -gravity_direction * step_height);
        let step_down = Vec3::new(0.0, 0.0, gravity_direction * step_height * STEP_DOWN_FACTOR);
        let movement_velocity = velocity + zone.velocity * (elapsed * 25.0);
        if has_horizontal_movement(movement_velocity) {
            let mut time_left = elapsed;
            for _ in 0..5 {
                if time_left <= 0.0 {
                    break;
                }
                let mut move_delta = movement_velocity * time_left;
                self.try_move_actor(actor, class, step_up.to_array(), instance, actions)?;
                let mut hit =
                    self.try_move_actor(actor, class, move_delta.to_array(), instance, actions)?;
                time_left -= time_left * hit.fraction;
                move_delta = movement_velocity * time_left;
                self.try_move_actor(actor, class, (-step_up).to_array(), instance, actions)?;

                if hit.fraction < f32::EPSILON {
                    hit = self.try_move_actor(
                        actor,
                        class,
                        move_delta.to_array(),
                        instance,
                        actions,
                    )?;
                    time_left -= time_left * hit.fraction;
                }
                if hit.fraction < 1.0 {
                    if player && self.try_mount(actor, class, instance, hit, actions)? {
                        return Ok(());
                    }
                    let pushable = if let (true, Some(other)) = (player, hit.actor)
                        && self
                            .actor_has_class(other, "Decoration")
                            .map_err(|error| error.to_string())?
                        && self.other_actor_bool(other, "bPushable")?
                        && hit.normal.dot(move_delta) < -0.9
                    {
                        Some(other)
                    } else {
                        None
                    };
                    if should_slide_walking_collision(pushable.is_some(), hit.normal) {
                        self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                        let aligned = (move_delta - hit.normal * move_delta.dot(hit.normal))
                            * (1.0 - hit.fraction);
                        if move_delta.dot(aligned) >= 0.0 {
                            hit = self.try_move_actor(
                                actor,
                                class,
                                aligned.to_array(),
                                instance,
                                actions,
                            )?;
                            time_left -= time_left * hit.fraction;
                            if hit.fraction < 1.0 {
                                self.call_hit_wall(
                                    actor, class, instance, hit.normal, hit.actor, actions,
                                )?;
                            }
                        } else {
                            time_left = 0.0;
                        }
                    } else if let Some(other) = pushable {
                        let mass = self.actor_float(class, instance, "Mass")?;
                        let other_mass = self.other_actor_float(other, "Mass")?;
                        self.set_actor_value(
                            class,
                            instance,
                            "bJustTeleported",
                            Value::Bool(true),
                        )?;
                        velocity *= mass / (mass + other_mass);
                        self.set_actor_value(
                            class,
                            instance,
                            "Velocity",
                            Value::Vector(velocity.to_array()),
                        )?;
                        self.call_hit_wall(actor, class, instance, hit.normal, hit.actor, actions)?;
                        time_left = 0.0;
                    }
                }

                if self.actor_byte(class, instance, "Physics")? != PHYS_WALKING {
                    return Ok(());
                }
                if !self.walk_to_floor(actor, class, instance, step_down, actions)? {
                    return Ok(());
                }
            }
        } else {
            self.walk_to_floor(actor, class, instance, step_down, actions)?;
        }

        if !self.actor_bool(class, instance, "bJustTeleported")? {
            let location = Vec3::from_array(self.actor_vector(class, instance, "Location")?);
            velocity = (location - Vec3::from_array(old_location)) / elapsed;
        }
        velocity.z = 0.0;
        self.set_actor_value(
            class,
            instance,
            "Velocity",
            Value::Vector(velocity.to_array()),
        )?;
        Ok(())
    }

    fn walk_to_floor(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        step_down: Vec3,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<bool, String> {
        let floor = self.test_move_actor(actor, class, step_down.to_array(), instance)?;
        if floor.fraction == 1.0 || floor.normal.z < WALKABLE_FLOOR_Z {
            self.start_falling(actor, class, instance, actions)?;
            return Ok(false);
        }
        let floor = self.try_move_actor(actor, class, step_down.to_array(), instance, actions)?;
        if floor.fraction != 1.0 {
            let base = floor
                .actor
                .and_then(|actor| self.actor_objects.get(&actor).cloned());
            self.set_actor_base(actor, class, instance, base, actions)?;
        }
        Ok(true)
    }

    fn start_falling(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        self.set_actor_physics(actor, class, instance, PHYS_FALLING, actions)?;
        self.set_actor_base(actor, class, instance, None, actions)
    }

    pub(in crate::world) fn set_actor_physics(
        &mut self,
        actor: usize,
        class: &ResolvedObject,
        instance: &mut InstanceState,
        physics: u8,
        actions: &mut Vec<ActorAction>,
    ) -> std::result::Result<(), String> {
        self.set_actor_value(class, instance, "Physics", Value::Byte(physics))?;
        actions.push(ActorAction::SetPhysics { actor, physics });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_can_slide_along_non_pushable_actor_collision() {
        assert!(should_slide_walking_collision(false, Vec3::X));
        assert!(!should_slide_walking_collision(true, Vec3::X));
        assert!(!should_slide_walking_collision(false, Vec3::Z));
    }

    #[test]
    fn two_wall_adjustment_matches_original_corner_branches() {
        let old_normal = Vec3::new(1.0, -3.0, 1.0).normalize();
        let hit_normal = Vec3::new(-3.0, 0.0, 1.0).normalize();
        let desired = Vec3::NEG_Z;
        let delta = desired - old_normal * desired.dot(old_normal);

        let adjusted = two_wall_adjust(delta, hit_normal, old_normal, desired, 0.0);

        assert!(adjusted.z < 0.0);
        assert!(adjusted.dot(old_normal).abs() < 1.0e-6);
        assert!(adjusted.dot(hit_normal).abs() < 1.0e-6);

        let hit_normal = Vec3::new(1.0, 1.0, 0.0).normalize();
        let adjusted = two_wall_adjust(Vec3::Y, hit_normal, Vec3::Y, Vec3::Y, 0.0);
        assert!(adjusted.abs_diff_eq(Vec3::new(-0.5, 0.5, 0.0), 1.0e-6));
        assert_eq!(
            two_wall_adjust(Vec3::Y, hit_normal, Vec3::Y, Vec3::X, 0.0),
            Vec3::ZERO
        );
    }

    #[test]
    fn upward_targets_use_positive_ue1_pitch() {
        assert_eq!(direction_pitch(Vec3::Z), 16_384);
    }
}
