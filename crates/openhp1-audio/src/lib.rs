//! Decoding and playback support for package-backed HP1 audio.

use std::sync::Arc;

use openhp1_package::Package;
use thiserror::Error;

#[cfg(feature = "playback")]
mod playback;

#[cfg(feature = "playback")]
pub use playback::AudioPlayer;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioClip {
    format: Arc<str>,
    data: Arc<[u8]>,
    looping: bool,
}

impl AudioClip {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        let summary = package.summary();
        let export = summary.exports.get(export_index).ok_or_else(|| {
            openhp1_package::Error::InvalidExportIndex {
                package: Arc::clone(&summary.source),
                index: export_index,
                export_count: summary.exports.len(),
            }
        })?;
        let class = summary.class_name(export).unwrap_or("<unknown>");
        let lazy_offset_version = if class.eq_ignore_ascii_case("Sound") {
            63
        } else if class.eq_ignore_ascii_case("Music") {
            62
        } else {
            return Err(Error::UnsupportedClass {
                package: Arc::clone(&summary.source),
                export: export_index,
                class: class.to_owned(),
            });
        };

        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}
        let format = reader.read_name_index("audio format")?;
        if summary.header.version >= lazy_offset_version {
            reader.read_u32()?;
        }
        let size =
            usize::try_from(reader.read_compact_index()?).map_err(|_| Error::NegativeDataSize {
                package: Arc::clone(&summary.source),
                export: export_index,
            })?;
        let data = Arc::from(reader.read_bytes(size)?);
        let looping = class.eq_ignore_ascii_case("Sound")
            && summary.header.version >= 70
            && reader.read_u32()? != 0;
        Ok(Self {
            format: Arc::from(summary.name(format)),
            data,
            looping,
        })
    }

    pub fn format(&self) -> &str {
        &self.format
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn looping(&self) -> bool {
        self.looping
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] openhp1_package::Error),

    #[error("`{package}` export {export} is {class}, expected Sound or Music")]
    UnsupportedClass {
        package: Arc<str>,
        export: usize,
        class: String,
    },

    #[error("`{package}` audio export {export} has a negative data size")]
    NegativeDataSize { package: Arc<str>, export: usize },

    #[cfg(feature = "playback")]
    #[error("audio playback failed: {0}")]
    Playback(String),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openhp1_package::{PACKAGE_MAGIC, Package};

    use super::*;

    #[test]
    fn decodes_sound_format_and_embedded_data() {
        let data = b"RIFFtest";
        let mut payload = vec![0, 4];
        payload.extend(compact_index(data.len() as i32));
        payload.extend(data);
        let package = package(&["None", "Core", "Class", "Sound", "wav"], 61, payload);

        let clip = AudioClip::decode(&package, 0).unwrap();

        assert_eq!(clip.format(), "wav");
        assert_eq!(clip.data(), data);
        assert!(!clip.looping());
    }

    #[test]
    fn decodes_hp1_sound_looping_flag() {
        let data = b"RIFFtest";
        let mut payload = vec![0, 4];
        payload.extend(0_u32.to_le_bytes());
        payload.extend(compact_index(data.len() as i32));
        payload.extend(data);
        payload.extend(1_u32.to_le_bytes());
        let package = package(&["None", "Core", "Class", "Sound", "wav"], 76, payload);

        assert!(AudioClip::decode(&package, 0).unwrap().looping());
    }

    fn package(names: &[&str], version: u16, payload: Vec<u8>) -> Package {
        let mut name_table = Vec::new();
        for name in names {
            if version >= 64 {
                name_table.extend(compact_index((name.len() + 1) as i32));
            }
            name_table.extend(name.as_bytes());
            name_table.push(0);
            name_table.extend(0_u32.to_le_bytes());
        }

        let mut import_table = vec![1, 2];
        import_table.extend(0_i32.to_le_bytes());
        import_table.extend(compact_index(3));

        let header_size = if version < 68 { 44 } else { 56 };
        let name_offset = header_size;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        export.extend(0_i32.to_le_bytes());
        export.extend(compact_index(3));
        export.extend(0_u32.to_le_bytes());
        export.extend(compact_index(payload.len() as i32));
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let encoded = compact_index(payload_offset as i32);
            let next = export_offset + export.len() + encoded.len();
            if next == payload_offset {
                export.extend(encoded);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        bytes.extend(PACKAGE_MAGIC.to_le_bytes());
        bytes.extend(version.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        for value in [names.len(), name_offset, 1, export_offset, 1, import_offset] {
            bytes.extend((value as i32).to_le_bytes());
        }
        if version < 68 {
            bytes.extend(0_i32.to_le_bytes());
            bytes.extend(0_i32.to_le_bytes());
        } else {
            bytes.extend([0; 16]);
            bytes.extend(0_i32.to_le_bytes());
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        bytes.extend(payload);
        Package::parse("test", Arc::from(bytes)).unwrap()
    }

    fn compact_index(value: i32) -> Vec<u8> {
        let mut value = value as u32;
        let mut bytes = vec![value as u8 & 0x3f];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = value as u8 & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }
}
