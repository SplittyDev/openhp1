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
    pub(crate) masked_surfaces: usize,
    pub(crate) translucent_surfaces: usize,
    pub(crate) modulated_surfaces: usize,
    pub(crate) fake_backdrop_surfaces: usize,
    pub(crate) has_sky_zone: bool,
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
        let lightmaps = model
            .lightmap_images(&package)
            .context("failed to reconstruct static lightmaps")?;
        let fake_backdrop_surfaces = model
            .surfaces
            .iter()
            .filter(|surface| surface.poly_flags.contains(PolyFlags::FAKE_BACKDROP))
            .count();
        let sky_zone = if fake_backdrop_surfaces == 0 {
            None
        } else {
            model
                .sky_zone(&package)
                .context("failed to decode the sky zone")?
        };
        let (textures, surface_materials) = load_materials(&mut packages, &package, &model);
        let textured_surfaces = surface_materials
            .iter()
            .filter(|material| material.texture.is_some())
            .count();
        let masked_surfaces = surface_materials
            .iter()
            .filter(|material| material.masked)
            .count();
        let translucent_surfaces = surface_materials
            .iter()
            .filter(|material| material.mode == SurfaceMode::Translucent)
            .count();
        let modulated_surfaces = surface_materials
            .iter()
            .filter(|material| material.mode == SurfaceMode::Modulated)
            .count();
        info!(
            map = %path.display(),
            points = model.points.len(),
            nodes = model.nodes.len(),
            surfaces = model.surfaces.len(),
            triangles = mesh.indices.len() / 3,
            textures = textures.len(),
            lightmaps = lightmaps.len(),
            textured_surfaces,
            masked_surfaces,
            translucent_surfaces,
            modulated_surfaces,
            fake_backdrop_surfaces,
            has_sky_zone = sky_zone.is_some(),
            "loaded map"
        );
        if fake_backdrop_surfaces != 0 && sky_zone.is_none() {
            warn!(
                fake_backdrop_surfaces,
                "map has fake backdrops but no BSP SkyZoneInfo"
            );
        }
        Ok(Self {
            path,
            render: RenderScene {
                mesh,
                textures,
                lightmaps,
                surface_materials,
                sky_zone,
            },
            points: model.points.len(),
            nodes: model.nodes.len(),
            surfaces: model.surfaces.len(),
            textured_surfaces,
            masked_surfaces,
            translucent_surfaces,
            modulated_surfaces,
            fake_backdrop_surfaces,
            has_sky_zone: sky_zone.is_some(),
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
        if surface.poly_flags.contains(PolyFlags::INVISIBLE) {
            materials.push(SurfaceMaterial {
                mode: SurfaceMode::Hidden,
                ..Default::default()
            });
            continue;
        }
        if surface.poly_flags.contains(PolyFlags::FAKE_BACKDROP) {
            materials.push(SurfaceMaterial {
                mode: SurfaceMode::Backdrop,
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
        let material = surface_material(surface.poly_flags, None, Some(texture_flags));
        let image_key = (key.0.clone(), key.1, material.masked);
        let texture_index = if let Some(index) = images.get(&image_key) {
            *index
        } else {
            let image = match decoded_texture.image(material.masked) {
                Ok(image) => image,
                Err(error) => {
                    warn!(surface_index, %error, "could not expand surface texture");
                    materials.push(material);
                    continue;
                }
            };
            let index = textures.len();
            textures.push(image);
            images.insert(image_key, index);
            index
        };
        materials.push(SurfaceMaterial {
            texture: Some(texture_index),
            ..material
        });
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
    let translucent = flags.contains(PolyFlags::TRANSLUCENT) || texture_flags.translucent;
    let modulated = flags.contains(PolyFlags::MODULATED) || texture_flags.modulated;
    SurfaceMaterial {
        texture,
        mode: if is_hidden(flags, texture_flags) {
            SurfaceMode::Hidden
        } else if flags.contains(PolyFlags::FAKE_BACKDROP) || texture_flags.fake_backdrop {
            SurfaceMode::Backdrop
        } else if translucent {
            SurfaceMode::Translucent
        } else if modulated {
            SurfaceMode::Modulated
        } else {
            SurfaceMode::Opaque
        },
        // UE1 precedence clears masking for translucent surfaces but retains
        // it for modulated surfaces.
        masked: !translucent && (flags.contains(PolyFlags::MASKED) || texture_flags.masked),
        two_sided: flags.contains(PolyFlags::TWO_SIDED) || texture_flags.two_sided,
        unlit: flags.contains(PolyFlags::UNLIT),
    }
}

fn is_hidden(flags: PolyFlags, texture_flags: TextureRenderFlags) -> bool {
    flags.contains(PolyFlags::INVISIBLE) || texture_flags.invisible
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
        assert_eq!(masked.mode, SurfaceMode::Opaque);
        assert!(masked.masked);
        assert!(masked.two_sided);
        assert!(!masked.unlit);

        let hidden =
            super::surface_material(PolyFlags::FAKE_BACKDROP, Some(1), Some(Default::default()));
        assert_eq!(hidden.mode, SurfaceMode::Backdrop);

        let unlit = super::surface_material(PolyFlags::UNLIT, None, None);
        assert!(unlit.unlit);
    }

    #[test]
    fn applies_ue1_blend_precedence() {
        let translucent = super::surface_material(
            PolyFlags::from_bits(0x0000_0046),
            Some(1),
            Some(Default::default()),
        );
        assert_eq!(translucent.mode, SurfaceMode::Translucent);
        assert!(!translucent.masked);

        let modulated = super::surface_material(
            PolyFlags::from_bits(0x0000_0042),
            Some(1),
            Some(Default::default()),
        );
        assert_eq!(modulated.mode, SurfaceMode::Modulated);
        assert!(modulated.masked);
    }
}
