use glam::Vec3;

/// Converts Unreal's left-handed X-forward/Y-right/Z-up coordinates into the
/// renderer's right-handed X-right/Y-up/-Z-forward coordinates.
#[inline]
pub fn unreal_to_render(position: Vec3) -> Vec3 {
    Vec3::new(position.y, position.z, -position.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unreal_axes_once() {
        assert_eq!(unreal_to_render(Vec3::X), -Vec3::Z);
        assert_eq!(unreal_to_render(Vec3::Y), Vec3::X);
        assert_eq!(unreal_to_render(Vec3::Z), Vec3::Y);
    }
}
