use openhp1_package::Package;

use crate::{
    Error, Result,
    decode::{nonnegative, require_class},
};

/// A palette color converted from Unreal's serialized BGRA byte order.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    /// UE1 palettes usually leave this byte at zero. Surface masking is a
    /// material property, so callers should not treat zero as transparency.
    pub alpha: u8,
}

impl Color {
    fn from_bgra(bgra: &[u8]) -> Self {
        Self {
            red: bgra[2],
            green: bgra[1],
            blue: bgra[0],
            alpha: bgra[3],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Palette {
    pub colors: Vec<Color>,
}

impl Palette {
    pub fn decode(package: &Package, export_index: usize) -> Result<Self> {
        require_class(package, export_index, "Palette")?;
        let mut reader = package.export_reader(export_index)?;
        while reader.next_property()?.is_some() {}

        let count_offset = reader.absolute_position();
        let count = nonnegative(reader.read_compact_index()?, count_offset, "palette colors")?;
        let bytes = reader.read_bytes(
            count
                .checked_mul(4)
                .ok_or(Error::InvalidPaletteCount { count })?,
        )?;
        Ok(Self {
            colors: bytes.chunks_exact(4).map(Color::from_bgra).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn serialized_palette_color_is_bgra() {
        assert_eq!(
            Color::from_bgra(&[1, 2, 3, 4]),
            Color {
                red: 3,
                green: 2,
                blue: 1,
                alpha: 4
            }
        );
    }
}
