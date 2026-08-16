use std::io;

use openhp1_render::{
    AmbientOcclusion, DisplaySettings, RendererMode, RendererSettings, ToneMapper,
};
use openhp1_runtime::ConsoleCommands;
use openhp1_scene::LoadedScene;

const CONFIG: &str = "OpenHP1";
const RENDERER_SECTION: &str = "OpenHP1.Renderer";
const CLASSIC_SECTION: &str = "OpenHP1.Renderer.Classic";
const MODERN_SECTION: &str = "OpenHP1.Renderer.Modern";
const WINDOWS_CLIENT_SECTION: &str = "WinDrv.WindowsClient";
const LEGACY_SECTION: &str = "OpenHP1.Graphics";
const MAX_RENDER_PIXELS: u64 = 3840 * 2160;

pub(super) const DEFAULT_RESOLUTION: [u32; 2] = [1024, 768];
pub(super) const DEFAULT_WINDOW_SIZE: [u32; 2] = [1280, 800];

pub(super) const RESOLUTION_PRESETS: [([u32; 2], &str); 12] = [
    ([512, 384], "512x384 (Classic)"),
    ([640, 480], "640x480 (Classic)"),
    ([800, 600], "800x600 (Classic)"),
    ([1024, 768], "1024x768 (Classic)"),
    ([1280, 960], "1280x960 (Enhanced 4:3)"),
    ([1600, 1200], "1600x1200 (Enhanced 4:3)"),
    ([1920, 1440], "1920x1440 (Enhanced 4:3)"),
    ([2560, 1920], "2560x1920 (Enhanced 4:3)"),
    ([1280, 720], "1280x720 (Widescreen)"),
    ([1920, 1080], "1920x1080 (Widescreen)"),
    ([2560, 1440], "2560x1440 (Widescreen)"),
    ([3840, 2160], "3840x2160 (Widescreen)"),
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ColorDepth {
    #[default]
    TrueColor,
    Rgb565,
}

impl ColorDepth {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::TrueColor => "32 Bit",
            Self::Rgb565 => "16 Bit (Emulated)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GraphicsSettings {
    pub(super) resolution: [u32; 2],
    window_size: [u32; 2],
    pub(super) renderer: RendererSettings,
    pub(super) color_depth: ColorDepth,
    pub(super) screen_flashes: bool,
    pub(super) classic_display: DisplaySettings,
    modern_agx_display: DisplaySettings,
    modern_reinhard_display: DisplaySettings,
    modern_aces_display: DisplaySettings,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_RESOLUTION,
            window_size: DEFAULT_WINDOW_SIZE,
            renderer: RendererSettings::default(),
            color_depth: ColorDepth::default(),
            screen_flashes: true,
            classic_display: DisplaySettings::for_mode(RendererMode::Classic),
            modern_agx_display: DisplaySettings::for_tone_mapper(ToneMapper::AgX),
            modern_reinhard_display: DisplaySettings::for_tone_mapper(ToneMapper::Reinhard),
            modern_aces_display: DisplaySettings::for_tone_mapper(ToneMapper::Aces),
        }
    }
}

