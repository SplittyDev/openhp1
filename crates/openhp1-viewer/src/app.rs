use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use eframe::{
    egui::{self, Key, Sense},
    wgpu,
};
use glam::Vec3;
use openhp1_render::{
    AmbientOcclusion, Camera, DisplaySettings, RenderStats, Renderer, RendererMode,
    RendererSettings, ToneMapper,
};
use openhp1_runtime::ScriptRuntime;
use openhp1_scene::{
    LoadedScene, SceneActor, SceneObjectId, apply_runtime_actions, initialize_runtime,
    render_to_unreal,
};

use crate::target::ColorTarget;

pub(crate) struct ViewerApp {
    state: eframe::egui_wgpu::RenderState,
    renderer: Renderer,
    target: ColorTarget,
    camera: Camera,
    movement_speed: f32,
    classic_brightness: f32,
    modern_brightness: f32,
    modern_contrast: f32,
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
    renderer_settings: RendererSettings,
}

impl ViewerApp {
    pub(crate) fn new(
        context: &eframe::CreationContext<'_>,
        mut scene: LoadedScene,
        renderer_settings: RendererSettings,
    ) -> Result<Self> {
        let runtime = initialize_runtime(&mut scene)?;
        let state = context
            .wgpu_render_state
            .clone()
            .context("the viewer requires eframe's wgpu renderer")?;
        let size = [800, 600];
        let target = ColorTarget::new(&state, size);
        let renderer = Renderer::new_with_settings(
            &state.device,
            &state.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.render,
            size,
            renderer_settings,
        );
        let bounds = renderer.bounds();
        let radius = bounds.radius().max(100.0);
        let center = bounds.center();
        let classic_display = DisplaySettings::for_mode(RendererMode::Classic);
        let modern_display = DisplaySettings::for_mode(RendererMode::Modern);
        // UE1 levels are commonly subtractive: the playable space is carved
        // inside solid BSP, so an exterior overview only sees the outer hull.
        let camera = Camera::looking_at(center, center - Vec3::Z, (radius * 10.0).max(10_000.0));
        Ok(Self {
            state,
            renderer,
            target,
            camera,
            movement_speed: (radius * 0.35).max(200.0),
            classic_brightness: classic_display.brightness,
            modern_brightness: modern_display.brightness,
            modern_contrast: modern_display.contrast,
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
            renderer_settings,
        })
    }

    fn display_settings(&self) -> DisplaySettings {
        match self.renderer_settings.mode {
            RendererMode::Classic => DisplaySettings {
                brightness: self.classic_brightness,
                ..DisplaySettings::for_mode(RendererMode::Classic)
            },
            RendererMode::Modern => DisplaySettings {
                brightness: self.modern_brightness,
                contrast: self.modern_contrast,
            },
        }
    }

