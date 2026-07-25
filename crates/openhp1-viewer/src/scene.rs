use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result, ensure};
use openhp1_map::{Model, PolyFlags, world_model_export};
use openhp1_package::{PackageStore, ResolvedObject};
use openhp1_render::{RenderScene, SurfaceMaterial, SurfaceMode, TextureImage};
use openhp1_texture::{Palette, Texture, TextureRenderFlags};
use tracing::{info, warn};

pub(crate) struct LoadedScene {
    pub(crate) path: PathBuf,
    pub(crate) render: RenderScene,
    pub(crate) points: usize,
    pub(crate) nodes: usize,
    pub(crate) surfaces: usize,
    pub(crate) textured_surfaces: usize,
}

impl LoadedScene {
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        let game_root = path
            .parent()
            .and_then(|directory| directory.parent())
            .context("map path must be inside the game's Maps directory")?;
        let mut packages =
            PackageStore::scan_game_root(game_root).context("failed to discover game packages")?;
        let package = packages
            .load_path(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let model_export =
            world_model_export(&package).context("failed to find the world model")?;
        let model =
            Model::decode(&package, model_export).context("failed to decode the world model")?;
        let mesh = model.triangulate().context("failed to triangulate BSP")?;
        let (textures, surface_materials) = load_materials(&mut packages, &package, &model);
        let textured_surfaces = surface_materials
            .iter()
            .filter(|material| material.texture.is_some())
            .count();
        info!(
            map = %path.display(),
            points = model.points.len(),
            nodes = model.nodes.len(),
            surfaces = model.surfaces.len(),
            triangles = mesh.indices.len() / 3,
            textures = textures.len(),
            textured_surfaces,
            "loaded map"
        );
        Ok(Self {
            path,
            render: RenderScene {
                mesh,
                textures,
                surface_materials,
            },
            points: model.points.len(),
            nodes: model.nodes.len(),
            surfaces: model.surfaces.len(),
            textured_surfaces,
        })
    }
}

fn load_materials(
    packages: &mut PackageStore,
    map: &std::sync::Arc<openhp1_package::Package>,
    model: &Model,
) -> (Vec<TextureImage>, Vec<SurfaceMaterial>) {
    let mut textures = Vec::new();
    let mut decoded = HashMap::<(String, usize), Option<DecodedTexture>>::new();
    let mut images = HashMap::<(String, usize, bool), usize>::new();
    let mut materials = Vec::with_capacity(model.surfaces.len());

    for (surface_index, surface) in model.surfaces.iter().enumerate() {
        if is_hidden(surface.poly_flags, TextureRenderFlags::default()) {
            materials.push(SurfaceMaterial {
                mode: SurfaceMode::Hidden,
                ..Default::default()
            });
            continue;
        }
        let resolved = match packages.resolve(map, surface.texture) {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                materials.push(surface_material(surface.poly_flags, None, None));
                continue;
            }
            Err(error) => {
                warn!(surface_index, %error, "could not resolve surface texture");
                materials.push(surface_material(surface.poly_flags, None, None));
                continue;
            }
        };
        let key = (
            resolved.package.summary().source.to_string(),
            resolved.export_index,
        );
        if !decoded.contains_key(&key) {
            let texture = match decode_texture(packages, &resolved) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    warn!(surface_index, %error, "could not decode surface texture");
                    None
                }
            };
            decoded.insert(key.clone(), texture);
        }
        let Some(decoded_texture) = decoded.get(&key).and_then(Option::as_ref) else {
            materials.push(surface_material(surface.poly_flags, None, None));
            continue;
        };
        let texture_flags = decoded_texture.texture.render_flags;
        let masked = surface.poly_flags.contains(PolyFlags::MASKED) || texture_flags.masked;
        let image_key = (key.0.clone(), key.1, masked);
        let texture_index = if let Some(index) = images.get(&image_key) {
            *index
        } else {
            let image = match decoded_texture.image(masked) {
                Ok(image) => image,
                Err(error) => {
                    warn!(surface_index, %error, "could not expand surface texture");
                    materials.push(surface_material(
                        surface.poly_flags,
                        None,
                        Some(texture_flags),
                    ));
                    continue;
                }
            };
            let index = textures.len();
            textures.push(image);
            images.insert(image_key, index);
            index
        };
        materials.push(surface_material(
            surface.poly_flags,
            Some(texture_index),
            Some(texture_flags),
        ));
    }

    (textures, materials)
}

fn decode_texture(
    packages: &mut PackageStore,
    resolved: &ResolvedObject,
) -> Result<DecodedTexture> {
    let texture = Texture::decode(&resolved.package, resolved.export_index)?;
    let mip = texture.mips.first().context("texture has no mip levels")?;
    ensure!(
        mip.width != 0 && mip.height != 0,
        "texture mip has zero dimensions"
    );
    let palette = packages
        .resolve(&resolved.package, texture.palette)?
        .context("texture has no palette reference")?;
    let palette = Palette::decode(&palette.package, palette.export_index)?;
    Ok(DecodedTexture { texture, palette })
}

struct DecodedTexture {
    texture: Texture,
    palette: Palette,
}

impl DecodedTexture {
    fn image(&self, masked: bool) -> Result<TextureImage> {
        let mip = self
            .texture
            .mips
            .first()
            .context("texture has no mip levels")?;
        Ok(TextureImage {
            width: mip.width,
            height: mip.height,
            rgba: self.texture.rgba(0, &self.palette, masked)?,
        })
    }
}

fn surface_material(
    flags: PolyFlags,
    texture: Option<usize>,
    texture_flags: Option<TextureRenderFlags>,
) -> SurfaceMaterial {
    let texture_flags = texture_flags.unwrap_or_default();
    SurfaceMaterial {
        texture,
        mode: if is_hidden(flags, texture_flags) {
            SurfaceMode::Hidden
        } else if flags.contains(PolyFlags::MASKED) || texture_flags.masked {
            SurfaceMode::Masked
        } else {
            SurfaceMode::Opaque
        },
        two_sided: flags.contains(PolyFlags::TWO_SIDED) || texture_flags.two_sided,
    }
}

fn is_hidden(flags: PolyFlags, texture_flags: TextureRenderFlags) -> bool {
    flags.contains(PolyFlags::INVISIBLE)
        || flags.contains(PolyFlags::FAKE_BACKDROP)
        || texture_flags.invisible
        || texture_flags.fake_backdrop
}

#[cfg(test)]
mod tests {
    use openhp1_map::PolyFlags;
    use openhp1_render::SurfaceMode;
    use openhp1_texture::TextureRenderFlags;

    #[test]
    fn combines_surface_and_texture_render_flags() {
        let masked = super::surface_material(
            PolyFlags::TWO_SIDED,
            Some(3),
            Some(TextureRenderFlags {
                masked: true,
                ..Default::default()
            }),
        );
        assert_eq!(masked.mode, SurfaceMode::Masked);
        assert!(masked.two_sided);

        let hidden =
            super::surface_material(PolyFlags::FAKE_BACKDROP, Some(1), Some(Default::default()));
        assert_eq!(hidden.mode, SurfaceMode::Hidden);
    }
}
