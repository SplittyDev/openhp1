use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use eframe::egui::{
    self, Color32, ColorImage, PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Vec2,
};
use image::{ColorType, ImageFormat};
use openhp1_package::{
    ObjectReference, PackageStore, ResolvedObject, write_derived_file_atomically,
};
use openhp1_texture::{Palette, Texture, is_window_texture_name, window_mask_filename};
use serde::{Deserialize, Serialize};

const STATE_VERSION: u32 = 1;

pub struct WindowEditor {
    packages: PackageStore,
    windows: Vec<String>,
    selected: usize,
    active: ActiveTexture,
    state_path: PathBuf,
    state: EditorState,
    picked_color: Option<PickedColor>,
    draft_kind: RuleKind,
    draft_threshold: f32,
    draft_radius: u32,
    tool: ToolMode,
    rectangle_start: Option<[u32; 2]>,
    draft_rectangle: Option<PixelRect>,
    preview: PreviewMode,
    zoom: f32,
    pan: Vec2,
    status: String,
}

struct ActiveTexture {
    name: String,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
    source: TextureHandle,
    mask: TextureHandle,
    overlay: TextureHandle,
}

struct DecodedImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewMode {
    Source,
    Mask,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolMode {
    Pick,
    ExcludeRectangle,
}

#[derive(Clone, Copy, Debug)]
struct PickedColor {
    color: [u8; 3],
    position: [u32; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EditorState {
    version: u32,
    #[serde(default)]
    windows: BTreeMap<String, WindowState>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            windows: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct WindowState {
    #[serde(default)]
    rules: Vec<ColorRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    excluded_rectangles: Vec<PixelRect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ColorRule {
    color: [u8; 3],
    kind: RuleKind,
    threshold: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "is_zero")]
    radius: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PixelRect {
    min: [u32; 2],
    max: [u32; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuleKind {
    Include,
    Exclude,
}

#[derive(Default)]
struct PanelAction {
    state_changed: bool,
    preview_changed: bool,
    export_active: bool,
    export_all: bool,
}

#[derive(Default)]
struct CanvasAction {
    preview_changed: bool,
    state_changed: bool,
}

impl WindowEditor {
    pub fn new(
        context: &eframe::CreationContext<'_>,
        mut packages: PackageStore,
        state_path: PathBuf,
    ) -> Result<Self> {
        let windows = discover_windows(&mut packages)?;
        let first = windows.first().context("no window textures were found")?;
        let state = load_state(&state_path)?;
        let image = decode_image(&mut packages, first)?;
        let window = state.windows.get(first);
        let rules = window.map_or(&[][..], |window| window.rules.as_slice());
        let rectangles = window.map_or(&[][..], |window| window.excluded_rectangles.as_slice());
        let active = ActiveTexture::new(&context.egui_ctx, first.clone(), image, rules, rectangles);
        Ok(Self {
            packages,
            windows,
            selected: 0,
            active,
            state_path,
            state,
            picked_color: None,
            draft_kind: RuleKind::Include,
            draft_threshold: 10.0,
            draft_radius: 0,
            tool: ToolMode::Pick,
            rectangle_start: None,
            draft_rectangle: None,
            preview: PreviewMode::Overlay,
            zoom: 1.0,
            pan: Vec2::ZERO,
            status: "Click the image to pick a color.".to_owned(),
        })
    }

    fn select(&mut self, context: &egui::Context, index: usize) {
        let name = self.windows[index].clone();
        match decode_image(&mut self.packages, &name) {
            Ok(image) => {
                let window = self.state.windows.get(&name);
                let rules = window.map_or(&[][..], |window| window.rules.as_slice());
                let rectangles =
                    window.map_or(&[][..], |window| window.excluded_rectangles.as_slice());
                self.active = ActiveTexture::new(context, name, image, rules, rectangles);
                self.selected = index;
                self.picked_color = None;
                self.rectangle_start = None;
                self.draft_rectangle = None;
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                self.status = "Click the image to pick a color.".to_owned();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn save(&mut self) {
        match save_state(&self.state_path, &self.state) {
            Ok(()) => self.status = format!("Saved {}", self.state_path.display()),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn refresh_preview(&mut self) {
        let window = self.state.windows.get(&self.active.name);
        let saved = window.map_or(&[][..], |window| window.rules.as_slice());
        let draft = self.picked_color.map(|picked| ColorRule {
            color: picked.color,
            kind: self.draft_kind,
            threshold: self.draft_threshold,
            position: Some(picked.position),
            radius: self.draft_radius,
        });
        let rules = preview_rules(saved, draft);
        let mut rectangles =
            window.map_or_else(Vec::new, |window| window.excluded_rectangles.clone());
        rectangles.extend(self.draft_rectangle);
        self.active.refresh_preview(&rules, &rectangles);
    }

    fn export_active(&mut self) {
        let result = self
            .state
            .windows
            .get(&self.active.name)
            .filter(|window| !window.rules.is_empty())
            .context("add at least one include rule before exporting")
            .and_then(|window| {
                let output = mask_path(&self.state_path, &self.active.name);
                write_mask(
                    &output,
                    self.active.width,
                    self.active.height,
                    &self.active.rgba,
                    &window.rules,
                    &window.excluded_rectangles,
                )?;
                Ok(output)
            });
        self.status = match result {
            Ok(path) => format!("Exported {}", path.display()),
            Err(error) => error.to_string(),
        };
    }

    fn export_all(&mut self) {
        let definitions = self
            .state
            .windows
            .iter()
            .filter(|(_, window)| !window.rules.is_empty())
            .map(|(name, window)| {
                (
                    name.clone(),
                    window.rules.clone(),
                    window.excluded_rectangles.clone(),
                )
            })
            .collect::<Vec<_>>();
        let result = (|| -> Result<usize> {
            for (name, rules, rectangles) in &definitions {
                let image = decode_image(&mut self.packages, name)?;
                write_mask(
                    &mask_path(&self.state_path, name),
                    image.width,
                    image.height,
                    &image.rgba,
                    rules,
                    rectangles,
                )?;
            }
            Ok(definitions.len())
        })();
        self.status = match result {
            Ok(count) => format!("Exported {count} masks to window_masks"),
            Err(error) => error.to_string(),
        };
    }

    fn window_sidebar(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        ui.heading("Windows");
        ui.label(format!("{} live textures", self.windows.len()));
        ui.separator();
        let mut requested = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, name) in self.windows.iter().enumerate() {
                let configured = self.state.windows.get(name).is_some_and(|window| {
                    !window.rules.is_empty() || !window.excluded_rectangles.is_empty()
                });
                let label = if configured {
                    format!("● {name}")
                } else {
                    name.clone()
                };
                if ui.selectable_label(index == self.selected, label).clicked() {
                    requested = Some(index);
                }
            }
        });
        requested
    }

    fn rules_panel(&mut self, ui: &mut egui::Ui) -> PanelAction {
        let mut action = PanelAction::default();
        ui.heading("Mask rules");
        ui.label(RichText::new(&self.active.name).monospace());
        ui.label(format!("{} × {}", self.active.width, self.active.height));
        ui.separator();

        if let Some(picked) = self.picked_color {
            ui.horizontal(|ui| {
                color_swatch(ui, picked.color);
                ui.monospace(format!(
                    "rgb({}, {}, {}) at {}, {}",
                    picked.color[0],
                    picked.color[1],
                    picked.color[2],
                    picked.position[0],
                    picked.position[1]
                ));
            });
        } else {
            ui.label("Pick a source color from the image.");
        }
        ui.horizontal(|ui| {
            action.preview_changed |= ui
                .radio_value(&mut self.draft_kind, RuleKind::Include, "Include")
                .changed();
            action.preview_changed |= ui
                .radio_value(&mut self.draft_kind, RuleKind::Exclude, "Exclude")
                .changed();
        });
        action.preview_changed |= ui
            .add(
                egui::Slider::new(&mut self.draft_threshold, 0.0..=100.0)
                    .suffix("%")
                    .text("Similarity"),
            )
            .changed();
        action.preview_changed |= ui
            .add(
                egui::Slider::new(
                    &mut self.draft_radius,
                    0..=u32::try_from(self.active.width.max(self.active.height))
                        .unwrap_or(u32::MAX),
                )
                .suffix(" px")
                .text("Radius (0 = global)"),
            )
            .changed();
        if ui
            .add_enabled(
                self.picked_color.is_some(),
                egui::Button::new("Add color rule"),
            )
            .clicked()
        {
            let picked = self.picked_color.expect("button requires a picked color");
            self.state
                .windows
                .entry(self.active.name.clone())
                .or_default()
                .rules
                .push(ColorRule {
                    color: picked.color,
                    kind: self.draft_kind,
                    threshold: self.draft_threshold,
                    position: Some(picked.position),
                    radius: self.draft_radius,
                });
            self.picked_color = None;
            action.state_changed = true;
        }

        ui.separator();
        ui.label("Saved rules");
        let mut remove = None;
        if let Some(window) = self.state.windows.get_mut(&self.active.name) {
            for (index, rule) in window.rules.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        color_swatch(ui, rule.color);
                        action.state_changed |= ui
                            .selectable_value(&mut rule.kind, RuleKind::Include, "Include")
                            .changed();
                        action.state_changed |= ui
                            .selectable_value(&mut rule.kind, RuleKind::Exclude, "Exclude")
                            .changed();
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                    });
                    action.state_changed |= ui
                        .add(
                            egui::Slider::new(&mut rule.threshold, 0.0..=100.0)
                                .suffix("%")
                                .text("Similarity"),
                        )
                        .changed();
                    if rule.position.is_some() {
                        action.state_changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut rule.radius,
                                    0..=u32::try_from(self.active.width.max(self.active.height))
                                        .unwrap_or(u32::MAX),
                                )
                                .suffix(" px")
                                .text("Radius (0 = global)"),
                            )
                            .changed();
                    } else {
                        ui.small("Global rule saved before spatial radii were available.");
                    }
                });
            }
            if let Some(index) = remove {
                window.rules.remove(index);
                action.state_changed = true;
            }
        }

        ui.separator();
        ui.label("Excluded rectangles");
        let mut remove_rectangle = None;
        if let Some(window) = self.state.windows.get_mut(&self.active.name) {
            for (index, rectangle) in window.excluded_rectangles.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(format!(
                        "{}, {} → {}, {}",
                        rectangle.min[0], rectangle.min[1], rectangle.max[0], rectangle.max[1]
                    ));
                    if ui.small_button("Remove").clicked() {
                        remove_rectangle = Some(index);
                    }
                });
            }
            if let Some(index) = remove_rectangle {
                window.excluded_rectangles.remove(index);
                action.state_changed = true;
            }
        }

        ui.separator();
        action.export_active = ui.button("Export active mask").clicked();
        action.export_all = ui.button("Export all configured masks").clicked();
        ui.small("Exports are 8-bit grayscale: black blocks, white transmits.");
        ui.separator();
        ui.label(&self.status);
        action
    }

    fn canvas(&mut self, ui: &mut egui::Ui) -> CanvasAction {
        let mut action = CanvasAction::default();
        let previous_tool = self.tool;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.preview, PreviewMode::Source, "Source");
            ui.selectable_value(&mut self.preview, PreviewMode::Mask, "Mask");
            ui.selectable_value(&mut self.preview, PreviewMode::Overlay, "Overlay");
            ui.separator();
            ui.selectable_value(&mut self.tool, ToolMode::Pick, "Pick / pan");
            ui.selectable_value(
                &mut self.tool,
                ToolMode::ExcludeRectangle,
                "Exclude rectangle",
            );
            ui.add(
                egui::Slider::new(&mut self.zoom, 0.25..=16.0)
                    .logarithmic(true)
                    .text("Zoom"),
            );
            if ui.button("Reset view").clicked() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
        });
        if self.tool != previous_tool {
            self.picked_color = None;
            self.rectangle_start = None;
            self.draft_rectangle = None;
            self.preview = PreviewMode::Overlay;
            action.preview_changed = true;
        }
        ui.label(match self.tool {
            ToolMode::Pick => "Click to pick · drag to pan · scroll to zoom",
            ToolMode::ExcludeRectangle => "Drag over any area that must never transmit light",
        });

        let available = ui.available_size();
        let (canvas, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        ui.painter()
            .rect_filled(canvas, 0.0, Color32::from_rgb(18, 18, 18));

        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.25, 16.0);
            }
            ui.ctx()
                .set_cursor_icon(if self.tool == ToolMode::Pick && response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Crosshair
                });
        }
        if self.tool == ToolMode::Pick && response.dragged() {
            self.pan += response.drag_delta();
        }

        let texture = match self.preview {
            PreviewMode::Source => &self.active.source,
            PreviewMode::Mask => &self.active.mask,
            PreviewMode::Overlay => &self.active.overlay,
        };
        let source_size = Vec2::new(self.active.width as f32, self.active.height as f32);
        let fit = (canvas.width() / source_size.x)
            .min(canvas.height() / source_size.y)
            .max(0.01);
        let image_size = source_size * fit * self.zoom;
        let image_rect = Rect::from_center_size(canvas.center() + self.pan, image_size);
        ui.painter().image(
            texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        let pointer_pixel = response.interact_pointer_pos().and_then(|position| {
            pixel_at(position, image_rect, self.active.width, self.active.height)
        });
        match self.tool {
            ToolMode::Pick => {
                if response.clicked_by(PointerButton::Primary)
                    && let Some(position) = pointer_pixel
                {
                    let offset =
                        (position[1] as usize * self.active.width + position[0] as usize) * 4;
                    self.picked_color = Some(PickedColor {
                        color: [
                            self.active.rgba[offset],
                            self.active.rgba[offset + 1],
                            self.active.rgba[offset + 2],
                        ],
                        position,
                    });
                    self.preview = PreviewMode::Overlay;
                    self.status = format!("Picked pixel {}, {}", position[0], position[1]);
                    action.preview_changed = true;
                }
            }
            ToolMode::ExcludeRectangle => {
                if response.drag_started_by(PointerButton::Primary)
                    && let Some(position) = pointer_pixel
                {
                    self.rectangle_start = Some(position);
                    self.draft_rectangle = Some(PixelRect::from_points(position, position));
                    action.preview_changed = true;
                }
                if response.dragged_by(PointerButton::Primary)
                    && let (Some(start), Some(position)) = (self.rectangle_start, pointer_pixel)
                {
                    self.draft_rectangle = Some(PixelRect::from_points(start, position));
                    action.preview_changed = true;
                }
                if response.drag_stopped_by(PointerButton::Primary) {
                    self.rectangle_start = None;
                    if let Some(rectangle) = self.draft_rectangle.take() {
                        self.state
                            .windows
                            .entry(self.active.name.clone())
                            .or_default()
                            .excluded_rectangles
                            .push(rectangle);
                        action.state_changed = true;
                    }
                }
            }
        }

        if self.tool == ToolMode::ExcludeRectangle {
            let rectangles = self
                .state
                .windows
                .get(&self.active.name)
                .map_or(&[][..], |window| window.excluded_rectangles.as_slice());
            for rectangle in rectangles {
                ui.painter().rect_stroke(
                    screen_rectangle(
                        *rectangle,
                        image_rect,
                        self.active.width,
                        self.active.height,
                    ),
                    0.0,
                    Stroke::new(1.0, Color32::from_rgb(190, 50, 50)),
                    StrokeKind::Inside,
                );
            }
            if let Some(rectangle) = self.draft_rectangle {
                ui.painter().rect_stroke(
                    screen_rectangle(rectangle, image_rect, self.active.width, self.active.height),
                    0.0,
                    Stroke::new(2.0, Color32::RED),
                    StrokeKind::Inside,
                );
            }
        } else if let Some(picked) = self.picked_color {
            let center = screen_position(
                picked.position,
                image_rect,
                self.active.width,
                self.active.height,
            );
            let radius = if self.draft_radius == 0 {
                4.0
            } else {
                self.draft_radius as f32 * image_rect.width() / self.active.width as f32
            };
            ui.painter()
                .circle_stroke(center, radius, Stroke::new(1.5, Color32::YELLOW));
        }
        action
    }
}

