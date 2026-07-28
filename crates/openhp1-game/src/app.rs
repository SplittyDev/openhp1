use std::{collections::HashSet, f32::consts::TAU, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use glam::{Mat3, Quat, Vec3};
use openhp1_audio::AudioPlayer;
use openhp1_render::{Camera, RenderStats, Renderer};
use openhp1_runtime::{ActorAction, PlayerInput, PlayerView, ScriptRuntime};
use openhp1_scene::{
    LoadedScene, Rotator, apply_runtime_actions_with, initialize_runtime_with, unreal_to_render,
};
use tracing::error;
use wgpu::{CurrentSurfaceTexture, SurfaceConfiguration};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize, Size},
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId},
};

const ROTATOR_RADIANS: f32 = TAU / 65_536.0;
const DEBUG_FAST_FORWARD_TICKS: usize = 16;

pub(crate) struct GameApp {
    scene: Option<LoadedScene>,
    graphics: Option<Graphics>,
}

impl GameApp {
    pub(crate) fn new(scene: LoadedScene) -> Self {
        Self {
            scene: Some(scene),
            graphics: None,
        }
    }
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let Some(scene) = self.scene.take() else {
            return;
        };
        let attributes = WindowAttributes::default()
            .with_title("OpenHP1")
            .with_inner_size(Size::Logical(LogicalSize::new(1280.0, 800.0)));
        let result = event_loop
            .create_window(attributes)
            .context("failed to create the game window")
            .and_then(|window| Graphics::new(Arc::new(window), scene));
        match result {
            Ok(graphics) => {
                graphics.window.request_redraw();
                self.graphics = Some(graphics);
            }
            Err(error) => {
                error!(%error, "could not start OpenHP1");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };
        if graphics.window.id() != window_id {
            return;
        }
        let egui_response = graphics.egui.on_window_event(&graphics.window, &event);
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.physical_key == PhysicalKey::Code(KeyCode::F1)
            && event.state == ElementState::Pressed
            && !event.repeat
        {
            graphics.overlay_visible = !graphics.overlay_visible;
            graphics.release_input();
            return;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(code) = event.physical_key
            && is_fast_forward_key(code)
        {
            graphics.input.set_key(code, event.state);
            return;
        }
        if egui_response.consumed {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => graphics.resize(size),
            WindowEvent::Focused(false) => graphics.release_input(),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                if code == KeyCode::Escape && event.state == ElementState::Pressed {
                    graphics.release_input();
                } else {
                    graphics.input.set_key(code, event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                graphics.mouse_button(button, state);
            }
            WindowEvent::RedrawRequested => {
                graphics.render();
                graphics.window.request_redraw();
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };
        if let DeviceEvent::MouseMotion { delta } = event
            && graphics.input.captured
        {
            graphics.input.mouse_delta.0 += delta.0;
            graphics.input.mouse_delta.1 += delta.1;
        }
    }
}

#[derive(Default)]
struct InputState {
    keys: HashSet<KeyCode>,
    mouse_delta: (f64, f64),
    cast_mouse: bool,
    jump_requested: bool,
    captured: bool,
}

impl InputState {
    fn set_key(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                let first_press = self.keys.insert(key);
                if first_press
                    && matches!(
                        key,
                        KeyCode::Space | KeyCode::ControlLeft | KeyCode::ControlRight
                    )
                {
                    self.jump_requested = true;
                }
            }
            ElementState::Released => {
                self.keys.remove(&key);
            }
        }
    }

    fn set_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        match button {
            MouseButton::Left if state == ElementState::Pressed => self.jump_requested = true,
            MouseButton::Right => self.cast_mouse = state == ElementState::Pressed,
            _ => {}
        }
    }

