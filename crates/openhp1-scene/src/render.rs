use std::{collections::HashMap, ops::Range, sync::Arc};

use glam::{Mat3, Vec3};
use openhp1_map::{LightVisibility, LightmapImage, SkyZone, TriangleMesh, hsb_to_rgb};
use openhp1_physics::BspCollision;

use crate::{Rotator, unreal_to_render};

#[derive(Clone, Debug, Default)]
pub struct CoronaVisibility(Option<Arc<BspCollision>>);

impl CoronaVisibility {
    pub(crate) fn new(collision: Arc<BspCollision>) -> Self {
        Self(Some(collision))
    }

    pub fn leaf_at(&self, point: Vec3) -> Option<usize> {
        self.0
            .as_ref()?
            .point_region(point)
            .and_then(|region| usize::try_from(region.leaf).ok())
    }

    pub fn line_clear(&self, start: Vec3, end: Vec3) -> bool {
        self.0
            .as_ref()
            .is_none_or(|collision| collision.single_line_clear(start, end))
    }
}

#[derive(Clone, Debug)]
pub struct TextureImage {
    /// Physical pixel width uploaded to the GPU.
    pub width: u32,
    /// Physical pixel height uploaded to the GPU.
    pub height: u32,
    /// Authored UE texel width used for UV normalization and sprite sizing.
    pub logical_width: u32,
    /// Authored UE texel height used for UV normalization and sprite sizing.
    pub logical_height: u32,
    pub rgba: Vec<u8>,
    /// Authored levels after mip zero, in descending size order.
    pub mips: Vec<TextureMipImage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextureMipImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransmissionMask {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u8>,
}

impl TextureImage {
    pub fn logical_dimensions(&self) -> [u32; 2] {
        [self.logical_width, self.logical_height]
    }

    pub fn byte_len(&self) -> usize {
        self.rgba.len() + self.mips.iter().map(|mip| mip.rgba.len()).sum::<usize>()
    }

