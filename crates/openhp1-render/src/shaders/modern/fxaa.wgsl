@group(0) @binding(0)
var color_texture: texture_2d<f32>;

@group(0) @binding(1)
var color_sampler: sampler;

fn fxaa_luma(color: vec3<f32>) -> f32 {
    return dot(color, vec3(0.299, 0.587, 0.114));
}

@fragment
fn fragment_fxaa(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(color_texture));
    let rgb_nw = textureSampleLevel(color_texture, color_sampler, input.uv + texel * vec2(-1.0, -1.0), 0.0).rgb;
    let rgb_ne = textureSampleLevel(color_texture, color_sampler, input.uv + texel * vec2(1.0, -1.0), 0.0).rgb;
    let rgb_sw = textureSampleLevel(color_texture, color_sampler, input.uv + texel * vec2(-1.0, 1.0), 0.0).rgb;
    let rgb_se = textureSampleLevel(color_texture, color_sampler, input.uv + texel * vec2(1.0, 1.0), 0.0).rgb;
    let rgb_m = textureSampleLevel(color_texture, color_sampler, input.uv, 0.0).rgb;

    let luma_nw = fxaa_luma(rgb_nw);
    let luma_ne = fxaa_luma(rgb_ne);
    let luma_sw = fxaa_luma(rgb_sw);
    let luma_se = fxaa_luma(rgb_se);
    let luma_m = fxaa_luma(rgb_m);
    let luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    let luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));

    var direction = vec2(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
        (luma_nw + luma_sw) - (luma_ne + luma_se),
    );
    let reduction = max((luma_nw + luma_ne + luma_sw + luma_se) * (0.25 * 0.125), 1.0 / 128.0);
    let inverse_minimum = 1.0 / (min(abs(direction.x), abs(direction.y)) + reduction);
    direction = clamp(direction * inverse_minimum, vec2(-8.0), vec2(8.0)) * texel;

    let rgb_a = 0.5 * (
        textureSampleLevel(color_texture, color_sampler, input.uv + direction * (1.0 / 3.0 - 0.5), 0.0).rgb
        + textureSampleLevel(color_texture, color_sampler, input.uv + direction * (2.0 / 3.0 - 0.5), 0.0).rgb
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        textureSampleLevel(color_texture, color_sampler, input.uv - direction * 0.5, 0.0).rgb
        + textureSampleLevel(color_texture, color_sampler, input.uv + direction * 0.5, 0.0).rgb
    );
    let luma_b = fxaa_luma(rgb_b);
    return vec4(select(rgb_b, rgb_a, luma_b < luma_min || luma_b > luma_max), 1.0);
}
