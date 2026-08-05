use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Sense, TextureHandle, Vec2};
use openhp1_package::{ObjectReference, PackageStore, ResolvedObject};
use openhp1_texture::{Palette, Texture};

const REFERENCE_SIZE: Vec2 = Vec2::new(640.0, 480.0);

pub(super) enum Action {
    Exit,
    LoadSave(u32),
    NewGame(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Main,
    Slots,
    Options,
    Quidditch,
}

struct UiTextures {
    main_background: Vec<TextureHandle>,
    logo: Vec<TextureHandle>,
    save_background: Vec<TextureHandle>,
    empty_slot: TextureHandle,
    back: TextureHandle,
    back_hover: TextureHandle,
}

pub(super) struct GameUi {
    open: bool,
    page: Page,
    confirm_exit: bool,
    confirm_replace: bool,
    selected_slot: Option<usize>,
    action: Option<Action>,
    save_slots: [bool; 6],
    labels: Labels,
    textures: UiTextures,
}

struct Labels {
    start: String,
    options: String,
    quidditch: String,
    exit: String,
    select_game: String,
    new_game: String,
    load_game: String,
    replace_game: String,
    confirm_replace: String,
    back: String,
    confirm_exit: String,
    yes: String,
    no: String,
}

impl GameUi {
    pub(super) fn load(
        context: &egui::Context,
        game_root: &Path,
        map: &Path,
        save_dir: &Path,
    ) -> Result<Self> {
        let mut packages = PackageStore::scan_game_root(game_root)?;
        let textures = UiTextures {
            main_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("MenuArt.MoonTitle{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            save_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FESaveBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            empty_slot: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.SaveSlotEmptyTexture",
                true,
            )?,
            back: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftReturnUpIcon",
                true,
            )?,
            back_hover: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftReturnOverIcon",
                true,
            )?,
            logo: (1..=2)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("MenuArt.Logo{index}"),
                        true,
                    )
                })
                .collect::<Result<_>>()?,
        };
        let localized = |key: &str| -> Result<String> {
            let value = packages.localize("HPMenu", "text", key);
            if value.is_empty() {
                bail!("HPMenu.int is missing [text] {key}");
            }
            Ok(value)
        };
        let labels = Labels {
            start: localized("main_menu_03")?,
            options: localized("main_menu_04")?,
            quidditch: localized("main_menu_05")?,
            exit: localized("main_menu_06")?,
            select_game: localized("select_game_01")?,
            new_game: localized("select_game_02")?,
            load_game: localized("select_game_03")?,
            replace_game: localized("select_game_04")?,
            confirm_replace: localized("select_game_05")?,
            back: localized("back_button")?,
            confirm_exit: localized("main_menu_08")?,
            yes: localized("main_menu_09")?,
            no: localized("main_menu_10")?,
        };
        let save_slots = std::array::from_fn(|slot| {
            fs::metadata(save_dir.join(format!("save{slot}.usa")))
                .is_ok_and(|metadata| metadata.is_file())
        });
        Ok(Self {
            open: is_startup_map(map),
            page: Page::Main,
            confirm_exit: false,
            confirm_replace: false,
            selected_slot: None,
            action: None,
            save_slots,
            labels,
            textures,
        })
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    pub(super) fn ui(&mut self, context: &egui::Context) {
        if !self.open {
            return;
        }
        let screen = context.content_rect();
        let scale = (screen.width() / REFERENCE_SIZE.x)
            .min(screen.height() / REFERENCE_SIZE.y)
            .max(0.01);
        let canvas = Rect::from_center_size(screen.center(), REFERENCE_SIZE * scale);
        let painter = context.layer_painter(LayerId::new(Order::Background, Id::new("game ui")));
        painter.rect_filled(screen, 0.0, Color32::BLACK);
        let painter = painter.with_clip_rect(canvas);
        let background = if self.page == Page::Slots {
            &self.textures.save_background
        } else {
            &self.textures.main_background
        };
        for (index, texture) in background.iter().enumerate() {
            let x = (index % 3) as f32 * 256.0;
            let y = (index / 3) as f32 * 256.0;
            draw_texture(&painter, canvas.min, scale, texture, Pos2::new(x, y));
        }
        if self.page == Page::Main {
            for (index, texture) in self.textures.logo.iter().enumerate() {
                draw_texture(
                    &painter,
                    canvas.min,
                    scale,
                    texture,
                    Pos2::new(74.0 + index as f32 * 256.0, 243.0),
                );
            }
        }

        egui::Area::new(Id::new("game ui controls"))
            .fixed_pos(canvas.min)
            .order(Order::Middle)
            .show(context, |ui| {
                ui.set_min_size(canvas.size());
                match self.page {
                    Page::Main => self.main_page(ui, scale),
                    Page::Slots => self.slot_page(ui, scale),
                    Page::Options => self.placeholder_page(ui, scale, self.labels.options.clone()),
                    Page::Quidditch => {
                        self.placeholder_page(ui, scale, self.labels.quidditch.clone())
                    }
                }
                if self.confirm_exit {
                    self.exit_confirmation(ui, scale);
                }
                if self.confirm_replace {
                    self.replace_confirmation(ui, scale);
                }
            });
    }

    fn main_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        let choices = [
            (self.labels.start.clone(), Page::Slots),
            (self.labels.options.clone(), Page::Options),
            (self.labels.quidditch.clone(), Page::Quidditch),
        ];
        for (index, (label, page)) in choices.into_iter().enumerate() {
            if menu_button(ui, scale, 265.0, 360.0 + index as f32 * 22.0, &label) {
                self.page = page;
            }
        }
        if menu_button(ui, scale, 265.0, 426.0, &self.labels.exit) {
            self.confirm_exit = true;
        }
    }

    fn slot_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        page_title(ui, scale, 30.0, &self.labels.select_game, Color32::MAGENTA);
        for slot in 0..6 {
            let row = slot / 3;
            let column = slot % 3;
            let kind = if self.save_slots[slot] {
                &self.labels.load_game
            } else {
                &self.labels.new_game
            };
            let x = 78.0 + column as f32 * 174.0;
            let y = 90.0 + row as f32 * 174.0;
            ui.painter().text(
                ui.min_rect().min + Vec2::new(x, y - 24.0) * scale,
                Align2::LEFT_TOP,
                (slot + 1).to_string(),
                FontId::proportional(16.0 * scale),
                Color32::WHITE,
            );
            if textured_button(
                ui,
                scale,
                x,
                y,
                &self.textures.empty_slot,
                &self.textures.empty_slot,
                kind,
            ) {
                if self.save_slots[slot] {
                    self.selected_slot = Some(slot);
                } else {
                    self.action = Some(Action::NewGame(slot as u32));
                }
            }
        }
        if let Some(slot) = self.selected_slot {
            if menu_button(ui, scale, 170.0, 408.0, &self.labels.load_game) {
                self.action = Some(Action::LoadSave(slot as u32));
            }
            if menu_button(ui, scale, 300.0, 408.0, &self.labels.replace_game) {
                self.confirm_replace = true;
            }
        }
        if textured_button(
            ui,
            scale,
            565.0,
            431.0,
            &self.textures.back,
            &self.textures.back_hover,
            "",
        ) {
            self.page = Page::Main;
            self.selected_slot = None;
        }
    }

    fn placeholder_page(&mut self, ui: &mut egui::Ui, scale: f32, heading: String) {
        page_title(ui, scale, 120.0, &heading, Color32::WHITE);
        if menu_button(ui, scale, 265.0, 410.0, &self.labels.back) {
            self.page = Page::Main;
        }
    }

    fn exit_confirmation(&mut self, ui: &mut egui::Ui, scale: f32) {
        confirmation_panel(ui, scale, &self.labels.confirm_exit);
        if menu_button(ui, scale, 205.0, 245.0, &self.labels.yes) {
            self.action = Some(Action::Exit);
        }
        if menu_button(ui, scale, 335.0, 245.0, &self.labels.no) {
            self.confirm_exit = false;
        }
    }

    fn replace_confirmation(&mut self, ui: &mut egui::Ui, scale: f32) {
        confirmation_panel(ui, scale, &self.labels.confirm_replace);
        if menu_button(ui, scale, 205.0, 245.0, &self.labels.yes) {
            if let Some(slot) = self.selected_slot {
                self.action = Some(Action::NewGame(slot as u32));
            }
            self.confirm_replace = false;
        }
        if menu_button(ui, scale, 335.0, 245.0, &self.labels.no) {
            self.confirm_replace = false;
        }
    }
}

