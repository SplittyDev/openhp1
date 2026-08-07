@group(0) @binding(0)
var scene_depth: texture_depth_2d;

@group(0) @binding(1)
var view_depth: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> settings: AoSettings;

struct AoOutput {
    @location(0) visibility: f32,
    @location(1) edges: f32,
};

fn ao_output(visibility: f32, edges: vec4<f32>) -> AoOutput {
    var output: AoOutput;
    output.visibility = visibility / settings.visibility_scale;
    output.edges = pack_edges(edges);
    return output;
}

fn center_depth(pixel: vec2<i32>) -> f32 {
    return textureLoad(view_depth, pixel, 0).r;
}

fn calculate_edges(
    center: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
) -> vec4<f32> {
    var edges = vec4(left, right, top, bottom) - center;
    let slope_lr = (edges.y - edges.x) * 0.5;
    let slope_tb = (edges.w - edges.z) * 0.5;
    let adjusted = edges + vec4(slope_lr, -slope_lr, slope_tb, -slope_tb);
    edges = min(abs(edges), abs(adjusted));
    return clamp(1.25 - edges / max(center * 0.011, 0.0001), vec4(0.0), vec4(1.0));
}

fn pack_edges(edges_value: vec4<f32>) -> f32 {
    let quantized = round(clamp(edges_value, vec4(0.0), vec4(1.0)) * 2.9);
    return dot(quantized, vec4(64.0 / 255.0, 16.0 / 255.0, 4.0 / 255.0, 1.0 / 255.0));
}

fn depth_neighborhood(pixel: vec2<i32>) -> array<f32, 5> {
    let dimensions = vec2<i32>(textureDimensions(view_depth));
    return array<f32, 5>(
        center_depth(pixel),
        center_depth(clamp_pixel(pixel + vec2(-1, 0), dimensions)),
        center_depth(clamp_pixel(pixel + vec2(1, 0), dimensions)),
        center_depth(clamp_pixel(pixel + vec2(0, -1), dimensions)),
        center_depth(clamp_pixel(pixel + vec2(0, 1), dimensions)),
    );
}

fn depth_normal(pixel: vec2<i32>, depths: array<f32, 5>, edges: vec4<f32>) -> vec3<f32> {
    let dimensions = vec2<i32>(textureDimensions(view_depth));
    let center = view_position(pixel, depths[0]);
    let left = normalize_or_zero(view_position(clamp_pixel(pixel + vec2(-1, 0), dimensions), depths[1]) - center);
    let right = normalize_or_zero(view_position(clamp_pixel(pixel + vec2(1, 0), dimensions), depths[2]) - center);
    let top = normalize_or_zero(view_position(clamp_pixel(pixel + vec2(0, -1), dimensions), depths[3]) - center);
    let bottom = normalize_or_zero(view_position(clamp_pixel(pixel + vec2(0, 1), dimensions), depths[4]) - center);
    let accepted = clamp(
        vec4(edges.x * edges.z, edges.z * edges.y, edges.y * edges.w, edges.w * edges.x) + 0.01,
        vec4(0.0),
        vec4(1.0),
    );
    let combined =
        accepted.x * cross(left, top)
        + accepted.y * cross(top, right)
        + accepted.z * cross(right, bottom)
        + accepted.w * cross(bottom, left);
    let length_squared = dot(combined, combined);
    return select(vec3(0.0, 0.0, -1.0), combined * inverseSqrt(max(length_squared, 0.00000001)), length_squared > 0.00000001);
}

fn normalize_or_zero(value: vec3<f32>) -> vec3<f32> {
    let length_squared = dot(value, value);
    return select(vec3(0.0), value * inverseSqrt(max(length_squared, 0.00000001)), length_squared > 0.00000001);
}

fn sample_view_depth(pixel: vec2<i32>, mip_level: u32) -> f32 {
    let dimensions = vec2<i32>(textureDimensions(view_depth, mip_level));
    let mip_pixel = pixel / i32(1u << mip_level);
    return textureLoad(view_depth, clamp_pixel(mip_pixel, dimensions), i32(mip_level)).r;
}
