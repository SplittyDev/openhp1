mod app;
mod scene;
mod target;

use std::{env, path::PathBuf};

use anyhow::{Result, anyhow};
use eframe::egui;
use tracing_subscriber::EnvFilter;

use crate::{app::ViewerApp, scene::LoadedScene};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res/Maps/Quid_RavenA.unr"));
    let scene = LoadedScene::load(path)?;
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
