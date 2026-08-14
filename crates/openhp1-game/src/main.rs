mod app;

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use app::GameApp;
use openhp1_package::{resolve_game_installation, settings_dir};
use openhp1_render::RendererSettings;
use openhp1_scene::LoadedScene;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let log_path = init_logging()?;
    info!(path = %log_path.display(), "logging game diagnostics");

    let options = options()?;
    let level = match options.level {
        Some(level) => level,
        None => resolve_game_installation()?.startup_map().to_path_buf(),
    };
    let scene = LoadedScene::load(level)?;
    let diagnostics = scene
        .actors
        .iter()
        .flat_map(|actor| {
            actor
                .diagnostics
                .iter()
                .map(move |message| (actor, message))
        })
        .collect::<Vec<_>>();
    for (actor, message) in &diagnostics {
        warn!(
            actor = actor.name,
            class = actor.class_name,
            draw_type = actor.draw_type,
            diagnostic = message.as_str(),
            "scene actor capability diagnostic"
        );
    }
    info!(
        actors = scene.actors.len(),
        diagnostics = diagnostics.len(),
        "loaded scene capabilities"
    );
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut GameApp::new(scene, options.renderer))?;
    Ok(())
}

fn init_logging() -> Result<PathBuf> {
    let directory = settings_dir().join("Logs");
    fs::create_dir_all(&directory).context("could not create logs directory")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    let path = directory.join(format!("openhp1-game-{timestamp}.log"));
    let file = File::create(&path)
        .with_context(|| format!("could not create diagnostic log {}", path.display()))?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(EnvFilter::from_default_env()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_filter(EnvFilter::new("info,symphonia_bundle_mp3=off")),
        )
        .try_init()
        .context("could not initialize diagnostic logging")?;
    Ok(path)
}

struct Options {
    level: Option<PathBuf>,
    renderer: Option<RendererSettings>,
}

fn options() -> Result<Options> {
    options_from(env::args_os().skip(1))
}

fn options_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Options> {
    let mut options = Options {
        level: None,
        renderer: None,
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--level" {
            options.level = Some(
                arguments
                    .next()
                    .map(PathBuf::from)
                    .context("--level requires a map path")?,
            );
            continue;
        }
        let argument = argument
            .to_str()
            .context("renderer arguments must be valid UTF-8")?;
        let renderer = options.renderer.get_or_insert_default();
        if let Some(value) = argument.strip_prefix("--renderer=") {
            renderer.mode = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--tone-mapper=") {
            renderer.tone_mapper = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--ambient-occlusion=") {
            renderer.ambient_occlusion = value.parse()?;
        } else if let Some(value) = argument.strip_prefix("--anti-aliasing=") {
            renderer.antialiasing = value.parse()?;
        } else {
            bail!(
                "usage: openhp1-game [--level <map path>] [--renderer=classic|modern] \
                 [--tone-mapper=agx|reinhard|aces] [--ambient-occlusion=off|ssao|xegtao] \
                 [--anti-aliasing=off|fxaa|smaa]"
            );
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhp1_render::{AmbientOcclusion, Antialiasing, RendererMode, ToneMapper};

    #[test]
    fn parses_renderer_and_level_options() {
        let defaults = options_from([]).unwrap();
        assert_eq!(defaults.level, None);
        assert_eq!(defaults.renderer, None);

        let options = options_from([
            OsString::from("--renderer=modern"),
            OsString::from("--tone-mapper=aces"),
            OsString::from("--ambient-occlusion=xegtao"),
            OsString::from("--anti-aliasing=fxaa"),
            OsString::from("--level"),
            OsString::from("/game/Maps/Lev2_HogFront.unr"),
        ])
        .unwrap();
        assert_eq!(
            options.level,
            Some(PathBuf::from("/game/Maps/Lev2_HogFront.unr"))
        );
        let renderer = options.renderer.unwrap();
        assert_eq!(renderer.mode, RendererMode::Modern);
        assert_eq!(renderer.tone_mapper, ToneMapper::Aces);
        assert_eq!(renderer.ambient_occlusion, AmbientOcclusion::XeGtao);
        assert_eq!(renderer.antialiasing, Antialiasing::Fxaa);
    }
}
