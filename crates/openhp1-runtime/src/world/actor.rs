use super::state::{event_disabled, set_event_disabled};
use super::*;

mod player;
mod tick;

pub use player::{PlayerTravelState, PlayerUiState};
pub(super) use tick::decode_latent_action;
use tick::{
    particle_bool, particle_byte, particle_color, particle_float, particle_int, particle_scalar,
    particle_vector,
};

#[cfg(test)]
pub(super) use tick::{advance_lifespan, update_touching_array};

impl ScriptRuntime {
    pub fn new(game_root: impl AsRef<Path>) -> DispatchResult<Self> {
        Self::with_packages(PackageStore::scan_game_root(game_root)?)
    }

    pub fn new_with_settings_dir(
        game_root: impl AsRef<Path>,
        settings_dir: impl AsRef<Path>,
    ) -> DispatchResult<Self> {
        Self::with_packages(PackageStore::scan_game_root_with_settings_dir(
            game_root,
            settings_dir,
        )?)
    }

    fn with_packages(packages: PackageStore) -> DispatchResult<Self> {
        Ok(Self {
            packages,
            console_command_host: None,
            scripts: HashMap::default(),
            function_lookups: HashMap::default(),
            state_lookups: HashMap::default(),
            instances: HashMap::default(),
            object_instances: HashMap::default(),
            host_console_instance: HashMap::default(),
            class_defaults: HashMap::default(),
            class_relations: HashMap::default(),
            fields: HashMap::default(),
            resolved_references: HashMap::default(),
            zero_values: HashMap::default(),
            frame_arguments: HashMap::default(),
            frame_zero_values: HashMap::default(),
            struct_members: HashMap::default(),
            actor_classes: HashMap::default(),
            actor_states: HashMap::default(),
            state_frames: HashMap::default(),
            state_revisions: HashMap::default(),
            active_state_actor: None,
            pending_latent: None,
            state_resumes: 0,
            tick_functions: HashMap::default(),
            failed_ticks: HashSet::default(),
            failed_physics: HashMap::default(),
            physics_ticked: HashSet::default(),
            disabled_events: HashMap::default(),
            object_actors: HashMap::default(),
            actor_objects: HashMap::default(),
            destroyed: HashSet::default(),
            timers: HashMap::default(),
            timer_callbacks: 0,
            random_state: 0x6d2b_79f5,
            object_handles: HashMap::default(),
            handle_objects: Vec::new(),
            next_actor: 0,
            collision: None,
            reach_specs: Vec::new(),
            level_package: None,
            level_info: None,
            player_actor: None,
            player_alt_fire_pressed: false,
            player_space_pressed: false,
            animation_sequences: HashMap::default(),
            actor_bone_names: HashMap::default(),
            actor_bone_positions: HashMap::default(),
            actor_visual_bounds: HashMap::default(),
            animation_channels: HashMap::default(),
            animation_commands: HashMap::default(),
            animating: HashSet::default(),
            sound_channels: HashMap::default(),
            player_probe_touching: HashSet::default(),
            collision_fields: HashMap::default(),
            brush_collisions: HashMap::default(),
            collision_actors: Vec::new(),
            collision_actors_by_min_x: Vec::new(),
            actor_bases: HashMap::default(),
            base_children: HashMap::default(),
            touching: HashSet::default(),
        })
    }

    pub fn set_console_command_host(&mut self, host: impl ConsoleCommandHost + 'static) {
        self.console_command_host = Some(Box::new(host));
    }

