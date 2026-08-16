use std::{
    collections::HashSet,
    f32::consts::TAU,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use glam::{Mat3, Quat, Vec3};
use openhp1_audio::AudioPlayer;
use openhp1_render::{Camera, DisplaySettings, RenderStats, Renderer, RendererSettings};
use openhp1_runtime::{
    ActorAction, ConsoleCommandAction, ConsoleCommandHost, ConsoleCommands, PlayerInput,
    PlayerTravelState, PlayerView, ScriptRuntime, Value,
};
use openhp1_scene::{
    LoadedScene, Rotator, apply_runtime_actions_with, initialize_runtime_with_console,
    unreal_to_render,
};
use tracing::error;
use wgpu::{CurrentSurfaceTexture, SurfaceConfiguration};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize, Size},
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key, KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId},
};

use self::{
    console::DeveloperConsole,
    graphics_settings::{ColorDepth, GraphicsSettings, RESOLUTION_PRESETS, window_size},
    presentation::Presentation,
    ui::GameUi,
};

mod console;
mod graphics_settings;
mod presentation;
mod ui;

const ROTATOR_RADIANS: f32 = TAU / 65_536.0;
const DEBUG_FAST_FORWARD_TICKS: usize = 16;
const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

pub(crate) struct GameApp {
    scene: Option<LoadedScene>,
    graphics: Option<Graphics>,
    renderer_override: Option<RendererSettings>,
    next_redraw: Option<Instant>,
}

impl GameApp {
    pub(crate) fn new(scene: LoadedScene, renderer_override: Option<RendererSettings>) -> Self {
        Self {
            scene: Some(scene),
            graphics: None,
            renderer_override,
            next_redraw: None,
        }
    }
}