    fn player_input(&mut self, delta_time: f32) -> PlayerInput {
        let pressed = |keys: &HashSet<KeyCode>, choices: &[KeyCode]| {
            choices.iter().any(|key| keys.contains(key)) as u8 as f32
        };
        let forward = pressed(&self.keys, &[KeyCode::KeyW, KeyCode::ArrowUp]);
        let backward = pressed(&self.keys, &[KeyCode::KeyS, KeyCode::ArrowDown]);
        let left = pressed(&self.keys, &[KeyCode::KeyA, KeyCode::ArrowLeft]);
        let right = pressed(&self.keys, &[KeyCode::KeyD, KeyCode::ArrowRight]);
        let casting =
            self.cast_mouse || pressed(&self.keys, &[KeyCode::AltLeft, KeyCode::AltRight]) != 0.0;
        let mouse_scale = 16.0 * 6.0 / (delta_time.max(0.000_001) * 150.0);
        let input = PlayerInput {
            base_x: if casting {
                0.0
            } else {
                (right - left) * 3_000.0
            },
            base_y: if casting {
                0.0
            } else {
                forward * 6_000.0 - backward * 3_000.0
            },
            strafe: 0.0,
            mouse_x: casting as u8 as f32 * self.mouse_delta.0 as f32 * mouse_scale,
            mouse_y: -(casting as u8 as f32) * self.mouse_delta.1 as f32 * mouse_scale,
            alt_fire: casting,
            jump: self.jump_requested,
        };
        self.mouse_delta = (0.0, 0.0);
        self.jump_requested = false;
        input
    }

    fn clear(&mut self) {
        self.keys.clear();
        self.mouse_delta = (0.0, 0.0);
        self.cast_mouse = false;
        self.jump_requested = false;
    }
}

