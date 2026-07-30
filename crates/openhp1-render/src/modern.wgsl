struct Settings {
    inverse_viewport: vec2<f32>,
    brightness_gamma: f32,
    bloom_strength: f32,
    tone_mapper: u32,
    ssao: u32,
    _padding: vec2<u32>,
};

@group(0) @binding(0)
var scene_texture: texture_2d<f32>;

@group(0) @binding(1)
var scene_sampler: sampler;

@group(0) @binding(2)
var scene_depth: texture_depth_2d;

@group(0) @binding(3)
var<uniform> settings: Settings;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const BLOOM_OFFSETS = array<vec2<f32>, 12>(
    vec2(1.0, 0.0),
    vec2(-1.0, 0.0),
    vec2(0.0, 1.0),
    vec2(0.0, -1.0),
    vec2(1.0, 1.0),
    vec2(-1.0, 1.0),
    vec2(1.0, -1.0),
    vec2(-1.0, -1.0),
    vec2(2.0, 0.0),
    vec2(-2.0, 0.0),
    vec2(0.0, 2.0),
    vec2(0.0, -2.0),
);

const SSAO_OFFSETS = array<vec2<i32>, 8>(
    vec2(2, 0),
    vec2(-2, 0),
    vec2(0, 2),
    vec2(0, -2),
    vec2(2, 2),
    vec2(-2, 2),
    vec2(2, -2),
    vec2(-2, -2),
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
fn fragment_post_process(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let scene = max(textureSampleLevel(scene_texture, scene_sampler, input.uv, 0.0).rgb, vec3(0.0));
    let ambient = ambient_visibility(vec2<i32>(input.position.xy));
    let hdr = scene * ambient + bloom(input.uv) * settings.bloom_strength;
    let mapped = tone_map(hdr);
    return vec4(pow(clamp(mapped, vec3(0.0), vec3(1.0)), vec3(settings.brightness_gamma)), 1.0);
}

fn bloom(uv: vec2<f32>) -> vec3<f32> {
    var color = vec3(0.0);
    for (var index = 0u; index < 12u; index += 1u) {
        let sample = max(
            textureSampleLevel(
                scene_texture,
                scene_sampler,
                uv + BLOOM_OFFSETS[index] * settings.inverse_viewport * 2.0,
                0.0,
            ).rgb,
            vec3(0.0),
        );
        color += sample * smoothstep(0.8, 1.6, max(sample.r, max(sample.g, sample.b)));
    }
    return color / 12.0;
}

fn ambient_visibility(pixel: vec2<i32>) -> f32 {
    if settings.ssao == 0u {
        return 1.0;
    }
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let center = textureLoad(scene_depth, clamp(pixel, vec2(0), dimensions - vec2(1)), 0);
    if center >= 0.99999 {
        return 1.0;
    }
    var occlusion = 0.0;
    for (var index = 0u; index < 8u; index += 1u) {
        let sample_pixel = clamp(pixel + SSAO_OFFSETS[index], vec2(0), dimensions - vec2(1));
        let sample = textureLoad(scene_depth, sample_pixel, 0);
        let delta = center - sample;
        let closer = smoothstep(0.0001, 0.006, delta);
        let same_surface = 1.0 - smoothstep(0.004, 0.03, abs(delta));
        occlusion += closer * same_surface;
    }
    return 1.0 - 0.45 * occlusion / 8.0;
}

fn tone_map(color: vec3<f32>) -> vec3<f32> {
    switch settings.tone_mapper {
        case 0u: {
            return agx(color);
        }
        case 1u: {
            return color / (vec3(1.0) + color);
        }
        default: {
            return aces(color);
        }
    }
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
    return clamp(
        vec3(
            1.1968790051 * value.r - 0.0980208811 * value.g - 0.0990297441 * value.b,
            -0.0528968518 * value.r + 1.1519031299 * value.g - 0.0989611768 * value.b,
            -0.0529716355 * value.r - 0.0980434501 * value.g + 1.1510736726 * value.b,
        ),
        vec3(0.0),
        vec3(1.0),
    );
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
