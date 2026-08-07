const SSAO_KERNEL = array<vec2<f32>, 16>(
    vec2(0.176777, 0.000000),
    vec2(-0.225772, 0.206826),
    vec2(0.034558, -0.393771),
    vec2(0.284571, 0.371173),
    vec2(-0.522223, -0.092374),
    vec2(0.494695, -0.314685),
    vec2(-0.165466, 0.615525),
    vec2(-0.315561, -0.607594),
    vec2(0.684642, 0.250030),
    vec2(-0.712256, 0.294009),
    vec2(0.343354, -0.733729),
    vec2(0.253730, 0.808932),
    vec2(-0.764746, -0.443186),
    vec2(0.897134, -0.197232),
    vec2(-0.547507, 0.778772),
    vec2(-0.126487, -0.976090),
);

@fragment
fn fragment_ssao(input: FullscreenVertex) -> AoOutput {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), dimensions);
    if textureLoad(scene_depth, pixel, 0) >= 0.99999 {
        return ao_output(1.0, vec4(0.0));
    }

    let depths = depth_neighborhood(pixel);
    let edges = calculate_edges(depths[0], depths[1], depths[2], depths[3], depths[4]);
    let center = view_position(pixel, depths[0]);
    let normal = depth_normal(pixel, depths, edges);
    let focal_pixels = 0.5 / (settings.inverse_viewport.y * settings.tan_half_fov.y);
    let radius_pixels = clamp(settings.effect_radius * focal_pixels / depths[0], 4.0, 64.0);

    var occlusion = 0.0;
    for (var index = 0u; index < 16u; index += 1u) {
        let offset = vec2<i32>(round(SSAO_KERNEL[index] * radius_pixels));
        let sample_pixel = clamp_pixel(pixel + offset, dimensions);
        let sample_depth = center_depth(sample_pixel);
        let delta = view_position(sample_pixel, sample_depth) - center;
        let distance = length(delta);
        if distance > 0.001 {
            let horizon = max(dot(normal, delta / distance) - 0.05, 0.0);
            let range = 1.0 - smoothstep(settings.effect_radius * 0.2, settings.effect_radius, distance);
            occlusion += horizon * range;
        }
    }
    return ao_output(clamp(1.0 - 2.5 * occlusion / 16.0, 0.35, 1.0), edges);
}