fn confirmation_panel(ui: &egui::Ui, scale: f32, text: &str) {
    let origin = ui.min_rect().min;
    let rect = scaled_rect(origin, scale, 140.0, 170.0, 360.0, 130.0);
    ui.painter()
        .rect_filled(rect, 6.0 * scale, Color32::from_black_alpha(225));
    ui.painter().text(
        origin + Vec2::new(320.0, 205.0) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0 * scale),
        Color32::WHITE,
    );
}

fn load_texture(
    context: &egui::Context,
    packages: &mut PackageStore,
    name: &str,
    masked: bool,
) -> Result<TextureHandle> {
    let ResolvedObject {
        package,
        export_index,
    } = packages
        .find_localized_object(name, "Texture")?
        .with_context(|| format!("shipped UI texture {name} is missing"))?;
    let texture = Texture::decode(&package, export_index)?;
    let ObjectReference::Export(palette_index) = texture.palette else {
        bail!("shipped UI texture {name} has a non-local palette");
    };
    let palette = Palette::decode(&package, palette_index)?;
    let mip = texture
        .mips
        .first()
        .context("shipped UI texture has no mip")?;
    let rgba = texture.rgba(0, &palette, masked)?;
    let image =
        egui::ColorImage::from_rgba_unmultiplied([mip.width as usize, mip.height as usize], &rgba);
    Ok(context.load_texture(name, image, egui::TextureOptions::NEAREST))
}

