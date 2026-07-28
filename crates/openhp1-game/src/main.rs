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
use openhp1_scene::LoadedScene;
use tracing::info;
use tracing_subscriber::{
    EnvFilter, Layer, filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt,
};
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let log_path = init_logging()?;
    info!(path = %log_path.display(), "logging game diagnostics");

    let scene = LoadedScene::load(level_path()?)?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut GameApp::new(scene))?;
    Ok(())
}

fn init_logging() -> Result<PathBuf> {
    let directory = PathBuf::from("logs");
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
                .with_filter(LevelFilter::INFO),
        )
        .try_init()
        .context("could not initialize diagnostic logging")?;
    Ok(path)
}

fn level_path() -> Result<PathBuf> {
    level_path_from(env::args_os().skip(1))
}

fn level_path_from(arguments: impl IntoIterator<Item = OsString>) -> Result<PathBuf> {
    let mut arguments = arguments.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(PathBuf::from("res/Maps/Lev_Tut1.unr"));
    };
    if argument != "--level" {
        bail!("usage: openhp1-game [--level <map path>]");
    }
    let Some(path) = arguments.next() else {
        bail!("--level requires a map path");
    };
    if arguments.next().is_some() {
        bail!("usage: openhp1-game [--level <map path>]");
    }
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_and_explicit_levels() {
        assert_eq!(
            level_path_from([]).unwrap(),
            PathBuf::from("res/Maps/Lev_Tut1.unr"),
        );
        assert_eq!(
            level_path_from([
                OsString::from("--level"),
                OsString::from("/game/Maps/Lev2_HogFront.unr"),
            ])
            .unwrap(),
            PathBuf::from("/game/Maps/Lev2_HogFront.unr")
        );
    }
}
