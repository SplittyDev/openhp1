@group(0) @binding(0)
var previous_depth: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> settings: AoSettings;

@fragment
fn fragment_downsample_depth(input: FullscreenVertex) -> @location(0) f32 {
    let dimensions = vec2<i32>(textureDimensions(previous_depth));
    let base = vec2<i32>(input.position.xy) * 2;
    let depth0 = textureLoad(previous_depth, clamp_pixel(base, dimensions), 0).r;
    let depth1 = textureLoad(previous_depth, clamp_pixel(base + vec2(1, 0), dimensions), 0).r;
    let depth2 = textureLoad(previous_depth, clamp_pixel(base + vec2(0, 1), dimensions), 0).r;
    let depth3 = textureLoad(previous_depth, clamp_pixel(base + vec2(1, 1), dimensions), 0).r;
    return depth_mip_filter(depth0, depth1, depth2, depth3);
}

fn depth_mip_filter(depth0: f32, depth1: f32, depth2: f32, depth3: f32) -> f32 {
    let maximum = max(max(depth0, depth1), max(depth2, depth3));
    let radius = 0.75 * settings.effect_radius * 1.457;
    let falloff_range = 0.615 * radius;
    let falloff_from = radius - falloff_range;
    let falloff_mul = -1.0 / max(falloff_range, 0.0001);
    let falloff_add = falloff_from / max(falloff_range, 0.0001) + 1.0;
    let weights = clamp(
        (maximum - vec4(depth0, depth1, depth2, depth3)) * falloff_mul + falloff_add,
        vec4(0.0),
        vec4(1.0),
    );
    return dot(weights, vec4(depth0, depth1, depth2, depth3))
        / max(dot(weights, vec4(1.0)), 0.0001);
}
