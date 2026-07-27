use std::sync::Arc;

use anyhow::{Context, Result};
use glam::Vec3;
use openhp1_render::{Camera, Renderer};
use openhp1_scene::LoadedScene;
use tracing::error;
use wgpu::{CurrentSurfaceTexture, SurfaceConfiguration};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalSize, Size},
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes, WindowId},
};

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
            .with_inner_size(Size::Physical(PhysicalSize::new(1280, 960)));
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => graphics.resize(size),
            WindowEvent::RedrawRequested => {
                graphics.render();
                graphics.window.request_redraw();
            }
            _ => {}
        }
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
}

impl Graphics {
    fn new(window: Arc<Window>, scene: LoadedScene) -> Result<Self> {
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
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);
        let renderer = Renderer::new(
            &device,
            &queue,
            config.format,
            &scene.render,
            [size.width, size.height],
        );
        let bounds = renderer.bounds();
        let center = bounds.center();
        let camera = Camera::looking_at(
            center,
            center - Vec3::Z,
            (bounds.radius().max(100.0) * 10.0).max(10_000.0),
        );
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            camera,
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

    fn render(&mut self) {
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
                error!("wgpu rejected the game surface");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("OpenHP1 game frame"),
            });
        self.renderer.render(
            &self.queue,
            &mut encoder,
            &view,
            &self.camera,
            [self.config.width, self.config.height],
            0.625,
        );
        self.queue.submit([encoder.finish()]);
        frame.present();
    }
}

fn nonzero_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}
