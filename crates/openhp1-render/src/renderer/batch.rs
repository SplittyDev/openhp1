use std::ops::Range;

use glam::Vec3;

use crate::{RenderScene, SurfaceMaterial, SurfaceMode};

use super::{PIPELINES_PER_MODE, Vertex};

pub(super) struct DrawBatch {
    pub(super) indices: Range<u32>,
    pub(super) texture: usize,
    pub(super) pipeline: usize,
}

pub(super) struct BackdropBatch {
    pub(super) indices: Range<u32>,
    pub(super) pipeline: usize,
}

pub(super) struct MirrorGeometry {
    pub(super) surface: usize,
    pub(super) indices: Vec<u32>,
    pub(super) pipeline: usize,
}

pub(super) struct BlendedSurface {
    pub(super) indices: Vec<u32>,
    center: Vec3,
    texture: usize,
    pipeline: usize,
}

pub(super) fn backdrop_batches(scene: &RenderScene) -> (Vec<u32>, Vec<BackdropBatch>) {
    let mut buckets = [Vec::new(), Vec::new()];
    for (triangle, &surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let Some(material) = scene.surface_materials.get(surface) else {
            continue;
        };
        if material.mode == SurfaceMode::Backdrop {
            buckets[usize::from(material.two_sided)].extend_from_slice(triangle);
        }
    }

    let mut indices = Vec::new();
    let mut batches = Vec::new();
    for (pipeline, bucket) in buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        let start = indices.len() as u32;
        indices.extend(bucket);
        batches.push(BackdropBatch {
            indices: start..indices.len() as u32,
            pipeline,
        });
    }
    (indices, batches)
}

pub(super) fn mirror_geometries(scene: &RenderScene) -> Vec<MirrorGeometry> {
    let mut surfaces = vec![Vec::new(); scene.surface_materials.len()];
    for (triangle, &surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let Some(material) = scene.surface_materials.get(surface) else {
            continue;
        };
        if material.mirror {
            surfaces[surface].extend_from_slice(triangle);
        }
    }

    surfaces
        .into_iter()
        .enumerate()
        .filter_map(|(surface, indices)| {
            (!indices.is_empty()).then(|| MirrorGeometry {
                surface,
                indices,
                pipeline: usize::from(scene.surface_materials[surface].two_sided),
            })
        })
        .collect()
}

pub(super) fn texture_batches(
    scene: &RenderScene,
    fallback_texture: usize,
) -> (Vec<u32>, Vec<DrawBatch>) {
    let mut buckets = vec![Vec::new(); (fallback_texture + 1) * PIPELINES_PER_MODE];
    for (triangle, surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let material = scene
            .surface_materials
            .get(*surface)
            .copied()
            .unwrap_or_default();
        if material.mode != SurfaceMode::Opaque || material.mirror {
            continue;
        }
        let texture = material
            .texture
            .filter(|index| *index < fallback_texture)
            .unwrap_or(fallback_texture);
        let pipeline = pipeline_index(material);
        buckets[texture * PIPELINES_PER_MODE + pipeline].extend_from_slice(triangle);
    }

    let mut indices = Vec::with_capacity(scene.mesh.indices.len());
    let mut batches = Vec::new();
    for (bucket, source) in buckets.into_iter().enumerate() {
        if source.is_empty() {
            continue;
        }
        let start = indices.len() as u32;
        indices.extend(source);
        batches.push(DrawBatch {
            indices: start..indices.len() as u32,
            texture: bucket / PIPELINES_PER_MODE,
            pipeline: bucket % PIPELINES_PER_MODE,
        });
    }
    (indices, batches)
}

pub(super) fn blended_surfaces(
    scene: &RenderScene,
    fallback_texture: usize,
    vertices: &[Vertex],
) -> Vec<BlendedSurface> {
    let mut indices = vec![Vec::new(); scene.surface_materials.len()];
    let mut center_sums = vec![Vec3::ZERO; scene.surface_materials.len()];
    let mut triangle_counts = vec![0_u32; scene.surface_materials.len()];

    for (triangle, &surface) in scene
        .mesh
        .indices
        .chunks_exact(3)
        .zip(&scene.mesh.triangle_surfaces)
    {
        let Some(material) = scene.surface_materials.get(surface).copied() else {
            continue;
        };
        if !matches!(
            material.mode,
            SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::AlphaBlended
        ) {
            continue;
        }
        indices[surface].extend_from_slice(triangle);
        center_sums[surface] += triangle
            .iter()
            .map(|&index| Vec3::from_array(vertices[index as usize].position))
            .sum::<Vec3>()
            / 3.0;
        triangle_counts[surface] += 1;
    }

    indices
        .into_iter()
        .enumerate()
        .filter_map(|(surface, indices)| {
            if indices.is_empty() {
                return None;
            }
            let material = scene.surface_materials[surface];
            Some(BlendedSurface {
                indices,
                center: center_sums[surface] / triangle_counts[surface] as f32,
                texture: material
                    .texture
                    .filter(|index| *index < fallback_texture)
                    .unwrap_or(fallback_texture),
                pipeline: pipeline_index(material),
            })
        })
        .collect()
}

pub(super) fn sorted_blended_batches(
    surfaces: &[BlendedSurface],
    camera_position: Vec3,
) -> (Vec<u32>, Vec<DrawBatch>) {
    let mut sorted = surfaces.iter().collect::<Vec<_>>();
    // Match UE1's translucent-node pass: closest surface origins first.
    sorted.sort_by(|left, right| {
        left.center
            .distance_squared(camera_position)
            .total_cmp(&right.center.distance_squared(camera_position))
    });

    let mut indices = Vec::new();
    let mut batches: Vec<DrawBatch> = Vec::new();
    for surface in sorted {
        let start = indices.len() as u32;
        indices.extend_from_slice(&surface.indices);
        let end = indices.len() as u32;
        if let Some(batch) = batches.last_mut()
            && batch.texture == surface.texture
            && batch.pipeline == surface.pipeline
        {
            batch.indices.end = end;
        } else {
            batches.push(DrawBatch {
                indices: start..end,
                texture: surface.texture,
                pipeline: surface.pipeline,
            });
        }
    }
    (indices, batches)
}

pub(super) fn update_blended_centers(surfaces: &mut [BlendedSurface], vertices: &[Vertex]) {
    for surface in surfaces {
        let mut center = Vec3::ZERO;
        let mut count = 0;
        for &index in &surface.indices {
            let Some(vertex) = vertices.get(index as usize) else {
                continue;
            };
            center += Vec3::from_array(vertex.position);
            count += 1;
        }
        if count != 0 {
            surface.center = center / count as f32;
        }
    }
}

fn pipeline_index(material: SurfaceMaterial) -> usize {
    let mode = match material.mode {
        SurfaceMode::Opaque | SurfaceMode::Backdrop | SurfaceMode::Hidden => 0,
        SurfaceMode::Translucent => 1,
        SurfaceMode::Modulated => 2,
        SurfaceMode::AlphaBlended => 3,
    };
    mode * PIPELINES_PER_MODE
        + usize::from(material.unlit) * 4
        + usize::from(material.masked) * 2
        + usize::from(material.two_sided)
}
