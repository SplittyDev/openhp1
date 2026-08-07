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
    #[default]
    Ssao,
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

impl DisplaySettings {
    pub const fn for_mode(mode: RendererMode) -> Self {
        match mode {
            RendererMode::Classic => Self {
                brightness: 0.6,
                contrast: 1.0,
            },
            RendererMode::Modern => Self {
                brightness: 0.33,
                contrast: 1.24,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererSettings {
    pub mode: RendererMode,
    pub tone_mapper: ToneMapper,
    pub ambient_occlusion: AmbientOcclusion,
    pub bloom: bool,
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            mode: RendererMode::Classic,
            tone_mapper: ToneMapper::default(),
            ambient_occlusion: AmbientOcclusion::Ssao,
            bloom: true,
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
        assert_eq!(
            RendererSettings::default().tone_mapper,
            ToneMapper::Reinhard
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
        assert!("filmic".parse::<ToneMapper>().is_err());
    }

    #[test]
    fn keeps_independent_display_defaults_for_each_mode() {
        assert_eq!(
            DisplaySettings::for_mode(RendererMode::Classic),
            DisplaySettings {
                brightness: 0.6,
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
                brightness: 0.33,
                contrast: 1.24,
            }
        );
    }
}
