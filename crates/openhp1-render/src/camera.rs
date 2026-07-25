use glam::{Mat4, Vec3};

/// A free camera expressed in renderer coordinates.
#[derive(Clone, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
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

    pub(crate) fn view_projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.vertical_fov, aspect, self.near, self.far)
            * Mat4::look_to_rh(self.position, self.forward(), Vec3::Y)
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
