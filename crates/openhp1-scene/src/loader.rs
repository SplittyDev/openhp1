use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec2, Vec3};
use openhp1_map::{
    Actor, ActorProperties, ActorVertexLighting, BspNode, Level, Model, PolyFlags, VertexLighting,
    bsp_zone_at,
};
use openhp1_mesh::{Mesh, MeshAnimationSequence, SkeletalAnimation};
use openhp1_package::{ObjectReference, Package, PackageStore, ResolvedObject};
use openhp1_physics::BspCollision;
use openhp1_script::class_defaults_reader;
use openhp1_texture::{Palette, Texture, TextureRenderFlags, WaterAnimation};
use tracing::{info, warn};

use crate::{
    RenderScene, Rotator, SceneActor, SceneActorAnimation, SceneActorRenderRange, SceneObjectId,
    SurfaceMaterial, SurfaceMode, TextureImage, render_to_unreal,
};

const NOT_FOR_SERVER: u32 = 0x0020_0000;
const PHYS_WALKING: u8 = 1;
const PHYS_FALLING: u8 = 2;
const PHYS_SWIMMING: u8 = 3;
const PHYS_FLYING: u8 = 4;

pub struct LoadedScene {
    pub path: PathBuf,
    pub levels: Vec<PathBuf>,
    pub render: RenderScene,
    pub points: usize,
    pub nodes: usize,
    pub surfaces: usize,
    pub visible_bsp_surfaces: usize,
    pub textured_surfaces: usize,
    pub masked_surfaces: usize,
    pub translucent_surfaces: usize,
    pub modulated_surfaces: usize,
    pub fake_backdrop_surfaces: usize,
    pub has_sky_zone: bool,
    pub actor_meshes: usize,
    pub animated_actor_meshes: usize,
    pub actors: Vec<SceneActor>,
    collision: Arc<BspCollision>,
    zone_nodes: Vec<BspNode>,
    zone_count: usize,
    animations: Vec<AnimatedActorMesh>,
    hidden_actor_positions: HashMap<usize, Vec<Vec3>>,
    water_animations: Vec<AnimatedWaterTexture>,
    actor_render: ActorRenderContext,
}

struct ActorRenderContext {
    packages: PackageStore,
    model: Model,
    vertex_lighting: VertexLighting,
    class_cache: HashMap<SceneObjectId, ClassState>,
    mesh_cache: HashMap<SceneObjectId, Option<Arc<Mesh>>>,
    animation_cache: HashMap<SceneObjectId, Option<Arc<SkeletalAnimation>>>,
    decoded_textures: HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: HashMap<(String, usize, bool), usize>,
}

impl LoadedScene {
    pub fn load(path: PathBuf) -> Result<Self> {
        let path = path
            .canonicalize()
            .with_context(|| format!("failed to locate {}", path.display()))?;
        let game_root = path
            .parent()
            .and_then(|directory| directory.parent())
            .context("map path must be inside the game's Maps directory")?;
        let mut packages =
            PackageStore::scan_game_root(game_root).context("failed to discover game packages")?;
        let map_directory = path.parent().expect("validated map path");
        let mut levels = packages
            .package_paths()
            .filter_map(|candidate| candidate.canonicalize().ok())
            .filter(|candidate| candidate.parent() == Some(map_directory))
            .collect::<Vec<_>>();
        if !levels.contains(&path) {
            levels.push(path.clone());
        }
        levels.sort_by_cached_key(|level| {
            level
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        });
        let package = packages
            .load_path(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let level_export = package
            .summary()
            .exports
            .iter()
            .position(|export| package.summary().class_name(export) == Some("Level"))
            .context("failed to find the level")?;
        let level = Level::decode(&package, level_export).context("failed to decode the level")?;
        let ObjectReference::Export(model_export) = level.model else {
            bail!("level world model is not a local export");
        };
        let model =
            Model::decode(&package, model_export).context("failed to decode the world model")?;
        let collision = Arc::new(
            BspCollision::from_model(&model).context("failed to build BSP collision model")?,
        );
        let mut mesh = model.triangulate().context("failed to triangulate BSP")?;
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
        let mut class_cache = HashMap::<SceneObjectId, ClassState>::new();
        let zone_pan_speeds =
            load_zone_pan_speeds(&mut packages, &package, &model, &mut class_cache);
        let mut water_animations = Vec::new();
        let (mut textures, mut surface_materials) = load_materials(
            &mut packages,
            &package,
            &model,
            &zone_pan_speeds,
            &mut water_animations,
        );
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
        let visible_bsp_surfaces = mesh
            .triangle_surfaces
            .iter()
            .copied()
            .filter(|&surface| surface_materials[surface].mode != SurfaceMode::Hidden)
            .collect::<HashSet<_>>()
            .len();
        let mut animations = Vec::new();
        let vertex_lighting = model
            .vertex_lighting(&package)
            .context("failed to decode actor vertex lighting")?;
        let mut actor_render = ActorRenderContext {
            packages,
            model,
            vertex_lighting,
            class_cache,
            mesh_cache: HashMap::new(),
            animation_cache: HashMap::new(),
            decoded_textures: HashMap::new(),
            images: HashMap::new(),
        };
        let actors = load_actors(
            &mut actor_render,
            &package,
            &level,
            &mut mesh,
            &mut textures,
            &mut surface_materials,
            &mut animations,
            &mut water_animations,
        );
        let mut hidden_actor_positions = HashMap::new();
        for (actor_index, actor) in actors.iter().enumerate().filter(|(_, actor)| actor.hidden) {
            let Some(render) = &actor.render else {
                continue;
            };
            hidden_actor_positions.insert(
                actor_index,
                mesh.positions[render.vertices.clone()].to_vec(),
            );
            collapse_positions(&mut mesh.positions[render.vertices.clone()]);
        }
        let actor_meshes = actors.iter().filter(|actor| actor.render.is_some()).count();
        let animated_actor_meshes = actors
            .iter()
            .filter(|actor| actor.animation.is_some())
            .count();
        info!(
            map = %path.display(),
            points = actor_render.model.points.len(),
            nodes = actor_render.model.nodes.len(),
            surfaces = actor_render.model.surfaces.len(),
            triangles = mesh.indices.len() / 3,
            textures = textures.len(),
            lightmaps = lightmaps.len(),
            textured_surfaces,
            masked_surfaces,
            translucent_surfaces,
            modulated_surfaces,
            fake_backdrop_surfaces,
            has_sky_zone = sky_zone.is_some(),
            actor_meshes,
            animated_actor_meshes,
            animated_water_textures = water_animations.len(),
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
            levels,
            render: RenderScene {
                mesh,
                textures,
                lightmaps,
                surface_materials,
                sky_zone,
            },
            points: actor_render.model.points.len(),
            nodes: actor_render.model.nodes.len(),
            surfaces: actor_render.model.surfaces.len(),
            visible_bsp_surfaces,
            textured_surfaces,
            masked_surfaces,
            translucent_surfaces,
            modulated_surfaces,
            fake_backdrop_surfaces,
            has_sky_zone: sky_zone.is_some(),
            actor_meshes,
            animated_actor_meshes,
            actors,
            collision,
            zone_nodes: actor_render.model.nodes.clone(),
            zone_count: actor_render.model.zones.len(),
            animations,
            hidden_actor_positions,
            water_animations,
            actor_render,
        })
    }