    pub fn set_collision(
        &mut self,
        collision: Arc<BspCollision>,
        level_package: impl AsRef<Path>,
    ) -> DispatchResult<()> {
        let package = self.packages.load_path(level_package)?;
        let level_export = package
            .summary()
            .exports
            .iter()
            .position(|export| package.summary().class_name(export) == Some("Level"))
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "level package has no Level export".to_owned(),
            })?;
        let level = Level::decode(&package, level_export).map_err(|error| {
            DispatchError::UnresolvedObject {
                message: error.to_string(),
            }
        })?;
        self.reach_specs.clear();
        for (index, spec) in level.reach_specs.into_iter().enumerate() {
            let Some(start) = self.packages.resolve(&package, spec.start_actor)? else {
                continue;
            };
            let Some(end) = self.packages.resolve(&package, spec.end_actor)? else {
                continue;
            };
            self.reach_specs.push(NavigationReachSpec {
                index,
                distance: spec.distance,
                start: object_id(&start.package, start.export_index),
                end: object_id(&end.package, end.export_index),
                collision_radius: spec.collision_radius,
                collision_height: spec.collision_height,
                pruned: spec.pruned,
            });
        }
        self.collision = Some(collision);
        self.level_package = Some(Arc::clone(&package.summary().source));
        Ok(())
    }

    pub fn set_actor_animation_sequences(
        &mut self,
        actor: usize,
        sequences: impl IntoIterator<Item = (String, String, f32, usize, Vec<(f32, String)>)>,
    ) -> DispatchResult<()> {
        self.animation_sequences.insert(
            actor,
            sequences
                .into_iter()
                .map(|(sequence, group, rate, frame_count, notifications)| {
                    (
                        sequence.to_ascii_lowercase(),
                        AnimationSequence {
                            group,
                            rate,
                            frame_count,
                            notifications,
                        },
                    )
                })
                .collect(),
        );
        self.synchronize_animation_command(actor)
            .map_err(|message| DispatchError::UnresolvedObject { message })
    }

    pub fn set_actor_bone_names(&mut self, actor: usize, bones: impl IntoIterator<Item = String>) {
        self.actor_bone_names
            .insert(actor, bones.into_iter().collect());
    }

    pub fn set_actor_bone_positions(
        &mut self,
        actor: usize,
        positions: impl IntoIterator<Item = [f32; 3]>,
    ) {
        self.actor_bone_positions
            .insert(actor, positions.into_iter().collect());
    }

    pub fn set_actor_weapon_pose(
        &mut self,
        actor: usize,
        location: [f32; 3],
        rotation: [i32; 3],
    ) -> DispatchResult<()> {
        if !location.into_iter().all(f32::is_finite) {
            return Err(DispatchError::UnresolvedObject {
                message: format!("actor {actor} weapon location is not finite"),
            });
        }
        let class = self
            .actor_classes
            .get(&actor)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor })?;
        let class = self.resolved_object(&class)?;
        let location_field = self
            .find_property(&class, "WeaponLoc", 0)
            .map_err(|error| DispatchError::UnresolvedObject {
                message: error.to_string(),
            })?;
        let rotation_field = self
            .find_property(&class, "WeaponRot", 0)
            .map_err(|error| DispatchError::UnresolvedObject {
                message: error.to_string(),
            })?;
        let Some(instance) = self.instances.get_mut(&actor) else {
            return Err(DispatchError::ActiveActorContext { actor });
        };
        if let Some(field) = location_field {
            instance.insert(field, StoredValue::Value(Value::Vector(location)));
        }
        if let Some(field) = rotation_field {
            instance.insert(field, StoredValue::Value(Value::Rotator(rotation)));
        }
        Ok(())
    }

    pub fn set_actor_visual_bounds(
        &mut self,
        actor: usize,
        minimum: [f32; 3],
        maximum: [f32; 3],
    ) -> DispatchResult<()> {
        if !self.actor_classes.contains_key(&actor) {
            return Err(DispatchError::UnregisteredActor { actor });
        }
        let minimum = Vec3::from_array(minimum);
        let maximum = Vec3::from_array(maximum);
        if !minimum.is_finite() || !maximum.is_finite() || minimum.cmpgt(maximum).any() {
            return Err(DispatchError::UnresolvedObject {
                message: format!(
                    "actor {actor} visual bounds {minimum:?}..{maximum:?} are invalid"
                ),
            });
        }
        self.actor_visual_bounds.insert(actor, (minimum, maximum));
        self.update_cached_collision_shape_bounds(actor, Some((minimum, maximum)));
        Ok(())
    }

    pub fn clear_actor_visual_bounds(&mut self, actor: usize) {
        self.actor_visual_bounds.remove(&actor);
        self.update_cached_collision_shape_bounds(actor, None);
    }

    pub fn register_actor(
        &mut self,
        actor: usize,
        actor_package: impl AsRef<Path>,
        actor_export: usize,
        class_package: impl AsRef<Path>,
        class_export: usize,
    ) -> DispatchResult<()> {
        self.next_actor = self.next_actor.max(actor.saturating_add(1));
        let actor_package = self.packages.load_path(actor_package)?;
        let actor_entry = actor_package.summary().exports.get(actor_export).ok_or(
            openhp1_package::Error::InvalidExportIndex {
                package: Arc::clone(&actor_package.summary().source),
                index: actor_export,
                export_count: actor_package.summary().exports.len(),
            },
        )?;
        let class = ResolvedObject {
            package: self.packages.load_path(class_package)?,
            export_index: class_export,
        };
        let object = object_id(&actor_package, actor_export);
        self.object_handle(object.clone())?;
        self.object_actors.insert(object.clone(), actor);
        self.actor_objects.insert(actor, object.clone());
        self.actor_classes
            .insert(actor, object_id(&class.package, class.export_index));
        if self.level_info.is_none() && self.class_has_name(&class, "LevelInfo")? {
            self.level_info = Some(actor);
        }
        if self.player_actor.is_none() && self.class_has_name(&class, "PlayerPawn")? {
            self.player_actor = Some(actor);
        }

        let mut instance = self.load_class_defaults(&class, 0)?;
        let mut reader = actor_package.export_reader(actor_export)?;
        let stack = reader.read_object_stack(actor_entry.object_flags)?;
        let state = stack
            .and_then(|stack| {
                (stack.function != ObjectReference::None)
                    .then_some(stack.function)
                    .or((stack.state != ObjectReference::None).then_some(stack.state))
            })
            .map(|state| self.packages.resolve(&actor_package, state))
            .transpose()?
            .flatten();
        let state_name = state.as_ref().map(|state| {
            state
                .package
                .summary()
                .name(state.package.summary().exports[state.export_index].object_name)
                .to_owned()
        });
        if let (Some(stack), Some(state)) = (stack, state.as_ref())
            && stack.function != ObjectReference::None
            && matches!(self.script(state)?.metadata, ScriptMetadata::State(_))
        {
            let script = self.script(state)?;
            let offset = stack
                .bytecode_offset
                .and_then(|offset| usize::try_from(offset).ok())
                .filter(|offset| *offset <= script.bytecode.bytes.len())
                .ok_or_else(|| DispatchError::InvalidStateLabel {
                    state: state
                        .package
                        .summary()
                        .name(state.package.summary().exports[state.export_index].object_name)
                        .to_owned(),
                    label: format!("#{}", stack.bytecode_offset.unwrap_or(-1)),
                    length: script.bytecode.bytes.len(),
                })?;
            self.state_frames.insert(
                actor,
                StateFrame {
                    state: object_id(&state.package, state.export_index),
                    frame: FrameSnapshot::at(offset),
                    latent: decode_latent_action(stack.latent_action, actor),
                },
            );
        }
        if let Some(stack) = stack {
            for (index, event) in PROBE_EVENTS.iter().enumerate() {
                if stack.probe_mask & (1_u64 << index) != 0 {
                    set_event_disabled(
                        &mut self.disabled_events,
                        actor,
                        state_name.as_deref(),
                        event,
                        true,
                    );
                }
            }
        }
        self.actor_states.insert(actor, state_name);
        self.refresh_tick_actor(actor, &class)?;
        self.apply_properties(&class, &actor_package, &mut reader, &mut instance)?;
        // UE1 starts desired turning from the actor's spawn orientation.
        if let Some(rotation) = self
            .find_property(&class, "Rotation", 0)?
            .and_then(|field| instance.get(&field))
            .cloned()
            && let Some(desired) = self.find_property(&class, "DesiredRotation", 0)?
        {
            instance.insert(desired, rotation);
        }
        let base =
            self.find_property(&class, "Base", 0)?
                .and_then(|field| match instance.get(&field) {
                    Some(StoredValue::Object(base)) => base.clone(),
                    _ => None,
                });
        let level = self
            .actor_object(&class, &instance, "Level")
            .map_err(|message| DispatchError::UnresolvedObject { message })?;
        self.instances.insert(actor, instance);
        self.update_actor_base(actor, base, level)?;
        Ok(())
    }

    pub fn player_actor(&self) -> Option<usize> {
        self.player_actor
    }

    pub fn active_actor_count(&self) -> usize {
        self.actor_classes
            .len()
            .saturating_sub(self.destroyed.len())
    }

    pub fn particle_emitters(&mut self) -> DispatchResult<Vec<ParticleEmitter>> {
        let actors = self
            .actor_classes
            .iter()
            .map(|(&actor, class)| (actor, class.clone()))
            .collect::<Vec<_>>();
        let winds = self.particle_winds(&actors)?;
        let mut emitters = Vec::new();
        for (actor, class) in actors {
            if self.destroyed.contains(&actor) {
                continue;
            }
            let class = self.resolved_object(&class)?;
            if !self.class_has_name(&class, "ParticleFX")? {
                continue;
            }
            let instance = self
                .instances
                .get(&actor)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor })?;
            let textures = match self.instance_property(&class, &instance, "Textures")? {
                Some(StoredValue::Array(values)) => values
                    .into_iter()
                    .filter_map(|value| match value {
                        StoredValue::Object(Some(object)) => Some(ParticleTexture {
                            package: object.package.to_string(),
                            export_index: object.export_index,
                        }),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let pattern = self.particle_pattern(&class, &instance)?;
            let owner = match self.instance_property(&class, &instance, "Owner")? {
                Some(StoredValue::Object(Some(owner))) => self.object_actors.get(&owner).copied(),
                _ => None,
            };
            let velocity_relative =
                particle_bool(self.instance_property(&class, &instance, "bVelocityRelative")?);
            let owner_velocity = if velocity_relative {
                owner
                    .map(|owner| self.other_actor_vector(owner, "Velocity"))
                    .transpose()
                    .map_err(|message| DispatchError::UnresolvedObject { message })?
                    .unwrap_or([0.0; 3])
            } else {
                [0.0; 3]
            };
            let gravity_modifier =
                particle_scalar(self.instance_property(&class, &instance, "GravityModifier")?);
            let location = Vec3::from_array(particle_vector(
                self.instance_property(&class, &instance, "Location")?,
            ));
            let gravity = Vec3::from_array(particle_vector(
                self.instance_property(&class, &instance, "Gravity")?,
            ));
            let gravity = if gravity_modifier == 0.0 {
                gravity
            } else {
                self.zone_physics(location, actor, &instance)
                    .map_err(|message| DispatchError::UnresolvedObject { message })?
                    .map_or(gravity, |zone| {
                        particle_acceleration(gravity, zone.gravity, gravity_modifier)
                    })
            };
            let damping = particle_scalar(self.instance_property(&class, &instance, "Damping")?);
            let wind_modifier =
                particle_scalar(self.instance_property(&class, &instance, "WindModifier")?);
            let wind = if damping * wind_modifier > 0.0 {
                ParticleWind::total_at(&winds, self.collision.as_deref(), location) * wind_modifier
            } else {
                Vec3::ZERO
            };
            let mut emitter = ParticleEmitter {
                actor,
                owner,
                emit: particle_bool(self.instance_property(&class, &instance, "bEmit")?),
                prime: particle_bool(self.instance_property(&class, &instance, "bPrime")?),
                distribution: particle_byte(self.instance_property(
                    &class,
                    &instance,
                    "Distribution",
                )?),
                style: particle_byte(self.instance_property(&class, &instance, "Style")?),
                unlit: particle_bool(self.instance_property(&class, &instance, "bUnlit")?),
                particles_alive: particle_int(self.instance_property(
                    &class,
                    &instance,
                    "ParticlesAlive",
                )?),
                particles_max: particle_int(self.instance_property(
                    &class,
                    &instance,
                    "ParticlesMax",
                )?),
                particles_emitted: particle_int(self.instance_property(
                    &class,
                    &instance,
                    "ParticlesEmitted",
                )?),
                particles_per_second: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "ParticlesPerSec",
                )?),
                parent_particles_per_second: None,
                period: particle_float(self.instance_property(&class, &instance, "Period")?),
                lifetime: particle_float(self.instance_property(&class, &instance, "Lifetime")?),
                speed: particle_float(self.instance_property(&class, &instance, "Speed")?),
                angular_spread_width: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "AngularSpreadWidth",
                )?),
                angular_spread_height: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "AngularSpreadHeight",
                )?),
                source_width: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SourceWidth",
                )?),
                source_height: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SourceHeight",
                )?),
                source_depth: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SourceDepth",
                )?),
                size_width: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SizeWidth",
                )?),
                size_length: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SizeLength",
                )?),
                size_end_scale: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "SizeEndScale",
                )?),
                color_start: particle_color(self.instance_property(
                    &class,
                    &instance,
                    "ColorStart",
                )?),
                color_end: particle_color(self.instance_property(&class, &instance, "ColorEnd")?),
                alpha_start: particle_float(self.instance_property(
                    &class,
                    &instance,
                    "AlphaStart",
                )?),
                alpha_end: particle_float(self.instance_property(&class, &instance, "AlphaEnd")?),
                color_delay: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "ColorDelay",
                )?),
                size_delay: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "SizeDelay",
                )?),
                size_grow_period: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "SizeGrowPeriod",
                )?),
                draw_scale: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "DrawScale",
                )?),
                system_relative: particle_bool(self.instance_property(
                    &class,
                    &instance,
                    "bSystemRelative",
                )?),
                damping,
                gravity: gravity.to_array(),
                wind: wind.to_array(),
                winds: winds.clone(),
                render_primitive: particle_byte(self.instance_property(
                    &class,
                    &instance,
                    "RenderPrimitive",
                )?),
                velocity_relative,
                owner_velocity,
                gravity_modifier,
                chaos: particle_scalar(self.instance_property(&class, &instance, "Chaos")?),
                chaos_delay: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "ChaosDelay",
                )?),
                attraction: particle_vector(self.instance_property(
                    &class,
                    &instance,
                    "Attraction",
                )?),
                elasticity: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "Elasticity",
                )?),
                wind_modifier,
                spin_rate: particle_float(self.instance_property(&class, &instance, "SpinRate")?),
                drip_time: particle_float(self.instance_property(&class, &instance, "DripTime")?),
                parent_blend: particle_scalar(self.instance_property(
                    &class,
                    &instance,
                    "ParentBlend",
                )?),
                color_palette: matches!(
                    self.instance_property(&class, &instance, "ColorPalette")?,
                    Some(StoredValue::Object(Some(_)))
                ),
                pattern,
                textures,
            };
            if emitter.parent_blend != 0.0
                && let Some(parent) = self.particle_parent_parameters(&class)?
            {
                emitter.blend_parent_parameters(&parent);
            }
            emitters.push(emitter);
        }
        Ok(emitters)
    }

    fn particle_parent_parameters(
        &mut self,
        class: &ResolvedObject,
    ) -> DispatchResult<Option<ParticleEmitter>> {
        let (metadata, _) = class_defaults_reader(&class.package, class.export_index)?;
        let Some(parent) = self.packages.resolve(&class.package, metadata.base_field)? else {
            return Ok(None);
        };
        let defaults = self.load_class_defaults(&parent, 0)?;
        let property =
            |runtime: &mut Self, name| runtime.instance_property(&parent, &defaults, name);
        Ok(Some(ParticleEmitter {
            particles_per_second: particle_float(property(self, "ParticlesPerSec")?),
            source_width: particle_float(property(self, "SourceWidth")?),
            source_height: particle_float(property(self, "SourceHeight")?),
            source_depth: particle_float(property(self, "SourceDepth")?),
            angular_spread_width: particle_float(property(self, "AngularSpreadWidth")?),
            angular_spread_height: particle_float(property(self, "AngularSpreadHeight")?),
            speed: particle_float(property(self, "Speed")?),
            lifetime: particle_float(property(self, "Lifetime")?),
            size_width: particle_float(property(self, "SizeWidth")?),
            size_length: particle_float(property(self, "SizeLength")?),
            size_end_scale: particle_float(property(self, "SizeEndScale")?),
            color_start: particle_color(property(self, "ColorStart")?),
            color_end: particle_color(property(self, "ColorEnd")?),
            alpha_start: particle_float(property(self, "AlphaStart")?),
            alpha_end: particle_float(property(self, "AlphaEnd")?),
            spin_rate: particle_float(property(self, "SpinRate")?),
            drip_time: particle_float(property(self, "DripTime")?),
            ..Default::default()
        }))
    }

    fn particle_winds(
        &mut self,
        actors: &[(usize, ObjectId)],
    ) -> DispatchResult<Vec<ParticleWind>> {
        let mut winds = Vec::new();
        for (actor, class) in actors {
            if self.destroyed.contains(actor) {
                continue;
            }
            let class = self.resolved_object(class)?;
            if !self.class_has_name(&class, "Wind")? {
                continue;
            }
            let instance = self
                .instances
                .get(actor)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor: *actor })?;
            let rotation = match self.instance_property(&class, &instance, "Rotation")? {
                Some(StoredValue::Value(Value::Rotator(rotation))) => rotation,
                _ => [0; 3],
            };
            winds.push(ParticleWind {
                location: particle_vector(self.instance_property(&class, &instance, "Location")?),
                direction: crate::rotator_axes(rotation)[0],
                fluctuation: particle_vector(self.instance_property(&class, &instance, "Fluc")?),
                speed: particle_scalar(self.instance_property(&class, &instance, "WindSpeed")?),
                radius: particle_byte(self.instance_property(&class, &instance, "WindRadius")?),
                inner_radius: particle_byte(self.instance_property(
                    &class,
                    &instance,
                    "WindRadiusInner",
                )?),
                source: particle_byte(self.instance_property(&class, &instance, "WindSource")?),
                permeating: particle_bool(self.instance_property(
                    &class,
                    &instance,
                    "bPermeating",
                )?),
            });
        }
        Ok(winds)
    }

    fn particle_pattern(
        &mut self,
        class: &ResolvedObject,
        instance: &InstanceState,
    ) -> DispatchResult<Vec<[f32; 3]>> {
        let Some(StoredValue::Object(Some(pattern))) =
            self.instance_property(class, instance, "Pattern")?
        else {
            return Ok(Vec::new());
        };
        let pattern = self.resolved_object(&pattern)?;
        let export = &pattern.package.summary().exports[pattern.export_index];
        let Some(pattern_class) = self.packages.resolve(&pattern.package, export.class)? else {
            return Ok(Vec::new());
        };
        let mut values = self.load_class_defaults(&pattern_class, 0)?;
        let mut reader = pattern.package.export_reader(pattern.export_index)?;
        reader.read_object_stack(export.object_flags)?;
        self.apply_properties(&pattern_class, &pattern.package, &mut reader, &mut values)?;
        Ok(
            match self.instance_property(&pattern_class, &values, "Points")? {
                Some(StoredValue::Array(points)) => points
                    .into_iter()
                    .filter_map(|point| match point {
                        StoredValue::Value(Value::Vector(point)) => Some(point),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
        )
    }

    pub fn weapon_attachments(&mut self) -> DispatchResult<Vec<WeaponAttachment>> {
        let actors = self
            .actor_classes
            .iter()
            .map(|(&actor, class)| (actor, class.clone()))
            .collect::<Vec<_>>();
        let mut attachments = Vec::new();
        for (pawn, class) in actors {
            if self.destroyed.contains(&pawn) {
                continue;
            }
            let class = self.resolved_object(&class)?;
            if !self.class_has_name(&class, "Pawn")? {
                continue;
            }
            let instance = self
                .instances
                .get(&pawn)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor: pawn })?;
            let Some(StoredValue::Object(Some(weapon))) =
                self.instance_property(&class, &instance, "Weapon")?
            else {
                continue;
            };
            let Some(&weapon) = self.object_actors.get(&weapon) else {
                continue;
            };
            let weapon_class = self
                .actor_classes
                .get(&weapon)
                .cloned()
                .ok_or(DispatchError::UnregisteredActor { actor: weapon })?;
            let weapon_class = self.resolved_object(&weapon_class)?;
            let weapon_instance = self
                .instances
                .get(&weapon)
                .cloned()
                .ok_or(DispatchError::ActiveActorContext { actor: weapon })?;
            let Some(StoredValue::Object(Some(mesh))) =
                self.instance_property(&weapon_class, &weapon_instance, "ThirdPersonMesh")?
            else {
                continue;
            };
            attachments.push(WeaponAttachment {
                pawn,
                weapon,
                mesh: RuntimeObject {
                    package: Arc::clone(&mesh.package),
                    export_index: mesh.export_index,
                },
                scale: particle_scalar(self.instance_property(
                    &weapon_class,
                    &weapon_instance,
                    "ThirdPersonScale",
                )?),
            });
        }
        Ok(attachments)
    }

    pub fn set_particle_counts(&mut self, actor: usize, emitted: usize) -> DispatchResult<()> {
        let Some(class) = self.actor_classes.get(&actor).cloned() else {
            return Ok(());
        };
        let class = self.resolved_object(&class)?;
        let mut instance = self
            .instances
            .remove(&actor)
            .ok_or(DispatchError::ActiveActorContext { actor })?;
        let result = self
            .set_actor_value(
                &class,
                &mut instance,
                "ParticlesEmitted",
                Value::Int(i32::try_from(emitted).unwrap_or(i32::MAX)),
            )
            .map_err(|message| DispatchError::UnresolvedObject { message });
        self.instances.insert(actor, instance);
        result
    }

    pub fn initialize_game(&mut self) -> DispatchResult<Vec<ActorAction>> {
        let level = self.level_info.ok_or(DispatchError::MissingLevelInfo)?;
        let level_class = self
            .actor_classes
            .get(&level)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: level })?;
        let level_class = self.resolved_object(&level_class)?;
        let game_package = self.packages.load("Engine")?;
        let game_export = game_package
            .summary()
            .exports
            .iter()
            .position(|export| {
                export.class == ObjectReference::None
                    && game_package
                        .summary()
                        .name(export.object_name)
                        .eq_ignore_ascii_case("GameInfo")
            })
            .ok_or_else(|| DispatchError::UnresolvedObject {
                message: "Engine.GameInfo class is missing".to_owned(),
            })?;
        let mut level_instance = self
            .instances
            .remove(&level)
            .ok_or(DispatchError::ActiveActorContext { actor: level })?;
        let mut actions = Vec::new();
        let result = self
            .spawn_actor(
                level,
                &level_class,
                &game_package,
                &[Value::Object(
                    i32::try_from(game_export + 1).map_err(|_| DispatchError::ObjectLimit)?,
                )],
                &mut level_instance,
                &mut actions,
            )
            .map_err(|message| DispatchError::UnresolvedObject { message });
        self.instances.insert(level, level_instance);
        let Value::Object(game_handle) = result? else {
            return Err(DispatchError::UnresolvedObject {
                message: "GameInfo spawn returned no actor".to_owned(),
            });
        };
        let game = self.actor_for_handle(game_handle)?;
        let game_object = self
            .actor_objects
            .get(&game)
            .cloned()
            .ok_or(DispatchError::UnregisteredActor { actor: game })?;
        let mut level_instance = self
            .instances
            .remove(&level)
            .ok_or(DispatchError::ActiveActorContext { actor: level })?;
        let result = self.set_actor_stored(
            &level_class,
            &mut level_instance,
            "Game",
            StoredValue::Object(Some(game_object)),
        );
        self.instances.insert(level, level_instance);
        result.map_err(|message| DispatchError::UnresolvedObject { message })?;
        actions.extend(self.dispatch_event_with_arguments(
            game,
            Path::new(game_package.summary().source.as_ref()),
            game_export,
            "InitGame",
            &[Value::String(String::new()), Value::String(String::new())],
        )?);
        Ok(actions)
    }
}