struct Graphics {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: SurfaceConfiguration,
    renderer: Renderer,
    camera: Camera,
    scene: LoadedScene,
    runtime: ScriptRuntime,
    audio: Option<AudioPlayer>,
    player: usize,
    input: InputState,
    last_frame: Instant,
    last_error: Option<String>,
    deferred_calls: usize,
    view_actor: usize,
    render_stats: RenderStats,
    frame_time_ms: f32,
    overlay_visible: bool,
    egui: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

impl Graphics {
    fn new(window: Arc<Window>, mut scene: LoadedScene) -> Result<Self> {
        let mut last_error = None;
        let (music_volume, sound_volume) = audio_volumes(&scene);
        let mut audio = match AudioPlayer::new(music_volume, sound_volume) {
            Ok(audio) => Some(audio),
            Err(error) => {
                last_error = Some(error.to_string());
                None
            }
        };
        let mut runtime = initialize_runtime_with(&mut scene, |action| {
            play_audio_action(audio.as_mut(), action)
        })?;
        let player = runtime
            .player_actor()
            .context("Lev_Tut1 has no registered PlayerPawn actor")?;
        let mut deferred_calls = 0;
        match runtime.dispatch_player_event("Possess", &[]) {
            Ok(actions) => {
                deferred_calls +=
                    apply_runtime_actions_with(&mut scene, &mut runtime, actions, |action| {
                        play_audio_action(audio.as_mut(), action)
                    })?
                    .1;
            }
            Err(error) => last_error = Some(format!("player possession deferred: {error}")),
        }

        let player_actor = scene
            .actors
            .get(player)
            .context("the registered player is missing from the scene")?;
        let player_location = player_actor.location.to_array();
        let player_rotation = [
            player_actor.rotation.pitch,
            player_actor.rotation.yaw,
            player_actor.rotation.roll,
        ];
        let fallback_view = PlayerView {
            actor: player,
            location: player_location,
            rotation: player_rotation,
            fov_degrees: 90.0,
        };
        let player_view = match runtime.player_view(fallback_view.location, fallback_view.rotation)
        {
            Ok((view, actions)) => {
                deferred_calls +=
                    apply_runtime_actions_with(&mut scene, &mut runtime, actions, |action| {
                        play_audio_action(audio.as_mut(), action)
                    })?
                    .1;
                view
            }
            Err(error) => {
                last_error = Some(format!("player camera deferred: {error}"));
                fallback_view
            }
        };

        let size = nonzero_size(window.inner_size());
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(Arc::clone(&window))
            .context("failed to create the game render surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("failed to find a compatible graphics adapter")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("OpenHP1 game device"),
            ..Default::default()
        }))
        .context("failed to create the graphics device")?;
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("the graphics adapter does not support this surface")?;
        let linear_format = config.format.remove_srgb_suffix();
        if surface
            .get_capabilities(&adapter)
            .formats
            .contains(&linear_format)
        {
            config.format = linear_format;
        }
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);
        let renderer = Renderer::new(
            &device,
            &queue,
            config.format,
            &scene.render,
            [size.width, size.height],
        );
        let far = (renderer.bounds().radius().max(100.0) * 10.0).max(10_000.0);
        let camera = camera_from_player_view(player_view, size, far);
        let egui_context = egui::Context::default();
        let egui = egui_winit::State::new(
            egui_context,
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            camera,
            scene,
            runtime,
            audio,
            player,
            input: InputState::default(),
            last_frame: Instant::now(),
            last_error,
            deferred_calls,
            view_actor: player_view.actor,
            render_stats: RenderStats::default(),
            frame_time_ms: 0.0,
            overlay_visible: true,
            egui,
            egui_renderer,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.renderer
            .resize(&self.device, [size.width, size.height]);
    }

    fn mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if !self.input.captured && state == ElementState::Pressed {
            self.capture_input();
        }
        self.input.set_mouse_button(button, state);
    }

    fn capture_input(&mut self) {
        let result = self
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
        match result {
            Ok(()) => {
                self.window.set_cursor_visible(false);
                self.input.captured = true;
            }
            Err(error) => self.last_error = Some(format!("could not capture the mouse: {error}")),
        }
    }

    fn release_input(&mut self) {
        self.input.clear();
        if self.input.captured {
            if let Err(error) = self.window.set_cursor_grab(CursorGrabMode::None) {
                self.last_error = Some(format!("could not release the mouse: {error}"));
            }
            self.window.set_cursor_visible(true);
            self.input.captured = false;
        }
    }

    fn render(&mut self) {
        let now = Instant::now();
        let delta_time = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.frame_time_ms = delta_time * 1_000.0;
        let ticks = if self.input.keys.iter().copied().any(is_fast_forward_key) {
            DEBUG_FAST_FORWARD_TICKS
        } else {
            1
        };
        let mut input = self.input.player_input(delta_time);
        for _ in 0..ticks {
            self.renderer.advance_time(delta_time);
            self.update_animations(delta_time);
            self.update_runtime(delta_time, input);
            input = repeated_player_input(input);
        }

        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            CurrentSurfaceTexture::Lost | CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return,
            CurrentSurfaceTexture::Validation => {
                self.last_error = Some("wgpu rejected the game surface".to_owned());
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let egui_context = self.egui.egui_ctx().clone();
        let egui_input = self.egui.take_egui_input(&self.window);
        let egui_output = egui_context.run_ui(egui_input, |ui| self.debug_overlay(ui.ctx()));
        self.egui
            .handle_platform_output(&self.window, egui_output.platform_output);
        for (id, delta) in &egui_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let paint_jobs = egui_context.tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: egui_output.pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("OpenHP1 game frame"),
            });
        self.render_stats = self.renderer.render(
            &self.queue,
            &mut encoder,
            &view,
            &self.camera,
            [self.config.width, self.config.height],
            0.625,
        );
        let mut commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OpenHP1 diagnostics overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui_renderer
                .render(&mut pass.forget_lifetime(), &paint_jobs, &screen);
        }
        commands.push(encoder.finish());
        self.queue.submit(commands);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        frame.present();
    }

    fn debug_overlay(&self, context: &egui::Context) {
        if !self.overlay_visible {
            return;
        }
        let player = self.scene.actors.get(self.player);
        let player_name = player.map_or("<missing>", |actor| actor.name.as_str());
        let location = player.map_or(Vec3::ZERO, |actor| actor.location);
        let camera_location = self
            .scene
            .actors
            .get(self.view_actor)
            .map_or(Vec3::ZERO, |actor| actor.location);
        let rotation = player.map_or([0; 3], |actor| {
            [
                actor.rotation.pitch,
                actor.rotation.yaw,
                actor.rotation.roll,
            ]
        });
        egui::Window::new("OpenHP1 diagnostics")
            .default_pos([12.0, 12.0])
            .resizable(false)
            .show(context, |ui| {
                ui.monospace(format!(
                    "{}  {:.1} ms ({:.0} fps)",
                    self.scene
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    self.frame_time_ms,
                    1_000.0 / self.frame_time_ms.max(0.001),
                ));
                ui.separator();
                ui.monospace(format!("player: {player_name} #{}", self.player));
                ui.monospace(format!(
                    "location: {:.1}, {:.1}, {:.1}",
                    location.x, location.y, location.z
                ));
                ui.monospace(format!(
                    "rotation: {}, {}, {}",
                    rotation[0], rotation[1], rotation[2]
                ));
                ui.monospace(format!("camera actor: #{}", self.view_actor));
                ui.monospace(format!(
                    "camera location: {:.1}, {:.1}, {:.1}",
                    camera_location.x, camera_location.y, camera_location.z
                ));
                ui.separator();
                ui.monospace(format!(
                    "{} actors  {} triangles  {} draw calls",
                    self.runtime.active_actor_count(),
                    self.scene.render.mesh.indices.len() / 3,
                    self.render_stats.draw_calls
                ));
                ui.monospace(format!("{} deferred runtime calls", self.deferred_calls));
                if self.input.keys.iter().copied().any(is_fast_forward_key) {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("{DEBUG_FAST_FORWARD_TICKS}x debug fast-forward"),
                    );
                }
                if let Some(error) = &self.last_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.separator();
                ui.label("W/S or ↑/↓ move · A/D or ←/→ turn");
                ui.label("Left click/Ctrl/Space jump · Right click/Alt cast");
                ui.label("Mouse aims while casting · Esc releases mouse · F1 toggles diagnostics");
                ui.label("Hold + or F to fast-forward normal game ticks at 16x");
            });
    }

    fn update_animations(&mut self, delta_time: f32) {
        match self.scene.tick_animations_with_completions(delta_time) {
            Ok((changed, completed)) => {
                for (actor, delta) in self.scene.take_root_motions() {
                    match self.runtime.apply_root_motion(actor, delta.to_array()) {
                        Ok(actions) => self.apply_actions(actions),
                        Err(error) => {
                            self.last_error = Some(format!("root motion failed: {error}"));
                        }
                    }
                }
                if changed {
                    self.update_vertices();
                }
                for actor in completed {
                    match self.runtime.animation_finished(actor) {
                        Ok(actions) => self.apply_actions(actions),
                        Err(error) => {
                            self.last_error = Some(format!("animation callback failed: {error}"));
                        }
                    }
                }
            }
            Err(error) => self.last_error = Some(format!("animation failed: {error:#}")),
        }
        match self.scene.tick_water(delta_time) {
            Ok(changed)
                if !self.renderer.update_textures(
                    &self.queue,
                    &self.scene.render.textures,
                    &changed,
                ) =>
            {
                self.last_error = Some("water changed the scene textures".to_owned());
            }
            Ok(_) => {}
            Err(error) => self.last_error = Some(format!("water animation failed: {error:#}")),
        }
    }

    fn update_runtime(&mut self, delta_time: f32, input: PlayerInput) {
        match self.runtime.tick(delta_time) {
            Ok(actions) => self.apply_actions(actions),
            Err(error) => self.last_error = Some(format!("runtime tick failed: {error}")),
        }

        match self.runtime.tick_player(input, delta_time) {
            Ok(actions) => self.apply_actions(actions),
            Err(error) => self.last_error = Some(format!("player tick failed: {error}")),
        }
        let Some(player) = self.scene.actors.get(self.player) else {
            self.last_error = Some("the player disappeared from the scene".to_owned());
            return;
        };
        let location = player.location.to_array();
        let rotation = [
            player.rotation.pitch,
            player.rotation.yaw,
            player.rotation.roll,
        ];
        match self.runtime.update_player_touches(location) {
            Ok(actions) => self.apply_actions(actions),
            Err(error) => self.last_error = Some(format!("trigger update failed: {error}")),
        }
        match self.runtime.take_player_music() {
            Ok(Some(music)) => {
                if let Some(audio) = self.audio.as_mut() {
                    let result = match music.clip {
                        Some(clip) => audio.play_music(&clip, 1.0),
                        None => {
                            audio.stop_music();
                            Ok(())
                        }
                    };
                    if let Err(error) = result {
                        self.last_error = Some(error.to_string());
                    }
                }
            }
            Ok(None) => {}
            Err(error) => self.last_error = Some(format!("music update failed: {error}")),
        }
        match self.runtime.player_view(location, rotation) {
            Ok((view, actions)) => {
                self.apply_actions(actions);
                self.view_actor = view.actor;
                self.camera = camera_from_player_view(
                    view,
                    PhysicalSize::new(self.config.width, self.config.height),
                    self.camera.far,
                );
                if self.scene.update_sprite_billboards(Rotator {
                    pitch: view.rotation[0],
                    yaw: view.rotation[1],
                    roll: view.rotation[2],
                }) {
                    self.update_vertices();
                }
                self.update_audio();
            }
            Err(error) => self.last_error = Some(format!("player camera failed: {error}")),
        }
    }

    fn update_audio(&mut self) {
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        let actor_positions = self
            .scene
            .actors
            .iter()
            .map(|actor| unreal_to_render(actor.location).to_array())
            .collect::<Vec<_>>();
        audio.update(
            self.camera.position.to_array(),
            camera_orientation(&self.camera),
            &actor_positions,
        );
    }

    fn apply_actions(&mut self, actions: Vec<ActorAction>) {
        let audio = &mut self.audio;
        match apply_runtime_actions_with(&mut self.scene, &mut self.runtime, actions, |action| {
            play_audio_action(audio.as_mut(), action)
        }) {
            Ok((_, deferred, transformed)) => {
                self.deferred_calls += deferred;
                if transformed {
                    self.update_vertices();
                }
            }
            Err(error) => self.last_error = Some(format!("runtime action failed: {error:#}")),
        }
    }

    fn update_vertices(&mut self) {
        if !self
            .renderer
            .update_vertices(&self.queue, &self.scene.render.mesh)
        {
            self.renderer
                .reload_scene(&self.device, &self.queue, &self.scene.render);
        }
    }
}

