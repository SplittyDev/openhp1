use super::*;

impl LoadedScene {
    pub fn set_light_brightness(
        &mut self,
        actor_index: usize,
        light_brightness: u8,
    ) -> Result<bool> {
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if self.actor_states[actor_index].actor.light_brightness == light_brightness {
            return Ok(false);
        }
        self.actor_states[actor_index].actor.light_brightness = light_brightness;
        if !self.actor_states[actor_index].is_light
            || actor.id.package != self.actor_render.map.summary().source.as_ref()
        {
            return Ok(false);
        }

        let light_export = actor.id.export_index;
        let vertex_lighting_changed = self
            .actor_render
            .vertex_lighting
            .set_light_brightness(light_export, light_brightness);
        self.actor_render
            .light_brightnesses
            .insert(light_export, light_brightness);
        let lightmaps = self.actor_render.model.relight_lightmaps(
            &self.actor_render.map,
            light_export,
            &self.actor_render.light_brightnesses,
        )?;
        let mut lightmaps_changed = false;
        for (index, image) in lightmaps {
            let destination = self
                .render
                .lightmaps
                .get_mut(index)
                .context("relit image refers to a missing scene lightmap")?;
            if *destination == image {
                continue;
            }
            *destination = image;
            if !self.changed_lightmaps.contains(&index) {
                self.changed_lightmaps.push(index);
            }
            lightmaps_changed = true;
        }
        let actors_changed = vertex_lighting_changed && self.relight_actor_vertices()?;
        Ok(lightmaps_changed || actors_changed)
    }

    pub fn take_changed_lightmaps(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.changed_lightmaps)
    }

    fn relight_actor_vertices(&mut self) -> Result<bool> {
        let mut changed = false;
        for actor_index in 0..self.actors.len() {
            changed |= self.relight_actor_vertices_at(actor_index)?;
        }
        Ok(changed)
    }
}
