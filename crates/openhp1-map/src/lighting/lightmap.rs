use std::collections::HashMap;

use glam::Vec3;
use openhp1_package::{ObjectReference, Package};

use crate::{Error, Model, Result, decode::index};

use super::{
    AmbientLight, LightActor, decode_ambient, decode_level_ambient, decode_light, hsb_to_rgb,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightmapImage {
    pub width: u32,
    pub height: u32,
    /// Linear RGB light values stored in normalized bytes. The renderer
    /// applies UE1's conventional 2x lightmap modulation.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightVisibility {
    pub width: u32,
    pub height: u32,
    /// UE1's blurred one-bit shadow mask encoded over the 0..=2 range.
    pub values: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredLight {
    pub export_index: usize,
    pub location: Vec3,
    pub rotation: crate::Rotator,
    pub effect: u8,
    pub brightness: u8,
    pub hue: u8,
    pub saturation: u8,
    pub radius: u8,
    pub cone: u8,
    pub dark: bool,
    pub volume_brightness: u8,
    pub volume_fog: u8,
    pub volume_radius: u8,
    pub visibility: LightVisibility,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredLightmap {
    pub ambient: Vec3,
    pub lights: Vec<AuthoredLight>,
}

struct LightmapContext {
    surfaces: Vec<Option<usize>>,
    zones: Vec<Option<usize>>,
    level_ambient: AmbientLight,
}

impl Model {
    /// Preserves the authored inputs needed to light BSP surfaces at render time.
    pub fn authored_lightmaps(&self, package: &Package) -> Result<Vec<AuthoredLightmap>> {
        let context = self.lightmap_context(package)?;
        let mut ambient_cache = HashMap::new();
        let mut light_cache = HashMap::new();
        let mut result = Vec::with_capacity(self.light_maps.len());
        for lightmap_index in 0..self.light_maps.len() {
            let ambient = self.lightmap_ambient(
                package,
                context.zones[lightmap_index],
                context.level_ambient,
                &mut ambient_cache,
            )?;
            let mut lights = Vec::new();
            if context.surfaces[lightmap_index].is_some() {
                let (width, height) =
                    lightmap_dimensions(lightmap_index, self.light_maps[lightmap_index].clamp)?;
                for (export_index, light, shadow) in self.lightmap_lights(
                    package,
                    lightmap_index,
                    (width, height),
                    &HashMap::new(),
                    &mut light_cache,
                    true,
                )? {
                    lights.push(AuthoredLight {
                        export_index,
                        location: light.location,
                        rotation: light.rotation,
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
                        visibility: LightVisibility {
                            width: width as u32,
                            height: height as u32,
                            values: shadow
                                .into_iter()
                                .map(|value| (value.clamp(0.0, 2.0) * 127.5 + 0.5) as u8)
                                .collect(),
                        },
                    });
                }
            }
            result.push(AuthoredLightmap {
                ambient: hsb_to_rgb(ambient.hue, ambient.saturation, ambient.brightness),
                lights,
            });
        }
        Ok(result)
    }

    /// Reconstructs UE1's static lightmaps from the serialized shadow masks,
    /// light actors, and zone ambient colors.
    pub fn lightmap_images(&self, package: &Package) -> Result<Vec<LightmapImage>> {
        Ok(self
            .build_lightmap_images(package, &HashMap::new(), None)?
            .into_iter()
            .map(|(_, image)| image)
            .collect())
    }

    /// Rebuilds only lightmaps whose serialized light list contains the
    /// changed actor export.
    pub fn relight_lightmaps(
        &self,
        package: &Package,
        light_export: usize,
        brightnesses: &HashMap<usize, u8>,
    ) -> Result<Vec<(usize, LightmapImage)>> {
        self.build_lightmap_images(package, brightnesses, Some(light_export))
    }

    fn build_lightmap_images(
        &self,
        package: &Package,
        brightnesses: &HashMap<usize, u8>,
        changed_light: Option<usize>,
    ) -> Result<Vec<(usize, LightmapImage)>> {
        let context = self.lightmap_context(package)?;
        let mut ambient_cache = HashMap::new();
        let mut light_cache = HashMap::new();
        let mut images = Vec::with_capacity(self.light_maps.len());
        for lightmap_index in 0..self.light_maps.len() {
            if let Some(light_export) = changed_light
                && !self.lightmap_uses_light(lightmap_index, light_export)?
            {
                continue;
            }
            let ambient = self.lightmap_ambient(
                package,
                context.zones[lightmap_index],
                context.level_ambient,
                &mut ambient_cache,
            )?;
            images.push((
                lightmap_index,
                self.build_lightmap(
                    package,
                    lightmap_index,
                    context.surfaces[lightmap_index],
                    ambient,
                    brightnesses,
                    &mut light_cache,
                )?,
            ));
        }
        Ok(images)
    }

    fn lightmap_context(&self, package: &Package) -> Result<LightmapContext> {
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
            let zone = node.zones[1];
            if let Ok(zone) = usize::try_from(zone)
                && zone < self.zones.len()
            {
                lightmap_zones[lightmap_index].get_or_insert(zone);
            }
        }
        Ok(LightmapContext {
            surfaces: lightmap_surfaces,
            zones: lightmap_zones,
            level_ambient: decode_level_ambient(package)?,
        })
    }

    fn lightmap_ambient(
        &self,
        package: &Package,
        zone: Option<usize>,
        level_ambient: AmbientLight,
        cache: &mut HashMap<usize, AmbientLight>,
    ) -> Result<AmbientLight> {
        match zone
            .and_then(|zone| self.zones.get(zone))
            .map(|zone| zone.actor)
        {
            Some(ObjectReference::Export(export_index)) => {
                if let Some(ambient) = cache.get(&export_index) {
                    Ok(*ambient)
                } else {
                    let ambient = decode_ambient(package, export_index)?;
                    cache.insert(export_index, ambient);
                    Ok(ambient)
                }
            }
            _ => Ok(level_ambient),
        }
    }

    fn lightmap_uses_light(&self, lightmap_index: usize, light_export: usize) -> Result<bool> {
        let lightmap = &self.light_maps[lightmap_index];
        let Ok(mut list_index) = usize::try_from(lightmap.light_actors) else {
            return if lightmap.light_actors == -1 {
                Ok(false)
            } else {
                Err(Error::InvalidLightList {
                    index: lightmap_index,
                    offset: lightmap.light_actors,
                    length: self.lights.len(),
                })
            };
        };
        loop {
            let reference = *self.lights.get(list_index).ok_or(Error::InvalidLightList {
                index: lightmap_index,
                offset: lightmap.light_actors,
                length: self.lights.len(),
            })?;
            list_index += 1;
            match reference {
                ObjectReference::None => return Ok(false),
                ObjectReference::Export(export_index) if export_index == light_export => {
                    return Ok(true);
                }
                _ => {}
            }
        }
    }

    fn build_lightmap(
        &self,
        package: &Package,
        lightmap_index: usize,
        surface_index: Option<usize>,
        ambient: AmbientLight,
        brightnesses: &HashMap<usize, u8>,
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

        for (_, light, shadow) in self.lightmap_lights(
            package,
            lightmap_index,
            (width, height),
            brightnesses,
            light_cache,
            false,
        )? {
            add_light(&mut pixels, &locations, normal, &shadow, light);
        }
        Ok(lightmap_image(width, height, &pixels))
    }

    fn lightmap_lights(
        &self,
        package: &Package,
        lightmap_index: usize,
        dimensions: (usize, usize),
        brightnesses: &HashMap<usize, u8>,
        light_cache: &mut HashMap<usize, LightActor>,
        include_zero_brightness: bool,
    ) -> Result<Vec<(usize, LightActor, Vec<f32>)>> {
        let (width, height) = dimensions;
        let lightmap = &self.light_maps[lightmap_index];
        let Ok(mut list_index) = usize::try_from(lightmap.light_actors) else {
            return if lightmap.light_actors == -1 {
                Ok(Vec::new())
            } else {
                Err(Error::InvalidLightList {
                    index: lightmap_index,
                    offset: lightmap.light_actors,
                    length: self.lights.len(),
                })
            };
        };
        let mut result = Vec::new();
        let mut shadow_index = 0;
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
                let mut light = if let Some(light) = light_cache.get(&export_index) {
                    *light
                } else {
                    let light = decode_light(package, export_index)?;
                    light_cache.insert(export_index, light);
                    light
                };
                if let Some(&brightness) = brightnesses.get(&export_index) {
                    light.brightness = brightness;
                }
                if light.light_type != 0 && (include_zero_brightness || light.brightness != 0) {
                    result.push((
                        export_index,
                        light,
                        self.shadowmap(lightmap_index, shadow_index, width, height)?,
                    ));
                }
            }
            shadow_index += 1;
        }
        Ok(result)
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
        let contribution = (color * illumination).min(Vec3::ONE);
        if light.dark {
            *pixel = (*pixel - contribution).max(Vec3::ZERO);
        } else {
            *pixel += contribution;
        }
    }
}