impl eframe::App for WindowEditor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let mut selection = None;
        egui::Panel::left("window_list")
            .default_size(310.0)
            .show(ui, |ui| selection = self.window_sidebar(ui));
        let mut panel_action = PanelAction::default();
        egui::Panel::right("mask_rules")
            .default_size(330.0)
            .show(ui, |ui| panel_action = self.rules_panel(ui));
        let mut canvas_action = CanvasAction::default();
        egui::CentralPanel::default().show(ui, |ui| canvas_action = self.canvas(ui));

        if let Some(index) = selection {
            self.select(&context, index);
        }
        if panel_action.state_changed
            || panel_action.preview_changed
            || canvas_action.preview_changed
            || canvas_action.state_changed
        {
            self.refresh_preview();
        }
        if panel_action.state_changed || canvas_action.state_changed {
            self.save();
        }
        if panel_action.export_active {
            self.export_active();
        }
        if panel_action.export_all {
            self.export_all();
        }
    }
}

impl ActiveTexture {
    fn new(
        context: &egui::Context,
        name: String,
        image: DecodedImage,
        rules: &[ColorRule],
        rectangles: &[PixelRect],
    ) -> Self {
        let source_image =
            ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba);
        let (mask_image, overlay_image) =
            preview_images(image.width, image.height, &image.rgba, rules, rectangles);
        let source = context.load_texture(
            format!("window-source:{name}"),
            source_image,
            TextureOptions::NEAREST,
        );
        let mask = context.load_texture(
            format!("window-mask:{name}"),
            mask_image,
            TextureOptions::NEAREST,
        );
        let overlay = context.load_texture(
            format!("window-overlay:{name}"),
            overlay_image,
            TextureOptions::NEAREST,
        );
        Self {
            name,
            width: image.width,
            height: image.height,
            rgba: image.rgba,
            source,
            mask,
            overlay,
        }
    }

    fn refresh_preview(&mut self, rules: &[ColorRule], rectangles: &[PixelRect]) {
        let (mask, overlay) =
            preview_images(self.width, self.height, &self.rgba, rules, rectangles);
        self.mask.set(mask, TextureOptions::NEAREST);
        self.overlay.set(overlay, TextureOptions::NEAREST);
    }
}