    pub fn mip_level_count(&self) -> u32 {
        u32::try_from(self.mips.len() + 1).unwrap_or(u32::MAX)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Corona {
    pub actor_index: usize,
    /// Authored UE-space light location.
    pub location: Vec3,
    /// Index into [`RenderScene::textures`].
    pub texture: Option<usize>,
    pub draw_scale: f32,
    pub color: Vec3,
    /// Native order within every serialized static actor chain containing this
    /// corona, keyed by convex-leaf index.
    pub static_leaf_orders: Vec<(usize, usize)>,
    /// Dynamic-light sphere radius used to rebuild `LeafLights`; `None` means
    /// this actor is not eligible for the dynamic-light path.
    pub dynamic_light_radius: Option<f32>,
    pub dynamic_admission_radius: Option<f32>,
    pub dynamic_leaves: Vec<usize>,
    pub light_brightness: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderLight {
    pub actor_index: usize,
    /// Texture used by a visible authored light sprite, such as a flame.
    pub source_texture: Option<usize>,
    pub location: Vec3,
    pub direction: Vec3,
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

impl RenderLight {
    pub fn color(&self) -> Vec3 {
        hsb_to_rgb(self.hue, self.saturation, self.brightness)
    }

    pub fn source_color(&self) -> Vec3 {
        hsb_to_rgb(self.hue, self.saturation, 255)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderLightmap {
    pub ambient: Vec3,
    pub lights: Vec<RenderLight>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpCoordinates {
    /// Authored UE-space origin.
    pub origin: Vec3,
    /// Authored UE-space coordinate axes.
    pub axes: [Vec3; 3],
}

impl WarpCoordinates {
    pub fn transform_to(self, destination: Self, position: Vec3) -> Vec3 {
        destination.rotation().transpose() * (self.rotation() * (position - self.origin))
            + destination.origin
    }

    pub fn transform_vector_to(self, destination: Self, vector: Vec3) -> Vec3 {
        destination.rotation().transpose() * (self.rotation() * vector)
    }

    fn rotation(self) -> Mat3 {
        Mat3::from_cols(self.axes[0], self.axes[1], self.axes[2])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpPortal {
    pub surface: usize,
    pub source_actor: usize,
    /// Authored BSP plane; positive space selects Zone0 and negative space Zone1.
    pub plane: [f32; 4],
    /// The source WarpZoneInfo occupies Zone0 when true, otherwise Zone1.
    pub source_on_positive_side: bool,
    pub source: WarpCoordinates,
    pub destination_actor: Option<usize>,
    pub destination: Option<WarpCoordinates>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorSubmission {
    pub actor_index: usize,
    /// Index range in [`RenderScene::mesh`], retained as one actor record.
    pub indices: Range<usize>,
    /// Retail's device path defers Style 3 and Opacity < 1 actors until after
    /// BSP list 2. Other actor styles remain in the ordinary actor pass.
    pub translucent_pass: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfaceMode {
    #[default]
    Opaque,
    Translucent,
    Modulated,
    /// HP1 actor opacity using source-alpha blending.
    AlphaBlended,
    /// Writes depth without changing the color target.
    DepthOnly,
    /// Samples the rendered sky zone in screen space.
    Backdrop,
    /// Not submitted to the GPU.
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMaterial {
    pub texture: Option<usize>,
    /// `UTexture::MacroTexture`, sampled independently of the base texture.
    pub macro_texture: Option<usize>,
    /// `UTexture::DetailTexture`, sampled independently of the base texture.
    pub detail_texture: Option<usize>,
    /// A generated BSP FogMap suppresses only the detail attachment.
    pub fog_map_attached: bool,
    /// Native `PF_Portal` suppresses the detail attachment.
    pub portal: bool,
    /// Native `FTextureInfo` scale for the macro attachment.
    pub macro_draw_scale: f32,
    /// Native `FTextureInfo` scale for the detail attachment.
    pub detail_draw_scale: f32,
    /// Authored BSP pan already included in the base mesh coordinates. Native
    /// attachment locks leave their own PanU/PanV at zero, so it is removed.
    pub bsp_texture_pan: [f32; 2],
    pub mode: SurfaceMode,
    /// Discard palette index zero. This remains independent of the blend mode
    /// because UE1 permits masked modulated surfaces.
    pub masked: bool,
    pub two_sided: bool,
    /// Select point minification/magnification for the base texture.
    pub no_smooth: bool,
    pub unlit: bool,
    /// Authored sky or corpus-identified window surface that can admit a
    /// directional volumetric shaft.
    pub volumetric_source: bool,
    /// Render the scene reflected across this BSP surface.
    pub mirror: bool,
    /// Derive texture coordinates from the reflected view direction.
    pub environment_map: bool,
    /// Base-mip `FTextureInfo` multiplier used by mesh environment UVs.
    pub texture_draw_scale: f32,
    /// HP1-specific multiplier for actor source color and alpha.
    pub opacity: f32,
    /// Apply UE1's small sinusoidal BSP texture-coordinate motion.
    pub small_wavy: bool,
}

impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            texture: None,
            macro_texture: None,
            detail_texture: None,
            fog_map_attached: false,
            portal: false,
            macro_draw_scale: 1.0,
            detail_draw_scale: 1.0,
            bsp_texture_pan: [0.0; 2],
            mode: SurfaceMode::Opaque,
            masked: false,
            two_sided: false,
            no_smooth: false,
            unlit: false,
            volumetric_source: false,
            mirror: false,
            environment_map: false,
            texture_draw_scale: 1.0,
            opacity: 1.0,
            small_wavy: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderScene {
    pub mesh: TriangleMesh,
    pub textures: Vec<TextureImage>,
    pub lightmaps: Vec<LightmapImage>,
    /// Authored UE1 lights and visibility masks evaluated by the Modern renderer.
    pub realtime_lightmaps: Vec<RenderLightmap>,
    pub coronas: Vec<Corona>,
    /// World BSP collision used by the original actor-to-camera corona trace.
    pub corona_visibility: CoronaVisibility,
    /// Dynamic actor geometry records used by the shared submission planner.
    pub actor_submissions: Vec<ActorSubmission>,
    /// Material for each surface. Missing visible textures use the renderer's
    /// checkerboard; the scene loader hides untextured actor-mesh faces.
    pub surface_materials: Vec<SurfaceMaterial>,
    /// Derived stained-glass transmission masks keyed by base texture index.
    pub transmission_masks: HashMap<usize, TransmissionMask>,
    /// Authored `PF_Portal` BSP surfaces backed by `WarpZoneInfo` actors.
    pub warp_portals: Vec<WarpPortal>,
    /// A fixed UE1 sky-box viewpoint rendered behind the main scene.
    pub sky_zone: Option<SkyZone>,
}

impl RenderScene {
    pub(crate) fn set_light_brightness(&mut self, actor_index: usize, brightness: u8) -> bool {
        let mut changed = false;
        for corona in self
            .coronas
            .iter_mut()
            .filter(|corona| corona.actor_index == actor_index)
        {
            changed |= corona.light_brightness != brightness;
            corona.light_brightness = brightness;
        }
        for light in self
            .realtime_lightmaps
            .iter_mut()
            .flat_map(|lightmap| &mut lightmap.lights)
            .filter(|light| light.actor_index == actor_index)
        {
            changed |= light.brightness != brightness;
            light.brightness = brightness;
        }
        changed
    }

    pub(crate) fn set_light_location(&mut self, actor_index: usize, location: Vec3) -> bool {
        let location = unreal_to_render(location);
        let mut changed = false;
        for light in self
            .realtime_lightmaps
            .iter_mut()
            .flat_map(|lightmap| &mut lightmap.lights)
            .filter(|light| light.actor_index == actor_index)
        {
            changed |= light.location != location;
            light.location = location;
        }
        changed
    }

    pub(crate) fn set_light_rotation(&mut self, actor_index: usize, rotation: Rotator) -> bool {
        let direction = unreal_to_render(light_direction(rotation)).normalize_or_zero();
        let mut changed = false;
        for light in self
            .realtime_lightmaps
            .iter_mut()
            .flat_map(|lightmap| &mut lightmap.lights)
            .filter(|light| light.actor_index == actor_index)
        {
            changed |= light.direction != direction;
            light.direction = direction;
        }
        changed
    }
}

pub(crate) fn light_direction(rotation: Rotator) -> Vec3 {
    let radians = rotation.radians();
    let (sin_pitch, cos_pitch) = radians.x.sin_cos();
    let (sin_yaw, cos_yaw) = radians.y.sin_cos();
    Vec3::new(-cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_light_changes_update_every_surface_copy() {
        let light = RenderLight {
            actor_index: 7,
            source_texture: None,
            location: Vec3::ZERO,
            direction: Vec3::X,
            effect: 0,
            brightness: 64,
            hue: 0,
            saturation: 255,
            radius: 64,
            cone: 128,
            dark: false,
            volume_brightness: 64,
            volume_fog: 0,
            volume_radius: 0,
            visibility: LightVisibility {
                width: 1,
                height: 1,
                values: vec![255],
            },
        };
        let mut scene = RenderScene {
            mesh: TriangleMesh::default(),
            textures: Vec::new(),
            lightmaps: Vec::new(),
            realtime_lightmaps: vec![
                RenderLightmap {
                    ambient: Vec3::ZERO,
                    lights: vec![light.clone()],
                },
                RenderLightmap {
                    ambient: Vec3::ZERO,
                    lights: vec![light],
                },
            ],
            coronas: Vec::new(),
            corona_visibility: Default::default(),
            actor_submissions: Vec::new(),
            surface_materials: Vec::new(),
            transmission_masks: Default::default(),
            warp_portals: Vec::new(),
            sky_zone: None,
        };

        assert!(scene.set_light_brightness(7, 128));
        assert!(
            scene
                .realtime_lightmaps
                .iter()
                .all(|lightmap| lightmap.lights[0].brightness == 128)
        );
        assert!(!scene.set_light_brightness(7, 128));
        assert!(scene.set_light_location(7, Vec3::new(1.0, 2.0, 3.0)));
        assert!(scene.set_light_rotation(7, Rotator::default()));
        assert!(scene.realtime_lightmaps.iter().all(|lightmap| {
            lightmap.lights[0].location == Vec3::new(2.0, 3.0, -1.0)
                && lightmap.lights[0].direction == Vec3::Z
        }));
    }
}
