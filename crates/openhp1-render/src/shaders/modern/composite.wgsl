struct ModernSettings {
    brightness_gamma: f32,
    bloom_strength: f32,
    contrast: f32,
    tone_mapper: u32,
    ambient_occlusion: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
};

@group(0) @binding(0)
var scene_texture: texture_2d<f32>;

@group(0) @binding(1)
var scene_sampler: sampler;

@group(0) @binding(2)
var bloom_texture: texture_2d<f32>;

@group(0) @binding(3)
var ao_texture: texture_2d<f32>;

@group(0) @binding(4)
var<uniform> settings: ModernSettings;

@fragment
fn fragment_composite(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(scene_texture, scene_sampler, input.uv, 0.0);
    var ambient = 1.0;
    if settings.ambient_occlusion != 0u && scene.a >= 0.5 {
        let ao_pixel = clamp(
            vec2<i32>(input.position.xy),
            vec2(0),
            vec2<i32>(textureDimensions(ao_texture)) - vec2(1),
        );
        ambient = textureLoad(ao_texture, ao_pixel, 0).r;
    }
    var hdr = max(scene.rgb, vec3(0.0)) * ambient;
    if settings.bloom_strength != 0.0 {
        let bloom = textureSampleLevel(bloom_texture, scene_sampler, input.uv, 0.0).rgb;
        hdr += bloom * settings.bloom_strength;
    }
    let mapped = tone_map(hdr);
    let encoded = srgb_encode(clamp(mapped, vec3(0.0), vec3(1.0)));
    let contrasted = display_contrast(encoded);
    return vec4(
        pow(
            clamp(contrasted, vec3(0.0), vec3(1.0)),
            vec3(settings.brightness_gamma),
        ),
        1.0,
    );
}

fn display_contrast(color: vec3<f32>) -> vec3<f32> {
    const LUMINANCE = vec3(0.2126, 0.7152, 0.0722);
    let luminance = dot(color, LUMINANCE);
    if luminance <= 0.0 {
        return vec3(0.0);
    }
    let lower = pow(clamp(luminance, 0.0, 1.0), settings.contrast);
    let upper = pow(1.0 - clamp(luminance, 0.0, 1.0), settings.contrast);
    let contrasted_luminance = lower / max(lower + upper, 0.00001);
    let scale = contrasted_luminance / luminance;
    let maximum = max(color.r, max(color.g, color.b));
    return color * min(scale, 1.0 / max(maximum, 0.00001));
}

fn srgb_encode(color: vec3<f32>) -> vec3<f32> {
    let linear = color * 12.92;
    let exponential = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return select(exponential, linear, color <= vec3(0.0031308));
}
