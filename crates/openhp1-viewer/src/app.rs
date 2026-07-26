use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use eframe::{
    egui::{self, Key, Sense},
    wgpu,
};
use glam::Vec3;
use openhp1_render::{Camera, RenderStats, Renderer};
use openhp1_scene::LoadedScene;

use crate::target::ColorTarget;

pub(crate) struct ViewerApp {
    state: eframe::egui_wgpu::RenderState,
    renderer: Renderer,
    target: ColorTarget,
    camera: Camera,
    movement_speed: f32,
    brightness: f32,
    scene: LoadedScene,
    last_frame: Instant,
    render_stats: RenderStats,
    load_error: Option<String>,
}

impl ViewerApp {
    pub(crate) fn new(context: &eframe::CreationContext<'_>, scene: LoadedScene) -> Result<Self> {
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
            scene,
            last_frame: Instant::now(),
            render_stats: RenderStats::default(),
            load_error: None,
        })
    }

    fn load_level(&mut self, path: PathBuf, viewport_size: [u32; 2]) {
        let scene = match LoadedScene::load(path) {
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
        self.render_stats = RenderStats::default();
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
                ui.label("Drag to look");
                ui.label("WASD move · Q/E down/up");
                ui.label("Hold Shift to move faster");
            });
        (selected_level != current_level).then(|| self.scene.levels[selected_level].clone())
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
                    ui.set_width(240.0);
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
