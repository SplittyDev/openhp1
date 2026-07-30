use glam::{Mat4, Quat, Vec3};
use openhp1_scene::SkyZone;

use crate::unreal_to_render;

/// A free camera expressed in renderer coordinates.
#[derive(Clone, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub vertical_fov: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn looking_at(position: Vec3, target: Vec3, far: f32) -> Self {
        let direction = (target - position).normalize_or_zero();
        Self {
            position,
            yaw: direction.x.atan2(-direction.z),
            pitch: direction.y.asin(),
            roll: 0.0,
            vertical_fov: 60_f32.to_radians(),
            near: 1.0,
            far,
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize_or_zero()
    }

    fn up(&self) -> Vec3 {
        let up = self.right().cross(self.forward()).normalize_or_zero();
        Quat::from_axis_angle(self.forward(), self.roll) * up
    }

    pub(crate) fn for_sky_zone(&self, sky: SkyZone) -> Self {
        let rotation = sky.rotation.radians();
        Self {
            position: unreal_to_render(sky.location),
            yaw: self.yaw - rotation.y,
            pitch: self.pitch - rotation.x,
            roll: self.roll - rotation.z,
            ..self.clone()
        }
    }

    pub(crate) fn view_projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.vertical_fov, aspect, self.near, self.far) * self.view()
    }

    pub(crate) fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.forward(), self.up())
    }
}

/// Bounds used to choose a useful initial camera and movement speed.
#[derive(Clone, Copy, Debug)]
pub struct SceneBounds {
    pub minimum: Vec3,
    pub maximum: Vec3,
}

impl SceneBounds {
    pub fn center(self) -> Vec3 {
        (self.minimum + self.maximum) * 0.5
    }

    pub fn radius(self) -> f32 {
        (self.maximum - self.minimum).length() * 0.5
    }
}

#[cfg(test)]
mod tests {
    use openhp1_scene::{Rotator, SkyZone};

    use super::*;

    #[test]
    fn sky_camera_uses_zone_location_and_relative_rotation() {
        let camera = Camera::looking_at(Vec3::ZERO, -Vec3::Z, 1000.0);
        let sky = camera.for_sky_zone(SkyZone {
            location: Vec3::new(10.0, 20.0, 30.0),
            rotation: Rotator {
                pitch: 0,
                yaw: 16_384,
                roll: 0,
            },
        });
        assert_eq!(sky.position, Vec3::new(20.0, 30.0, -10.0));
        assert!((sky.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.000_001);
    }
}
