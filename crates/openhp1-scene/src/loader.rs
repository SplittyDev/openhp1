use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
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
use openhp1_script::class_defaults_reader;
use openhp1_texture::{Palette, Texture, TextureRenderFlags};
use tracing::{info, warn};

use crate::{
    RenderScene, Rotator, SceneActor, SceneActorAnimation, SceneActorRenderRange, SceneObjectId,
    SurfaceMaterial, SurfaceMode, TextureImage, render_to_unreal,
};

const NOT_FOR_SERVER: u32 = 0x0020_0000;

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
    zone_nodes: Vec<BspNode>,
    zone_count: usize,
    animations: Vec<AnimatedActorMesh>,
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
        let (mut textures, mut surface_materials) = load_materials(&mut packages, &package, &model);
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
        let actors = load_actors(
            &mut packages,
            &package,
            &level,
            &model,
            &vertex_lighting,
            &mut mesh,
            &mut textures,
            &mut surface_materials,
            &mut animations,
        );
        let actor_meshes = actors.iter().filter(|actor| actor.render.is_some()).count();
        let animated_actor_meshes = actors
            .iter()
            .filter(|actor| actor.animation.is_some())
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
            actor_meshes,
            animated_actor_meshes,
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
            points: model.points.len(),
            nodes: model.nodes.len(),
            surfaces: model.surfaces.len(),
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
            zone_nodes: model.nodes.clone(),
            zone_count: model.zones.len(),
            animations,
        })
    }

    pub fn zone_at(&self, render_position: Vec3) -> usize {
        bsp_zone_at(
            &self.zone_nodes,
            self.zone_count,
            render_to_unreal(render_position),
        )
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
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &vertices {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }

        self.actors[actor_index].location = location;
        if let Some(vertices) = vertices {
            translate_positions(&mut self.render.mesh.positions[vertices], delta);
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

    pub fn destroy_actor(&mut self, actor_index: usize) -> Result<bool> {
        let actor = self
            .actors
            .get_mut(actor_index)
            .context("runtime refers to a missing scene actor")?;
        actor.hidden = true;
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
        if delta_time <= 0.0 || !delta_time.is_finite() {
            return Ok(false);
        }
        let mut changed = false;
        for animation in &mut self.animations {
            if !animation.playing || animation.rate == 0.0 {
                continue;
            }
            changed = true;
            animation.phase = (animation.phase + animation.rate * delta_time).rem_euclid(1.0);
            let actor = self
                .actors
                .get_mut(animation.actor_index)
                .context("animation refers to a missing scene actor")?;
            let actor_animation = actor
                .animation
                .as_mut()
                .context("animated scene actor has no animation state")?;
            actor_animation.phase = animation.phase;
            let triangles = animation.sample()?;
            ensure!(
                triangles.len() * 3 == animation.vertices.len(),
                "animation changed actor vertex count"
            );
            for (destination, vertex) in animation
                .vertices
                .clone()
                .zip(triangles.into_iter().flat_map(|triangle| triangle.vertices))
            {
                let position = animation.transform.transform_point3(vertex.position);
                let normal = (animation.normal_transform * vertex.normal).normalize_or_zero();
                self.render.mesh.positions[destination] = position;
                self.render.mesh.vertex_colors[destination] =
                    animation.lighting.color(position, normal, animation.unlit);
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
        ensure!(relative_rate.is_finite(), "animation rate is not finite");
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
        let source = &animation.sequences()[sequence];
        let source_name = source.name.clone();
        let source_rate = source.rate;
        let source_frames = source.frame_count;
        animation.sequence = sequence;
        animation.phase = 0.0;
        animation.rate = relative_rate * source_rate / source_frames.max(1) as f32;
        animation.playing = true;
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

fn translate_positions(positions: &mut [Vec3], delta: Vec3) {
    for position in positions {
        *position += delta;
    }
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
    vertices: Range<usize>,
    transform: Mat4,
    normal_transform: Mat3,
    lighting: ActorVertexLighting,
    unlit: bool,
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
    hidden: bool,
    unlit: bool,
}

#[derive(Clone)]
struct ClassState {
    actor: ActorState,
    diagnostics: Vec<String>,
}

impl Default for ActorState {
    fn default() -> Self {
        Self {
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            pre_pivot: Vec3::ZERO,
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
        if let Some(hidden) = properties.hidden {
            self.hidden = hidden;
        }
        if let Some(unlit) = properties.unlit {
            self.unlit = unlit;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn load_actors(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    level: &Level,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    animations: &mut Vec<AnimatedActorMesh>,
) -> Vec<SceneActor> {
    let mut class_cache = HashMap::<SceneObjectId, ClassState>::new();
    let mut mesh_cache = HashMap::<SceneObjectId, Option<Arc<Mesh>>>::new();
    let mut animation_cache = HashMap::<SceneObjectId, Option<Arc<SkeletalAnimation>>>::new();
    let mut decoded_textures = HashMap::<SceneObjectId, Option<DecodedTexture>>::new();
    let mut images = HashMap::<(String, usize, bool), usize>::new();
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
        let class = match packages.resolve(map, export.class) {
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
        let class_state = class_state(packages, &class, &mut class_cache, 0);
        scene_actor.diagnostics.extend(class_state.diagnostics);
        let mut state = class_state.actor;
        if let Err(error) = state.apply(packages, map, &actor.properties) {
            warn!(export_index, %error, "could not resolve actor properties");
            scene_actor
                .diagnostics
                .push(format!("actor property resolution failed: {error}"));
            actors.push(scene_actor);
            continue;
        }
        apply_scene_actor_state(&mut scene_actor, &state);
        if state.hidden || state.draw_type != 2 {
            scene_actor.diagnostics.push(if state.hidden {
                "hidden by actor state".to_owned()
            } else {
                format!("DrawType {} is not rendered as a mesh", state.draw_type)
            });
            actors.push(scene_actor);
            continue;
        }
        let Some(mesh_object) = state.mesh.clone() else {
            scene_actor
                .diagnostics
                .push("mesh draw type has no mesh assigned".to_owned());
            actors.push(scene_actor);
            continue;
        };
        let mesh_key = mesh_object.id();
        if !mesh_cache.contains_key(&mesh_key) {
            let decoded = match Mesh::decode(&mesh_object.package, mesh_object.export_index) {
                Ok(mesh) => Some(Arc::new(mesh)),
                Err(error) => {
                    warn!(
                        actor = %map.summary().name(export.object_name),
                        %error,
                        "could not decode actor mesh"
                    );
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
            actors.push(scene_actor);
            continue;
        };
        let animation_object = if let Some(animation) = state.skeletal_animation.clone() {
            Some(animation)
        } else {
            match packages.resolve(&mesh_object.package, mesh.default_animation) {
                Ok(animation) => animation.map(SceneObject::from),
                Err(error) => {
                    warn!(
                        actor = %map.summary().name(export.object_name),
                        %error,
                        "could not resolve actor skeletal animation"
                    );
                    None
                }
            }
        };
        let skeletal_animation = match animation_object {
            Some(animation_object) => {
                let key = animation_object.id();
                if !animation_cache.contains_key(&key) {
                    let decoded = match SkeletalAnimation::decode(
                        &animation_object.package,
                        animation_object.export_index,
                    ) {
                        Ok(animation) => Some(Arc::new(animation)),
                        Err(error) => {
                            warn!(
                                actor = %map.summary().name(export.object_name),
                                %error,
                                "could not decode actor skeletal animation"
                            );
                            None
                        }
                    };
                    animation_cache.insert(key.clone(), decoded);
                }
                animation_cache.get(&key).and_then(Option::as_ref).cloned()
            }
            None => None,
        };
        let actor_index = actors.len();
        match append_actor_mesh(
            packages,
            &mesh_object,
            &mesh,
            skeletal_animation.as_ref(),
            &state,
            actor_index,
            model,
            vertex_lighting,
            render_mesh,
            textures,
            materials,
            &mut decoded_textures,
            &mut images,
            animations,
        ) {
            Ok(Some(appended)) => {
                scene_actor.render = Some(appended.render);
                scene_actor.animation = appended.animation;
            }
            Ok(None) => {
                scene_actor
                    .diagnostics
                    .push("mesh contains no renderable triangles".to_owned());
            }
            Err(error) => {
                warn!(
                    actor = %map.summary().name(export.object_name),
                    %error,
                    "could not append actor mesh"
                );
                scene_actor
                    .diagnostics
                    .push(format!("mesh assembly failed: {error}"));
            }
        }
        actors.push(scene_actor);
    }
    actors
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
                diagnostics: vec![error],
            };
            cache.insert(key, state.clone());
            return state;
        }
    };
    let mut state = match packages.resolve(&class.package, base) {
        Ok(Some(base)) => ClassState {
            actor: class_state(packages, &SceneObject::from(base), cache, depth + 1).actor,
            diagnostics: Vec::new(),
        },
        Ok(None) => ClassState {
            actor: ActorState::default(),
            diagnostics: Vec::new(),
        },
        Err(error) => {
            let error = format!("base class resolution failed for {}: {error}", class.name());
            ClassState {
                actor: ActorState::default(),
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
    actor_index: usize,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    decoded_textures: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
    animations: &mut Vec<AnimatedActorMesh>,
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

fn actor_surface_material(
    packages: &mut PackageStore,
    texture: Option<&SceneObject>,
    mut flags: u32,
    actor: &ActorState,
    textures: &mut Vec<TextureImage>,
    decoded: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
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
        match texture.image(material.masked) {
            Ok(image) => {
                let index = textures.len();
                textures.push(image);
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
    fn collapses_destroyed_actor_vertices() {
        let mut positions = [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z];
        assert!(super::collapse_positions(&mut positions));
        assert_eq!(positions, [glam::Vec3::X; 3]);
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
