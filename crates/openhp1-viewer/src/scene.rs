use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result, ensure};
use openhp1_map::{Model, world_model_export};
use openhp1_package::{PackageStore, ResolvedObject};
use openhp1_render::{RenderScene, TextureImage};
use openhp1_texture::{Palette, Texture};
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
        let (textures, surface_textures) = load_textures(&mut packages, &package, &model);
        let textured_surfaces = surface_textures
            .iter()
            .filter(|texture| texture.is_some())
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
                surface_textures,
            },
            points: model.points.len(),
            nodes: model.nodes.len(),
            surfaces: model.surfaces.len(),
            textured_surfaces,
        })
    }
}

fn load_textures(
    packages: &mut PackageStore,
    map: &std::sync::Arc<openhp1_package::Package>,
    model: &Model,
) -> (Vec<TextureImage>, Vec<Option<usize>>) {
    let mut textures = Vec::new();
    let mut loaded = HashMap::<(String, usize), Option<usize>>::new();
    let mut surface_textures = Vec::with_capacity(model.surfaces.len());

    for (surface_index, surface) in model.surfaces.iter().enumerate() {
        let resolved = match packages.resolve(map, surface.texture) {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                surface_textures.push(None);
                continue;
            }
            Err(error) => {
                warn!(surface_index, %error, "could not resolve surface texture");
                surface_textures.push(None);
                continue;
            }
        };
        let key = (
            resolved.package.summary().source.to_string(),
            resolved.export_index,
        );
        let texture_index = if let Some(index) = loaded.get(&key) {
            *index
        } else {
            let decoded = match decode_texture(packages, &resolved) {
                Ok(image) => {
                    let index = textures.len();
                    textures.push(image);
                    Some(index)
                }
                Err(error) => {
                    warn!(surface_index, %error, "could not decode surface texture");
                    None
                }
            };
            loaded.insert(key, decoded);
            decoded
        };
        surface_textures.push(texture_index);
    }

    (textures, surface_textures)
}

fn decode_texture(packages: &mut PackageStore, resolved: &ResolvedObject) -> Result<TextureImage> {
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
    Ok(TextureImage {
        width: mip.width,
        height: mip.height,
        rgba: texture.rgba(0, &palette, false)?,
    })
}
