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
    node_geometry: Vec<NodeGeometry>,
    actor_submissions: Vec<ActorSubmission>,
    materials: Vec<SurfaceMaterial>,
    surface_bindings: Vec<usize>,
    resolved_portals: Vec<usize>,
    has_sky_zone: bool,
}

struct NodeGeometry {
    indices: Vec<u32>,
    surface: Option<usize>,
}

impl SubmissionGeometry {
    pub(super) fn new(scene: &RenderScene, surface_bindings: Vec<usize>) -> Self {
        let node_geometry = node_geometry(scene);
        Self {
            indices: scene.mesh.indices.clone(),
            triangle_surfaces: scene.mesh.triangle_surfaces.clone(),
            triangle_nodes: scene.mesh.triangle_nodes.clone(),
            bsp_nodes: scene.mesh.bsp_nodes.clone(),
            node_geometry,
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
        let fallback = uses_fallback(&self.bsp_nodes, &self.triangle_nodes);
        let traversal = if fallback {
            (0..self.node_geometry.len()).collect()
        } else {
            traversal_order(&self.bsp_nodes, render_to_unreal(camera_position))
        };

        let mut children = Vec::new();
        let mut list0 = Vec::new();
        let mut list1 = Vec::new();
        let mut list2 = Vec::new();
        for node in traversal {
            let Some(geometry) = self
                .node_geometry
                .get(node)
                .filter(|geometry| !geometry.indices.is_empty())
            else {
                continue;
            };
            let Some(surface) = geometry.surface else {
                continue;
            };
            let Some(material) = self.materials.get(surface) else {
                continue;
            };
            let record = (node, surface, geometry.indices.as_slice());
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
        let geometry_changed = self.indices != scene.mesh.indices
            || self.triangle_surfaces != scene.mesh.triangle_surfaces
            || self.triangle_nodes != scene.mesh.triangle_nodes
            || (uses_fallback(&self.bsp_nodes, &self.triangle_nodes)
                && fallback_triangle_count(&self.actor_submissions, self.triangle_surfaces.len())
                    != fallback_triangle_count(
                        &scene.actor_submissions,
                        scene.mesh.triangle_surfaces.len(),
                    ));
        if geometry_changed {
            self.node_geometry = node_geometry(scene);
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

fn node_geometry(scene: &RenderScene) -> Vec<NodeGeometry> {
    let fallback = uses_fallback(&scene.mesh.bsp_nodes, &scene.mesh.triangle_nodes);
    let count = if fallback {
        fallback_triangle_count(&scene.actor_submissions, scene.mesh.triangle_surfaces.len())
    } else {
        scene.mesh.bsp_nodes.len()
    };
    let mut geometry = (0..count)
        .map(|_| NodeGeometry {
            indices: Vec::new(),
            surface: None,
        })
        .collect::<Vec<_>>();
    if fallback {
        for ((triangle, &surface), record) in scene
            .mesh
            .indices
            .chunks_exact(3)
            .zip(&scene.mesh.triangle_surfaces)
            .zip(&mut geometry)
        {
            record.indices.extend_from_slice(triangle);
            record.surface = Some(surface);
        }
    } else {
        for ((triangle, &surface), &node) in scene
            .mesh
            .indices
            .chunks_exact(3)
            .zip(&scene.mesh.triangle_surfaces)
            .zip(&scene.mesh.triangle_nodes)
        {
            if let Some(record) = geometry.get_mut(node) {
                record.indices.extend_from_slice(triangle);
                record.surface.get_or_insert(surface);
            }
        }
    }
    geometry
}

fn uses_fallback(bsp_nodes: &[BspNode], triangle_nodes: &[usize]) -> bool {
    bsp_nodes.is_empty() || triangle_nodes.is_empty()
}

fn fallback_triangle_count(actors: &[ActorSubmission], default: usize) -> usize {
    actors
        .iter()
        .map(|actor| actor.indices.start / 3)
        .min()
        .unwrap_or(default)
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

    fn scene(
        indices: Range<u32>,
        triangle_surfaces: Vec<usize>,
        triangle_nodes: Vec<usize>,
        bsp_nodes: Vec<BspNode>,
        actor_submissions: Vec<ActorSubmission>,
        materials: Vec<SurfaceMaterial>,
    ) -> RenderScene {
        RenderScene {
            mesh: openhp1_scene::TriangleMesh {
                indices: indices.collect(),
                triangle_surfaces,
                triangle_nodes,
                bsp_nodes,
                ..Default::default()
            },
            textures: Vec::new(),
            lightmaps: Vec::new(),
            realtime_lightmaps: Vec::new(),
            coronas: Vec::new(),
            actor_submissions,
            surface_materials: materials,
            warp_portals: Vec::new(),
            sky_zone: None,
        }
    }

    fn geometry(scene: RenderScene, surface_bindings: Vec<usize>) -> SubmissionGeometry {
        SubmissionGeometry::new(&scene, surface_bindings)
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
        let geometry = geometry(
            scene(
                0..18,
                vec![1, 0, 0, 2, 3, 4],
                vec![1, 0, 3, 2],
                vec![
                    node(1, 2, 3),
                    node(-1, -1, -1),
                    node(-1, -1, -1),
                    node(-1, -1, -1),
                ],
                vec![
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
            ),
            vec![0, 1, 2, 3, 4],
        );
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
        let mut geometry = geometry(
            scene(
                0..9,
                vec![0, 1, 2],
                vec![0, 1, 2],
                vec![node(-1, -1, 1), node(-1, -1, 2), node(-1, -1, -1)],
                Vec::new(),
                vec![
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
            ),
            vec![0, 1, 2],
        );
        geometry.resolved_portals = vec![1];
        geometry.has_sky_zone = true;
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
        let geometry = geometry(
            scene(
                0..6,
                vec![0, 1],
                vec![0, 1],
                vec![node(-1, -1, 1), node(-1, -1, -1)],
                Vec::new(),
                vec![
                    SurfaceMaterial {
                        mode: SurfaceMode::Backdrop,
                        ..Default::default()
                    },
                    SurfaceMaterial {
                        portal: true,
                        ..Default::default()
                    },
                ],
            ),
            vec![0, 1],
        );
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
    fn refresh_reuses_node_geometry_while_replacing_runtime_actor_records() {
        let mut geometry = geometry(
            scene(
                0..6,
                vec![0, 1],
                Vec::new(),
                Vec::new(),
                vec![ActorSubmission {
                    actor_index: 3,
                    indices: 3..6,
                    translucent_pass: false,
                }],
                vec![SurfaceMaterial::default(), SurfaceMaterial::default()],
            ),
            vec![0, 0],
        );
        let scene = scene(
            0..6,
            vec![0, 1],
            Vec::new(),
            Vec::new(),
            vec![ActorSubmission {
                actor_index: 3,
                indices: 3..6,
                translucent_pass: true,
            }],
            vec![SurfaceMaterial::default(), SurfaceMaterial::default()],
        );

        let node_geometry = geometry.node_geometry.as_ptr();
        let node_indices = geometry.node_geometry[0].indices.as_ptr();
        assert!(geometry.refresh(&scene, vec![0, 0]));
        assert_eq!(geometry.node_geometry.as_ptr(), node_geometry);
        assert_eq!(geometry.node_geometry[0].indices.as_ptr(), node_indices);
        let plan = geometry.plan(Vec3::ZERO, &[binding(0)]);
        assert!(matches!(
            plan.commands.last(),
            Some(SubmissionCommand::Geometry {
                source: GeometrySource::Actor { actor_index: 3 },
                ..
            })
        ));
    }

    #[test]
    fn refresh_rebuilds_fallback_geometry_when_actor_boundary_moves() {
        let materials = vec![SurfaceMaterial::default(); 3];
        let bsp_nodes = vec![node(-1, -1, -1)];
        let mut geometry = geometry(
            scene(
                0..9,
                vec![0, 1, 2],
                Vec::new(),
                bsp_nodes.clone(),
                vec![ActorSubmission {
                    actor_index: 7,
                    indices: 3..9,
                    translucent_pass: false,
                }],
                materials.clone(),
            ),
            vec![0; 3],
        );
        let scene = scene(
            0..9,
            vec![0, 1, 2],
            Vec::new(),
            bsp_nodes,
            vec![ActorSubmission {
                actor_index: 7,
                indices: 6..9,
                translucent_pass: false,
            }],
            materials,
        );

        assert!(geometry.refresh(&scene, vec![0; 3]));
        let plan = geometry.plan(Vec3::ZERO, &[binding(0)]);
        assert_eq!(plan.indices, (0..9).collect::<Vec<_>>());
        assert!(matches!(
            plan.commands.as_slice(),
            [
                SubmissionCommand::Geometry {
                    batch: DrawBatch { indices, .. },
                    source: GeometrySource::BspList1,
                },
                SubmissionCommand::Geometry {
                    source: GeometrySource::Actor { actor_index: 7 },
                    ..
                }
            ] if indices == &(0..6)
        ));
    }
}
