use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RendererMode {
    #[default]
    Classic,
    Modern,
}

impl FromStr for RendererMode {
    type Err = RendererSettingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "classic" => Ok(Self::Classic),
            "modern" => Ok(Self::Modern),
            _ => Err(RendererSettingError::new(
                "renderer",
                value,
                "classic, modern",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToneMapper {
    AgX,
    #[default]
    Reinhard,
    Aces,
}

impl FromStr for ToneMapper {
    type Err = RendererSettingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "agx" => Ok(Self::AgX),
            "reinhard" | "classic" => Ok(Self::Reinhard),
            "aces" => Ok(Self::Aces),
            _ => Err(RendererSettingError::new(
                "tone mapper",
                value,
                "agx, reinhard, aces",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AmbientOcclusion {
    Off,
    Ssao,
    #[default]
    XeGtao,
}

impl FromStr for AmbientOcclusion {
    type Err = RendererSettingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "ssao" => Ok(Self::Ssao),
            "xegtao" | "gtao" => Ok(Self::XeGtao),
            _ => Err(RendererSettingError::new(
                "ambient occlusion",
                value,
                "off, ssao, xegtao",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Antialiasing {
    Off,
    Fxaa,
    #[default]
    Smaa,
}

impl Antialiasing {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
        }
    }
}

impl FromStr for Antialiasing {
    type Err = RendererSettingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "fxaa" => Ok(Self::Fxaa),
            "smaa" => Ok(Self::Smaa),
            _ => Err(RendererSettingError::new(
                "anti-aliasing",
                value,
                "off, fxaa, smaa",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplaySettings {
    pub brightness: f32,
    pub contrast: f32,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self::for_mode(RendererMode::Classic)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumetricTuning {
    pub debug_view: VolumetricDebugView,
    pub shaft_intensity: f32,
    pub shaft_tilt_degrees: f32,
    pub shaft_saturation: f32,
    pub shaft_anisotropy: f32,
    pub shaft_projection: f32,
    pub dust_size: f32,
    pub dust_density: u32,
    pub dust_opacity: f32,
    pub dust_speed: f32,
    pub haze_size: f32,
    pub haze_density: f32,
    pub haze_opacity: f32,
    pub haze_speed: f32,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VolumetricDebugView {
    #[default]
    Composite,
    Scattering,
    ApertureMask,
    DirectionalVisibility,
    LocalVisibility,
}

impl VolumetricDebugView {
    pub const ALL: [Self; 5] = [
        Self::Composite,
        Self::Scattering,
        Self::ApertureMask,
        Self::DirectionalVisibility,
        Self::LocalVisibility,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Composite => "Composite",
            Self::Scattering => "Scattering only",
            Self::ApertureMask => "Window mask",
            Self::DirectionalVisibility => "Directional visibility",
            Self::LocalVisibility => "Local-light visibility",
        }
    }

    pub(crate) const fn shader_id(self) -> u32 {
        self as u32
    }
}

impl Default for VolumetricTuning {
    fn default() -> Self {
        Self {
            debug_view: VolumetricDebugView::Composite,
            shaft_intensity: 1.0,
            shaft_tilt_degrees: 20.0,
            shaft_saturation: 1.5,
            shaft_anisotropy: 0.4,
            shaft_projection: 0.005,
            dust_size: 2.0,
            dust_density: 128,
            dust_opacity: 0.5,
            dust_speed: 20.0,
            haze_size: 30.0,
            haze_density: 2.0,
            haze_opacity: 1.0,
            haze_speed: 25.0,
        }
    }
}

impl DisplaySettings {
    pub const fn for_mode(mode: RendererMode) -> Self {
        match mode {
            RendererMode::Classic => Self {
                brightness: 0.5,
                contrast: 1.0,
            },
            RendererMode::Modern => Self::for_tone_mapper(ToneMapper::Reinhard),
        }
    }

    pub const fn for_tone_mapper(tone_mapper: ToneMapper) -> Self {
        match tone_mapper {
            ToneMapper::AgX => Self {
                brightness: 0.6,
                contrast: 0.9,
            },
            ToneMapper::Reinhard => Self {
                brightness: 0.66,
                contrast: 1.05,
            },
            ToneMapper::Aces => Self {
                brightness: 0.64,
                contrast: 0.75,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererSettings {
    pub mode: RendererMode,
    /// Enables the original three-band `UTexture::DetailTexture` pass.
    pub detail_textures: bool,
    /// Enables the high-resolution PC CRT presentation effect in Classic mode.
    pub crt_effect: bool,
    pub tone_mapper: ToneMapper,
    pub ambient_occlusion: AmbientOcclusion,
    pub antialiasing: Antialiasing,
    pub bloom: bool,
    pub volumetric_lighting: bool,
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            mode: RendererMode::Classic,
            detail_textures: false,
            crt_effect: false,
            tone_mapper: ToneMapper::default(),
            ambient_occlusion: AmbientOcclusion::default(),
            antialiasing: Antialiasing::Smaa,
            bloom: false,
            volumetric_lighting: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererSettingError {
    setting: &'static str,
    value: String,
    expected: &'static str,
}

impl RendererSettingError {
    fn new(setting: &'static str, value: &str, expected: &'static str) -> Self {
        Self {
            setting,
            value: value.to_owned(),
            expected,
        }
    }
}

impl fmt::Display for RendererSettingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported {} {:?}; expected one of {}",
            self.setting, self.value, self.expected
        )
    }
}

impl std::error::Error for RendererSettingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_renderer_choices() {
        assert_eq!(ToneMapper::default(), ToneMapper::Reinhard);
        assert_eq!(AmbientOcclusion::default(), AmbientOcclusion::XeGtao);
        assert_eq!(
            RendererSettings::default().tone_mapper,
            ToneMapper::Reinhard
        );
        assert_eq!(
            RendererSettings::default().ambient_occlusion,
            AmbientOcclusion::XeGtao
        );
        assert!(!RendererSettings::default().bloom);
        assert!(!RendererSettings::default().volumetric_lighting);
        assert!(!RendererSettings::default().detail_textures);
        assert!(!RendererSettings::default().crt_effect);
        assert_eq!(
            VolumetricTuning::default(),
            VolumetricTuning {
                debug_view: VolumetricDebugView::Composite,
                shaft_intensity: 1.0,
                shaft_tilt_degrees: 20.0,
                shaft_saturation: 1.5,
                shaft_anisotropy: 0.4,
                shaft_projection: 0.005,
                dust_size: 2.0,
                dust_density: 128,
                dust_opacity: 0.5,
                dust_speed: 20.0,
                haze_size: 30.0,
                haze_density: 2.0,
                haze_opacity: 1.0,
                haze_speed: 25.0,
            }
        );
        assert_eq!(
            VolumetricDebugView::ALL.map(VolumetricDebugView::shader_id),
            [0, 1, 2, 3, 4]
        );
        assert_eq!("modern".parse(), Ok(RendererMode::Modern));
        assert_eq!("cLaSsIc".parse(), Ok(RendererMode::Classic));
        assert_eq!("agx".parse(), Ok(ToneMapper::AgX));
        assert_eq!("ReInHaRd".parse(), Ok(ToneMapper::Reinhard));
        assert_eq!("classic".parse(), Ok(ToneMapper::Reinhard));
        assert_eq!("aces".parse(), Ok(ToneMapper::Aces));
        assert_eq!("off".parse(), Ok(AmbientOcclusion::Off));
        assert_eq!("sSaO".parse(), Ok(AmbientOcclusion::Ssao));
        assert_eq!("XeGTAO".parse(), Ok(AmbientOcclusion::XeGtao));
        assert_eq!("gtao".parse(), Ok(AmbientOcclusion::XeGtao));
        assert_eq!(Antialiasing::default(), Antialiasing::Smaa);
        assert_eq!("off".parse(), Ok(Antialiasing::Off));
        assert_eq!("FxAa".parse(), Ok(Antialiasing::Fxaa));
        assert_eq!("SMAA".parse(), Ok(Antialiasing::Smaa));
        assert_eq!(Antialiasing::Smaa.label(), "SMAA");
        assert!("taa".parse::<Antialiasing>().is_err());
        assert!("filmic".parse::<ToneMapper>().is_err());
    }

    #[test]
    fn keeps_independent_display_defaults_for_each_pipeline_and_tone_mapper() {
        assert_eq!(
            DisplaySettings::for_mode(RendererMode::Classic),
            DisplaySettings {
                brightness: 0.5,
                contrast: 1.0,
            }
        );
        assert_eq!(
            DisplaySettings::for_mode(RendererMode::Classic),
            DisplaySettings::default()
        );
        assert_eq!(
            DisplaySettings::for_mode(RendererMode::Modern),
            DisplaySettings {
                brightness: 0.66,
                contrast: 1.05,
            }
        );
        assert_eq!(
            DisplaySettings::for_tone_mapper(ToneMapper::AgX),
            DisplaySettings {
                brightness: 0.6,
                contrast: 0.9,
            }
        );
        assert_eq!(
            DisplaySettings::for_tone_mapper(ToneMapper::Aces),
            DisplaySettings {
                brightness: 0.64,
                contrast: 0.75,
            }
        );
    }
}
