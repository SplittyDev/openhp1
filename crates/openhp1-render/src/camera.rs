use glam::{Mat4, Quat, Vec3};
use openhp1_scene::{SkyZone, WarpCoordinates};

use crate::{render_to_unreal, unreal_to_render};

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

    pub(crate) fn up(&self) -> Vec3 {
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

    pub(crate) fn reflected_view(&self, plane_point: Vec3, plane_normal: Vec3) -> (Vec3, Mat4) {
        let normal = plane_normal.normalize_or_zero();
        let reflection = Mat4::from_translation(plane_point)
            * Mat4::from_cols(
                (Vec3::X - 2.0 * normal.x * normal).extend(0.0),
                (Vec3::Y - 2.0 * normal.y * normal).extend(0.0),
                (Vec3::Z - 2.0 * normal.z * normal).extend(0.0),
                Vec3::ZERO.extend(1.0),
            )
            * Mat4::from_translation(-plane_point);
        (
            reflection.transform_point3(self.position),
            self.view() * reflection,
        )
    }

    pub(crate) fn view_projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.vertical_fov, aspect, self.near, self.far) * self.view()
    }

    pub(crate) fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.forward(), self.up())
    }
}

pub(crate) fn warp_view(
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    world_to_view: Mat4,
    source: WarpCoordinates,
    destination: WarpCoordinates,
) -> (Vec3, Vec3, Vec3, Mat4) {
    let position = unreal_to_render(source.transform_to(destination, render_to_unreal(position)));
    let forward =
        unreal_to_render(source.transform_vector_to(destination, render_to_unreal(forward)));
    let up = unreal_to_render(source.transform_vector_to(destination, render_to_unreal(up)));
    let warp_to_world = Mat4::from_cols(
        unreal_to_render(source.transform_vector_to(destination, render_to_unreal(Vec3::X)))
            .extend(0.0),
        unreal_to_render(source.transform_vector_to(destination, render_to_unreal(Vec3::Y)))
            .extend(0.0),
        unreal_to_render(source.transform_vector_to(destination, render_to_unreal(Vec3::Z)))
            .extend(0.0),
        unreal_to_render(source.transform_to(destination, Vec3::ZERO)).extend(1.0),
    );
    (
        position,
        forward,
        up,
        world_to_view * warp_to_world.inverse(),
    )
}

pub(crate) fn reflected_view(
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    world_to_view: Mat4,
    plane_point: Vec3,
    plane_normal: Vec3,
) -> (Vec3, Vec3, Vec3, Mat4) {
    let normal = plane_normal.normalize_or_zero();
    let reflection = Mat4::from_translation(plane_point)
        * Mat4::from_cols(
            (Vec3::X - 2.0 * normal.x * normal).extend(0.0),
            (Vec3::Y - 2.0 * normal.y * normal).extend(0.0),
            (Vec3::Z - 2.0 * normal.z * normal).extend(0.0),
            Vec3::ZERO.extend(1.0),
        )
        * Mat4::from_translation(-plane_point);
    (
        reflection.transform_point3(position),
        reflection.transform_vector3(forward),
        reflection.transform_vector3(up),
        world_to_view * reflection,
    )
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

    #[test]
    fn mirror_view_reflects_world_across_the_authored_plane() {
        let camera = Camera::looking_at(Vec3::new(2.0, 3.0, 4.0), Vec3::ZERO, 1000.0);
        let normal = Vec3::new(1.0, 2.0, 0.5).normalize();
        let point = Vec3::new(-3.0, 1.0, 2.0);
        let (position, view) = camera.reflected_view(point, normal);

        assert!(position.abs_diff_eq(
            camera.position - 2.0 * (camera.position - point).dot(normal) * normal,
            0.000_01,
        ));
        let original_view = camera.view();
        assert!(
            view.transform_point3(point)
                .abs_diff_eq(original_view.transform_point3(point), 0.000_01)
        );
        assert!(view.transform_point3(point + normal * 7.0).abs_diff_eq(
            original_view.transform_point3(point - normal * 7.0),
            0.000_01,
        ));
        assert!(view.determinant().is_sign_negative());
    }

    #[test]
    fn warp_view_moves_the_camera_between_authored_coordinate_frames() {
        let camera = Camera {
            position: unreal_to_render(Vec3::new(0.0, 10.0, 0.0)),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            roll: 0.0,
            vertical_fov: 1.0,
            near: 1.0,
            far: 1000.0,
        };
        let source = WarpCoordinates {
            origin: Vec3::ZERO,
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
        };
        let destination = WarpCoordinates {
            origin: Vec3::new(100.0, 200.0, 0.0),
            axes: [-Vec3::X, -Vec3::Y, Vec3::Z],
        };

        let (position, _, _, view) = warp_view(
            camera.position,
            camera.forward(),
            camera.up(),
            camera.view(),
            source,
            destination,
        );

        assert!(position.abs_diff_eq(unreal_to_render(Vec3::new(100.0, 190.0, 0.0)), 0.000_01));
        assert!(
            view.transform_point3(position)
                .abs_diff_eq(Vec3::ZERO, 0.000_01)
        );
        assert!(
            view.transform_point3(position + Vec3::X)
                .abs_diff_eq(-Vec3::Z, 0.000_01)
        );
        assert!(view.determinant().is_sign_positive());
    }
}
