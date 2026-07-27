mod app;

use std::{env, ffi::OsString, path::PathBuf};

use anyhow::{Result, bail};
use app::GameApp;
use openhp1_scene::LoadedScene;
use tracing_subscriber::EnvFilter;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let scene = LoadedScene::load(level_path()?)?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut GameApp::new(scene))?;
    Ok(())
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
