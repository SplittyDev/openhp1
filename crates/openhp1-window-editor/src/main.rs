mod app;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use eframe::egui;
use openhp1_package::{PackageStore, resolve_game_installation};

use crate::app::WindowEditor;

fn main() -> Result<()> {
    let installation = resolve_game_installation()?;
    let packages = PackageStore::scan_game_root(installation.root())?;
    let state_path = PathBuf::from("window_editor.state.json");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "OpenHP1 window editor",
        options,
        Box::new(move |context| Ok(Box::new(WindowEditor::new(context, packages, state_path)?))),
    )
    .map_err(|error| anyhow!(error.to_string()))
}