    pub fn collision(&self) -> Arc<BspCollision> {
        Arc::clone(&self.collision)
    }

    pub fn zone_at(&self, render_position: Vec3) -> usize {
        bsp_zone_at(
            &self.zone_nodes,
            self.zone_count,
            render_to_unreal(render_position),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_actor(
        &mut self,
        actor_index: usize,
        name: String,
        class_package: String,
        class_export: usize,
        class_name: String,
        location: Vec3,
        rotation: Rotator,
    ) -> Result<()> {
        ensure!(location.is_finite(), "spawned actor location is not finite");
        while self.actors.len() < actor_index {
            self.actors
                .push(runtime_actor_placeholder(self.actors.len()));
        }
        let package = self
            .actor_render
            .packages
            .load_path(Path::new(&class_package))?;
        let class = SceneObject {
            package,
            export_index: class_export,
        };
        let class_state = class_state(
            &mut self.actor_render.packages,
            &class,
            &mut self.actor_render.class_cache,
            0,
        );
        let is_pawn = class_state.is_pawn;
        let mut state = class_state.actor;
        state.location = location;
        state.rotation = rotation;
        let mut actor = SceneActor {
            id: SceneObjectId {
                package: "<runtime>".to_owned(),
                export_index: actor_index,
            },
            name,
            class: Some(class.id()),
            class_name,
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            pre_pivot: Vec3::ZERO,
            draw_scale: 1.0,
            draw_type: 0,
            hidden: false,
            unlit: false,
            mesh: None,
            mesh_name: None,
            animation: None,
            render: None,
            diagnostics: class_state.diagnostics,
        };
        apply_scene_actor_state(&mut actor, &state);
        append_scene_actor_mesh(
            &mut self.actor_render,
            &mut actor,
            &state,
            is_pawn,
            actor_index,
            &mut self.render.mesh,
            &mut self.render.textures,
            &mut self.render.surface_materials,
            &mut self.animations,
            &mut self.water_animations,
        );
        if let Some(render) = &actor.render {
            self.actor_meshes += 1;
            if actor.animation.is_some() {
                self.animated_actor_meshes += 1;
            }
            if actor.hidden {
                self.hidden_actor_positions.insert(
                    actor_index,
                    self.render.mesh.positions[render.vertices.clone()].to_vec(),
                );
                collapse_positions(&mut self.render.mesh.positions[render.vertices.clone()]);
            }
        }
        if actor_index == self.actors.len() {
            self.actors.push(actor);
        } else {
            ensure!(
                self.actors[actor_index].id.package == "<runtime>"
                    && self.actors[actor_index].class.is_none(),
                "runtime actor index {actor_index} already exists in the scene"
            );
            self.actors[actor_index] = actor;
        }
        Ok(())
    }

    pub fn ensure_runtime_actor(&mut self, actor_index: usize) {
        while self.actors.len() <= actor_index {
            self.actors
                .push(runtime_actor_placeholder(self.actors.len()));
        }
    }

    pub fn set_actor_location(&mut self, actor_index: usize, location: Vec3) -> Result<bool> {
        ensure!(location.is_finite(), "actor location is not finite");
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let delta = location - actor.location;
        if delta == Vec3::ZERO {
            return Ok(false);
        }
        let hidden = actor.hidden;
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &vertices {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }

        self.actors[actor_index].location = location;
        if let Some(vertices) = vertices {
            if hidden {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&actor_index)
                    .context("hidden actor has no saved render positions")?;
                translate_positions(positions, delta);
            } else {
                translate_positions(&mut self.render.mesh.positions[vertices], delta);
            }
        }
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.transform = Mat4::from_translation(delta) * animation.transform;
        }
        // ponytail: retain baked actor lighting until moving lights/zones are observable.
        Ok(true)
    }