fn repeated_player_input(input: PlayerInput) -> PlayerInput {
    PlayerInput {
        mouse_x: 0.0,
        mouse_y: 0.0,
        jump: false,
        ..input
    }
}

fn is_fast_forward_key(key: KeyCode) -> bool {
    matches!(key, KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::KeyF)
}

fn audio_volumes(scene: &LoadedScene) -> (f32, f32) {
    let subsystem = scene
        .config_value("Engine.Engine", "AudioDevice")
        .unwrap_or_else(|| "Galaxy.GalaxyAudioSubsystem".to_owned());
    let volume = |key, fallback| {
        scene
            .config_value(&subsystem, key)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(fallback)
            / 255.0
    };
    (
        volume("MusicVolume", 160.0),
        volume("SoundVolume", 200.0) * 0.5,
    )
}

fn play_audio_action(audio: Option<&mut AudioPlayer>, action: ActorAction) -> Result<()> {
    let Some(audio) = audio else {
        return Ok(());
    };
    if let ActorAction::PlaySound {
        actor,
        clip,
        location,
        slot,
        volume,
        no_override,
        radius,
        pitch,
    } = action
    {
        audio.play_sound(
            actor,
            &clip,
            unreal_to_render(Vec3::from_array(location)).to_array(),
            slot,
            volume,
            no_override,
            radius,
            pitch,
        )?;
    }
    Ok(())
}

