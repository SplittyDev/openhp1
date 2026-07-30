struct Settings {
    inverse_viewport: vec2<f32>,
    brightness_gamma: f32,
    bloom_strength: f32,
    tone_mapper: u32,
    ssao: u32,
    _padding: vec2<u32>,
    projection: vec4<f32>,
};

@group(0) @binding(0)
var scene_texture: texture_2d<f32>;

@group(0) @binding(1)
var scene_sampler: sampler;

@group(0) @binding(2)
var scene_depth: texture_depth_2d;

@group(0) @binding(3)
var<uniform> settings: Settings;

@group(0) @binding(4)
var bloom_texture: texture_2d<f32>;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const SSAO_KERNEL = array<vec2<f32>, 16>(
    vec2(0.35, 0.0),
    vec2(-0.35, 0.0),
    vec2(0.0, 0.35),
    vec2(0.0, -0.35),
    vec2(0.2475, 0.2475),
    vec2(-0.2475, 0.2475),
    vec2(0.2475, -0.2475),
    vec2(-0.2475, -0.2475),
    vec2(1.0, 0.0),
    vec2(-1.0, 0.0),
    vec2(0.0, 1.0),
    vec2(0.0, -1.0),
    vec2(0.7071, 0.7071),
    vec2(-0.7071, 0.7071),
    vec2(0.7071, -0.7071),
    vec2(-0.7071, -0.7071),
);

@vertex
fn vertex_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertex {
    let positions = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0),
    );
    var output: FullscreenVertex;
    output.position = vec4(positions[vertex_index], 0.0, 1.0);
    output.uv = positions[vertex_index] * vec2(0.5, -0.5) + vec2(0.5);
    return output;
}

@fragment
fn fragment_bloom_extract(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(scene_texture));
    let color = 0.25 * (
        textureSampleLevel(scene_texture, scene_sampler, input.uv + vec2(-1.0, -1.0) * texel, 0.0).rgb
        + textureSampleLevel(scene_texture, scene_sampler, input.uv + vec2(1.0, -1.0) * texel, 0.0).rgb
        + textureSampleLevel(scene_texture, scene_sampler, input.uv + vec2(-1.0, 1.0) * texel, 0.0).rgb
        + textureSampleLevel(scene_texture, scene_sampler, input.uv + vec2(1.0, 1.0) * texel, 0.0).rgb
    );
    return vec4(extract_bloom(max(color, vec3(0.0))), 1.0);
}

@fragment
fn fragment_bloom_horizontal(input: FullscreenVertex) -> @location(0) vec4<f32> {
    return vec4(gaussian_blur(input.uv, vec2(1.0, 0.0)), 1.0);
}

@fragment
fn fragment_bloom_vertical(input: FullscreenVertex) -> @location(0) vec4<f32> {
    return vec4(gaussian_blur(input.uv, vec2(0.0, 1.0)), 1.0);
}

@fragment
fn fragment_post_process(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let scene = max(textureSampleLevel(scene_texture, scene_sampler, input.uv, 0.0).rgb, vec3(0.0));
    let ambient = ambient_visibility(vec2<i32>(input.position.xy));
    let bloom = textureSampleLevel(bloom_texture, scene_sampler, input.uv, 0.0).rgb;
    let hdr = scene * ambient + bloom * settings.bloom_strength;
    let mapped = tone_map(hdr);
    let encoded = srgb_encode(clamp(mapped, vec3(0.0), vec3(1.0)));
    return vec4(pow(encoded, vec3(settings.brightness_gamma)), 1.0);
}

fn extract_bloom(color: vec3<f32>) -> vec3<f32> {
    const THRESHOLD = 0.8;
    const KNEE = 0.4;
    let brightness = max(color.r, max(color.g, color.b));
    var soft = clamp(brightness - THRESHOLD + KNEE, 0.0, 2.0 * KNEE);
    soft = soft * soft / (4.0 * KNEE + 0.00001);
    let contribution = max(brightness - THRESHOLD, soft) / max(brightness, 0.00001);
    return color * contribution;
}

fn gaussian_blur(uv: vec2<f32>, direction: vec2<f32>) -> vec3<f32> {
    // Quarter resolution plus this radius produces a broad ~26-pixel halo.
    const RADIUS = 2.0;
    let texel = direction * RADIUS / vec2<f32>(textureDimensions(scene_texture));
    var color = textureSampleLevel(scene_texture, scene_sampler, uv, 0.0).rgb * 0.227027;
    color += (
        textureSampleLevel(scene_texture, scene_sampler, uv + texel * 1.384615, 0.0).rgb
        + textureSampleLevel(scene_texture, scene_sampler, uv - texel * 1.384615, 0.0).rgb
    ) * 0.316216;
    color += (
        textureSampleLevel(scene_texture, scene_sampler, uv + texel * 3.230769, 0.0).rgb
        + textureSampleLevel(scene_texture, scene_sampler, uv - texel * 3.230769, 0.0).rgb
    ) * 0.070270;
    return color;
}

