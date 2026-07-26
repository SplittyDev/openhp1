use std::time::Instant;

use anyhow::{Context, Result};
use eframe::{
    egui::{self, Key, Sense},
    wgpu,
};
use glam::Vec3;
use openhp1_render::{Camera, Renderer};

use crate::{scene::LoadedScene, target::ColorTarget};

pub(crate) struct ViewerApp {
    state: eframe::egui_wgpu::RenderState,
    renderer: Renderer,
    target: ColorTarget,
    camera: Camera,
    movement_speed: f32,
    brightness: f32,
    scene: LoadedScene,
    last_frame: Instant,
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
        })
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

        let full_height = ui.available_height();
        ui.horizontal(|ui| {
            ui.set_min_height(full_height);
            ui.vertical(|ui| {
                ui.set_width(240.0);
                ui.heading("OpenHP1");
                ui.label(self.scene.path.display().to_string());
                ui.separator();
                egui::Grid::new("map statistics").show(ui, |ui| {
                    ui.label("Points");
                    ui.label(self.scene.points.to_string());
                    ui.end_row();
                    ui.label("BSP nodes");
                    ui.label(self.scene.nodes.to_string());
                    ui.end_row();
                    ui.label("Surfaces");
                    ui.label(self.scene.surfaces.to_string());
                    ui.end_row();
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
                    ui.label("Sky zone");
                    ui.label(if self.scene.has_sky_zone { "yes" } else { "no" });
                    ui.end_row();
                    ui.label("Triangles");
                    ui.label((self.scene.render.mesh.indices.len() / 3).to_string());
                    ui.end_row();
                });
                if self.scene.render.mesh.indices.is_empty() {
                    ui.colored_label(egui::Color32::YELLOW, "This map contains no BSP geometry.");
                }
                ui.separator();
                ui.add(egui::Slider::new(&mut self.brightness, 0.2..=1.0).text("Brightness"));
                ui.separator();
                ui.label("Drag to look");
                ui.label("WASD move · Q/E down/up");
                ui.label("Hold Shift to move faster");
            });
            ui.separator();

            let available = ui.available_size().max(egui::vec2(1.0, 1.0));
            let pixels_per_point = ui.ctx().pixels_per_point();
            let size = [
                (available.x * pixels_per_point).round().max(1.0) as u32,
                (available.y * pixels_per_point).round().max(1.0) as u32,
            ];
            self.target.resize(&self.state, size);
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
            self.renderer.render(
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
