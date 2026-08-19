use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use image::ImageFormat;
use openhp1_package::{PackageStore, ResolvedObject};
use tracing::warn;

use crate::{TextureImage, TextureMipImage};

const DDS_HEADER_LEN: usize = 128;

#[derive(Clone, Copy, PartialEq)]
enum DxtFormat {
    Dxt1,
    Dxt5,
}

/// Loads a DirectX 11 renderer-style DDS replacement when one is installed.
/// Missing, unsupported, and malformed replacements fall back to the package texture.
pub fn load_texture_override(
    packages: &PackageStore,
    object: &ResolvedObject,
    logical_dimensions: [u32; 2],
) -> Option<TextureImage> {
    let name = match PackageStore::qualified_object_name(object) {
        Ok(name) => name,
        Err(error) => {
            warn!(%error, "could not name texture for DDS override lookup");
            return None;
        }
    };
    let path = find_override(packages.game_root(), &name)?;
    match fs::read(&path)
        .with_context(|| format!("could not read `{}`", path.display()))
        .and_then(|bytes| decode_dds(&bytes))
    {
        Ok(mut image) => {
            image.logical_width = logical_dimensions[0];
            image.logical_height = logical_dimensions[1];
            Some(image)
        }
        Err(error) => {
            warn!(texture = name, path = %path.display(), %error, "could not decode DDS texture override; using packaged texture");
            None
        }
    }
}

fn find_override(game_root: &Path, qualified_name: &str) -> Option<PathBuf> {
    let mut parts = qualified_name.split('.').peekable();
    let package = parts.next()?;
    let mut directory = find_child(game_root, "Textures", true)?;
    directory = find_child(&directory, package, true)?;
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            directory = find_child(&directory, part, true)?;
        } else {
            return find_child(&directory, &format!("{part}.dds"), false);
        }
    }
    None
}

fn find_child(directory: &Path, name: &str, want_directory: bool) -> Option<PathBuf> {
    let exact = directory.join(name);
    if exact
        .metadata()
        .is_ok_and(|metadata| metadata.is_dir() == want_directory)
    {
        return Some(exact);
    }
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() == want_directory)
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
        })
        .map(|entry| entry.path())
}

fn decode_dds(bytes: &[u8]) -> Result<TextureImage> {
    ensure!(bytes.len() >= DDS_HEADER_LEN, "DDS header is truncated");
    ensure!(&bytes[..4] == b"DDS ", "DDS signature is missing");
    ensure!(read_u32(bytes, 4)? == 124, "DDS header size is invalid");
    ensure!(
        read_u32(bytes, 76)? == 32,
        "DDS pixel format size is invalid"
    );
    ensure!(
        read_u32(bytes, 112)? == 0,
        "DDS arrays and cube maps are unsupported"
    );

    let mut width = read_u32(bytes, 16)?;
    let mut height = read_u32(bytes, 12)?;
    ensure!(width != 0 && height != 0, "DDS dimensions are zero");
    let mip_count = read_u32(bytes, 28)?.max(1);
    ensure!(mip_count <= 32, "DDS mip count is invalid");
    let format = match bytes.get(84..88) {
        Some(b"DXT1") => DxtFormat::Dxt1,
        Some(b"DXT5") => DxtFormat::Dxt5,
        Some(value) => bail!(
            "unsupported DDS format {:?}",
            String::from_utf8_lossy(value)
        ),
        None => bail!("DDS pixel format is truncated"),
    };
    let block_bytes = if format == DxtFormat::Dxt1 { 8 } else { 16 };
    let mut offset = DDS_HEADER_LEN;
    let mut levels = Vec::new();

    for _ in 0..mip_count {
        let blocks_wide = width.div_ceil(4);
        let blocks_high = height.div_ceil(4);
        let encoded_len = usize::try_from(blocks_wide)
            .ok()
            .and_then(|wide| {
                usize::try_from(blocks_high)
                    .ok()
                    .and_then(|high| wide.checked_mul(high))
            })
            .and_then(|blocks| blocks.checked_mul(block_bytes))
            .context("DDS mip size overflows")?;
        let end = offset
            .checked_add(encoded_len)
            .context("DDS mip offset overflows")?;
        let encoded = bytes
            .get(offset..end)
            .context("DDS mip data is truncated")?;
        levels.push(decode_level(
            &bytes[..DDS_HEADER_LEN],
            encoded,
            width,
            height,
            format,
        )?);
        offset = end;
        if width == 1 && height == 1 {
            break;
        }
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }

    let first = levels.first().context("DDS has no image levels")?;
    Ok(TextureImage {
        width: first.width,
        height: first.height,
        logical_width: first.width,
        logical_height: first.height,
        rgba: first.rgba.clone(),
        mips: levels.into_iter().skip(1).collect(),
    })
}

