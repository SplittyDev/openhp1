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
    SetBrightness(f32),
    SetMusicVolume(u8),
    SetResolution(u32, u32),
    SetSoundVolume(u8),
}

pub(super) struct OptionsState {
    pub(super) resolution: (u32, u32),
    pub(super) resolutions: Vec<(u32, u32)>,
    pub(super) brightness: f32,
    pub(super) music_volume: f32,
    pub(super) sound_volume: f32,
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
    options_background: Vec<TextureHandle>,
    option_bar: TextureHandle,
    slider_track: TextureHandle,
    slider_knob: TextureHandle,
    checkbox_off: TextureHandle,
    checkbox_on: TextureHandle,
}

struct OptionValues {
    resolution: usize,
    resolutions: Vec<(u32, u32)>,
    brightness: f32,
    mouse_speed: f32,
    music_volume: f32,
    sound_volume: f32,
    texture_detail: usize,
    object_detail: usize,
    auto_jump: bool,
    invert_broom: bool,
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
    option_labels: OptionLabels,
    options: OptionValues,
    textures: UiTextures,
}

struct OptionLabels {
    title: String,
    video: String,
    controls: String,
    resolution: String,
    color_depth: String,
    texture_detail: String,
    object_detail: String,
    brightness: String,
    mouse_speed: String,
    low: String,
    high: String,
    audio: String,
    music_volume: String,
    sound_volume: String,
    keys: [String; 8],
    auto_jump: String,
    invert_broom: String,
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
        options: OptionsState,
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
            options_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEOptionsBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            option_bar: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOverOptionTexture",
                true,
            )?,
            slider_track: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOverSliderTexture",
                true,
            )?,
            slider_knob: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FESliderKnobTexture",
                true,
            )?,
            checkbox_off: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOptionTickUncheckedTex",
                true,
            )?,
            checkbox_on: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOptionTickCheckedTex",
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
        let pickup = |key: &str| -> Result<String> {
            let value = packages.localize("Pickup", "all", key);
            if value.is_empty() {
                bail!("Pickup.int is missing [all] {key}");
            }
            Ok(value)
        };
        let option_labels = OptionLabels {
            title: localized("options_01")?,
            video: pickup("videoText")?,
            controls: localized("options_16")?,
            resolution: localized("options_02")?,
            color_depth: localized("options_03")?,
            texture_detail: localized("options_05")?,
            object_detail: localized("options_12")?,
            brightness: localized("options_04")?,
            mouse_speed: localized("options_17")?,
            low: localized("options_10")?,
            high: localized("options_07")?,
            audio: localized("options_13")?,
            music_volume: localized("options_14")?,
            sound_volume: localized("options_15")?,
            keys: [
                localized("options_21")?,
                localized("options_22")?,
                localized("options_23")?,
                localized("options_24")?,
                localized("options_25")?,
                localized("options_26")?,
                localized("flying_02")?,
                localized("flying_03")?,
            ],
            auto_jump: pickup("AutoJumpText")?,
            invert_broom: localized("flying_04")?,
        };
        let mut resolutions = options.resolutions;
        if !resolutions.contains(&options.resolution) {
            resolutions.push(options.resolution);
        }
        resolutions.sort_unstable();
        resolutions.dedup();
        let resolution = resolutions
            .iter()
            .position(|candidate| *candidate == options.resolution)
            .unwrap_or_default();
        let options = OptionValues {
            resolution,
            resolutions,
            brightness: ((options.brightness - 0.2) / 0.8).clamp(0.0, 1.0),
            mouse_speed: 0.5,
            music_volume: options.music_volume,
            sound_volume: options.sound_volume,
            texture_detail: 0,
            object_detail: 0,
            auto_jump: false,
            invert_broom: false,
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
            option_labels,
            options,
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
        let background = match self.page {
            Page::Slots => &self.textures.save_background,
            Page::Options => &self.textures.options_background,
            Page::Main | Page::Quidditch => &self.textures.main_background,
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
                    Page::Options => self.options_page(ui, scale),
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

    fn options_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        const PURPLE: Color32 = Color32::from_rgb(96, 0, 96);
        const BLUE: Color32 = Color32::from_rgb(20, 60, 210);
        page_title(ui, scale, 27.0, &self.option_labels.title, PURPLE);
        option_text(ui, scale, 212.0, 59.0, &self.option_labels.video, BLUE);
        option_text(ui, scale, 374.0, 59.0, &self.option_labels.controls, BLUE);

        let detail = ["High", "Medium", "Low"];
        let object_detail = ["Very High", "High", "Medium", "Low", "Very Low"];
        let left_rows = [87.0, 118.0, 149.0, 180.0];
        let left_labels = [
            &self.option_labels.resolution,
            &self.option_labels.color_depth,
            &self.option_labels.texture_detail,
            &self.option_labels.object_detail,
        ];
        for (y, label) in left_rows.into_iter().zip(left_labels) {
            option_label(ui, scale, 45.0, y, label, PURPLE);
        }

        let resolution = self.options.resolutions[self.options.resolution];
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[0],
            &self.textures.option_bar,
            &format!("{}x{}", resolution.0, resolution.1),
        ) {
            self.options.resolution =
                (self.options.resolution + 1) % self.options.resolutions.len();
            let resolution = self.options.resolutions[self.options.resolution];
            self.action = Some(Action::SetResolution(resolution.0, resolution.1));
        }
        let _ = option_button(
            ui,
            scale,
            159.0,
            left_rows[1],
            &self.textures.option_bar,
            "32 Bit",
        );
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[2],
            &self.textures.option_bar,
            detail[self.options.texture_detail],
        ) {
            self.options.texture_detail = (self.options.texture_detail + 1) % detail.len();
        }
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[3],
            &self.textures.option_bar,
            object_detail[self.options.object_detail],
        ) {
            self.options.object_detail = (self.options.object_detail + 1) % object_detail.len();
        }

        option_label(
            ui,
            scale,
            45.0,
            211.0,
            &self.option_labels.brightness,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            205.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.brightness,
        ) {
            self.action = Some(Action::SetBrightness(0.2 + self.options.brightness * 0.8));
        }
        option_label(
            ui,
            scale,
            45.0,
            244.0,
            &self.option_labels.mouse_speed,
            PURPLE,
        );
        option_slider(
            ui,
            scale,
            159.0,
            238.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.mouse_speed,
        );
        option_text(ui, scale, 159.0, 268.0, &self.option_labels.low, PURPLE);
        option_text(ui, scale, 293.0, 268.0, &self.option_labels.high, PURPLE);
        option_text(ui, scale, 212.0, 294.0, &self.option_labels.audio, BLUE);
        option_label(
            ui,
            scale,
            45.0,
            320.0,
            &self.option_labels.music_volume,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            314.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.music_volume,
        ) {
            self.action = Some(Action::SetMusicVolume(
                (self.options.music_volume * 255.0).round() as u8,
            ));
        }
        option_label(
            ui,
            scale,
            45.0,
            357.0,
            &self.option_labels.sound_volume,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            351.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.sound_volume,
        ) {
            self.action = Some(Action::SetSoundVolume(
                (self.options.sound_volume * 255.0).round() as u8,
            ));
        }

        let key_values = [
            "W or Up",
            "S or Down",
            "A or Left",
            "D or Right",
            "Space or Right Mouse",
            "Left Mouse or Alt",
            "Z",
            "X",
        ];
        for (index, (label, value)) in self.option_labels.keys.iter().zip(key_values).enumerate() {
            let y = 87.0 + index as f32 * 31.0;
            let _ = option_button(ui, scale, 329.0, y, &self.textures.option_bar, value);
            option_label(ui, scale, 484.0, y, label, PURPLE);
        }
        if option_checkbox(
            ui,
            scale,
            329.0,
            338.0,
            &self.textures.checkbox_off,
            &self.textures.checkbox_on,
            &self.option_labels.auto_jump,
            self.options.auto_jump,
        ) {
            self.options.auto_jump = !self.options.auto_jump;
        }
        if option_checkbox(
            ui,
            scale,
            329.0,
            358.0,
            &self.textures.checkbox_off,
            &self.textures.checkbox_on,
            &self.option_labels.invert_broom,
            self.options.invert_broom,
        ) {
            self.options.invert_broom = !self.options.invert_broom;
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

fn option_text(ui: &egui::Ui, scale: f32, x: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(x, y) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        color,
    );
}

