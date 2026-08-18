mod app;
mod target;

use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Result, anyhow, bail};
use eframe::{
    egui,
    egui_wgpu::{SurfaceConfig, WgpuConfiguration},
    wgpu::PresentMode,
};
use openhp1_render::RendererSettings;
use tracing_subscriber::EnvFilter;

use crate::app::ViewerApp;
use openhp1_scene::LoadedScene;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let command = options_from(env::args_os().skip(1))?;
    let scene = LoadedScene::load(command.path)?;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: WgpuConfiguration::default().with_surface_config(SurfaceConfig {
            present_mode: if command.no_vsync {
                PresentMode::AutoNoVsync
            } else {
                PresentMode::AutoVsync
            },
            desired_maximum_frame_latency: Some(2),
        }),
        ..Default::default()
    };
    eframe::run_native(
        "OpenHP1 map viewer",
        native_options,
        Box::new(move |context| Ok(Box::new(ViewerApp::new(context, scene, command.renderer)?))),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

struct Options {
    path: PathBuf,
    renderer: RendererSettings,
    no_vsync: bool,
}

fn options_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Options> {
    let mut options = Options {
        path: PathBuf::from("res/Maps/Quid_RavenA.unr"),
        renderer: RendererSettings::default(),
        no_vsync: false,
    };
    let mut has_path = false;
    for argument in arguments {
        let Some(argument) = argument.to_str() else {
            bail!("viewer arguments must be valid UTF-8");
        };
        if let Some(value) = argument.strip_prefix("--renderer=") {
            options.renderer.mode = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--tone-mapper=") {
            options.renderer.tone_mapper = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--ambient-occlusion=") {
            options.renderer.ambient_occlusion = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--anti-aliasing=") {
            options.renderer.antialiasing = value.parse()?;
        } else if argument == "--crt" {
            options.renderer.crt_effect = true;
        } else if argument == "--no-vsync" {
            options.no_vsync = true;
        } else if argument.starts_with('-') || has_path {
            bail!(
                "usage: openhp1-viewer [map path] [--renderer=classic|modern] \
                 [--tone-mapper=agx|reinhard|aces] [--ambient-occlusion=off|ssao|xegtao] \
                 [--anti-aliasing=off|fxaa|smaa] [--crt] [--no-vsync]"
            );
        } else {
            options.path = PathBuf::from(argument);
            has_path = true;
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhp1_render::{
        AmbientOcclusion, Antialiasing, RendererMode, RendererSettings, ToneMapper,
    };

    #[test]
    fn parses_map_and_modern_renderer_options() {
        let defaults = options_from([]).unwrap();
        assert_eq!(defaults.path, PathBuf::from("res/Maps/Quid_RavenA.unr"));
        assert_eq!(defaults.renderer, RendererSettings::default());
        assert!(!defaults.no_vsync);

        let crt = options_from([
            OsString::from("res/Maps/Lev5_Chess.unr"),
            OsString::from("--crt"),
        ])
        .unwrap();
        assert!(crt.renderer.crt_effect);

        let options = options_from([
            OsString::from("res/Maps/Lev5_Chess.unr"),
            OsString::from("--renderer=modern"),
            OsString::from("--tone-mapper=reinhard"),
            OsString::from("--ambient-occlusion=xegtao"),
            OsString::from("--anti-aliasing=smaa"),
            OsString::from("--no-vsync"),
        ])
        .unwrap();
        assert_eq!(options.path, PathBuf::from("res/Maps/Lev5_Chess.unr"));
        assert_eq!(options.renderer.mode, RendererMode::Modern);
        assert_eq!(options.renderer.tone_mapper, ToneMapper::Reinhard);
        assert_eq!(options.renderer.ambient_occlusion, AmbientOcclusion::XeGtao);
        assert_eq!(options.renderer.antialiasing, Antialiasing::Smaa);
        assert!(options.no_vsync);
    }
}