fn discover_windows(packages: &mut PackageStore) -> Result<Vec<String>> {
    let paths = packages
        .package_paths()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    let mut windows = BTreeSet::new();
    for path in paths {
        let package = packages.load_path(&path)?;
        let summary = package.summary();
        for (export_index, export) in summary.exports.iter().enumerate() {
            let Some(class) = summary.class_name(export) else {
                continue;
            };
            if !class.to_ascii_lowercase().ends_with("texture") {
                continue;
            }
            let object = ResolvedObject {
                package: package.clone(),
                export_index,
            };
            let Ok(name) = PackageStore::qualified_object_name(&object) else {
                continue;
            };
            if is_window_texture_name(&name) {
                windows.insert(name);
            }
        }
    }
    Ok(windows.into_iter().collect())
}

fn decode_image(packages: &mut PackageStore, name: &str) -> Result<DecodedImage> {
    let resolved = packages
        .find_localized_object(name, "Texture")?
        .with_context(|| format!("texture {name} is missing"))?;
    let texture = Texture::decode(&resolved.package, resolved.export_index)?;
    let palette = match texture.palette {
        ObjectReference::Export(export_index) => Palette::decode(&resolved.package, export_index)?,
        reference => {
            let palette = packages
                .resolve(&resolved.package, reference)?
                .with_context(|| format!("texture {name} has no palette"))?;
            Palette::decode(&palette.package, palette.export_index)?
        }
    };
    let mip = texture
        .mips
        .first()
        .with_context(|| format!("texture {name} has no mip"))?;
    Ok(DecodedImage {
        width: mip.width as usize,
        height: mip.height as usize,
        rgba: texture.rgba(0, &palette, false)?,
    })
}

