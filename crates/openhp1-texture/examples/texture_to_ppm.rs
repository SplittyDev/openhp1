use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use openhp1_package::{ObjectReference, Package};
use openhp1_texture::{Palette, Texture};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let package_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: texture_to_ppm <package> <texture-name> <output.ppm>")?;
    let texture_name = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or("texture name must be valid UTF-8")?;
    let output_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("missing output path")?;

    let package = Package::open(&package_path)?;
    let export_index = package
        .summary()
        .exports
        .iter()
        .position(|export| {
            package.summary().name(export.object_name) == texture_name
                && matches!(
                    package.summary().class_name(export),
                    Some("Texture" | "WetTexture" | "FireTexture")
                )
        })
        .ok_or("texture export not found")?;
    let texture = Texture::decode(&package, export_index)?;
    let ObjectReference::Export(palette_index) = texture.palette else {
        return Err("this minimal exporter requires a palette in the same package".into());
    };
    let palette = Palette::decode(&package, palette_index)?;
    write_ppm(&output_path, &texture, &palette)?;
    println!("wrote {}", output_path.display());
    Ok(())
}

fn write_ppm(path: &Path, texture: &Texture, palette: &Palette) -> Result<(), Box<dyn Error>> {
    let mip = texture.mips.first().ok_or("texture has no mipmaps")?;
    let rgba = texture.rgba(0, palette, false)?;
    let mut ppm = format!("P6\n{} {}\n255\n", mip.width, mip.height).into_bytes();
    for pixel in rgba.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    fs::write(path, ppm)?;
    Ok(())
}
