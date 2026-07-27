use std::{collections::VecDeque, path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use eframe::{
    egui::{self, Key, Sense},
    wgpu,
};
use glam::Vec3;
use openhp1_render::{Camera, RenderStats, Renderer};
use openhp1_runtime::{ActorAction, ScriptRuntime};
use openhp1_scene::{LoadedScene, Rotator, SceneActor, SceneObjectId, render_to_unreal};
use tracing::info;

use crate::target::ColorTarget;

pub(crate) struct ViewerApp {
    state: eframe::egui_wgpu::RenderState,
    renderer: Renderer,
    target: ColorTarget,
    camera: Camera,
    movement_speed: f32,
    brightness: f32,
    animations_playing: bool,
    animation_speed: f32,
    actor_filter: String,
    selected_actor: Option<usize>,
    scene: LoadedScene,
    runtime: ScriptRuntime,
    player_touch_position: Option<Vec3>,
    last_frame: Instant,
    render_stats: RenderStats,
    load_error: Option<String>,
}

impl ViewerApp {
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        mut scene: LoadedScene,
    ) -> Result<Self> {
        let runtime = apply_begin_play(&mut scene)?;
        let state = context
            .wgpu_render_state
            .clone()
            .context("the viewer requires eframe's wgpu renderer")?;
        let size = [800, 600];
        let target = ColorTarget::new(&state, size);
        let renderer = Renderer::new(
            &state.device,
            &state.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.render,
            size,
        );
        let bounds = renderer.bounds();
        let radius = bounds.radius().max(100.0);
        let center = bounds.center();
        // UE1 levels are commonly subtractive: the playable space is carved
        // inside solid BSP, so an exterior overview only sees the outer hull.
        let camera = Camera::looking_at(center, center - Vec3::Z, (radius * 10.0).max(10_000.0));
        Ok(Self {
            state,
            renderer,
            target,
            camera,
            movement_speed: (radius * 0.35).max(200.0),
            brightness: 0.625,
            animations_playing: true,
            animation_speed: 1.0,
            actor_filter: String::new(),
            selected_actor: None,
            scene,
            runtime,
            player_touch_position: None,
            last_frame: Instant::now(),
            render_stats: RenderStats::default(),
            load_error: None,
        })
    }

    fn load_level(&mut self, path: PathBuf, viewport_size: [u32; 2]) {
        let (scene, runtime) = match LoadedScene::load(path).and_then(|mut scene| {
            let runtime = apply_begin_play(&mut scene)?;
            Ok((scene, runtime))
        }) {
            Ok(scene) => scene,
            Err(error) => {
                self.load_error = Some(format!("{error:#}"));
                return;
            }
        };
        let renderer = Renderer::new(
            &self.state.device,
            &self.state.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.render,
            viewport_size,
        );
        let bounds = renderer.bounds();
        let radius = bounds.radius().max(100.0);
        let center = bounds.center();
        self.camera = Camera::looking_at(center, center - Vec3::Z, (radius * 10.0).max(10_000.0));
        self.movement_speed = (radius * 0.35).max(200.0);
        self.renderer = renderer;
        self.scene = scene;
        self.runtime = runtime;
        self.player_touch_position = None;
        self.actor_filter.clear();
        self.selected_actor = None;
        self.render_stats = RenderStats::default();
        self.last_frame = Instant::now();
        self.load_error = None;
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, stable_delta_time: f32) -> Option<PathBuf> {
        let current_level = self
            .scene
            .levels
            .iter()
            .position(|level| level == &self.scene.path)
            .unwrap_or(0);
        let mut selected_level = current_level;

        ui.heading("OpenHP1");
        egui::CollapsingHeader::new("Level")
            .default_open(true)
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("level selector")
                    .width(ui.available_width())
                    .selected_text(level_name(&self.scene.levels[current_level]))
                    .show_ui(ui, |ui| {
                        for (index, path) in self.scene.levels.iter().enumerate() {
                            ui.selectable_value(&mut selected_level, index, level_name(path));
                        }
                    });
                ui.small(self.scene.path.display().to_string());
                if let Some(error) = &self.load_error {
                    ui.colored_label(egui::Color32::RED, error);
                }
            });
        egui::CollapsingHeader::new(format!("Actors ({})", self.scene.actors.len()))
            .default_open(false)
            .show(ui, |ui| self.actor_inspector(ui));
        egui::CollapsingHeader::new("Performance")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("performance statistics").show(ui, |ui| {
                    ui.label("FPS");
                    ui.label(format!("{:.1}", stable_delta_time.recip()));
                    ui.end_row();
                    ui.label("Frame time");
                    ui.label(format!("{:.2} ms", stable_delta_time * 1_000.0));
                    ui.end_row();
                });
            });
        egui::CollapsingHeader::new("Rendering")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("rendering statistics").show(ui, |ui| {
                    ui.label("Visible BSP surfaces");
                    ui.label(self.scene.visible_bsp_surfaces.to_string());
                    ui.end_row();
                    ui.label("Visible meshes");
                    ui.label(self.scene.actor_meshes.to_string());
                    ui.end_row();
                    ui.label("Animated meshes");
                    ui.label(self.scene.animated_actor_meshes.to_string());
                    ui.end_row();
                    ui.label("Draw calls");
                    ui.label(self.render_stats.draw_calls.to_string());
                    ui.end_row();
                    ui.label("Triangles");
                    ui.label((self.scene.render.mesh.indices.len() / 3).to_string());
                    ui.end_row();
                });
                if self.scene.render.mesh.indices.is_empty() {
                    ui.colored_label(egui::Color32::YELLOW, "This map contains no BSP geometry.");
                }
            });
        egui::CollapsingHeader::new("Memory")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("memory statistics").show(ui, |ui| {
                    ui.label("Textures");
                    ui.label(self.scene.render.textures.len().to_string());
                    ui.end_row();
                    ui.label("Texture memory");
                    ui.label(mebibytes(self.render_stats.texture_memory_bytes));
                    ui.end_row();
                    ui.label("Lightmaps");
                    ui.label(self.scene.render.lightmaps.len().to_string());
                    ui.end_row();
                    ui.label("Lightmap memory");
                    ui.label(mebibytes(self.render_stats.lightmap_memory_bytes));
                    ui.end_row();
                });
            });
        egui::CollapsingHeader::new("World / Level")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("world statistics").show(ui, |ui| {
                    ui.label("Points");
                    ui.label(self.scene.points.to_string());
                    ui.end_row();
                    ui.label("BSP nodes");
                    ui.label(self.scene.nodes.to_string());
                    ui.end_row();
                    ui.label("Surfaces");
                    ui.label(self.scene.surfaces.to_string());
                    ui.end_row();
                    ui.label("Zone ID");
                    ui.label(self.scene.zone_at(self.camera.position).to_string());
                    ui.end_row();
                    ui.label("Sky zone");
                    ui.label(if self.scene.has_sky_zone { "yes" } else { "no" });
                    ui.end_row();
                });
            });
        egui::CollapsingHeader::new("Materials")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("material statistics").show(ui, |ui| {
                    ui.label("Textured");
                    ui.label(self.scene.textured_surfaces.to_string());
                    ui.end_row();
                    ui.label("Masked");
                    ui.label(self.scene.masked_surfaces.to_string());
                    ui.end_row();
                    ui.label("Translucent");
                    ui.label(self.scene.translucent_surfaces.to_string());
                    ui.end_row();
                    ui.label("Modulated");
                    ui.label(self.scene.modulated_surfaces.to_string());
                    ui.end_row();
                    ui.label("Fake backdrops");
                    ui.label(self.scene.fake_backdrop_surfaces.to_string());
                    ui.end_row();
                });
            });
        egui::CollapsingHeader::new("View")
            .default_open(true)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut self.brightness, 0.2..=1.0).text("Brightness"));
                ui.checkbox(&mut self.animations_playing, "Play animations");
                ui.add(
                    egui::Slider::new(&mut self.animation_speed, 0.1..=2.0).text("Animation speed"),
                );
                ui.label("Drag to look");
                ui.label("WASD move · Q/E down/up");
                ui.label("Hold Shift to move faster");
            });
        (selected_level != current_level).then(|| self.scene.levels[selected_level].clone())
    }

    fn actor_inspector(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::singleline(&mut self.actor_filter)
                .hint_text("Filter name, class, or mesh"),
        );
        let query = self.actor_filter.trim().to_ascii_lowercase();
        let matching = self
            .scene
            .actors
            .iter()
            .enumerate()
            .filter_map(|(index, actor)| actor_matches(actor, &query).then_some(index))
            .collect::<Vec<_>>();
        let mut selected = self.selected_actor;
        egui::ScrollArea::vertical()
            .id_salt("actor list")
            .max_height(220.0)
            .show_rows(
                ui,
                ui.spacing().interact_size.y,
                matching.len(),
                |ui, rows| {
                    for &index in &matching[rows] {
                        let actor = &self.scene.actors[index];
                        ui.selectable_value(
                            &mut selected,
                            Some(index),
                            format!("{} — {}", actor.name, actor.class_name),
                        );
                    }
                },
            );
        self.selected_actor = selected.filter(|index| matching.binary_search(index).is_ok());
        ui.small(format!("{} matching", matching.len()));

        let Some(actor) = self
            .selected_actor
            .and_then(|index| self.scene.actors.get(index))
        else {
            return;
        };
        ui.separator();
        ui.strong(&actor.name);
        egui::Grid::new("actor details")
            .num_columns(2)
            .show(ui, |ui| {
                detail_row(ui, "Object", object_id_label(&actor.id));
                detail_row(
                    ui,
                    "Class",
                    actor.class.as_ref().map_or_else(
                        || actor.class_name.clone(),
                        |id| format!("{} ({})", actor.class_name, object_id_label(id)),
                    ),
                );
                detail_row(ui, "Location (UE)", format_vec3(actor.location));
                detail_row(
                    ui,
                    "Rotation",
                    format!(
                        "{}, {}, {}",
                        actor.rotation.pitch, actor.rotation.yaw, actor.rotation.roll
                    ),
                );
                detail_row(ui, "PrePivot", format_vec3(actor.pre_pivot));
                detail_row(ui, "Draw scale", format!("{:.3}", actor.draw_scale));
                detail_row(ui, "Draw type", actor.draw_type.to_string());
                detail_row(ui, "Hidden", yes_no(actor.hidden));
                detail_row(ui, "Unlit", yes_no(actor.unlit));
                detail_row(
                    ui,
                    "Mesh",
                    actor.mesh.as_ref().map_or_else(
                        || "none".to_owned(),
                        |id| {
                            format!(
                                "{} ({})",
                                actor.mesh_name.as_deref().unwrap_or("<unnamed>"),
                                object_id_label(id)
                            )
                        },
                    ),
                );
                detail_row(
                    ui,
                    "Animation",
                    actor.animation.as_ref().map_or_else(
                        || "none".to_owned(),
                        |animation| {
                            format!(
                                "{} · {:.3} · {:.3}/s · {} frames",
                                animation.sequence,
                                animation.phase,
                                animation.rate,
                                animation.frame_count
                            )
                        },
                    ),
                );
                detail_row(
                    ui,
                    "Geometry",
                    actor.render.as_ref().map_or_else(
                        || "none".to_owned(),
                        |render| {
                            format!(
                                "{} vertices · {} triangles",
                                render.vertices.len(),
                                render.indices.len() / 3
                            )
                        },
                    ),
                );
            });
        for diagnostic in &actor.diagnostics {
            ui.label(format!("Diagnostic: {diagnostic}"));
        }
    }

    fn update_camera(&mut self, ui: &egui::Ui, response: &egui::Response, delta_time: f32) {
        if response.dragged() {
            let drag = ui.input(|input| input.pointer.delta());
            self.camera.yaw += drag.x * 0.004;
            self.camera.pitch = (self.camera.pitch - drag.y * 0.004).clamp(-1.55, 1.55);
        }
        if !response.hovered() {
            return;
        }

        let (mut movement, fast) = ui.input(|input| {
            let mut movement = Vec3::ZERO;
            movement.z +=
                (input.key_down(Key::W) as u8 as f32) - (input.key_down(Key::S) as u8 as f32);
            movement.x +=
                (input.key_down(Key::D) as u8 as f32) - (input.key_down(Key::A) as u8 as f32);
            movement.y +=
                (input.key_down(Key::E) as u8 as f32) - (input.key_down(Key::Q) as u8 as f32);
            (movement, input.modifiers.shift)
        });
        movement = movement.normalize_or_zero();
        let speed = self.movement_speed * if fast { 4.0 } else { 1.0 };
        self.camera.position += (self.camera.forward() * movement.z
            + self.camera.right() * movement.x
            + Vec3::Y * movement.y)
            * speed
            * delta_time;
    }

    fn update_animations(&mut self, delta_time: f32) {
        if !self.animations_playing {
            return;
        }
        let delta_time = delta_time * self.animation_speed;
        let completed = match self.scene.tick_animations_with_completions(delta_time) {
            Ok((true, completed)) => {
                if !self
                    .renderer
                    .update_vertices(&self.state.queue, &self.scene.render.mesh)
                {
                    self.animations_playing = false;
                    self.load_error = Some("animation changed the scene vertex count".to_owned());
                }
                completed
            }
            Ok((false, completed)) => completed,
            Err(error) => {
                self.animations_playing = false;
                self.load_error = Some(format!("animation failed: {error:#}"));
                Vec::new()
            }
        };
        for actor in completed {
            let actions = match self.runtime.animation_finished(actor) {
                Ok(actions) => actions,
                Err(error) => {
                    self.load_error = Some(format!("animation callback failed: {error}"));
                    break;
                }
            };
            match apply_runtime_actions(&mut self.scene, &mut self.runtime, actions) {
                Ok((_, _, true))
                    if !self
                        .renderer
                        .update_vertices(&self.state.queue, &self.scene.render.mesh) =>
                {
                    self.load_error =
                        Some("animation callback changed the scene vertex count".to_owned());
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    self.load_error = Some(format!("animation callback failed: {error:#}"));
                    break;
                }
            }
        }
        if !self.animations_playing {
            return;
        }
        match self.scene.tick_water(delta_time) {
            Ok(changed)
                if !self.renderer.update_textures(
                    &self.state.queue,
                    &self.scene.render.textures,
                    &changed,
                ) =>
            {
                self.animations_playing = false;
                self.load_error = Some("animation changed the scene textures".to_owned());
            }
            Ok(_) => {}
            Err(error) => {
                self.animations_playing = false;
                self.load_error = Some(format!("water animation failed: {error:#}"));
            }
        }
    }

    fn update_runtime(&mut self, delta_time: f32) {
        let mut actions = match self.runtime.tick(delta_time) {
            Ok(actions) => actions,
            Err(error) => {
                self.load_error = Some(format!("runtime tick failed: {error}"));
                return;
            }
        };
        if self.player_touch_position != Some(self.camera.position) {
            self.player_touch_position = Some(self.camera.position);
            match self
                .runtime
                .update_player_touches(render_to_unreal(self.camera.position).to_array())
            {
                Ok(touch_actions) => actions.extend(touch_actions),
                Err(error) => {
                    self.load_error = Some(format!("player touch update failed: {error}"));
                }
            }
        }
        match apply_runtime_actions(&mut self.scene, &mut self.runtime, actions) {
            Ok((_, _, true))
                if !self
                    .renderer
                    .update_vertices(&self.state.queue, &self.scene.render.mesh) =>
            {
                self.load_error = Some("runtime changed the scene vertex count".to_owned());
            }
            Ok(_) => {}
            Err(error) => {
                self.load_error = Some(format!("runtime action failed: {error:#}"));
            }
        }
    }
}