fn preview_images(
    width: usize,
    height: usize,
    rgba: &[u8],
    rules: &[ColorRule],
    rectangles: &[PixelRect],
) -> (ColorImage, ColorImage) {
    let alpha = mask_alpha(width, rgba, rules, rectangles);
    let mut mask = Vec::with_capacity(rgba.len());
    let mut overlay = rgba.to_vec();
    for (index, alpha) in alpha.into_iter().enumerate() {
        mask.extend_from_slice(&[alpha, alpha, alpha, 255]);
        if alpha == 0 {
            let offset = index * 4;
            overlay[offset] = overlay[offset] / 4 + 96;
            overlay[offset + 1] /= 4;
            overlay[offset + 2] /= 4;
        }
    }
    (
        ColorImage::from_rgba_unmultiplied([width, height], &mask),
        ColorImage::from_rgba_unmultiplied([width, height], &overlay),
    )
}

fn mask_alpha(width: usize, rgba: &[u8], rules: &[ColorRule], rectangles: &[PixelRect]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .map(|(index, pixel)| {
            let x = (index % width) as u32;
            let y = (index / width) as u32;
            let included = rules
                .iter()
                .filter(|rule| rule.kind == RuleKind::Include)
                .any(|rule| color_matches(pixel, x, y, rule));
            let excluded = rules
                .iter()
                .filter(|rule| rule.kind == RuleKind::Exclude)
                .any(|rule| color_matches(pixel, x, y, rule))
                || rectangles
                    .iter()
                    .any(|rectangle| rectangle.contains([x, y]));
            if included && !excluded { 255 } else { 0 }
        })
        .collect()
}

