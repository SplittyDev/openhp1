use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec2, Vec3};
use openhp1_map::{
    Actor, ActorProperties, ActorVertexLighting, BrushPolys, BspNode, Level, Model, PolyFlags,
    TriangleMesh, VertexLighting, bsp_zone_at, hsb_to_rgb,
};
use openhp1_mesh::{Mesh, MeshAnimationSequence, SkeletalAnimation};
use openhp1_package::{ObjectReference, Package, PackageStore, ResolveError, ResolvedObject};
use openhp1_physics::BspCollision;
use openhp1_runtime::{
    ParticleColor, ParticleEmitter, ParticleFloat, ParticleWind, RuntimeObject, WeaponAttachment,
};
use openhp1_script::class_defaults_reader;
use openhp1_texture::{IceAnimation, Palette, Texture, TextureRenderFlags, WaterAnimation};
use tracing::{info, warn};

use crate::{
    ActorSubmission, Corona, RenderLight, RenderLightmap, RenderScene, Rotator, SceneActor,
    SceneActorAnimation, SceneActorRenderRange, SceneObjectId, SurfaceMaterial, SurfaceMode,
    TextureImage, WarpCoordinates, WarpPortal, render::light_direction, render_to_unreal,
    unreal_to_render,
};

mod runtime_display;
mod runtime_light;

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
    actor_states: Vec<ActorRenderState>,
    collision: Arc<BspCollision>,
    zone_nodes: Vec<BspNode>,
    zone_count: usize,
    animations: Vec<AnimatedActorMesh>,
    sprites: Vec<SpriteActor>,
    root_motions: Vec<(usize, Vec3)>,
    hidden_actor_positions: HashMap<usize, Vec<Vec3>>,
    attached_weapons: HashMap<usize, SceneObject>,
    water_animations: TextureAnimations,
    changed_lightmaps: Vec<usize>,
    particles: HashMap<usize, ParticleSystem>,
    particle_view_rotation: Mat4,
    actor_render: ActorRenderContext,
}

struct ActorRenderContext {
    packages: PackageStore,
    map: Arc<Package>,
    model: Model,
    level_environment_map: Option<SceneObject>,
    zone_environment_maps: Vec<Option<SceneObject>>,
    vertex_lighting: VertexLighting,
    light_brightnesses: HashMap<usize, u8>,
    class_cache: HashMap<SceneObjectId, ClassState>,
    mesh_cache: HashMap<SceneObjectId, Option<Arc<Mesh>>>,
    brush_cache: HashMap<SceneObjectId, Option<Arc<BrushPolys>>>,
    animation_cache: HashMap<SceneObjectId, std::result::Result<Arc<SkeletalAnimation>, String>>,
    decoded_textures: HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: HashMap<(String, usize, bool), usize>,
}

enum ActorAnimationSource {
    Legacy,
    Skeletal(Arc<SkeletalAnimation>),
    Error(String),
}

impl ActorAnimationSource {
    fn sequences<'a>(
        &'a self,
        mesh: &'a Mesh,
    ) -> std::result::Result<&'a [MeshAnimationSequence], String> {
        match self {
            Self::Legacy => Ok(&mesh.animation_sequences),
            Self::Skeletal(animation) => Ok(&animation.sequences),
            Self::Error(error) => Err(error.clone()),
        }
    }
}

impl LoadedScene {
    pub fn config_value(&self, section: &str, key: &str) -> Option<String> {
        self.actor_render.packages.config_value(section, key)
    }