fn decode_level(
    header: &[u8],
    encoded: &[u8],
    width: u32,
    height: u32,
    format: DxtFormat,
) -> Result<TextureMipImage> {
    let padded_width = width
        .max(4)
        .div_ceil(4)
        .checked_mul(4)
        .context("DDS padded width overflows")?;
    let padded_height = height
        .max(4)
        .div_ceil(4)
        .checked_mul(4)
        .context("DDS padded height overflows")?;
    let mut dds = header.to_vec();
    write_u32(&mut dds, 12, padded_height)?;
    write_u32(&mut dds, 16, padded_width)?;
    write_u32(
        &mut dds,
        20,
        u32::try_from(encoded.len()).context("DDS mip is too large")?,
    )?;
    write_u32(&mut dds, 28, 1)?;
    dds.extend_from_slice(encoded);

    let decoded = image::load_from_memory_with_format(&dds, ImageFormat::Dds)
        .context("DDS block decompression failed")?
        .to_rgba8();
    ensure!(
        decoded.width() == padded_width && decoded.height() == padded_height,
        "DDS decoder returned unexpected dimensions"
    );
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .context("DDS row size overflows")?;
    let mut rgba = Vec::with_capacity(
        row_bytes
            .checked_mul(usize::try_from(height).context("DDS height overflows")?)
            .context("DDS image size overflows")?,
    );
    for row in decoded
        .as_raw()
        .chunks_exact(
            usize::try_from(padded_width)
                .ok()
                .and_then(|width| width.checked_mul(4))
                .context("DDS padded row size overflows")?,
        )
        .take(usize::try_from(height).context("DDS height overflows")?)
    {
        rgba.extend_from_slice(&row[..row_bytes]);
    }
    if format == DxtFormat::Dxt1 {
        apply_dxt1_alpha(encoded, width, height, &mut rgba)?;
    }
    Ok(TextureMipImage {
        width,
        height,
        rgba,
    })
}

fn apply_dxt1_alpha(encoded: &[u8], width: u32, height: u32, rgba: &mut [u8]) -> Result<()> {
    let blocks_wide = width.div_ceil(4);
    for (block, bytes) in encoded.chunks_exact(8).enumerate() {
        let color_0 = u16::from_le_bytes([bytes[0], bytes[1]]);
        let color_1 = u16::from_le_bytes([bytes[2], bytes[3]]);
        if color_0 > color_1 {
            continue;
        }
        let selectors = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let block = u32::try_from(block).context("DDS block index overflows")?;
        let block_x = (block % blocks_wide)
            .checked_mul(4)
            .context("DDS block coordinate overflows")?;
        let block_y = (block / blocks_wide)
            .checked_mul(4)
            .context("DDS block coordinate overflows")?;
        for y in 0..4 {
            for x in 0..4 {
                if block_x + x >= width || block_y + y >= height {
                    continue;
                }
                let selector = (selectors >> (2 * (y * 4 + x))) & 3;
                if selector == 3 {
                    let pixel = u64::from(block_y + y)
                        .checked_mul(u64::from(width))
                        .and_then(|row| row.checked_add(u64::from(block_x + x)))
                        .and_then(|pixel| usize::try_from(pixel).ok())
                        .and_then(|pixel| pixel.checked_mul(4))
                        .context("DDS pixel offset overflows")?;
                    *rgba
                        .get_mut(pixel + 3)
                        .context("DDS pixel offset is outside decoded image")? = 0;
                }
            }
        }
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("DDS header field is truncated")?
            .try_into()
            .unwrap(),
    ))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<()> {
    bytes
        .get_mut(offset..offset + 4)
        .context("DDS header field is truncated")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dds(fourcc: &[u8; 4], width: u32, height: u32, mip_count: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; DDS_HEADER_LEN];
        bytes[..4].copy_from_slice(b"DDS ");
        write_u32(&mut bytes, 4, 124).unwrap();
        write_u32(&mut bytes, 8, 0x0002_1007).unwrap();
        write_u32(&mut bytes, 12, height).unwrap();
        write_u32(&mut bytes, 16, width).unwrap();
        write_u32(&mut bytes, 28, mip_count).unwrap();
        write_u32(&mut bytes, 76, 32).unwrap();
        write_u32(&mut bytes, 80, 4).unwrap();
        bytes[84..88].copy_from_slice(fourcc);
        write_u32(&mut bytes, 108, 0x0040_1008).unwrap();
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn decodes_all_dxt_mips_and_dxt1_transparency() {
        let opaque_red = [0x00, 0xf8, 0xe0, 0x07, 0, 0, 0, 0];
        let transparent = [0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let mut data = opaque_red.repeat(4);
        data.extend_from_slice(&transparent);
        let image = decode_dds(&dds(b"DXT1", 8, 8, 2, &data)).unwrap();

        assert_eq!((image.width, image.height, image.mips.len()), (8, 8, 1));
        assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!((image.mips[0].width, image.mips[0].height), (4, 4));
        assert!(
            image.mips[0]
                .rgba
                .chunks_exact(4)
                .all(|pixel| pixel[3] == 0)
        );
    }

    #[test]
    fn rejects_malformed_replacements() {
        assert!(decode_dds(b"not a DDS file").is_err());
    }

    #[test]
    fn finds_renderer_style_paths_case_insensitively() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openhp1-dds-override-{unique}"));
        let directory = root.join("textures").join("tUt1").join("sKins");
        fs::create_dir_all(&directory).unwrap();
        let expected = directory.join("SKBARRELTEX0.DDS");
        fs::write(&expected, []).unwrap();

        assert_eq!(
            find_override(&root, "Tut1.Skins.skbarrelTex0")
                .and_then(|path| path.canonicalize().ok()),
            expected.canonicalize().ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