impl GraphicsSettings {
    pub(super) fn load(
        console: &ConsoleCommands,
        renderer_override: Option<RendererSettings>,
    ) -> Self {
        let defaults = Self::default();
        let mut renderer = defaults.renderer;
        renderer.mode = config(console, RENDERER_SECTION, "Renderer", "Renderer")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.mode);
        renderer.detail_textures = config(
            console,
            RENDERER_SECTION,
            "DetailTextures",
            "DetailTextures",
        )
        .and_then(|value| parse_bool(&value))
        .unwrap_or(renderer.detail_textures);
        renderer.tone_mapper = config(console, MODERN_SECTION, "ToneMapper", "ToneMapper")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.tone_mapper);
        renderer.ambient_occlusion = config(
            console,
            MODERN_SECTION,
            "AmbientOcclusion",
            "AmbientOcclusion",
        )
        .and_then(|value| value.parse().ok())
        .unwrap_or(renderer.ambient_occlusion);
        renderer.antialiasing = config(console, MODERN_SECTION, "AntiAliasing", "AntiAliasing")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.antialiasing);
        renderer.bloom = config(console, MODERN_SECTION, "Bloom", "Bloom")
            .and_then(|value| parse_bool(&value))
            .unwrap_or(renderer.bloom);
        renderer.volumetric_lighting = config(
            console,
            MODERN_SECTION,
            "VolumetricLighting",
            "VolumetricLighting",
        )
        .and_then(|value| parse_bool(&value))
        .unwrap_or(renderer.volumetric_lighting);
        if let Some(override_settings) = renderer_override {
            renderer = override_settings;
        }

        let width = config(console, RENDERER_SECTION, "ResolutionX", "ResolutionX")
            .and_then(|value| dimension(&value));
        let height = config(console, RENDERER_SECTION, "ResolutionY", "ResolutionY")
            .and_then(|value| dimension(&value));
        let window_width = config(console, RENDERER_SECTION, "WindowSizeX", "WindowSizeX")
            .and_then(|value| dimension(&value));
        let window_height = config(console, RENDERER_SECTION, "WindowSizeY", "WindowSizeY")
            .and_then(|value| dimension(&value));
        let classic = DisplaySettings::for_mode(RendererMode::Classic);
        Self {
            resolution: resolution(width, height).unwrap_or(defaults.resolution),
            window_size: resolution(window_width, window_height).unwrap_or(defaults.window_size),
            renderer,
            color_depth: config(console, CLASSIC_SECTION, "ColorMode", "ColorDepth")
                .as_deref()
                .and_then(color_depth)
                .unwrap_or(defaults.color_depth),
            screen_flashes: console
                .config_value(CONFIG, WINDOWS_CLIENT_SECTION, "ScreenFlashes")
                .and_then(|value| parse_bool(&value))
                .unwrap_or(defaults.screen_flashes),
            classic_display: DisplaySettings {
                brightness: setting(
                    console,
                    CLASSIC_SECTION,
                    "Brightness",
                    "ClassicBrightness",
                    classic.brightness,
                    0.2,
                    1.0,
                ),
                ..classic
            },
            modern_agx_display: modern_display(console, renderer.tone_mapper, ToneMapper::AgX),
            modern_reinhard_display: modern_display(
                console,
                renderer.tone_mapper,
                ToneMapper::Reinhard,
            ),
            modern_aces_display: modern_display(console, renderer.tone_mapper, ToneMapper::Aces),
        }
    }

    pub(super) fn modern_display(&self) -> DisplaySettings {
        match self.renderer.tone_mapper {
            ToneMapper::AgX => self.modern_agx_display,
            ToneMapper::Reinhard => self.modern_reinhard_display,
            ToneMapper::Aces => self.modern_aces_display,
        }
    }

    pub(super) fn modern_display_mut(&mut self) -> &mut DisplaySettings {
        match self.renderer.tone_mapper {
            ToneMapper::AgX => &mut self.modern_agx_display,
            ToneMapper::Reinhard => &mut self.modern_reinhard_display,
            ToneMapper::Aces => &mut self.modern_aces_display,
        }
    }

    pub(super) fn display(self) -> DisplaySettings {
        match self.renderer.mode {
            RendererMode::Classic => self.classic_display,
            RendererMode::Modern => self.modern_display(),
        }
    }

    pub(super) fn visible_flash(self, flash: [f32; 4]) -> [f32; 4] {
        if self.screen_flashes {
            flash
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }

    pub(super) fn save(self, console: &ConsoleCommands) -> io::Result<()> {
        console.save_config_values(
            CONFIG,
            RENDERER_SECTION,
            &[
                ("ResolutionX", self.resolution[0].to_string()),
                ("ResolutionY", self.resolution[1].to_string()),
                ("WindowSizeX", self.window_size[0].to_string()),
                ("WindowSizeY", self.window_size[1].to_string()),
                ("Renderer", renderer_name(self.renderer.mode).to_owned()),
                ("DetailTextures", self.renderer.detail_textures.to_string()),
            ],
        )?;
        console.save_config_values(
            CONFIG,
            CLASSIC_SECTION,
            &[
                ("Brightness", self.classic_display.brightness.to_string()),
                ("ColorMode", color_depth_name(self.color_depth).to_owned()),
            ],
        )?;
        console.save_config_value(
            CONFIG,
            WINDOWS_CLIENT_SECTION,
            "ScreenFlashes",
            self.screen_flashes.to_string(),
        )?;
        console.save_config_values(
            CONFIG,
            MODERN_SECTION,
            &[
                (
                    "ToneMapper",
                    tone_mapper_name(self.renderer.tone_mapper).to_owned(),
                ),
                (
                    "ReinhardBrightness",
                    self.modern_reinhard_display.brightness.to_string(),
                ),
                (
                    "ReinhardContrast",
                    self.modern_reinhard_display.contrast.to_string(),
                ),
                (
                    "ACESBrightness",
                    self.modern_aces_display.brightness.to_string(),
                ),
                (
                    "ACESContrast",
                    self.modern_aces_display.contrast.to_string(),
                ),
                (
                    "AgXBrightness",
                    self.modern_agx_display.brightness.to_string(),
                ),
                ("AgXContrast", self.modern_agx_display.contrast.to_string()),
                (
                    "AmbientOcclusion",
                    ambient_occlusion_name(self.renderer.ambient_occlusion).to_owned(),
                ),
                (
                    "AntiAliasing",
                    self.renderer.antialiasing.label().to_owned(),
                ),
                ("Bloom", self.renderer.bloom.to_string()),
                (
                    "VolumetricLighting",
                    self.renderer.volumetric_lighting.to_string(),
                ),
            ],
        )?;
        console.remove_config_section(CONFIG, LEGACY_SECTION)
    }
}