fn camera_orientation(camera: &Camera) -> [f32; 4] {
    let forward = camera.forward();
    let right = camera.right();
    let up = Quat::from_axis_angle(forward, camera.roll) * right.cross(forward);
    Quat::from_mat3(&Mat3::from_cols(right, up, -forward)).to_array()
}

fn camera_from_player_view(view: PlayerView, viewport: PhysicalSize<u32>, far: f32) -> Camera {
    let aspect = viewport.width.max(1) as f32 / viewport.height.max(1) as f32;
    let rotation = view.rotation.map(|value| value as f32 * ROTATOR_RADIANS);
    Camera {
        position: unreal_to_render(Vec3::from_array(view.location)),
        yaw: rotation[1],
        pitch: -rotation[0],
        roll: -rotation[2],
        vertical_fov: horizontal_to_vertical_fov(view.fov_degrees.to_radians(), aspect),
        near: 1.0,
        far,
    }
}

fn horizontal_to_vertical_fov(horizontal: f32, aspect: f32) -> f32 {
    2.0 * ((horizontal * 0.5).tan() / aspect).atan()
}

fn nonzero_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_the_ue1_player_view_once() {
        let camera = camera_from_player_view(
            PlayerView {
                actor: 7,
                location: [10.0, 20.0, 30.0],
                rotation: [8_192, 16_384, -8_192],
                fov_degrees: 90.0,
            },
            PhysicalSize::new(1600, 900),
            10_000.0,
        );
        assert_eq!(camera.position, Vec3::new(20.0, 30.0, -10.0));
        assert!((camera.yaw - TAU * 0.25).abs() < 0.000_001);
        assert!((camera.pitch + TAU * 0.125).abs() < 0.000_001);
        assert!((camera.roll - TAU * 0.125).abs() < 0.000_001);
        assert!(
            (camera.vertical_fov - 2.0 * (1.0_f32 / (1600.0 / 900.0)).atan()).abs() < 0.000_001
        );
    }

    #[test]
    fn maps_desktop_controls_to_original_input_axes() {
        let mut input = InputState::default();
        input.set_key(KeyCode::KeyW, ElementState::Pressed);
        input.set_key(KeyCode::KeyD, ElementState::Pressed);
        input.set_key(KeyCode::Space, ElementState::Pressed);
        input.mouse_delta = (2.0, -1.0);
        let player = input.player_input(1.0 / 60.0);
        assert_eq!(player.base_x, 3_000.0);
        assert_eq!(player.base_y, 6_000.0);
        assert_eq!(player.strafe, 0.0);
        assert_eq!(player.mouse_x, 0.0);
        assert_eq!(player.mouse_y, 0.0);
        assert!(!player.alt_fire);
        assert!(player.jump);
        assert!(!input.player_input(1.0 / 60.0).jump);

        input.set_mouse_button(MouseButton::Right, ElementState::Pressed);
        input.mouse_delta = (2.0, -1.0);
        let player = input.player_input(1.0 / 60.0);
        assert_eq!(player.base_x, 0.0);
        assert_eq!(player.base_y, 0.0);
        assert!((player.mouse_x - 76.8).abs() < 0.000_01);
        assert!((player.mouse_y - 38.4).abs() < 0.000_01);
        assert!(player.alt_fire);

        input.set_mouse_button(MouseButton::Right, ElementState::Released);
        input.set_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).jump);
        input.set_key(KeyCode::AltLeft, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).alt_fire);
    }

    #[test]
    fn fast_forward_repeats_held_but_not_transient_input() {
        let repeated = repeated_player_input(PlayerInput {
            base_y: 6_000.0,
            mouse_x: 76.8,
            alt_fire: true,
            jump: true,
            ..PlayerInput::default()
        });
        assert_eq!(repeated.base_y, 6_000.0);
        assert!(repeated.alt_fire);
        assert_eq!(repeated.mouse_x, 0.0);
        assert!(!repeated.jump);

        assert!(is_fast_forward_key(KeyCode::Equal));
        assert!(is_fast_forward_key(KeyCode::NumpadAdd));
        assert!(is_fast_forward_key(KeyCode::KeyF));
        assert!(!is_fast_forward_key(KeyCode::F2));
    }
}