    fn load_level(&mut self, path: PathBuf, viewport_size: [u32; 2]) {
        let (scene, runtime) = match LoadedScene::load(path).and_then(|mut scene| {
            let runtime = initialize_runtime(&mut scene)?;
            Ok((scene, runtime))
        }) {
            Ok(scene) => scene,
            Err(error) => {
                self.load_error = Some(format!("{error:#}"));
                return;
            }
        };
        let renderer = Renderer::new_with_settings(
            &self.state.device,
            &self.state.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.render,
            viewport_size,
            self.renderer_settings,
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

    fn rebuild_renderer(&mut self, viewport_size: [u32; 2]) {
        self.renderer = Renderer::new_with_settings(
            &self.state.device,
            &self.state.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &self.scene.render,
            viewport_size,
            self.renderer_settings,
        );
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, stable_delta_time: f32) -> (Option<PathBuf>, bool) {
        let previous_renderer_settings = self.renderer_settings;
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
        egui::CollapsingHeader::new("Renderer")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("renderer settings").show(ui, |ui| {
                    ui.label("Mode");
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.renderer_settings.mode,
                            RendererMode::Classic,
                            "Classic",
                        );
                        ui.selectable_value(
                            &mut self.renderer_settings.mode,
                            RendererMode::Modern,
                            "Modern",
                        );
                    });
                    ui.end_row();

                    if self.renderer_settings.mode == RendererMode::Modern {
                        ui.label("Tone mapper");
                        egui::ComboBox::from_id_salt("tone mapper selector")
                            .selected_text(tone_mapper_name(self.renderer_settings.tone_mapper))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.renderer_settings.tone_mapper,
                                    ToneMapper::AgX,
                                    "AgX",
                                );
                                ui.selectable_value(
                                    &mut self.renderer_settings.tone_mapper,
                                    ToneMapper::Reinhard,
                                    "Reinhard",
                                );
                                ui.selectable_value(
                                    &mut self.renderer_settings.tone_mapper,
                                    ToneMapper::Aces,
                                    "ACES",
                                );
                            });
                        ui.end_row();
                    }

                    ui.label("Brightness");
                    let brightness = match self.renderer_settings.mode {
                        RendererMode::Classic => &mut self.classic_brightness,
                        RendererMode::Modern => &mut self.modern_brightness,
                    };
                    ui.add(egui::Slider::new(brightness, 0.2..=1.0));
                    ui.end_row();

                    if self.renderer_settings.mode == RendererMode::Modern {
                        ui.label("Contrast");
                        ui.add(egui::Slider::new(&mut self.modern_contrast, 0.5..=2.0));
                        ui.end_row();

                        ui.label("Ambient occlusion");
                        egui::ComboBox::from_id_salt("ambient occlusion selector")
                            .selected_text(match self.renderer_settings.ambient_occlusion {
                                AmbientOcclusion::Off => "Off",
                                AmbientOcclusion::Ssao => "SSAO",
                                AmbientOcclusion::XeGtao => "XeGTAO",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.renderer_settings.ambient_occlusion,
                                    AmbientOcclusion::Off,
                                    "Off",
                                );
                                ui.selectable_value(
                                    &mut self.renderer_settings.ambient_occlusion,
                                    AmbientOcclusion::Ssao,
                                    "SSAO",
                                );
                                ui.selectable_value(
                                    &mut self.renderer_settings.ambient_occlusion,
                                    AmbientOcclusion::XeGtao,
                                    "XeGTAO",
                                );
                            });
                        ui.end_row();

                        ui.label("Effects");
                        ui.checkbox(&mut self.renderer_settings.bloom, "Bloom");
                        ui.end_row();
                    }
                });
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
                ui.checkbox(&mut self.animations_playing, "Play animations");
                ui.add(
                    egui::Slider::new(&mut self.animation_speed, 0.1..=2.0).text("Animation speed"),
                );
                ui.label("Drag to look");
                ui.label("WASD move · Q/E down/up");
                ui.label("Hold Shift to move faster");
            });
        (
            (selected_level != current_level).then(|| self.scene.levels[selected_level].clone()),
            self.renderer_settings != previous_renderer_settings,
        )
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
                self.update_vertices();
                completed
            }
            Ok((false, completed)) => completed,
            Err(error) => {
                self.animations_playing = false;
                self.load_error = Some(format!("animation failed: {error:#}"));
                Vec::new()
            }
        };
        for (actor, delta) in self.scene.take_root_motions() {
            let actions = match self.runtime.apply_root_motion(actor, delta.to_array()) {
                Ok(actions) => actions,
                Err(error) => {
                    self.load_error = Some(format!("root motion failed: {error}"));
                    break;
                }
            };
            match apply_runtime_actions(&mut self.scene, &mut self.runtime, actions) {
                Ok((_, _, true)) => self.update_vertices(),
                Ok(_) => {}
                Err(error) => {
                    self.load_error = Some(format!("root motion failed: {error:#}"));
                    break;
                }
            }
        }
        for actor in completed {
            let actions = match self.runtime.animation_finished(actor) {
                Ok(actions) => actions,
                Err(error) => {
                    self.load_error = Some(format!("animation callback failed: {error}"));
                    break;
                }
            };
            match apply_runtime_actions(&mut self.scene, &mut self.runtime, actions) {
                Ok((_, _, true)) => self.update_vertices(),
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
        if let Err(error) = openhp1_scene::sync_runtime_pose(&self.scene, &mut self.runtime) {
            self.animations_playing = false;
            self.load_error = Some(format!("animation pose sync failed: {error:#}"));
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
            Ok((_, _, true)) => self.update_vertices(),
            Ok(_) => {}
            Err(error) => {
                self.load_error = Some(format!("runtime action failed: {error:#}"));
            }
        }
    }

    fn update_vertices(&mut self) {
        let changed_lightmaps = self.scene.take_changed_lightmaps();
        let scene_updated = self
            .renderer
            .update_scene(&self.state.queue, &self.scene.render);
        let lightmaps_updated = scene_updated
            && self.renderer.update_lightmaps(
                &self.state.queue,
                &self.scene.render.lightmaps,
                &changed_lightmaps,
            );
        if !scene_updated || !lightmaps_updated {
            self.renderer
                .reload_scene(&self.state.device, &self.state.queue, &self.scene.render);
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let delta_time = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.renderer.advance_time(delta_time);
        let stable_delta_time = ui.input(|input| input.stable_dt);

        let full_height = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(full_height);
            let (requested_level, renderer_settings_changed) = ui
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
            } else if renderer_settings_changed {
                self.rebuild_renderer(size);
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
                self.display_settings(),
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

fn tone_mapper_name(tone_mapper: ToneMapper) -> &'static str {
    match tone_mapper {
        ToneMapper::AgX => "AgX",
        ToneMapper::Reinhard => "Reinhard",
        ToneMapper::Aces => "ACES",
    }
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
