use std::io;

use openhp1_render::{
    AmbientOcclusion, DisplaySettings, RendererMode, RendererSettings, ToneMapper,
};
use openhp1_runtime::ConsoleCommands;

const CONFIG: &str = "OpenHP1";
const SECTION: &str = "OpenHP1.Graphics";
const MAX_RENDER_PIXELS: u64 = 3840 * 2160;

pub(super) const DEFAULT_RESOLUTION: [u32; 2] = [1024, 768];

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
    pub(super) renderer: RendererSettings,
    pub(super) color_depth: ColorDepth,
    pub(super) classic_display: DisplaySettings,
    pub(super) modern_display: DisplaySettings,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_RESOLUTION,
            renderer: RendererSettings::default(),
            color_depth: ColorDepth::default(),
            classic_display: DisplaySettings::for_mode(RendererMode::Classic),
            modern_display: DisplaySettings::for_mode(RendererMode::Modern),
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
        renderer.mode = config(console, "Renderer")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.mode);
        renderer.tone_mapper = config(console, "ToneMapper")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.tone_mapper);
        renderer.ambient_occlusion = config(console, "AmbientOcclusion")
            .and_then(|value| value.parse().ok())
            .unwrap_or(renderer.ambient_occlusion);
        renderer.bloom = config(console, "Bloom")
            .and_then(|value| parse_bool(&value))
            .unwrap_or(renderer.bloom);
        if let Some(override_settings) = renderer_override {
            renderer = override_settings;
        }

        let width = config(console, "ResolutionX").and_then(|value| dimension(&value));
        let height = config(console, "ResolutionY").and_then(|value| dimension(&value));
        let classic = DisplaySettings::for_mode(RendererMode::Classic);
        let modern = DisplaySettings::for_mode(RendererMode::Modern);
        Self {
            resolution: resolution(width, height).unwrap_or(defaults.resolution),
            renderer,
            color_depth: match config(console, "ColorDepth").as_deref() {
                Some("rgb565") => ColorDepth::Rgb565,
                _ => ColorDepth::TrueColor,
            },
            classic_display: DisplaySettings {
                brightness: setting(console, "ClassicBrightness", classic.brightness, 0.2, 1.0),
                ..classic
            },
            modern_display: DisplaySettings {
                brightness: setting(console, "ModernBrightness", modern.brightness, 0.2, 1.0),
                contrast: setting(console, "ModernContrast", modern.contrast, 0.5, 2.0),
            },
        }
    }

    pub(super) fn display(self) -> DisplaySettings {
        match self.renderer.mode {
            RendererMode::Classic => self.classic_display,
            RendererMode::Modern => self.modern_display,
        }
    }

    pub(super) fn save(self, console: &ConsoleCommands) -> io::Result<()> {
        console.save_config_values(
            CONFIG,
            SECTION,
            &[
                ("ResolutionX", self.resolution[0].to_string()),
                ("ResolutionY", self.resolution[1].to_string()),
                ("Renderer", renderer_name(self.renderer.mode).to_owned()),
                (
                    "ToneMapper",
                    tone_mapper_name(self.renderer.tone_mapper).to_owned(),
                ),
                (
                    "AmbientOcclusion",
                    ambient_occlusion_name(self.renderer.ambient_occlusion).to_owned(),
                ),
                ("Bloom", self.renderer.bloom.to_string()),
                (
                    "ColorDepth",
                    match self.color_depth {
                        ColorDepth::TrueColor => "truecolor",
                        ColorDepth::Rgb565 => "rgb565",
                    }
                    .to_owned(),
                ),
                (
                    "ClassicBrightness",
                    self.classic_display.brightness.to_string(),
                ),
                (
                    "ModernBrightness",
                    self.modern_display.brightness.to_string(),
                ),
                ("ModernContrast", self.modern_display.contrast.to_string()),
            ],
        )
    }
}

pub(super) const fn renderer_name(mode: RendererMode) -> &'static str {
    match mode {
        RendererMode::Classic => "classic",
        RendererMode::Modern => "modern",
    }
}

pub(super) const fn tone_mapper_name(tone_mapper: ToneMapper) -> &'static str {
    match tone_mapper {
        ToneMapper::AgX => "agx",
        ToneMapper::Reinhard => "reinhard",
        ToneMapper::Aces => "aces",
    }
}

fn ambient_occlusion_name(ambient_occlusion: AmbientOcclusion) -> &'static str {
    match ambient_occlusion {
        AmbientOcclusion::Off => "off",
        AmbientOcclusion::Ssao => "ssao",
    }
}

fn config(console: &ConsoleCommands, key: &str) -> Option<String> {
    console.config_value(CONFIG, SECTION, key)
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

fn setting(console: &ConsoleCommands, key: &str, default: f32, min: f32, max: f32) -> f32 {
    config(console, key)
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
    use super::*;

    #[test]
    fn defaults_match_the_classic_game_presentation() {
        assert_eq!(
            GraphicsSettings::default(),
            GraphicsSettings {
                resolution: [1024, 768],
                renderer: RendererSettings::default(),
                color_depth: ColorDepth::TrueColor,
                classic_display: DisplaySettings {
                    brightness: 0.6,
                    contrast: 1.0,
                },
                modern_display: DisplaySettings::for_mode(RendererMode::Modern),
            }
        );
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
