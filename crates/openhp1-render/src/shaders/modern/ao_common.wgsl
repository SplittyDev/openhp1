struct AoSettings {
    viewport_size: vec2<u32>,
    inverse_viewport: vec2<f32>,
    near_far: vec2<f32>,
    tan_half_fov: vec2<f32>,
    effect_radius: f32,
    visibility_scale: f32,
    depth_mip_count: u32,
    _padding: u32,
};

fn clamp_pixel(pixel: vec2<i32>, dimensions: vec2<i32>) -> vec2<i32> {
    return clamp(pixel, vec2(0), dimensions - vec2(1));
}

fn linear_depth(depth: f32) -> f32 {
    let near_plane = max(settings.near_far.x, 0.0001);
    let far_plane = max(settings.near_far.y, near_plane + 0.0001);
    return near_plane * far_plane
        / max(far_plane - depth * (far_plane - near_plane), 0.0001);
}

fn view_position(pixel: vec2<i32>, distance: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2(0.5)) * settings.inverse_viewport;
    return vec3(
        (2.0 * uv.x - 1.0) * distance * settings.tan_half_fov.x,
        (1.0 - 2.0 * uv.y) * distance * settings.tan_half_fov.y,
        distance,
    );
}
