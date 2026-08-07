// Ported from SMAA v2.8; see SMAA-LICENSE.txt.

@group(0) @binding(0)
var color_texture: texture_2d<f32>;

@group(0) @binding(1)
var color_sampler: sampler;

fn smaa_color(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(color_texture, color_sampler, uv, 0.0).rgb;
}

fn smaa_color_delta(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let delta = abs(a - b);
    return max(delta.r, max(delta.g, delta.b));
}

@fragment
fn fragment_smaa_edges(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(color_texture));
    let color = smaa_color(input.uv);
    let delta_left = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(-1.0, 0.0)));
    let delta_top = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(0.0, -1.0)));
    var edges = step(vec2(0.1), vec2(delta_left, delta_top));
    if all(edges == vec2(0.0)) {
        discard;
    }

    let delta_right = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(1.0, 0.0)));
    let delta_bottom = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(0.0, 1.0)));
    let delta_left_left = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(-2.0, 0.0)));
    let delta_top_top = smaa_color_delta(color, smaa_color(input.uv + texel * vec2(0.0, -2.0)));
    let maximum = max(
        max(delta_left, delta_top),
        max(max(delta_right, delta_bottom), max(delta_left_left, delta_top_top)),
    );
    edges *= step(vec2(maximum), 2.0 * vec2(delta_left, delta_top));
    return vec4(edges, 0.0, 0.0);
}