fn ambient_visibility(pixel: vec2<i32>) -> f32 {
    if settings.ssao == 0u {
        return 1.0;
    }
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let clamped_pixel = clamp(pixel, vec2(0), dimensions - vec2(1));
    let center_depth = textureLoad(scene_depth, clamped_pixel, 0);
    if center_depth >= 0.99999 {
        return 1.0;
    }

    let center_distance = linear_depth(center_depth);
    let center = view_position(clamped_pixel, center_depth, dimensions);
    let normal = view_normal(clamped_pixel, center, dimensions);
    let focal_pixels = 0.5 / (settings.inverse_viewport.y * settings.projection.z);
    let radius_pixels = clamp(96.0 * focal_pixels / center_distance, 4.0, 64.0);

    var occlusion = 0.0;
    for (var index = 0u; index < 16u; index += 1u) {
        let offset = vec2<i32>(round(SSAO_KERNEL[index] * radius_pixels));
        let sample_pixel = clamp(clamped_pixel + offset, vec2(0), dimensions - vec2(1));
        let sample_depth = textureLoad(scene_depth, sample_pixel, 0);
        if sample_depth < 0.99999 {
            let delta = view_position(sample_pixel, sample_depth, dimensions) - center;
            let distance = length(delta);
            if distance > 0.001 {
                let horizon = max(dot(normal, delta / distance) - 0.05, 0.0);
                let range = 1.0 - smoothstep(19.2, 96.0, distance);
                occlusion += horizon * range;
            }
        }
    }
    return clamp(1.0 - 2.5 * occlusion / 16.0, 0.35, 1.0);
}

fn linear_depth(depth: f32) -> f32 {
    let near_plane = max(settings.projection.x, 0.0001);
    let far_plane = max(settings.projection.y, near_plane + 0.0001);
    return near_plane * far_plane / max(far_plane - depth * (far_plane - near_plane), 0.0001);
}

fn view_position(pixel: vec2<i32>, depth: f32, dimensions: vec2<i32>) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(dimensions);
    let distance = linear_depth(depth);
    let view_y = (1.0 - 2.0 * uv.y) * distance * settings.projection.z;
    let view_x = (2.0 * uv.x - 1.0) * distance * settings.projection.z * settings.projection.w;
    return vec3(view_x, view_y, -distance);
}

fn view_normal(pixel: vec2<i32>, center: vec3<f32>, dimensions: vec2<i32>) -> vec3<f32> {
    let minimum = vec2<i32>(0);
    let maximum = dimensions - vec2<i32>(1);
    let right_pixel = clamp(pixel + vec2(1, 0), minimum, maximum);
    let left_pixel = clamp(pixel + vec2(-1, 0), minimum, maximum);
    let up_pixel = clamp(pixel + vec2(0, -1), minimum, maximum);
    let down_pixel = clamp(pixel + vec2(0, 1), minimum, maximum);
    let right = view_position(right_pixel, textureLoad(scene_depth, right_pixel, 0), dimensions);
    let left = view_position(left_pixel, textureLoad(scene_depth, left_pixel, 0), dimensions);
    let up = view_position(up_pixel, textureLoad(scene_depth, up_pixel, 0), dimensions);
    let down = view_position(down_pixel, textureLoad(scene_depth, down_pixel, 0), dimensions);

    let right_delta = right - center;
    let left_delta = center - left;
    let up_delta = up - center;
    let down_delta = center - down;
    let horizontal = select(left_delta, right_delta, dot(right_delta, right_delta) < dot(left_delta, left_delta));
    let vertical = select(down_delta, up_delta, dot(up_delta, up_delta) < dot(down_delta, down_delta));
    let cross_value = cross(horizontal, vertical);
    if dot(cross_value, cross_value) < 0.00000001 {
        return vec3(0.0, 0.0, 1.0);
    }
    var normal = normalize(cross_value);
    if normal.z < 0.0 {
        normal = -normal;
    }
    return normal;
}

fn tone_map(color: vec3<f32>) -> vec3<f32> {
    switch settings.tone_mapper {
        case 0u: {
            return agx(color);
        }
        case 1u: {
            return reinhard(color);
        }
        default: {
            return aces(color);
        }
    }
}

fn reinhard(color: vec3<f32>) -> vec3<f32> {
    const WHITE = 4.0;
    const LUMINANCE = vec3(0.2126, 0.7152, 0.0722);
    let luminance = dot(color, LUMINANCE);
    if luminance <= 0.0 {
        return vec3(0.0);
    }
    let mapped_luminance =
        luminance * (1.0 + luminance / (WHITE * WHITE)) / (1.0 + luminance);
    return color * (mapped_luminance / luminance);
}

fn srgb_encode(color: vec3<f32>) -> vec3<f32> {
    let linear = color * 12.92;
    let exponential = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return select(exponential, linear, color <= vec3(0.0031308));
}

fn aces(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((color * (a * color + b)) / (color * (c * color + d) + e), vec3(0.0), vec3(1.0));
}

fn agx(color: vec3<f32>) -> vec3<f32> {
    var value = vec3(
        0.8424790623 * color.r + 0.0784336000 * color.g + 0.0792237451 * color.b,
        0.0423282423 * color.r + 0.8784686365 * color.g + 0.0791661275 * color.b,
        0.0423756549 * color.r + 0.0784336000 * color.g + 0.8791429738 * color.b,
    );
    const MIN_EV = -12.47393;
    const MAX_EV = 4.026069;
    value = clamp((log2(max(value, vec3(1e-10))) - MIN_EV) / (MAX_EV - MIN_EV), vec3(0.0), vec3(1.0));
    value = agx_contrast(value);
    value = vec3(
        1.1968790051 * value.r - 0.0980208811 * value.g - 0.0990297441 * value.b,
        -0.0528968518 * value.r + 1.1519031299 * value.g - 0.0989611768 * value.b,
        -0.0529716355 * value.r - 0.0980434501 * value.g + 1.1510736726 * value.b,
    );
    return clamp(pow(max(value, vec3(0.0)), vec3(2.2)), vec3(0.0), vec3(1.0));
}

fn agx_contrast(value: vec3<f32>) -> vec3<f32> {
    let value2 = value * value;
    let value4 = value2 * value2;
    return 15.5 * value4 * value2
        - 40.14 * value4 * value
        + 31.96 * value4
        - 6.868 * value2 * value
        + 0.4298 * value2
        + 0.1191 * value
        - 0.00232;
}