fn preview_rules(saved: &[ColorRule], draft: Option<ColorRule>) -> Vec<ColorRule> {
    saved.iter().cloned().chain(draft).collect()
}

fn color_matches(pixel: &[u8], x: u32, y: u32, rule: &ColorRule) -> bool {
    if rule.radius > 0
        && let Some([center_x, center_y]) = rule.position
    {
        let dx = i64::from(x) - i64::from(center_x);
        let dy = i64::from(y) - i64::from(center_y);
        if dx * dx + dy * dy > i64::from(rule.radius).pow(2) {
            return false;
        }
    }
    let distance = pixel[..3]
        .iter()
        .zip(rule.color)
        .map(|(component, sample)| {
            let delta = f32::from(*component) - f32::from(sample);
            delta * delta
        })
        .sum::<f32>();
    let radius = rule.threshold.clamp(0.0, 100.0) / 100.0 * 255.0 * 3.0_f32.sqrt();
    distance <= radius * radius
}

impl PixelRect {
    fn from_points(first: [u32; 2], second: [u32; 2]) -> Self {
        Self {
            min: [first[0].min(second[0]), first[1].min(second[1])],
            max: [
                first[0].max(second[0]).saturating_add(1),
                first[1].max(second[1]).saturating_add(1),
            ],
        }
    }