fn apply_begin_play(scene: &mut LoadedScene) -> Result<ScriptRuntime> {
    let game_root = scene
        .path
        .parent()
        .and_then(|directory| directory.parent())
        .context("map path must be inside the game's Maps directory")?;
    let mut runtime = ScriptRuntime::new(game_root)?;
    runtime.set_collision(scene.collision(), &scene.path)?;
    let classes = scene
        .actors
        .iter()
        .enumerate()
        .filter_map(|(actor, value)| {
            value.class.as_ref().map(|class| {
                (
                    actor,
                    value.id.package.clone(),
                    value.id.export_index,
                    class.package.clone(),
                    class.export_index,
                )
            })
        })
        .collect::<Vec<_>>();
    for &(actor, ref actor_package, actor_export, ref class_package, class_export) in &classes {
        if let Err(error) = runtime.register_actor(
            actor,
            actor_package,
            actor_export,
            class_package,
            class_export,
        ) {
            scene.actors[actor]
                .diagnostics
                .push(format!("runtime registration failed: {error}"));
        }
    }
    let mut events = 0;
    let mut animations = 0;
    let mut deferred = 0;
    for event in [
        "PreBeginPlay",
        "BeginPlay",
        "PostBeginPlay",
        "SetInitialState",
    ] {
        for &(actor, _, _, ref package, export) in &classes {
            match runtime.dispatch_event(actor, package, export, event) {
                Ok(actions) => {
                    events += 1;
                    let applied = apply_runtime_actions(scene, &mut runtime, actions)?;
                    animations += applied.0;
                    deferred += applied.1;
                }
                Err(error) => {
                    deferred += 1;
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime deferred {event}: {error}"));
                }
            }
        }
    }
    info!(events, animations, deferred, "initialized script runtime");
    Ok(runtime)
}