fn spotlight_direction(rotation: crate::Rotator) -> Vec3 {
    let radians = rotation.radians();
    let (sin_pitch, cos_pitch) = radians.x.sin_cos();
    let (sin_yaw, cos_yaw) = radians.y.sin_cos();
    Vec3::new(-cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch)
}

fn distance_falloff(distance_squared: f32) -> f32 {
    let value = (distance_squared + 0.0001).sqrt();
    ((1.0 + 2.0 * value.powi(3) - 3.0 * value.powi(2)) / value).min(1.0)
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
    fn zero_brightness_removes_the_authored_lightmap_contribution() {
        let mut light = LightActor {
            location: Vec3::new(25.0, 0.0, 0.0),
            brightness: 64,
            ..Default::default()
        };
        let locations = [Vec3::ZERO];
        let shadow = [1.0];
        let mut authored = [Vec3::ZERO];
        add_light(&mut authored, &locations, Vec3::X, &shadow, light);
        assert!(authored[0].length_squared() > 0.0);

        light.brightness = 0;
        let mut dark = [Vec3::ZERO];
        add_light(&mut dark, &locations, Vec3::X, &shadow, light);
        assert_eq!(dark[0], Vec3::ZERO);
    }

    #[test]
    fn dark_light_subtracts_and_clamps_the_authored_contribution() {
        let light = LightActor {
            location: Vec3::new(25.0, 0.0, 0.0),
            brightness: 64,
            dark: true,
            ..Default::default()
        };
        let mut pixels = [Vec3::splat(0.1)];
        add_light(&mut pixels, &[Vec3::ZERO], Vec3::X, &[1.0], light);
        assert_eq!(pixels[0], Vec3::ZERO);
    }
}