    pub fn config_value_in(&self, config: &str, section: &str, key: &str) -> Option<String> {
        self.actor_render
            .packages
            .config_values(config, section, key)
            .into_iter()
            .next()
    }

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
        let (level_pan_speed, zone_pan_speeds) =
            load_zone_pan_speeds(&mut packages, &package, &level, &model, &mut class_cache);
        let (level_environment_map, zone_environment_maps) =
            load_environment_maps(&mut packages, &package, &level, &model, &mut class_cache);
        let level_default_texture = level.level_info_export().and_then(|export_index| {
            actor_default_texture(&mut packages, &package, export_index, &mut class_cache)
                .inspect_err(|error| {
                    warn!(export_index, %error, "could not decode LevelInfo default texture");
                })
                .ok()
                .flatten()
        });
        mesh.texture_pan_speeds = bsp_texture_pan_speeds(&model, level_pan_speed, &zone_pan_speeds);
        let mut water_animations = TextureAnimations::default();
        let (mut textures, mut surface_materials) = load_materials(
            &mut packages,
            &package,
            &model,
            level_default_texture.as_ref(),
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
        let mut sprites = Vec::new();
        let mut coronas = Vec::new();
        let vertex_lighting = model
            .vertex_lighting(&package)
            .context("failed to decode actor vertex lighting")?;
        let mut actor_render = ActorRenderContext {
            packages,
            map: Arc::clone(&package),
            model,
            level_environment_map,
            zone_environment_maps,
            vertex_lighting,
            light_brightnesses: HashMap::new(),
            class_cache,
            mesh_cache: HashMap::new(),
            brush_cache: HashMap::new(),
            animation_cache: HashMap::new(),
            decoded_textures: HashMap::new(),
            images: HashMap::new(),
        };
        let (actors, actor_states) = load_actors(
            &mut actor_render,
            &package,
            &level,
            &mut mesh,
            &mut textures,
            &mut surface_materials,
            &mut coronas,
            &mut animations,
            &mut sprites,
            &mut water_animations,
        );
        let actor_indices = actors
            .iter()
            .enumerate()
            .filter(|(_, actor)| actor.id.package == package.summary().source.as_ref())
            .map(|(index, actor)| (actor.id.export_index, index))
            .collect::<HashMap<_, _>>();
        let warp_portals = load_warp_portals(
            &package,
            &actor_render.model,
            &surface_materials,
            &actors,
            &actor_indices,
        )?;
        let visible_light_sources = sprites
            .iter()
            .filter(|sprite| actor_states[sprite.actor_index].is_light)
            .map(|sprite| (sprite.actor_index, sprite.texture))
            .collect::<HashMap<_, _>>();
        let realtime_lightmaps = actor_render
            .model
            .authored_lightmaps(&package)
            .context("failed to decode authored real-time lighting")?
            .into_iter()
            .map(|lightmap| RenderLightmap {
                ambient: lightmap.ambient,
                lights: lightmap
                    .lights
                    .into_iter()
                    .map(|light| RenderLight {
                        actor_index: actor_indices
                            .get(&light.export_index)
                            .copied()
                            .unwrap_or(usize::MAX),
                        source_texture: actor_indices
                            .get(&light.export_index)
                            .and_then(|index| visible_light_sources.get(index))
                            .copied(),
                        location: unreal_to_render(light.location),
                        direction: unreal_to_render(light_direction(light.rotation))
                            .normalize_or_zero(),
                        effect: light.effect,
                        brightness: light.brightness,
                        hue: light.hue,
                        saturation: light.saturation,
                        radius: light.radius,
                        cone: light.cone,
                        dark: light.dark,
                        volume_brightness: light.volume_brightness,
                        volume_fog: light.volume_fog,
                        volume_radius: light.volume_radius,
                        visibility: light.visibility,
                    })
                    .collect(),
            })
            .collect();
        let mut hidden_actor_positions = HashMap::new();
        for (actor_index, actor) in actors
            .iter()
            .enumerate()
            .filter(|(_, actor)| actor.hidden || actor.draw_type == 0)
        {
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
        let actor_submissions = actors
            .iter()
            .zip(&actor_states)
            .enumerate()
            .filter_map(|(actor_index, (actor, state))| {
                actor.render.as_ref().map(|render| ActorSubmission {
                    actor_index,
                    indices: render.indices.clone(),
                    translucent_pass: state.actor.style == 3 || state.actor.opacity < 1.0,
                })
            })
            .collect();
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
            animated_water_textures = water_animations.water.len(),
            animated_ice_textures = water_animations.ice.len(),
            animated_generic_textures = water_animations.generic.len(),
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
                realtime_lightmaps,
                coronas,
                actor_submissions,
                surface_materials,
                warp_portals,
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
            actor_states,
            collision,
            zone_nodes: actor_render.model.nodes.clone(),
            zone_count: actor_render.model.zones.len(),
            animations,
            sprites,
            root_motions: Vec::new(),
            hidden_actor_positions,
            attached_weapons: HashMap::new(),
            water_animations,
            changed_lightmaps: Vec::new(),
            particles: HashMap::new(),
            particle_view_rotation: Mat4::IDENTITY,
            actor_render,
        })
    }

    pub fn collision(&self) -> Arc<BspCollision> {
        Arc::clone(&self.collision)
    }

    pub fn set_warp_destination(
        &mut self,
        source_actor: usize,
        destination: Option<RuntimeObject>,
    ) -> Result<bool> {
        self.actors
            .get(source_actor)
            .context("runtime refers to a missing warp-zone actor")?;
        let destination_actor = destination
            .as_ref()
            .map(|destination| {
                self.actors
                    .iter()
                    .position(|actor| {
                        actor.id.package == destination.package.as_ref()
                            && actor.id.export_index == destination.export_index
                    })
                    .context("warp destination is not a scene actor")
            })
            .transpose()?;
        let destination_coordinates = destination_actor
            .map(|actor| {
                self.render
                    .warp_portals
                    .iter()
                    .find(|portal| portal.source_actor == actor)
                    .map(|portal| portal.source)
                    .context("warp destination has no authored coordinates")
            })
            .transpose()?;
        let mut changed = false;
        for portal in self
            .render
            .warp_portals
            .iter_mut()
            .filter(|portal| portal.source_actor == source_actor)
        {
            changed |= portal.destination_actor != destination_actor
                || portal.destination != destination_coordinates;
            portal.destination_actor = destination_actor;
            portal.destination = destination_coordinates;
        }
        Ok(changed)
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
            self.actor_states.push(ActorRenderState::default());
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
        let is_light = class_state.is_light;
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
            main_scale: Vec3::ONE,
            draw_scale: 1.0,
            draw_type: 0,
            hidden: false,
            unlit: false,
            brush: None,
            mesh: None,
            mesh_name: None,
            animation: None,
            render: None,
            mesh_transform: None,
            mesh_to_object: None,
            visual_bounds: None,
            diagnostics: class_state.diagnostics,
        };
        apply_scene_actor_state(&mut actor, &state);
        append_scene_actor_corona(
            &mut self.actor_render,
            actor_index,
            &state,
            &mut self.render.textures,
            &mut self.render.coronas,
            &mut self.water_animations,
        );
        append_scene_actor_render(
            &mut self.actor_render,
            &mut actor,
            &state,
            is_light,
            actor_index,
            &mut self.render.mesh,
            &mut self.render.textures,
            &mut self.render.surface_materials,
            &mut self.animations,
            &mut self.sprites,
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
        let render_state = ActorRenderState {
            actor: state,
            is_light,
        };
        if actor_index == self.actors.len() {
            self.actors.push(actor);
            self.actor_states.push(render_state);
        } else {
            ensure!(
                self.actors[actor_index].id.package == "<runtime>"
                    && self.actors[actor_index].class.is_none(),
                "runtime actor index {actor_index} already exists in the scene"
            );
            self.actors[actor_index] = actor;
            self.actor_states[actor_index] = render_state;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_actor_animation(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
        tween_time: f32,
        looping: bool,
        tween_only: bool,
        root_motion: bool,
        phase: f32,
    ) -> Result<bool> {
        ensure!(phase.is_finite(), "animation phase is not finite");
        let played = self.set_actor_animation(
            actor_index,
            sequence_name,
            relative_rate,
            tween_time,
            looping,
            root_motion,
        )?;
        if !played {
            return Ok(false);
        }
        let animation = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
            .expect("played animation is present");
        animation.phase = phase;
        animation.playing = !tween_only || tween_time > 0.0;
        animation.looping = looping;
        animation.root_motion = root_motion;
        let sample = animation.sample()?;
        let bone_positions = animation.bone_positions_from(&sample);
        animation.bone_positions = bone_positions;
        if root_motion {
            animation.root_motion_position =
                animation.transform.transform_vector3(sample.root_motion);
        }
        if let Some(actor) = self.actors.get_mut(actor_index)
            && let Some(actor_animation) = actor.animation.as_mut()
        {
            actor_animation.phase = phase;
            actor_animation.rate = animation.rate;
        }
        Ok(true)
    }

    pub fn set_actor_animation_frame(&mut self, actor_index: usize, frame: f32) -> Result<bool> {
        ensure!(
            frame.is_finite() && frame >= 0.0,
            "animation frame is invalid"
        );
        let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        else {
            return Ok(false);
        };
        animation.phase = frame;
        animation.tween_from = None;
        animation.tween_attachment_from = None;
        animation.tween_bone_positions_from = None;
        let sample = animation.sample()?;
        animation.bone_positions = animation.bone_positions_from(&sample);
        if animation.root_motion {
            animation.root_motion_position =
                animation.transform.transform_vector3(sample.root_motion);
        }
        if let Some(actor_animation) = self
            .actors
            .get_mut(actor_index)
            .and_then(|actor| actor.animation.as_mut())
        {
            actor_animation.phase = frame;
        }
        Ok(true)
    }

    pub fn ensure_runtime_actor(&mut self, actor_index: usize) {
        while self.actors.len() <= actor_index {
            self.actors
                .push(runtime_actor_placeholder(self.actors.len()));
            self.actor_states.push(ActorRenderState::default());
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
        let collapsed = self.hidden_actor_positions.contains_key(&actor_index);
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &vertices {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }

        self.actors[actor_index].location = location;
        self.actor_states[actor_index].actor.location = location;
        if self.actor_states[actor_index].is_light {
            self.render.set_light_location(actor_index, location);
        }
        for corona in self
            .render
            .coronas
            .iter_mut()
            .filter(|corona| corona.actor_index == actor_index)
        {
            corona.location = location;
        }
        if let Some(transform) = &mut self.actors[actor_index].mesh_transform {
            *transform = Mat4::from_translation(delta) * *transform;
        }
        if let Some(vertices) = vertices {
            if collapsed {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&actor_index)
                    .context("collapsed actor has no saved render positions")?;
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
            let transform = Mat4::from_translation(delta);
            animation.transform = transform * animation.transform;
            transform_animation_pose_positions(
                &mut animation.bone_positions,
                animation.tween_from.as_deref_mut(),
                animation.tween_bone_positions_from.as_deref_mut(),
                transform,
            );
        }
        // ponytail: retain baked actor lighting until moving lights/zones are observable.
        Ok(true)
    }

    pub fn update_sprite_billboards(&mut self, view_rotation: Rotator) -> bool {
        let rotation = rotation_matrix(view_rotation);
        self.particle_view_rotation = rotation;
        let mut changed = false;
        for sprite in &self.sprites {
            let Some(actor) = self.actors.get(sprite.actor_index) else {
                continue;
            };
            let Some(render) = actor.render.as_ref() else {
                continue;
            };
            if actor.hidden {
                continue;
            }
            let center = actor.location + actor.pre_pivot;
            let positions = sprite_positions(center, sprite.half_size, rotation);
            let target = &mut self.render.mesh.positions[render.vertices.clone()];
            if target != positions {
                target.copy_from_slice(&positions);
                changed = true;
            }
        }
        changed
    }

    pub fn sync_particle_emitters(&mut self, emitters: Vec<ParticleEmitter>) -> Result<bool> {
        let mut changed = false;
        let active = emitters
            .iter()
            .map(|emitter| emitter.actor)
            .collect::<HashSet<_>>();
        let inactive = self
            .particles
            .keys()
            .filter(|actor| !active.contains(actor))
            .copied()
            .collect::<Vec<_>>();
        for actor in inactive {
            if let Some(system) = self.particles.remove(&actor) {
                self.render.mesh.positions[system.vertices.clone()].fill(Vec3::ZERO);
                let _ = remove_particle_submission_range(
                    &mut self.render.actor_submissions,
                    actor,
                    &system.indices,
                );
                changed = true;
            }
        }
        for emitter in emitters {
            if let Some(actor) = self.actors.get_mut(emitter.actor) {
                for diagnostic in emitter.capability_diagnostics() {
                    if !actor
                        .diagnostics
                        .iter()
                        .any(|existing| existing == diagnostic)
                    {
                        warn!(
                            actor = emitter.actor,
                            actor_name = actor.name,
                            class = actor.class_name,
                            diagnostic,
                            "particle capability diagnostic"
                        );
                        actor.diagnostics.push(diagnostic.to_owned());
                    }
                }
            }
            if emitter.textures.is_empty() {
                continue;
            }
            let translucent_pass = emitter.style == 3
                || self
                    .actor_states
                    .get(emitter.actor)
                    .is_some_and(|state| state.actor.opacity < 1.0);
            if let Some(system) = self.particles.get_mut(&emitter.actor) {
                let actor = emitter.actor;
                system.config = emitter;
                changed |= upsert_particle_submission(
                    &mut self.render.actor_submissions,
                    actor,
                    system.indices.clone(),
                    translucent_pass,
                );
                continue;
            }
            let capacity = particle_capacity(&emitter);
            ensure!(
                capacity <= MAX_PARTICLE_CAPACITY,
                "particle emitter requests {capacity} particles"
            );
            if capacity == 0 || emitter.actor >= self.actors.len() {
                continue;
            }
            let texture = &emitter.textures[0];
            let package = self
                .actor_render
                .packages
                .load_path(Path::new(&texture.package))?;
            let texture = SceneObject {
                package,
                export_index: texture.export_index,
            };
            let mut state = ActorState::default();
            state.style = emitter.style;
            state.unlit = emitter.unlit;
            let material = actor_surface_material(
                &mut self.actor_render.packages,
                Some(&texture),
                PolyFlags::TWO_SIDED.bits(),
                &state,
                &mut self.render.textures,
                &mut self.actor_render.decoded_textures,
                &mut self.actor_render.images,
                &mut self.water_animations,
            );
            let Some(texture_index) = material.texture else {
                continue;
            };
            let dimensions = Vec2::new(
                self.render.textures[texture_index].width as f32,
                self.render.textures[texture_index].height as f32,
            );
            let surface = self.render.surface_materials.len();
            self.render.surface_materials.push(SurfaceMaterial {
                unlit: emitter.unlit,
                ..material
            });
            let index_start = self.render.mesh.indices.len();
            let vertices = append_particle_slots(
                &mut self.render.mesh,
                capacity,
                surface,
                [
                    Vec2::ZERO,
                    Vec2::new(dimensions.x, 0.0),
                    dimensions,
                    Vec2::new(0.0, dimensions.y),
                ],
            );
            let indices = index_start..self.render.mesh.indices.len();
            let actor = emitter.actor;
            let emitted = emitter.particles_emitted;
            self.particles.insert(
                actor,
                ParticleSystem {
                    config: emitter,
                    particles: Vec::new(),
                    capacity,
                    vertices,
                    indices: indices.clone(),
                    residue: 0.0,
                    last_location: self.actors[actor].location,
                    random: actor as u32 ^ 0xa341_316c,
                    primed: false,
                    emitted,
                },
            );
            upsert_particle_submission(
                &mut self.render.actor_submissions,
                actor,
                indices,
                translucent_pass,
            );
            changed = true;
        }
        Ok(changed)
    }

    pub fn tick_particles(&mut self, delta_time: f32) -> bool {
        if !delta_time.is_finite() || delta_time <= 0.0 {
            return false;
        }
        let collision = Arc::clone(&self.collision);
        let mut changed = false;
        for (&actor, system) in &mut self.particles {
            let Some(owner) = self.actors.get(actor) else {
                continue;
            };
            system.particles.retain_mut(|particle| {
                particle.age += delta_time;
                particle.spin += particle.spin_rate * delta_time;
                let origin = system
                    .config
                    .system_relative
                    .then_some(owner.location)
                    .unwrap_or(Vec3::ZERO);
                let wind = if system.config.damping * system.config.wind_modifier > 0.0 {
                    if system.config.wind_per_particle {
                        ParticleWind::total_at(
                            &system.config.winds,
                            Some(&collision),
                            particle.location + origin,
                        ) * system.config.wind_modifier
                    } else {
                        Vec3::from_array(system.config.wind)
                    }
                } else {
                    Vec3::ZERO
                };
                let previous_location = particle.location;
                particle_drag(
                    &mut particle.location,
                    &mut particle.velocity,
                    Vec3::from_array(system.config.gravity),
                    wind,
                    system.config.damping,
                    delta_time,
                );
                if let Some(location) = particle_collision_response(
                    &collision,
                    previous_location + origin,
                    particle.location + origin,
                    &mut particle.velocity,
                    system.config.elasticity,
                ) {
                    particle.location = location - origin;
                }
                particle.velocity += particle_attraction(
                    particle.location,
                    owner.location,
                    system.config.system_relative,
                    system.config.attraction,
                ) * delta_time;
                apply_particle_chaos(
                    &mut particle.velocity,
                    &mut particle.chaos_timer,
                    system.config.chaos,
                    system.config.chaos_delay,
                    delta_time,
                    &mut system.random,
                );
                particle_is_alive(particle.age, particle.lifetime)
            });
            let rate = sample_particle_emission_rate(&system.config, &mut system.random).max(0.0);
            if system.config.emit && !owner.hidden {
                if system.config.prime && !system.primed {
                    system.residue += rate
                        * sample_particle_float(system.config.lifetime, &mut system.random)
                            .max(0.0);
                    system.primed = true;
                }
                system.residue += if system.config.distribution == 1 && rate > 0.0 {
                    uniform_particle_distance(
                        &system.config.pattern,
                        system.config.period,
                        system.config.draw_scale,
                        system.last_location.distance(owner.location),
                        &mut system.random,
                    ) / rate
                } else {
                    rate * delta_time
                };
                let remaining = if system.config.particles_max == 0 {
                    usize::MAX
                } else {
                    system.config.particles_max.saturating_sub(system.emitted)
                };
                let requested = (system.residue.floor() as usize).min(remaining);
                let needed = system.particles.len().saturating_add(requested);
                if system.config.particles_alive == 0 && needed > system.capacity {
                    let capacity = system
                        .capacity
                        .saturating_mul(2)
                        .max(needed)
                        .min(MAX_PARTICLE_CAPACITY);
                    grow_particle_system(&mut self.render.mesh, system, capacity);
                    let _ = upsert_particle_submission(
                        &mut self.render.actor_submissions,
                        actor,
                        system.indices.clone(),
                        system.config.style == 3 || self.actor_states[actor].actor.opacity < 1.0,
                    );
                }
                let count = if system.config.particles_alive == 0 {
                    requested.min(system.capacity.saturating_sub(system.particles.len()))
                } else {
                    requested.min(system.capacity)
                };
                system.residue -= count as f32;
                if system.config.particles_alive != 0 {
                    let recycle = count
                        .saturating_sub(system.capacity.saturating_sub(system.particles.len()));
                    if recycle != 0 {
                        system.particles.drain(..recycle);
                    }
                }
                for index in 0..count {
                    let fraction = match system.config.distribution {
                        0 => random_unit(&mut system.random),
                        1 => (index + 1) as f32 / count.max(1) as f32,
                        _ => (index as f32 + 0.5) / count.max(1) as f32,
                    };
                    let owner_mesh_position = (system.config.distribution == 2)
                        .then(|| {
                            system.config.owner.and_then(|owner| {
                                let render = self.actors.get(owner)?.render.as_ref()?;
                                let hidden = self
                                    .hidden_actor_positions
                                    .get(&owner)
                                    .map(|positions| (positions.as_slice(), render.vertices.start));
                                random_mesh_position(
                                    &self.render.mesh.positions,
                                    hidden,
                                    &self.render.mesh.indices[render.indices.clone()],
                                    &mut system.random,
                                )
                            })
                        })
                        .flatten();
                    let center = owner_mesh_position
                        .map(|position| {
                            if system.config.system_relative {
                                position - owner.location
                            } else {
                                position
                            }
                        })
                        .unwrap_or_else(|| {
                            if system.config.system_relative {
                                Vec3::ZERO
                            } else {
                                system.last_location.lerp(owner.location, fraction)
                            }
                        });
                    let source = rotate_unreal(
                        owner.rotation,
                        if owner_mesh_position.is_some() {
                            Vec3::ZERO
                        } else {
                            Vec3::new(
                                random_signed(&mut system.random)
                                    * sample_particle_float(
                                        system.config.source_depth,
                                        &mut system.random,
                                    ),
                                random_signed(&mut system.random)
                                    * sample_particle_float(
                                        system.config.source_width,
                                        &mut system.random,
                                    ),
                                random_signed(&mut system.random)
                                    * sample_particle_float(
                                        system.config.source_height,
                                        &mut system.random,
                                    ),
                            ) * 0.5
                        },
                    );
                    let pattern = pattern_position(
                        &system.config.pattern,
                        system.config.period.base
                            + random_unit(&mut system.random) * system.config.period.random,
                    )
                    .map(|point| {
                        rotate_unreal(
                            owner.rotation,
                            Vec3::new(
                                0.0,
                                (point.x - 0.5) * system.config.draw_scale,
                                (0.5 - point.y) * system.config.draw_scale,
                            ),
                        )
                    })
                    .unwrap_or(Vec3::ZERO);
                    let speed =
                        sample_particle_float(system.config.speed, &mut system.random).max(0.0);
                    let direction = particle_direction(
                        owner.rotation,
                        sample_particle_float(
                            system.config.angular_spread_width,
                            &mut system.random,
                        ),
                        sample_particle_float(
                            system.config.angular_spread_height,
                            &mut system.random,
                        ),
                        &mut system.random,
                    ) * speed;
                    system.particles.push(Particle {
                        location: center + source + pattern,
                        velocity: direction
                            + system
                                .config
                                .velocity_relative
                                .then_some(Vec3::from_array(system.config.owner_velocity))
                                .unwrap_or(Vec3::ZERO),
                        age: 0.0,
                        lifetime: sample_particle_float(system.config.lifetime, &mut system.random),
                        half_size: Vec2::new(
                            sample_particle_float(system.config.size_width, &mut system.random),
                            sample_particle_float(system.config.size_length, &mut system.random),
                        ) * 0.5,
                        end_scale: sample_particle_float(
                            system.config.size_end_scale,
                            &mut system.random,
                        )
                        .max(0.0),
                        color_start: sample_particle_color(
                            system.config.color_start,
                            &mut system.random,
                        ),
                        color_end: sample_particle_color(
                            system.config.color_end,
                            &mut system.random,
                        ),
                        alpha_start: sample_particle_float(
                            system.config.alpha_start,
                            &mut system.random,
                        ),
                        alpha_end: sample_particle_float(
                            system.config.alpha_end,
                            &mut system.random,
                        ),
                        spin: 0.0,
                        spin_rate: sample_particle_float(
                            system.config.spin_rate,
                            &mut system.random,
                        ),
                        chaos_timer: 0.0,
                        drip_time: sample_particle_float(
                            system.config.drip_time,
                            &mut system.random,
                        )
                        .max(0.0),
                    });
                    system.emitted += 1;
                }
            }
            system.last_location = owner.location;
            for slot in 0..system.capacity {
                let target = system.vertices.start + slot * 4;
                if let Some(particle) = system.particles.get(slot) {
                    let progress = (particle.age / particle.lifetime).clamp(0.0, 1.0);
                    let grow = if system.config.size_grow_period > 0.0 {
                        (progress / system.config.size_grow_period).min(1.0)
                    } else {
                        1.0
                    };
                    let shrink = if particle.age > system.config.size_delay {
                        let duration =
                            (particle.lifetime - system.config.size_delay).max(f32::EPSILON);
                        1.0 + (particle.end_scale - 1.0)
                            * ((particle.age - system.config.size_delay) / duration).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let drip = if particle.drip_time > 0.0 {
                        (particle.age / particle.drip_time).min(1.0)
                    } else {
                        1.0
                    };
                    let location = if system.config.system_relative {
                        owner.location + particle.location
                    } else {
                        particle.location
                    };
                    let positions = particle_render_primitive_positions(
                        location,
                        particle.half_size * grow * shrink * drip,
                        self.particle_view_rotation,
                        particle.spin,
                        system.config.render_primitive,
                    );
                    self.render.mesh.positions[target..target + 4].copy_from_slice(&positions);
                    let color_progress = if particle.age > system.config.color_delay {
                        ((particle.age - system.config.color_delay)
                            / (particle.lifetime - system.config.color_delay).max(f32::EPSILON))
                        .clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let color = particle
                        .color_start
                        .lerp(particle.color_end, color_progress);
                    let alpha = particle_alpha(
                        particle.age,
                        particle.lifetime,
                        particle.alpha_start,
                        particle.alpha_end,
                        system.config.alpha_delay,
                        system.config.alpha_grow_period,
                    );
                    self.render.mesh.vertex_colors[target..target + 4].fill(particle_vertex_color(
                        system.config.style,
                        color,
                        alpha,
                    ));
                } else {
                    self.render.mesh.positions[target..target + 4].fill(Vec3::ZERO);
                }
            }
            changed = true;
        }
        changed
    }

    pub fn particle_counts(&self) -> Vec<(usize, usize)> {
        self.particles
            .iter()
            .map(|(&actor, system)| (actor, system.emitted))
            .collect()
    }

    pub fn set_actor_rotation(&mut self, actor_index: usize, rotation: Rotator) -> Result<bool> {
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.rotation == rotation {
            return Ok(false);
        }
        let collapsed = self.hidden_actor_positions.contains_key(&actor_index);
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        if let Some(vertices) = &vertices {
            ensure!(
                vertices.start <= vertices.end && vertices.end <= self.render.mesh.positions.len(),
                "actor render range is outside the scene mesh"
            );
        }

        let origin = if actor.draw_type == 3 {
            actor.location
        } else {
            actor.location + actor.pre_pivot
        };
        let transform = rotation_delta(origin, actor.rotation, rotation);
        self.actors[actor_index].rotation = rotation;
        self.actor_states[actor_index].actor.rotation = rotation;
        if self.actor_states[actor_index].is_light {
            self.render.set_light_rotation(actor_index, rotation);
        }
        if let Some(mesh_transform) = &mut self.actors[actor_index].mesh_transform {
            *mesh_transform = transform * *mesh_transform;
        }
        if let Some(vertices) = vertices {
            if collapsed {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&actor_index)
                    .context("collapsed actor has no saved render positions")?;
                transform_positions(positions, transform);
            } else {
                transform_positions(&mut self.render.mesh.positions[vertices.clone()], transform);
            }
            transform_normals(&mut self.render.mesh.normals[vertices], transform);
        }
        if let Some(animation) = self
            .animations
            .iter_mut()
            .find(|animation| animation.actor_index == actor_index)
        {
            animation.transform = transform * animation.transform;
            animation.normal_transform = Mat3::from_mat4(animation.transform).inverse().transpose();
            transform_animation_pose_positions(
                &mut animation.bone_positions,
                animation.tween_from.as_deref_mut(),
                animation.tween_bone_positions_from.as_deref_mut(),
                transform,
            );
        }
        // ponytail: retain baked actor lighting until moving lights/zones are observable.
        Ok(true)
    }

    pub fn set_actor_pre_pivot(&mut self, actor_index: usize, pre_pivot: Vec3) -> Result<bool> {
        ensure!(pre_pivot.is_finite(), "actor pre-pivot is not finite");
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        if actor.pre_pivot == pre_pivot {
            return Ok(false);
        }
        let delta = pre_pivot_translation(actor, pre_pivot);
        let collapsed = self.hidden_actor_positions.contains_key(&actor_index);
        let vertices = actor.render.as_ref().map(|render| render.vertices.clone());
        self.actors[actor_index].pre_pivot = pre_pivot;
        self.actor_states[actor_index].actor.pre_pivot = pre_pivot;
        if let Some(transform) = &mut self.actors[actor_index].mesh_transform {
            *transform = Mat4::from_translation(delta) * *transform;
        }
        if let Some(vertices) = vertices {
            if collapsed {
                let positions = self
                    .hidden_actor_positions
                    .get_mut(&actor_index)
                    .context("collapsed actor has no saved render positions")?;
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
        self.actor_states[actor_index].actor.hidden = hidden;
        self.sync_actor_render_visibility(actor_index)
    }

    fn sync_actor_render_visibility(&mut self, actor_index: usize) -> Result<bool> {
        let actor = self
            .actors
            .get(actor_index)
            .context("runtime refers to a missing scene actor")?;
        let Some(render) = &actor.render else {
            return Ok(false);
        };
        ensure!(
            render.vertices.start <= render.vertices.end
                && render.vertices.end <= self.render.mesh.positions.len(),
            "actor render range is outside the scene mesh"
        );
        let visible = !actor.hidden && actor.draw_type != 0;
        let collapsed = self.hidden_actor_positions.contains_key(&actor_index);
        if visible == !collapsed {
            return Ok(false);
        }
        let positions = &mut self.render.mesh.positions[render.vertices.clone()];
        if !visible {
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
        let mut changed = if let Some(particles) = self.particles.remove(&actor_index) {
            ensure!(
                particles.vertices.start <= particles.vertices.end
                    && particles.vertices.end <= self.render.mesh.positions.len(),
                "particle render range is outside the scene mesh"
            );
            collapse_positions(&mut self.render.mesh.positions[particles.vertices])
        } else {
            false
        };
        changed |= remove_particle_submission(&mut self.render.actor_submissions, actor_index);
        let corona_count = self.render.coronas.len();
        self.render
            .coronas
            .retain(|corona| corona.actor_index != actor_index);
        changed |= self.render.coronas.len() != corona_count;
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
            return Ok(changed);
        };
        ensure!(
            render.vertices.start <= render.vertices.end
                && render.vertices.end <= self.render.mesh.positions.len(),
            "actor render range is outside the scene mesh"
        );
        self.actor_meshes = self.actor_meshes.saturating_sub(1);
        changed |= collapse_positions(&mut self.render.mesh.positions[render.vertices]);
        Ok(changed)
    }

    pub fn tick_animations(&mut self, delta_time: f32) -> Result<bool> {
        Ok(self.tick_animations_with_completions(delta_time)?.0)
    }

    pub fn actor_animation_sequences(
        &mut self,
        actor: usize,
    ) -> Vec<(String, String, f32, usize, Vec<(f32, String)>)> {
        let sequences = match self.resolve_actor_animation_sequences(actor) {
            Ok(sequences) => sequences,
            Err(error) => {
                if let Some(target) = self.actors.get_mut(actor) {
                    let diagnostic = format!("animation metadata could not be decoded: {error:#}");
                    if !target.diagnostics.contains(&diagnostic) {
                        target.diagnostics.push(diagnostic);
                    }
                }
                Vec::new()
            }
        };
        sequences
            .into_iter()
            .map(|sequence| {
                (
                    sequence.name,
                    sequence.group,
                    sequence.rate,
                    sequence.frame_count,
                    sequence
                        .notifications
                        .into_iter()
                        .map(|notification| (notification.time, notification.function))
                        .collect(),
                )
            })
            .collect()
    }

    fn resolve_actor_animation_sequences(
        &mut self,
        actor: usize,
    ) -> Result<Vec<MeshAnimationSequence>> {
        if let Some(animation) = self
            .animations
            .iter()
            .find(|animation| animation.actor_index == actor)
        {
            return Ok(animation.sequences().to_vec());
        }
        let state = self
            .actor_states
            .get(actor)
            .context("runtime refers to a missing scene actor")?
            .actor
            .clone();
        let Some(mesh_object) = state.mesh.as_ref() else {
            return Ok(Vec::new());
        };
        let mesh = Mesh::decode(&mesh_object.package, mesh_object.export_index)
            .context("could not decode actor mesh animation metadata")?;
        let animation_source = actor_animation_source(
            &mut self.actor_render.packages,
            &mut self.actor_render.animation_cache,
            &state,
            mesh_object,
            &mesh,
        );
        animation_source
            .sequences(&mesh)
            .map(<[MeshAnimationSequence]>::to_vec)
            .map_err(anyhow::Error::msg)
    }

    pub fn animation_request_exposes_capability_gap(
        &mut self,
        actor: usize,
        sequence: &str,
    ) -> bool {
        let rendered_mesh = self
            .actors
            .get(actor)
            .is_some_and(|target| target.draw_type == 2 && target.mesh.is_some());
        rendered_mesh
            && self
                .actor_animation_sequences(actor)
                .iter()
                .any(|(name, ..)| name.eq_ignore_ascii_case(sequence))
    }

    pub fn actor_bone_names(&self, actor: usize) -> Vec<String> {
        self.animations
            .iter()
            .find(|animation| animation.actor_index == actor)
            .map(|animation| animation.mesh.bone_names().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    pub(crate) fn runtime_bone_positions(&self) -> Result<Vec<(usize, Vec<[f32; 3]>)>> {
        self.animations
            .iter()
            .map(|animation| {
                Ok((
                    animation.actor_index,
                    animation
                        .bone_positions
                        .iter()
                        .copied()
                        .map(|position| position.to_array())
                        .collect(),
                ))
            })
            .collect()
    }

    pub(crate) fn runtime_weapon_poses(&self) -> Result<Vec<(usize, [f32; 3], [i32; 3])>> {
        let mut poses = Vec::new();
        for animation in &self.animations {
            let Some(transform) = animation.attachment()? else {
                continue;
            };
            let Some(rotation) = ortho_rotation(transform) else {
                continue;
            };
            poses.push((
                animation.actor_index,
                transform.w_axis.truncate().to_array(),
                rotation,
            ));
        }
        Ok(poses)
    }

    pub fn actor_visual_bounds(&self, actor: usize) -> Option<([f32; 3], [f32; 3])> {
        self.actors
            .get(actor)
            .and_then(|actor| actor.visual_bounds)
            .map(|(minimum, maximum)| (minimum.to_array(), maximum.to_array()))
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
                tween == Some(1.0) && animation.rate == 0.0
            };
            if finished {
                completed.push(animation.actor_index);
                if tween.is_some() {
                    animation.playing = false;
                }
            }
            let collapsed = {
                let actor = self
                    .actors
                    .get_mut(animation.actor_index)
                    .context("animation refers to a missing scene actor")?;
                let actor_animation = actor
                    .animation
                    .as_mut()
                    .context("animated scene actor has no animation state")?;
                actor_animation.phase = animation.phase;
                self.hidden_actor_positions
                    .contains_key(&animation.actor_index)
            };
            let sample = animation.sample()?;
            ensure!(
                sample.positions.len() == sample.normals.len(),
                "animation position and normal counts differ"
            );
            let positions = sample
                .positions
                .iter()
                .map(|&position| animation.transform.transform_point3(position))
                .collect::<Vec<_>>();
            let normals = sample
                .normals
                .iter()
                .map(|&normal| (animation.normal_transform * normal).normalize_or_zero())
                .collect::<Vec<_>>();
            let mut lit_colors = tween.is_none().then(|| vec![None; positions.len()]);
            let bone_positions = animation.bone_positions_from(&sample);
            animation.bone_positions = bone_positions;
            if animation.root_motion {
                let root_motion = animation.transform.transform_vector3(sample.root_motion);
                let delta = root_motion - animation.root_motion_position;
                animation.root_motion_position = root_motion;
                if delta != Vec3::ZERO {
                    self.root_motions.push((animation.actor_index, delta));
                }
            }
            let faces = animation.mesh.animation_faces();
            ensure!(
                faces.len().checked_mul(3) == Some(animation.vertices.len()),
                "animation changed actor vertex count"
            );
            for (index, (destination, &source)) in animation
                .vertices
                .clone()
                .zip(faces.iter().flatten())
                .enumerate()
            {
                let target = positions
                    .get(source)
                    .context("animation refers to a missing mesh vertex")?;
                let normal = normals
                    .get(source)
                    .context("animation refers to a missing mesh normal")?;
                let position = animation
                    .tween_from
                    .as_ref()
                    .zip(tween)
                    .map_or(*target, |(from, tween)| from[index].lerp(*target, tween));
                if collapsed {
                    self.hidden_actor_positions
                        .get_mut(&animation.actor_index)
                        .context("collapsed animated actor has no saved render positions")?
                        [index] = position;
                } else {
                    self.render.mesh.positions[destination] = position;
                }
                self.render.mesh.normals[destination] = *normal;
                let surface = self.render.mesh.vertex_surfaces[destination];
                let unlit = animation.unlit
                    || self
                        .render
                        .surface_materials
                        .get(surface)
                        .context("animated actor vertex refers to a missing material")?
                        .unlit;
                let color = if unlit {
                    animation.lighting.color(position, *normal, true)
                } else if let Some(colors) = lit_colors.as_mut() {
                    if let Some(color) = colors[source] {
                        color
                    } else {
                        let color = animation.lighting.color(position, *normal, false);
                        colors[source] = Some(color);
                        color
                    }
                } else {
                    animation.lighting.color(position, *normal, false)
                };
                self.render.mesh.vertex_colors[destination] = color;
            }
            if tween == Some(1.0) {
                animation.tween_from = None;
                animation.tween_attachment_from = None;
                animation.tween_bone_positions_from = None;
            }
        }
        Ok((changed, completed))
    }

    pub fn tick_textures(&mut self, delta_time: f32) -> Result<(Vec<usize>, bool)> {
        let mut changed = Vec::new();
        let animations = &mut self.water_animations;
        for water in &mut animations.water {
            if water.animation.tick(delta_time) {
                animations
                    .pixels
                    .insert(water.id.clone(), water.animation.indices().to_vec());
                if let Some(texture) = water.texture {
                    self.render.textures[texture].rgba =
                        water.animation.rgba(&water.palette, water.masked)?;
                    changed.push(texture);
                }
            }
        }
        for animation in &mut animations.generic {
            if animation.tick(delta_time) {
                animations.pixels.insert(
                    animation.id.clone(),
                    animation.index_frames[animation.current].clone(),
                );
                if let Some(texture) = animation.texture {
                    self.render.textures[texture].clone_from(&animation.frames[animation.current]);
                    changed.push(texture);
                }
            }
        }
        for ice in &mut animations.ice {
            let source = animations
                .pixels
                .get(&ice.source)
                .context("ice source runtime pixels are missing")?;
            let glass = animations
                .pixels
                .get(&ice.glass)
                .context("ice glass runtime pixels are missing")?;
            ice.animation.update_dependencies(
                ice.source_dimensions[0],
                ice.source_dimensions[1],
                source,
                ice.glass_dimensions[0],
                ice.glass_dimensions[1],
                glass,
            )?;
            if ice.animation.tick(delta_time) {
                animations
                    .pixels
                    .insert(ice.id.clone(), ice.animation.indices().to_vec());
                self.render.textures[ice.texture].rgba =
                    ice.animation.rgba(&ice.palette, ice.masked)?;
                changed.push(ice.texture);
            }
        }
        let mut materials_changed = false;
        for attachments in &animations.attachments {
            let Some(frame) = animations
                .generic
                .get(attachments.animation)
                .and_then(|animation| attachments.frames.get(animation.current))
                .copied()
            else {
                continue;
            };
            let Some(material) = self.render.surface_materials.get_mut(attachments.surface) else {
                continue;
            };
            let before = (
                material.macro_texture,
                material.detail_texture,
                material.macro_draw_scale,
                material.detail_draw_scale,
            );
            material.macro_texture = frame.macro_texture;
            material.detail_texture = frame.detail_texture;
            material.macro_draw_scale = frame.macro_draw_scale;
            material.detail_draw_scale = frame.detail_draw_scale;
            materials_changed |= before
                != (
                    material.macro_texture,
                    material.detail_texture,
                    material.macro_draw_scale,
                    material.detail_draw_scale,
                );
        }
        Ok((changed, materials_changed))
    }

    pub fn tick_water(&mut self, delta_time: f32) -> Result<Vec<usize>> {
        self.tick_textures(delta_time).map(|changes| changes.0)
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
        self.set_actor_animation(
            actor_index,
            sequence_name,
            relative_rate,
            tween_time,
            true,
            false,
        )
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
        self.set_actor_animation(
            actor_index,
            sequence_name,
            relative_rate,
            tween_time,
            false,
            false,
        )
    }

    pub fn play_actor_animation_with_root_motion(
        &mut self,
        actor_index: usize,
        sequence_name: &str,
        relative_rate: f32,
        tween_time: f32,
        looping: bool,
    ) -> Result<bool> {
        self.set_actor_animation(
            actor_index,
            sequence_name,
            relative_rate,
            tween_time,
            looping,
            true,
        )
    }

    pub fn take_root_motions(&mut self) -> Vec<(usize, Vec3)> {
        std::mem::take(&mut self.root_motions)
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
        root_motion: bool,
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
        let tween_attachment_from = (tween_time > 0.0)
            .then(|| animation.local_attachment())
            .transpose()?
            .flatten();
        let tween_bone_positions_from =
            (tween_time > 0.0).then(|| animation.bone_positions.clone());
        animation.tween_attachment_from = tween_attachment_from;
        animation.tween_bone_positions_from = tween_bone_positions_from;
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
        animation.root_motion = root_motion;
        animation.root_motion_position = if root_motion {
            animation
                .transform
                .transform_vector3(animation.sample()?.root_motion)
        } else {
            Vec3::ZERO
        };
        if animation.rate == 0.0 && animation.tween_from.is_none() {
            let sample = animation.sample()?;
            let bone_positions = animation.bone_positions_from(&sample);
            animation.bone_positions = bone_positions;
        }
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

fn pre_pivot_translation(actor: &SceneActor, pre_pivot: Vec3) -> Vec3 {
    if actor.draw_type == 3 {
        rotation_matrix(actor.rotation)
            .transform_vector3(actor.main_scale * (actor.pre_pivot - pre_pivot))
    } else {
        pre_pivot - actor.pre_pivot
    }
}

fn transform_positions(positions: &mut [Vec3], transform: Mat4) {
    for position in positions {
        *position = transform.transform_point3(*position);
    }
}

fn transform_animation_pose_positions(
    bone_positions: &mut [Vec3],
    tween_positions: Option<&mut [Vec3]>,
    tween_bone_positions: Option<&mut [Vec3]>,
    transform: Mat4,
) {
    transform_positions(bone_positions, transform);
    if let Some(positions) = tween_positions {
        transform_positions(positions, transform);
    }
    if let Some(positions) = tween_bone_positions {
        transform_positions(positions, transform);
    }
}

fn transform_normals(normals: &mut [Vec3], transform: Mat4) {
    let transform = Mat3::from_mat4(transform).inverse().transpose();
    for normal in normals {
        *normal = (transform * *normal).normalize_or_zero();
    }
}

fn scale_bounds_about(bounds: (Vec3, Vec3), pivot: Vec3, scale: f32) -> (Vec3, Vec3) {
    let (minimum, maximum) = bounds;
    let center = pivot + ((minimum + maximum) * 0.5 - pivot) * scale;
    let extents = (maximum - minimum) * 0.5 * scale.abs();
    (center - extents, center + extents)
}

fn triangle_attachment_transform(points: [Vec3; 3]) -> Option<Mat4> {
    let x = (points[1] - points[0]).normalize_or_zero();
    let y = x.cross(points[2] - points[0]).normalize_or_zero();
    let z = x.cross(y).normalize_or_zero();
    (x != Vec3::ZERO && y != Vec3::ZERO && z != Vec3::ZERO).then(|| {
        Mat4::from_cols(
            x.extend(0.0),
            y.extend(0.0),
            z.extend(0.0),
            ((points[0] + points[2]) * 0.5).extend(1.0),
        )
    })
}

fn ortho_rotation(transform: Mat4) -> Option<[i32; 3]> {
    let x = transform.x_axis.truncate().normalize_or_zero();
    let y = transform.y_axis.truncate().normalize_or_zero();
    let z = transform.z_axis.truncate().normalize_or_zero();
    if !x.is_finite()
        || !y.is_finite()
        || !z.is_finite()
        || x == Vec3::ZERO
        || y == Vec3::ZERO
        || z == Vec3::ZERO
    {
        return None;
    }
    let units = 65_536.0 / std::f32::consts::TAU;
    Some([
        (x.z.atan2(x.x.hypot(x.y)) * units) as i32,
        (x.y.atan2(x.x) * units) as i32,
        ((-y.z).atan2(z.z) * units) as i32,
    ])
}

fn interpolate_transform(from: Mat4, to: Mat4, amount: f32) -> Mat4 {
    let (from_scale, from_rotation, from_translation) = from.to_scale_rotation_translation();
    let (to_scale, to_rotation, to_translation) = to.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(
        from_scale.lerp(to_scale, amount),
        from_rotation.slerp(to_rotation, amount),
        from_translation.lerp(to_translation, amount),
    )
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
    root_motion: bool,
    root_motion_position: Vec3,
    tween_from: Option<Vec<Vec3>>,
    tween_attachment_from: Option<Mat4>,
    tween_bone_positions_from: Option<Vec<Vec3>>,
    bone_positions: Vec<Vec3>,
    tween_elapsed: f32,
    tween_duration: f32,
    vertices: Range<usize>,
    transform: Mat4,
    normal_transform: Mat3,
    lighting: ActorVertexLighting,
    unlit: bool,
}

struct AnimatedWaterTexture {
    id: SceneObjectId,
    texture: Option<usize>,
    masked: bool,
    palette: Palette,
    animation: WaterAnimation,
}

#[derive(Default)]
struct TextureAnimations {
    water: Vec<AnimatedWaterTexture>,
    ice: Vec<AnimatedIceTexture>,
    generic: Vec<AnimatedGenericTexture>,
    attachments: Vec<AnimatedSurfaceAttachments>,
    pixels: HashMap<SceneObjectId, Vec<u8>>,
}

struct AnimatedSurfaceAttachments {
    surface: usize,
    animation: usize,
    frames: Vec<SurfaceAttachmentFrame>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SurfaceAttachmentFrame {
    macro_texture: Option<usize>,
    detail_texture: Option<usize>,
    macro_draw_scale: f32,
    detail_draw_scale: f32,
}

struct AnimatedIceTexture {
    id: SceneObjectId,
    texture: usize,
    masked: bool,
    palette: Palette,
    animation: IceAnimation,
    source: SceneObjectId,
    glass: SceneObjectId,
    source_dimensions: [u32; 2],
    glass_dimensions: [u32; 2],
}

struct AnimatedGenericTexture {
    id: SceneObjectId,
    texture: Option<usize>,
    frames: Vec<TextureImage>,
    index_frames: Vec<Vec<u8>>,
    next: Vec<Option<usize>>,
    current: usize,
    accumulator: f32,
    prime_count: u8,
    prime_current: u8,
    min_frame_rate: f32,
    max_frame_rate: f32,
}

struct GenericTextureAnimation {
    frames: Vec<(Texture, Palette, Arc<Package>)>,
    next: Vec<Option<usize>>,
    prime_count: u8,
    min_frame_rate: f32,
    max_frame_rate: f32,
}

impl AnimatedGenericTexture {
    fn tick(&mut self, delta_time: f32) -> bool {
        if !delta_time.is_finite() || delta_time == 0.0 {
            return false;
        }
        let before = self.current;
        while self.prime_current < self.prime_count {
            self.prime_current += 1;
            self.advance();
        }
        if self.max_frame_rate == 0.0 {
            self.advance();
            return self.current != before;
        }

        let maximum = texture_frame_rate(self.max_frame_rate);
        let minimum_period = 1.0 / texture_frame_rate(self.min_frame_rate);
        self.accumulator += delta_time;
        if self.accumulator >= 1.0 / maximum {
            self.advance();
            if self.accumulator < minimum_period {
                self.accumulator = 0.0;
            } else {
                self.accumulator -= minimum_period;
                if self.accumulator > minimum_period {
                    self.accumulator = minimum_period;
                }
            }
        }
        self.current != before
    }

    fn advance(&mut self) {
        self.current = self.next.get(self.current).copied().flatten().unwrap_or(0);
    }
}

fn texture_frame_rate(rate: f32) -> f32 {
    if 0.1 <= rate {
        if 100.0 <= rate { 100.0 } else { rate }
    } else {
        0.1
    }
}

struct SpriteActor {
    actor_index: usize,
    half_size: Vec2,
    texture: usize,
}

struct ParticleSystem {
    config: ParticleEmitter,
    particles: Vec<Particle>,
    capacity: usize,
    vertices: Range<usize>,
    indices: Range<usize>,
    residue: f32,
    last_location: Vec3,
    random: u32,
    primed: bool,
    emitted: usize,
}

struct Particle {
    location: Vec3,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    half_size: Vec2,
    end_scale: f32,
    color_start: Vec3,
    color_end: Vec3,
    alpha_start: f32,
    alpha_end: f32,
    spin: f32,
    spin_rate: f32,
    chaos_timer: f32,
    drip_time: f32,
}

const MAX_PARTICLE_CAPACITY: usize = 100_000;

fn grow_particle_system(mesh: &mut TriangleMesh, system: &mut ParticleSystem, capacity: usize) {
    let old_vertices = system.vertices.clone();
    let texture_coordinates = mesh.texture_coordinates[old_vertices.start..old_vertices.start + 4]
        .try_into()
        .expect("particle quad has four texture coordinates");
    let surface = mesh.vertex_surfaces[old_vertices.start];
    mesh.positions[old_vertices].fill(Vec3::ZERO);
    let index_start = mesh.indices.len();
    system.vertices = append_particle_slots(mesh, capacity, surface, texture_coordinates);
    system.indices = index_start..mesh.indices.len();
    system.capacity = capacity;
}

fn upsert_particle_submission(
    submissions: &mut Vec<crate::ActorSubmission>,
    actor_index: usize,
    indices: Range<usize>,
    translucent_pass: bool,
) -> bool {
    if let Some(submission) = submissions
        .iter_mut()
        .find(|submission| submission.actor_index == actor_index)
    {
        let changed =
            submission.indices != indices || submission.translucent_pass != translucent_pass;
        submission.indices = indices;
        submission.translucent_pass = translucent_pass;
        changed
    } else {
        submissions.push(crate::ActorSubmission {
            actor_index,
            indices,
            translucent_pass,
        });
        submissions.sort_by_key(|submission| submission.actor_index);
        true
    }
}

fn remove_particle_submission(
    submissions: &mut Vec<crate::ActorSubmission>,
    actor_index: usize,
) -> bool {
    let previous = submissions.len();
    submissions.retain(|submission| submission.actor_index != actor_index);
    submissions.len() != previous
}

fn remove_particle_submission_range(
    submissions: &mut Vec<crate::ActorSubmission>,
    actor_index: usize,
    indices: &Range<usize>,
) -> bool {
    let previous = submissions.len();
    submissions.retain(|submission| {
        submission.actor_index != actor_index || submission.indices != *indices
    });
    submissions.len() != previous
}

fn append_particle_slots(
    mesh: &mut TriangleMesh,
    capacity: usize,
    surface: usize,
    texture_coordinates: [Vec2; 4],
) -> Range<usize> {
    let vertices = mesh.positions.len()..mesh.positions.len() + capacity * 4;
    for slot in 0..capacity {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend([Vec3::ZERO; 4]);
        mesh.normals.extend([Vec3::ZERO; 4]);
        mesh.texture_coordinates.extend(texture_coordinates);
        mesh.lightmap_coordinates.extend([Vec2::ZERO; 4]);
        mesh.vertex_lightmaps.extend([None; 4]);
        mesh.vertex_colors.extend([Vec3::ONE; 4]);
        mesh.vertex_surfaces.extend([surface; 4]);
        mesh.indices
            .extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        mesh.triangle_surfaces.extend([surface; 2]);
        debug_assert_eq!(vertices.start + slot * 4, base as usize);
    }
    vertices
}

fn particle_capacity(emitter: &ParticleEmitter) -> usize {
    if emitter.particles_alive != 0 {
        return emitter.particles_alive;
    }
    let child = emitter.particles_per_second;
    let rate = emitter.parent_particles_per_second.map_or_else(
        || child.base.abs() + child.random.abs(),
        |parent| {
            let blend = emitter.parent_blend;
            let base = child.base + (parent.base - child.base) * blend;
            base.abs()
                + child.random.abs() * (1.0 - blend).abs()
                + parent.random.abs() * blend.abs()
        },
    );
    let capacity = if emitter.distribution == 1 && emitter.pattern.len() > 1 {
        let spacing = (emitter.particles_per_second.base.abs()
            - emitter.particles_per_second.random.abs())
        .max(f32::EPSILON);
        (pattern_length(&emitter.pattern) * emitter.draw_scale.abs() / spacing)
            .ceil()
            .max(1.0) as usize
    } else {
        let lifetime = emitter.lifetime.base.abs() + emitter.lifetime.random.abs();
        (rate * lifetime).ceil().max(1.0) as usize
    };
    if emitter.particles_max == 0 {
        capacity
    } else {
        capacity.min(emitter.particles_max)
    }
}

fn pattern_length(points: &[[f32; 3]]) -> f32 {
    points
        .windows(2)
        .map(|points| Vec3::from_array(points[0]).distance(Vec3::from_array(points[1])))
        .sum()
}

fn sample_particle_float(value: ParticleFloat, random: &mut u32) -> f32 {
    value.base + value.random * random_unit(random)
}

fn sample_particle_emission_rate(emitter: &ParticleEmitter, random: &mut u32) -> f32 {
    let child = sample_particle_float(emitter.particles_per_second, random);
    emitter.parent_particles_per_second.map_or(child, |parent| {
        let parent = sample_particle_float(parent, random);
        child + (parent - child) * emitter.parent_blend
    })
}

fn particle_is_alive(age: f32, lifetime: f32) -> bool {
    lifetime <= 0.0 || age < lifetime
}

fn particle_alpha(
    age: f32,
    lifetime: f32,
    start: f32,
    end: f32,
    delay: f32,
    grow_period: f32,
) -> f32 {
    let grow_duration = grow_period * lifetime;
    let alpha = if grow_duration > age {
        (start * age / grow_duration).min(start).max(0.0)
    } else if lifetime > delay && age > delay {
        (start + (end - start) * (age - delay) / (lifetime - delay)).max(0.0)
    } else {
        start.max(0.0)
    };
    if alpha < 0.001 { 0.0 } else { alpha }
}

fn particle_vertex_color(style: u8, color: Vec3, alpha: f32) -> Vec3 {
    if style == 4 {
        Vec3::ONE
    } else {
        color * alpha.min(1.0)
    }
}

fn sample_particle_color(value: ParticleColor, random: &mut u32) -> Vec3 {
    let base = Vec3::new(
        value.base[0] as f32,
        value.base[1] as f32,
        value.base[2] as f32,
    );
    let range = Vec3::new(
        value.random[0] as f32,
        value.random[1] as f32,
        value.random[2] as f32,
    );
    (base + range * random_unit(random)) / 255.0
}

fn particle_damping(damping: f32, delta_time: f32) -> f32 {
    (-damping * delta_time).exp()
}

fn particle_drag(
    location: &mut Vec3,
    velocity: &mut Vec3,
    gravity: Vec3,
    wind: Vec3,
    damping: f32,
    delta_time: f32,
) {
    if damping <= 0.0 {
        *location += *velocity * delta_time + gravity * (0.5 * delta_time * delta_time);
        *velocity += gravity * delta_time;
        return;
    }
    let terminal = gravity / damping + wind;
    let difference = *velocity - terminal;
    let decay = particle_damping(damping, delta_time);
    *location += terminal * delta_time + difference * ((1.0 - decay) / damping);
    *velocity = terminal + difference * decay;
}

fn particle_attraction(
    location: Vec3,
    owner_location: Vec3,
    system_relative: bool,
    attraction: [f32; 3],
) -> Vec3 {
    let target = if system_relative {
        Vec3::ZERO
    } else {
        owner_location
    };
    (target - location) * Vec3::from_array(attraction)
}

fn particle_collision_response(
    collision: &BspCollision,
    start: Vec3,
    end: Vec3,
    velocity: &mut Vec3,
    elasticity: f32,
) -> Option<Vec3> {
    if elasticity == 0.0 {
        return None;
    }
    let hit = collision.line_trace(start, end)?;
    *velocity -= hit.normal * ((1.0 + elasticity) * velocity.dot(hit.normal));
    Some(start.lerp(end, hit.fraction))
}

fn sync_hidden_attachment(rendered: &mut [Vec3], saved: &[Vec3], attached: bool) -> bool {
    if attached {
        rendered.copy_from_slice(saved);
        true
    } else {
        collapse_positions(rendered)
    }
}

fn pattern_position(points: &[[f32; 3]], progress: f32) -> Option<Vec3> {
    let last = points.len().checked_sub(1)?;
    if last == 0 {
        return Some(Vec3::from_array(points[0]));
    }
    let position = progress.clamp(0.0, 1.0) * last as f32;
    let index = (position.floor() as usize).min(last - 1);
    Some(
        Vec3::from_array(points[index])
            .lerp(Vec3::from_array(points[index + 1]), position - index as f32),
    )
}

fn uniform_particle_distance(
    pattern: &[[f32; 3]],
    period: ParticleFloat,
    draw_scale: f32,
    moved: f32,
    random: &mut u32,
) -> f32 {
    let last = match pattern.len().checked_sub(1) {
        Some(0) | None => return moved,
        Some(last) => last,
    };
    let position = sample_particle_float(period, random).clamp(0.0, 1.0) * last as f32;
    let index = (position.floor() as usize).min(last - 1);
    Vec3::from_array(pattern[index]).distance(Vec3::from_array(pattern[index + 1]))
        * last as f32
        * draw_scale
        * period.random
}

fn random_mesh_position(
    positions: &[Vec3],
    hidden: Option<(&[Vec3], usize)>,
    indices: &[u32],
    random: &mut u32,
) -> Option<Vec3> {
    let triangles = indices.len() / 3;
    let triangle = (random_unit(random) * triangles as f32) as usize;
    let indices = indices.get(triangle * 3..triangle * 3 + 3)?;
    let position = |index: u32| {
        let index = index as usize;
        hidden
            .and_then(|(positions, first)| {
                index
                    .checked_sub(first)
                    .and_then(|index| positions.get(index))
            })
            .or_else(|| positions.get(index))
            .copied()
    };
    let a = position(indices[0])?;
    let b = position(indices[1])?;
    let c = position(indices[2])?;
    let first = random_unit(random);
    let second = random_unit(random);
    Some(a * (first * second) + b * (first * (1.0 - second)) + c * (1.0 - first))
}

fn random_signed(random: &mut u32) -> f32 {
    random_unit(random) * 2.0 - 1.0
}

fn apply_particle_chaos(
    velocity: &mut Vec3,
    timer: &mut f32,
    chaos: f32,
    delay: f32,
    delta_time: f32,
    random: &mut u32,
) {
    *timer = (*timer - delta_time).max(0.0);
    if chaos == 0.0 || *timer > 0.0 {
        return;
    }

    let mut direction = Vec3::new(
        random_signed(random),
        random_signed(random),
        random_signed(random),
    );
    if direction.length_squared() >= 1.0e-8 {
        direction = direction.normalize();
    }
    *velocity += direction * chaos;
    *timer = delay;
}

fn random_unit(random: &mut u32) -> f32 {
    *random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*random >> 8) as f32 / 16_777_216.0
}

fn particle_direction(
    rotation: Rotator,
    width_degrees: f32,
    height_degrees: f32,
    random: &mut u32,
) -> Vec3 {
    let units_per_degree = 65_536.0 / 360.0;
    let rotation = Rotator {
        pitch: rotation.pitch
            + (random_signed(random) * height_degrees * units_per_degree).round() as i32,
        yaw: rotation.yaw
            + (random_signed(random) * width_degrees * units_per_degree).round() as i32,
        roll: rotation.roll,
    };
    rotate_unreal(rotation, Vec3::X)
}

impl AnimatedActorMesh {
    fn sequences(&self) -> &[MeshAnimationSequence] {
        self.skeletal_animation
            .as_ref()
            .map_or(self.mesh.animation_sequences.as_slice(), |animation| {
                animation.sequences.as_slice()
            })
    }

    fn sample(&self) -> openhp1_mesh::Result<openhp1_mesh::MeshSample> {
        if let Some(animation) = &self.skeletal_animation {
            self.mesh.sample_skeletal_vertices(
                animation,
                self.sequence,
                self.phase,
                self.root_motion,
            )
        } else {
            self.mesh
                .sample_sequence_vertices(&self.mesh.animation_sequences[self.sequence], self.phase)
        }
    }

    fn attachment(&self) -> openhp1_mesh::Result<Option<Mat4>> {
        self.local_attachment().map(|attachment| {
            attachment.map(|target| {
                let local = self.tween_attachment_from.map_or(target, |from| {
                    interpolate_transform(from, target, self.tween_elapsed / self.tween_duration)
                });
                self.transform * local
            })
        })
    }

    fn bone_positions_from(&self, sample: &openhp1_mesh::MeshSample) -> Vec<Vec3> {
        let positions = sample
            .bone_positions()
            .map(|position| self.transform.transform_point3(position))
            .collect::<Vec<_>>();
        if let Some(from) = &self.tween_bone_positions_from {
            from.iter()
                .zip(&positions)
                .map(|(from, to)| from.lerp(*to, self.tween_elapsed / self.tween_duration))
                .collect()
        } else {
            positions
        }
    }

    fn local_attachment(&self) -> openhp1_mesh::Result<Option<Mat4>> {
        let Some(animation) = &self.skeletal_animation else {
            return Ok(None);
        };
        if !self.mesh.has_attachment_pose() {
            return Ok(None);
        }
        if let Some(points) =
            self.mesh
                .sample_skeletal_attachment(animation, self.sequence, self.phase)?
        {
            return Ok(triangle_attachment_transform(points));
        }
        self.mesh.sample_skeletal_weapon_transform(
            animation,
            self.sequence,
            self.phase,
            self.root_motion,
        )
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
    physics: u8,
    collision_height: f32,
    collide_type: u8,
    collide_world: bool,
    align_bottom: bool,
    draw_scale: f32,
    draw_type: u8,
    brush: Option<SceneObject>,
    main_scale: Vec3,
    mesh: Option<SceneObject>,
    skeletal_animation: Option<SceneObject>,
    skin: Option<SceneObject>,
    texture: Option<SceneObject>,
    default_texture: Option<SceneObject>,
    environment_map: Option<SceneObject>,
    editor_sprite: bool,
    multi_skins: Vec<Option<SceneObject>>,
    style: u8,
    ambient_glow: u8,
    scale_glow: f32,
    opacity: f32,
    light_brightness: u8,
    anim_sequence: Option<String>,
    anim_frame: f32,
    anim_rate: f32,
    texture_pan_speed: Vec2,
    light_hue: u8,
    light_saturation: u8,
    corona: bool,
    hidden: bool,
    unlit: bool,
    mesh_environment_map: bool,
}

#[derive(Clone)]
struct ClassState {
    actor: ActorState,
    is_light: bool,
    diagnostics: Vec<String>,
}

#[derive(Clone, Default)]
struct ActorRenderState {
    actor: ActorState,
    is_light: bool,
}

impl Default for ActorState {
    fn default() -> Self {
        Self {
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            pre_pivot: Vec3::ZERO,
            physics: 0,
            collision_height: 0.0,
            collide_type: 0,
            collide_world: false,
            align_bottom: false,
            draw_scale: 1.0,
            draw_type: 0,
            brush: None,
            main_scale: Vec3::ONE,
            mesh: None,
            skeletal_animation: None,
            skin: None,
            texture: None,
            default_texture: None,
            environment_map: None,
            editor_sprite: false,
            multi_skins: Vec::new(),
            style: 1,
            ambient_glow: 0,
            scale_glow: 1.0,
            opacity: 1.0,
            light_brightness: 64,
            anim_sequence: None,
            anim_frame: 0.0,
            anim_rate: 0.0,
            texture_pan_speed: Vec2::ONE,
            light_hue: 0,
            light_saturation: 255,
            corona: false,
            hidden: false,
            unlit: false,
            mesh_environment_map: false,
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
        if let Some(physics) = properties.physics {
            self.physics = physics;
        }
        if let Some(collision_height) = properties.collision_height {
            self.collision_height = collision_height;
        }
        if let Some(collide_type) = properties.collide_type {
            self.collide_type = collide_type;
        }
        if let Some(collide_world) = properties.collide_world {
            self.collide_world = collide_world;
        }
        if let Some(align_bottom) = properties.align_bottom {
            self.align_bottom = align_bottom;
        }
        if let Some(draw_scale) = properties.draw_scale {
            self.draw_scale = draw_scale;
        }
        if let Some(draw_type) = properties.draw_type {
            self.draw_type = draw_type;
        }
        if let Some(reference) = properties.brush {
            self.brush = packages.resolve(source, reference)?.map(Into::into);
        }
        if let Some(main_scale) = properties.main_scale {
            self.main_scale = main_scale;
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
            self.editor_sprite = false;
            self.texture = match packages.resolve(source, reference) {
                Ok(texture) => texture.map(Into::into),
                Err(ResolveError::MissingObject { package, class, .. })
                    if package.eq_ignore_ascii_case("HPEdit")
                        && class.eq_ignore_ascii_case("Texture") =>
                {
                    self.editor_sprite = true;
                    None
                }
                Err(error) => return Err(error.into()),
            };
            self.editor_sprite |= self.texture.as_ref().is_some_and(|texture| {
                texture.name().starts_with("S_")
                    || Path::new(texture.package.summary().source.as_ref())
                        .file_stem()
                        .is_some_and(|name| name.eq_ignore_ascii_case("HPEdit"))
            });
        }
        if let Some(reference) = properties.default_texture {
            self.default_texture = packages.resolve(source, reference)?.map(Into::into);
        }
        if let Some(reference) = properties.environment_map {
            self.environment_map = packages.resolve(source, reference)?.map(Into::into);
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
        if let Some(opacity) = properties.opacity {
            self.opacity = opacity;
        }
        if let Some(light_brightness) = properties.light_brightness {
            self.light_brightness = light_brightness;
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
        if let Some(hue) = properties.light_hue {
            self.light_hue = hue;
        }
        if let Some(saturation) = properties.light_saturation {
            self.light_saturation = saturation;
        }
        if let Some(corona) = properties.corona {
            self.corona = corona;
        }
        if let Some(hidden) = properties.hidden {
            self.hidden = hidden;
        }
        if let Some(unlit) = properties.unlit {
            self.unlit = unlit;
        }
        if let Some(mesh_environment_map) = properties.mesh_environment_map {
            self.mesh_environment_map = mesh_environment_map;
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
        main_scale: Vec3::ONE,
        draw_scale: 1.0,
        draw_type: 0,
        hidden: false,
        unlit: false,
        brush: None,
        mesh: None,
        mesh_name: None,
        animation: None,
        render: None,
        mesh_transform: None,
        mesh_to_object: None,
        visual_bounds: None,
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
    coronas: &mut Vec<Corona>,
    animations: &mut Vec<AnimatedActorMesh>,
    sprites: &mut Vec<SpriteActor>,
    water_animations: &mut TextureAnimations,
) -> (Vec<SceneActor>, Vec<ActorRenderState>) {
    let mut actors = Vec::new();
    let mut actor_states = Vec::new();
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
            main_scale: Vec3::ONE,
            draw_scale: 1.0,
            draw_type: 0,
            hidden: false,
            unlit: false,
            brush: None,
            mesh: None,
            mesh_name: None,
            animation: None,
            render: None,
            mesh_transform: None,
            mesh_to_object: None,
            visual_bounds: None,
            diagnostics: Vec::new(),
        };
        let actor = match Actor::decode(map, export_index) {
            Ok(actor) => actor,
            Err(error) => {
                warn!(export_index, %error, "could not decode actor");
                scene_actor
                    .diagnostics
                    .push(format!("actor decode failed: {error}"));
                actor_states.push(ActorRenderState::default());
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
                actor_states.push(ActorRenderState::default());
                actors.push(scene_actor);
                continue;
            }
            Err(error) => {
                warn!(export_index, %error, "could not resolve actor class");
                scene_actor
                    .diagnostics
                    .push(format!("class resolution failed: {error}"));
                actor_states.push(ActorRenderState::default());
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
        let ClassState {
            actor: mut state,
            is_light,
            diagnostics,
        } = class_state;
        scene_actor.diagnostics.extend(diagnostics);
        if let Err(error) = state.apply(&mut actor_render.packages, map, &actor.properties) {
            warn!(export_index, %error, "could not resolve actor properties");
            scene_actor
                .diagnostics
                .push(format!("actor property resolution failed: {error}"));
            actor_states.push(ActorRenderState {
                actor: state,
                is_light,
            });
            actors.push(scene_actor);
            continue;
        }
        apply_scene_actor_state(&mut scene_actor, &state);
        append_scene_actor_corona(
            actor_render,
            actors.len(),
            &state,
            textures,
            coronas,
            water_animations,
        );
        append_scene_actor_render(
            actor_render,
            &mut scene_actor,
            &state,
            is_light,
            actors.len(),
            render_mesh,
            textures,
            materials,
            animations,
            sprites,
            water_animations,
        );
        actor_states.push(ActorRenderState {
            actor: state,
            is_light,
        });
        actors.push(scene_actor);
    }
    (actors, actor_states)
}

fn append_scene_actor_corona(
    actor_render: &mut ActorRenderContext,
    actor_index: usize,
    state: &ActorState,
    textures: &mut Vec<TextureImage>,
    coronas: &mut Vec<Corona>,
    water_animations: &mut TextureAnimations,
) {
    if !state.corona {
        return;
    }
    let Some(skin) = state.skin.as_ref() else {
        return;
    };
    let texture = actor_surface_material(
        &mut actor_render.packages,
        Some(skin),
        0,
        state,
        textures,
        &mut actor_render.decoded_textures,
        &mut actor_render.images,
        water_animations,
    )
    .texture;
    let Some(texture) = texture else {
        return;
    };
    coronas.push(Corona {
        actor_index,
        location: state.location,
        texture,
        draw_scale: state.draw_scale,
        color: hsb_to_rgb(state.light_hue, state.light_saturation, 255),
    });
}

#[allow(clippy::too_many_arguments)]
fn append_scene_actor_render(
    actor_render: &mut ActorRenderContext,
    scene_actor: &mut SceneActor,
    state: &ActorState,
    is_light: bool,
    actor_index: usize,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    animations: &mut Vec<AnimatedActorMesh>,
    sprites: &mut Vec<SpriteActor>,
    water_animations: &mut TextureAnimations,
) {
    if is_light && !state.editor_sprite && state.texture.is_some() {
        append_scene_actor_sprite(
            actor_render,
            scene_actor,
            state,
            actor_index,
            render_mesh,
            textures,
            materials,
            sprites,
            water_animations,
        );
        return;
    }
    if state.draw_type == 0 {
        return;
    }
    if matches!(state.draw_type, 1 | 4) && is_light {
        return;
    }
    if matches!(state.draw_type, 1 | 4) {
        append_scene_actor_sprite(
            actor_render,
            scene_actor,
            state,
            actor_index,
            render_mesh,
            textures,
            materials,
            sprites,
            water_animations,
        );
        return;
    }
    if state.draw_type == 3 {
        append_scene_actor_brush(
            actor_render,
            scene_actor,
            state,
            render_mesh,
            textures,
            materials,
            water_animations,
        );
        return;
    }
    if state.draw_type == 8 {
        return;
    }
    if state.draw_type != 2 {
        scene_actor.diagnostics.push(format!(
            "DrawType {} is not rendered as a mesh",
            state.draw_type
        ));
        return;
    }
    let Some(mesh_object) = state.mesh.clone() else {
        return;
    };
    let environment_map = state.mesh_environment_map.then(|| {
        let zone = actor_render.model.zone_at(state.location);
        select_environment_map(
            state.texture.clone(),
            actor_render
                .zone_environment_maps
                .get(zone)
                .and_then(Clone::clone),
            actor_render.level_environment_map.clone(),
        )
    });
    let environment_map = environment_map.flatten();
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
    let animation_source =
        actor_animation_source(packages, animation_cache, state, &mesh_object, &mesh);
    if let ActorAnimationSource::Error(error) = &animation_source {
        warn!(actor = %scene_actor.name, %error, "could not decode actor animation metadata");
        let diagnostic = format!("animation metadata could not be decoded: {error}");
        if !scene_actor.diagnostics.contains(&diagnostic) {
            scene_actor.diagnostics.push(diagnostic);
        }
    }
    match append_actor_mesh(
        packages,
        &mesh_object,
        &mesh,
        &animation_source,
        state,
        environment_map.as_ref(),
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
            scene_actor.mesh_transform = Some(appended.transform);
            scene_actor.mesh_to_object = Some(appended.mesh_to_object);
            scene_actor.visual_bounds = appended.visual_bounds;
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

fn actor_animation_source(
    packages: &mut PackageStore,
    animation_cache: &mut HashMap<
        SceneObjectId,
        std::result::Result<Arc<SkeletalAnimation>, String>,
    >,
    state: &ActorState,
    mesh_object: &SceneObject,
    mesh: &Mesh,
) -> ActorAnimationSource {
    let animation_object = state.skeletal_animation.clone().map_or_else(
        || {
            packages
                .resolve(&mesh_object.package, mesh.default_animation)
                .map(|animation| animation.map(SceneObject::from))
                .map_err(|error| format!("could not resolve actor skeletal animation: {error}"))
        },
        |animation| Ok(Some(animation)),
    );
    let Some(animation_object) = (match animation_object {
        Ok(animation) => animation,
        Err(error) => return ActorAnimationSource::Error(error),
    }) else {
        return ActorAnimationSource::Legacy;
    };
    let key = animation_object.id();
    let decoded = animation_cache.entry(key).or_insert_with(|| {
        SkeletalAnimation::decode(&animation_object.package, animation_object.export_index)
            .map(Arc::new)
            .map_err(|error| format!("could not decode actor skeletal animation: {error}"))
    });
    match decoded {
        Ok(animation) => ActorAnimationSource::Skeletal(Arc::clone(animation)),
        Err(error) => ActorAnimationSource::Error(error.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
fn append_scene_actor_sprite(
    actor_render: &mut ActorRenderContext,
    scene_actor: &mut SceneActor,
    state: &ActorState,
    actor_index: usize,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    sprites: &mut Vec<SpriteActor>,
    water_animations: &mut TextureAnimations,
) {
    if state.editor_sprite {
        return;
    }
    let Some(texture) = state.texture.as_ref() else {
        return;
    };
    let material = actor_surface_material(
        &mut actor_render.packages,
        Some(texture),
        PolyFlags::TWO_SIDED.bits(),
        state,
        textures,
        &mut actor_render.decoded_textures,
        &mut actor_render.images,
        water_animations,
    );
    let Some(texture_index) = material.texture else {
        scene_actor
            .diagnostics
            .push("sprite texture could not be decoded".to_owned());
        return;
    };
    let dimensions = Vec2::new(
        textures[texture_index].width as f32,
        textures[texture_index].height as f32,
    );
    let surface = materials.len();
    materials.push(SurfaceMaterial {
        unlit: true,
        ..material
    });
    let half_size = dimensions * state.draw_scale * 0.5;
    let first_vertex = render_mesh.positions.len();
    let first_index = render_mesh.indices.len();
    let center = state.location + state.pre_pivot;
    for (position, uv) in sprite_positions(center, half_size, Mat4::IDENTITY)
        .into_iter()
        .zip([
            Vec2::ZERO,
            Vec2::new(dimensions.x, 0.0),
            dimensions,
            Vec2::new(0.0, dimensions.y),
        ])
    {
        render_mesh.positions.push(position);
        render_mesh.normals.push(Vec3::ZERO);
        render_mesh.texture_coordinates.push(uv);
        render_mesh.lightmap_coordinates.push(Vec2::ZERO);
        render_mesh.vertex_lightmaps.push(None);
        render_mesh
            .vertex_colors
            .push(Vec3::splat(state.scale_glow.clamp(0.0, 1.0)));
        render_mesh.vertex_surfaces.push(surface);
    }
    let base = first_vertex as u32;
    render_mesh
        .indices
        .extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    render_mesh
        .triangle_surfaces
        .extend_from_slice(&[surface, surface]);
    scene_actor.render = Some(SceneActorRenderRange {
        vertices: first_vertex..render_mesh.positions.len(),
        indices: first_index..render_mesh.indices.len(),
    });
    sprites.push(SpriteActor {
        actor_index,
        half_size,
        texture: texture_index,
    });
}

fn sprite_positions(center: Vec3, half_size: Vec2, view_rotation: Mat4) -> [Vec3; 4] {
    let side = view_rotation.transform_vector3(Vec3::Y) * half_size.x;
    let up = view_rotation.transform_vector3(Vec3::Z) * half_size.y;
    [
        center - side - up,
        center + side - up,
        center + side + up,
        center - side + up,
    ]
}

fn particle_sprite_positions(
    center: Vec3,
    half_size: Vec2,
    view_rotation: Mat4,
    spin: f32,
) -> [Vec3; 4] {
    let (sin, cos) = spin.sin_cos();
    let view_side = view_rotation.transform_vector3(Vec3::Y);
    let view_up = view_rotation.transform_vector3(Vec3::Z);
    let side = (view_side * cos + view_up * sin) * half_size.x;
    let up = (view_up * cos - view_side * sin) * half_size.y;
    [
        center - side - up,
        center + side - up,
        center + side + up,
        center - side + up,
    ]
}

fn particle_render_primitive_positions(
    center: Vec3,
    half_size: Vec2,
    view_rotation: Mat4,
    spin: f32,
    render_primitive: u8,
) -> [Vec3; 4] {
    if render_primitive != 2 {
        return particle_sprite_positions(center, half_size, view_rotation, spin);
    }
    let side = view_rotation.transform_vector3(Vec3::Y) * half_size.x;
    let down = -view_rotation.transform_vector3(Vec3::Z) * half_size.y;
    [
        center - side + down * 1.5,
        center + down * 2.0,
        center + side + down,
        center,
    ]
}

#[allow(clippy::too_many_arguments)]
fn append_scene_actor_brush(
    actor_render: &mut ActorRenderContext,
    scene_actor: &mut SceneActor,
    state: &ActorState,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    water_animations: &mut TextureAnimations,
) {
    let Some(brush_object) = state.brush.clone() else {
        scene_actor
            .diagnostics
            .push("brush draw type has no brush assigned".to_owned());
        return;
    };
    let ActorRenderContext {
        packages,
        model: world_model,
        vertex_lighting,
        brush_cache,
        decoded_textures,
        images,
        ..
    } = actor_render;
    let brush_model = match Model::decode(&brush_object.package, brush_object.export_index) {
        Ok(model) => model,
        Err(error) => {
            scene_actor
                .diagnostics
                .push(format!("brush model could not be decoded: {error}"));
            return;
        }
    };
    let ObjectReference::Export(polys_export) = brush_model.polys else {
        scene_actor
            .diagnostics
            .push("brush model has no local Polys export".to_owned());
        return;
    };
    let polys_key = SceneObjectId {
        package: brush_object.package.summary().source.to_string(),
        export_index: polys_export,
    };
    if !brush_cache.contains_key(&polys_key) {
        let decoded = BrushPolys::decode(&brush_object.package, polys_export)
            .map(Arc::new)
            .map_err(|error| {
                scene_actor
                    .diagnostics
                    .push(format!("brush polygons could not be decoded: {error}"));
            })
            .ok();
        brush_cache.insert(polys_key.clone(), decoded);
    }
    let Some(polys) = brush_cache
        .get(&polys_key)
        .and_then(Option::as_ref)
        .cloned()
    else {
        return;
    };
    match append_actor_brush(
        packages,
        &polys,
        state,
        world_model,
        vertex_lighting,
        render_mesh,
        textures,
        materials,
        decoded_textures,
        images,
        water_animations,
    ) {
        Ok(Some(render)) => scene_actor.render = Some(render),
        Ok(None) => scene_actor
            .diagnostics
            .push("brush contains no renderable polygons".to_owned()),
        Err(error) => scene_actor
            .diagnostics
            .push(format!("brush assembly failed: {error}")),
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
            is_light: false,
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
                is_light: class.name().eq_ignore_ascii_case("Light"),
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
                is_light: base.is_light,
                diagnostics: Vec::new(),
            }
        }
        Ok(None) => ClassState {
            actor: ActorState::default(),
            is_light: false,
            diagnostics: Vec::new(),
        },
        Err(error) => {
            let error = format!("base class resolution failed for {}: {error}", class.name());
            ClassState {
                actor: ActorState::default(),
                is_light: false,
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
    state.is_light |= class.name().eq_ignore_ascii_case("Light");
    cache.insert(key, state.clone());
    state
}

fn apply_scene_actor_state(actor: &mut SceneActor, state: &ActorState) {
    actor.location = state.location;
    actor.rotation = state.rotation;
    actor.pre_pivot = state.pre_pivot;
    actor.main_scale = state.main_scale;
    actor.draw_scale = state.draw_scale;
    actor.draw_type = state.draw_type;
    actor.hidden = state.hidden;
    actor.unlit = state.unlit;
    actor.brush = state.brush.as_ref().map(SceneObject::id);
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
    transform: Mat4,
    mesh_to_object: Mat4,
    visual_bounds: Option<(Vec3, Vec3)>,
}

#[allow(clippy::too_many_arguments)]
fn append_actor_brush(
    packages: &mut PackageStore,
    polys: &BrushPolys,
    actor: &ActorState,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    decoded_textures: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
    water_animations: &mut TextureAnimations,
) -> Result<Option<SceneActorRenderRange>> {
    ensure!(
        actor.main_scale.is_finite(),
        "brush MainScale is not finite"
    );
    let transform = brush_transform(actor);
    let mirrored = transform.determinant() < 0.0;
    let normal_transform = Mat3::from_mat4(transform).inverse().transpose();
    let transformed = polys
        .polygons
        .iter()
        .flat_map(|polygon| &polygon.vertices)
        .map(|&position| transform.transform_point3(position))
        .collect::<Vec<_>>();
    let Some(&first_position) = transformed.first() else {
        return Ok(None);
    };
    let (minimum, maximum) = transformed.iter().copied().fold(
        (first_position, first_position),
        |(minimum, maximum), position| (minimum.min(position), maximum.max(position)),
    );
    let center = (minimum + maximum) * 0.5;
    let actor_lighting =
        vertex_lighting.for_actor(model, center, actor.ambient_glow, actor.scale_glow);
    let zone_unlit = model.zone_at(center) == 0;
    let first_vertex = render_mesh.positions.len();
    let first_index = render_mesh.indices.len();

    for polygon in &polys.polygons {
        if polygon.vertices.len() < 3 {
            continue;
        }
        let texture = packages
            .resolve(
                &actor
                    .brush
                    .as_ref()
                    .context("brush actor has no model object")?
                    .package,
                polygon.texture,
            )?
            .map(SceneObject::from);
        let surface = materials.len();
        materials.push(actor_surface_material(
            packages,
            texture.as_ref(),
            polygon.poly_flags.bits(),
            actor,
            textures,
            decoded_textures,
            images,
            water_animations,
        ));
        let base = u32::try_from(render_mesh.positions.len())?;
        let normal = (normal_transform * polygon.normal).normalize_or_zero();
        let unlit = actor.unlit || polygon.poly_flags.contains(PolyFlags::UNLIT) || zone_unlit;
        for &vertex in &polygon.vertices {
            let position = transform.transform_point3(vertex);
            render_mesh.positions.push(position);
            render_mesh.normals.push(normal);
            render_mesh.texture_coordinates.push(Vec2::new(
                polygon.texture_u.dot(vertex - polygon.base) + f32::from(polygon.pan_u),
                polygon.texture_v.dot(vertex - polygon.base) + f32::from(polygon.pan_v),
            ));
            render_mesh.lightmap_coordinates.push(Vec2::ZERO);
            render_mesh.vertex_lightmaps.push(None);
            render_mesh
                .vertex_colors
                .push(actor_lighting.color(position, normal, unlit));
            render_mesh.vertex_surfaces.push(surface);
        }
        for offset in 1..u32::try_from(polygon.vertices.len() - 1)? {
            render_mesh
                .indices
                .extend_from_slice(&brush_triangle(base, offset, mirrored));
            render_mesh.triangle_surfaces.push(surface);
        }
    }
    Ok(
        (render_mesh.positions.len() != first_vertex).then_some(SceneActorRenderRange {
            vertices: first_vertex..render_mesh.positions.len(),
            indices: first_index..render_mesh.indices.len(),
        }),
    )
}

fn brush_transform(actor: &ActorState) -> Mat4 {
    Mat4::from_translation(actor.location)
        * rotation_matrix(actor.rotation)
        * Mat4::from_scale(actor.main_scale)
        * Mat4::from_translation(-actor.pre_pivot)
}

fn brush_triangle(base: u32, offset: u32, mirrored: bool) -> [u32; 3] {
    if mirrored {
        [base, base + offset + 1, base + offset]
    } else {
        [base, base + offset, base + offset + 1]
    }
}

#[allow(clippy::too_many_arguments)]
fn append_actor_mesh(
    packages: &mut PackageStore,
    mesh_object: &SceneObject,
    mesh: &Arc<Mesh>,
    animation_source: &ActorAnimationSource,
    actor: &ActorState,
    environment_map: Option<&SceneObject>,
    actor_index: usize,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    decoded_textures: &mut HashMap<SceneObjectId, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
    animations: &mut Vec<AnimatedActorMesh>,
    water_animations: &mut TextureAnimations,
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
    let mesh_to_object = mesh_to_object_transform(
        mesh.scale,
        mesh.origin,
        Rotator {
            pitch: mesh.rotation_origin.x,
            yaw: mesh.rotation_origin.y,
            roll: mesh.rotation_origin.z,
        },
    );
    let is_skeletal_mesh = mesh_object
        .package
        .summary()
        .class_name(&mesh_object.package.summary().exports[mesh_object.export_index])
        == Some("SkeletalMesh");
    let local_transform = Mat4::from_translation(skeletal_mesh_adjust(
        is_skeletal_mesh,
        mesh.bounds,
        mesh.origin,
        mesh.scale,
        actor,
    )) * Mat4::from_scale(Vec3::splat(actor.draw_scale))
        * mesh_to_object;
    let visual_bounds = mesh.bounds.map(|(minimum, maximum)| {
        let center = local_transform.transform_point3((minimum + maximum) * 0.5);
        let extents = Mat3::from_mat4(local_transform).abs() * ((maximum - minimum) * 0.5);
        (center - extents, center + extents)
    });
    let transform = Mat4::from_translation(actor.location + actor.pre_pivot)
        * rotation_matrix(actor.rotation)
        * local_transform;
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
    let sequences = animation_source.sequences(mesh).unwrap_or_default();
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
        sampled = if let ActorAnimationSource::Skeletal(animation) = animation_source {
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
            let texture = if actor.mesh_environment_map {
                environment_map.cloned()
            } else {
                select_actor_texture(actor, &mesh_textures, triangle.texture_index)
            };
            let mut material = actor_surface_material(
                packages,
                texture.as_ref(),
                triangle.poly_flags,
                actor,
                textures,
                decoded_textures,
                images,
                water_animations,
            );
            if material.texture.is_none() {
                material.mode = SurfaceMode::Hidden;
            }
            material.environment_map = actor.mesh_environment_map;
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
        let vertex_unlit = unlit || materials[surface].unlit;
        for vertex in triangle.vertices {
            let position = transform.transform_point3(vertex.position);
            let normal = (normal_transform * vertex.normal).normalize_or_zero();
            render_mesh.positions.push(position);
            render_mesh.normals.push(normal);
            render_mesh
                .texture_coordinates
                .push(actor_mesh_texture_coordinates(
                    vertex.texture_coordinates,
                    dimensions,
                    materials[surface],
                ));
            render_mesh.lightmap_coordinates.push(Vec2::ZERO);
            render_mesh.vertex_lightmaps.push(None);
            render_mesh
                .vertex_colors
                .push(actor_lighting.color(position, normal, vertex_unlit));
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
        let bone_positions = match animation_source {
            ActorAnimationSource::Skeletal(animation) => mesh
                .sample_skeletal_bone_positions(animation, sequence.unwrap_or(0), phase, false)?
                .into_iter()
                .map(|position| transform.transform_point3(position))
                .collect(),
            ActorAnimationSource::Legacy | ActorAnimationSource::Error(_) => Vec::new(),
        };
        animations.push(AnimatedActorMesh {
            actor_index,
            mesh: Arc::clone(mesh),
            skeletal_animation: match animation_source {
                ActorAnimationSource::Skeletal(animation) => Some(Arc::clone(animation)),
                ActorAnimationSource::Legacy | ActorAnimationSource::Error(_) => None,
            },
            sequence: sequence.unwrap_or(0),
            phase,
            rate: animation.as_ref().map_or(0.0, |animation| animation.rate),
            playing: animation.is_some(),
            looping: true,
            root_motion: false,
            root_motion_position: Vec3::ZERO,
            tween_from: None,
            tween_attachment_from: None,
            tween_bone_positions_from: None,
            bone_positions,
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
            transform,
            mesh_to_object,
            visual_bounds,
        }),
    )
}

fn actor_mesh_texture_coordinates(
    coordinates: Vec2,
    dimensions: Vec2,
    material: SurfaceMaterial,
) -> Vec2 {
    let draw_scale = if material.environment_map {
        1.0
    } else {
        material.texture_draw_scale
    };
    coordinates * dimensions * draw_scale
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
    water_animations: &mut TextureAnimations,
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
    let opacity = if actor.opacity.is_finite() {
        actor.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let Some(texture) = texture else {
        return actor_opacity_material(SurfaceMaterial {
            opacity,
            ..surface_material(flags, None, None)
        });
    };
    let key = texture.id();
    if !decoded.contains_key(&key) {
        let resolved = ResolvedObject {
            package: Arc::clone(&texture.package),
            export_index: texture.export_index,
        };
        let value = match decode_texture(packages, &resolved, true) {
            Ok(texture) => Some(texture),
            Err(error) => {
                warn!(%error, "could not decode actor texture");
                None
            }
        };
        decoded.insert(key.clone(), value);
    }
    let Some(texture) = decoded.get(&key).and_then(Option::as_ref) else {
        return actor_opacity_material(SurfaceMaterial {
            opacity,
            ..surface_material(flags, None, None)
        });
    };
    let mut material = actor_opacity_material(SurfaceMaterial {
        opacity,
        texture_draw_scale: texture.texture.draw_scale,
        ..surface_material(flags, None, Some(texture.texture.render_flags))
    });
    let image_key = (key.package, key.export_index, material.masked);
    let image = if let Some(index) = images.get(&image_key) {
        Some(*index)
    } else {
        match append_texture_image(textures, water_animations, texture, material.masked, true) {
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

fn actor_opacity_material(mut material: SurfaceMaterial) -> SurfaceMaterial {
    if material.opacity < 1.0
        && !matches!(material.mode, SurfaceMode::Backdrop | SurfaceMode::Hidden)
    {
        material.mode = SurfaceMode::AlphaBlended;
        material.masked = false;
    }
    material
}

fn rotation_matrix(rotation: Rotator) -> Mat4 {
    let radians = rotation.radians();
    Mat4::from_rotation_z(radians.y)
        * Mat4::from_rotation_y(-radians.x)
        * Mat4::from_rotation_x(-radians.z)
}

fn rotate_unreal(rotation: Rotator, vector: Vec3) -> Vec3 {
    let radians = rotation.radians();
    let (pitch_sin, pitch_cos) = radians.x.sin_cos();
    let (yaw_sin, yaw_cos) = radians.y.sin_cos();
    let (roll_sin, roll_cos) = radians.z.sin_cos();
    let forward = Vec3::new(pitch_cos * yaw_cos, pitch_cos * yaw_sin, pitch_sin);
    let right = Vec3::new(
        roll_sin * pitch_sin * yaw_cos - roll_cos * yaw_sin,
        roll_sin * pitch_sin * yaw_sin + roll_cos * yaw_cos,
        -roll_sin * pitch_cos,
    );
    let up = Vec3::new(
        -roll_cos * pitch_sin * yaw_cos - roll_sin * yaw_sin,
        -roll_cos * pitch_sin * yaw_sin + roll_sin * yaw_cos,
        roll_cos * pitch_cos,
    );
    forward * vector.x + right * vector.y + up * vector.z
}

fn load_warp_portals(
    package: &Package,
    model: &Model,
    surface_materials: &[SurfaceMaterial],
    actors: &[SceneActor],
    actor_indices: &HashMap<usize, usize>,
) -> Result<Vec<WarpPortal>> {
    model
        .surfaces
        .iter()
        .enumerate()
        .filter(|(surface, _)| is_portal_surface(surface_materials, *surface))
        .filter_map(|(surface, _)| {
            let (source_actor, plane, source_on_positive_side) = model
                .nodes
                .iter()
                .filter(|node| node.surface == surface as i32)
                .flat_map(|node| {
                    node.zones
                        .into_iter()
                        .enumerate()
                        .map(move |(side, zone)| (node, side, zone))
                })
                .filter_map(|(node, side, zone)| {
                    let zone = usize::try_from(zone)
                        .ok()
                        .and_then(|zone| model.zones.get(zone))?;
                    let ObjectReference::Export(export) = zone.actor else {
                        return None;
                    };
                    actor_indices
                        .get(&export)
                        .copied()
                        .map(|actor| (actor, node.plane, side == 0))
                })
                .find(|(actor, _, _)| {
                    actors[*actor]
                        .class_name
                        .eq_ignore_ascii_case("WarpZoneInfo")
                })?;
            Some((surface, source_actor, plane, source_on_positive_side))
        })
        .map(|(surface, source_actor, plane, source_on_positive_side)| {
            let actor = Actor::decode(package, actors[source_actor].id.export_index)
                .context("could not decode warp-zone coordinates")?;
            let [origin, x_axis, y_axis, z_axis] = actor
                .properties
                .warp_coordinates
                .context("warp zone has no authored WarpCoords")?;
            Ok(WarpPortal {
                surface,
                source_actor,
                plane,
                source_on_positive_side,
                source: WarpCoordinates {
                    origin,
                    axes: [x_axis, y_axis, z_axis],
                },
                destination_actor: None,
                destination: None,
            })
        })
        .collect()
}

fn is_portal_surface(surface_materials: &[SurfaceMaterial], surface: usize) -> bool {
    surface_materials
        .get(surface)
        .is_some_and(|material| material.portal)
}

fn mesh_to_object_transform(scale: Vec3, origin: Vec3, rotation_origin: Rotator) -> Mat4 {
    rotation_matrix(rotation_origin) * Mat4::from_scale(scale) * Mat4::from_translation(-origin)
}

fn skeletal_mesh_adjust(
    is_skeletal_mesh: bool,
    bounds: Option<(Vec3, Vec3)>,
    origin: Vec3,
    scale: Vec3,
    actor: &ActorState,
) -> Vec3 {
    const CT_SHAPE: u8 = 3;
    const TOLERANCE: f32 = 2.5;

    if !is_skeletal_mesh
        || !actor.align_bottom
        || !actor.collide_world
        || actor.physics == 0
        || actor.collide_type == CT_SHAPE
    {
        return Vec3::ZERO;
    }
    let Some((minimum, _)) = bounds else {
        return Vec3::ZERO;
    };
    Vec3::Z
        * ((origin.z - minimum.z) * scale.z * actor.draw_scale - actor.collision_height - TOLERANCE)
}

fn load_materials(
    packages: &mut PackageStore,
    map: &std::sync::Arc<openhp1_package::Package>,
    model: &Model,
    default_texture: Option<&SceneObject>,
    water_animations: &mut TextureAnimations,
) -> (Vec<TextureImage>, Vec<SurfaceMaterial>) {
    let mut textures = Vec::new();
    let mut decoded = HashMap::<(String, usize, bool), Option<DecodedTexture>>::new();
    let mut images = HashMap::<(String, usize, bool), usize>::new();
    let mut attachment_images = HashMap::<(String, usize), usize>::new();
    let mut materials = Vec::with_capacity(model.surfaces.len());
    for (surface_index, surface) in model.surfaces.iter().enumerate() {
        let raw_texture_present = !matches!(surface.texture, ObjectReference::None);
        let authored = match surface.texture {
            ObjectReference::None => None,
            reference => Some(packages.resolve(map, reference)),
        };
        let default = default_texture.map(|texture| ResolvedObject {
            package: Arc::clone(&texture.package),
            export_index: texture.export_index,
        });
        let resolved = match select_bsp_texture(authored, default) {
            Ok(resolved) => resolved,
            Err(error) => {
                warn!(surface_index, %error, "could not resolve surface texture");
                materials.push(bsp_surface_material(surface.poly_flags, None, None));
                continue;
            }
        };
        let Some(resolved) = resolved else {
            materials.push(bsp_surface_material(surface.poly_flags, None, None));
            continue;
        };
        let key = (
            resolved.package.summary().source.to_string(),
            resolved.export_index,
            true,
        );
        if !decoded.contains_key(&key) {
            let texture = match decode_texture(packages, &resolved, true) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    warn!(surface_index, %error, "could not decode surface texture");
                    None
                }
            };
            decoded.insert(key.clone(), texture);
        }
        let Some(decoded_texture) = decoded.get(&key).and_then(Option::as_ref) else {
            materials.push(bsp_surface_material(surface.poly_flags, None, None));
            continue;
        };
        let texture_flags = decoded_texture.texture.render_flags;
        let macro_reference = decoded_texture.texture.macro_texture;
        let detail_reference = decoded_texture.texture.detail_texture;
        let animated_attachments = decoded_texture.generic.as_ref().map(|animation| {
            animation
                .frames
                .iter()
                .map(|(texture, _, owner)| {
                    (
                        texture.macro_texture,
                        texture.detail_texture,
                        Arc::clone(owner),
                    )
                })
                .collect::<Vec<_>>()
        });
        let mut material = bsp_surface_material(
            surface.poly_flags,
            None,
            raw_texture_present.then_some(texture_flags),
        );
        material.portal =
            bsp_root_portal(surface.poly_flags, raw_texture_present, Some(texture_flags));
        if material.mode == SurfaceMode::Hidden {
            materials.push(material);
            continue;
        }
        let volumetric_source = material.volumetric_source
            || is_window_texture(
                resolved
                    .package
                    .summary()
                    .exports
                    .get(resolved.export_index)
                    .map(|export| resolved.package.summary().name(export.object_name))
                    .unwrap_or_default(),
            );
        if material.mode == SurfaceMode::Backdrop {
            materials.push(SurfaceMaterial {
                volumetric_source,
                ..material
            });
            continue;
        }
        let image_key = (key.0.clone(), key.1, material.masked);
        let texture_index = if let Some(index) = images.get(&image_key) {
            *index
        } else {
            let index = match append_texture_image(
                &mut textures,
                water_animations,
                decoded_texture,
                material.masked,
                true,
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
        let macro_attachment = load_texture_attachment(
            packages,
            &resolved.package,
            macro_reference,
            &mut decoded,
            &mut attachment_images,
            &mut textures,
            water_animations,
        )
        .inspect_err(|error| warn!(surface_index, %error, "could not decode macro texture"))
        .ok()
        .flatten();
        let detail_attachment = (!material.portal)
            .then(|| {
                load_texture_attachment(
                    packages,
                    &resolved.package,
                    detail_reference,
                    &mut decoded,
                    &mut attachment_images,
                    &mut textures,
                    water_animations,
                )
            })
            .transpose()
            .inspect_err(|error| warn!(surface_index, %error, "could not decode detail texture"))
            .ok()
            .flatten()
            .flatten();
        if let Some(frames) = animated_attachments {
            let frames = frames
                .into_iter()
                .map(|(macro_reference, detail_reference, owner)| {
                    let macro_texture = load_texture_attachment(
                        packages,
                        &owner,
                        macro_reference,
                        &mut decoded,
                        &mut attachment_images,
                        &mut textures,
                        water_animations,
                    )?;
                    let detail_texture = if material.portal {
                        None
                    } else {
                        load_texture_attachment(
                            packages,
                            &owner,
                            detail_reference,
                            &mut decoded,
                            &mut attachment_images,
                            &mut textures,
                            water_animations,
                        )?
                    };
                    Ok(SurfaceAttachmentFrame {
                        macro_texture: macro_texture.map(|attachment| attachment.0),
                        detail_texture: detail_texture.map(|attachment| attachment.0),
                        macro_draw_scale: macro_texture.map_or(1.0, |attachment| attachment.1),
                        detail_draw_scale: detail_texture.map_or(1.0, |attachment| attachment.1),
                    })
                })
                .collect::<Result<Vec<_>>>();
            match frames {
                Ok(frames) => {
                    if let Some(animation) = water_animations
                        .generic
                        .iter()
                        .rposition(|animation| animation.texture == Some(texture_index))
                    {
                        water_animations
                            .attachments
                            .push(AnimatedSurfaceAttachments {
                                surface: surface_index,
                                animation,
                                frames,
                            });
                    }
                }
                Err(error) => {
                    warn!(surface_index, %error, "could not decode animated texture attachments");
                }
            }
        }
        materials.push(SurfaceMaterial {
            texture: Some(texture_index),
            macro_texture: macro_attachment.map(|attachment| attachment.0),
            detail_texture: detail_attachment.map(|attachment| attachment.0),
            macro_draw_scale: macro_attachment.map_or(1.0, |attachment| attachment.1),
            detail_draw_scale: detail_attachment.map_or(1.0, |attachment| attachment.1),
            bsp_texture_pan: [f32::from(surface.pan_u), f32::from(surface.pan_v)],
            volumetric_source,
            ..material
        });
    }

    (textures, materials)
}

fn select_bsp_texture<T, E>(
    authored: Option<std::result::Result<Option<T>, E>>,
    default: Option<T>,
) -> std::result::Result<Option<T>, E> {
    match authored {
        Some(resolved) => resolved,
        None => Ok(default),
    }
}

#[allow(clippy::too_many_arguments)]
fn load_texture_attachment(
    packages: &mut PackageStore,
    owner: &Arc<Package>,
    reference: ObjectReference,
    decoded: &mut HashMap<(String, usize, bool), Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize), usize>,
    textures: &mut Vec<TextureImage>,
    animations: &mut TextureAnimations,
) -> Result<Option<(usize, f32)>> {
    let Some(resolved) = packages.resolve(owner, reference)? else {
        return Ok(None);
    };
    let key = (
        resolved.package.summary().source.to_string(),
        resolved.export_index,
        false,
    );
    if !decoded.contains_key(&key) {
        let texture = decode_texture(packages, &resolved, false)?;
        decoded.insert(key.clone(), Some(texture));
    }
    let Some(texture) = decoded.get(&key).and_then(Option::as_ref) else {
        return Ok(None);
    };
    let draw_scale = texture.texture.draw_scale;
    // SetTexture receives zero poly flags for both native attachment passes;
    // palette index zero therefore stays opaque even on a masked base surface.
    let image_key = (key.0.clone(), key.1);
    let index = if let Some(index) = images.get(&image_key) {
        *index
    } else {
        let index = append_texture_image(textures, animations, texture, false, false)?;
        images.insert(image_key, index);
        index
    };
    Ok(Some((index, draw_scale)))
}

fn is_window_texture(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("win")
        && !["arch", "column", "wood", "wallwindow", "furnace"]
            .iter()
            .any(|token| name.contains(token))
}

fn load_zone_pan_speeds(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    level: &Level,
    model: &Model,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> (Vec2, Vec<Vec2>) {
    let level_pan_speed = level
        .level_info_export()
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

    let zone_pan_speeds = model
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
        .collect();
    (level_pan_speed, zone_pan_speeds)
}

fn load_environment_maps(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    level: &Level,
    model: &Model,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> (Option<SceneObject>, Vec<Option<SceneObject>>) {
    let level_map = level.level_info_export().and_then(|export_index| {
        actor_environment_map(packages, map, export_index, class_cache)
            .inspect_err(|error| {
                warn!(export_index, %error, "could not decode LevelInfo environment map");
            })
            .ok()
            .flatten()
    });
    let zone_maps = model
        .zones
        .iter()
        .enumerate()
        .map(|(zone_index, zone)| {
            let ObjectReference::Export(export_index) = zone.actor else {
                return None;
            };
            actor_environment_map(packages, map, export_index, class_cache).unwrap_or_else(
                |error| {
                    warn!(zone_index, export_index, %error, "could not decode zone environment map");
                    None
                },
            )
        })
        .collect();
    (level_map, zone_maps)
}

fn select_environment_map<T>(actor: Option<T>, zone: Option<T>, level: Option<T>) -> Option<T> {
    actor.or(zone).or(level)
}

fn bsp_texture_pan_speeds(
    model: &Model,
    level_pan_speed: Vec2,
    zone_pan_speeds: &[Vec2],
) -> Vec<[f32; 4]> {
    let mut speeds = Vec::new();
    for node in model.nodes.iter().filter(|node| node.vertex_count >= 3) {
        let flags = usize::try_from(node.surface)
            .ok()
            .and_then(|surface| model.surfaces.get(surface))
            .map_or(PolyFlags::default(), |surface| surface.poly_flags);
        let speed = node_texture_pan_speeds(flags, node.zones, level_pan_speed, zone_pan_speeds);
        speeds.extend(std::iter::repeat_n(speed, usize::from(node.vertex_count)));
    }
    speeds
}

fn node_texture_pan_speeds(
    flags: PolyFlags,
    zones: [i32; 2],
    level_pan_speed: Vec2,
    zone_pan_speeds: &[Vec2],
) -> [f32; 4] {
    let zone_speed = |zone| {
        usize::try_from(zone)
            .ok()
            .and_then(|zone| zone_pan_speeds.get(zone))
            .copied()
            .unwrap_or(level_pan_speed)
    };
    let [zone0, zone1] = zones.map(zone_speed);
    [
        if flags.contains(PolyFlags::AUTO_U_PAN) {
            zone0.x
        } else {
            0.0
        },
        if flags.contains(PolyFlags::AUTO_V_PAN) {
            zone0.y
        } else {
            0.0
        },
        if flags.contains(PolyFlags::AUTO_U_PAN) {
            zone1.x
        } else {
            0.0
        },
        if flags.contains(PolyFlags::AUTO_V_PAN) {
            zone1.y
        } else {
            0.0
        },
    ]
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

fn actor_environment_map(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    export_index: usize,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> Result<Option<SceneObject>> {
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
    Ok(state.environment_map)
}

fn actor_default_texture(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    export_index: usize,
    class_cache: &mut HashMap<SceneObjectId, ClassState>,
) -> Result<Option<SceneObject>> {
    let actor = Actor::decode(map, export_index)?;
    let export = map
        .summary()
        .exports
        .get(export_index)
        .context("LevelInfo actor export is missing")?;
    let class = packages
        .resolve(map, export.class)?
        .map(SceneObject::from)
        .context("LevelInfo actor class is missing")?;
    let mut state = class_state(packages, &class, class_cache, 0).actor;
    state.apply(packages, map, &actor.properties)?;
    Ok(state.default_texture)
}

fn decode_texture(
    packages: &mut PackageStore,
    resolved: &ResolvedObject,
    follow_generic: bool,
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
    let generic = follow_generic
        .then(|| decode_generic_texture_animation(packages, resolved, &texture, &palette))
        .transpose()?
        .flatten();
    let mut palette = palette;
    let ice = if let Some(ice) = &texture.ice {
        let source_object = packages
            .resolve(&resolved.package, ice.source_texture)?
            .context("ice texture has no source texture")?;
        let source_texture = Texture::decode(&source_object.package, source_object.export_index)?;
        let glass_object = packages
            .resolve(&resolved.package, ice.glass_texture)?
            .context("ice texture has no glass texture")?;
        let glass_texture = Texture::decode(&glass_object.package, glass_object.export_index)?;
        let source = decode_ice_dependency(packages, &source_object, source_texture)?;
        let glass = decode_ice_dependency(packages, &glass_object, glass_texture)?;
        let animation = ice.animate(
            mip.width,
            mip.height,
            source.width,
            source.height,
            &source.pixels,
            glass.width,
            glass.height,
            &glass.pixels,
            texture.min_frame_rate,
            texture.max_frame_rate,
            texture.prime_count,
        )?;
        palette = source.palette.clone();
        Some(DecodedIceTexture {
            animation,
            source,
            glass,
        })
    } else {
        None
    };
    let water = if let Some(wet) = &texture.wet {
        let source_object = packages
            .resolve(&resolved.package, wet.source_texture)?
            .context("wet texture has no source texture")?;
        let source_texture = Texture::decode(&source_object.package, source_object.export_index)?;
        let source = source_texture
            .mips
            .first()
            .context("wet texture source has no mip levels")?;
        let water = wet.animate(
            mip.width,
            mip.height,
            source.width,
            source.height,
            &source.indices,
        )?;
        if water.is_some() {
            let source_palette = packages
                .resolve(&source_object.package, source_texture.palette)?
                .context("wet texture source has no palette reference")?;
            palette = Palette::decode(&source_palette.package, source_palette.export_index)?;
        }
        water
    } else {
        None
    };
    Ok(DecodedTexture {
        id: SceneObjectId {
            package: resolved.package.summary().source.to_string(),
            export_index: resolved.export_index,
        },
        texture,
        palette,
        water,
        ice,
        generic,
    })
}

fn decode_generic_texture_animation(
    packages: &mut PackageStore,
    root: &ResolvedObject,
    texture: &Texture,
    palette: &Palette,
) -> Result<Option<GenericTextureAnimation>> {
    if texture.anim_next == ObjectReference::None {
        return Ok(None);
    }
    let root_mip = texture.mips.first().context("texture has no mip levels")?;
    let root_dimensions = (root_mip.width, root_mip.height);
    let root_id = (root.package.summary().source.to_string(), root.export_index);
    let mut indices = HashMap::from([(root_id, 0)]);
    let mut frames = vec![(texture.clone(), palette.clone(), Arc::clone(&root.package))];
    let mut next = Vec::new();
    let mut current = ResolvedObject {
        package: Arc::clone(&root.package),
        export_index: root.export_index,
    };

    loop {
        ensure!(frames.len() <= 4096, "texture animation chain is too long");
        let reference = frames[next.len()].0.anim_next;
        let Some(resolved) = packages.resolve(&current.package, reference)? else {
            next.push(None);
            break;
        };
        let id = (
            resolved.package.summary().source.to_string(),
            resolved.export_index,
        );
        if let Some(&index) = indices.get(&id) {
            next.push(Some(index));
            break;
        }

        let frame = Texture::decode(&resolved.package, resolved.export_index)?;
        let mip = frame
            .mips
            .first()
            .context("texture animation frame has no mip levels")?;
        ensure!(
            (mip.width, mip.height) == root_dimensions,
            "texture animation frame dimensions differ from the root"
        );
        let frame_palette = packages
            .resolve(&resolved.package, frame.palette)?
            .context("texture animation frame has no palette reference")?;
        let frame_palette = Palette::decode(&frame_palette.package, frame_palette.export_index)?;
        let index = frames.len();
        indices.insert(id, index);
        next.push(Some(index));
        frames.push((frame, frame_palette, Arc::clone(&resolved.package)));
        current = resolved;
    }

    Ok(Some(GenericTextureAnimation {
        frames,
        next,
        prime_count: texture.prime_count,
        min_frame_rate: texture.min_frame_rate,
        max_frame_rate: texture.max_frame_rate,
    }))
}

fn decode_ice_dependency(
    packages: &mut PackageStore,
    resolved: &ResolvedObject,
    texture: Texture,
) -> Result<DecodedIceDependency> {
    ensure!(
        texture.ice.is_none(),
        "recursive IceTexture dependencies are not supported"
    );
    let mip = texture
        .mips
        .first()
        .context("ice dependency has no mip levels")?;
    let palette = packages
        .resolve(&resolved.package, texture.palette)?
        .context("ice dependency has no palette reference")?;
    let palette = Palette::decode(&palette.package, palette.export_index)?;
    let generic = decode_generic_texture_animation(packages, resolved, &texture, &palette)?;
    let water = if let Some(wet) = &texture.wet {
        let source_object = packages
            .resolve(&resolved.package, wet.source_texture)?
            .context("ice water dependency has no source texture")?;
        let source_texture = Texture::decode(&source_object.package, source_object.export_index)?;
        let source = source_texture
            .mips
            .first()
            .context("ice water dependency source has no mip levels")?;
        wet.animate(
            mip.width,
            mip.height,
            source.width,
            source.height,
            &source.indices,
        )?
    } else {
        None
    };
    Ok(DecodedIceDependency {
        id: SceneObjectId {
            package: resolved.package.summary().source.to_string(),
            export_index: resolved.export_index,
        },
        width: mip.width,
        height: mip.height,
        pixels: water
            .as_ref()
            .map_or_else(|| mip.indices.clone(), |water| water.indices().to_vec()),
        palette,
        water,
        generic,
    })
}

struct DecodedTexture {
    id: SceneObjectId,
    texture: Texture,
    palette: Palette,
    water: Option<WaterAnimation>,
    ice: Option<DecodedIceTexture>,
    generic: Option<GenericTextureAnimation>,
}

struct DecodedIceTexture {
    animation: IceAnimation,
    source: DecodedIceDependency,
    glass: DecodedIceDependency,
}

struct DecodedIceDependency {
    id: SceneObjectId,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    palette: Palette,
    water: Option<WaterAnimation>,
    generic: Option<GenericTextureAnimation>,
}

impl DecodedTexture {
    fn image(&self, masked: bool) -> Result<TextureImage> {
        let mip = self
            .texture
            .mips
            .first()
            .context("texture has no mip levels")?;
        if let Some(water) = &self.water {
            return Ok(TextureImage {
                width: mip.width,
                height: mip.height,
                rgba: water.rgba(&self.palette, masked)?,
                mips: Vec::new(),
            });
        }
        if let Some(ice) = &self.ice {
            return Ok(TextureImage {
                width: mip.width,
                height: mip.height,
                rgba: ice.animation.rgba(&self.palette, masked)?,
                mips: Vec::new(),
            });
        }
        authored_texture_image(&self.texture, &self.palette, masked)
    }
}

fn authored_texture_image(
    texture: &Texture,
    palette: &Palette,
    masked: bool,
) -> Result<TextureImage> {
    let mip = texture.mips.first().context("texture has no mip levels")?;
    Ok(TextureImage {
        width: mip.width,
        height: mip.height,
        rgba: texture.rgba(0, palette, masked)?,
        mips: texture
            .mips
            .iter()
            .enumerate()
            .skip(1)
            .map(|(index, mip)| {
                Ok(crate::TextureMipImage {
                    width: mip.width,
                    height: mip.height,
                    rgba: texture.rgba(index, palette, masked)?,
                })
            })
            .collect::<Result<_>>()?,
    })
}

fn append_texture_image(
    textures: &mut Vec<TextureImage>,
    water_animations: &mut TextureAnimations,
    decoded: &DecodedTexture,
    masked: bool,
    follow_generic: bool,
) -> Result<usize> {
    let index = textures.len();
    textures.push(decoded.image(masked)?);
    let mip = decoded
        .texture
        .mips
        .first()
        .context("texture has no mip levels")?;
    let pixels = if let Some(water) = &decoded.water {
        water.indices().to_vec()
    } else if let Some(ice) = &decoded.ice {
        ice.animation.indices().to_vec()
    } else {
        mip.indices.clone()
    };
    water_animations
        .pixels
        .entry(decoded.id.clone())
        .or_insert(pixels);
    if let Some(animation) = &decoded.water {
        water_animations.water.push(AnimatedWaterTexture {
            id: decoded.id.clone(),
            texture: Some(index),
            masked,
            palette: decoded.palette.clone(),
            animation: animation.clone(),
        });
    }
    if let Some(ice) = &decoded.ice {
        register_ice_dependency(water_animations, &ice.source)?;
        register_ice_dependency(water_animations, &ice.glass)?;
        water_animations.ice.push(AnimatedIceTexture {
            id: decoded.id.clone(),
            texture: index,
            masked,
            palette: decoded.palette.clone(),
            animation: ice.animation.clone(),
            source: ice.source.id.clone(),
            glass: ice.glass.id.clone(),
            source_dimensions: [ice.source.width, ice.source.height],
            glass_dimensions: [ice.glass.width, ice.glass.height],
        });
    }
    if follow_generic && let Some(animation) = &decoded.generic {
        water_animations.generic.push(animated_generic_texture(
            decoded.id.clone(),
            Some(index),
            animation,
            masked,
            decoded.water.is_none() && decoded.ice.is_none(),
        )?);
    }
    Ok(index)
}

fn register_ice_dependency(
    animations: &mut TextureAnimations,
    dependency: &DecodedIceDependency,
) -> Result<()> {
    if animations.pixels.contains_key(&dependency.id) {
        return Ok(());
    }
    animations
        .pixels
        .insert(dependency.id.clone(), dependency.pixels.clone());
    if let Some(water) = &dependency.water {
        animations.water.push(AnimatedWaterTexture {
            id: dependency.id.clone(),
            texture: None,
            masked: false,
            palette: dependency.palette.clone(),
            animation: water.clone(),
        });
    }
    if let Some(generic) = &dependency.generic {
        animations.generic.push(animated_generic_texture(
            dependency.id.clone(),
            None,
            generic,
            false,
            dependency.water.is_none(),
        )?);
    }
    Ok(())
}

fn animated_generic_texture(
    id: SceneObjectId,
    texture: Option<usize>,
    animation: &GenericTextureAnimation,
    masked: bool,
    mipmapped: bool,
) -> Result<AnimatedGenericTexture> {
    let frames = animation
        .frames
        .iter()
        .map(|(texture, palette, _)| {
            let mut image = authored_texture_image(texture, palette, masked)?;
            if !mipmapped {
                image.mips.clear();
            }
            Ok(image)
        })
        .collect::<Result<Vec<_>>>()?;
    let index_frames = animation
        .frames
        .iter()
        .map(|(texture, _, _)| {
            texture
                .mips
                .first()
                .context("texture animation frame has no mip levels")
                .map(|mip| mip.indices.clone())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AnimatedGenericTexture {
        id,
        texture,
        frames,
        index_frames,
        next: animation.next.clone(),
        current: 0,
        accumulator: 0.0,
        prime_count: animation.prime_count,
        prime_current: 0,
        min_frame_rate: animation.min_frame_rate,
        max_frame_rate: animation.max_frame_rate,
    })
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
        macro_texture: None,
        detail_texture: None,
        fog_map_attached: false,
        portal: flags.contains(PolyFlags::PORTAL) || texture_flags.portal,
        macro_draw_scale: 1.0,
        detail_draw_scale: 1.0,
        bsp_texture_pan: [0.0; 2],
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
        no_smooth: flags.contains(PolyFlags::NO_SMOOTH) || texture_flags.no_smooth,
        unlit: flags.contains(PolyFlags::UNLIT),
        volumetric_source: false,
        mirror: flags.contains(PolyFlags::MIRRORED) || texture_flags.mirrored,
        environment_map: false,
        texture_draw_scale: 1.0,
        opacity: 1.0,
        small_wavy: false,
    }
}

fn bsp_surface_material(
    flags: PolyFlags,
    texture: Option<usize>,
    texture_flags: Option<TextureRenderFlags>,
) -> SurfaceMaterial {
    let texture_flags = texture_flags.unwrap_or_default();
    let mut material = surface_material(flags, texture, Some(texture_flags));
    if is_hidden(flags, texture_flags)
        && !flags.contains(PolyFlags::ALPHA_BLEND)
        && !flags.contains(PolyFlags::TRANSLUCENT)
        && !texture_flags.translucent
        && !flags.contains(PolyFlags::MODULATED)
        && !texture_flags.modulated
    {
        material.mode = SurfaceMode::DepthOnly;
    }
    material.volumetric_source = flags.contains(PolyFlags::FAKE_BACKDROP);
    material.small_wavy = flags.contains(PolyFlags::SMALL_WAVY);
    material
}

fn bsp_root_portal(
    flags: PolyFlags,
    raw_texture_present: bool,
    texture_flags: Option<TextureRenderFlags>,
) -> bool {
    flags.contains(PolyFlags::PORTAL)
        || (raw_texture_present && texture_flags.is_some_and(|flags| flags.portal))
}

fn is_hidden(flags: PolyFlags, texture_flags: TextureRenderFlags) -> bool {
    flags.contains(PolyFlags::INVISIBLE) || texture_flags.invisible
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use openhp1_map::{BspSurface, BspVertex, Model, PolyFlags, PrimitiveBounds};
    use openhp1_package::{ObjectReference, PackageStore};
    use openhp1_physics::BspCollision;
    use openhp1_runtime::{
        ActorAction, ParticleColor, ParticleEmitter, ParticleFloat, ParticleTexture, ParticleWind,
        RuntimeObject, ScriptRuntime, WeaponAttachment,
    };
    use openhp1_texture::{
        Color, IcePanningStyle, IceTexture, IceTimeMethod, MipLevel, Palette, Texture,
        TextureRenderFlags,
    };

    use crate::{SurfaceMaterial, SurfaceMode};

    static PARTICLE_TEST_ROOT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn bsp_default_texture_is_only_used_for_a_raw_null_reference() {
        assert_eq!(
            super::select_bsp_texture::<_, &str>(None, Some(7)),
            Ok(Some(7))
        );
        assert_eq!(
            super::select_bsp_texture::<_, &str>(Some(Ok(Some(3))), Some(7)),
            Ok(Some(3))
        );
        assert_eq!(
            super::select_bsp_texture::<_, &str>(Some(Ok(None)), Some(7)),
            Ok(None)
        );
        assert_eq!(
            super::select_bsp_texture(Some(Err("broken authored reference")), Some(7)),
            Err("broken authored reference")
        );
    }

    #[test]
    fn decoded_texture_preserves_exact_authored_mip_chain() {
        let dimensions = [(8, 8), (4, 4), (2, 2), (1, 1)];
        let decoded = super::DecodedTexture {
            id: crate::SceneObjectId {
                package: "mip-test".to_owned(),
                export_index: 0,
            },
            texture: Texture {
                palette: ObjectReference::None,
                anim_next: ObjectReference::None,
                detail_texture: ObjectReference::None,
                macro_texture: ObjectReference::None,
                prime_count: 0,
                min_frame_rate: 0.0,
                max_frame_rate: 0.0,
                draw_scale: 1.0,
                declared_width: Some(8),
                declared_height: Some(8),
                render_flags: TextureRenderFlags::default(),
                mips: dimensions
                    .iter()
                    .enumerate()
                    .map(|(index, &(width, height))| MipLevel {
                        width,
                        height,
                        width_bits: width.ilog2() as u8,
                        height_bits: height.ilog2() as u8,
                        indices: vec![index as u8; (width * height) as usize],
                    })
                    .collect(),
                wet: None,
                ice: None,
            },
            palette: Palette {
                colors: (0..4)
                    .map(|index| Color {
                        red: index,
                        green: index + 10,
                        blue: index + 20,
                        alpha: 0,
                    })
                    .collect(),
            },
            water: None,
            ice: None,
            generic: None,
        };

        let image = decoded.image(false).unwrap();
        assert_eq!(
            (image.width, image.height, image.rgba[..4].to_vec()),
            (8, 8, vec![0, 10, 20, 255])
        );
        assert_eq!(
            image
                .mips
                .iter()
                .map(|mip| (mip.width, mip.height, mip.rgba[..4].to_vec()))
                .collect::<Vec<_>>(),
            [
                (4, 4, vec![1, 11, 21, 255]),
                (2, 2, vec![2, 12, 22, 255]),
                (1, 1, vec![3, 13, 23, 255]),
            ]
        );
    }

    #[test]
    fn direct_attachment_lock_does_not_follow_its_own_anim_next() {
        let decoded = super::DecodedTexture {
            id: crate::SceneObjectId {
                package: "attachment-test".to_owned(),
                export_index: 0,
            },
            texture: Texture {
                palette: ObjectReference::None,
                anim_next: ObjectReference::Export(1),
                detail_texture: ObjectReference::None,
                macro_texture: ObjectReference::None,
                prime_count: 0,
                min_frame_rate: 0.0,
                max_frame_rate: 0.0,
                draw_scale: 1.0,
                declared_width: Some(1),
                declared_height: Some(1),
                render_flags: TextureRenderFlags::default(),
                mips: vec![MipLevel {
                    width: 1,
                    height: 1,
                    width_bits: 0,
                    height_bits: 0,
                    indices: vec![0],
                }],
                wet: None,
                ice: None,
            },
            palette: Palette {
                colors: vec![Color {
                    red: 1,
                    green: 2,
                    blue: 3,
                    alpha: 255,
                }],
            },
            water: None,
            ice: None,
            generic: Some(super::GenericTextureAnimation {
                frames: Vec::new(),
                next: Vec::new(),
                prime_count: 0,
                min_frame_rate: 0.0,
                max_frame_rate: 0.0,
            }),
        };
        let mut images = Vec::new();
        let mut animations = super::TextureAnimations::default();

        super::append_texture_image(&mut images, &mut animations, &decoded, false, false).unwrap();

        assert_eq!(images.len(), 1);
        assert!(animations.generic.is_empty());
    }

    #[test]
    fn generic_texture_with_zero_maximum_advances_once_per_tick() {
        let mut animation = generic_texture(vec![Some(1), None], 0, 0.0, 0.0);

        assert!(animation.tick(0.01));
        assert_eq!(animation.current, 1);
        assert!(animation.tick(0.01));
        assert_eq!(animation.current, 0);
    }

    #[test]
    fn generic_texture_zero_delta_does_not_prime_or_advance() {
        let mut animation = generic_texture(vec![Some(1), Some(0)], 1, 0.0, 0.0);

        assert!(!animation.tick(0.0));
        assert_eq!(animation.current, 0);
        assert_eq!(animation.prime_current, 0);
    }

    #[test]
    fn generic_texture_uses_fixed_and_ranged_frame_periods() {
        let mut fixed = generic_texture(vec![Some(1), Some(0)], 0, 10.0, 10.0);
        assert!(!fixed.tick(0.05));
        assert!(fixed.tick(0.05));
        assert_eq!(fixed.current, 1);

        let mut ranged = generic_texture(vec![Some(1), Some(0)], 0, 10.0, 20.0);
        assert!(ranged.tick(0.05));
        assert_eq!(ranged.current, 1);
        assert_eq!(ranged.accumulator, 0.0);
    }

    #[test]
    fn generic_texture_clamps_frame_rates() {
        let mut high = generic_texture(vec![Some(1), Some(0)], 0, 1000.0, 1000.0);
        assert!(high.tick(0.01));

        let mut low = generic_texture(vec![Some(1), Some(0)], 0, 0.01, 0.01);
        assert!(!low.tick(9.9));
        assert!(low.tick(0.1));
    }

    #[test]
    fn generic_texture_long_delta_advances_at_most_once() {
        let mut animation = generic_texture(vec![Some(1), Some(2), Some(0)], 0, 10.0, 20.0);

        assert!(animation.tick(1.0));
        assert_eq!(animation.current, 1);
        assert_eq!(animation.accumulator, 0.1);
    }

    #[test]
    fn generic_texture_null_falls_back_to_root_and_cycles_stay_stable() {
        let mut fallback = generic_texture(vec![Some(1), None], 0, 0.0, 0.0);
        fallback.tick(0.01);
        fallback.tick(0.01);
        assert_eq!(fallback.current, 0);

        let mut cycle = generic_texture(vec![Some(1), Some(0)], 0, 0.0, 0.0);
        cycle.tick(0.01);
        cycle.tick(0.01);
        assert_eq!(cycle.current, 0);
    }

    #[test]
    fn generic_texture_prime_count_advances_before_rate_tick() {
        let mut animation = generic_texture(vec![Some(1), Some(2), Some(0)], 2, 10.0, 10.0);

        assert!(animation.tick(0.01));
        assert_eq!(animation.current, 2);
        assert_eq!(animation.prime_current, 2);
    }

    #[test]
    fn scene_texture_tick_does_not_require_actor_animation() {
        let mut scene = particle_test_scene();
        scene.render.textures.push(crate::TextureImage {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            mips: Vec::new(),
        });
        scene
            .water_animations
            .generic
            .push(generic_texture(vec![Some(1), Some(0)], 0, 0.0, 0.0));

        assert!(scene.animations.is_empty());
        assert_eq!(scene.tick_textures(0.01).unwrap(), (vec![0], false));
        assert_eq!(scene.render.textures[0].rgba, [1; 4]);
    }

    #[test]
    fn base_anim_current_swaps_attachments_without_changing_root_portal() {
        let mut scene = particle_test_scene();
        scene.render.textures.push(crate::TextureImage {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            mips: Vec::new(),
        });
        scene.render.surface_materials.push(SurfaceMaterial {
            macro_texture: Some(1),
            macro_draw_scale: 1.0,
            portal: true,
            ..Default::default()
        });
        scene
            .water_animations
            .generic
            .push(generic_texture(vec![Some(1), Some(0)], 0, 0.0, 0.0));
        scene
            .water_animations
            .attachments
            .push(super::AnimatedSurfaceAttachments {
                surface: 0,
                animation: 0,
                frames: vec![
                    super::SurfaceAttachmentFrame {
                        macro_texture: Some(1),
                        detail_texture: None,
                        macro_draw_scale: 1.0,
                        detail_draw_scale: 1.0,
                    },
                    super::SurfaceAttachmentFrame {
                        macro_texture: Some(2),
                        detail_texture: None,
                        macro_draw_scale: 2.0,
                        detail_draw_scale: 4.0,
                    },
                ],
            });

        assert_eq!(scene.tick_textures(0.01).unwrap(), (vec![0], true));
        let material = scene.render.surface_materials[0];
        assert_eq!(material.macro_texture, Some(2));
        assert_eq!(material.detail_texture, None);
        assert_eq!(material.macro_draw_scale, 2.0);
        assert_eq!(material.detail_draw_scale, 4.0);
        assert!(material.portal);
    }

    #[test]
    fn generic_texture_tick_replaces_every_authored_mip() {
        let mut scene = particle_test_scene();
        scene.render.textures.push(crate::TextureImage {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            mips: vec![crate::TextureMipImage {
                width: 1,
                height: 1,
                rgba: vec![10; 4],
            }],
        });
        let mut animation = generic_texture(vec![Some(1), Some(0)], 0, 0.0, 0.0);
        animation.frames[0] = scene.render.textures[0].clone();
        animation.frames[1] = crate::TextureImage {
            width: 2,
            height: 2,
            rgba: vec![1; 16],
            mips: vec![crate::TextureMipImage {
                width: 1,
                height: 1,
                rgba: vec![11; 4],
            }],
        };
        scene.water_animations.generic.push(animation);

        assert_eq!(scene.tick_textures(0.01).unwrap(), (vec![0], false));
        assert_eq!(scene.render.textures[0].rgba, [1; 16]);
        assert_eq!(scene.render.textures[0].mips[0].rgba, [11; 4]);
    }

    #[test]
    fn scene_ice_tick_reports_only_changed_texture_indices() {
        let pixels = [0; 64];
        let animation = IceTexture {
            glass_texture: ObjectReference::None,
            source_texture: ObjectReference::None,
            panning_style: IcePanningStyle::Linear,
            time_method: IceTimeMethod::Realtime,
            horiz_pan_speed: 128,
            vert_pan_speed: 128,
            frequency: 0,
            amplitude: 0,
            move_ice: false,
            master_count: 0.0,
            u_displace: 0.0,
            v_displace: 0.0,
            u_position: 0.0,
            v_position: 0.0,
        }
        .animate(8, 8, 8, 8, &pixels, 8, 8, &pixels, 0.0, 0.0, 0)
        .unwrap();
        let mut scene = particle_test_scene();
        scene.render.textures.push(crate::TextureImage {
            width: 8,
            height: 8,
            rgba: vec![0; 8 * 8 * 4],
            mips: Vec::new(),
        });
        let source = crate::SceneObjectId {
            package: "test".to_owned(),
            export_index: 1,
        };
        let glass = crate::SceneObjectId {
            package: "test".to_owned(),
            export_index: 2,
        };
        scene
            .water_animations
            .pixels
            .insert(source.clone(), pixels.to_vec());
        scene
            .water_animations
            .pixels
            .insert(glass.clone(), pixels.to_vec());
        scene.water_animations.ice.push(super::AnimatedIceTexture {
            id: crate::SceneObjectId {
                package: "test".to_owned(),
                export_index: 0,
            },
            texture: 0,
            masked: false,
            palette: Palette {
                colors: vec![
                    Color {
                        red: 1,
                        green: 2,
                        blue: 3,
                        alpha: 0,
                    },
                    Color {
                        red: 9,
                        green: 8,
                        blue: 7,
                        alpha: 0,
                    },
                ],
            },
            animation,
            source: source.clone(),
            glass,
            source_dimensions: [8, 8],
            glass_dimensions: [8, 8],
        });

        assert!(scene.tick_textures(1.0 / 120.0).unwrap().0.is_empty());
        assert!(scene.tick_textures(1.0 / 120.0).unwrap().0.is_empty());

        let mut dependency = generic_texture(vec![Some(1), Some(1)], 0, 0.0, 0.0);
        dependency.id = source;
        dependency.texture = None;
        dependency.index_frames = vec![vec![0; 64], vec![1; 64]];
        scene.water_animations.generic.push(dependency);
        assert_eq!(scene.tick_textures(1.0 / 120.0).unwrap(), (vec![0], false));
        assert!(
            scene.render.textures[0]
                .rgba
                .chunks_exact(4)
                .all(|pixel| pixel == [9, 8, 7, 255])
        );
        assert!(scene.tick_textures(1.0 / 120.0).unwrap().0.is_empty());
    }

    #[test]
    fn runtime_set_location_action_updates_scene_actor() {
        let root = std::env::temp_dir().join(format!(
            "openhp1-scene-set-location-{}-{}",
            std::process::id(),
            PARTICLE_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        let system = root.join("System");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
        let mut runtime = ScriptRuntime::new(&root).unwrap();
        let mut scene = particle_test_scene();
        assert!(!scene.actor_states[0].is_light);

        assert_eq!(
            crate::apply_runtime_actions(
                &mut scene,
                &mut runtime,
                vec![ActorAction::SetLocation {
                    actor: 0,
                    location: [3.0, 4.0, 5.0],
                }],
            )
            .unwrap(),
            (0, 0, true)
        );
        assert_eq!(scene.actors[0].location, glam::Vec3::new(3.0, 4.0, 5.0));
        assert_eq!(
            scene.actor_states[0].actor.location,
            glam::Vec3::new(3.0, 4.0, 5.0)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_animation_action_is_an_authored_no_op() {
        let root = std::env::temp_dir().join(format!(
            "openhp1-scene-missing-animation-{}-{}",
            std::process::id(),
            PARTICLE_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        let system = root.join("System");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
        let mut runtime = ScriptRuntime::new(&root).unwrap();
        let mut scene = particle_test_scene();

        assert_eq!(
            crate::apply_runtime_actions(
                &mut scene,
                &mut runtime,
                vec![ActorAction::LoopAnimation {
                    actor: 0,
                    sequence: "Missing".to_owned(),
                    rate: 1.0,
                    tween_time: 0.0,
                    root_motion: false,
                }],
            )
            .unwrap(),
            (0, 0, false)
        );
        assert!(scene.actors[0].animation.is_none());
        assert!(scene.actors[0].diagnostics.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_rendered_skeletal_sequence_failure_is_diagnostic_case_insensitively() {
        let (root, mut runtime) = animation_test_runtime();
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let animation = Arc::new(synthetic_animation_package("Breathe"));
        let mesh_object = super::SceneObject {
            package: Arc::clone(&mesh),
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actor_states[0].actor.skeletal_animation = Some(super::SceneObject {
            package: animation,
            export_index: 0,
        });
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());

        assert_eq!(scene.actor_animation_sequences(0)[0].0, "Breathe");
        assert_eq!(
            crate::apply_runtime_actions(
                &mut scene,
                &mut runtime,
                vec![ActorAction::LoopAnimation {
                    actor: 0,
                    sequence: "breathE".to_owned(),
                    rate: 1.0,
                    tween_time: 0.0,
                    root_motion: false,
                }],
            )
            .unwrap(),
            (0, 0, false)
        );
        assert!(
            scene.actors[0]
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic == "runtime could not play animation breathE")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invisible_legacy_all_mesh_does_not_claim_missing_breathe() {
        let (root, mut runtime) = animation_test_runtime();
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actors[0].draw_type = 2;
        scene.actors[0].hidden = true;
        scene.actors[0].mesh = Some(mesh_object.id());

        assert_eq!(scene.actor_animation_sequences(0)[0].0, "All");
        crate::apply_runtime_actions(
            &mut scene,
            &mut runtime,
            vec![ActorAction::LoopAnimation {
                actor: 0,
                sequence: "Breathe".to_owned(),
                rate: 1.0,
                tween_time: 0.0,
                root_motion: false,
            }],
        )
        .unwrap();
        assert!(scene.actors[0].diagnostics.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn animation_metadata_decode_failure_is_recorded() {
        let mut scene = particle_test_scene();
        let invalid_mesh = super::SceneObject {
            package: Arc::clone(&scene.actor_render.map),
            export_index: 0,
        };
        scene.actor_states[0].actor.mesh = Some(invalid_mesh);

        assert!(scene.actor_animation_sequences(0).is_empty());
        assert!(scene.actors[0].diagnostics.iter().any(|diagnostic| {
            diagnostic.starts_with("animation metadata could not be decoded:")
        }));
    }

    #[test]
    fn rendered_invalid_skeletal_animation_keeps_its_metadata_diagnostic() {
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        let invalid_animation = super::SceneObject {
            package: Arc::clone(&scene.actor_render.map),
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actor_states[0].actor.skeletal_animation = Some(invalid_animation);
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());
        let state = scene.actor_states[0].actor.clone();
        let super::LoadedScene {
            actor_render,
            actors,
            render,
            animations,
            sprites,
            water_animations,
            ..
        } = &mut scene;
        super::append_scene_actor_render(
            actor_render,
            &mut actors[0],
            &state,
            false,
            0,
            &mut render.mesh,
            &mut render.textures,
            &mut render.surface_materials,
            animations,
            sprites,
            water_animations,
        );

        assert!(scene.actors[0].render.is_some());
        assert!(scene.actor_animation_sequences(0).is_empty());
        assert!(scene.actors[0].diagnostics.iter().any(|diagnostic| {
            diagnostic.starts_with("animation metadata could not be decoded:")
        }));
        assert!(!scene.animation_request_exposes_capability_gap(0, "All"));
    }

    #[test]
    fn untextured_actor_mesh_surfaces_are_hidden() {
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());
        let state = scene.actor_states[0].actor.clone();
        let super::LoadedScene {
            actor_render,
            actors,
            render,
            animations,
            sprites,
            water_animations,
            ..
        } = &mut scene;

        super::append_scene_actor_render(
            actor_render,
            &mut actors[0],
            &state,
            false,
            0,
            &mut render.mesh,
            &mut render.textures,
            &mut render.surface_materials,
            animations,
            sprites,
            water_animations,
        );

        assert_eq!(render.surface_materials[0].mode, SurfaceMode::Hidden);
    }

    #[test]
    fn display_rebuild_does_not_start_the_default_mesh_animation() {
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());
        let state = scene.actor_states[0].actor.clone();
        let super::LoadedScene {
            actor_render,
            actors,
            render,
            animations,
            sprites,
            water_animations,
            ..
        } = &mut scene;
        super::append_scene_actor_render(
            actor_render,
            &mut actors[0],
            &state,
            false,
            0,
            &mut render.mesh,
            &mut render.textures,
            &mut render.surface_materials,
            animations,
            sprites,
            water_animations,
        );
        assert!(scene.actors[0].animation.is_none());
        assert_eq!(scene.animations.len(), 1);

        scene.set_actor_physics(0, 2).unwrap();

        assert!(scene.actors[0].animation.is_none());
    }

    #[test]
    fn weapon_attachment_is_removed_when_its_owner_is_hidden() {
        let mut scene = particle_test_scene();
        scene.actors[0].hidden = true;
        scene.actors[0].render = Some(crate::SceneActorRenderRange {
            vertices: 3..4,
            indices: 0..0,
        });
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: Arc::clone(&mesh),
            export_index: 0,
        };
        let mut weapon = scene.actors[0].clone();
        weapon.name = "baseWand".to_owned();
        weapon.draw_type = 2;
        weapon.render = Some(crate::SceneActorRenderRange {
            vertices: 0..3,
            indices: 0..0,
        });
        scene.actors.push(weapon);
        scene.actor_states.push(super::ActorRenderState::default());
        scene.attached_weapons.insert(1, mesh_object);

        assert!(
            scene
                .sync_weapon_attachments(vec![WeaponAttachment {
                    pawn: 0,
                    weapon: 1,
                    mesh: RuntimeObject {
                        package: Arc::clone(&mesh.summary().source),
                        export_index: 0,
                    },
                    scale: 1.0,
                }])
                .unwrap()
        );
        assert!(scene.attached_weapons.is_empty());
        assert!(scene.actors[1].render.is_none());
    }

    #[test]
    fn anim_frame_action_seeks_the_new_sequence_without_tweening() {
        let (root, mut runtime) = animation_test_runtime();
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("Retract"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());
        scene.rebuild_current_actor_render(0).unwrap();

        crate::apply_runtime_actions(
            &mut scene,
            &mut runtime,
            vec![
                ActorAction::PlayAnimation {
                    actor: 0,
                    sequence: "Retract".to_owned(),
                    rate: 1.0,
                    tween_time: 0.2,
                    root_motion: false,
                },
                ActorAction::SetAnimationFrame {
                    actor: 0,
                    frame: 0.625,
                },
            ],
        )
        .unwrap();

        assert_eq!(scene.actors[0].animation.as_ref().unwrap().phase, 0.625);
        assert_eq!(scene.animations[0].phase, 0.625);
        assert!(scene.animations[0].tween_from.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_style_and_opacity_transitions_refresh_actor_submission_pass() {
        let mut scene = particle_test_scene();
        let mesh = Arc::new(synthetic_mesh_package("All"));
        let mesh_object = super::SceneObject {
            package: mesh,
            export_index: 0,
        };
        scene.actor_states[0].actor.draw_type = 2;
        scene.actor_states[0].actor.mesh = Some(mesh_object.clone());
        scene.actors[0].draw_type = 2;
        scene.actors[0].mesh = Some(mesh_object.id());
        assert!(scene.rebuild_current_actor_render(0).unwrap());
        assert!(!scene.render.actor_submissions[0].translucent_pass);

        assert!(scene.set_actor_style(0, 3).unwrap());
        assert!(scene.render.actor_submissions[0].translucent_pass);
        assert!(scene.set_actor_style(0, 1).unwrap());
        assert!(!scene.render.actor_submissions[0].translucent_pass);

        assert!(scene.set_actor_opacity(0, 0.5).unwrap());
        assert!(scene.render.actor_submissions[0].translucent_pass);
        assert!(scene.set_actor_opacity(0, 1.0).unwrap());
        assert!(!scene.render.actor_submissions[0].translucent_pass);
    }

    #[test]
    fn particle_capacity_uses_alive_limit_and_finite_emission_count() {
        let mut emitter = ParticleEmitter {
            actor: 0,
            owner: None,
            emit: true,
            prime: false,
            distribution: 0,
            style: 3,
            unlit: true,
            particles_alive: 7,
            particles_max: 7,
            particles_emitted: 0,
            particles_per_second: ParticleFloat {
                base: 10.0,
                random: 5.0,
            },
            parent_particles_per_second: None,
            period: ParticleFloat::default(),
            lifetime: ParticleFloat {
                base: 1.0,
                random: 1.0,
            },
            speed: ParticleFloat::default(),
            angular_spread_width: ParticleFloat::default(),
            angular_spread_height: ParticleFloat::default(),
            source_width: ParticleFloat::default(),
            source_height: ParticleFloat::default(),
            source_depth: ParticleFloat::default(),
            size_width: ParticleFloat::default(),
            size_length: ParticleFloat::default(),
            size_end_scale: ParticleFloat::default(),
            color_start: ParticleColor {
                base: [255; 4],
                random: [0; 4],
            },
            color_end: ParticleColor {
                base: [255; 4],
                random: [0; 4],
            },
            alpha_start: ParticleFloat::default(),
            alpha_end: ParticleFloat::default(),
            alpha_delay: 0.0,
            alpha_grow_period: 0.0,
            color_delay: 0.0,
            size_delay: 0.0,
            size_grow_period: 0.0,
            draw_scale: 1.0,
            system_relative: false,
            wind_per_particle: false,
            damping: 0.0,
            gravity: [0.0; 3],
            wind: [0.0; 3],
            winds: Vec::new(),
            render_primitive: 1,
            velocity_relative: false,
            owner_velocity: [0.0; 3],
            gravity_modifier: 0.0,
            chaos: 0.0,
            chaos_delay: 0.0,
            attraction: [0.0; 3],
            elasticity: 0.0,
            wind_modifier: 0.0,
            spin_rate: ParticleFloat::default(),
            drip_time: ParticleFloat::default(),
            parent_blend: 0.0,
            color_palette: false,
            pattern: Vec::new(),
            textures: Vec::new(),
        };
        assert_eq!(super::particle_capacity(&emitter), 7);
        emitter.particles_alive = 0;
        emitter.particles_max = 0;
        assert_eq!(super::particle_capacity(&emitter), 30);
        emitter.distribution = 1;
        emitter.particles_per_second = ParticleFloat {
            base: 2.0,
            random: 0.0,
        };
        emitter.lifetime.base = 100_000_000.0;
        emitter.draw_scale = 10.0;
        emitter.pattern = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert_eq!(super::particle_capacity(&emitter), 10);
    }

    #[test]
    fn parent_blended_emission_rate_samples_child_and_parent_independently() {
        let emitter = ParticleEmitter {
            particles_per_second: ParticleFloat {
                base: 10.0,
                random: 4.0,
            },
            parent_particles_per_second: Some(ParticleFloat {
                base: 30.0,
                random: 8.0,
            }),
            parent_blend: 0.25,
            ..Default::default()
        };
        let mut expected_random = 0x1234_5678;
        let child =
            super::sample_particle_float(emitter.particles_per_second, &mut expected_random);
        let parent = super::sample_particle_float(
            emitter.parent_particles_per_second.unwrap(),
            &mut expected_random,
        );
        let mut actual_random = 0x1234_5678;

        assert_eq!(
            super::sample_particle_emission_rate(&emitter, &mut actual_random),
            child + (parent - child) * 0.25
        );
        assert_eq!(actual_random, expected_random);
    }

    #[test]
    fn parent_blended_emission_rate_uses_the_raw_nonzero_blend() {
        let mut emitter = ParticleEmitter {
            particles_per_second: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            parent_particles_per_second: Some(ParticleFloat {
                base: 30.0,
                random: 0.0,
            }),
            parent_blend: 1.5,
            ..Default::default()
        };
        let mut random = 0x1234_5678;
        assert_eq!(
            super::sample_particle_emission_rate(&emitter, &mut random),
            40.0
        );

        emitter.parent_blend = -0.5;
        assert_eq!(
            super::sample_particle_emission_rate(&emitter, &mut random),
            0.0
        );
    }

    #[test]
    fn particle_patterns_interpolate_authored_gesture_points() {
        let points = [[0.0, 0.0, 0.0], [0.25, 1.0, 0.0], [1.0, 1.0, 0.0]];
        assert_eq!(
            super::pattern_position(&points, 0.25),
            Some(glam::Vec3::new(0.125, 0.5, 0.0))
        );
        assert_eq!(
            super::pattern_position(&points, 0.75),
            Some(glam::Vec3::new(0.625, 1.0, 0.0))
        );
        let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
        let period = ParticleFloat {
            base: 0.0,
            random: 1.0,
        };
        assert_eq!(
            super::uniform_particle_distance(&points, period, 1.0, 0.0, &mut 0),
            2.0
        );
        assert_eq!(
            super::uniform_particle_distance(&points, period, 1.0, 0.0, &mut 0x8000_0000,),
            6.0
        );
    }

    #[test]
    fn zero_lifetime_particles_live_until_the_emitter_is_removed() {
        assert!(super::particle_is_alive(10_000.0, 0.0));
        assert!(super::particle_is_alive(0.5, 1.0));
        assert!(!super::particle_is_alive(1.0, 1.0));
    }

    #[test]
    fn unlimited_emitter_grows_before_a_burst_without_recycling() {
        let mut scene = particle_test_scene();
        let system = scene.particles.get_mut(&0).unwrap();
        system.config = ParticleEmitter {
            emit: true,
            particles_alive: 0,
            particles_per_second: ParticleFloat {
                base: 2.0,
                random: 0.0,
            },
            parent_particles_per_second: None,
            lifetime: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            render_primitive: 1,
            ..Default::default()
        };
        system.particles.clear();

        assert!(scene.tick_particles(1.0));
        let system = &scene.particles[&0];
        assert_eq!(system.capacity, 2);
        assert_eq!(system.particles.len(), 2);
    }

    #[test]
    fn particle_spawn_placement_uses_native_distribution_and_local_source_box() {
        let emitter = |distribution, rate| ParticleEmitter {
            emit: true,
            distribution,
            particles_alive: 0,
            particles_per_second: ParticleFloat {
                base: rate,
                random: 0.0,
            },
            lifetime: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            render_primitive: 1,
            ..Default::default()
        };

        let mut random = particle_test_scene();
        random.actors[0].location = glam::Vec3::new(10.0, 0.0, 0.0);
        random.particles.get_mut(&0).unwrap().config = emitter(0, 2.0);
        random.particles.get_mut(&0).unwrap().particles.clear();
        assert!(random.tick_particles(1.0));
        let positions = random.particles[&0]
            .particles
            .iter()
            .map(|particle| particle.location.x)
            .collect::<Vec<_>>();
        assert_ne!(positions, [2.5, 7.5]);

        let mut uniform = particle_test_scene();
        uniform.actors[0].location = glam::Vec3::new(10.0, 0.0, 0.0);
        uniform.particles.get_mut(&0).unwrap().config = emitter(1, 2.0);
        uniform.particles.get_mut(&0).unwrap().particles.clear();
        assert!(uniform.tick_particles(1.0));
        assert_eq!(
            uniform.particles[&0]
                .particles
                .iter()
                .map(|particle| particle.location.x)
                .collect::<Vec<_>>(),
            [2.0, 4.0, 6.0, 8.0, 10.0]
        );

        let mut rotated = particle_test_scene();
        rotated.actors[0].rotation = super::Rotator {
            yaw: 16_384,
            ..Default::default()
        };
        rotated.particles.get_mut(&0).unwrap().config = ParticleEmitter {
            source_width: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            ..emitter(0, 1.0)
        };
        rotated.particles.get_mut(&0).unwrap().particles.clear();
        assert!(rotated.tick_particles(1.0));
        let position = rotated.particles[&0].particles[0].location;
        assert!(position.x.abs() > 0.01);
        assert!(position.y.abs() < 1.0e-5);
    }

    #[test]
    fn particle_damping_is_exponential_over_elapsed_time() {
        assert!((super::particle_damping(1.0, 1.0) - std::f32::consts::E.recip()).abs() < 1e-6);
        assert_eq!(super::particle_damping(0.0, 10.0), 1.0);
    }

    #[test]
    fn particle_alpha_follows_native_grow_hold_and_fade_phases() {
        let mut scene = particle_test_scene();
        let system = scene.particles.get_mut(&0).unwrap();
        system.config.alpha_delay = 2.0;
        system.config.alpha_grow_period = 0.25;
        system.particles[0].lifetime = 4.0;
        system.particles[0].alpha_start = 1.0;
        system.particles[0].alpha_end = 0.0;

        for (delta_time, expected) in [(0.5, 0.5), (0.5, 1.0), (1.0, 1.0), (1.0, 0.5)] {
            assert!(scene.tick_particles(delta_time));
            assert!(
                scene.render.mesh.vertex_colors[0].abs_diff_eq(glam::Vec3::splat(expected), 1.0e-6)
            );
        }
        assert_eq!(super::particle_alpha(100.0, 0.0, 0.8, 0.0, 0.0, 0.5), 0.8);
        assert_eq!(super::particle_alpha(3.999, 4.0, 1.0, 0.0, 2.0, 0.0), 0.0);
    }

    #[test]
    fn modulated_particles_use_the_d3d_white_vertex_override() {
        let color = glam::Vec3::new(0.2, 0.4, 0.8);

        assert_eq!(
            super::particle_vertex_color(4, color, 0.25),
            glam::Vec3::ONE
        );
    }

    #[test]
    fn wind_per_particle_selects_particle_location_sampling() {
        let mut scene = particle_test_scene();
        let system = scene.particles.get_mut(&0).unwrap();
        system.config.system_relative = true;
        system.config.damping = 2.0;
        system.config.wind_modifier = 1.0;
        let wind = ParticleWind {
            location: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
            speed: 20.0,
            radius: 10,
            permeating: true,
            ..Default::default()
        };
        assert_eq!(
            ParticleWind::total_at(&[wind], None, glam::Vec3::new(10.1, 0.0, 0.0)),
            glam::Vec3::ZERO
        );
        system.config.winds = vec![wind];
        system.particles[0].location = glam::Vec3::new(5.0, 0.0, 0.0);
        system.particles[0].velocity = glam::Vec3::ZERO;

        assert!(scene.tick_particles(1.0));
        assert_eq!(scene.particles[&0].particles[0].velocity, glam::Vec3::ZERO);

        let system = scene.particles.get_mut(&0).unwrap();
        system.config.wind_per_particle = true;
        system.particles[0].age = 0.0;
        system.particles[0].location = glam::Vec3::new(5.0, 0.0, 0.0);
        assert!(scene.tick_particles(1.0));
        let particle = &scene.particles[&0].particles[0];
        let decay = (-2.0_f32).exp();
        assert!((particle.velocity.x - 15.0 * (1.0 - decay)).abs() < 1.0e-5);
        assert!((particle.location.x - (5.0 + 15.0 * (1.0 + decay) / 2.0)).abs() < 1.0e-5);
    }

    #[test]
    fn particle_attraction_accelerates_each_authored_axis_toward_the_emitter() {
        assert_eq!(
            super::particle_attraction(
                glam::Vec3::new(8.0, 3.0, 4.0),
                glam::Vec3::new(10.0, 2.0, 9.0),
                false,
                [2.0, 3.0, 0.0],
            ),
            glam::Vec3::new(4.0, -3.0, 0.0)
        );
    }

    #[test]
    fn tick_particles_bounces_elastic_particles_from_world_bsp_once() {
        let mut scene = particle_test_scene();

        assert!(scene.tick_particles(1.0));
        let particle = &scene.particles[&0].particles[0];
        assert!((-2.0..0.0).contains(&particle.location.x));
        assert!(
            particle
                .velocity
                .abs_diff_eq(glam::Vec3::new(-5.0, 2.0, 0.0), 1.0e-6)
        );
        let location = particle.location;

        assert!(scene.tick_particles(0.1));
        let particle = &scene.particles[&0].particles[0];
        assert!(particle.location.x < location.x);
        assert!(
            particle
                .velocity
                .abs_diff_eq(glam::Vec3::new(-5.0, 2.0, 0.0), 1.0e-6)
        );
    }

    #[test]
    fn finite_particle_limit_recycles_the_oldest_particle() {
        let mut scene = particle_test_scene();
        let system = scene.particles.get_mut(&0).unwrap();
        system.config = ParticleEmitter {
            emit: true,
            particles_alive: 1,
            particles_per_second: ParticleFloat {
                base: 1.0,
                random: 0.0,
            },
            lifetime: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            render_primitive: 1,
            velocity_relative: true,
            owner_velocity: [3.0, -4.0, 5.0],
            ..Default::default()
        };
        system.particles.clear();

        assert!(scene.tick_particles(1.0));
        let particle = &scene.particles[&0].particles[0];
        assert_eq!(particle.velocity, glam::Vec3::new(3.0, -4.0, 5.0));

        scene.particles.get_mut(&0).unwrap().config.owner_velocity = [6.0, -8.0, 10.0];
        assert!(scene.tick_particles(1.0));
        let particle = &scene.particles[&0].particles[0];
        assert_eq!(particle.velocity, glam::Vec3::new(6.0, -8.0, 10.0));
        assert_eq!(particle.location, glam::Vec3::ZERO);
    }

    #[test]
    fn dynamic_particle_submission_tracks_growth_pass_and_removal() {
        let mut mesh = openhp1_map::TriangleMesh::default();
        let index_start = mesh.indices.len();
        let vertices = super::append_particle_slots(&mut mesh, 1, 0, [glam::Vec2::ZERO; 4]);
        let mut system = super::ParticleSystem {
            config: ParticleEmitter::default(),
            particles: Vec::new(),
            capacity: 1,
            vertices,
            indices: index_start..mesh.indices.len(),
            residue: 0.0,
            last_location: glam::Vec3::ZERO,
            random: 0,
            primed: false,
            emitted: 0,
        };
        let mut submissions = Vec::new();
        super::upsert_particle_submission(&mut submissions, 7, system.indices.clone(), false);
        assert_eq!(submissions[0].indices, 0..6);
        assert!(!submissions[0].translucent_pass);

        super::grow_particle_system(&mut mesh, &mut system, 2);
        super::upsert_particle_submission(&mut submissions, 7, system.indices.clone(), true);
        assert_eq!(submissions[0].indices, 6..18);
        assert!(submissions[0].translucent_pass);

        super::remove_particle_submission(&mut submissions, 7);
        assert!(submissions.is_empty());
    }

    #[test]
    fn particle_emitter_sync_refreshes_submission_pass_and_removes_it() {
        let mut scene = particle_test_scene();
        let emitter = |style| ParticleEmitter {
            actor: 0,
            style,
            textures: vec![ParticleTexture {
                package: "unused-existing-system".to_owned(),
                export_index: 0,
            }],
            ..Default::default()
        };

        assert!(
            !scene
                .render
                .actor_submissions
                .iter()
                .any(|s| s.actor_index == 0)
        );
        assert!(scene.sync_particle_emitters(vec![emitter(1)]).unwrap());
        assert!(!scene.render.actor_submissions[0].translucent_pass);

        scene.actor_states[0].actor.opacity = 0.5;
        assert!(scene.sync_particle_emitters(vec![emitter(1)]).unwrap());
        assert!(scene.render.actor_submissions[0].translucent_pass);

        scene.actor_states[0].actor.opacity = 1.0;
        assert!(scene.sync_particle_emitters(vec![emitter(1)]).unwrap());
        assert!(!scene.render.actor_submissions[0].translucent_pass);
        assert!(scene.sync_particle_emitters(vec![emitter(3)]).unwrap());
        assert!(scene.render.actor_submissions[0].translucent_pass);

        assert!(scene.sync_particle_emitters(Vec::new()).unwrap());
        assert!(scene.render.actor_submissions.is_empty());
    }

    #[test]
    fn inactive_particle_teardown_preserves_rebuilt_actor_submission() {
        let mut scene = particle_test_scene();
        scene.render.actor_submissions.push(crate::ActorSubmission {
            actor_index: 0,
            indices: 12..18,
            translucent_pass: false,
        });

        assert!(scene.sync_particle_emitters(Vec::new()).unwrap());
        assert_eq!(scene.render.actor_submissions.len(), 1);
        assert_eq!(scene.render.actor_submissions[0].indices, 12..18);
    }

    fn particle_test_scene() -> super::LoadedScene {
        let root = std::env::temp_dir().join(format!(
            "openhp1-scene-particle-elasticity-{}-{}",
            std::process::id(),
            PARTICLE_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        let system = root.join("System");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
        let package_path = system.join("Test.u");
        fs::write(&package_path, synthetic_level_package()).unwrap();
        let mut packages = PackageStore::scan_game_root(&root).unwrap();
        let map = packages.load_path(&package_path).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let model = particle_collision_model();
        let collision = Arc::new(BspCollision::from_model(&model).unwrap());
        let vertex_lighting = model.vertex_lighting(&map).unwrap();
        super::LoadedScene {
            path: PathBuf::from("particle-test.unr"),
            levels: Vec::new(),
            render: crate::RenderScene {
                mesh: openhp1_map::TriangleMesh {
                    positions: vec![glam::Vec3::ZERO; 4],
                    texture_coordinates: vec![glam::Vec2::ZERO; 4],
                    vertex_colors: vec![glam::Vec3::ONE; 4],
                    vertex_surfaces: vec![0; 4],
                    ..Default::default()
                },
                textures: Vec::new(),
                lightmaps: Vec::new(),
                realtime_lightmaps: Vec::new(),
                coronas: Vec::new(),
                actor_submissions: Vec::new(),
                surface_materials: Vec::new(),
                warp_portals: Vec::new(),
                sky_zone: None,
            },
            points: 0,
            nodes: 0,
            surfaces: 0,
            visible_bsp_surfaces: 0,
            textured_surfaces: 0,
            masked_surfaces: 0,
            translucent_surfaces: 0,
            modulated_surfaces: 0,
            fake_backdrop_surfaces: 0,
            has_sky_zone: false,
            actor_meshes: 0,
            animated_actor_meshes: 0,
            actors: vec![crate::SceneActor {
                id: crate::SceneObjectId {
                    package: "<test>".to_owned(),
                    export_index: 0,
                },
                name: "ParticleTest".to_owned(),
                class: None,
                class_name: "ParticleFX".to_owned(),
                location: glam::Vec3::ZERO,
                rotation: crate::Rotator::default(),
                pre_pivot: glam::Vec3::ZERO,
                main_scale: glam::Vec3::ONE,
                draw_scale: 1.0,
                draw_type: 8,
                hidden: false,
                unlit: false,
                brush: None,
                mesh: None,
                mesh_name: None,
                animation: None,
                render: None,
                mesh_transform: None,
                mesh_to_object: None,
                visual_bounds: None,
                diagnostics: Vec::new(),
            }],
            actor_states: vec![super::ActorRenderState::default()],
            collision,
            zone_nodes: Vec::new(),
            zone_count: 0,
            animations: Vec::new(),
            sprites: Vec::new(),
            root_motions: Vec::new(),
            hidden_actor_positions: Default::default(),
            attached_weapons: Default::default(),
            water_animations: super::TextureAnimations::default(),
            changed_lightmaps: Vec::new(),
            particles: [(
                0,
                super::ParticleSystem {
                    config: ParticleEmitter {
                        elasticity: 0.5,
                        ..Default::default()
                    },
                    particles: vec![super::Particle {
                        location: glam::Vec3::new(-10.0, 0.0, 0.0),
                        velocity: glam::Vec3::new(10.0, 2.0, 0.0),
                        age: 0.0,
                        lifetime: 5.0,
                        half_size: glam::Vec2::ZERO,
                        end_scale: 1.0,
                        color_start: glam::Vec3::ONE,
                        color_end: glam::Vec3::ONE,
                        alpha_start: 1.0,
                        alpha_end: 0.0,
                        spin: 0.0,
                        spin_rate: 0.0,
                        chaos_timer: 0.0,
                        drip_time: 0.0,
                    }],
                    capacity: 1,
                    vertices: 0..4,
                    indices: 0..6,
                    residue: 0.0,
                    last_location: glam::Vec3::ZERO,
                    random: 0,
                    primed: false,
                    emitted: 0,
                },
            )]
            .into_iter()
            .collect(),
            particle_view_rotation: glam::Mat4::IDENTITY,
            actor_render: super::ActorRenderContext {
                packages,
                map,
                model,
                level_environment_map: None,
                zone_environment_maps: Vec::new(),
                vertex_lighting,
                light_brightnesses: Default::default(),
                class_cache: Default::default(),
                mesh_cache: Default::default(),
                brush_cache: Default::default(),
                animation_cache: Default::default(),
                decoded_textures: Default::default(),
                images: Default::default(),
            },
        }
    }

    fn generic_texture(
        next: Vec<Option<usize>>,
        prime_count: u8,
        min_frame_rate: f32,
        max_frame_rate: f32,
    ) -> super::AnimatedGenericTexture {
        super::AnimatedGenericTexture {
            id: crate::SceneObjectId {
                package: "generic-test".to_owned(),
                export_index: 0,
            },
            texture: Some(0),
            frames: (0..next.len())
                .map(|index| crate::TextureImage {
                    width: 1,
                    height: 1,
                    rgba: vec![index as u8; 4],
                    mips: Vec::new(),
                })
                .collect(),
            index_frames: (0..next.len()).map(|index| vec![index as u8; 1]).collect(),
            next,
            current: 0,
            accumulator: 0.0,
            prime_count,
            prime_current: 0,
            min_frame_rate,
            max_frame_rate,
        }
    }

    fn animation_test_runtime() -> (PathBuf, ScriptRuntime) {
        let root = std::env::temp_dir().join(format!(
            "openhp1-scene-animation-action-{}-{}",
            std::process::id(),
            PARTICLE_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        let system = root.join("System");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("Default.ini"), "[Core.System]\nPaths=*.u\n").unwrap();
        let runtime = ScriptRuntime::new(&root).unwrap();
        (root, runtime)
    }

    fn synthetic_mesh_package(sequence: &str) -> openhp1_package::Package {
        let names = [
            "None", "Core", "Class", "Mesh", "TestMesh", sequence, "Movement", "Step",
        ];
        let mut payload = vec![0];
        for value in [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0] {
            push_f32(&mut payload, value);
        }
        payload.push(1);
        payload.extend([0; 12]);
        payload.push(6);
        for (x, y, z) in [
            (0, 0, 0),
            (1, 0, 0),
            (0, 1, 0),
            (0, 0, 1),
            (1, 0, 1),
            (0, 1, 1),
        ] {
            push_i32(
                &mut payload,
                ((z & 0x3ff) << 22) | ((y & 0x7ff) << 11) | (x & 0x7ff),
            );
        }
        payload.push(1);
        for index in [0_u16, 1, 2] {
            payload.extend(index.to_le_bytes());
        }
        payload.extend([0; 6]);
        push_u32(&mut payload, 0);
        push_i32(&mut payload, 0);
        payload.extend([1, 5, 6]);
        push_i32(&mut payload, 0);
        push_i32(&mut payload, 2);
        payload.push(1);
        push_f32(&mut payload, 0.25);
        payload.push(7);
        push_f32(&mut payload, 10.0);
        payload.push(0);
        payload.extend([0; 25 + 12]);
        payload.extend([0; 4]);
        push_i32(&mut payload, 3);
        push_i32(&mut payload, 2);
        payload.extend([0; 8]);
        for value in [1.0, 1.0, 1.0, 0.0, 0.0, 0.0] {
            push_f32(&mut payload, value);
        }
        payload.extend([0; 20]);
        synthetic_object_package("synthetic mesh", &names, 3, 4, payload)
    }

    fn synthetic_animation_package(sequence: &str) -> openhp1_package::Package {
        let names = [
            "None",
            "Core",
            "Class",
            "Animation",
            "TestAnimation",
            "Root",
            sequence,
            "Movement",
            "Notify",
        ];
        let mut payload = vec![0, 1, 5];
        push_u32(&mut payload, 0);
        push_u32(&mut payload, 0);
        payload.push(1);
        for value in [0.0, 0.0, 0.0, 1.0] {
            push_f32(&mut payload, value);
        }
        push_u32(&mut payload, 0);
        push_u32(&mut payload, 0);
        payload.push(1);
        push_u32(&mut payload, 0);
        payload.push(1);
        push_u32(&mut payload, 0);
        payload.extend([2, 1, 2]);
        push_f32(&mut payload, 2.0);
        push_f32(&mut payload, 0.5);
        payload.extend([1, 6, 7]);
        push_i32(&mut payload, 0);
        push_i32(&mut payload, 2);
        payload.push(1);
        push_f32(&mut payload, 0.25);
        payload.push(8);
        push_f32(&mut payload, 2.0);
        payload.push(2);
        for value in [0_i16, 0, 0, 0, 0, 16_384] {
            payload.extend(value.to_le_bytes());
        }
        payload.push(1);
        for value in [16_384_i16, 0, 0] {
            payload.extend(value.to_le_bytes());
        }
        payload.extend([2, 0, 2]);
        synthetic_object_package("synthetic animation", &names, 3, 4, payload)
    }

    fn synthetic_object_package(
        source: &str,
        names: &[&str],
        class_name: usize,
        object_name: usize,
        payload: Vec<u8>,
    ) -> openhp1_package::Package {
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            push_u32(&mut name_table, 0);
        }
        let mut import_table = vec![1, 2];
        push_i32(&mut import_table, 0);
        import_table.extend(compact_signed_index(class_name as i32));
        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        push_i32(&mut export, 0);
        export.extend(compact_signed_index(object_name as i32));
        push_u32(&mut export, 0);
        export.extend(compact_signed_index(payload.len() as i32));
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let encoded = compact_signed_index(payload_offset as i32);
            let next = export_offset + export.len() + encoded.len();
            if next == payload_offset {
                export.extend(encoded);
                break;
            }
            payload_offset = next;
        }
        let mut bytes = Vec::new();
        push_u32(&mut bytes, openhp1_package::PACKAGE_MAGIC);
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        push_u32(&mut bytes, 0);
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            push_i32(&mut bytes, value as i32);
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        assert_eq!(bytes.len(), payload_offset);
        bytes.extend(payload);
        openhp1_package::Package::parse(source, Arc::from(bytes)).unwrap()
    }

    fn compact_signed_index(value: i32) -> Vec<u8> {
        let negative = value < 0;
        let mut value = value.unsigned_abs();
        let mut bytes = vec![(value as u8 & 0x3f) | if negative { 0x80 } else { 0 }];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = value as u8 & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend(value.to_le_bytes());
    }

    fn particle_collision_model() -> Model {
        let mut model = Model {
            bounds: PrimitiveBounds {
                minimum: glam::Vec3::ZERO,
                maximum: glam::Vec3::ZERO,
                valid: false,
                sphere: [0.0; 4],
            },
            vectors: Vec::new(),
            points: vec![
                glam::Vec3::new(0.0, -10.0, -10.0),
                glam::Vec3::new(0.0, 10.0, -10.0),
                glam::Vec3::new(0.0, 10.0, 10.0),
                glam::Vec3::new(0.0, -10.0, 10.0),
            ],
            nodes: Vec::new(),
            surfaces: vec![BspSurface {
                texture: ObjectReference::None,
                poly_flags: PolyFlags::default(),
                base_point: 0,
                normal: 0,
                texture_u: 0,
                texture_v: 0,
                light_map: -1,
                brush_poly: -1,
                pan_u: 0,
                pan_v: 0,
                brush_actor: ObjectReference::None,
            }],
            vertices: (0..4).map(|point| BspVertex { point, side: -1 }).collect(),
            shared_side_count: 0,
            zones: Vec::new(),
            polys: ObjectReference::None,
            light_maps: Vec::new(),
            light_bits: Vec::new(),
            collision_bounds: Vec::new(),
            leaf_hulls: Vec::new(),
            leaves: Vec::new(),
            lights: Vec::new(),
            root_outside: true,
            linked: false,
        };
        model.nodes.push(openhp1_map::BspNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            zone_mask: 0,
            flags: 0,
            vertex_pool: 0,
            surface: 0,
            back: -1,
            front: -1,
            coplanar: -1,
            collision_bound: -1,
            render_bound: -1,
            zones: [0; 2],
            vertex_count: 4,
            leaves: [0; 2],
        });
        model
    }

    fn synthetic_level_package() -> Vec<u8> {
        let names = ["None", "Package", "Class", "Level", "Test"];
        let mut name_table = Vec::new();
        for name in names {
            name_table.extend(name.as_bytes());
            name_table.push(0);
            name_table.extend(0_u32.to_le_bytes());
        }
        let mut payload = vec![0];
        payload.extend(0_i32.to_le_bytes());
        payload.extend(0_i32.to_le_bytes());
        payload.extend([0; 4]);
        payload.push(0);
        payload.extend(0_i32.to_le_bytes());
        payload.extend(0_u32.to_le_bytes());
        payload.extend([0; 2]);

        const HEADER_SIZE: usize = 44;
        let name_offset = HEADER_SIZE;
        let import_offset = name_offset + name_table.len();
        let mut import_table = vec![1, 2];
        import_table.extend(0_i32.to_le_bytes());
        import_table.push(3);
        let export_offset = import_offset + import_table.len();
        let mut export = vec![0x81, 0];
        export.extend(0_i32.to_le_bytes());
        export.push(4);
        export.extend(0_u32.to_le_bytes());
        export.push(payload.len() as u8);
        let mut payload_offset = export_offset + export.len() + 1;
        loop {
            let offset = compact_index(payload_offset);
            let next = export_offset + export.len() + offset.len();
            if next == payload_offset {
                export.extend(offset);
                break;
            }
            payload_offset = next;
        }

        let mut bytes = Vec::new();
        bytes.extend(openhp1_package::PACKAGE_MAGIC.to_le_bytes());
        bytes.extend(61_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        for value in [
            names.len(),
            name_offset,
            1,
            export_offset,
            1,
            import_offset,
            0,
            0,
        ] {
            bytes.extend((value as i32).to_le_bytes());
        }
        bytes.extend(name_table);
        bytes.extend(import_table);
        bytes.extend(export);
        bytes.extend(payload);
        bytes
    }

    fn compact_index(mut value: usize) -> Vec<u8> {
        let mut bytes = vec![value as u8 & 0x3f];
        value >>= 6;
        if value != 0 {
            bytes[0] |= 0x40;
        }
        while value != 0 {
            let mut byte = value as u8 & 0x7f;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
        }
        bytes
    }

    #[test]
    fn particle_chaos_is_an_undiluted_delayed_per_particle_impulse() {
        let mut short_velocity = glam::Vec3::ZERO;
        let mut long_velocity = glam::Vec3::ZERO;
        let (mut short_timer, mut long_timer) = (0.0, 0.0);
        let (mut short_random, mut long_random) = (7, 7);
        super::apply_particle_chaos(
            &mut short_velocity,
            &mut short_timer,
            3.0,
            0.5,
            0.01,
            &mut short_random,
        );
        super::apply_particle_chaos(
            &mut long_velocity,
            &mut long_timer,
            3.0,
            0.5,
            10.0,
            &mut long_random,
        );
        assert!(short_velocity.abs_diff_eq(long_velocity, 1.0e-6));
        assert!((short_velocity.length() - 3.0).abs() < 1.0e-6);

        let first_impulse = short_velocity;
        super::apply_particle_chaos(
            &mut short_velocity,
            &mut short_timer,
            3.0,
            0.5,
            0.1,
            &mut short_random,
        );
        assert_eq!(short_velocity, first_impulse);
        assert!((short_timer - 0.4).abs() < 1.0e-6);

        let mut independent_velocity = glam::Vec3::ZERO;
        let mut independent_timer = 0.0;
        let mut independent_random = 9;
        super::apply_particle_chaos(
            &mut independent_velocity,
            &mut independent_timer,
            3.0,
            0.5,
            0.1,
            &mut independent_random,
        );
        assert!((independent_velocity.length() - 3.0).abs() < 1.0e-6);
        assert_eq!(independent_timer, 0.5);
        assert!((short_timer - 0.4).abs() < 1.0e-6);

        super::apply_particle_chaos(
            &mut short_velocity,
            &mut short_timer,
            3.0,
            0.5,
            0.4,
            &mut short_random,
        );
        assert!((short_velocity - first_impulse).length() > 2.999);

        let mut every_update_velocity = glam::Vec3::ZERO;
        let mut every_update_timer = 0.0;
        let mut every_update_random = 11;
        super::apply_particle_chaos(
            &mut every_update_velocity,
            &mut every_update_timer,
            3.0,
            0.0,
            0.1,
            &mut every_update_random,
        );
        let first_update_velocity = every_update_velocity;
        super::apply_particle_chaos(
            &mut every_update_velocity,
            &mut every_update_timer,
            3.0,
            0.0,
            0.1,
            &mut every_update_random,
        );
        assert!(((every_update_velocity - first_update_velocity).length() - 3.0).abs() < 1.0e-6);
        assert_eq!(every_update_timer, 0.0);

        let (mut location, mut velocity, mut timer, mut random) =
            (glam::Vec3::ZERO, glam::Vec3::ZERO, 0.0, 13);
        location += velocity * 0.25;
        super::apply_particle_chaos(&mut velocity, &mut timer, 3.0, 0.5, 0.25, &mut random);
        assert_eq!(location, glam::Vec3::ZERO);
        location += velocity * 0.25;
        assert!(location.abs_diff_eq(velocity * 0.25, 1.0e-6));
    }

    #[test]
    fn hidden_inventory_weapons_render_only_while_attached() {
        let saved = [glam::Vec3::X, glam::Vec3::Y];
        let mut rendered = [glam::Vec3::ZERO; 2];
        assert!(super::sync_hidden_attachment(&mut rendered, &saved, true));
        assert_eq!(rendered, saved);
        assert!(super::sync_hidden_attachment(&mut rendered, &saved, false));
        assert_eq!(rendered, [glam::Vec3::X; 2]);
    }

    #[test]
    fn particle_vectors_stay_in_unreal_space_until_rendering() {
        let yaw_right = super::Rotator {
            yaw: 16_384,
            ..super::Rotator::default()
        };
        assert!(super::rotate_unreal(yaw_right, glam::Vec3::X).abs_diff_eq(glam::Vec3::Y, 0.0001));
        assert_eq!(
            crate::unreal_to_render(glam::Vec3::new(1.0, 2.0, 3.0)),
            glam::Vec3::new(2.0, 3.0, -1.0)
        );
    }

    #[test]
    fn particle_spread_uses_authored_horizontal_and_vertical_angles() {
        let mut random = 0;
        let direction =
            super::particle_direction(super::Rotator::default(), 180.0, 180.0, &mut random);
        assert!(direction.is_finite());
        assert!((direction.length() - 1.0).abs() < 0.0001);
        assert!(!direction.abs_diff_eq(glam::Vec3::X, 0.0001));
    }

    #[test]
    fn owner_mesh_distribution_samples_authored_triangle_vertices() {
        let mut random = 0;
        let point = super::random_mesh_position(
            &[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
            None,
            &[0, 1, 2],
            &mut random,
        )
        .unwrap();
        assert!(point.x >= 0.0 && point.y >= 0.0 && point.x + point.y <= 1.0001);
    }

    #[test]
    fn owner_mesh_distribution_samples_hidden_source_geometry() {
        let mut scene = particle_test_scene();
        scene.actors[0].draw_type = 2;
        scene.actor_states[0].actor.draw_type = 2;
        scene.actors[0].render = Some(crate::SceneActorRenderRange {
            vertices: 4..7,
            indices: 0..3,
        });
        scene.render.mesh.positions = vec![glam::Vec3::ZERO; 4];
        scene.render.mesh.positions.extend([
            glam::Vec3::new(10.0, 0.0, 0.0),
            glam::Vec3::new(10.0, 1.0, 0.0),
            glam::Vec3::new(10.0, 0.0, 1.0),
        ]);
        scene.render.mesh.indices = vec![4, 5, 6];
        let system = scene.particles.get_mut(&0).unwrap();
        system.config = ParticleEmitter {
            actor: 0,
            owner: Some(0),
            emit: true,
            distribution: 2,
            particles_alive: 1,
            particles_per_second: ParticleFloat {
                base: 1.0,
                random: 0.0,
            },
            lifetime: ParticleFloat {
                base: 10.0,
                random: 0.0,
            },
            ..Default::default()
        };
        system.particles.clear();

        assert!(scene.set_actor_draw_type(0, 0).unwrap());
        assert!(scene.actors[0].render.is_some());
        assert!(scene.tick_particles(1.0));
        assert!((scene.particles[&0].particles[0].location.x - 10.0).abs() < 0.0001);
    }

    #[test]
    fn weapon_attachment_uses_the_authored_special_triangle_axes() {
        let transform =
            super::triangle_attachment_transform([glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y])
                .unwrap()
                * glam::Mat4::from_scale(glam::Vec3::splat(2.0));
        assert_eq!(
            transform.transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(0.0, 0.5, 0.0)
        );
        assert_eq!(
            transform.transform_vector3(glam::Vec3::X),
            glam::Vec3::X * 2.0
        );
        assert_eq!(
            transform.transform_vector3(glam::Vec3::Y),
            glam::Vec3::Z * 2.0
        );
    }

    #[test]
    fn animation_tween_pose_follows_actor_transform() {
        let transform = glam::Mat4::from_translation(glam::Vec3::new(3.0, 4.0, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut bones = [glam::Vec3::X];
        let mut tween_positions = [glam::Vec3::Y];
        let mut tween_bones = [glam::Vec3::new(2.0, 0.0, 0.0)];

        super::transform_animation_pose_positions(
            &mut bones,
            Some(&mut tween_positions),
            Some(&mut tween_bones),
            transform,
        );

        assert!(bones[0].abs_diff_eq(glam::Vec3::new(3.0, 5.0, 0.0), 0.0001));
        assert!(tween_positions[0].abs_diff_eq(glam::Vec3::new(2.0, 4.0, 0.0), 0.0001));
        assert!(tween_bones[0].abs_diff_eq(glam::Vec3::new(3.0, 6.0, 0.0), 0.0001));
    }

    #[test]
    fn weapon_attachment_rotation_matches_unreal_ortho_rotation() {
        let expected = openhp1_map::Rotator {
            pitch: 4_096,
            yaw: 12_288,
            roll: -8_192,
        };
        let actual = super::ortho_rotation(super::rotation_matrix(expected)).unwrap();
        for (actual, expected) in
            actual
                .into_iter()
                .zip([expected.pitch, expected.yaw, expected.roll])
        {
            assert!((actual - expected).abs() <= 1, "{actual} != {expected}");
        }
    }

    #[test]
    fn weapon_attachment_follows_animation_tween() {
        let from = glam::Mat4::IDENTITY;
        let to = glam::Mat4::from_translation(glam::Vec3::X * 2.0)
            * glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let transform = super::interpolate_transform(from, to, 0.5);
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;

        assert!(
            transform
                .transform_point3(glam::Vec3::ZERO)
                .abs_diff_eq(glam::Vec3::X, 0.0001)
        );
        assert!(
            transform
                .transform_vector3(glam::Vec3::X)
                .abs_diff_eq(glam::Vec3::new(diagonal, diagonal, 0.0), 0.0001)
        );
    }

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

        let mirror = super::surface_material(PolyFlags::MIRRORED, None, None);
        assert!(mirror.mirror);

        let texture_mirror = super::surface_material(
            PolyFlags::default(),
            None,
            Some(TextureRenderFlags {
                mirrored: true,
                ..Default::default()
            }),
        );
        assert!(texture_mirror.mirror);

        let texture_portal = super::surface_material(
            PolyFlags::default(),
            None,
            Some(TextureRenderFlags {
                portal: true,
                ..Default::default()
            }),
        );
        assert!(texture_portal.portal);

        let raw_portal = super::surface_material(PolyFlags::PORTAL, None, None);
        assert!(super::is_portal_surface(&[raw_portal, texture_portal], 0));
        assert!(super::is_portal_surface(&[raw_portal, texture_portal], 1));
        assert!(!super::is_portal_surface(&[SurfaceMaterial::default()], 0));

        let smooth = super::surface_material(PolyFlags::default(), None, None);
        let surface_no_smooth = super::surface_material(PolyFlags::NO_SMOOTH, None, None);
        let texture_no_smooth = super::surface_material(
            PolyFlags::default(),
            None,
            Some(TextureRenderFlags {
                no_smooth: true,
                ..Default::default()
            }),
        );
        let both_no_smooth = super::surface_material(
            PolyFlags::NO_SMOOTH,
            None,
            Some(TextureRenderFlags {
                no_smooth: true,
                ..Default::default()
            }),
        );
        assert!(!smooth.no_smooth);
        assert!(surface_no_smooth.no_smooth);
        assert!(texture_no_smooth.no_smooth);
        assert!(both_no_smooth.no_smooth);

        let surface_backdrop = super::bsp_surface_material(
            PolyFlags::from_bits(PolyFlags::FAKE_BACKDROP.bits() | PolyFlags::NO_SMOOTH.bits()),
            None,
            None,
        );
        let texture_backdrop = super::bsp_surface_material(
            PolyFlags::FAKE_BACKDROP,
            None,
            Some(TextureRenderFlags {
                no_smooth: true,
                ..Default::default()
            }),
        );
        assert_eq!(surface_backdrop.mode, SurfaceMode::Backdrop);
        assert_eq!(texture_backdrop.mode, SurfaceMode::Backdrop);
        assert!(surface_backdrop.no_smooth);
        assert!(texture_backdrop.no_smooth);

        let bsp_environment =
            super::bsp_surface_material(PolyFlags::from_bits(0x0000_0010), None, None);
        assert!(!bsp_environment.environment_map);
    }

    #[test]
    fn bsp_portal_provenance_uses_only_explicit_root_textures() {
        let portal_texture = TextureRenderFlags {
            portal: true,
            ..Default::default()
        };

        assert!(super::bsp_root_portal(
            PolyFlags::INVISIBLE,
            true,
            Some(portal_texture),
        ));
        assert!(!super::bsp_root_portal(
            PolyFlags::default(),
            false,
            Some(portal_texture),
        ));
        assert!(super::bsp_root_portal(PolyFlags::PORTAL, false, None,));
    }

    #[test]
    fn mesh_environment_map_prefers_actor_then_zone_then_level() {
        assert_eq!(
            super::select_environment_map(Some(1), Some(2), Some(3)),
            Some(1)
        );
        assert_eq!(
            super::select_environment_map(None, Some(2), Some(3)),
            Some(2)
        );
        assert_eq!(super::select_environment_map(None, None, Some(3)), Some(3));
        assert_eq!(super::select_environment_map::<u8>(None, None, None), None);
    }

    #[test]
    fn applies_texture_draw_scale_to_ordinary_mesh_uvs_only() {
        let coordinates = glam::Vec2::new(128.0 / 256.0, 64.0 / 256.0);
        let dimensions = glam::Vec2::new(64.0, 32.0);
        let coordinates_at = |draw_scale, environment_map| {
            super::actor_mesh_texture_coordinates(
                coordinates,
                dimensions,
                crate::SurfaceMaterial {
                    texture_draw_scale: draw_scale,
                    environment_map,
                    ..Default::default()
                },
            )
        };

        assert_eq!(coordinates_at(0.5, false), glam::Vec2::new(16.0, 4.0));
        assert_eq!(coordinates_at(1.0, false), glam::Vec2::new(32.0, 8.0));
        assert_eq!(coordinates_at(24.0, false), glam::Vec2::new(768.0, 192.0));
        assert_eq!(coordinates_at(24.0, true), glam::Vec2::new(32.0, 8.0));
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
    fn bsp_invisible_writes_only_depth_without_an_effective_blend_mode() {
        let depth_only = super::bsp_surface_material(PolyFlags::INVISIBLE, None, None);
        assert_eq!(depth_only.mode, SurfaceMode::DepthOnly);

        let masked = super::bsp_surface_material(
            PolyFlags::from_bits(PolyFlags::INVISIBLE.bits() | PolyFlags::MASKED.bits()),
            None,
            None,
        );
        assert_eq!(masked.mode, SurfaceMode::DepthOnly);
        assert!(masked.masked);

        let texture_invisible = super::bsp_surface_material(
            PolyFlags::PORTAL,
            None,
            Some(TextureRenderFlags {
                invisible: true,
                ..Default::default()
            }),
        );
        assert_eq!(texture_invisible.mode, SurfaceMode::DepthOnly);
        assert!(texture_invisible.portal);

        for blend in [
            PolyFlags::TRANSLUCENT,
            PolyFlags::MODULATED,
            PolyFlags::ALPHA_BLEND,
        ] {
            let material = super::bsp_surface_material(
                PolyFlags::from_bits(PolyFlags::INVISIBLE.bits() | blend.bits()),
                None,
                None,
            );
            assert_eq!(material.mode, SurfaceMode::Hidden);
        }

        for texture_flags in [
            TextureRenderFlags {
                invisible: true,
                translucent: true,
                ..Default::default()
            },
            TextureRenderFlags {
                invisible: true,
                modulated: true,
                ..Default::default()
            },
        ] {
            assert_eq!(
                super::bsp_surface_material(PolyFlags::default(), None, Some(texture_flags)).mode,
                SurfaceMode::Hidden
            );
        }
    }

    #[test]
    fn hp_actor_opacity_forces_alpha_blending() {
        let faded = super::actor_opacity_material(crate::SurfaceMaterial {
            opacity: 0.5,
            masked: true,
            ..Default::default()
        });
        assert_eq!(faded.mode, SurfaceMode::AlphaBlended);
        assert!(!faded.masked);

        let opaque = super::actor_opacity_material(crate::SurfaceMaterial::default());
        assert_eq!(opaque.mode, SurfaceMode::Opaque);
    }

    #[test]
    fn identifies_fixed_game_window_materials_without_frames_or_furnaces() {
        assert!(super::is_window_texture("StainedGlassWind"));
        assert!(super::is_window_texture("Topwindow13_B"));
        assert!(super::is_window_texture("bottomBRWind"));
        assert!(!super::is_window_texture("WindowArch"));
        assert!(!super::is_window_texture("Win9_Wood_3"));
        assert!(!super::is_window_texture("Furnacewindow"));
        assert!(!super::is_window_texture("CastleWall"));
    }

    #[test]
    fn keeps_both_node_zone_speeds_and_only_requested_pan_axes() {
        let zones = [
            glam::Vec2::ONE,
            glam::Vec2::new(2.0, 3.5),
            glam::Vec2::new(4.0, 5.5),
        ];
        assert_eq!(
            super::node_texture_pan_speeds(
                PolyFlags::from_bits(PolyFlags::AUTO_U_PAN.bits() | PolyFlags::AUTO_V_PAN.bits(),),
                [1, 2],
                glam::Vec2::ONE,
                &zones,
            ),
            [2.0, 3.5, 4.0, 5.5]
        );
        assert_eq!(
            super::node_texture_pan_speeds(PolyFlags::AUTO_V_PAN, [1, 2], glam::Vec2::ONE, &zones,),
            [0.0, 3.5, 0.0, 5.5]
        );
        assert!(PolyFlags::from_bits(0x0000_2000).contains(PolyFlags::SMALL_WAVY));

        let material =
            super::bsp_surface_material(PolyFlags::from_bits(0x0000_2000), Some(1), None);
        assert!(material.small_wavy);
    }

    #[test]
    fn missing_node_zone_uses_level_info_not_zone_zero() {
        let level = glam::Vec2::new(6.0, 7.0);
        let zones = [glam::Vec2::new(2.0, 3.0)];
        let flags =
            PolyFlags::from_bits(PolyFlags::AUTO_U_PAN.bits() | PolyFlags::AUTO_V_PAN.bits());

        assert_eq!(
            super::node_texture_pan_speeds(flags, [0, -1], level, &zones),
            [2.0, 3.0, 6.0, 7.0]
        );
        assert_eq!(
            super::node_texture_pan_speeds(flags, [99, 0], level, &zones),
            [6.0, 7.0, 2.0, 3.0]
        );
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
    fn draw_scale_resizes_mesh_bounds_about_the_pivot() {
        let bounds = (
            glam::Vec3::new(1.0, 2.0, 3.0),
            glam::Vec3::new(3.0, 6.0, 7.0),
        );
        assert_eq!(
            super::scale_bounds_about(bounds, glam::Vec3::new(1.0, 2.0, 3.0), 0.5),
            (
                glam::Vec3::new(1.0, 2.0, 3.0),
                glam::Vec3::new(2.0, 4.0, 5.0)
            )
        );
        assert_eq!(
            super::scale_bounds_about(bounds, glam::Vec3::new(1.0, 2.0, 3.0), -1.0),
            (
                glam::Vec3::new(-1.0, -2.0, -1.0),
                glam::Vec3::new(1.0, 2.0, 3.0)
            )
        );
    }

    #[test]
    fn pre_pivot_changes_follow_mesh_and_brush_transform_conventions() {
        let mut actor = super::runtime_actor_placeholder(0);
        actor.pre_pivot = glam::Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(
            super::pre_pivot_translation(&actor, glam::Vec3::new(4.0, 6.0, 8.0)),
            glam::Vec3::new(3.0, 4.0, 5.0)
        );

        actor.draw_type = 3;
        actor.main_scale = glam::Vec3::splat(2.0);
        actor.rotation.yaw = 16_384;
        let delta = super::pre_pivot_translation(&actor, glam::Vec3::new(2.0, 2.0, 3.0));
        assert!(delta.abs_diff_eq(glam::Vec3::new(0.0, -2.0, 0.0), 1.0e-5));
    }

    #[test]
    fn mesh_to_object_uses_the_authored_origin() {
        let transform = super::mesh_to_object_transform(
            glam::Vec3::ONE,
            glam::Vec3::new(0.0, 0.0, 42.0),
            openhp1_map::Rotator::default(),
        );
        assert_eq!(
            transform.transform_point3(glam::Vec3::new(1.0, 2.0, 42.0)),
            glam::Vec3::new(1.0, 2.0, 0.0)
        );
    }

    #[test]
    fn skeletal_mesh_adjust_matches_retail_alignment_conditions() {
        let actor = super::ActorState {
            physics: 1,
            collision_height: 50.0,
            collide_world: true,
            align_bottom: true,
            draw_scale: 1.2,
            ..Default::default()
        };
        let adjust = super::skeletal_mesh_adjust(
            true,
            Some((
                glam::Vec3::new(-20.0, -70.0, 0.25),
                glam::Vec3::splat(120.0),
            )),
            glam::Vec3::ZERO,
            glam::Vec3::ONE,
            &actor,
        );
        assert!((adjust.z - -52.8).abs() < f32::EPSILON);

        for actor in [
            super::ActorState {
                physics: 0,
                ..actor.clone()
            },
            super::ActorState {
                collide_type: 3,
                ..actor.clone()
            },
        ] {
            assert_eq!(
                super::skeletal_mesh_adjust(
                    true,
                    Some((glam::Vec3::ZERO, glam::Vec3::ONE)),
                    glam::Vec3::ZERO,
                    glam::Vec3::ONE,
                    &actor,
                ),
                glam::Vec3::ZERO
            );
        }
    }

    #[test]
    fn rotates_actor_vertices_around_their_unreal_origin() {
        let origin = glam::Vec3::new(10.0, 0.0, 0.0);
        let mut positions = [glam::Vec3::new(11.0, 0.0, 0.0)];
        let mut normals = [glam::Vec3::X];
        let transform = super::rotation_delta(
            origin,
            Default::default(),
            openhp1_map::Rotator {
                yaw: 16_384,
                ..Default::default()
            },
        );
        super::transform_positions(&mut positions, transform);
        super::transform_normals(&mut normals, transform);
        assert!(positions[0].abs_diff_eq(glam::Vec3::new(10.0, 1.0, 0.0), 1.0e-5));
        assert!(normals[0].abs_diff_eq(glam::Vec3::Y, 1.0e-5));
    }

    #[test]
    fn transforms_moving_brushes_around_their_pre_pivot() {
        let actor = super::ActorState {
            location: glam::Vec3::new(10.0, 20.0, 30.0),
            pre_pivot: glam::Vec3::new(2.0, 3.0, 4.0),
            main_scale: glam::Vec3::splat(2.0),
            ..Default::default()
        };
        let transform = super::brush_transform(&actor);
        assert_eq!(transform.transform_point3(actor.pre_pivot), actor.location);
        assert_eq!(
            transform.transform_point3(actor.pre_pivot + glam::Vec3::X),
            actor.location + glam::Vec3::X * 2.0
        );
    }

    #[test]
    fn mirrored_brushes_preserve_polygon_winding() {
        let actor = super::ActorState {
            main_scale: glam::Vec3::new(1.0, -1.0, 1.0),
            ..Default::default()
        };
        let transform = super::brush_transform(&actor);
        let points = [glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]
            .map(|point| transform.transform_point3(point));
        let triangle = super::brush_triangle(0, 1, transform.determinant() < 0.0);
        let geometric_normal = (points[triangle[1] as usize] - points[triangle[0] as usize])
            .cross(points[triangle[2] as usize] - points[triangle[0] as usize]);
        let transformed_normal =
            glam::Mat3::from_mat4(transform).inverse().transpose() * glam::Vec3::Z;

        assert!(geometric_normal.dot(transformed_normal) > 0.0);
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
        assert!(pitch.transform_vector3(glam::Vec3::X).abs_diff_eq(
            super::rotate_unreal(
                openhp1_map::Rotator {
                    pitch: quarter_turn,
                    ..Default::default()
                },
                glam::Vec3::X,
            ),
            1.0e-6,
        ));
        assert!(
            roll.transform_vector3(glam::Vec3::Y)
                .abs_diff_eq(-glam::Vec3::Z, 1.0e-6)
        );
    }

    #[test]
    fn sprite_quad_follows_the_view_axes() {
        let center = glam::Vec3::new(1.0, 2.0, 3.0);
        let rotation = super::rotation_matrix(openhp1_map::Rotator {
            yaw: 16_384,
            ..Default::default()
        });
        let positions = super::sprite_positions(center, glam::Vec2::new(2.0, 1.0), rotation);
        assert!(positions[0].abs_diff_eq(glam::Vec3::new(3.0, 2.0, 2.0), 1.0e-5));
        assert!(positions[2].abs_diff_eq(glam::Vec3::new(-1.0, 2.0, 4.0), 1.0e-5));
    }

    #[test]
    fn particle_sprite_spin_rotates_within_the_view_plane() {
        let center = glam::Vec3::new(1.0, 2.0, 3.0);
        let positions = super::particle_sprite_positions(
            center,
            glam::Vec2::new(2.0, 1.0),
            glam::Mat4::IDENTITY,
            std::f32::consts::FRAC_PI_2,
        );
        assert!(positions[0].abs_diff_eq(glam::Vec3::new(1.0, 3.0, 1.0), 1.0e-5));
        assert!(positions[2].abs_diff_eq(glam::Vec3::new(1.0, 1.0, 5.0), 1.0e-5));
        assert!(((positions[0] + positions[2]) * 0.5).abs_diff_eq(center, 1.0e-5));
    }

    #[test]
    fn particle_liquid_uses_the_view_plane() {
        let center = glam::Vec3::new(1.0, 2.0, 3.0);
        let positions = super::particle_render_primitive_positions(
            center,
            glam::Vec2::new(2.0, 1.0),
            super::rotation_matrix(openhp1_map::Rotator {
                yaw: 16_384,
                ..Default::default()
            }),
            0.0,
            2,
        );
        assert!(positions[0].abs_diff_eq(glam::Vec3::new(3.0, 2.0, 1.5), 1.0e-5));
        assert!(positions[1].abs_diff_eq(glam::Vec3::new(1.0, 2.0, 1.0), 1.0e-5));
        assert!(positions[2].abs_diff_eq(glam::Vec3::new(-1.0, 2.0, 2.0), 1.0e-5));
        assert_eq!(positions[3], center);
    }
}