pub(super) fn window_size(scene: &LoadedScene) -> [u32; 2] {
    let width = scene
        .config_value_in(CONFIG, RENDERER_SECTION, "WindowSizeX")
        .and_then(|value| dimension(&value));
    let height = scene
        .config_value_in(CONFIG, RENDERER_SECTION, "WindowSizeY")
        .and_then(|value| dimension(&value));
    resolution(width, height).unwrap_or(DEFAULT_WINDOW_SIZE)
}

pub(super) const fn renderer_name(mode: RendererMode) -> &'static str {
    match mode {
        RendererMode::Classic => "Classic",
        RendererMode::Modern => "Modern",
    }
}

pub(super) const fn tone_mapper_name(tone_mapper: ToneMapper) -> &'static str {
    match tone_mapper {
        ToneMapper::AgX => "AgX",
        ToneMapper::Reinhard => "Reinhard",
        ToneMapper::Aces => "ACES",
    }
}

fn ambient_occlusion_name(ambient_occlusion: AmbientOcclusion) -> &'static str {
    match ambient_occlusion {
        AmbientOcclusion::Off => "Off",
        AmbientOcclusion::Ssao => "SSAO",
        AmbientOcclusion::XeGtao => "XeGTAO",
    }
}

fn color_depth_name(color_depth: ColorDepth) -> &'static str {
    match color_depth {
        ColorDepth::TrueColor => "32Bit",
        ColorDepth::Rgb565 => "RGB565",
    }
}

fn color_depth(value: &str) -> Option<ColorDepth> {
    if ["32", "32bit", "truecolor", "rgba8888"]
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
    {
        Some(ColorDepth::TrueColor)
    } else if ["16", "16bit", "rgb565"]
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
    {
        Some(ColorDepth::Rgb565)
    } else {
        None
    }
}

fn config(console: &ConsoleCommands, section: &str, key: &str, legacy_key: &str) -> Option<String> {
    console
        .config_value(CONFIG, section, key)
        .or_else(|| console.config_value(CONFIG, LEGACY_SECTION, legacy_key))
}

fn dimension(value: &str) -> Option<u32> {
    value
        .parse()
        .ok()
        .filter(|value| (320..=8192).contains(value))
}

fn resolution(width: Option<u32>, height: Option<u32>) -> Option<[u32; 2]> {
    width
        .zip(height)
        .filter(|(width, height)| u64::from(*width) * u64::from(*height) <= MAX_RENDER_PIXELS)
        .map(|(width, height)| [width, height])
}

fn setting(
    console: &ConsoleCommands,
    section: &str,
    key: &str,
    legacy_key: &str,
    default: f32,
    min: f32,
    max: f32,
) -> f32 {
    setting_value(config(console, section, key, legacy_key), default, min, max)
}

