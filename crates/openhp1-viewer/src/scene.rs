use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use glam::{Mat3, Mat4, Vec2, Vec3};
use openhp1_map::{Actor, ActorProperties, Level, Model, PolyFlags, Rotator, VertexLighting};
use openhp1_mesh::Mesh;
use openhp1_package::{ObjectReader, ObjectReference, Package, PackageStore, ResolvedObject};
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
    pub(crate) actor_meshes: usize,
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
        let actor_meshes = load_actor_meshes(
            &mut packages,
            &package,
            &level,
            &model,
            &mut mesh,
            &mut textures,
            &mut surface_materials,
        )
        .context("failed to load actor meshes")?;
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
            actor_meshes,
        })
    }
}

type ObjectKey = (String, usize);

#[derive(Clone)]
struct SceneObject {
    package: Arc<Package>,
    export_index: usize,
}

impl SceneObject {
    fn key(&self) -> ObjectKey {
        (self.package.summary().source.to_string(), self.export_index)
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
    skin: Option<SceneObject>,
    texture: Option<SceneObject>,
    multi_skins: Vec<Option<SceneObject>>,
    style: u8,
    ambient_glow: u8,
    scale_glow: f32,
    hidden: bool,
    unlit: bool,
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
            skin: None,
            texture: None,
            multi_skins: Vec::new(),
            style: 1,
            ambient_glow: 0,
            scale_glow: 1.0,
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
        if let Some(hidden) = properties.hidden {
            self.hidden = hidden;
        }
        if let Some(unlit) = properties.unlit {
            self.unlit = unlit;
        }
        Ok(())
    }
}

fn load_actor_meshes(
    packages: &mut PackageStore,
    map: &Arc<Package>,
    level: &Level,
    model: &Model,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
) -> Result<usize> {
    let vertex_lighting = model
        .vertex_lighting(map)
        .context("failed to decode actor vertex lighting")?;
    let mut class_cache = HashMap::<ObjectKey, Option<ActorState>>::new();
    let mut mesh_cache = HashMap::<ObjectKey, Option<Arc<Mesh>>>::new();
    let mut decoded_textures = HashMap::<ObjectKey, Option<DecodedTexture>>::new();
    let mut images = HashMap::<(String, usize, bool), usize>::new();
    let mut rendered = 0;

    for reference in &level.actors {
        let ObjectReference::Export(export_index) = *reference else {
            continue;
        };
        let export = &map.summary().exports[export_index];
        let actor = match Actor::decode(map, export_index) {
            Ok(actor) => actor,
            Err(error) => {
                warn!(export_index, %error, "could not decode actor");
                continue;
            }
        };
        let class = match packages.resolve(map, export.class) {
            Ok(Some(class)) => SceneObject::from(class),
            Ok(None) => continue,
            Err(error) => {
                warn!(export_index, %error, "could not resolve actor class");
                continue;
            }
        };
        let mut state = class_state(packages, &class, &mut class_cache, 0).unwrap_or_default();
        if let Err(error) = state.apply(packages, map, &actor.properties) {
            warn!(export_index, %error, "could not resolve actor properties");
            continue;
        }
        if state.hidden || state.draw_type != 2 {
            continue;
        }
        let Some(mesh_object) = state.mesh.clone() else {
            continue;
        };
        let mesh_key = mesh_object.key();
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
            continue;
        };
        match append_actor_mesh(
            packages,
            &mesh_object,
            &mesh,
            &state,
            model,
            &vertex_lighting,
            render_mesh,
            textures,
            materials,
            &mut decoded_textures,
            &mut images,
        ) {
            Ok(true) => rendered += 1,
            Ok(false) => {}
            Err(error) => {
                warn!(
                    actor = %map.summary().name(export.object_name),
                    %error,
                    "could not append actor mesh"
                );
            }
        }
    }
    Ok(rendered)
}

fn class_state(
    packages: &mut PackageStore,
    class: &SceneObject,
    cache: &mut HashMap<ObjectKey, Option<ActorState>>,
    depth: usize,
) -> Option<ActorState> {
    if depth > 32 {
        return None;
    }
    let key = class.key();
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
            cache.insert(key, None);
            return None;
        }
    };
    let mut state = packages
        .resolve(&class.package, base)
        .ok()
        .flatten()
        .map(SceneObject::from)
        .and_then(|base| class_state(packages, &base, cache, depth + 1))
        .unwrap_or_default();
    if let Err(error) = state.apply(packages, &class.package, &properties) {
        warn!(%error, "could not resolve actor class properties");
        cache.insert(key, None);
        return None;
    }
    cache.insert(key, Some(state.clone()));
    Some(state)
}

fn decode_class_defaults(class: &SceneObject) -> Result<(ObjectReference, ActorProperties)> {
    let mut reader = class.package.export_reader(class.export_index)?;
    let base = reader.read_object_reference()?;
    reader.read_object_reference()?; // next field
    reader.read_object_reference()?; // script text
    reader.read_object_reference()?; // children
    reader.read_compact_index()?; // friendly name
    reader.read_u32()?; // line
    reader.read_u32()?; // text position
    let script_size = reader.read_u32()?;
    ensure!(
        script_size == 0,
        "class contains bytecode ({script_size} decoded bytes)"
    );

    reader.read_u64()?; // probe mask
    reader.read_u64()?; // ignore mask
    reader.read_u16()?; // label table
    reader.read_u32()?; // state flags
    if class.package.summary().header.version <= 61 {
        reader.read_u32()?; // old class record size
    }
    reader.read_u32()?; // class flags
    reader.read_bytes(16)?; // class GUID
    skip_class_array(&mut reader, "class dependencies", |reader| {
        reader.read_object_reference()?;
        reader.read_u32()?;
        reader.read_u32()?;
        Ok(())
    })?;
    skip_class_array(&mut reader, "class package imports", |reader| {
        reader.read_compact_index()?;
        Ok(())
    })?;
    if class.package.summary().header.version >= 62 {
        reader.read_compact_index()?; // class within
        reader.read_compact_index()?; // config name
    }
    Ok((base, ActorProperties::decode(&mut reader)?))
}

fn skip_class_array(
    reader: &mut ObjectReader<'_>,
    field: &'static str,
    mut element: impl FnMut(&mut ObjectReader<'_>) -> Result<()>,
) -> Result<()> {
    let count = reader.read_compact_index()?;
    let count = usize::try_from(count).with_context(|| format!("{field} has negative count"))?;
    for _ in 0..count {
        element(reader)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_actor_mesh(
    packages: &mut PackageStore,
    mesh_object: &SceneObject,
    mesh: &Mesh,
    actor: &ActorState,
    model: &Model,
    vertex_lighting: &VertexLighting,
    render_mesh: &mut openhp1_map::TriangleMesh,
    textures: &mut Vec<TextureImage>,
    materials: &mut Vec<SurfaceMaterial>,
    decoded_textures: &mut HashMap<ObjectKey, Option<DecodedTexture>>,
    images: &mut HashMap<(String, usize, bool), usize>,
) -> Result<bool> {
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

    for triangle in &mesh.triangles {
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
    Ok(render_mesh.positions.len() != first_vertex)
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
    decoded: &mut HashMap<ObjectKey, Option<DecodedTexture>>,
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
    let key = texture.key();
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
    let image_key = (key.0, key.1, material.masked);
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
    Mat4::from_rotation_x(radians.z)
        * Mat4::from_rotation_y(radians.x)
        * Mat4::from_rotation_z(-radians.y)
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
