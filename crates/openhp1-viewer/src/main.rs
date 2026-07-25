use std::{env, path::PathBuf, time::Instant};

use anyhow::{Context, Result, anyhow};
use eframe::{
    egui::{self, Key, Sense, TextureId},
    wgpu,
};
use glam::Vec3;
use openhp1_map::{Model, TriangleMesh, world_model_export};
use openhp1_package::Package;
use openhp1_render::{Camera, Renderer};
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res/Maps/Quid_RavenA.unr"));
    let scene = load_scene(path)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "OpenHP1 map viewer",
        options,
        Box::new(move |context| Ok(Box::new(ViewerApp::new(context, scene)?))),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

struct LoadedScene {
    path: PathBuf,
    mesh: TriangleMesh,
    points: usize,
    nodes: usize,
    surfaces: usize,
}

fn load_scene(path: PathBuf) -> Result<LoadedScene> {
    let package =
        Package::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    let model_export = world_model_export(&package).context("failed to find the world model")?;
    let model =
        Model::decode(&package, model_export).context("failed to decode the world model")?;
    let mesh = model.triangulate().context("failed to triangulate BSP")?;
    info!(
        map = %path.display(),
        points = model.points.len(),
        nodes = model.nodes.len(),
        surfaces = model.surfaces.len(),
        triangles = mesh.indices.len() / 3,
        "loaded map"
    );
    Ok(LoadedScene {
        path,
        mesh,
        points: model.points.len(),
        nodes: model.nodes.len(),
        surfaces: model.surfaces.len(),
    })
}

struct ViewerApp {
    state: eframe::egui_wgpu::RenderState,
    renderer: Renderer,
    target: ColorTarget,
    camera: Camera,
    movement_speed: f32,
    scene: LoadedScene,
    last_frame: Instant,
}

impl ViewerApp {
    fn new(context: &eframe::CreationContext<'_>, scene: LoadedScene) -> Result<Self> {
        let state = context
            .wgpu_render_state
            .clone()
            .context("the viewer requires eframe's wgpu renderer")?;
        let size = [800, 600];
        let target = ColorTarget::new(&state, size);
        let renderer = Renderer::new(
            &state.device,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.mesh,
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
            scene,
            last_frame: Instant::now(),
        })
    }

    fn update_camera(&mut self, ui: &egui::Ui, response: &egui::Response, delta_time: f32) {
        if response.dragged() {
            let drag = ui.input(|input| input.pointer.delta());
            self.camera.yaw -= drag.x * 0.004;
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
                    ui.label("Triangles");
                    ui.label((self.scene.mesh.indices.len() / 3).to_string());
                    ui.end_row();
                });
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
            );
            self.state.queue.submit([encoder.finish()]);
        });
        ui.ctx().request_repaint();
    }
}

struct ColorTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: TextureId,
    size: [u32; 2],
}

impl ColorTarget {
    fn new(state: &eframe::egui_wgpu::RenderState, size: [u32; 2]) -> Self {
        let (texture, view) = create_color_texture(&state.device, size);
        let id = state.renderer.write().register_native_texture(
            &state.device,
            &view,
            wgpu::FilterMode::Linear,
        );
        Self {
            _texture: texture,
            view,
            id,
            size,
        }
    }

    fn resize(&mut self, state: &eframe::egui_wgpu::RenderState, size: [u32; 2]) {
        if self.size == size {
            return;
        }
        let (texture, view) = create_color_texture(&state.device, size);
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &view,
                wgpu::FilterMode::Linear,
                self.id,
            );
        self._texture = texture;
        self.view = view;
        self.size = size;
    }
}

fn create_color_texture(
    device: &wgpu::Device,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 viewport"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}