    pub fn set_actor_rotation(&mut self, actor_index: usize, rotation: Rotator) -> Result<bool> {
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.rotation == rotation {
            return Ok(false);
        }
        let hidden = actor.hidden;
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &vertices {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }

        let transform = rotation_delta(actor.location + actor.pre_pivot, actor.rotation, rotation);
        self.actors[actor_index].rotation = rotation;
        if let Some(vertices) = vertices {
            if hidden {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&actor_index)
                    .context("hidden actor has no saved render positions")?;
                transform_positions(positions, transform);
            } else {
                transform_positions(&mut self.render.mesh.positions[vertices], transform);
            }
        }
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.transform = transform * animation.transform;
            animation.normal_transform = Mat3::from_mat4(animation.transform).inverse().transpose();
        }
        // ponytail: retain baked actor lighting until moving lights/zones are observable.
        Ok(true)
    }

    pub fn set_actor_hidden(&mut self, actor_index: usize, hidden: bool) -> Result<bool> {
        let actor = self
            .actors
            .get_mut(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.hidden == hidden {
            return Ok(false);
        }
        actor.hidden = hidden;
        let Some(render) = &actor.render else {
            return Ok(false);
        };
        ensure!(
            render.vertices.start <= render.vertices.end
                && render.vertices.end <= self.render.mesh.positions.len(),
            "actor render range is outside the scene mesh"
        );
        let positions = &mut self.render.mesh.positions[render.vertices.clone()];
        if hidden {
            self.hidden_actor_positions
                .insert(actor_index, positions.to_vec());
            Ok(collapse_positions(positions))
        } else {
            let restored = self
                .hidden_actor_positions
                .remove(&actor_index)
                .context("hidden actor has no saved render positions")?;
            ensure!(
                restored.len() == positions.len(),
                "hidden actor changed vertex count"
            );
            positions.copy_from_slice(&restored);
            Ok(true)
        }
    }

    pub fn destroy_actor(&mut self, actor_index: usize) -> Result<bool> {
        let actor = self
            .actors
            .get_mut(actor_index)
            .context("runtime refers to a missing scene actor")?;
        actor.hidden = true;
        self.hidden_actor_positions.remove(&actor_index);
        let render = actor.render.take();
        let animated = actor.animation.take().is_some();
        self.animations
            .retain(|animation| animation.actor_index != actor_index);
        if animated {
            self.animated_actor_meshes = self.animated_actor_meshes.saturating_sub(1);
        }
        let Some(render) = render else {
            return Ok(false);
        };
        ensure!(
            render.vertices.start <= render.vertices.end
                && render.vertices.end <= self.render.mesh.positions.len(),
            "actor render range is outside the scene mesh"
        );
        self.actor_meshes = self.actor_meshes.saturating_sub(1);
        Ok(collapse_positions(
            &mut self.render.mesh.positions[render.vertices],
        ))
    }

    pub fn tick_animations(&mut self, delta_time: f32) -> Result<bool> {
        Ok(self.tick_animations_with_completions(delta_time)?.0)
    }

    pub(crate) fn actor_animation_groups(&self, actor: usize) -> Vec<(String, String)> {
        self.animations
            .iter()
            .find(|animation| animation.actor_index == actor)
            .map(|animation| {
                animation
                    .sequences()
                    .iter()
                    .map(|sequence| (sequence.name.clone(), sequence.group.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn tick_animations_with_completions(
        &mut self,
        delta_time: f32,
    ) -> Result<(bool, Vec<usize>)> {
        if delta_time <= 0.0 || !delta_time.is_finite() {
            return Ok((false, Vec::new()));
        }
        let mut changed = false;
        let mut completed = Vec::new();
        for animation in &mut self.animations {
            if !animation.playing || animation.rate == 0.0 && animation.tween_from.is_none() {
                continue;
            }
            changed = true;
            let tween = animation.tween_from.as_ref().map(|_| {
                animation.tween_elapsed =
                    (animation.tween_elapsed + delta_time).min(animation.tween_duration);
                animation.tween_elapsed / animation.tween_duration
            });
            let finished = if tween.is_none() {
                let frame_count = animation.sequences()[animation.sequence].frame_count;
                let (phase, finished, playing) = advance_animation(
                    animation.phase,
                    animation.rate,
                    delta_time,
                    animation.looping,
                    frame_count,
                );
                animation.phase = phase;
                animation.playing = playing;
                finished
            } else {
                false
            };
            if finished {
                completed.push(animation.actor_index);
            }
            let hidden = {
                let actor = self
                    .actors
                    .get_mut(animation.actor_index)
                    .context("animation refers to a missing scene actor")?;
                let actor_animation = actor
                    .animation
                    .as_mut()
                    .context("animated scene actor has no animation state")?;
                actor_animation.phase = animation.phase;
                actor.hidden
            };
            let triangles = animation.sample()?;
            ensure!(
                triangles.len() * 3 == animation.vertices.len(),
                "animation changed actor vertex count"
            );
            for (index, (destination, vertex)) in animation
                .vertices
                .clone()
                .zip(triangles.into_iter().flat_map(|triangle| triangle.vertices))
                .enumerate()
            {
                let target = animation.transform.transform_point3(vertex.position);
                let position = animation
                    .tween_from
                    .as_ref()
                    .zip(tween)
                    .map_or(target, |(from, tween)| from[index].lerp(target, tween));
                let normal = (animation.normal_transform * vertex.normal).normalize_or_zero();
                if hidden {
                    self.hidden_actor_positions
                        .get_mut(&animation.actor_index)
                        .context("hidden animated actor has no saved render positions")?[index] =
                        position;
                } else {
                    self.render.mesh.positions[destination] = position;
                }
                self.render.mesh.vertex_colors[destination] =
                    animation.lighting.color(position, normal, animation.unlit);
            }
            if tween == Some(1.0) {
                animation.tween_from = None;
            }
        }
        Ok((changed, completed))
    }

    pub fn tick_water(&mut self, delta_time: f32) -> Result<Vec<usize>> {
        let mut changed = Vec::new();
        for water in &mut self.water_animations {
            if water.animation.tick(delta_time) {
                self.render.textures[water.texture].rgba =
                    water.animation.rgba(&water.palette, water.masked)?;
                changed.push(water.texture);
            }
        }
        Ok(changed)
    }

    pub fn loop_actor_animation(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
    ) -> Result<bool> {
        self.loop_actor_animation_with_tween(actor_index, sequence_name, relative_rate, 0.0)
    }

    pub fn loop_actor_animation_with_tween(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
        tween_time: f32,
    ) -> Result<bool> {
        self.set_actor_animation(actor_index, sequence_name, relative_rate, tween_time, true)
    }

    pub fn play_actor_animation(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
    ) -> Result<bool> {
        self.play_actor_animation_with_tween(actor_index, sequence_name, relative_rate, 0.0)
    }

    pub fn play_actor_animation_with_tween(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
        tween_time: f32,
    ) -> Result<bool> {
        self.set_actor_animation(actor_index, sequence_name, relative_rate, tween_time, false)
    }

    pub fn actor_animation_playing(&self, actor_index: usize) -> bool {
        self.animations
            .iter()
            .find(|animation| animation.actor_index == actor_index)
            .is_some_and(|animation| {
                animation.playing && (animation.rate != 0.0 || animation.tween_from.is_some())
            })
    }

    pub fn finish_actor_animation(&mut self, actor_index: usize) {
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.looping = false;
        }
    }

    fn set_actor_animation(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
        tween_time: f32,
        looping: bool,
    ) -> Result<bool> {
        ensure!(relative_rate.is_finite(), "animation rate is not finite");
        ensure!(tween_time.is_finite(), "animation tween time is not finite");
        let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        else {
            return Ok(false);
        };
        let Some(sequence) = animation
            .sequences()
            .iter()
            .position(|sequence| sequence.name.eq_ignore_ascii_case(sequence_name))
        else {
            return Ok(false);
        };
        if looping
            && animation.sequence == sequence
            && animation.looping
            && animation.playing
            && animation.rate != 0.0
        {
            let source = &animation.sequences()[sequence];
            animation.rate = relative_rate * source.rate / source.frame_count.max(1) as f32;
            if let Some(actor_animation) = self
                .actors
                .get_mut(actor_index)
                .and_then(|actor| actor.animation.as_mut())
            {
                actor_animation.rate = animation.rate;
            }
            return Ok(true);
        }
        let source = &animation.sequences()[sequence];
        let source_name = source.name.clone();
        let source_rate = source.rate;
        let source_frames = source.frame_count;
        // ponytail: keep the displayed render-space pose until moving actors need
        // concurrent root-motion tweening.
        animation.tween_from = (tween_time > 0.0)
            .then(|| self.render.mesh.positions[animation.vertices.clone()].to_vec());
        animation.tween_elapsed = 0.0;
        animation.tween_duration = tween_time;
        animation.sequence = sequence;
        animation.phase = 0.0;
        animation.rate = relative_rate * source_rate / source_frames.max(1) as f32;
        animation.playing = true;
        animation.looping = looping;
        let actor = self
            .actors
            .get_mut(actor_index)
            .context("animation refers to a missing scene actor")?;
        if actor.animation.is_none() {
            self.animated_actor_meshes += 1;
        }
        actor.animation = Some(SceneActorAnimation {
            sequence: source_name,
            phase: 0.0,
            rate: animation.rate,
            frame_count: source_frames,
        });
        Ok(true)
    }
}

fn advance_animation(
    phase: f32,
    rate: f32,
    delta_time: f32,
    looping: bool,
    frame_count: usize,
) -> (f32, bool, bool) {
    let last = if frame_count > 1 {
        1.0 - 1.0 / frame_count as f32
    } else {
        1.0 - f32::EPSILON
    };
    let next = phase + rate * delta_time;
    if looping {
        let finished = phase < last && next >= last || phase >= last && next >= 1.0 + last;
        (
            if finished { last } else { next.rem_euclid(1.0) },
            finished,
            true,
        )
    } else if next >= last {
        (last, true, false)
    } else {
        (next.rem_euclid(1.0), false, true)
    }
}

fn translate_positions(positions: &mut [Vec3], delta: Vec3) {
    for position in positions {
        *position += delta;
    }
}

fn transform_positions(positions: &mut [Vec3], transform: Mat4) {
    for position in positions {
        *position = transform.transform_point3(*position);
    }
}

fn rotation_delta(origin: Vec3, old: Rotator, new: Rotator) -> Mat4 {
    Mat4::from_translation(origin)
        * rotation_matrix(new)
        * rotation_matrix(old).inverse()
        * Mat4::from_translation(-origin)
}

fn collapse_positions(positions: &mut [Vec3]) -> bool {
    let Some(position) = positions.first().copied() else {
        return false;
    };
    positions.fill(position);
    true
}

struct AnimatedActorMesh {
    actor_index: usize,
    mesh: Arc<Mesh>,
    skeletal_animation: Option<Arc<SkeletalAnimation>>,
    sequence: usize,
    phase: f32,
    rate: f32,
    playing: bool,
    looping: bool,
    tween_from: Option<Vec<Vec3>>,
    tween_elapsed: f32,
    tween_duration: f32,
    vertices: Range<usize>,
    transform: Mat4,
    normal_transform: Mat3,
    lighting: ActorVertexLighting,
    unlit: bool,
}

struct AnimatedWaterTexture {
    texture: usize,
    masked: bool,
    palette: Palette,
    animation: WaterAnimation,
}

impl AnimatedActorMesh {
    fn sequences(&self) -> &[MeshAnimationSequence] {
        self.skeletal_animation
            .as_ref()
            .map_or(self.mesh.animation_sequences.as_slice(), |animation| {
                animation.sequences.as_slice()
            })
    }

    fn sample(&self) -> openhp1_mesh::Result<Vec<openhp1_mesh::MeshTriangle>> {
        if let Some(animation) = &self.skeletal_animation {
            self.mesh
                .sample_skeletal_sequence(animation, self.sequence, self.phase)
        } else {
            self.mesh
                .sample_sequence(&self.mesh.animation_sequences[self.sequence], self.phase)
        }
    }
}

#[derive(Clone)]
struct SceneObject {
    package: Arc<Package>,
    export_index: usize,
}

impl SceneObject {
    fn id(&self) -> SceneObjectId {
        SceneObjectId {
            package: self.package.summary().source.to_string(),
            export_index: self.export_index,
        }
    }

    fn name(&self) -> String {
        self.package
            .summary()
            .name(self.package.summary().exports[self.export_index].object_name)
            .to_owned()
    }
}

impl From<ResolvedObject> for SceneObject {
    fn from(value: ResolvedObject) -> Self {
        Self {
            package: value.package,
            export_index: value.export_index,
        }
    }
}

#[derive(Clone)]
struct ActorState {
    location: Vec3,
    rotation: Rotator,
    pre_pivot: Vec3,
    collision_height: f32,
    physics: u8,
    draw_scale: f32,
    draw_type: u8,
    mesh: Option<SceneObject>,
    skeletal_animation: Option<SceneObject>,
    skin: Option<SceneObject>,
    texture: Option<SceneObject>,
    multi_skins: Vec<Option<SceneObject>>,
    style: u8,
    ambient_glow: u8,
    scale_glow: f32,
    anim_sequence: Option<String>,
    anim_frame: f32,
    anim_rate: f32,
    texture_pan_speed: Vec2,
    hidden: bool,
    unlit: bool,
}

#[derive(Clone)]
struct ClassState {
    actor: ActorState,
    is_pawn: bool,
    diagnostics: Vec<String>,
}

impl Default for ActorState {
    fn default() -> Self {
        Self {
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            pre_pivot: Vec3::ZERO,
            collision_height: 0.0,
            physics: 0,
            draw_scale: 1.0,
            draw_type: 0,
            mesh: None,
            skeletal_animation: None,
            skin: None,
            texture: None,
            multi_skins: Vec::new(),
            style: 1,
            ambient_glow: 0,
            scale_glow: 1.0,
            anim_sequence: None,
            anim_frame: 0.0,
            anim_rate: 0.0,
            texture_pan_speed: Vec2::ONE,
            hidden: false,
            unlit: false,
        }
    }
}

impl ActorState {
    fn apply(
        &mut self,
        packages: &mut PackageStore,
        source: &Arc<Package>,
        properties: &ActorProperties,
    ) -> Result<()> {
        if let Some(location) = properties.location {
            self.location = location;
        }
        if let Some(rotation) = properties.rotation {
            self.rotation = rotation;
        }
        if let Some(pre_pivot) = properties.pre_pivot {
            self.pre_pivot = pre_pivot;
        }
        if let Some(collision_height) = properties.collision_height {
            self.collision_height = collision_height;
        }
        if let Some(physics) = properties.physics {
            self.physics = physics;
        }
        if let Some(draw_scale) = properties.draw_scale {
            self.draw_scale = draw_scale;
        }
        if let Some(draw_type) = properties.draw_type {
            self.draw_type = draw_type;
        }
        if let Some(reference) = properties.mesh {
            self.mesh = packages.resolve(source, reference)?.map(Into::into);
        }
        if let Some(reference) = properties.skeletal_animation {
            self.skeletal_animation = packages.resolve(source, reference)?.map(Into::into);
        }
        if let Some(reference) = properties.skin {
            self.skin = packages.resolve(source, reference)?.map(Into::into);
        }
        if let Some(reference) = properties.texture {
            self.texture = packages.resolve(source, reference)?.map(Into::into);
        }
        for (index, reference) in properties.multi_skins.iter().enumerate() {
            let Some(reference) = reference else {
                continue;
            };
            self.multi_skins.resize(index + 1, None);
            self.multi_skins[index] = packages.resolve(source, *reference)?.map(Into::into);
        }
        if let Some(style) = properties.style {
            self.style = style;
        }
        if let Some(ambient_glow) = properties.ambient_glow {
            self.ambient_glow = ambient_glow;
        }
        if let Some(scale_glow) = properties.scale_glow {
            self.scale_glow = scale_glow;
        }
        if let Some(anim_sequence) = &properties.anim_sequence {
            self.anim_sequence = Some(anim_sequence.clone());
        }
        if let Some(anim_frame) = properties.anim_frame {
            self.anim_frame = anim_frame;
        }
        if let Some(anim_rate) = properties.anim_rate {
            self.anim_rate = anim_rate;
        }
        if let Some(speed) = properties.texture_u_pan_speed {
            self.texture_pan_speed.x = speed;
        }
        if let Some(speed) = properties.texture_v_pan_speed {
            self.texture_pan_speed.y = speed;
        }
        if let Some(hidden) = properties.hidden {
            self.hidden = hidden;
        }
        if let Some(unlit) = properties.unlit {
            self.unlit = unlit;
        }
        Ok(())
    }
}

fn runtime_actor_placeholder(actor_index: usize) -> SceneActor {
    SceneActor {
        id: SceneObjectId {
            package: "<runtime>".to_owned(),
            export_index: actor_index,
        },
        name: format!("RuntimeActor{actor_index}"),
        class: None,
        class_name: "<unknown>".to_owned(),
        location: Vec3::ZERO,
        rotation: Rotator::default(),
        pre_pivot: Vec3::ZERO,
        draw_scale: 1.0,
        draw_type: 0,
        hidden: false,
        unlit: false,
        mesh: None,
        mesh_name: None,
        animation: None,
        render: None,
        diagnostics: vec!["spawn action was lost after a deferred script failure".to_owned()],
    }
}

#[allow(clippy::too_many_arguments)]
fn load_actors(
    actor_render: &mut ActorRenderContext,
    map: &Arc<Package>,
    level: &Level,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    animations: &mut Vec<AnimatedActorMesh>,
    water_animations: &mut Vec<AnimatedWaterTexture>,
) -> Vec<SceneActor> {
    let mut actors = Vec::new();
    let mut seen_exports = HashSet::new();

    for reference in &level.actors {
        let ObjectReference::Export(export_index) = *reference else {
            continue;
        };
        if !seen_exports.insert(export_index) {
            warn!(export_index, "level contains a duplicate actor reference");
            continue;
        }
        let export = &map.summary().exports[export_index];
        if export.object_flags & NOT_FOR_SERVER != 0 {
            continue;
        }
        let mut scene_actor = SceneActor {
            id: SceneObjectId {
                package: map.summary().source.to_string(),
                export_index,
            },
            name: map.summary().name(export.object_name).to_owned(),
            class: None,
            class_name: map
                .summary()
                .object_name(export.class)
                .unwrap_or("<no class>")
                .to_owned(),
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            pre_pivot: Vec3::ZERO,
            draw_scale: 1.0,
            draw_type: 0,
            hidden: false,
            unlit: false,
            mesh: None,
            mesh_name: None,
            animation: None,
            render: None,
            diagnostics: Vec::new(),
        };
        let actor = match Actor::decode(map, export_index) {
            Ok(actor) => actor,
            Err(error) => {
                warn!(export_index, %error, "could not decode actor");
                scene_actor
                    .diagnostics
                    .push(format!("actor decode failed: {error}"));
                actors.push(scene_actor);
                continue;
            }
        };
        let class = match actor_render.packages.resolve(map, export.class) {
            Ok(Some(class)) => {
                let class = SceneObject::from(class);
                scene_actor.class = Some(class.id());
                scene_actor.class_name = class.name();
                class
            }
            Ok(None) => {
                scene_actor
                    .diagnostics
                    .push("actor has no resolvable class".to_owned());
                actors.push(scene_actor);
                continue;
            }
            Err(error) => {
                warn!(export_index, %error, "could not resolve actor class");
                scene_actor
                    .diagnostics
                    .push(format!("class resolution failed: {error}"));
                actors.push(scene_actor);
                continue;
            }
        };
        let class_state = class_state(
            &mut actor_render.packages,
            &class,
            &mut actor_render.class_cache,
            0,
        );
        scene_actor.diagnostics.extend(class_state.diagnostics);
        let is_pawn = class_state.is_pawn;
        let mut state = class_state.actor;
        if let Err(error) = state.apply(&mut actor_render.packages, map, &actor.properties) {
            warn!(export_index, %error, "could not resolve actor properties");
            scene_actor
                .diagnostics
                .push(format!("actor property resolution failed: {error}"));
            actors.push(scene_actor);
            continue;
        }
        apply_scene_actor_state(&mut scene_actor, &state);
        append_scene_actor_mesh(
            actor_render,
            &mut scene_actor,
            &state,
            is_pawn,
            actors.len(),
            render_mesh,
            textures,
            materials,
            animations,
            water_animations,
        );
        actors.push(scene_actor);
    }
    actors
}

#[allow(clippy::too_many_arguments)]
fn append_scene_actor_mesh(
    actor_render: &mut ActorRenderContext,
    scene_actor: &mut SceneActor,
    state: &ActorState,
    is_pawn: bool,
    actor_index: usize,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    animations: &mut Vec<AnimatedActorMesh>,
    water_animations: &mut Vec<AnimatedWaterTexture>,
) {
    if state.draw_type != 2 {
        scene_actor.diagnostics.push(format!(
            "DrawType {} is not rendered as a mesh",
            state.draw_type
        ));
        return;
    }
    let Some(mesh_object) = state.mesh.clone() else {
        scene_actor
            .diagnostics
            .push("mesh draw type has no mesh assigned".to_owned());
        return;
    };
    let ActorRenderContext {
        packages,
        model,
        vertex_lighting,
        mesh_cache,
        animation_cache,
        decoded_textures,
        images,
        ..
    } = actor_render;
    let mesh_key = mesh_object.id();
    if !mesh_cache.contains_key(&mesh_key) {
        let decoded = match Mesh::decode(&mesh_object.package, mesh_object.export_index) {
            Ok(mesh) => Some(Arc::new(mesh)),
            Err(error) => {
                warn!(actor = %scene_actor.name, %error, "could not decode actor mesh");
                None
            }
        };
        mesh_cache.insert(mesh_key.clone(), decoded);
    }
    let Some(mesh) = mesh_cache.get(&mesh_key).and_then(Option::as_ref).cloned() else {
        scene_actor.diagnostics.push(format!(
            "mesh {} could not be decoded",
            scene_actor.mesh_name.as_deref().unwrap_or("<unnamed>")
        ));
        return;
    };
    let mesh_offset = pawn_mesh_offset(
        is_pawn,
        mesh_object
            .package
            .summary()
            .class_name(&mesh_object.package.summary().exports[mesh_object.export_index])
            == Some("SkeletalMesh"),
        state.physics,
        state.collision_height,
        mesh.origin.z,
    );
    let animation_object = state.skeletal_animation.clone().or_else(|| {
        match packages.resolve(&mesh_object.package, mesh.default_animation) {
            Ok(animation) => animation.map(SceneObject::from),
            Err(error) => {
                warn!(
                    actor = %scene_actor.name,
                    %error,
                    "could not resolve actor skeletal animation"
                );
                None
            }
        }
    });
    let skeletal_animation = animation_object.and_then(|animation_object| {
        let key = animation_object.id();
        if !animation_cache.contains_key(&key) {
            let decoded = match SkeletalAnimation::decode(
                &animation_object.package,
                animation_object.export_index,
            ) {
                Ok(animation) => Some(Arc::new(animation)),
                Err(error) => {
                    warn!(
                        actor = %scene_actor.name,
                        %error,
                        "could not decode actor skeletal animation"
                    );
                    None
                }
            };
            animation_cache.insert(key.clone(), decoded);
        }
        animation_cache.get(&key).and_then(Option::as_ref).cloned()
    });
    match append_actor_mesh(
        packages,
        &mesh_object,
        &mesh,
        skeletal_animation.as_ref(),
        state,
        mesh_offset,
        actor_index,
        model,
        vertex_lighting,
        render_mesh,
        textures,
        materials,
        decoded_textures,
        images,
        animations,
        water_animations,
    ) {
        Ok(Some(appended)) => {
            scene_actor.render = Some(appended.render);
            scene_actor.animation = appended.animation;
        }
        Ok(None) => scene_actor
            .diagnostics
            .push("mesh contains no renderable triangles".to_owned()),
        Err(error) => {
            warn!(actor = %scene_actor.name, %error, "could not append actor mesh");
            scene_actor
                .diagnostics
                .push(format!("mesh assembly failed: {error}"));
        }
    }
}

fn class_state(
    packages: &mut PackageStore,
    class: &SceneObject,
    cache: &mut HashMap<SceneObjectId, ClassState>,
    depth: usize,
) -> ClassState {
    if depth > 32 {
        return ClassState {
            actor: ActorState::default(),
            is_pawn: false,
            diagnostics: vec!["class inheritance exceeds 32 levels".to_owned()],
        };
    }
    let key = class.id();
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let (base, properties) = match decode_class_defaults(class) {
        Ok(defaults) => defaults,
        Err(error) => {
            warn!(
                class = %class.package.summary().name(
                    class.package.summary().exports[class.export_index].object_name
                ),
                %error,
                "could not decode actor class defaults"
            );
            let error = format!("class defaults failed for {}: {error}", class.name());
            let state = ClassState {
                actor: ActorState::default(),
                is_pawn: class.name().eq_ignore_ascii_case("Pawn"),
                diagnostics: vec![error],
            };
            cache.insert(key, state.clone());
            return state;
        }
    };
    let mut state = match packages.resolve(&class.package, base) {
        Ok(Some(base)) => {
            let base = class_state(packages, &SceneObject::from(base), cache, depth + 1);
            ClassState {
                actor: base.actor,
                is_pawn: base.is_pawn,
                diagnostics: Vec::new(),
            }
        }
        Ok(None) => ClassState {
            actor: ActorState::default(),
            is_pawn: false,
            diagnostics: Vec::new(),
        },
        Err(error) => {
            let error = format!("base class resolution failed for {}: {error}", class.name());
            ClassState {
                actor: ActorState::default(),
                is_pawn: false,
                diagnostics: vec![error],
            }
        }
    };
    if let Err(error) = state.actor.apply(packages, &class.package, &properties) {
        warn!(%error, "could not resolve actor class properties");
        let error = format!(
            "class property resolution failed for {}: {error}",
            class.name()
        );
        state.diagnostics.push(error);
    }
    state.is_pawn |= class.name().eq_ignore_ascii_case("Pawn");
    cache.insert(key, state.clone());
    state
}

fn apply_scene_actor_state(actor: &mut SceneActor, state: &ActorState) {
    actor.location = state.location;
    actor.rotation = state.rotation;
    actor.pre_pivot = state.pre_pivot;
    actor.draw_scale = state.draw_scale;
    actor.draw_type = state.draw_type;
    actor.hidden = state.hidden;
    actor.unlit = state.unlit;
    actor.mesh = state.mesh.as_ref().map(SceneObject::id);
    actor.mesh_name = state.mesh.as_ref().map(SceneObject::name);
}

fn decode_class_defaults(class: &SceneObject) -> Result<(ObjectReference, ActorProperties)> {
    let (metadata, mut defaults) = class_defaults_reader(&class.package, class.export_index)?;
    Ok((metadata.base_field, ActorProperties::decode(&mut defaults)?))
}

struct AppendedActorMesh {
    render: SceneActorRenderRange,
    animation: Option<SceneActorAnimation>,
}

#[allow(clippy::too_many_arguments)]
fn append_actor_mesh(
    packages: &mut PackageStore,
    mesh_object: &SceneObject,
    mesh: &Arc<Mesh>,
    skeletal_animation: Option<&Arc<SkeletalAnimation>>,
    actor: &ActorState,
    mesh_offset: Vec3,
    actor_index: usize,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    decoded_textures: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
    animations: &mut Vec<AnimatedActorMesh>,
    water_animations: &mut Vec<AnimatedWaterTexture>,
) -> Result<Option<AppendedActorMesh>> {
    let mesh_textures = mesh
        .textures
        .iter()
        .map(|reference| {
            packages
                .resolve(&mesh_object.package, *reference)
                .map(|object| object.map(SceneObject::from))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let transform = Mat4::from_translation(actor.location + actor.pre_pivot)
        * rotation_matrix(actor.rotation)
        * Mat4::from_translation(mesh_offset)
        * Mat4::from_scale(Vec3::splat(actor.draw_scale))
        * rotation_matrix(Rotator {
            pitch: mesh.rotation_origin.x,
            yaw: mesh.rotation_origin.y,
            roll: mesh.rotation_origin.z,
        })
        * Mat4::from_scale(mesh.scale)
        * Mat4::from_translation(-mesh.origin);
    let normal_transform = Mat3::from_mat4(transform).inverse().transpose();
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for vertex in mesh.triangles.iter().flat_map(|triangle| triangle.vertices) {
        let position = transform.transform_point3(vertex.position);
        minimum = minimum.min(position);
        maximum = maximum.max(position);
    }
    let center = (minimum + maximum) * 0.5;
    let actor_lighting =
        vertex_lighting.for_actor(model, center, actor.ambient_glow, actor.scale_glow);
    let unlit = actor.unlit || model.zone_at(center) == 0;
    let mut actor_materials = HashMap::<(u32, i32), usize>::new();
    let first_vertex = render_mesh.positions.len();
    let first_index = render_mesh.indices.len();
    let sequences = skeletal_animation.map_or(mesh.animation_sequences.as_slice(), |animation| {
        animation.sequences.as_slice()
    });
    let sequence = actor
        .anim_sequence
        .as_deref()
        .and_then(|name| {
            sequences
                .iter()
                .position(|sequence| sequence.name.eq_ignore_ascii_case(name))
        })
        .filter(|&index| sequences[index].frame_count != 0);
    let phase = actor.anim_frame.max(0.0).rem_euclid(1.0);
    let sampled;
    let triangles = if let Some(sequence) = sequence {
        sampled = if let Some(animation) = skeletal_animation {
            mesh.sample_skeletal_sequence(animation, sequence, phase)?
        } else {
            mesh.sample_sequence(&sequences[sequence], phase)?
        };
        sampled.as_slice()
    } else {
        &mesh.triangles
    };

    for triangle in triangles {
        let material_key = (triangle.poly_flags, triangle.texture_index);
        let surface = if let Some(surface) = actor_materials.get(&material_key) {
            *surface
        } else {
            let texture = select_actor_texture(actor, &mesh_textures, triangle.texture_index);
            let material = actor_surface_material(
                packages,
                texture.as_ref(),
                triangle.poly_flags,
                actor,
                textures,
                decoded_textures,
                images,
                water_animations,
            );
            let surface = materials.len();
            materials.push(material);
            actor_materials.insert(material_key, surface);
            surface
        };
        let dimensions = materials[surface]
            .texture
            .and_then(|index| textures.get(index))
            .map_or(Vec2::splat(64.0), |texture| {
                Vec2::new(texture.width as f32, texture.height as f32)
            });
        let base = u32::try_from(render_mesh.positions.len())?;
        for vertex in triangle.vertices {
            let position = transform.transform_point3(vertex.position);
            let normal = (normal_transform * vertex.normal).normalize_or_zero();
            render_mesh.positions.push(position);
            render_mesh
                .texture_coordinates
                .push(vertex.texture_coordinates * dimensions);
            render_mesh.lightmap_coordinates.push(Vec2::ZERO);
            render_mesh.vertex_lightmaps.push(None);
            render_mesh
                .vertex_colors
                .push(actor_lighting.color(position, normal, unlit));
            render_mesh.vertex_surfaces.push(surface);
        }
        render_mesh
            .indices
            .extend_from_slice(&[base, base + 2, base + 1]);
        render_mesh.triangle_surfaces.push(surface);
    }
    let animation = sequence.map(|sequence| SceneActorAnimation {
        sequence: sequences[sequence].name.clone(),
        phase,
        rate: actor.anim_rate,
        frame_count: sequences[sequence].frame_count,
    });
    if !sequences.is_empty() {
        animations.push(AnimatedActorMesh {
            actor_index,
            mesh: Arc::clone(mesh),
            skeletal_animation: skeletal_animation.cloned(),
            sequence: sequence.unwrap_or(0),
            phase,
            rate: animation.as_ref().map_or(0.0, |animation| animation.rate),
            playing: animation.is_some(),
            looping: true,
            tween_from: None,
            tween_elapsed: 0.0,
            tween_duration: 0.0,
            vertices: first_vertex..render_mesh.positions.len(),
            transform,
            normal_transform,
            lighting: actor_lighting,
            unlit,
        });
    }
    Ok(
        (render_mesh.positions.len() != first_vertex).then_some(AppendedActorMesh {
            render: SceneActorRenderRange {
                vertices: first_vertex..render_mesh.positions.len(),
                indices: first_index..render_mesh.indices.len(),
            },
            animation,
        }),
    )
}

fn select_actor_texture(
    actor: &ActorState,
    mesh_textures: &[Option<SceneObject>],
    texture_index: i32,
) -> Option<SceneObject> {
    let index = usize::try_from(texture_index).ok()?;
    if let Some(texture) = actor.multi_skins.get(index).and_then(Clone::clone) {
        return Some(texture);
    }
    let mesh_texture = mesh_textures.get(index).and_then(Clone::clone);
    if (mesh_texture.is_none() || index == 0) && actor.skin.is_some() {
        return actor.skin.clone();
    }
    mesh_texture.or_else(|| actor.texture.clone())
}

#[allow(clippy::too_many_arguments)]
fn actor_surface_material(
    packages: &mut PackageStore,
    texture: Option<&SceneObject>,
    mut flags: u32,
    actor: &ActorState,
    textures: &mut Vec<TextureImage>,
    decoded: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
    water_animations: &mut Vec<AnimatedWaterTexture>,
) -> SurfaceMaterial {
    flags |= match actor.style {
        2 => 0x0000_0002,
        3 => 0x0000_0004,
        4 => 0x0000_0040,
        _ => 0,
    };
    if actor.unlit {
        flags |= 0x0040_0000;
    }
    let flags = PolyFlags::from_bits(flags);
    let Some(texture) = texture else {
        return surface_material(flags, None, None);
    };
    let key = texture.id();
    if !decoded.contains_key(&key) {
        let resolved = ResolvedObject {
            package: Arc::clone(&texture.package),
            export_index: texture.export_index,
        };
        let value = match decode_texture(packages, &resolved) {
            Ok(texture) => Some(texture),
            Err(error) => {
                warn!(%error, "could not decode actor texture");
                None
            }
        };
        decoded.insert(key.clone(), value);
    }
    let Some(texture) = decoded.get(&key).and_then(Option::as_ref) else {
        return surface_material(flags, None, None);
    };
    let mut material = surface_material(flags, None, Some(texture.texture.render_flags));
    let image_key = (key.package, key.export_index, material.masked);
    let image = if let Some(index) = images.get(&image_key) {
        Some(*index)
    } else {
        match append_texture_image(textures, water_animations, texture, material.masked) {
            Ok(index) => {
                images.insert(image_key, index);
                Some(index)
            }
            Err(error) => {
                warn!(%error, "could not expand actor texture");
                None
            }
        }
    };
    material.texture = image;
    material
}

fn rotation_matrix(rotation: Rotator) -> Mat4 {
    let radians = rotation.radians();
    Mat4::from_rotation_z(radians.y)
        * Mat4::from_rotation_y(-radians.x)
        * Mat4::from_rotation_x(-radians.z)
}

fn pawn_mesh_offset(
    is_pawn: bool,
    is_skeletal_mesh: bool,
    physics: u8,
    collision_height: f32,
    mesh_origin_z: f32,
) -> Vec3 {
    if is_pawn
        && is_skeletal_mesh
        && matches!(
            physics,
            PHYS_WALKING | PHYS_FALLING | PHYS_SWIMMING | PHYS_FLYING
        )
    {
        Vec3::new(0.0, 0.0, mesh_origin_z - collision_height)
    } else {
        Vec3::ZERO
    }
}

fn load_materials(
    packages: &mut PackageStore,
    map: &std::sync::Arc<openhp1_package::Package>,
    model: &Model,
    zone_pan_speeds: &[Vec2],
    water_animations: &mut Vec<AnimatedWaterTexture>,
) -> (Vec<TextureImage>, Vec<SurfaceMaterial>) {
    let mut textures = Vec::new();
    let mut decoded = HashMap::<(String, usize), Option<DecodedTexture>>::new();
    let mut images = HashMap::<(String, usize, bool), usize>::new();
    let mut materials = Vec::with_capacity(model.surfaces.len());
    // ponytail: materials are surface-wide; carry pan speed per BSP node if a
    // map gives one shared surface different front-zone speeds.
    let mut surface_pan_speeds = vec![None; model.surfaces.len()];
    let level_pan_speed = zone_pan_speeds.first().copied().unwrap_or(Vec2::ONE);
    for node in &model.nodes {
        let (Ok(surface), Ok(zone)) = (
            usize::try_from(node.surface),
            usize::try_from(node.zones[1]),
        ) else {
            continue;
        };
        if let Some(speed) = surface_pan_speeds.get_mut(surface)
            && speed.is_none()
        {
            *speed = Some(
                zone_pan_speeds
                    .get(zone)
                    .copied()
                    .unwrap_or(level_pan_speed),
            );
        }
    }

    for (surface_index, surface) in model.surfaces.iter().enumerate() {
        let pan_speed = surface_pan_speeds[surface_index].unwrap_or(level_pan_speed);
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
                materials.push(bsp_surface_material(
                    surface.poly_flags,
                    None,
                    None,
                    pan_speed,
                ));
                continue;
            }
            Err(error) => {
                warn!(surface_index, %error, "could not resolve surface texture");
                materials.push(bsp_surface_material(
                    surface.poly_flags,
                    None,
                    None,
                    pan_speed,
                ));
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
            materials.push(bsp_surface_material(
                surface.poly_flags,
                None,
                None,
                pan_speed,
            ));
            continue;
        };
        let texture_flags = decoded_texture.texture.render_flags;
        let material =
            bsp_surface_material(surface.poly_flags, None, Some(texture_flags), pan_speed);
        let image_key = (key.0.clone(), key.1, material.masked);
        let texture_index = if let Some(index) = images.get(&image_key) {
            *index
        } else {
            let index = match append_texture_image(
                &mut textures,
                water_animations,
                decoded_texture,
                material.masked,
            ) {
                Ok(index) => index,
                Err(error) => {
                    warn!(surface_index, %error, "could not expand surface texture");
                    materials.push(material);
                    continue;
                }
            };
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

fn load_zone_pan_speeds(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    model: &Model,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> Vec<Vec2> {
    let level_pan_speed = map
        .summary()
        .exports
        .iter()
        .position(|export| map.summary().class_name(export) == Some("LevelInfo"))
        .and_then(
            |export_index| match actor_pan_speed(packages, map, export_index, class_cache) {
                Ok(speed) => Some(speed),
                Err(error) => {
                    warn!(export_index, %error, "could not decode LevelInfo texture pan speed");
                    None
                }
            },
        )
        .unwrap_or(Vec2::ONE);

    model
        .zones
        .iter()
        .enumerate()
        .map(|(zone_index, zone)| {
            let ObjectReference::Export(export_index) = zone.actor else {
                return level_pan_speed;
            };
            actor_pan_speed(packages, map, export_index, class_cache).unwrap_or_else(|error| {
                warn!(zone_index, export_index, %error, "could not decode zone texture pan speed");
                level_pan_speed
            })
        })
        .collect()
}

fn actor_pan_speed(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    export_index: usize,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> Result<Vec2> {
    let actor = Actor::decode(map, export_index)?;
    let export = map
        .summary()
        .exports
        .get(export_index)
        .context("zone actor export is missing")?;
    let class = packages
        .resolve(map, export.class)?
        .map(SceneObject::from)
        .context("zone actor class is missing")?;
    let mut state = class_state(packages, &class, class_cache, 0).actor;
    state.apply(packages, map, &actor.properties)?;
    ensure!(
        state.texture_pan_speed.is_finite(),
        "zone texture pan speed is not finite"
    );
    Ok(state.texture_pan_speed)
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
    let water = if let Some(wet) = &texture.wet {
        let source = packages
            .resolve(&resolved.package, wet.source_texture)?
            .context("wet texture has no source texture")?;
        let source = Texture::decode(&source.package, source.export_index)?;
        let source = source
            .mips
            .first()
            .context("wet texture source has no mip levels")?;
        ensure!(
            source.width == mip.width && source.height == mip.height,
            "wet texture source is {}x{}, expected {}x{}",
            source.width,
            source.height,
            mip.width,
            mip.height,
        );
        Some(wet.animate(mip.width, mip.height, &source.indices)?)
    } else {
        None
    };
    Ok(DecodedTexture {
        texture,
        palette,
        water,
    })
}

struct DecodedTexture {
    texture: Texture,
    palette: Palette,
    water: Option<WaterAnimation>,
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
            rgba: match &self.water {
                Some(water) => water.rgba(&self.palette, masked)?,
                None => self.texture.rgba(0, &self.palette, masked)?,
            },
        })
    }
}

fn append_texture_image(
    textures: &mut Vec<TextureImage>,
    water_animations: &mut Vec<AnimatedWaterTexture>,
    decoded: &DecodedTexture,
    masked: bool,
) -> Result<usize> {
    let index = textures.len();
    textures.push(decoded.image(masked)?);
    if let Some(animation) = &decoded.water {
        water_animations.push(AnimatedWaterTexture {
            texture: index,
            masked,
            palette: decoded.palette.clone(),
            animation: animation.clone(),
        });
    }
    Ok(index)
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
        texture_pan_speed: [0.0; 2],
    }
}

fn bsp_surface_material(
    flags: PolyFlags,
    texture: Option<usize>,
    texture_flags: Option<TextureRenderFlags>,
    zone_pan_speed: Vec2,
) -> SurfaceMaterial {
    let mut material = surface_material(flags, texture, texture_flags);
    material.texture_pan_speed = [
        if flags.contains(PolyFlags::AUTO_U_PAN) {
            zone_pan_speed.x
        } else {
            0.0
        },
        if flags.contains(PolyFlags::AUTO_V_PAN) {
            zone_pan_speed.y
        } else {
            0.0
        },
    ];
    material
}

fn is_hidden(flags: PolyFlags, texture_flags: TextureRenderFlags) -> bool {
    flags.contains(PolyFlags::INVISIBLE) || texture_flags.invisible
}

#[cfg(test)]
mod tests {
    use openhp1_map::PolyFlags;
    use openhp1_texture::TextureRenderFlags;

    use crate::SurfaceMode;

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

    #[test]
    fn applies_only_requested_zone_texture_pan_axes() {
        let material = super::bsp_surface_material(
            PolyFlags::AUTO_V_PAN,
            Some(1),
            None,
            glam::Vec2::new(2.0, 3.5),
        );
        assert_eq!(material.texture_pan_speed, [0.0, 3.5]);
    }

    #[test]
    fn translates_actor_vertices_in_unreal_space() {
        let mut positions = [glam::Vec3::ZERO, glam::Vec3::ONE];
        super::translate_positions(&mut positions, glam::Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(
            positions,
            [
                glam::Vec3::new(2.0, 3.0, 4.0),
                glam::Vec3::new(3.0, 4.0, 5.0)
            ]
        );
    }

    #[test]
    fn aligns_only_skeletal_pawns_to_their_collision_feet() {
        assert_eq!(
            super::pawn_mesh_offset(true, true, 1, 50.0, 0.0),
            glam::Vec3::new(0.0, 0.0, -50.0)
        );
        assert_eq!(
            super::pawn_mesh_offset(true, true, 1, 40.0, 42.0),
            glam::Vec3::new(0.0, 0.0, 2.0)
        );
        assert_eq!(
            super::pawn_mesh_offset(true, true, 0, 42.0, 0.0),
            glam::Vec3::ZERO
        );
        assert_eq!(
            super::pawn_mesh_offset(false, true, 1, 50.0, 0.0),
            glam::Vec3::ZERO
        );
        assert_eq!(
            super::pawn_mesh_offset(true, false, 1, 50.0, 0.0),
            glam::Vec3::ZERO
        );
    }

    #[test]
    fn rotates_actor_vertices_around_their_unreal_origin() {
        let origin = glam::Vec3::new(10.0, 0.0, 0.0);
        let mut positions = [glam::Vec3::new(11.0, 0.0, 0.0)];
        let transform = super::rotation_delta(
            origin,
            Default::default(),
            openhp1_map::Rotator {
                yaw: 16_384,
                ..Default::default()
            },
        );
        super::transform_positions(&mut positions, transform);
        assert!(positions[0].abs_diff_eq(glam::Vec3::new(10.0, 1.0, 0.0), 1.0e-5));
    }

    #[test]
    fn collapses_destroyed_actor_vertices() {
        let mut positions = [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z];
        assert!(super::collapse_positions(&mut positions));
        assert_eq!(positions, [glam::Vec3::X; 3]);
    }

    #[test]
    fn one_shot_animations_finish_while_loops_wrap() {
        let one_shot = super::advance_animation(0.5, 1.0, 0.5, false, 4);
        assert_eq!(one_shot, (0.75, true, false));

        let looping = super::advance_animation(0.5, 1.0, 0.3, true, 4);
        assert_eq!(looping, (0.75, true, true));
        let wrapped = super::advance_animation(0.75, 1.0, 0.3, true, 4);
        assert!((wrapped.0 - 0.05).abs() < f32::EPSILON);
        assert_eq!((wrapped.1, wrapped.2), (false, true));
    }

    #[test]
    fn rotates_actor_axes_using_ue1_rotator_direction() {
        let quarter_turn = 16_384;
        let yaw = super::rotation_matrix(openhp1_map::Rotator {
            yaw: quarter_turn,
            ..Default::default()
        });
        let pitch = super::rotation_matrix(openhp1_map::Rotator {
            pitch: quarter_turn,
            ..Default::default()
        });
        let roll = super::rotation_matrix(openhp1_map::Rotator {
            roll: quarter_turn,
            ..Default::default()
        });
        assert!(
            yaw.transform_vector3(glam::Vec3::X)
                .abs_diff_eq(glam::Vec3::Y, 1.0e-6)
        );
        assert!(
            pitch
                .transform_vector3(glam::Vec3::X)
                .abs_diff_eq(glam::Vec3::Z, 1.0e-6)
        );
        assert!(
            roll.transform_vector3(glam::Vec3::Y)
                .abs_diff_eq(-glam::Vec3::Z, 1.0e-6)
        );
    }
}
