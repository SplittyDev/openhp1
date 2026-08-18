@group(0) @binding(0)
var ao_input: texture_2d<f32>;

@group(0) @binding(1)
var edge_input: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> settings: AoSettings;

@fragment
fn fragment_denoise_first(input: FullscreenVertex) -> @location(0) f32 {
    return denoise(vec2<i32>(input.position.xy), 1.2 / 5.0);
}

@fragment
fn fragment_denoise_final(input: FullscreenVertex) -> @location(0) f32 {
    return clamp(denoise(vec2<i32>(input.position.xy), 1.2) * settings.visibility_scale, 0.0, 1.0);
}

@fragment
fn fragment_apply(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(ao_input));
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), dimensions);
    let background = textureLoad(edge_input, pixel, 0).r >= settings.near_far.y * 0.99999;
    let visibility = select(textureLoad(ao_input, pixel, 0).r, 1.0, background);
    return vec4(vec3(visibility), 1.0);
}

fn denoise(pixel_unclamped: vec2<i32>, center_weight: f32) -> f32 {
    let dimensions = vec2<i32>(textureDimensions(ao_input));
    let pixel = clamp_pixel(pixel_unclamped, dimensions);
    let left_pixel = clamp_pixel(pixel + vec2(-1, 0), dimensions);
    let right_pixel = clamp_pixel(pixel + vec2(1, 0), dimensions);
    let top_pixel = clamp_pixel(pixel + vec2(0, -1), dimensions);
    let bottom_pixel = clamp_pixel(pixel + vec2(0, 1), dimensions);
    var edges = unpack_edges(textureLoad(edge_input, pixel, 0).r);
    let left_edges = unpack_edges(textureLoad(edge_input, left_pixel, 0).r);
    let right_edges = unpack_edges(textureLoad(edge_input, right_pixel, 0).r);
    let top_edges = unpack_edges(textureLoad(edge_input, top_pixel, 0).r);
    let bottom_edges = unpack_edges(textureLoad(edge_input, bottom_pixel, 0).r);
    edges *= vec4(left_edges.y, right_edges.x, top_edges.w, bottom_edges.z);
    let edginess = clamp((1.5 - dot(edges, vec4(1.0))) / 1.5, 0.0, 1.0) * 0.5;
    edges = clamp(edges + edginess, vec4(0.0), vec4(1.0));

    let diagonal = 0.425 * vec4(
        edges.x * left_edges.z + edges.z * top_edges.x,
        edges.z * top_edges.y + edges.y * right_edges.z,
        edges.x * left_edges.w + edges.w * bottom_edges.x,
        edges.y * right_edges.w + edges.w * bottom_edges.y,
    );
    var sum = textureLoad(ao_input, pixel, 0).r * center_weight;
    var weight = center_weight;
    sum += textureLoad(ao_input, left_pixel, 0).r * edges.x;
    sum += textureLoad(ao_input, right_pixel, 0).r * edges.y;
    sum += textureLoad(ao_input, top_pixel, 0).r * edges.z;
    sum += textureLoad(ao_input, bottom_pixel, 0).r * edges.w;
    weight += dot(edges, vec4(1.0));

    let top_left = clamp_pixel(pixel + vec2(-1, -1), dimensions);
    let top_right = clamp_pixel(pixel + vec2(1, -1), dimensions);
    let bottom_left = clamp_pixel(pixel + vec2(-1, 1), dimensions);
    let bottom_right = clamp_pixel(pixel + vec2(1, 1), dimensions);
    sum += textureLoad(ao_input, top_left, 0).r * diagonal.x;
    sum += textureLoad(ao_input, top_right, 0).r * diagonal.y;
    sum += textureLoad(ao_input, bottom_left, 0).r * diagonal.z;
    sum += textureLoad(ao_input, bottom_right, 0).r * diagonal.w;
    weight += dot(diagonal, vec4(1.0));
    return sum / max(weight, 0.0001);
}

fn unpack_edges(packed_value: f32) -> vec4<f32> {
    let packed = u32(packed_value * 255.5);
    return vec4<f32>(
        f32((packed >> 6u) & 3u),
        f32((packed >> 4u) & 3u),
        f32((packed >> 2u) & 3u),
        f32(packed & 3u),
    ) / 3.0;
}
