use super::*;

impl LoadedScene {
    pub fn sync_weapon_attachments(&mut self, attachments: Vec<WeaponAttachment>) -> Result<bool> {
        let mut changed = false;
        let mut attached_weapons = HashMap::new();
        for attachment in &attachments {
            let mesh = self
                .attached_weapons
                .get(&attachment.weapon)
                .filter(|mesh| {
                    mesh.export_index == attachment.mesh.export_index
                        && mesh.package.summary().source.as_ref()
                            == attachment.mesh.package.as_ref()
                })
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| self.resolve_runtime_object(&attachment.mesh))?;
            attached_weapons.insert(attachment.weapon, mesh);
        }
        let removed = self
            .attached_weapons
            .keys()
            .filter(|weapon| !attached_weapons.contains_key(weapon))
            .copied()
            .collect::<Vec<_>>();
        for weapon in removed {
            changed |= self.rebuild_actor_render(weapon, None)?;
        }
        for (&weapon, mesh) in &attached_weapons {
            let unchanged = self
                .attached_weapons
                .get(&weapon)
                .is_some_and(|current| current.id() == mesh.id());
            if !unchanged {
                changed |= self.rebuild_actor_render(weapon, Some(mesh.clone()))?;
            }
        }
        for attachment in attachments {
            let Some(attachment_transform) = self
                .animations
                .iter()
                .find(|animation| animation.actor_index == attachment.pawn)
                .map(AnimatedActorMesh::attachment)
                .transpose()?
                .flatten()
            else {
                continue;
            };
            let Some(weapon) = self.actors.get(attachment.weapon) else {
                continue;
            };
            let (Some(render), Some(current), Some(mesh_to_object)) = (
                weapon.render.as_ref().map(|render| render.vertices.clone()),
                weapon.mesh_transform,
                weapon.mesh_to_object,
            ) else {
                continue;
            };
            let desired = attachment_transform
                * Mat4::from_scale(Vec3::splat(attachment.scale))
                * mesh_to_object;
            let delta = desired * current.inverse();
            if self.hidden_actor_positions.contains_key(&attachment.weapon) {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&attachment.weapon)
                    .context("collapsed attached weapon has no saved render positions")?;
                transform_positions(positions, delta);
                sync_hidden_attachment(
                    &mut self.render.mesh.positions[render.clone()],
                    positions,
                    true,
                );
            } else {
                transform_positions(&mut self.render.mesh.positions[render.clone()], delta);
            }
            transform_normals(&mut self.render.mesh.normals[render], delta);
            self.actors[attachment.weapon].mesh_transform = Some(desired);
            if let Some(animation) = self
                .animations
                .iter_mut()
                .find(|animation| animation.actor_index == attachment.weapon)
            {
                animation.transform = desired;
                animation.normal_transform = Mat3::from_mat4(desired).inverse().transpose();
            }
            changed = true;
        }
        self.attached_weapons = attached_weapons;
        Ok(changed)
    }

    pub fn set_actor_mesh(
        &mut self,
        actor_index: usize,
        mesh: Option<RuntimeObject>,
    ) -> Result<bool> {
        let mesh = mesh
            .as_ref()
            .map(|mesh| self.resolve_runtime_object(mesh))
            .transpose()?;
        let mesh_id = mesh.as_ref().map(SceneObject::id);
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.mesh == mesh_id {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.mesh = mesh;
        self.actors[actor_index].mesh = mesh_id;
        self.actors[actor_index].mesh_name = self.actor_states[actor_index]
            .actor
            .mesh
            .as_ref()
            .map(SceneObject::name);
        if self.attached_weapons.contains_key(&actor_index) {
            return Ok(false);
        }
        self.rebuild_actor_render(actor_index, None)
    }

    pub fn set_actor_draw_scale(&mut self, actor_index: usize, draw_scale: f32) -> Result<bool> {
        ensure!(draw_scale.is_finite(), "actor draw scale is not finite");
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let previous = actor.draw_scale;
        if previous == draw_scale {
            return Ok(false);
        }
        let render = actor.render.as_ref().map(|render| render.vertices.clone());
        let mesh_transform = actor.mesh_transform;
        let mesh_to_object = actor.mesh_to_object;
        let actor_from_world = (Mat4::from_translation(actor.location + actor.pre_pivot)
            * rotation_matrix(actor.rotation))
        .inverse();
        self.actors[actor_index].draw_scale = draw_scale;
        self.actor_states[actor_index].actor.draw_scale = draw_scale;

        if self.attached_weapons.contains_key(&actor_index) || render.is_none() {
            return Ok(false);
        }
        if previous == 0.0 {
            return self.rebuild_current_actor_render(actor_index);
        }
        let ratio = draw_scale / previous;
        if let Some(sprite) = self
            .sprites
            .iter_mut()
            .find(|sprite| sprite.actor_index == actor_index)
        {
            sprite.half_size *= ratio;
            let actor = &self.actors[actor_index];
            let positions = sprite_positions(
                actor.location + actor.pre_pivot,
                sprite.half_size,
                self.particle_view_rotation,
            );
            let vertices = render.expect("checked above");
            if let Some(hidden) = self.hidden_actor_positions.get_mut(&actor_index) {
                hidden.copy_from_slice(&positions);
            } else {
                self.render.mesh.positions[vertices].copy_from_slice(&positions);
            }
            return Ok(true);
        }

        let (Some(vertices), Some(current), Some(mesh_to_object)) =
            (render, mesh_transform, mesh_to_object)
        else {
            return Ok(false);
        };
        let world_pivot = (current * mesh_to_object.inverse()).transform_point3(Vec3::ZERO);
        ensure!(
            world_pivot.is_finite(),
            "actor draw scale has a non-invertible mesh transform"
        );
        let transform = Mat4::from_translation(world_pivot)
            * Mat4::from_scale(Vec3::splat(ratio))
            * Mat4::from_translation(-world_pivot);
        if let Some(hidden) = self.hidden_actor_positions.get_mut(&actor_index) {
            transform_positions(hidden, transform);
        } else {
            transform_positions(&mut self.render.mesh.positions[vertices.clone()], transform);
        }
        if ratio < 0.0 {
            for normal in &mut self.render.mesh.normals[vertices] {
                *normal = -*normal;
            }
        }
        self.actors[actor_index].mesh_transform = Some(transform * current);
        let local_pivot = actor_from_world.transform_point3(world_pivot);
        if let Some(bounds) = self.actors[actor_index].visual_bounds {
            self.actors[actor_index].visual_bounds =
                Some(scale_bounds_about(bounds, local_pivot, ratio));
        }
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.transform = transform * animation.transform;
            if draw_scale != 0.0 {
                animation.normal_transform =
                    Mat3::from_mat4(animation.transform).inverse().transpose();
            }
            animation.root_motion_position *= ratio;
            if let Some(positions) = &mut animation.tween_from {
                transform_positions(positions, transform);
            }
            if let Some(attachment) = &mut animation.tween_attachment_from {
                *attachment = transform * *attachment;
            }
            if let Some(positions) = &mut animation.tween_bone_positions_from {
                for position in positions {
                    *position = transform.transform_point3(*position);
                }
            }
        }
        self.relight_actor_vertices_at(actor_index)?;
        Ok(true)
    }

    pub fn set_actor_style(&mut self, actor_index: usize, style: u8) -> Result<bool> {
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if self.actor_states[actor_index].actor.style == style {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.style = style;
        self.rebuild_current_actor_render(actor_index)
    }

    pub fn set_actor_scale_glow(&mut self, actor_index: usize, scale_glow: f32) -> Result<bool> {
        ensure!(scale_glow.is_finite(), "actor scale glow is not finite");
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if self.actor_states[actor_index].actor.scale_glow == scale_glow {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.scale_glow = scale_glow;
        self.relight_actor_vertices_at(actor_index)
    }

    pub fn set_actor_skin(
        &mut self,
        actor_index: usize,
        skin: Option<RuntimeObject>,
    ) -> Result<bool> {
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let skin = skin
            .as_ref()
            .map(|skin| self.resolve_runtime_object(skin))
            .transpose()?;
        let previous = self.actor_states[actor_index]
            .actor
            .skin
            .as_ref()
            .map(SceneObject::id);
        let current = skin.as_ref().map(SceneObject::id);
        if previous == current {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.skin = skin;
        self.rebuild_current_actor_render(actor_index)
    }

    pub fn set_actor_skeletal_animation(
        &mut self,
        actor_index: usize,
        animation: Option<RuntimeObject>,
    ) -> Result<bool> {
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let animation = animation
            .as_ref()
            .map(|animation| self.resolve_runtime_object(animation))
            .transpose()?;
        let previous = self.actor_states[actor_index]
            .actor
            .skeletal_animation
            .as_ref()
            .map(SceneObject::id);
        let current = animation.as_ref().map(SceneObject::id);
        if previous == current {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.skeletal_animation = animation;
        self.rebuild_current_actor_render(actor_index)
    }

    pub fn set_actor_ambient_glow(&mut self, actor_index: usize, ambient_glow: u8) -> Result<bool> {
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if self.actor_states[actor_index].actor.ambient_glow == ambient_glow {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.ambient_glow = ambient_glow;
        self.relight_actor_vertices_at(actor_index)
    }

    pub fn set_actor_opacity(&mut self, actor_index: usize, opacity: f32) -> Result<bool> {
        ensure!(opacity.is_finite(), "actor opacity is not finite");
        self.actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if self.actor_states[actor_index].actor.opacity == opacity {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.opacity = opacity;
        let Some(vertices) = self.actors[actor_index]
            .render
            .as_ref()
            .map(|render| render.vertices.clone())
        else {
            return Ok(false);
        };
        ensure!(
            vertices.end <= self.render.mesh.vertex_surfaces.len(),
            "actor render range is outside the scene mesh"
        );
        let opacity = opacity.clamp(0.0, 1.0);
        let mut changed = false;
        for vertex in vertices {
            let surface = self.render.mesh.vertex_surfaces[vertex];
            let material = self
                .render
                .surface_materials
                .get_mut(surface)
                .context("actor vertex refers to a missing material")?;
            if material.opacity != opacity {
                material.opacity = opacity;
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) fn relight_actor_vertices_at(&mut self, actor_index: usize) -> Result<bool> {
        let state = &self.actor_states[actor_index].actor;
        let Some(vertices) = self.actors[actor_index]
            .render
            .as_ref()
            .map(|render| render.vertices.clone())
        else {
            return Ok(false);
        };
        ensure!(
            vertices.start <= vertices.end
                && vertices.end <= self.render.mesh.positions.len()
                && vertices.end <= self.render.mesh.normals.len()
                && vertices.end <= self.render.mesh.vertex_colors.len(),
            "actor render range is outside the scene mesh"
        );
        if self
            .sprites
            .iter()
            .any(|sprite| sprite.actor_index == actor_index)
        {
            let color = Vec3::splat(state.scale_glow.clamp(0.0, 1.0));
            let target = &mut self.render.mesh.vertex_colors[vertices];
            let changed = target.iter().any(|&existing| existing != color);
            target.fill(color);
            return Ok(changed);
        }
        if state.draw_type == 8 {
            return Ok(false);
        }

        let positions = self
            .hidden_actor_positions
            .get(&actor_index)
            .map(Vec::as_slice)
            .unwrap_or(&self.render.mesh.positions[vertices.clone()]);
        let Some(&first) = positions.first() else {
            return Ok(false);
        };
        ensure!(
            positions.len() == vertices.len(),
            "hidden actor position count differs from its render range"
        );
        let (minimum, maximum) = positions
            .iter()
            .copied()
            .fold((first, first), |(minimum, maximum), position| {
                (minimum.min(position), maximum.max(position))
            });
        let center = (minimum + maximum) * 0.5;
        let lighting = self.actor_render.vertex_lighting.for_actor(
            &self.actor_render.model,
            center,
            state.ambient_glow,
            state.scale_glow,
        );
        let actor_unlit = state.unlit || self.actor_render.model.zone_at(center) == 0;
        let mut changed = false;
        for (offset, &position) in positions.iter().enumerate() {
            let destination = vertices.start + offset;
            let surface = self.render.mesh.vertex_surfaces[destination];
            let unlit = actor_unlit
                || self
                    .render
                    .surface_materials
                    .get(surface)
                    .context("actor vertex refers to a missing material")?
                    .unlit;
            let color = lighting.color(position, self.render.mesh.normals[destination], unlit);
            if self.render.mesh.vertex_colors[destination] != color {
                self.render.mesh.vertex_colors[destination] = color;
                changed = true;
            }
        }
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.lighting = lighting;
            animation.unlit = actor_unlit;
        }
        Ok(changed)
    }

    fn rebuild_current_actor_render(&mut self, actor_index: usize) -> Result<bool> {
        if self.actors[actor_index].draw_type == 8 {
            return Ok(false);
        }
        let mesh_override = self.attached_weapons.get(&actor_index).cloned();
        self.rebuild_actor_render(actor_index, mesh_override)
    }

    fn resolve_runtime_object(&mut self, object: &RuntimeObject) -> Result<SceneObject> {
        let package = self
            .actor_render
            .packages
            .load_path(Path::new(object.package.as_ref()))?;
        ensure!(
            object.export_index < package.summary().exports.len(),
            "runtime object export {} is outside package {}",
            object.export_index,
            package.summary().source
        );
        Ok(SceneObject {
            package,
            export_index: object.export_index,
        })
    }

    fn rebuild_actor_render(
        &mut self,
        actor_index: usize,
        mesh_override: Option<SceneObject>,
    ) -> Result<bool> {
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let render = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &render {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }
        let playback = self
            .animations
            .iter()
            .find(|animation| animation.actor_index == actor_index)
            .map(AnimationPlayback::from);
        let mut state = self.actor_states[actor_index].actor.clone();
        if let Some(mesh) = mesh_override {
            state.draw_type = 2;
            state.mesh = Some(mesh);
        }
        if let Some(playback) = &playback {
            state.anim_sequence = Some(playback.sequence.clone());
            state.anim_frame = playback.phase;
            state.anim_rate = playback.rate;
        }

        self.hidden_actor_positions.remove(&actor_index);
        self.animations
            .retain(|animation| animation.actor_index != actor_index);
        self.sprites
            .retain(|sprite| sprite.actor_index != actor_index);
        let mut changed = render.is_some();
        if let Some(vertices) = render {
            changed |= collapse_positions(&mut self.render.mesh.positions[vertices]);
        }
        {
            let actor = &mut self.actors[actor_index];
            actor.render = None;
            actor.animation = None;
            actor.mesh_transform = None;
            actor.mesh_to_object = None;
            actor.visual_bounds = None;
            append_scene_actor_render(
                &mut self.actor_render,
                actor,
                &state,
                self.actor_states[actor_index].is_light,
                actor_index,
                &mut self.render.mesh,
                &mut self.render.textures,
                &mut self.render.surface_materials,
                &mut self.animations,
                &mut self.sprites,
                &mut self.water_animations,
            );
            changed |= actor.render.is_some();
        }
        if let Some(playback) = playback {
            playback.restore(
                self.animations
                    .iter_mut()
                    .find(|animation| animation.actor_index == actor_index),
                self.actors[actor_index].animation.as_mut(),
            );
        }
        if self.actors[actor_index].render.is_some()
            && (self.actors[actor_index].hidden || self.actors[actor_index].draw_type == 0)
        {
            self.sync_actor_render_visibility(actor_index)?;
        }
        self.actor_meshes = self
            .actors
            .iter()
            .filter(|actor| actor.render.is_some())
            .count();
        self.animated_actor_meshes = self
            .actors
            .iter()
            .filter(|actor| actor.animation.is_some())
            .count();
        // ponytail: authored display transitions append replacement geometry and
        // collapse the old range; compact only if a shipped trace shows churn.
        Ok(changed)
    }

    pub fn set_actor_draw_type(&mut self, actor_index: usize, draw_type: u8) -> Result<bool> {
        let actor = self
            .actors
            .get_mut(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.draw_type == draw_type {
            return Ok(false);
        }
        actor.draw_type = draw_type;
        self.actor_states[actor_index].actor.draw_type = draw_type;
        if draw_type == 0 {
            return self.sync_actor_render_visibility(actor_index);
        }
        self.rebuild_current_actor_render(actor_index)
    }
}

struct AnimationPlayback {
    sequence: String,
    phase: f32,
    rate: f32,
    playing: bool,
    looping: bool,
    root_motion: bool,
    root_motion_position: Vec3,
}

impl From<&AnimatedActorMesh> for AnimationPlayback {
    fn from(animation: &AnimatedActorMesh) -> Self {
        Self {
            sequence: animation.sequences()[animation.sequence].name.clone(),
            phase: animation.phase,
            rate: animation.rate,
            playing: animation.playing,
            looping: animation.looping,
            root_motion: animation.root_motion,
            root_motion_position: animation.root_motion_position,
        }
    }
}

impl AnimationPlayback {
    fn restore(
        self,
        animation: Option<&mut AnimatedActorMesh>,
        actor: Option<&mut SceneActorAnimation>,
    ) {
        let Some(animation) = animation else {
            return;
        };
        let Some(sequence) = animation
            .sequences()
            .iter()
            .position(|sequence| sequence.name.eq_ignore_ascii_case(&self.sequence))
        else {
            return;
        };
        animation.sequence = sequence;
        animation.phase = self.phase;
        animation.rate = self.rate;
        animation.playing = self.playing;
        animation.looping = self.looping;
        animation.root_motion = self.root_motion;
        animation.root_motion_position = self.root_motion_position;
        // ponytail: a display rebuild preserves the sampled sequence but ends an
        // in-flight tween; retain transformed tween vertices if a shipped trace
        // ever changes display properties during a tween.
        animation.tween_from = None;
        animation.tween_attachment_from = None;
        animation.tween_bone_positions_from = None;
        if let Some(actor) = actor {
            actor.sequence = self.sequence;
            actor.phase = self.phase;
            actor.rate = self.rate;
            actor.frame_count = animation.sequences()[sequence].frame_count;
        }
    }
}