fn apply_runtime_actions(
    scene: &mut LoadedScene,
    runtime: &mut ScriptRuntime,
    actions: Vec<ActorAction>,
) -> Result<(usize, usize, bool)> {
    let mut animations = 0;
    let mut deferred = 0;
    let mut transformed = false;
    let mut actions = VecDeque::from(actions);
    while let Some(action) = actions.pop_front() {
        scene.ensure_runtime_actor(action.actor());
        match action {
            ActorAction::PlayAnimation {
                actor,
                sequence,
                rate,
                tween_time,
            } => {
                if scene.play_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
                    animations += 1;
                } else {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime could not play animation {sequence}"));
                }
            }
            ActorAction::LoopAnimation {
                actor,
                sequence,
                rate,
                tween_time,
            } => {
                if scene.loop_actor_animation_with_tween(actor, &sequence, rate, tween_time)? {
                    animations += 1;
                } else {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime could not play animation {sequence}"));
                }
            }
            ActorAction::AwaitAnimation { actor } => {
                scene.finish_actor_animation(actor);
                if !scene.actor_animation_playing(actor) {
                    actions.extend(runtime.animation_finished(actor)?);
                }
            }
            ActorAction::SpawnActor {
                actor,
                name,
                class_package,
                class_export,
                class_name,
                location,
                rotation,
            } => {
                scene.spawn_actor(
                    actor,
                    name,
                    class_package.to_string(),
                    class_export,
                    class_name,
                    Vec3::from_array(location),
                    Rotator {
                        pitch: rotation[0],
                        yaw: rotation[1],
                        roll: rotation[2],
                    },
                )?;
            }
            ActorAction::SetLocation { actor, location } => {
                transformed |= scene.set_actor_location(actor, Vec3::from_array(location))?;
            }
            ActorAction::SetRotation { actor, rotation } => {
                transformed |= scene.set_actor_rotation(
                    actor,
                    Rotator {
                        pitch: rotation[0],
                        yaw: rotation[1],
                        roll: rotation[2],
                    },
                )?;
            }
            ActorAction::SetHidden { actor, hidden } => {
                transformed |= scene.set_actor_hidden(actor, hidden)?;
            }
            ActorAction::DestroyActor { actor } => {
                transformed |= scene.destroy_actor(actor)?;
            }
            ActorAction::Log {
                actor,
                message,
                tag,
            } => {
                info!(
                    actor,
                    actor_name = scene.actors[actor].name,
                    tag = tag.as_deref().unwrap_or(""),
                    message = %message,
                    "UnrealScript log"
                );
            }
            ActorAction::DeferredCall { actor, message } => {
                deferred += 1;
                if scene.actors[actor]
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.starts_with("runtime deferred"))
                    .count()
                    < 3
                {
                    scene.actors[actor]
                        .diagnostics
                        .push(format!("runtime deferred call: {message}"));
                }
            }
            ActorAction::DispatchEvent {
                actor,
                event,
                arguments,
            } => {
                let Some(class) = scene.actors[actor].class.clone() else {
                    continue;
                };
                match runtime.dispatch_event_with_arguments(
                    actor,
                    &class.package,
                    class.export_index,
                    event,
                    &arguments,
                ) {
                    Ok(event_actions) => actions.extend(event_actions),
                    Err(error) => actions.push_back(ActorAction::DeferredCall {
                        actor,
                        message: format!("{event}: {error}"),
                    }),
                }
            }
        }
    }
    Ok((animations, deferred, transformed))
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let delta_time = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let stable_delta_time = ui.input(|input| input.stable_dt);

        let full_height = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(full_height);
            let requested_level = ui
                .vertical(|ui| {
                    ui.set_width(300.0);
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.sidebar(ui, stable_delta_time))
                        .inner
                })
                .inner;
            ui.separator();

            let available = ui.available_size().max(egui::vec2(1.0, 1.0));
            let pixels_per_point = ui.ctx().pixels_per_point();
            let size = [
                (available.x * pixels_per_point).round().max(1.0) as u32,
                (available.y * pixels_per_point).round().max(1.0) as u32,
            ];
            self.target.resize(&self.state, size);
            if let Some(path) = requested_level {
                self.load_level(path, size);
            }
            self.renderer.resize(&self.state.device, size);
            let response = ui.add(
                egui::Image::new((self.target.id, available))
                    .sense(Sense::drag())
                    .maintain_aspect_ratio(false),
            );
            self.update_camera(ui, &response, delta_time);
            self.update_animations(delta_time);
            self.update_runtime(delta_time);

            let mut encoder =
                self.state
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("OpenHP1 frame"),
                    });
            self.render_stats = self.renderer.render(
                &self.state.queue,
                &mut encoder,
                &self.target.view,
                &self.camera,
                size,
                self.brightness,
            );
            self.state.queue.submit([encoder.finish()]);
        });
        ui.ctx().request_repaint();
    }
}

fn level_name(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn mebibytes(bytes: usize) -> String {
    format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn actor_matches(actor: &SceneActor, query: &str) -> bool {
    query.is_empty()
        || actor.name.to_ascii_lowercase().contains(query)
        || actor.class_name.to_ascii_lowercase().contains(query)
        || actor
            .mesh_name
            .as_deref()
            .is_some_and(|mesh| mesh.to_ascii_lowercase().contains(query))
        || actor.id.export_index.to_string().contains(query)
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: impl Into<egui::WidgetText>) {
    ui.label(label);
    ui.label(value);
    ui.end_row();
}

fn object_id_label(id: &SceneObjectId) -> String {
    let package = std::path::Path::new(&id.package)
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| id.package.as_str().into());
    format!("{package}#{}", id.export_index)
}

fn format_vec3(value: Vec3) -> String {
    format!("{:.2}, {:.2}, {:.2}", value.x, value.y, value.z)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
