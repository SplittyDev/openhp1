use std::collections::{HashMap, HashSet};

use glam::Vec3;
use openhp1_package::{ObjectReference, Package, PropertyKind};

use crate::{
    Error, Level, Model, Result, Rotator,
    decode::{index, skip_object_stack},
};

#[derive(Clone, Debug)]
pub struct LightmapImage {
    pub width: u32,
    pub height: u32,
    /// Linear RGB light values stored in normalized bytes. The renderer
    /// applies UE1's conventional 2x lightmap modulation.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct LightActor {
    location: Vec3,
    rotation: Rotator,
    light_type: u8,
    effect: u8,
    brightness: u8,
    hue: u8,
    saturation: u8,
    radius: u8,
    cone: u8,
    corona: bool,
    special_lit: bool,
}

impl Default for LightActor {
    fn default() -> Self {
        Self {
            location: Vec3::ZERO,
            rotation: Rotator::default(),
            light_type: 1,
            effect: 0,
            brightness: 64,
            hue: 0,
            saturation: 255,
            radius: 64,
            cone: 128,
            corona: false,
            special_lit: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct VertexLight {
    location: Vec3,
    color: Vec3,
    radius: f32,
    effect: u8,
}

#[derive(Clone, Debug)]
pub struct VertexLighting {
    level_ambient: Vec3,
    zone_ambient: Vec<Vec3>,
    lights: Vec<VertexLight>,
}

#[derive(Clone, Debug)]
pub struct ActorVertexLighting {
    ambient: Vec3,
    scale_glow: f32,
    lights: Vec<VertexLight>,
}

impl ActorVertexLighting {
    pub fn color(&self, location: Vec3, normal: Vec3, unlit: bool) -> Vec3 {
        if unlit {
            return (self.ambient + self.scale_glow * 0.5) * 2.0;
        }
        let mut color = Vec3::ZERO;
        for light in &self.lights {
            let direction = light.location - location;
            let distance = direction.length();
            if distance == 0.0 || distance >= light.radius {
                continue;
            }
            let attenuation = 1.0 - distance / light.radius;
            let angle_attenuation = (direction / distance).dot(normal).abs();
            color += light.color * (attenuation * angle_attenuation);
        }
        (self.ambient + color * (self.scale_glow * 1.5)) * 2.0
    }
}

#[derive(Clone, Copy)]
struct AmbientLight {
    brightness: u8,
    hue: u8,
    saturation: u8,
}

impl Default for AmbientLight {
    fn default() -> Self {
        Self {
            brightness: 0,
            hue: 0,
            saturation: 255,
        }
    }
}

impl Model {
    /// Reconstructs UE1's static lightmaps from the serialized shadow masks,
    /// light actors, and zone ambient colors.
    pub fn lightmap_images(&self, package: &Package) -> Result<Vec<LightmapImage>> {
        let mut lightmap_surfaces = vec![None; self.light_maps.len()];
        let mut lightmap_zones = vec![None; self.light_maps.len()];
        for node in &self.nodes {
            let surface_index = index(node.surface, self.surfaces.len(), "node surface")?;
            let lightmap = self.surfaces[surface_index].light_map;
            let Ok(lightmap_index) = usize::try_from(lightmap) else {
                continue;
            };
            if lightmap_index >= self.light_maps.len() {
                return Err(Error::InvalidIndex {
                    field: "surface lightmap",
                    value: lightmap,
                    length: self.light_maps.len(),
                });
            }
            lightmap_surfaces[lightmap_index].get_or_insert(surface_index);
            // UE1 draws the stored polygon winding from Zone1. Zone zero is
            // valid and inherits the active LevelInfo's ambient settings.
            let zone = node.zones[1];
            if let Ok(zone) = usize::try_from(zone)
                && zone < self.zones.len()
            {
                lightmap_zones[lightmap_index].get_or_insert(zone);
            }
        }

        let level_ambient = decode_level_ambient(package)?;
        let mut ambient_cache = HashMap::new();
        let mut light_cache = HashMap::new();
        let mut images = Vec::with_capacity(self.light_maps.len());
        for lightmap_index in 0..self.light_maps.len() {
            let ambient = match lightmap_zones[lightmap_index]
                .and_then(|zone| self.zones.get(zone))
                .map(|zone| zone.actor)
            {
                Some(ObjectReference::Export(export_index)) => {
                    if let Some(ambient) = ambient_cache.get(&export_index) {
                        *ambient
                    } else {
                        let ambient = decode_ambient(package, export_index)?;
                        ambient_cache.insert(export_index, ambient);
                        ambient
                    }
                }
                _ => level_ambient,
            };
            images.push(self.build_lightmap(
                package,
                lightmap_index,
                lightmap_surfaces[lightmap_index],
                ambient,
                &mut light_cache,
            )?);
        }
        Ok(images)
    }

    fn build_lightmap(
        &self,
        package: &Package,
        lightmap_index: usize,
        surface_index: Option<usize>,
        ambient: AmbientLight,
        light_cache: &mut HashMap<usize, LightActor>,
    ) -> Result<LightmapImage> {
        let lightmap = &self.light_maps[lightmap_index];
        let (width, height) = lightmap_dimensions(lightmap_index, lightmap.clamp)?;
        if !lightmap.scale[0].is_finite()
            || !lightmap.scale[1].is_finite()
            || lightmap.scale[0] <= 0.0
            || lightmap.scale[1] <= 0.0
        {
            return Err(Error::InvalidLightmapScale {
                index: lightmap_index,
                u: lightmap.scale[0],
                v: lightmap.scale[1],
            });
        }

        let mut pixels =
            vec![hsb_to_rgb(ambient.hue, ambient.saturation, ambient.brightness); width * height];
        let Some(surface_index) = surface_index else {
            return Ok(lightmap_image(width, height, &pixels));
        };
        let surface = &self.surfaces[surface_index];
        let base = self.points[index(surface.base_point, self.points.len(), "surface base point")?];
        let texture_u =
            self.vectors[index(surface.texture_u, self.vectors.len(), "surface texture U")?];
        let texture_v =
            self.vectors[index(surface.texture_v, self.vectors.len(), "surface texture V")?];
        let normal = self.vectors[index(surface.normal, self.vectors.len(), "surface normal")?]
            .normalize_or_zero();
        let locations = lightmap_locations(
            lightmap_index,
            lightmap,
            base,
            texture_u,
            texture_v,
            width,
            height,
        )?;

        let Ok(mut list_index) = usize::try_from(lightmap.light_actors) else {
            return if lightmap.light_actors == -1 {
                Ok(lightmap_image(width, height, &pixels))
            } else {
                Err(Error::InvalidLightList {
                    index: lightmap_index,
                    offset: lightmap.light_actors,
                    length: self.lights.len(),
                })
            };
        };
        let mut shadow_index = 0_usize;
        loop {
            let reference = *self.lights.get(list_index).ok_or(Error::InvalidLightList {
                index: lightmap_index,
                offset: lightmap.light_actors,
                length: self.lights.len(),
            })?;
            list_index += 1;
            if reference == ObjectReference::None {
                break;
            }
            if let ObjectReference::Export(export_index) = reference {
                let light = if let Some(light) = light_cache.get(&export_index) {
                    *light
                } else {
                    let light = decode_light(package, export_index)?;
                    light_cache.insert(export_index, light);
                    light
                };
                if light.light_type != 0 && light.brightness != 0 {
                    let shadow = self.shadowmap(lightmap_index, shadow_index, width, height)?;
                    add_light(&mut pixels, &locations, normal, &shadow, light);
                }
            }
            shadow_index += 1;
        }
        Ok(lightmap_image(width, height, &pixels))
    }

    fn shadowmap(
        &self,
        lightmap_index: usize,
        shadow_index: usize,
        width: usize,
        height: usize,
    ) -> Result<Vec<f32>> {
        let lightmap = &self.light_maps[lightmap_index];
        let pitch = width.div_ceil(8);
        let mask_size = pitch
            .checked_mul(height)
            .ok_or(Error::InvalidLightmapDimensions {
                index: lightmap_index,
                width: lightmap.clamp[0],
                height: lightmap.clamp[1],
            })?;
        let data_offset =
            usize::try_from(lightmap.data_offset).map_err(|_| Error::InvalidLightBits {
                index: lightmap_index,
                start: 0,
                end: mask_size,
                length: self.light_bits.len(),
            })?;
        let start = shadow_index
            .checked_mul(mask_size)
            .and_then(|offset| data_offset.checked_add(offset))
            .ok_or(Error::InvalidLightBits {
                index: lightmap_index,
                start: data_offset,
                end: usize::MAX,
                length: self.light_bits.len(),
            })?;
        let end = start
            .checked_add(mask_size)
            .ok_or(Error::InvalidLightBits {
                index: lightmap_index,
                start,
                end: usize::MAX,
                length: self.light_bits.len(),
            })?;
        let bits = self
            .light_bits
            .get(start..end)
            .ok_or(Error::InvalidLightBits {
                index: lightmap_index,
                start,
                end,
                length: self.light_bits.len(),
            })?;
        Ok(blur_shadow_bits(bits, width, height))
    }

    pub fn vertex_lighting(&self, package: &Package) -> Result<VertexLighting> {
        let level_ambient = decode_level_ambient(package)?;
        let level_ambient = hsb_to_rgb(
            level_ambient.hue,
            level_ambient.saturation,
            level_ambient.brightness,
        );
        let mut zone_ambient = Vec::with_capacity(self.zones.len());
        for zone in &self.zones {
            zone_ambient.push(match zone.actor {
                ObjectReference::Export(export_index) => {
                    let ambient = decode_ambient(package, export_index)?;
                    hsb_to_rgb(ambient.hue, ambient.saturation, ambient.brightness)
                }
                _ => level_ambient,
            });
        }

        let mut seen = HashSet::new();
        let mut lights = Vec::new();
        for reference in &self.lights {
            let ObjectReference::Export(export_index) = *reference else {
                continue;
            };
            if !seen.insert(export_index) {
                continue;
            }
            let light = decode_light(package, export_index)?;
            if light.light_type == 0 || light.brightness == 0 || light.corona || light.special_lit {
                continue;
            }
            lights.push(VertexLight {
                location: light.location,
                color: hsb_to_rgb(light.hue, light.saturation, light.brightness),
                radius: (f32::from(light.radius) + 1.0) * 25.0,
                effect: light.effect,
            });
        }
        Ok(VertexLighting {
            level_ambient,
            zone_ambient,
            lights,
        })
    }

    pub fn zone_at(&self, point: Vec3) -> usize {
        let mut node_index = 0;
        loop {
            let Some(node) = self.nodes.get(node_index) else {
                return 0;
            };
            let side = plane_side(node.plane, point);
            let child = if side >= 0.0 { node.front } else { node.back };
            if let Ok(child) = usize::try_from(child) {
                node_index = child;
                continue;
            }
            return usize::try_from(if side >= 0.0 {
                node.zones[1]
            } else {
                node.zones[0]
            })
            .ok()
            .filter(|zone| *zone < self.zones.len())
            .unwrap_or(0);
        }
    }

    fn blocks_light(&self, from: Vec3, to: Vec3) -> bool {
        let offset = to - from;
        let distance = offset.length();
        if self.nodes.is_empty() || distance < 0.01 {
            return false;
        }
        self.trace_light_node(0, from, offset / distance, 0.01, distance + 1.0)
    }

    fn trace_light_node(
        &self,
        node_index: usize,
        origin: Vec3,
        direction: Vec3,
        minimum: f32,
        maximum: f32,
    ) -> bool {
        let Some(node) = self.nodes.get(node_index) else {
            return false;
        };
        let mut polygon_index = Some(node_index);
        while let Some(index) = polygon_index {
            let Some(polygon) = self.nodes.get(index) else {
                break;
            };
            if polygon.flags & 4 == 0
                && self
                    .node_ray_intersection(polygon, origin, direction, maximum)
                    .is_some_and(|distance| distance >= minimum && distance < maximum)
            {
                return true;
            }
            polygon_index = usize::try_from(polygon.coplanar).ok();
        }

        let from_side = plane_side(node.plane, origin);
        let to_side = plane_side(node.plane, origin + direction * maximum);
        if let Ok(front) = usize::try_from(node.front)
            && (from_side >= 0.0 || to_side >= 0.0)
            && self.trace_light_node(front, origin, direction, minimum, maximum)
        {
            return true;
        }
        if let Ok(back) = usize::try_from(node.back)
            && (from_side <= 0.0 || to_side <= 0.0)
            && self.trace_light_node(back, origin, direction, minimum, maximum)
        {
            return true;
        }
        false
    }

    fn node_ray_intersection(
        &self,
        node: &crate::BspNode,
        origin: Vec3,
        direction: Vec3,
        maximum: f32,
    ) -> Option<f32> {
        if node.vertex_count < 3 {
            return None;
        }
        let surface = usize::try_from(node.surface)
            .ok()
            .and_then(|index| self.surfaces.get(index))?;
        if surface.poly_flags.contains(crate::PolyFlags::NOT_SOLID) {
            return None;
        }
        let from_side = plane_side(node.plane, origin);
        let to_side = plane_side(node.plane, origin + direction * maximum);
        if from_side.is_sign_positive() == to_side.is_sign_positive()
            && from_side != 0.0
            && to_side != 0.0
        {
            return None;
        }
        let first = usize::try_from(node.vertex_pool).ok()?;
        let polygon = self
            .vertices
            .get(first..first.checked_add(usize::from(node.vertex_count))?)?;
        let point = |vertex: &crate::BspVertex| {
            usize::try_from(vertex.point)
                .ok()
                .and_then(|index| self.points.get(index))
                .copied()
        };
        let a = point(&polygon[0])?;
        let mut b = point(&polygon[1])?;
        let mut nearest = None;
        for vertex in &polygon[2..] {
            let c = point(vertex)?;
            if let Some(distance) = ray_triangle(origin, direction, maximum, [a, b, c]) {
                nearest = Some(nearest.map_or(distance, |current: f32| current.min(distance)));
            }
            b = c;
        }
        nearest
    }
}

impl VertexLighting {
    pub fn for_actor(
        &self,
        model: &Model,
        center: Vec3,
        ambient_glow: u8,
        scale_glow: f32,
    ) -> ActorVertexLighting {
        let ambient_glow = if ambient_glow == 255 {
            0.2
        } else {
            f32::from(ambient_glow) / 255.0
        };
        let ambient = self
            .zone_ambient
            .get(model.zone_at(center))
            .copied()
            .unwrap_or(self.level_ambient)
            + ambient_glow;
        let lights = self
            .lights
            .iter()
            .copied()
            .filter(|light| {
                let mut offset = light.location - center;
                if light.effect == 17 {
                    offset.z = 0.0;
                }
                offset.length_squared() < light.radius * light.radius
                    && !model.blocks_light(light.location, center)
            })
            .collect();
        ActorVertexLighting {
            ambient,
            scale_glow,
            lights,
        }
    }
}

fn plane_side(plane: [f32; 4], point: Vec3) -> f32 {
    Vec3::from_array([plane[0], plane[1], plane[2]]).dot(point) - plane[3]
}

fn ray_triangle(origin: Vec3, direction: Vec3, maximum: f32, points: [Vec3; 3]) -> Option<f32> {
    let edge1 = points[1] - points[0];
    let edge2 = points[2] - points[0];
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let t = origin - points[0];
    let u = t.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge2.dot(q) * inverse;
    (distance > f32::EPSILON && distance <= maximum).then_some(distance)
}

fn decode_level_ambient(package: &Package) -> Result<AmbientLight> {
    let Some(level_index) = package
        .summary()
        .exports
        .iter()
        .position(|export| package.summary().class_name(export) == Some("Level"))
    else {
        return Err(Error::MissingLevel);
    };
    let level = Level::decode(package, level_index)?;
    match level.actors.into_iter().find_map(|actor| match actor {
        ObjectReference::Export(index)
            if package
                .summary()
                .exports
                .get(index)
                .and_then(|export| package.summary().class_name(export))
                == Some("LevelInfo") =>
        {
            Some(index)
        }
        _ => None,
    }) {
        Some(index) => decode_ambient(package, index),
        None => Ok(AmbientLight::default()),
    }
}

fn lightmap_dimensions(index: usize, clamp: [i32; 2]) -> Result<(usize, usize)> {
    let width = usize::try_from(clamp[0]).ok();
    let height = usize::try_from(clamp[1]).ok();
    match (width, height) {
        (Some(width), Some(height)) if width != 0 && height != 0 => Ok((width, height)),
        _ => Err(Error::InvalidLightmapDimensions {
            index,
            width: clamp[0],
            height: clamp[1],
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn lightmap_locations(
    lightmap_index: usize,
    lightmap: &crate::LightMap,
    base: Vec3,
    texture_u: Vec3,
    texture_v: Vec3,
    width: usize,
    height: usize,
) -> Result<Vec<Vec3>> {
    let pan_u = texture_u.dot(base) + lightmap.pan.x - 0.5 * lightmap.scale[0];
    let pan_v = texture_v.dot(base) + lightmap.pan.y - 0.5 * lightmap.scale[1];
    let points = [base, base + texture_u, base + texture_v];
    let coordinates = points.map(|point| {
        glam::Vec2::new(
            (texture_u.dot(point) - pan_u) / lightmap.scale[0],
            (texture_v.dot(point) - pan_v) / lightmap.scale[1],
        )
    });
    let left_step = (coordinates[2].x - coordinates[0].x) / (coordinates[2].y - coordinates[0].y);
    let right_step = (coordinates[2].x - coordinates[1].x) / (coordinates[2].y - coordinates[1].y);
    let mut locations = Vec::with_capacity(width * height);
    for y in 0..height {
        let sample_y = y as f32 + 0.5;
        let mut x0 = coordinates[0].x + left_step * (sample_y - coordinates[0].y) + 0.5;
        let mut x1 = coordinates[1].x + right_step * (sample_y - coordinates[1].y) + 0.5;
        let t0 = (sample_y - coordinates[0].y) / (coordinates[2].y - coordinates[0].y);
        let t1 = (sample_y - coordinates[1].y) / (coordinates[2].y - coordinates[1].y);
        let mut point0 = points[0].lerp(points[2], t0);
        let mut point1 = points[1].lerp(points[2], t1);
        if x1 < x0 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut point0, &mut point1);
        }
        for x in 0..width {
            let t = (x as f32 + 0.5 - x0) / (x1 - x0);
            locations.push(point0.lerp(point1, t));
        }
    }
    if locations.iter().any(|point| !point.is_finite()) {
        return Err(Error::InvalidLightmapScale {
            index: lightmap_index,
            u: lightmap.scale[0],
            v: lightmap.scale[1],
        });
    }
    Ok(locations)
}

fn decode_light(package: &Package, export_index: usize) -> Result<LightActor> {
    let mut reader = package.export_reader(export_index)?;
    skip_object_stack(package, export_index, &mut reader)?;
    let mut light = LightActor::default();
    while let Some(property) = reader.next_property()? {
        let name = reader.summary().name(property.name);
        let struct_name = property
            .struct_name
            .map(|index| reader.summary().name(index));
        let mut value = reader.property_reader(&property);
        match (name, property.kind, struct_name) {
            ("Location", PropertyKind::Struct, Some("Vector")) => {
                light.location = Vec3::new(value.read_f32()?, value.read_f32()?, value.read_f32()?);
            }
            ("Rotation", PropertyKind::Struct, Some("Rotator")) => {
                light.rotation = Rotator {
                    pitch: value.read_i32()?,
                    yaw: value.read_i32()?,
                    roll: value.read_i32()?,
                };
            }
            ("LightType", PropertyKind::Byte, _) => light.light_type = value.read_u8()?,
            ("LightEffect", PropertyKind::Byte, _) => light.effect = value.read_u8()?,
            ("LightBrightness", PropertyKind::Byte, _) => light.brightness = value.read_u8()?,
            ("LightHue", PropertyKind::Byte, _) => light.hue = value.read_u8()?,
            ("LightSaturation", PropertyKind::Byte, _) => light.saturation = value.read_u8()?,
            ("LightRadius", PropertyKind::Byte, _) => light.radius = value.read_u8()?,
            ("LightCone", PropertyKind::Byte, _) => light.cone = value.read_u8()?,
            ("bCorona", PropertyKind::Bool, _) => {
                light.corona = property.bool_value.unwrap_or(false);
            }
            ("bSpecialLit", PropertyKind::Bool, _) => {
                light.special_lit = property.bool_value.unwrap_or(false);
            }
            _ => {}
        }
    }
    Ok(light)
}

fn decode_ambient(package: &Package, export_index: usize) -> Result<AmbientLight> {
    let mut reader = package.export_reader(export_index)?;
    skip_object_stack(package, export_index, &mut reader)?;
    let mut ambient = AmbientLight::default();
    while let Some(property) = reader.next_property()? {
        let name = reader.summary().name(property.name);
        let mut value = reader.property_reader(&property);
        match (name, property.kind) {
            ("AmbientBrightness", PropertyKind::Byte) => {
                ambient.brightness = value.read_u8()?;
            }
            ("AmbientHue", PropertyKind::Byte) => ambient.hue = value.read_u8()?,
            ("AmbientSaturation", PropertyKind::Byte) => {
                ambient.saturation = value.read_u8()?;
            }
            _ => {}
        }
    }
    Ok(ambient)
}

fn blur_shadow_bits(bits: &[u8], width: usize, height: usize) -> Vec<f32> {
    let pitch = width.div_ceil(8);
    let mut source = vec![0.0; width * height];
    for y in 0..height {
        for x in 0..width {
            source[y * width + x] = f32::from(bits[y * pitch + x / 8] & (1 << (x & 7)) != 0);
        }
    }
    const WEIGHTS: [f32; 9] = [0.125, 0.25, 0.125, 0.25, 0.5, 0.25, 0.125, 0.25, 0.125];
    let mut result = vec![0.0; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut value = 0.0;
            for offset_y in -1..=1 {
                let sample_y = y.saturating_add_signed(offset_y).min(height - 1);
                for offset_x in -1..=1 {
                    let sample_x = x.saturating_add_signed(offset_x).min(width - 1);
                    let weight = WEIGHTS[((offset_y + 1) * 3 + offset_x + 1) as usize];
                    value += source[sample_y * width + sample_x] * weight;
                }
            }
            result[y * width + x] = value;
        }
    }
    result
}

fn add_light(
    pixels: &mut [Vec3],
    locations: &[Vec3],
    normal: Vec3,
    shadow: &[f32],
    light: LightActor,
) {
    let radius = (f32::from(light.radius) + 1.0) * 25.0;
    let radius_squared = radius * radius;
    let color = hsb_to_rgb(light.hue, light.saturation, light.brightness);
    let spot_direction = spotlight_direction(light.rotation);
    for ((pixel, point), &shadow) in pixels.iter_mut().zip(locations).zip(shadow) {
        let direction = light.location - *point;
        let distance_squared = direction.length_squared();
        let distance = distance_squared.sqrt();
        let illumination = match light.effect {
            13 => shadow * (1.0 - distance / radius).max(0.0),
            14 => {
                let normalized = distance / radius;
                shadow
                    * if (0.8..1.0).contains(&normalized) {
                        1.0 - 10.0 * (normalized - 0.9).abs()
                    } else {
                        0.0
                    }
            }
            17 => {
                let planar = direction.x * direction.x + direction.y * direction.y;
                shadow * (1.0 - planar / radius_squared).max(0.0)
            }
            8 | 12 => {
                let normalized_distance = distance_squared / radius_squared;
                if normalized_distance >= 1.0 || light.cone == 0 || distance == 0.0 {
                    0.0
                } else {
                    let outer = 1.0 - f32::from(light.cone) / 255.0;
                    let cosine = (direction / distance).dot(spot_direction);
                    let spot = (1.0 - ((1.0 - cosine) / (1.0 - outer)).min(1.0)).max(0.0);
                    shadow
                        * distance_falloff(normalized_distance)
                        * (direction / distance).dot(normal).abs()
                        * spot
                        * spot
                }
            }
            4 => 0.0,
            _ if distance_squared < radius_squared && distance != 0.0 => {
                shadow
                    * distance_falloff(distance_squared / radius_squared)
                    * (direction / distance).dot(normal).abs()
            }
            _ => 0.0,
        };
        *pixel += (color * illumination).min(Vec3::ONE);
    }
}

fn spotlight_direction(rotation: Rotator) -> Vec3 {
    let radians = rotation.radians();
    let (sin_pitch, cos_pitch) = radians.x.sin_cos();
    let (sin_yaw, cos_yaw) = radians.y.sin_cos();
    Vec3::new(-cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch)
}

fn distance_falloff(distance_squared: f32) -> f32 {
    let value = (distance_squared + 0.0001).sqrt();
    ((1.0 + 2.0 * value.powi(3) - 3.0 * value.powi(2)) / value).min(1.0)
}

fn hsb_to_rgb(hue: u8, saturation: u8, brightness: u8) -> Vec3 {
    let value = 6.512_735 * f32::from(brightness).sqrt();
    if saturation >= 250 {
        return Vec3::splat(value / 255.0);
    }
    if brightness == 0 {
        return Vec3::ZERO;
    }
    let mut saturation = f32::from(saturation) / 2.5;
    if saturation > 32.0 {
        saturation += 2.0;
    }
    let sector = f32::from(hue) / 85.0;
    let fraction = sector.fract();
    let low = saturation * value / 104.0;
    let falling = (1.0 - fraction) * value + low * fraction;
    let rising = fraction * value + low * (1.0 - fraction);
    let rgb = if hue < 85 {
        Vec3::new(falling, rising, low)
    } else if hue < 170 {
        Vec3::new(low, falling, rising)
    } else {
        Vec3::new(rising, low, falling)
    };
    rgb / 255.0
}

fn lightmap_image(width: usize, height: usize, pixels: &[Vec3]) -> LightmapImage {
    let mut rgba = Vec::with_capacity(width * height * 4);
    for color in pixels {
        for channel in color.to_array() {
            rgba.push((channel.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
        rgba.push(255);
    }
    LightmapImage {
        width: width as u32,
        height: height as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_mask_uses_little_endian_bits_and_ue1_blur() {
        assert_eq!(blur_shadow_bits(&[0b0000_0001], 1, 1), vec![2.0]);
        assert_eq!(blur_shadow_bits(&[0b0000_0010], 1, 1), vec![0.0]);
    }

    #[test]
    fn default_unreal_saturation_produces_grey_light() {
        let color = hsb_to_rgb(123, 255, 64);
        assert_eq!(color.x, color.y);
        assert_eq!(color.y, color.z);
        assert!((color.x - 0.204_321_1).abs() < 0.000_001);
    }

    #[test]
    fn vertex_lighting_matches_ue1_ambient_and_radial_falloff() {
        let lighting = ActorVertexLighting {
            ambient: Vec3::splat(0.1),
            scale_glow: 1.0,
            lights: vec![VertexLight {
                location: Vec3::new(5.0, 0.0, 0.0),
                color: Vec3::splat(0.2),
                radius: 10.0,
                effect: 0,
            }],
        };
        assert_eq!(lighting.color(Vec3::ZERO, Vec3::X, false), Vec3::splat(0.5));
        assert_eq!(lighting.color(Vec3::ZERO, Vec3::X, true), Vec3::splat(1.2));
    }

    #[test]
    fn light_trace_intersects_two_sided_triangle() {
        let triangle = [
            Vec3::new(2.0, -1.0, -1.0),
            Vec3::new(2.0, 1.0, -1.0),
            Vec3::new(2.0, 0.0, 1.0),
        ];
        assert_eq!(ray_triangle(Vec3::ZERO, Vec3::X, 10.0, triangle), Some(2.0));
        assert_eq!(
            ray_triangle(Vec3::new(4.0, 0.0, 0.0), -Vec3::X, 10.0, triangle),
            Some(2.0)
        );
    }
}