fn next_redraw_deadline(frame_started: Instant, now: Instant) -> Instant {
    (frame_started + FRAME_INTERVAL).max(now)
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let Some(scene) = self.scene.take() else {
            return;
        };
        let window_size = window_size(&scene);
        let attributes = WindowAttributes::default()
            .with_title("OpenHP1")
            .with_inner_size(Size::Logical(LogicalSize::new(
                f64::from(window_size[0]),
                f64::from(window_size[1]),
            )));
        let result = event_loop
            .create_window(attributes)
            .context("failed to create the game window")
            .and_then(|window| {
                Graphics::new(Arc::new(window), scene, self.renderer_override.take())
            });
        match result {
            Ok(mut graphics) => {
                if !graphics.game_ui.is_open() {
                    graphics.capture_input();
                }
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
        if matches!(&event, WindowEvent::RedrawRequested) {
            let Some(mut graphics) = self.graphics.take() else {
                return;
            };
            if graphics.window.id() != window_id {
                self.graphics = Some(graphics);
                return;
            }
            self.next_redraw = None;
            match graphics.render() {
                RenderOutcome::Continue => {
                    let deadline = next_redraw_deadline(graphics.last_frame, Instant::now());
                    self.next_redraw = Some(deadline);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                    self.graphics = Some(graphics);
                }
                RenderOutcome::Exit => event_loop.exit(),
                RenderOutcome::Load(saved) => {
                    let window = Arc::clone(&graphics.window);
                    let graphics_settings = graphics.graphics_settings;
                    let slot = saved.slot;
                    let path = saved.map_path(&graphics.scene.path);
                    match path.and_then(LoadedScene::load).and_then(|scene| {
                        Graphics::new_with_save(window, scene, &saved.bytes, graphics_settings)
                    }) {
                        Ok(mut replacement) => {
                            replacement.last_save_slot = Some(slot);
                            std::mem::swap(
                                &mut replacement.debug_console,
                                &mut graphics.debug_console,
                            );
                            if !replacement.game_ui.is_open() {
                                replacement.capture_input();
                            }
                            replacement.window.request_redraw();
                            self.graphics = Some(replacement);
                        }
                        Err(error) => {
                            graphics.last_error = Some(format!(
                                "could not load saved game {}: {error:#}",
                                saved.map
                            ));
                            self.graphics = Some(graphics);
                        }
                    }
                }
                RenderOutcome::LoadLevel(path, save_slot, travel) => {
                    let window = Arc::clone(&graphics.window);
                    let graphics_settings = graphics.graphics_settings;
                    match LoadedScene::load(path.clone())
                        .and_then(|scene| {
                            Graphics::new_with_settings(
                                window,
                                scene,
                                graphics_settings,
                                save_slot.is_some(),
                            )
                        })
                        .and_then(|mut replacement| {
                            if let Some(travel) = &travel {
                                replacement
                                    .runtime
                                    .restore_player_travel_state(travel)
                                    .context("could not restore player travel properties")?;
                            }
                            Ok(replacement)
                        }) {
                        Ok(mut replacement) => {
                            replacement.last_save_slot = save_slot;
                            if let Some(slot) = save_slot
                                && let Err(error) = replacement.save_game(slot)
                            {
                                replacement.last_error =
                                    Some(format!("could not save new level: {error:#}"));
                            }
                            replacement.game_ui.preserve_session_from(&graphics.game_ui);
                            std::mem::swap(
                                &mut replacement.debug_console,
                                &mut graphics.debug_console,
                            );
                            if !replacement.game_ui.is_open() {
                                replacement.capture_input();
                            }
                            replacement.window.request_redraw();
                            self.graphics = Some(replacement);
                        }
                        Err(error) => {
                            graphics.last_error =
                                Some(format!("could not load {}: {error:#}", path.display()));
                            self.graphics = Some(graphics);
                        }
                    }
                }
            }
            return;
        }
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };
        if graphics.window.id() != window_id {
            return;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && is_console_toggle_key(event.physical_key, &event.logical_key)
            && event.state == ElementState::Pressed
            && !event.repeat
        {
            graphics.debug_console.toggle();
            graphics.release_input();
            return;
        }
        let egui_response = graphics.egui.on_window_event(&graphics.window, &event);
        if graphics.debug_console.is_open() || graphics.game_ui.is_open() {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => graphics.resize(size),
                WindowEvent::Focused(false) => graphics.release_input(),
                WindowEvent::KeyboardInput { event, .. }
                    if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                        && event.state == ElementState::Pressed
                        && !event.repeat =>
                {
                    if graphics.game_ui.escape() {
                        graphics.capture_input();
                    }
                }
                _ => {}
            }
            return;
        }
        if let WindowEvent::MouseInput { state, button, .. } = &event
            && graphics.input.captured
        {
            graphics.mouse_button(*button, *state);
            return;
        }
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
                    graphics.game_ui.open_pause();
                } else {
                    graphics.input.set_key(code, event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                graphics.mouse_button(button, state);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(deadline) = self.next_redraw else {
            return;
        };
        if Instant::now() >= deadline {
            self.next_redraw = None;
            if let Some(graphics) = &self.graphics {
                graphics.window.request_redraw();
            }
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
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

enum RenderOutcome {
    Continue,
    Exit,
    Load(SavedGame),
    LoadLevel(PathBuf, Option<u32>, Option<PlayerTravelState>),
}

struct SavedGame {
    slot: u32,
    map: String,
    bytes: Vec<u8>,
}

impl SavedGame {
    fn map_path(&self, current_map: &Path) -> Result<PathBuf> {
        let game_root = current_map
            .parent()
            .and_then(|directory| directory.parent())
            .context("map path must be inside the game's Maps directory")?;
        let relative = Path::new(&self.map);
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
            || !relative
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("unr"))
        {
            anyhow::bail!("saved map identifier `{}` is invalid", self.map);
        }
        Ok(game_root.join(relative))
    }
}

#[derive(Default)]
struct InputState {
    keys: HashSet<KeyCode>,
    mouse_delta: (f64, f64),
    cast_mouse: bool,
    boost_mouse: bool,
    cast_requested: bool,
    cast_release_requested: bool,
    space_requested: bool,
    space_release_requested: bool,
    jump_requested: bool,
    captured: bool,
}

impl InputState {
    fn set_key(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                let first_press = self.keys.insert(key);
                if first_press {
                    if key == KeyCode::Space {
                        self.space_requested = true;
                        self.jump_requested = true;
                    } else if matches!(key, KeyCode::ControlLeft | KeyCode::ControlRight) {
                        self.jump_requested = true;
                    } else if matches!(key, KeyCode::AltLeft | KeyCode::AltRight) {
                        self.cast_requested = true;
                    }
                }
            }
            ElementState::Released => {
                self.keys.remove(&key);
                if key == KeyCode::Space {
                    self.space_release_requested = true;
                } else if matches!(key, KeyCode::AltLeft | KeyCode::AltRight)
                    && !self.cast_mouse
                    && !self.keys.contains(&KeyCode::AltLeft)
                    && !self.keys.contains(&KeyCode::AltRight)
                {
                    self.cast_release_requested = true;
                }
            }
        }
    }

    fn set_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        match button {
            MouseButton::Left => {
                let pressed = state == ElementState::Pressed;
                self.cast_requested |= pressed && !self.cast_mouse;
                self.cast_mouse = pressed;
                self.cast_release_requested |= !pressed
                    && !self.keys.contains(&KeyCode::AltLeft)
                    && !self.keys.contains(&KeyCode::AltRight);
            }
            MouseButton::Right => {
                let pressed = state == ElementState::Pressed;
                self.jump_requested |= pressed && !self.boost_mouse;
                self.boost_mouse = pressed;
            }
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
        let broom_pitch_up = pressed(&self.keys, &[KeyCode::KeyW, KeyCode::ArrowUp]) != 0.0;
        let broom_pitch_down = pressed(&self.keys, &[KeyCode::KeyS, KeyCode::ArrowDown]) != 0.0;
        let casting =
            self.cast_mouse || pressed(&self.keys, &[KeyCode::AltLeft, KeyCode::AltRight]) != 0.0;
        let input = PlayerInput {
            base_x: (right - left) * 3_000.0,
            base_y: forward * 6_000.0 - backward * 3_000.0,
            strafe: (right - left) * 6_000.0,
            mouse_x: mouse_axis(self.mouse_delta.0, delta_time, 6.0),
            mouse_y: mouse_axis(-self.mouse_delta.1, delta_time, 6.0),
            alt_fire: casting,
            alt_fire_pressed: self.cast_requested,
            alt_fire_released: self.cast_release_requested,
            space_pressed: self.space_requested,
            space_released: self.space_release_requested,
            jump: self.jump_requested,
            broom_pitch_up,
            broom_pitch_down,
            broom_boost: pressed(&self.keys, &[KeyCode::KeyZ]) != 0.0,
            broom_brake: pressed(&self.keys, &[KeyCode::KeyX]) != 0.0,
        };
        self.mouse_delta = (0.0, 0.0);
        self.cast_requested = false;
        self.cast_release_requested = false;
        self.space_requested = false;
        self.space_release_requested = false;
        self.jump_requested = false;
        input
    }

    fn clear(&mut self) {
        self.keys.clear();
        self.mouse_delta = (0.0, 0.0);
        self.cast_mouse = false;
        self.boost_mouse = false;
        self.cast_requested = false;
        self.cast_release_requested = false;
        self.space_requested = false;
        self.space_release_requested = false;
        self.jump_requested = false;
    }
}

fn mouse_axis(delta: f64, delta_time: f32, speed: f32) -> f32 {
    if !delta_time.is_finite() || delta_time <= 0.0 {
        return 0.0;
    }
    const DESKTOP_MOUSE_SCALE: f32 = 2.5;
    delta as f32 * 16.0 * speed * DESKTOP_MOUSE_SCALE / (delta_time * 150.0)
}

fn is_console_toggle_key(physical: PhysicalKey, logical: &Key) -> bool {
    physical == PhysicalKey::Code(KeyCode::Backquote)
        || matches!(logical, Key::Character(character) if matches!(character.as_str(), "`" | "~"))
}

struct Graphics {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: SurfaceConfiguration,
    presentation: Presentation,
    renderer: Renderer,
    camera: Camera,
    scene: LoadedScene,
    runtime: ScriptRuntime,
    console: ConsoleCommands,
    audio: Option<AudioPlayer>,
    player: usize,
    input: InputState,
    last_frame: Instant,
    last_error: Option<String>,
    deferred_calls: usize,
    view_actor: usize,
    render_stats: RenderStats,
    frame_time_ms: f32,
    vertices_dirty: bool,
    overlay_visible: bool,
    debug_console: DeveloperConsole,
    game_ui: GameUi,
    pending_level_load: Option<PathBuf>,
    pending_level_travel: Option<String>,
    fly_camera_active: bool,
    egui: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    screenshot_dir: PathBuf,
    save_dir: PathBuf,
    last_save_slot: Option<u32>,
    pending_screenshots: Vec<Option<u32>>,
    graphics_settings: GraphicsSettings,
    display_settings: DisplaySettings,
    screen_flash: [f32; 4],
}

fn initialize_saved_runtime(
    scene: &mut LoadedScene,
    console: ConsoleCommands,
    in_hub_flow: bool,
) -> Result<(ScriptRuntime, usize)> {
    let mut runtime = initialize_runtime_with_console(scene, console, in_hub_flow, |_| Ok(()))?;
    let mut deferred_calls = 0;
    let actions = runtime.dispatch_player_event("Possess", &[])?;
    deferred_calls += apply_runtime_actions_with(scene, &mut runtime, actions, |_| Ok(()))?.1;
    let actions = runtime.initialize_player_hud()?;
    deferred_calls += apply_runtime_actions_with(scene, &mut runtime, actions, |_| Ok(()))?.1;
    Ok((runtime, deferred_calls))
}

impl Graphics {
    fn new(
        window: Arc<Window>,
        scene: LoadedScene,
        renderer_override: Option<RendererSettings>,
    ) -> Result<Self> {
        Self::new_inner(window, scene, None, None, renderer_override, false)
    }

    fn new_with_settings(
        window: Arc<Window>,
        scene: LoadedScene,
        graphics_settings: GraphicsSettings,
        in_hub_flow: bool,
    ) -> Result<Self> {
        Self::new_inner(
            window,
            scene,
            None,
            Some(graphics_settings),
            None,
            in_hub_flow,
        )
    }

    fn new_with_save(
        window: Arc<Window>,
        scene: LoadedScene,
        bytes: &[u8],
        graphics_settings: GraphicsSettings,
    ) -> Result<Self> {
        Self::new_inner(
            window,
            scene,
            Some(bytes),
            Some(graphics_settings),
            None,
            true,
        )
    }

    fn new_inner(
        window: Arc<Window>,
        mut scene: LoadedScene,
        saved: Option<&[u8]>,
        graphics_settings: Option<GraphicsSettings>,
        renderer_override: Option<RendererSettings>,
        in_hub_flow: bool,
    ) -> Result<Self> {
        let mut last_error = None;
        let game_root = scene
            .path
            .parent()
            .and_then(|directory| directory.parent())
            .context("map path must be inside the game's Maps directory")?
            .to_path_buf();
        let initial_size = window.inner_size();
        let resolutions = RESOLUTION_PRESETS
            .iter()
            .map(|(size, _)| (size[0], size[1]))
            .collect::<Vec<_>>();
        let console = ConsoleCommands::production(
            &game_root,
            (initial_size.width, initial_size.height),
            resolutions,
        )
        .context("could not configure game console commands")?;
        let graphics_settings = match graphics_settings {
            Some(settings) => settings,
            None => {
                let settings = GraphicsSettings::load(&console, renderer_override);
                if let Err(error) = settings.save(&console) {
                    last_error = Some(format!("could not initialize graphics settings: {error}"));
                }
                settings
            }
        };
        let display_settings = graphics_settings.display();
        let screenshot_dir = console.settings_dir().join("Screenshots");
        let settings_dir = console.settings_dir().to_path_buf();
        let save_dir = console.settings_dir().join("Saves");
        let (music_volume, sound_volume, sound_latency) = audio_settings(&scene);
        let mut audio = match AudioPlayer::new(music_volume, sound_volume, sound_latency) {
            Ok(audio) => Some(audio),
            Err(error) => {
                last_error = Some(error.to_string());
                None
            }
        };
        let (mut runtime, mut deferred_calls) = if saved.is_some() {
            initialize_saved_runtime(&mut scene, console.clone(), in_hub_flow)?
        } else {
            (
                initialize_runtime_with_console(
                    &mut scene,
                    console.clone(),
                    in_hub_flow,
                    |action| play_audio_action(audio.as_mut(), action),
                )?,
                0,
            )
        };
        let player = runtime
            .player_actor()
            .context("Lev_Tut1 has no registered PlayerPawn actor")?;
        if let Some(saved) = saved {
            let map = map_identifier(&scene.path, &game_root)?;
            let actions = runtime.restore_game(&map, saved)?;
            deferred_calls +=
                apply_runtime_actions_with(&mut scene, &mut runtime, actions, |action| {
                    play_audio_action(audio.as_mut(), action)
                })?
                .1;
        } else if let Err(error) = (|| -> Result<()> {
            let actions = runtime.dispatch_player_event("Possess", &[])?;
            deferred_calls +=
                apply_runtime_actions_with(&mut scene, &mut runtime, actions, |action| {
                    play_audio_action(audio.as_mut(), action)
                })?
                .1;
            let actions = runtime.initialize_player_hud()?;
            deferred_calls +=
                apply_runtime_actions_with(&mut scene, &mut runtime, actions, |action| {
                    play_audio_action(audio.as_mut(), action)
                })?
                .1;
            Ok(())
        })() {
            last_error = Some(format!("player initialization deferred: {error}"));
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
            flash_fog: [0.0, 0.0, 0.0, 1.0],
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
        scene.sync_particle_emitters(runtime.particle_emitters()?)?;
        scene.tick_particles(1.0 / 60.0);
        for (actor, emitted) in scene.particle_counts() {
            runtime.set_particle_counts(actor, emitted)?;
        }
        scene.sync_weapon_attachments(runtime.weapon_attachments()?)?;

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
        config.present_mode = wgpu::PresentMode::AutoNoVsync;
        config.usage |= wgpu::TextureUsages::COPY_SRC;
        surface.configure(&device, &config);
        let presentation = Presentation::new(&device, config.format, graphics_settings.resolution);
        let renderer = Renderer::new_with_settings(
            &device,
            &queue,
            config.format,
            &scene.render,
            presentation.size(),
            graphics_settings.renderer,
        );
        let far = (renderer.bounds().radius().max(100.0) * 10.0).max(10_000.0);
        let camera = camera_from_player_view(
            player_view,
            PhysicalSize::new(presentation.size()[0], presentation.size()[1]),
            far,
        );
        let egui_context = egui::Context::default();
        let egui = egui_winit::State::new(
            egui_context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        let game_ui = GameUi::load(
            &egui_context,
            &game_root,
            &scene.path,
            &settings_dir,
            &save_dir,
            ui::OptionsState {
                graphics: graphics_settings,
                music_volume,
                sound_volume,
            },
        )?;
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            presentation,
            renderer,
            camera,
            scene,
            runtime,
            console,
            audio,
            player,
            input: InputState::default(),
            last_frame: Instant::now(),
            last_error,
            deferred_calls,
            view_actor: player_view.actor,
            render_stats: RenderStats::default(),
            frame_time_ms: 0.0,
            vertices_dirty: false,
            overlay_visible: false,
            debug_console: DeveloperConsole::new(),
            game_ui,
            pending_level_load: None,
            pending_level_travel: None,
            fly_camera_active: false,
            egui,
            egui_renderer,
            screenshot_dir,
            save_dir,
            last_save_slot: None,
            pending_screenshots: Vec::new(),
            graphics_settings,
            display_settings,
            screen_flash: player_view.flash_fog,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
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

    fn render(&mut self) -> RenderOutcome {
        if let Some(path) = self.pending_level_load.take() {
            return RenderOutcome::LoadLevel(path, None, None);
        }
        let now = Instant::now();
        let delta_time = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.frame_time_ms = delta_time * 1_000.0;
        let ticks = if self.input.keys.iter().copied().any(is_fast_forward_key) {
            DEBUG_FAST_FORWARD_TICKS
        } else {
            1
        };
        let mut input = if self.fly_camera_active {
            update_fly_camera(
                &mut self.camera,
                &self.input,
                delta_time,
                (self.renderer.bounds().radius() * 0.35).max(200.0),
            );
            let _ = self.input.player_input(delta_time);
            PlayerInput::default()
        } else {
            self.input.player_input(delta_time)
        };
        if !self.game_ui.pauses_game() {
            for _ in 0..ticks {
                self.renderer.advance_time(delta_time);
                self.update_animations(delta_time);
                self.update_runtime(delta_time, input);
                input = repeated_player_input(input);
            }
        }
        if let Some(url) = self.pending_level_travel.take() {
            if console::commands::is_restart_travel(&url) {
                return RenderOutcome::LoadLevel(
                    self.scene.path.clone(),
                    self.last_save_slot,
                    None,
                );
            }
            match console::commands::resolve_travel(&self.scene.path, &self.scene.levels, &url) {
                Ok(path) => match self.runtime.player_travel_state() {
                    Ok(travel) => {
                        return RenderOutcome::LoadLevel(path, self.last_save_slot, Some(travel));
                    }
                    Err(error) => {
                        self.last_error = Some(format!(
                            "could not preserve player state while travelling: {error}"
                        ));
                    }
                },
                Err(error) => {
                    self.last_error = Some(format!("could not travel to {url}: {error:#}"))
                }
            }
        }
        self.update_audio();
        match self.runtime.update_player_hud_game() {
            Ok(actions) => self.apply_actions(actions),
            Err(error) => self.last_error = Some(format!("HUD update failed: {error}")),
        }
        if let Ok(player) = self.runtime.player_ui_state() {
            self.game_ui.set_player_state(player);
        }
        if self.vertices_dirty {
            self.update_vertices();
        }
        match self.apply_console_commands() {
            RenderOutcome::Continue => {}
            outcome => return outcome,
        }

        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            CurrentSurfaceTexture::Lost | CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return RenderOutcome::Continue;
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return RenderOutcome::Continue;
            }
            CurrentSurfaceTexture::Validation => {
                self.last_error = Some("wgpu rejected the game surface".to_owned());
                return RenderOutcome::Continue;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let egui_context = self.egui.egui_ctx().clone();
        let mut egui_input = self.egui.take_egui_input(&self.window);
        self.presentation.transform_input(
            &mut egui_input,
            [self.config.width, self.config.height],
            self.window.scale_factor() as f32,
        );
        let egui_output = egui_context.run_ui(egui_input, |ui| {
            self.game_ui.ui(ui.ctx());
            self.debug_overlay(ui.ctx());
            self.debug_console.ui(ui);
        });
        self.run_debug_console_commands();
        self.egui
            .handle_platform_output(&self.window, egui_output.platform_output);
        for (id, delta) in &egui_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let paint_jobs = egui_context.tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let render_size = self.presentation.size();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: render_size,
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
            self.presentation.view(),
            &self.camera,
            render_size,
            self.display_settings,
            self.graphics_settings.visible_flash(self.screen_flash),
        );
        let screenshots = match prepare_screenshots(
            &self.device,
            &mut encoder,
            self.presentation.texture(),
            self.config.format,
            render_size[0],
            render_size[1],
            &self.screenshot_dir,
            std::mem::take(&mut self.pending_screenshots),
        ) {
            Ok(screenshots) => screenshots,
            Err(error) => {
                self.last_error = Some(format!("could not capture screenshot: {error}"));
                Vec::new()
            }
        };
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
                    view: self.presentation.view(),
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
        self.presentation.draw(
            &mut encoder,
            &view,
            [self.config.width, self.config.height],
            if self.graphics_settings.renderer.mode == openhp1_render::RendererMode::Classic {
                self.graphics_settings.color_depth
            } else {
                ColorDepth::TrueColor
            },
        );
        commands.push(encoder.finish());
        self.queue.submit(commands);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        frame.present();
        for screenshot in screenshots {
            if let Err(error) = write_screenshot(&self.device, screenshot) {
                self.last_error = Some(format!("could not save screenshot: {error}"));
            }
        }
        match self.game_ui.take_action() {
            Some(ui::Action::Exit) => RenderOutcome::Exit,
            Some(ui::Action::LoadSave(slot)) => match self.open_save(slot) {
                Ok(saved) => RenderOutcome::Load(saved),
                Err(error) => {
                    self.last_error = Some(format!("could not open saved game: {error:#}"));
                    RenderOutcome::Continue
                }
            },
            Some(ui::Action::LoadLevel(level)) => RenderOutcome::LoadLevel(
                self.scene
                    .path
                    .parent()
                    .expect("loaded map has a parent directory")
                    .join(level),
                None,
                None,
            ),
            Some(ui::Action::NewGame(slot)) => RenderOutcome::LoadLevel(
                self.scene
                    .path
                    .parent()
                    .expect("loaded map has a parent directory")
                    .join("Lev_Tut1.unr"),
                Some(slot),
                None,
            ),
            Some(ui::Action::PlayUiSound(clip)) => {
                if let Some(audio) = self.audio.as_mut()
                    && let Err(error) = audio.play_sound(
                        self.player,
                        &clip,
                        self.camera.position.to_array(),
                        3,
                        3.2,
                        false,
                        2_000.0,
                        1.0,
                    )
                {
                    self.last_error = Some(format!("could not play story narration: {error}"));
                }
                RenderOutcome::Continue
            }
            Some(ui::Action::Resume) => {
                self.capture_input();
                RenderOutcome::Continue
            }
            Some(ui::Action::ApplyGraphics(settings)) => {
                self.apply_graphics_settings(settings);
                RenderOutcome::Continue
            }
            Some(ui::Action::SaveGraphics(settings)) => {
                self.apply_graphics_settings(settings);
                if let Err(error) = settings.save(&self.console) {
                    self.last_error = Some(format!("could not save graphics settings: {error}"));
                }
                RenderOutcome::Continue
            }
            Some(ui::Action::SetMusicVolume(volume)) => {
                self.console.console_command(
                    self.player,
                    "PlayerPawn",
                    &format!("set ini:Engine.Engine.AudioDevice MusicVolume {volume}"),
                );
                self.console
                    .console_command(self.player, "PlayerPawn", "FLUSH");
                RenderOutcome::Continue
            }
            Some(ui::Action::SetMouseSensitivity(sensitivity)) => {
                self.dispatch_player_option("SetSensitivity", &[Value::Float(sensitivity)]);
                RenderOutcome::Continue
            }
            Some(ui::Action::SetAutoJump(enabled)) => {
                self.dispatch_player_option("AutoJump", &[Value::Bool(enabled)]);
                RenderOutcome::Continue
            }
            Some(ui::Action::SetInvertBroom(enabled)) => {
                self.dispatch_player_option("InvertBroomPitch", &[Value::Bool(enabled)]);
                RenderOutcome::Continue
            }
            Some(ui::Action::SetSoundVolume(volume)) => {
                self.console.console_command(
                    self.player,
                    "PlayerPawn",
                    &format!("set ini:Engine.Engine.AudioDevice SoundVolume {volume}"),
                );
                self.console
                    .console_command(self.player, "PlayerPawn", "FLUSH");
                RenderOutcome::Continue
            }
            None => RenderOutcome::Continue,
        }
    }

    fn run_debug_console_commands(&mut self) {
        for input in self.debug_console.take_submitted() {
            let result = console::commands::execute(self, &input);
            self.debug_console.record_result(result);
        }
    }

    fn apply_graphics_settings(&mut self, settings: GraphicsSettings) {
        let resolution_changed = self.graphics_settings.resolution != settings.resolution;
        let renderer_changed = self.graphics_settings.renderer != settings.renderer;
        if resolution_changed {
            self.presentation
                .resize(&self.device, self.config.format, settings.resolution);
        }
        if renderer_changed {
            self.renderer = Renderer::new_with_settings(
                &self.device,
                &self.queue,
                self.config.format,
                &self.scene.render,
                self.presentation.size(),
                settings.renderer,
            );
        } else if resolution_changed {
            self.renderer.resize(&self.device, self.presentation.size());
        }
        self.graphics_settings = settings;
        self.display_settings = settings.display();
        self.game_ui.set_graphics_settings(settings);
    }

    fn dispatch_player_option(&mut self, function: &str, arguments: &[Value]) {
        let result = (|| -> Result<()> {
            let actions = self.runtime.dispatch_player_event(function, arguments)?;
            self.deferred_calls += apply_runtime_actions_with(
                &mut self.scene,
                &mut self.runtime,
                actions,
                |action| play_audio_action(self.audio.as_mut(), action),
            )?
            .1;
            Ok(())
        })();
        if let Err(error) = result {
            self.last_error = Some(format!("could not apply {function}: {error:#}"));
        }
    }

    fn apply_console_commands(&mut self) -> RenderOutcome {
        let mut exit = false;
        let mut load = None;
        for action in self.console.take_actions() {
            match action {
                ConsoleCommandAction::Exit => exit = true,
                ConsoleCommandAction::SetResolution { width, height } => {
                    let mut settings = self.graphics_settings;
                    settings.resolution = [width, height];
                    self.apply_graphics_settings(settings);
                }
                ConsoleCommandAction::SetMusicVolume(volume) => {
                    if let Some(audio) = self.audio.as_mut() {
                        audio.set_music_volume(f32::from(volume) / 255.0);
                    }
                }
                ConsoleCommandAction::SetSoundVolume(volume) => {
                    if let Some(audio) = self.audio.as_mut() {
                        audio.set_sound_volume(f32::from(volume) / 255.0);
                    }
                }
                ConsoleCommandAction::Open(url) => {
                    if let Err(error) = open_url(&url) {
                        self.last_error = Some(format!("could not open {url}: {error}"));
                    }
                }
                ConsoleCommandAction::Screenshot { snapshot } => {
                    self.pending_screenshots.push(snapshot);
                }
                ConsoleCommandAction::SaveGame { slot } => {
                    let slot = active_save_slot(slot, self.last_save_slot);
                    if let Err(error) = self.save_game(slot) {
                        self.last_error = Some(format!("could not save game: {error:#}"));
                    } else {
                        self.last_save_slot = Some(slot);
                    }
                }
                ConsoleCommandAction::OpenSave { slot } => {
                    match self.open_save(active_save_slot(slot, self.last_save_slot)) {
                        Ok(saved) => load = Some(saved),
                        Err(error) => {
                            self.last_error = Some(format!("could not open saved game: {error:#}"));
                        }
                    }
                }
            }
        }
        if exit {
            RenderOutcome::Exit
        } else if let Some(saved) = load {
            RenderOutcome::Load(saved)
        } else {
            RenderOutcome::Continue
        }
    }

    fn save_game(&mut self, slot: u32) -> Result<()> {
        let game_root = self
            .scene
            .path
            .parent()
            .and_then(|directory| directory.parent())
            .context("map path must be inside the game's Maps directory")?;
        let map = map_identifier(&self.scene.path, game_root)?;
        let bytes = self.runtime.save_game(&map)?;
        write_save_atomically(&self.save_path(slot), &bytes)
    }

    fn open_save(&self, slot: u32) -> Result<SavedGame> {
        let path = self.save_path(slot);
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let map = ScriptRuntime::saved_game_map(&bytes)?;
        Ok(SavedGame { slot, map, bytes })
    }

    fn save_path(&self, slot: u32) -> PathBuf {
        self.save_dir.join(format!("save{slot}.usa"))
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
        let capability_diagnostics = self
            .scene
            .actors
            .iter()
            .map(|actor| actor.diagnostics.len())
            .sum::<usize>();
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
                ui.monospace(if self.fly_camera_active {
                    "camera mode: fly"
                } else {
                    "camera mode: play"
                });
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
                ui.monospace(format!(
                    "{capability_diagnostics} capability diagnostics (see log)"
                ));
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
                ui.label("Right click/Ctrl/Space jump · Left click/Alt cast");
                ui.label("Mouse aims · Esc releases mouse · F1 toggles diagnostics");
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
                    self.vertices_dirty = true;
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
        if let Err(error) = openhp1_scene::sync_runtime_pose(&self.scene, &mut self.runtime) {
            self.last_error = Some(format!("animation pose sync failed: {error:#}"));
        }
        match self.scene.tick_textures(delta_time) {
            Ok(changed)
                if !self.renderer.update_textures(
                    &self.device,
                    &self.queue,
                    &self.scene.render.textures,
                    &changed,
                ) =>
            {
                self.last_error = Some("animation changed the scene textures".to_owned());
            }
            Ok(_) => {}
            Err(error) => self.last_error = Some(format!("texture animation failed: {error:#}")),
        }
    }

    fn update_runtime(&mut self, delta_time: f32, input: PlayerInput) {
        match self
            .runtime
            .set_player_input(input)
            .and_then(|_| self.runtime.tick(delta_time))
        {
            Ok(actions) => self.apply_actions(actions),
            Err(error) => self.last_error = Some(format!("runtime tick failed: {error}")),
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
        match self.runtime.particle_emitters().and_then(|emitters| {
            self.scene
                .sync_particle_emitters(emitters)
                .map_err(|error| openhp1_runtime::DispatchError::UnresolvedObject {
                    message: error.to_string(),
                })
        }) {
            Ok(topology_changed) => {
                if topology_changed || self.scene.tick_particles(delta_time) {
                    self.vertices_dirty = true;
                }
                for (actor, emitted) in self.scene.particle_counts() {
                    if let Err(error) = self.runtime.set_particle_counts(actor, emitted) {
                        self.last_error = Some(format!("particle count update failed: {error}"));
                        break;
                    }
                }
            }
            Err(error) => self.last_error = Some(format!("particle update failed: {error}")),
        }
        match self.runtime.weapon_attachments().and_then(|attachments| {
            self.scene
                .sync_weapon_attachments(attachments)
                .map_err(|error| openhp1_runtime::DispatchError::UnresolvedObject {
                    message: error.to_string(),
                })
        }) {
            Ok(true) => self.vertices_dirty = true,
            Ok(false) => {}
            Err(error) => self.last_error = Some(format!("weapon attachment failed: {error}")),
        }
        match self.runtime.player_view(location, rotation) {
            Ok((view, actions)) => {
                self.apply_actions(actions);
                self.view_actor = view.actor;
                self.screen_flash = view.flash_fog;
                if !self.fly_camera_active {
                    self.camera = camera_from_player_view(
                        view,
                        PhysicalSize::new(self.presentation.size()[0], self.presentation.size()[1]),
                        self.camera.far,
                    );
                    if self.scene.update_sprite_billboards(Rotator {
                        pitch: view.rotation[0],
                        yaw: view.rotation[1],
                        roll: view.rotation[2],
                    }) {
                        self.vertices_dirty = true;
                    }
                }
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
        let game_ui = &mut self.game_ui;
        let pending_level_travel = &mut self.pending_level_travel;
        let mut opened_quidditch = false;
        match apply_runtime_actions_with(&mut self.scene, &mut self.runtime, actions, |action| {
            match action {
                ActorAction::ClientTravel { url, .. } => {
                    *pending_level_travel = Some(url);
                    Ok(())
                }
                ActorAction::UnlockQuidditch { level, .. } => game_ui.unlock_quidditch(level),
                ActorAction::FinishQuidditchMatch {
                    team0_score,
                    opponent_score,
                    ..
                } => {
                    game_ui.finish_quidditch_match(team0_score, opponent_score);
                    opened_quidditch = true;
                    Ok(())
                }
                action => play_audio_action(audio.as_mut(), action),
            }
        }) {
            Ok((_, deferred, transformed)) => {
                self.deferred_calls += deferred;
                if transformed {
                    self.vertices_dirty = true;
                }
            }
            Err(error) => self.last_error = Some(format!("runtime action failed: {error:#}")),
        }
        if opened_quidditch {
            self.release_input();
        }
    }

    fn update_vertices(&mut self) {
        self.vertices_dirty = false;
        let changed_lightmaps = self.scene.take_changed_lightmaps();
        let scene_updated = self.renderer.update_scene(&self.queue, &self.scene.render);
        let lightmaps_updated = scene_updated
            && self.renderer.update_lightmaps(
                &self.queue,
                &self.scene.render.lightmaps,
                &changed_lightmaps,
            );
        if !scene_updated || !lightmaps_updated {
            self.renderer
                .reload_scene(&self.device, &self.queue, &self.scene.render);
        }
    }
}

fn active_save_slot(requested: u32, selected: Option<u32>) -> u32 {
    if requested == 99 {
        selected.unwrap_or(requested)
    } else {
        requested
    }
}

fn update_fly_camera(
    camera: &mut Camera,
    input: &InputState,
    delta_time: f32,
    movement_speed: f32,
) {
    camera.yaw += input.mouse_delta.0 as f32 * 0.004;
    camera.pitch = (camera.pitch - input.mouse_delta.1 as f32 * 0.004).clamp(-1.55, 1.55);

    let pressed = |key| input.keys.contains(&key) as u8 as f32;
    let movement = Vec3::new(
        pressed(KeyCode::KeyD) - pressed(KeyCode::KeyA),
        pressed(KeyCode::KeyE) - pressed(KeyCode::KeyQ),
        pressed(KeyCode::KeyW) - pressed(KeyCode::KeyS),
    )
    .normalize_or_zero();
    let fast = pressed(KeyCode::ShiftLeft).max(pressed(KeyCode::ShiftRight));
    let speed = movement_speed * (1.0 + fast * 3.0);
    camera.position +=
        (camera.forward() * movement.z + camera.right() * movement.x + Vec3::Y * movement.y)
            * speed
            * delta_time;
}

fn repeated_player_input(input: PlayerInput) -> PlayerInput {
    PlayerInput {
        mouse_x: 0.0,
        mouse_y: 0.0,
        alt_fire_pressed: false,
        alt_fire_released: false,
        space_pressed: false,
        space_released: false,
        jump: false,
        ..input
    }
}

fn is_fast_forward_key(key: KeyCode) -> bool {
    matches!(key, KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::KeyF)
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    command.arg(url).spawn().map(|_| ())
}

fn audio_settings(scene: &LoadedScene) -> (f32, f32, Duration) {
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
    let latency = scene
        .config_value(&subsystem, "Latency")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(40);
    (
        volume("MusicVolume", 160.0),
        volume("SoundVolume", 200.0),
        Duration::from_millis(latency),
    )
}

fn play_audio_action(audio: Option<&mut AudioPlayer>, action: ActorAction) -> Result<()> {
    let Some(audio) = audio else {
        return Ok(());
    };
    match action {
        ActorAction::PlaySound {
            actor,
            clip,
            location,
            slot,
            volume,
            no_override,
            radius,
            pitch,
        } => audio.play_sound(
            actor,
            &clip,
            unreal_to_render(Vec3::from_array(location)).to_array(),
            slot,
            volume,
            no_override,
            radius,
            pitch,
        )?,
        ActorAction::ModifySound {
            actor,
            slot,
            parameter,
            value,
        } => {
            audio.modify_sound(actor, slot, parameter, value);
        }
        ActorAction::StopSound { actor, clip, slot } => {
            audio.stop_sound(actor, clip.as_ref(), slot)
        }
        _ => {}
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
        pitch: rotation[0],
        roll: -rotation[2],
        vertical_fov: horizontal_to_vertical_fov(
            view.fov_degrees.to_radians(),
            aspect.min(4.0 / 3.0),
        ),
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

fn map_identifier(map: &Path, game_root: &Path) -> Result<String> {
    let identifier = map
        .strip_prefix(game_root)
        .context("map path is outside the game root")?;
    if !identifier
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
        || !identifier
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("unr"))
    {
        anyhow::bail!("map path `{}` is not a relative .unr map", map.display());
    }
    Ok(identifier.to_string_lossy().to_string())
}

fn write_save_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("save file has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("save file has no valid name")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), nonce));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("failed to create {}", temporary.display()))?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to write {}", temporary.display()));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to replace {}", path.display()));
    }
    Ok(())
}

struct ScreenshotReadback {
    path: PathBuf,
    buffer: wgpu::Buffer,
    bytes_per_row: u32,
    width: u32,
    height: u32,
    bgra: bool,
}

fn prepare_screenshots(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    directory: &Path,
    snapshots: Vec<Option<u32>>,
) -> Result<Vec<ScreenshotReadback>> {
    let bgra = match format.remove_srgb_suffix() {
        wgpu::TextureFormat::Bgra8Unorm => true,
        wgpu::TextureFormat::Rgba8Unorm => false,
        format => anyhow::bail!("surface format {format:?} cannot be saved as a BMP"),
    };
    let row_bytes = width.checked_mul(4).context("screenshot row is too wide")?;
    let bytes_per_row = (row_bytes + 255) & !255;
    let size = u64::from(bytes_per_row)
        .checked_mul(u64::from(height))
        .context("screenshot is too large")?;
    snapshots
        .into_iter()
        .map(|snapshot| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("OpenHP1 screenshot readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            Ok(ScreenshotReadback {
                path: next_screenshot_path(directory, snapshot)?,
                buffer,
                bytes_per_row,
                width,
                height,
                bgra,
            })
        })
        .collect()
}

fn next_screenshot_path(directory: &Path, snapshot: Option<u32>) -> Result<PathBuf> {
    fs::create_dir_all(directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let prefix = snapshot.map_or_else(|| "Shot".to_owned(), |value| format!("Snap{value}"));
    for index in 0..=u16::MAX {
        let path = directory.join(format!("{prefix}{index:04}.bmp"));
        if !path.exists() {
            return Ok(path);
        }
    }
    anyhow::bail!("all screenshot names for {prefix} are in use")
}

fn write_screenshot(device: &wgpu::Device, screenshot: ScreenshotReadback) -> Result<()> {
    let slice = screenshot.buffer.slice(..);
    let (sender, receiver) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("GPU readback failed")?;
    receiver
        .recv()
        .context("GPU screenshot readback was cancelled")?
        .context("GPU screenshot readback failed")?;
    let mapped = slice.get_mapped_range();
    let bmp = bmp_bytes(
        &mapped,
        screenshot.bytes_per_row,
        screenshot.width,
        screenshot.height,
        screenshot.bgra,
    )?;
    drop(mapped);
    screenshot.buffer.unmap();
    let temporary = screenshot.path.with_extension("tmp");
    fs::write(&temporary, bmp)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    fs::rename(&temporary, &screenshot.path)
        .with_context(|| format!("could not finalize {}", screenshot.path.display()))?;
    Ok(())
}

fn bmp_bytes(
    pixels: &[u8],
    bytes_per_row: u32,
    width: u32,
    height: u32,
    bgra: bool,
) -> Result<Vec<u8>> {
    let row_bytes = usize::try_from(width.checked_mul(4).context("screenshot row is too wide")?)?;
    let bytes_per_row = usize::try_from(bytes_per_row)?;
    let height = usize::try_from(height)?;
    if bytes_per_row < row_bytes
        || pixels.len()
            < bytes_per_row
                .checked_mul(height)
                .context("screenshot is too large")?
    {
        anyhow::bail!("screenshot readback has an invalid row layout");
    }
    let image_size = row_bytes
        .checked_mul(height)
        .context("screenshot is too large")?;
    let file_size = 54_usize
        .checked_add(image_size)
        .context("screenshot is too large")?;
    let width = i32::try_from(width)?;
    let height = i32::try_from(height)?;
    let mut output = Vec::with_capacity(file_size);
    output.extend(b"BM");
    output.extend(u32::try_from(file_size)?.to_le_bytes());
    output.extend([0; 4]);
    output.extend(54_u32.to_le_bytes());
    output.extend(40_u32.to_le_bytes());
    output.extend(width.to_le_bytes());
    output.extend((-height).to_le_bytes());
    output.extend(1_u16.to_le_bytes());
    output.extend(32_u16.to_le_bytes());
    output.extend(0_u32.to_le_bytes());
    output.extend(u32::try_from(image_size)?.to_le_bytes());
    output.extend([0; 16]);
    for source in pixels.chunks(bytes_per_row).take(height as usize) {
        for pixel in source[..row_bytes].chunks_exact(4) {
            let [red, green, blue, alpha] = if bgra {
                [pixel[2], pixel[1], pixel[0], pixel[3]]
            } else {
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            };
            output.extend([blue, green, red, alpha]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraw_deadline_caps_frames_at_sixty_hertz_without_sleeping_after_overruns() {
        let started = Instant::now();
        assert_eq!(
            next_redraw_deadline(started, started + Duration::from_millis(1)),
            started + FRAME_INTERVAL
        );
        assert_eq!(
            next_redraw_deadline(started, started + Duration::from_millis(20)),
            started + Duration::from_millis(20)
        );
    }

    #[test]
    fn preserves_the_authored_vertical_view_on_widescreen() {
        let camera = camera_from_player_view(
            PlayerView {
                actor: 7,
                location: [10.0, 20.0, 30.0],
                rotation: [8_192, 16_384, -8_192],
                fov_degrees: 90.0,
                flash_fog: [0.0, 0.0, 0.0, 1.0],
            },
            PhysicalSize::new(1600, 900),
            10_000.0,
        );
        assert_eq!(camera.position, Vec3::new(20.0, 30.0, -10.0));
        assert!((camera.yaw - TAU * 0.25).abs() < 0.000_001);
        assert!((camera.pitch - TAU * 0.125).abs() < 0.000_001);
        assert!((camera.roll - TAU * 0.125).abs() < 0.000_001);
        assert!((camera.vertical_fov - 2.0 * (1.0_f32 / (4.0 / 3.0)).atan()).abs() < 0.000_001);
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
        assert_eq!(player.strafe, 6_000.0);
        assert!((player.mouse_x - 192.0).abs() < 1e-4);
        assert!((player.mouse_y - 96.0).abs() < 1e-4);
        assert!(!player.alt_fire);
        assert!(!player.alt_fire_pressed);
        assert!(player.space_pressed);
        assert!(player.jump);
        assert!(player.broom_pitch_up);
        assert!(!player.broom_pitch_down);
        assert!(!player.broom_boost);
        assert!(!player.broom_brake);
        assert!(!input.player_input(1.0 / 60.0).space_pressed);
        input.set_key(KeyCode::Space, ElementState::Released);
        let player = input.player_input(1.0 / 60.0);
        assert!(player.space_released);
        assert!(!input.player_input(1.0 / 60.0).space_released);

        input.set_key(KeyCode::KeyW, ElementState::Released);
        input.set_key(KeyCode::KeyS, ElementState::Pressed);
        let player = input.player_input(1.0 / 60.0);
        assert!(!player.broom_pitch_up);
        assert!(player.broom_pitch_down);
        input.set_key(KeyCode::ArrowDown, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).broom_pitch_down);
        input.set_key(KeyCode::KeyS, ElementState::Released);
        input.set_key(KeyCode::ArrowDown, ElementState::Released);
        input.set_key(KeyCode::ArrowUp, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).broom_pitch_up);

        input.set_mouse_button(MouseButton::Left, ElementState::Pressed);
        input.mouse_delta = (2.0, -1.0);
        let player = input.player_input(1.0 / 60.0);
        assert_eq!(player.base_x, 3_000.0);
        assert_eq!(player.base_y, 6_000.0);
        assert!((player.mouse_x - 192.0).abs() < 1e-4);
        assert!((player.mouse_y - 96.0).abs() < 1e-4);
        assert!(player.alt_fire);
        assert!(player.alt_fire_pressed);
        assert!(!player.alt_fire_released);
        assert!(!player.broom_brake);
        assert!(!input.player_input(1.0 / 60.0).alt_fire_pressed);

        input.set_mouse_button(MouseButton::Left, ElementState::Released);
        let player = input.player_input(1.0 / 60.0);
        assert!(!player.alt_fire);
        assert!(player.alt_fire_released);
        assert!(!player.broom_brake);
        input.set_mouse_button(MouseButton::Right, ElementState::Pressed);
        let player = input.player_input(1.0 / 60.0);
        assert!(player.jump);
        assert!(!player.broom_boost);
        let player = input.player_input(1.0 / 60.0);
        assert!(!player.jump);
        assert!(!player.broom_boost);
        input.set_mouse_button(MouseButton::Right, ElementState::Released);
        assert!(!input.player_input(1.0 / 60.0).broom_boost);
        input.set_key(KeyCode::AltLeft, ElementState::Pressed);
        let player = input.player_input(1.0 / 60.0);
        assert!(player.alt_fire);
        assert!(player.alt_fire_pressed);
        assert!(!player.broom_brake);
        input.set_key(KeyCode::KeyZ, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).broom_boost);
        input.set_key(KeyCode::KeyZ, ElementState::Released);
        input.set_key(KeyCode::KeyX, ElementState::Pressed);
        assert!(input.player_input(1.0 / 60.0).broom_brake);
    }

    #[test]
    fn console_toggle_accepts_physical_and_logical_backquote_keys() {
        assert!(is_console_toggle_key(
            PhysicalKey::Code(KeyCode::Backquote),
            &Key::Character("x".into()),
        ));
        assert!(is_console_toggle_key(
            PhysicalKey::Code(KeyCode::KeyA),
            &Key::Character("`".into()),
        ));
        assert!(is_console_toggle_key(
            PhysicalKey::Code(KeyCode::KeyA),
            &Key::Character("~".into()),
        ));
        assert!(!is_console_toggle_key(
            PhysicalKey::Code(KeyCode::KeyA),
            &Key::Character("a".into()),
        ));
    }

    #[test]
    fn fly_camera_uses_viewer_controls() {
        let mut camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 10_000.0);
        let mut input = InputState::default();
        input.set_key(KeyCode::KeyW, ElementState::Pressed);
        input.set_key(KeyCode::KeyE, ElementState::Pressed);
        input.set_key(KeyCode::ShiftLeft, ElementState::Pressed);
        input.mouse_delta = (2.0, -1.0);

        update_fly_camera(&mut camera, &input, 0.5, 200.0);

        assert!((camera.yaw - 0.008).abs() < 0.000_001);
        assert!((camera.pitch - 0.004).abs() < 0.000_001);
        let expected = (camera.forward() + Vec3::Y) * (400.0 / 2.0_f32.sqrt());
        assert!((camera.position - expected).length() < 0.001);
    }

    #[test]
    fn mouse_axes_are_frame_rate_independent_rates() {
        assert!((mouse_axis(2.0, 1.0 / 60.0, 6.0) - mouse_axis(4.0, 1.0 / 30.0, 6.0)).abs() < 1e-5);
    }

    #[test]
    fn fast_forward_repeats_held_but_not_transient_input() {
        let repeated = repeated_player_input(PlayerInput {
            base_y: 6_000.0,
            mouse_x: 76.8,
            alt_fire: true,
            alt_fire_pressed: true,
            space_pressed: true,
            space_released: true,
            jump: true,
            broom_boost: true,
            broom_brake: true,
            ..PlayerInput::default()
        });
        assert_eq!(repeated.base_y, 6_000.0);
        assert!(repeated.alt_fire);
        assert!(repeated.broom_boost);
        assert!(repeated.broom_brake);
        assert!(!repeated.alt_fire_pressed);
        assert!(!repeated.space_pressed);
        assert!(!repeated.space_released);
        assert_eq!(repeated.mouse_x, 0.0);
        assert!(!repeated.jump);

        assert!(is_fast_forward_key(KeyCode::Equal));
        assert!(is_fast_forward_key(KeyCode::NumpadAdd));
        assert!(is_fast_forward_key(KeyCode::KeyF));
        assert!(!is_fast_forward_key(KeyCode::F2));
    }

    #[test]
    fn selected_slot_bridge_keeps_direct_level_slot_99_as_the_fallback() {
        assert_eq!(active_save_slot(99, Some(3)), 3);
        assert_eq!(active_save_slot(99, None), 99);
        assert_eq!(active_save_slot(2, Some(3)), 2);
    }

    #[test]
    #[ignore = "requires the local original-game corpus"]
    fn saved_quidditch_runtime_reconstructs_hoop_particle_owners() {
        let game_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res");
        let map = game_root.join("Maps/Lev_Tut2.unr");
        let mut source_scene = LoadedScene::load(map.clone()).unwrap();
        let source_console = ConsoleCommands::headless(&game_root).unwrap();
        let (source, _) =
            initialize_saved_runtime(&mut source_scene, source_console, true).unwrap();
        let snapshot = source.save_game("Maps/Lev_Tut2.unr").unwrap();

        let mut scene = LoadedScene::load(map).unwrap();
        let console = ConsoleCommands::headless(&game_root).unwrap();
        let (mut runtime, _) = initialize_saved_runtime(&mut scene, console, true).unwrap();
        let actions = runtime
            .restore_game("Maps/Lev_Tut2.unr", &snapshot)
            .unwrap();
        apply_runtime_actions_with(&mut scene, &mut runtime, actions, |_| Ok(())).unwrap();

        let hoop_actors = scene
            .actors
            .iter()
            .enumerate()
            .filter(|(_, actor)| actor.name.to_ascii_lowercase().starts_with("broomhoop"))
            .map(|(actor, _)| actor)
            .collect::<HashSet<_>>();
        let emitters = runtime.particle_emitters().unwrap();
        let hoop_emitters = emitters
            .iter()
            .filter(|emitter| {
                emitter
                    .owner
                    .is_some_and(|owner| hoop_actors.contains(&owner))
            })
            .collect::<Vec<_>>();
        assert_eq!(hoop_emitters.len(), hoop_actors.len());
        assert_eq!(
            hoop_emitters.iter().filter(|emitter| emitter.emit).count(),
            3
        );
    }

    #[test]
    fn bmp_readback_uses_top_down_bgra_pixels_and_numbered_names() {
        let mut readback = vec![0; 256];
        readback[..8].copy_from_slice(&[1, 2, 3, 4, 20, 30, 40, 50]);
        let bmp = bmp_bytes(&readback, 256, 2, 1, false).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), 62);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -1);
        assert_eq!(&bmp[54..62], &[3, 2, 1, 4, 40, 30, 20, 50]);

        let directory = std::env::temp_dir().join(format!(
            "openhp1-screenshot-name-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let first = next_screenshot_path(&directory, Some(3)).unwrap();
        assert_eq!(first.file_name().unwrap(), "Snap30000.bmp");
        fs::write(&first, []).unwrap();
        assert_eq!(
            next_screenshot_path(&directory, Some(3))
                .unwrap()
                .file_name()
                .unwrap(),
            "Snap30001.bmp"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