fn option_label(ui: &egui::Ui, scale: f32, x: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(x, y + 8.0) * scale,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        color,
    );
}

fn option_button(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    texture: &TextureHandle,
    text: &str,
) -> bool {
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 134.0, 18.0);
    let response = ui.interact(
        rect,
        Id::new(("option", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    ui.painter().image(
        texture.id(),
        rect,
        texture_uv(texture, Vec2::new(134.0, 18.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        Color32::BLACK,
    );
    response.clicked()
}

fn option_slider(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    track: &TextureHandle,
    knob: &TextureHandle,
    value: &mut f32,
) -> bool {
    let origin = ui.min_rect().min;
    let rect = scaled_rect(origin, scale, x, y, 134.0, 25.0);
    let response = ui.interact(
        rect,
        Id::new(("slider", x.to_bits(), y.to_bits())),
        Sense::click_and_drag(),
    );
    let changed = (response.clicked() || response.dragged())
        && response.interact_pointer_pos().is_some_and(|pointer| {
            let next = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let changed = (*value - next).abs() > f32::EPSILON;
            *value = next;
            changed
        });
    let track_rect = Rect::from_min_size(
        origin + Vec2::new(x, y + 8.0) * scale,
        Vec2::new(134.0, 9.0) * scale,
    );
    if response.hovered() {
        ui.painter().image(
            track.id(),
            track_rect,
            texture_uv(track, Vec2::new(134.0, 9.0)),
            Color32::WHITE,
        );
    }
    let knob_width = 9.0 * scale;
    let knob_position = Pos2::new(
        rect.left() + (rect.width() - knob_width) * *value,
        rect.top(),
    );
    ui.painter().image(
        knob.id(),
        Rect::from_min_size(knob_position, Vec2::new(9.0, 25.0) * scale),
        texture_uv(knob, Vec2::new(9.0, 25.0)),
        Color32::WHITE,
    );
    changed
}

fn option_checkbox(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    off: &TextureHandle,
    on: &TextureHandle,
    text: &str,
    checked: bool,
) -> bool {
    let texture = if checked { on } else { off };
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 160.0, 18.0);
    let response = ui.interact(
        rect,
        Id::new(("check", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    ui.painter().image(
        texture.id(),
        Rect::from_min_size(rect.min, Vec2::new(12.0, 12.0) * scale),
        texture_uv(texture, Vec2::new(12.0, 12.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.min + Vec2::new(17.0, 8.0) * scale,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        Color32::from_rgb(96, 0, 96),
    );
    response.clicked()
}

fn texture_uv(texture: &TextureHandle, size: Vec2) -> Rect {
    let texture_size = texture.size_vec2();
    Rect::from_min_max(
        Pos2::ZERO,
        Pos2::new(
            size.x.min(texture_size.x) / texture_size.x,
            size.y.min(texture_size.y) / texture_size.y,
        ),
    )
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