    fn contains(self, point: [u32; 2]) -> bool {
        point[0] >= self.min[0]
            && point[1] >= self.min[1]
            && point[0] < self.max[0]
            && point[1] < self.max[1]
    }
}

fn pixel_at(position: Pos2, image: Rect, width: usize, height: usize) -> Option<[u32; 2]> {
    if !image.contains(position) {
        return None;
    }
    let relative = (position - image.min) / image.size();
    Some([
        ((relative.x * width as f32) as u32).min(width.saturating_sub(1) as u32),
        ((relative.y * height as f32) as u32).min(height.saturating_sub(1) as u32),
    ])
}

fn screen_position(position: [u32; 2], image: Rect, width: usize, height: usize) -> Pos2 {
    image.min
        + Vec2::new(
            (position[0] as f32 + 0.5) / width as f32 * image.width(),
            (position[1] as f32 + 0.5) / height as f32 * image.height(),
        )
}

fn screen_rectangle(rectangle: PixelRect, image: Rect, width: usize, height: usize) -> Rect {
    Rect::from_min_max(
        image.min
            + Vec2::new(
                rectangle.min[0] as f32 / width as f32 * image.width(),
                rectangle.min[1] as f32 / height as f32 * image.height(),
            ),
        image.min
            + Vec2::new(
                rectangle.max[0] as f32 / width as f32 * image.width(),
                rectangle.max[1] as f32 / height as f32 * image.height(),
            ),
    )
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn load_state(path: &Path) -> Result<EditorState> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(EditorState::default()),
        Err(source) => return Err(source).with_context(|| format!("reading {}", path.display())),
    };
    let state: EditorState =
        serde_json::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    if state.version != STATE_VERSION {
        bail!(
            "{} has unsupported version {}",
            path.display(),
            state.version
        );
    }
    Ok(state)
}