fn modern_display(
    console: &ConsoleCommands,
    selected: ToneMapper,
    tone_mapper: ToneMapper,
) -> DisplaySettings {
    let defaults = DisplaySettings::for_tone_mapper(tone_mapper);
    let value = |suffix, legacy_key| {
        console
            .config_value(
                CONFIG,
                MODERN_SECTION,
                &format!("{}{suffix}", tone_mapper_name(tone_mapper)),
            )
            .or_else(|| {
                (selected == tone_mapper)
                    .then(|| config(console, MODERN_SECTION, suffix, legacy_key))
                    .flatten()
            })
    };
    DisplaySettings {
        brightness: setting_value(
            value("Brightness", "ModernBrightness"),
            defaults.brightness,
            0.2,
            1.0,
        ),
        contrast: setting_value(
            value("Contrast", "ModernContrast"),
            defaults.contrast,
            0.5,
            2.0,
        ),
    }
}

fn setting_value(value: Option<String>, default: f32, min: f32, max: f32) -> f32 {
    value
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map_or(default, |value| value.clamp(min, max))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use openhp1_render::Antialiasing;

    use super::*;

    #[test]
    fn defaults_match_the_classic_game_presentation() {
        assert_eq!(
            GraphicsSettings::default(),
            GraphicsSettings {
                resolution: [1024, 768],
                window_size: [1280, 800],
                renderer: RendererSettings::default(),
                color_depth: ColorDepth::TrueColor,
                screen_flashes: true,
                classic_display: DisplaySettings {
                    brightness: 0.6,
                    contrast: 1.0,
                },
                modern_agx_display: DisplaySettings::for_tone_mapper(ToneMapper::AgX),
                modern_reinhard_display: DisplaySettings::for_tone_mapper(ToneMapper::Reinhard),
                modern_aces_display: DisplaySettings::for_tone_mapper(ToneMapper::Aces),
            }
        );
    }

    #[test]
    fn tone_mappers_keep_independent_display_values() {
        let mut settings = GraphicsSettings::default();
        settings.renderer.mode = RendererMode::Modern;
        settings.modern_display_mut().brightness = 0.42;

        settings.renderer.tone_mapper = ToneMapper::AgX;
        assert_eq!(
            settings.display(),
            DisplaySettings::for_tone_mapper(ToneMapper::AgX)
        );
        settings.renderer.tone_mapper = ToneMapper::Reinhard;
        assert_eq!(settings.display().brightness, 0.42);
    }

    #[test]
    fn color_modes_accept_readable_and_legacy_spellings() {
        assert_eq!(color_depth("32Bit"), Some(ColorDepth::TrueColor));
        assert_eq!(color_depth("TRUECOLOR"), Some(ColorDepth::TrueColor));
        assert_eq!(color_depth("rGb565"), Some(ColorDepth::Rgb565));
        assert_eq!(color_depth("unknown"), None);
    }

    #[test]
    fn disabled_screen_flashes_supply_draw_time_identity() {
        let runtime_state = [0.25, 0.5, 0.75, 0.125];
        let mut settings = GraphicsSettings::default();
        settings.screen_flashes = false;

        assert_eq!(settings.visible_flash(runtime_state), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(runtime_state, [0.25, 0.5, 0.75, 0.125]);
    }

    #[test]
    fn writes_canonical_modern_effect_names() {
        assert_eq!(ambient_occlusion_name(AmbientOcclusion::XeGtao), "XeGTAO");
        assert_eq!(Antialiasing::Smaa.label(), "SMAA");
    }

    #[test]
    fn dimensions_reject_invalid_or_unsafe_render_targets() {
        assert_eq!(dimension("1920"), Some(1920));
        assert_eq!(dimension("0"), None);
        assert_eq!(dimension("16384"), None);
        assert_eq!(dimension("wide"), None);
        assert_eq!(resolution(Some(3840), Some(2160)), Some([3840, 2160]));
        assert_eq!(resolution(Some(8192), Some(8192)), None);
    }

    #[test]
    fn presets_are_unique_and_have_expected_aspects() {
        let mut sizes = RESOLUTION_PRESETS.map(|(size, _)| size);
        sizes.sort_unstable();
        assert!(sizes.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(RESOLUTION_PRESETS.iter().all(|([width, height], label)| {
            (width * 3 == height * 4 || width * 9 == height * 16) && label.contains('x')
        }));
    }
}
