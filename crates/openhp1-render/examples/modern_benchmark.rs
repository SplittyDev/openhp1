use std::{
    env,
    error::Error,
    hash::{DefaultHasher, Hasher},
    sync::mpsc,
    time::Instant,
};

use glam::Vec3;
use openhp1_render::{
    AmbientOcclusion, Antialiasing, Camera, DisplaySettings, Renderer, RendererMode,
    RendererSettings, unreal_to_render,
};
use openhp1_scene::LoadedScene;

const SIZE: [u32; 2] = [1024, 768];
const WARMUP_FRAMES: usize = 10;
const MEASURED_FRAMES: usize = 60;

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let path = arguments
        .first()
        .ok_or(
            "usage: modern_benchmark <map path> [--baseline] [--classic] [--portal] [--retina] [--updates]",
        )?;
    let portal_view = arguments.iter().any(|value| value == "--portal");
    let baseline = arguments.iter().any(|value| value == "--baseline");
    let updates = arguments.iter().any(|value| value == "--updates");
    let mode = if arguments.iter().any(|value| value == "--classic") {
        RendererMode::Classic
    } else {
        RendererMode::Modern
    };
    let size = if arguments.iter().any(|value| value == "--retina") {
        [1792, 1536]
    } else {
        SIZE
    };
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
    let mut settings = RendererSettings {
        mode,
        ..Default::default()
    };
    if baseline {
        settings.ambient_occlusion = AmbientOcclusion::Off;
        settings.antialiasing = Antialiasing::Off;
        settings.bloom = false;
        settings.volumetric_lighting = false;
    }
    let mut renderer = Renderer::new_with_settings(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        &scene.render,
        size,
        settings,
    );
    let bounds = renderer.bounds();
    let radius = bounds.radius().max(100.0);
    let center = bounds.center();
    let far = (radius * 10.0).max(10_000.0);
    let corona_actors = scene
        .render
        .coronas
        .iter()
        .map(|corona| corona.actor_index)
        .collect::<std::collections::HashSet<_>>();
    let camera = portal_view
        .then(|| portal_camera(&scene.render, far))
        .flatten()
        .or_else(|| local_light_camera(&scene.render, &corona_actors, far))
        .unwrap_or_else(|| Camera::looking_at(center, center - Vec3::Z, far));
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("OpenHP1 Modern benchmark output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
        render_frame(
            &device,
            &queue,
            &mut renderer,
            &output_view,
            &camera,
            size,
            mode,
            updates.then_some(&scene.render),
        );
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let start = Instant::now();
    let mut stats = Default::default();
    for _ in 0..MEASURED_FRAMES {
        stats = render_frame(
            &device,
            &queue,
            &mut renderer,
            &output_view,
            &camera,
            size,
            mode,
            updates.then_some(&scene.render),
        );
    }
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let elapsed = start.elapsed();
    let checksum = readback_checksum(&device, &queue, &output, size)?;
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

fn portal_camera(scene: &openhp1_render::RenderScene, far: f32) -> Option<Camera> {
    let (triangle, _) = scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
        .find(|(_, surface)| {
            scene
                .surface_materials
                .get(**surface)
                .is_some_and(|material| material.volumetric_source)
        })?;
    let [a, b, c] = <[u32; 3]>::try_from(triangle).ok()?;
    let [a, b, c] = [a, b, c].map(|index| {
        scene
            .mesh
            .positions
            .get(index as usize)
            .copied()
            .map(unreal_to_render)
    });
    let [a, b, c] = [a?, b?, c?];
    let center = (a + b + c) / 3.0;
    let normal = (b - a).cross(c - a).normalize_or_zero();
    Some(Camera::looking_at(center - normal * 300.0, center, far))
}

fn local_light_camera(
    scene: &openhp1_render::RenderScene,
    corona_actors: &std::collections::HashSet<usize>,
    far: f32,
) -> Option<Camera> {
    scene
        .realtime_lightmaps
        .iter()
        .flat_map(|lightmap| &lightmap.lights)
        .find(|light| {
            light.actor_index != usize::MAX
                && light.effect != 4
                && (light.source_texture.is_some()
                    || corona_actors.contains(&light.actor_index)
                    || (light.brightness != 0
                        && light.volume_radius != 0
                        && light.volume_brightness != 0))
        })
        .map(|light| {
            Camera::looking_at(
                light.location + Vec3::new(0.0, -250.0, 50.0),
                light.location,
                far,
            )
        })
}

fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    output: &wgpu::TextureView,
    camera: &Camera,
    size: [u32; 2],
    mode: RendererMode,
    scene: Option<&openhp1_render::RenderScene>,
) -> openhp1_render::RenderStats {
    if let Some(scene) = scene {
        assert!(renderer.update_scene(queue, scene));
    }
    renderer.advance_time(1.0 / 60.0);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("OpenHP1 Modern benchmark frame"),
    });
    let stats = renderer.render(
        queue,
        &mut encoder,
        output,
        camera,
        size,
        DisplaySettings::for_mode(mode),
    );
    queue.submit([encoder.finish()]);
    stats
}

fn readback_checksum(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    size: [u32; 2],
) -> Result<u64, Box<dyn Error>> {
    let buffer_size = u64::from(size[0]) * u64::from(size[1]) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("OpenHP1 Modern benchmark readback"),
        size: buffer_size,
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
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
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
