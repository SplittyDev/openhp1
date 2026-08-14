use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result, anyhow, bail};
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use openhp1_package::{GameInstallation, configure_game_installation, resolve_game_installation};

const SPLASH: &[u8] = include_bytes!("../../../splash.jpg");

fn main() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([724.0, 690.0])
            .with_resizable(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "OpenHP1",
        options,
        Box::new(|context| Ok(Box::new(Launcher::new(context)?))),
    )
    .map_err(|error| anyhow!(error.to_string()))
}

struct Launcher {
    splash: TextureHandle,
    installation: Option<GameInstallation>,
    status: String,
    status_color: Color32,
}

impl Launcher {
    fn new(context: &eframe::CreationContext<'_>) -> Result<Self> {
        let image = image::load_from_memory(SPLASH)
            .context("failed to decode launcher splash image")?
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let splash = context.egui_ctx.load_texture(
            "OpenHP1 launcher splash",
            egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        let (installation, status, status_color) = match resolve_game_installation() {
            Ok(installation) => {
                let status = format!("Game files: {}", installation.root().display());
                (Some(installation), status, Color32::from_rgb(150, 210, 150))
            }
            Err(error) => (None, error.to_string(), Color32::from_rgb(235, 185, 110)),
        };
        Ok(Self {
            splash,
            installation,
            status,
            status_color,
        })
    }

    fn configure(&mut self) {
        let Some(root) = rfd::FileDialog::new()
            .set_title("Choose the Harry Potter game folder")
            .pick_folder()
        else {
            return;
        };
        match configure_game_installation(&root, None) {
            Ok(installation) => self.set_installation(installation),
            Err(error) => self.set_error(error),
        }
    }

    fn select_language(&mut self, root: &PathBuf, language: &str) {
        match configure_game_installation(root, Some(language)) {
            Ok(installation) => self.set_installation(installation),
            Err(error) => self.set_error(error),
        }
    }

    fn set_installation(&mut self, installation: GameInstallation) {
        self.status = format!("Game files: {}", installation.root().display());
        self.status_color = Color32::from_rgb(150, 210, 150);
        self.installation = Some(installation);
    }

    fn set_error(&mut self, error: impl std::fmt::Display) {
        self.status = error.to_string();
        self.status_color = Color32::from_rgb(240, 135, 120);
    }

    fn language_selector(&mut self, ui: &mut egui::Ui) {
        let Some(installation) = &self.installation else {
            return;
        };
        let root = installation.root().to_path_buf();
        let mut selected = installation.language().to_owned();
        let languages = installation.available_languages().to_vec();
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 270.0) / 2.0).max(0.0));
            ui.label("Language");
            egui::ComboBox::from_id_salt("game-language")
                .width(190.0)
                .selected_text(language_label(&selected))
                .show_ui(ui, |ui| {
                    for language in languages {
                        changed |= ui
                            .selectable_value(
                                &mut selected,
                                language.clone(),
                                language_label(&language),
                            )
                            .changed();
                    }
                });
        });
        if changed {
            self.select_language(&root, &selected);
        }
    }

    fn play(&mut self, context: &egui::Context) {
        match launch_game() {
            Ok(()) => context.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => {
                self.status = error.to_string();
                self.status_color = Color32::from_rgb(240, 135, 120);
            }
        }
    }
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(3, 3, 12));
        egui::Frame::NONE.show(ui, |ui| {
            let width = ui.available_width();
            ui.add(
                egui::Image::new(&self.splash)
                    .fit_to_exact_size(Vec2::new(width, width * 543.0 / 724.0)),
            );
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(&self.status).color(self.status_color));
                ui.add_space(8.0);
                self.language_selector(ui);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 456.0) / 2.0);
                    let button_size = Vec2::new(144.0, 42.0);
                    if ui
                        .add_enabled(
                            self.installation.is_some(),
                            egui::Button::new("Play").min_size(button_size),
                        )
                        .clicked()
                    {
                        self.play(&context);
                    }
                    if ui
                        .add(egui::Button::new("Configure").min_size(button_size))
                        .clicked()
                    {
                        self.configure();
                    }
                    if ui
                        .add(egui::Button::new("Exit").min_size(button_size))
                        .clicked()
                    {
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });
    }
}

fn launch_game() -> Result<()> {
    let executable = game_executable()?;
    Command::new(&executable)
        .spawn()
        .with_context(|| format!("failed to launch {}", executable.display()))?;
    Ok(())
}

fn game_executable() -> Result<PathBuf> {
    let launcher = std::env::current_exe().context("failed to locate the OpenHP1 launcher")?;
    let directory = launcher
        .parent()
        .context("the OpenHP1 launcher has no parent directory")?;
    let executable = directory.join(if cfg!(target_os = "windows") {
        "openhp1-game.exe"
    } else {
        "openhp1-game"
    });
    if !executable.is_file() {
        bail!("could not find {}", executable.display());
    }
    Ok(executable)
}

fn language_label(language: &str) -> String {
    let name = match language.to_ascii_lowercase().as_str() {
        "int" | "eng" => Some("English"),
        "fre" => Some("French"),
        "ger" => Some("German"),
        "spa" => Some("Spanish"),
        "ita" => Some("Italian"),
        "dut" => Some("Dutch"),
        "por" => Some("Portuguese"),
        "pol" => Some("Polish"),
        "rus" => Some("Russian"),
        "hun" => Some("Hungarian"),
        "cze" => Some("Czech"),
        _ => None,
    };
    name.map_or_else(
        || language.to_ascii_uppercase(),
        |name| format!("{name} ({language})"),
    )
}

#[cfg(test)]
mod tests {
    use super::language_label;

    #[test]
    fn labels_known_and_unknown_game_languages() {
        assert_eq!(language_label("fre"), "French (fre)");
        assert_eq!(language_label("xyz"), "XYZ");
    }
}
