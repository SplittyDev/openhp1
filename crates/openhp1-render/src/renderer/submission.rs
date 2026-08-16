use std::ops::Range;

use glam::Vec3;

use crate::{
    ActorSubmission, BspNode, RenderScene, SurfaceMaterial, SurfaceMode, render_to_unreal,
};

use super::batch::{DrawBatch, MaterialBinding, pipeline_index};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SubmissionCommand {
    Geometry {
        batch: DrawBatch,
        source: GeometrySource,
    },
    Portal {
        surface: usize,
        indices: Range<u32>,
    },
    Mirror {
        surface: usize,
        indices: Range<u32>,
    },
    Backdrop {
        surface: usize,
        indices: Range<u32>,
        pipeline: usize,
        no_smooth: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GeometrySource {
    BspList1,
    BspList2 { node: usize },
    Actor { actor_index: usize },
}

pub(super) struct SubmissionPlan {
    pub(super) indices: Vec<u32>,
    pub(super) commands: Vec<SubmissionCommand>,
}

pub(super) struct SubmissionGeometry {
    indices: Vec<u32>,
    triangle_surfaces: Vec<usize>,
    triangle_nodes: Vec<usize>,
    bsp_nodes: Vec<BspNode>,
    actor_submissions: Vec<ActorSubmission>,
    materials: Vec<SurfaceMaterial>,
    surface_bindings: Vec<usize>,
    resolved_portals: Vec<usize>,
    has_sky_zone: bool,
}

impl SubmissionGeometry {
    pub(super) fn new(scene: &RenderScene, surface_bindings: Vec<usize>) -> Self {
        Self {
            indices: scene.mesh.indices.clone(),
            triangle_surfaces: scene.mesh.triangle_surfaces.clone(),
            triangle_nodes: scene.mesh.triangle_nodes.clone(),
            bsp_nodes: scene.mesh.bsp_nodes.clone(),
            actor_submissions: scene.actor_submissions.clone(),
            materials: scene.surface_materials.clone(),
            surface_bindings,
            resolved_portals: scene
                .warp_portals
                .iter()
                .filter(|portal| portal.destination.is_some())
                .map(|portal| portal.surface)
                .collect(),
            has_sky_zone: scene.sky_zone.is_some(),
        }
    }

    pub(super) fn plan(
        &self,
        camera_position: Vec3,
        bindings: &[MaterialBinding],
    ) -> SubmissionPlan {
        let fallback = self.bsp_nodes.is_empty() || self.triangle_nodes.is_empty();
        let fallback_triangles = self
            .actor_submissions
            .iter()
            .map(|actor| actor.indices.start / 3)
            .min()
            .unwrap_or(self.triangle_surfaces.len());
        let traversal = if fallback {
            (0..fallback_triangles).collect()
        } else {
            traversal_order(&self.bsp_nodes, render_to_unreal(camera_position))
        };
        let mut node_indices = vec![
            Vec::new();
            if fallback {
                fallback_triangles
            } else {
                self.bsp_nodes.len()
            }
        ];
        if fallback {
            for (triangle, indices) in self
                .indices
                .chunks_exact(3)
                .take(fallback_triangles)
                .zip(&mut node_indices)
            {
                indices.extend_from_slice(triangle);
            }
        } else {
            for (triangle, &node) in self.indices.chunks_exact(3).zip(&self.triangle_nodes) {
                if let Some(indices) = node_indices.get_mut(node) {
                    indices.extend_from_slice(triangle);
                }
            }
        }

        let mut children = Vec::new();
        let mut list0 = Vec::new();
        let mut list1 = Vec::new();
        let mut list2 = Vec::new();
        for node in traversal {
            let Some(indices) = node_indices.get(node).filter(|indices| !indices.is_empty()) else {
                continue;
            };
            let surface = if fallback {
                self.triangle_surfaces.get(node).copied()
            } else {
                self.triangle_nodes
                    .iter()
                    .position(|candidate| *candidate == node)
                    .and_then(|triangle| self.triangle_surfaces.get(triangle))
                    .copied()
            };
            let Some(surface) = surface else {
                continue;
            };
            let Some(material) = self.materials.get(surface) else {
                continue;
            };
            let record = (node, surface, indices.as_slice());
            if material.mode == SurfaceMode::Backdrop && self.has_sky_zone {
                children.push(SpecialRecord::Backdrop(record));
            } else if material.portal && self.resolved_portals.contains(&surface) {
                children.push(SpecialRecord::Portal(record));
            } else if material.mirror {
                list0.push(record);
            } else if material.mode == SurfaceMode::Hidden {
                continue;
            } else if material.masked
                || matches!(
                    material.mode,
                    SurfaceMode::Translucent | SurfaceMode::Modulated | SurfaceMode::DepthOnly
                )
            {
                list2.push(record);
            } else {
                list1.push(record);
            }
        }

        // Retail uses a composite UObject key. OpenHP1 does not retain those
        // runtime object indices, so keep the existing deterministic binding
        // grouping for this pixel-neutral opaque list.
        list1.sort_by_key(|(_, surface, _)| self.surface_bindings.get(*surface).copied());

        let mut plan = SubmissionPlan {
            indices: Vec::with_capacity(self.indices.len()),
            commands: Vec::new(),
        };
        for child in children {
            match child {
                SpecialRecord::Portal((_, surface, indices)) => {
                    plan.push_special(surface, indices, SpecialKind::Portal, &self.materials)
                }
                SpecialRecord::Backdrop((_, surface, indices)) => {
                    plan.push_special(surface, indices, SpecialKind::Backdrop, &self.materials)
                }
            }
        }
        for (_, surface, indices) in list0 {
            plan.push_special(surface, indices, SpecialKind::Mirror, &self.materials);
        }
        for (_, surface, indices) in list1 {
            plan.push_geometry(
                surface,
                indices,
                GeometrySource::BspList1,
                &self.materials,
                &self.surface_bindings,
                bindings,
            );
        }
        self.push_actors(false, &mut plan, bindings);
        for (node, surface, indices) in list2.into_iter().rev() {
            plan.push_geometry(
                surface,
                indices,
                GeometrySource::BspList2 { node },
                &self.materials,
                &self.surface_bindings,
                bindings,
            );
        }
        self.push_actors(true, &mut plan, bindings);
        plan
    }

    pub(super) fn refresh(&mut self, scene: &RenderScene, surface_bindings: Vec<usize>) -> bool {
        if scene.mesh.indices.len() != self.indices.len()
            || scene.mesh.triangle_surfaces.len() != self.triangle_surfaces.len()
            || scene.mesh.triangle_nodes.len() != self.triangle_nodes.len()
            || scene.mesh.bsp_nodes.len() != self.bsp_nodes.len()
            || scene.surface_materials.len() != self.materials.len()
            || surface_bindings.len() != self.surface_bindings.len()
        {
            return false;
        }
        self.indices.clone_from(&scene.mesh.indices);
        self.triangle_surfaces
            .clone_from(&scene.mesh.triangle_surfaces);
        self.triangle_nodes.clone_from(&scene.mesh.triangle_nodes);
        self.bsp_nodes.clone_from(&scene.mesh.bsp_nodes);
        self.actor_submissions.clone_from(&scene.actor_submissions);
        self.materials.clone_from(&scene.surface_materials);
        self.surface_bindings = surface_bindings;
        self.resolved_portals = scene
            .warp_portals
            .iter()
            .filter(|portal| portal.destination.is_some())
            .map(|portal| portal.surface)
            .collect();
        self.has_sky_zone = scene.sky_zone.is_some();
        true
    }

    fn push_actors(
        &self,
        translucent_pass: bool,
        plan: &mut SubmissionPlan,
        bindings: &[MaterialBinding],
    ) {
        for actor in self
            .actor_submissions
            .iter()
            .filter(|actor| actor.translucent_pass == translucent_pass)
        {
            let start_triangle = actor.indices.start / 3;
            let triangles = self
                .indices
                .get(actor.indices.clone())
                .unwrap_or_default()
                .chunks_exact(3);
            for (offset, triangle) in triangles.enumerate() {
                let Some(&surface) = self.triangle_surfaces.get(start_triangle + offset) else {
                    continue;
                };
                if self
                    .materials
                    .get(surface)
                    .is_some_and(|material| material.mode != SurfaceMode::Hidden)
                {
                    plan.push_geometry(
                        surface,
                        triangle,
                        GeometrySource::Actor {
                            actor_index: actor.actor_index,
                        },
                        &self.materials,
                        &self.surface_bindings,
                        bindings,
                    );
                }
            }
        }
    }
}

enum SpecialRecord<'a> {
    Portal((usize, usize, &'a [u32])),
    Backdrop((usize, usize, &'a [u32])),
}

enum SpecialKind {
    Portal,
    Mirror,
    Backdrop,
}

impl SubmissionPlan {
    fn push_geometry(
        &mut self,
        surface: usize,
        source: &[u32],
        owner: GeometrySource,
        materials: &[SurfaceMaterial],
        surface_bindings: &[usize],
        bindings: &[MaterialBinding],
    ) {
        let Some(&binding) = surface_bindings.get(surface) else {
            return;
        };
        let Some(material) = materials.get(surface).copied() else {
            return;
        };
        if bindings.get(binding).is_none() {
            return;
        }
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(source);
        let end = self.indices.len() as u32;
        if let Some(SubmissionCommand::Geometry {
            batch,
            source: previous,
        }) = self.commands.last_mut()
            && *previous == owner
            && batch.binding == binding
            && batch.pipeline == pipeline_index(material)
            && batch.no_smooth == material.no_smooth
        {
            batch.indices.end = end;
            return;
        }
        self.commands.push(SubmissionCommand::Geometry {
            batch: DrawBatch {
                indices: start..end,
                binding,
                pipeline: pipeline_index(material),
                no_smooth: material.no_smooth,
            },
            source: owner,
        });
    }

    fn push_special(
        &mut self,
        surface: usize,
        source: &[u32],
        kind: SpecialKind,
        materials: &[SurfaceMaterial],
    ) {
        let indices = self.append_indices(source);
        self.commands.push(match kind {
            SpecialKind::Portal => SubmissionCommand::Portal { surface, indices },
            SpecialKind::Mirror => SubmissionCommand::Mirror { surface, indices },
            SpecialKind::Backdrop => {
                let material = materials.get(surface).copied().unwrap_or_default();
                SubmissionCommand::Backdrop {
                    surface,
                    indices,
                    pipeline: usize::from(material.two_sided),
                    no_smooth: material.no_smooth,
                }
            }
        });
    }

    fn append_indices(&mut self, source: &[u32]) -> Range<u32> {
        let start = self.indices.len() as u32;
        self.indices.extend_from_slice(source);
        start..self.indices.len() as u32
    }
}

fn traversal_order(nodes: &[BspNode], camera: Vec3) -> Vec<usize> {
    fn visit(
        index: usize,
        nodes: &[BspNode],
        camera: Vec3,
        visited: &mut [bool],
        order: &mut Vec<usize>,
    ) {
        let Some(node) = nodes.get(index) else {
            return;
        };
        if std::mem::replace(&mut visited[index], true) {
            return;
        }
        let positive =
            Vec3::from_array(node.plane[..3].try_into().unwrap()).dot(camera) >= node.plane[3];
        let (near, far) = if positive {
            (node.front, node.back)
        } else {
            (node.back, node.front)
        };
        if let Ok(near) = usize::try_from(near) {
            visit(near, nodes, camera, visited, order);
        }
        let mut coplanar = Some(index);
        while let Some(current) = coplanar {
            if current != index && std::mem::replace(&mut visited[current], true) {
                break;
            }
            order.push(current);
            coplanar = nodes
                .get(current)
                .and_then(|node| usize::try_from(node.coplanar).ok());
        }
        if let Ok(far) = usize::try_from(far) {
            visit(far, nodes, camera, visited, order);
        }
    }

    let mut visited = vec![false; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());
    if !nodes.is_empty() {
        visit(0, nodes, camera, &mut visited, &mut order);
    }
    for index in 0..nodes.len() {
        visit(index, nodes, camera, &mut visited, &mut order);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(front: i32, back: i32, coplanar: i32) -> BspNode {
        BspNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            zone_mask: 0,
            flags: 0,
            vertex_pool: 0,
            surface: 0,
            back,
            front,
            coplanar,
            collision_bound: -1,
            render_bound: -1,
            zones: [0; 2],
            vertex_count: 3,
            leaves: [-1; 2],
        }
    }

    fn binding(texture: usize) -> MaterialBinding {
        MaterialBinding {
            texture,
            macro_texture: texture,
            detail_texture: texture,
            no_smooth: false,
            pipeline: 0,
            macro_enabled: false,
            detail_enabled: false,
            lit: true,
        }
    }

    #[test]
    fn trace_keeps_coplanar_node_records_and_retail_device_actor_gates() {
        let materials = vec![
            SurfaceMaterial {
                masked: true,
                ..Default::default()
            },
            SurfaceMaterial {
                mode: SurfaceMode::Translucent,
                ..Default::default()
            },
            SurfaceMaterial::default(),
            SurfaceMaterial::default(),
            SurfaceMaterial {
                mode: SurfaceMode::AlphaBlended,
                ..Default::default()
            },
        ];
        let geometry = SubmissionGeometry {
            indices: (0..18).collect(),
            triangle_surfaces: vec![1, 0, 0, 2, 3, 4],
            triangle_nodes: vec![1, 0, 3, 2],
            bsp_nodes: vec![
                node(1, 2, 3),
                node(-1, -1, -1),
                node(-1, -1, -1),
                node(-1, -1, -1),
            ],
            actor_submissions: vec![
                ActorSubmission {
                    actor_index: 7,
                    indices: 12..15,
                    translucent_pass: false,
                },
                ActorSubmission {
                    actor_index: 8,
                    indices: 15..18,
                    translucent_pass: true,
                },
            ],
            materials,
            surface_bindings: vec![0, 1, 2, 3, 4],
            resolved_portals: Vec::new(),
            has_sky_zone: false,
        };
        let bindings = (0..5).map(binding).collect::<Vec<_>>();
        let plan = geometry.plan(-Vec3::Z, &bindings);
        let trace = plan
            .commands
            .iter()
            .filter_map(|command| match command {
                SubmissionCommand::Geometry { source, .. } => Some(*source),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            trace,
            [
                GeometrySource::BspList1,
                GeometrySource::Actor { actor_index: 7 },
                GeometrySource::BspList2 { node: 3 },
                GeometrySource::BspList2 { node: 0 },
                GeometrySource::BspList2 { node: 1 },
                GeometrySource::Actor { actor_index: 8 },
            ]
        );
        assert_eq!(
            plan.indices,
            [9, 10, 11, 12, 13, 14, 6, 7, 8, 3, 4, 5, 0, 1, 2, 15, 16, 17]
        );
    }

    #[test]
    fn all_traversal_children_precede_list_zero_mirror_overlays() {
        let geometry = SubmissionGeometry {
            indices: (0..9).collect(),
            triangle_surfaces: vec![0, 1, 2],
            triangle_nodes: vec![0, 1, 2],
            bsp_nodes: vec![node(-1, -1, 1), node(-1, -1, 2), node(-1, -1, -1)],
            actor_submissions: Vec::new(),
            materials: vec![
                SurfaceMaterial {
                    mode: SurfaceMode::Backdrop,
                    ..Default::default()
                },
                SurfaceMaterial {
                    portal: true,
                    ..Default::default()
                },
                SurfaceMaterial {
                    mirror: true,
                    ..Default::default()
                },
            ],
            surface_bindings: vec![0, 1, 2],
            resolved_portals: vec![1],
            has_sky_zone: true,
        };
        let plan = geometry.plan(Vec3::ZERO, &[binding(0), binding(1), binding(2)]);

        assert!(matches!(
            plan.commands.as_slice(),
            [
                SubmissionCommand::Backdrop { surface: 0, .. },
                SubmissionCommand::Portal { surface: 1, .. },
                SubmissionCommand::Mirror { surface: 2, .. }
            ]
        ));
    }

    #[test]
    fn unresolved_portal_and_backdrop_fall_back_to_list_one() {
        let geometry = SubmissionGeometry {
            indices: (0..6).collect(),
            triangle_surfaces: vec![0, 1],
            triangle_nodes: vec![0, 1],
            bsp_nodes: vec![node(-1, -1, 1), node(-1, -1, -1)],
            actor_submissions: Vec::new(),
            materials: vec![
                SurfaceMaterial {
                    mode: SurfaceMode::Backdrop,
                    ..Default::default()
                },
                SurfaceMaterial {
                    portal: true,
                    ..Default::default()
                },
            ],
            surface_bindings: vec![0, 1],
            resolved_portals: Vec::new(),
            has_sky_zone: false,
        };
        let plan = geometry.plan(Vec3::ZERO, &[binding(0), binding(1)]);

        assert!(plan.commands.iter().all(|command| matches!(
            command,
            SubmissionCommand::Geometry {
                source: GeometrySource::BspList1,
                ..
            }
        )));
        assert_eq!(plan.indices.len(), 6);
    }

    #[test]
    fn refresh_replaces_runtime_actor_records() {
        let mut geometry = SubmissionGeometry {
            indices: (0..6).collect(),
            triangle_surfaces: vec![0, 1],
            triangle_nodes: Vec::new(),
            bsp_nodes: Vec::new(),
            actor_submissions: vec![ActorSubmission {
                actor_index: 3,
                indices: 3..6,
                translucent_pass: false,
            }],
            materials: vec![SurfaceMaterial::default(), SurfaceMaterial::default()],
            surface_bindings: vec![0, 0],
            resolved_portals: Vec::new(),
            has_sky_zone: false,
        };
        let scene = RenderScene {
            mesh: openhp1_scene::TriangleMesh {
                indices: (0..6).collect(),
                triangle_surfaces: vec![0, 1],
                ..Default::default()
            },
            textures: Vec::new(),
            lightmaps: Vec::new(),
            realtime_lightmaps: Vec::new(),
            coronas: Vec::new(),
            actor_submissions: vec![ActorSubmission {
                actor_index: 3,
                indices: 3..6,
                translucent_pass: true,
            }],
            surface_materials: vec![SurfaceMaterial::default(), SurfaceMaterial::default()],
            warp_portals: Vec::new(),
            sky_zone: None,
        };

        assert!(geometry.refresh(&scene, vec![0, 0]));
        let plan = geometry.plan(Vec3::ZERO, &[binding(0)]);
        assert!(matches!(
            plan.commands.last(),
            Some(SubmissionCommand::Geometry {
                source: GeometrySource::Actor { actor_index: 3 },
                ..
            })
        ));
    }
}
