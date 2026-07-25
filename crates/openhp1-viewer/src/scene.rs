use std::path::PathBuf;

use anyhow::{Context, Result};
use openhp1_map::{Model, TriangleMesh, world_model_export};
use openhp1_package::Package;
use tracing::info;

pub(crate) struct LoadedScene {
    pub(crate) path: PathBuf,
    pub(crate) mesh: TriangleMesh,
    pub(crate) points: usize,
    pub(crate) nodes: usize,
    pub(crate) surfaces: usize,
}

impl LoadedScene {
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        let package =
            Package::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
        let model_export =
            world_model_export(&package).context("failed to find the world model")?;
        let model =
            Model::decode(&package, model_export).context("failed to decode the world model")?;
        let mesh = model.triangulate().context("failed to triangulate BSP")?;
        info!(
            map = %path.display(),
            points = model.points.len(),
            nodes = model.nodes.len(),
            surfaces = model.surfaces.len(),
            triangles = mesh.indices.len() / 3,
            "loaded map"
        );
        Ok(Self {
            path,
            mesh,
            points: model.points.len(),
            nodes: model.nodes.len(),
            surfaces: model.surfaces.len(),
        })
    }
}
