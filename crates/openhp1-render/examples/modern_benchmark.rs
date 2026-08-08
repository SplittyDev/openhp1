use std::{
    env,
    error::Error,
    hash::{DefaultHasher, Hasher},
    sync::mpsc,
    time::Instant,
};

use glam::Vec3;
use openhp1_render::{Camera, DisplaySettings, Renderer, RendererMode, RendererSettings};
use openhp1_scene::LoadedScene;

const SIZE: [u32; 2] = [1024, 768];
const WARMUP_FRAMES: usize = 10;
const MEASURED_FRAMES: usize = 60;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .ok_or("usage: modern_benchmark <map path>")?;
    let scene = LoadedScene::load(path.into())?;
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("OpenHP1 Modern benchmark device"),
        ..Default::default()
    }))?;
    let settings = RendererSettings {
        mode: RendererMode::Modern,
        ..Default::default()
    };
    let mut renderer = Renderer::new_with_settings(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        &scene.render,
        SIZE,
        settings,
    );
    let bounds = renderer.bounds();
    let radius = bounds.radius().max(100.0);
    let center = bounds.center();
    let camera = Camera::looking_at(center, center - Vec3::Z, (radius * 10.0).max(10_000.0));
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 Modern benchmark output"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let output_view = output.create_view(&Default::default());

    for _ in 0..WARMUP_FRAMES {
        render_frame(&device, &queue, &mut renderer, &output_view, &camera);
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let start = Instant::now();
    let mut stats = Default::default();
    for _ in 0..MEASURED_FRAMES {
        stats = render_frame(&device, &queue, &mut renderer, &output_view, &camera);
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let elapsed = start.elapsed();
    let checksum = readback_checksum(&device, &queue, &output)?;
    println!(
        "adapter={} frames={} total_ms={:.3} ms_per_frame={:.3} draw_calls={} checksum={checksum:016x}",
        adapter.get_info().name,
        MEASURED_FRAMES,
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000.0 / MEASURED_FRAMES as f64,
        stats.draw_calls,
    );
    Ok(())
}

fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    output: &wgpu::TextureView,
    camera: &Camera,
) -> openhp1_render::RenderStats {
    renderer.advance_time(1.0 / 60.0);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("OpenHP1 Modern benchmark frame"),
    });
    let stats = renderer.render(
        queue,
        &mut encoder,
        output,
        camera,
        SIZE,
        DisplaySettings::for_mode(RendererMode::Modern),
    );
    queue.submit([encoder.finish()]);
    stats
}

fn readback_checksum(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
) -> Result<u64, Box<dyn Error>> {
    let size = u64::from(SIZE[0]) * u64::from(SIZE[1]) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("OpenHP1 Modern benchmark readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("OpenHP1 Modern benchmark readback"),
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE[0] * 4),
                rows_per_image: Some(SIZE[1]),
            },
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;
    let data = buffer.slice(..).get_mapped_range();
    let mut hasher = DefaultHasher::new();
    hasher.write(&data);
    Ok(hasher.finish())
}