fn draw_texture(
    painter: &egui::Painter,
    origin: Pos2,
    scale: f32,
    texture: &TextureHandle,
    position: Pos2,
) {
    let size = texture.size_vec2() * scale;
    let position = origin + position.to_vec2() * scale;
    painter.image(
        texture.id(),
        Rect::from_min_size(position, size),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn page_title(ui: &egui::Ui, scale: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(320.0, y) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0 * scale),
        color,
    );
}

fn menu_button(ui: &mut egui::Ui, scale: f32, x: f32, y: f32, text: &str) -> bool {
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 140.0, 25.0);
    let response = ui.interact(
        rect,
        Id::new((text, x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(16.0 * scale),
        if response.hovered() {
            Color32::RED
        } else {
            Color32::WHITE
        },
    );
    response.clicked()
}

fn textured_button(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    texture: &TextureHandle,
    hover_texture: &TextureHandle,
    text: &str,
) -> bool {
    let rect = scaled_rect(
        ui.min_rect().min,
        scale,
        x,
        y,
        texture.size_vec2().x,
        texture.size_vec2().y,
    );
    let response = ui.interact(
        rect,
        Id::new(("texture button", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    let texture = if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        hover_texture
    } else {
        texture
    };
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    if !text.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(16.0 * scale),
            if response.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(250, 4, 30)
            },
        );
    }
    response.clicked()
}

fn scaled_rect(origin: Pos2, scale: f32, x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(x, y) * scale,
        Vec2::new(width * scale, height * scale),
    )
}

fn is_startup_map(path: &Path) -> bool {
    path.file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("startup"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_only_opens_for_the_authored_startup_map() {
        assert!(is_startup_map(Path::new("game/Maps/STARTUP.unr")));
        assert!(!is_startup_map(Path::new("game/Maps/Entry.unr")));
        assert!(!is_startup_map(Path::new("game/Maps/Lev_Tut1.unr")));
    }
}
