use anyhow::{Context, Result, ensure};
use image::ImageFormat;
use openhp1_texture::window_mask_filename;

use crate::TransmissionMask;

const MASKS: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/window_masks.rs"));

pub(super) fn decode(name: &str, expected_size: [u32; 2]) -> Result<Option<TransmissionMask>> {
    let filename = window_mask_filename(name);
    let Some((_, bytes)) = MASKS
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&filename))
    else {
        return Ok(None);
    };
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .with_context(|| format!("decoding embedded window mask {name}"))?
        .into_luma8();
    ensure!(
        [image.width(), image.height()] == expected_size,
        "window mask {name} is {}x{}, expected {}x{}",
        image.width(),
        image.height(),
        expected_size[0],
        expected_size[1]
    );
    Ok(Some(TransmissionMask {
        width: image.width(),
        height: image.height(),
        values: image.into_raw(),
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_masks_are_found_case_insensitively() {
        let mask = super::decode("hp_1st.windows.win15_a_1", [256, 256])
            .unwrap()
            .unwrap();
        assert_eq!(mask.values.len(), 256 * 256);
    }
}
