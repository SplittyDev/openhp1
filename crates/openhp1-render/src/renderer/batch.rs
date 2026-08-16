use std::ops::Range;

use std::collections::BTreeMap;

use crate::{RenderScene, SurfaceMaterial, SurfaceMode};

use super::PIPELINES_PER_MODE;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DrawBatch {
    pub(super) indices: Range<u32>,
    pub(super) binding: usize,
    pub(super) pipeline: usize,
    pub(super) no_smooth: bool,
}

pub(super) struct MirrorGeometry {
    pub(super) surface: usize,
    pub(super) binding: usize,
    pub(super) pipeline: usize,
    pub(super) no_smooth: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct MaterialBinding {
    pub(super) texture: usize,
    pub(super) macro_texture: usize,
    pub(super) detail_texture: usize,
    pub(super) no_smooth: bool,
    pub(super) pipeline: usize,
    pub(super) macro_enabled: bool,
    pub(super) detail_enabled: bool,
    pub(super) lit: bool,
}

pub(super) fn attachment_enabled(material: SurfaceMaterial, detail_textures: bool) -> [bool; 2] {
    if material.mode == SurfaceMode::DepthOnly {
        return [false; 2];
    }
    [
        material.macro_texture.is_some(),
        detail_textures
            && material.detail_texture.is_some()
            && !material.fog_map_attached
            && !material.portal,
    ]
}

pub(super) fn material_bindings(
    scene: &RenderScene,
    fallback_texture: usize,
    detail_textures: bool,
) -> (Vec<MaterialBinding>, Vec<usize>) {
    let texture = |index: Option<usize>| {
        index
            .filter(|index| *index < fallback_texture)
            .unwrap_or(fallback_texture)
    };
    let keys = scene
        .surface_materials
        .iter()
        .map(|material| {
            let [macro_enabled, detail_enabled] = attachment_enabled(*material, detail_textures);
            MaterialBinding {
                texture: texture(material.texture),
                macro_texture: texture(material.macro_texture),
                detail_texture: texture(material.detail_texture),
                no_smooth: material.no_smooth,
                pipeline: pipeline_index(*material),
                macro_enabled,
                detail_enabled,
                lit: !material.unlit,
            }
        })
        .collect::<Vec<_>>();
    let mut unique = keys.clone();
    unique.sort_unstable();
    unique.dedup();
    let lookup = unique
        .iter()
        .copied()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect::<BTreeMap<_, _>>();
    let surfaces = keys.iter().map(|key| lookup[key]).collect();
    (unique, surfaces)
}

pub(super) fn mirror_geometries(
    scene: &RenderScene,
    surface_bindings: &[usize],
) -> Vec<MirrorGeometry> {
    let mut surfaces = vec![false; scene.surface_materials.len()];
    for &surface in &scene.mesh.triangle_surfaces {
        let Some(material) = scene.surface_materials.get(surface) else {
            continue;
        };
        if material.mirror {
            surfaces[surface] = true;
        }
    }

    surfaces
        .into_iter()
        .enumerate()
        .filter_map(|(surface, present)| {
            let &binding = surface_bindings.get(surface)?;
            present.then(|| MirrorGeometry {
                surface,
                binding,
                pipeline: usize::from(scene.surface_materials[surface].two_sided),
                no_smooth: scene.surface_materials[surface].no_smooth,
            })
        })
        .collect()
}

pub(super) fn pipeline_index(material: SurfaceMaterial) -> usize {
    let mode = match material.mode {
        SurfaceMode::Opaque | SurfaceMode::Backdrop | SurfaceMode::Hidden => 0,
        SurfaceMode::Translucent => 1,
        SurfaceMode::Modulated => 2,
        SurfaceMode::AlphaBlended => 3,
        SurfaceMode::DepthOnly => 4,
    };
    mode * PIPELINES_PER_MODE
        + usize::from(material.unlit) * 4
        + usize::from(material.masked) * 2
        + usize::from(material.two_sided)
}