fn save_state(path: &Path, state: &EditorState) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(state)?;
    contents.push('\n');
    write_derived_file_atomically(path, contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

fn mask_path(state_path: &Path, name: &str) -> PathBuf {
    let directory = state_path.parent().unwrap_or_else(|| Path::new("."));
    directory
        .join("window_masks")
        .join(window_mask_filename(name))
}

fn write_mask(
    path: &Path,
    width: usize,
    height: usize,
    source: &[u8],
    rules: &[ColorRule],
    rectangles: &[PixelRect],
) -> Result<()> {
    let mask = mask_alpha(width, source, rules, rectangles);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer_with_format(
        path,
        &mask,
        u32::try_from(width)?,
        u32::try_from(height)?,
        ColorType::L8,
        ImageFormat::Png,
    )
    .with_context(|| format!("writing {}", path.display()))
}

fn color_swatch(ui: &mut egui::Ui, color: [u8; 3]) {
    ui.label(
        RichText::new("■■")
            .size(20.0)
            .color(Color32::from_rgb(color[0], color[1], color[2])),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn included_colors_form_mask_and_exclusions_win() {
        let pixels = [100, 100, 100, 255, 200, 20, 20, 255, 205, 25, 25, 255];
        let rules = [
            ColorRule {
                color: [200, 20, 20],
                kind: RuleKind::Include,
                threshold: 5.0,
                position: None,
                radius: 0,
            },
            ColorRule {
                color: [205, 25, 25],
                kind: RuleKind::Exclude,
                threshold: 0.0,
                position: None,
                radius: 0,
            },
        ];
        assert_eq!(mask_alpha(3, &pixels, &rules, &[]), [0, 255, 0]);

        let json = serde_json::to_string(&EditorState {
            version: STATE_VERSION,
            windows: BTreeMap::from([(
                "HP_K.Window Frames.Win14_T1".to_owned(),
                WindowState {
                    rules: rules.to_vec(),
                    excluded_rectangles: vec![PixelRect {
                        min: [0, 0],
                        max: [1, 1],
                    }],
                },
            )]),
        })
        .unwrap();
        let decoded: EditorState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.windows.len(), 1);
        assert_eq!(decoded.windows.values().next().unwrap().rules.len(), 2);

        let legacy: EditorState = serde_json::from_str(
            r#"{"version":1,"windows":{"Legacy.Window.Test":{"rules":[{"color":[1,2,3],"kind":"include","threshold":4.0}]}}}"#,
        )
        .unwrap();
        let legacy_window = legacy.windows.values().next().unwrap();
        assert!(legacy_window.excluded_rectangles.is_empty());
        assert_eq!(legacy_window.rules[0].position, None);
        assert_eq!(legacy_window.rules[0].radius, 0);

        let draft = ColorRule {
            color: [100, 100, 100],
            kind: RuleKind::Include,
            threshold: 0.0,
            position: Some([0, 0]),
            radius: 1,
        };
        assert_eq!(
            mask_alpha(3, &pixels, &preview_rules(&[], Some(draft)), &[]),
            [255, 0, 0]
        );

        let include_everything = ColorRule {
            color: [0, 0, 0],
            kind: RuleKind::Include,
            threshold: 100.0,
            position: None,
            radius: 0,
        };
        assert_eq!(
            mask_alpha(
                3,
                &pixels,
                &[include_everything],
                &[PixelRect {
                    min: [0, 0],
                    max: [1, 1]
                }]
            ),
            [0, 255, 255]
        );
    }
}
