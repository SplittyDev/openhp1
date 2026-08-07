@group(0) @binding(0)
var scene_texture: texture_2d<f32>;

@group(0) @binding(1)
var scene_sampler: sampler;

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

fn extract_bloom(color: vec3<f32>) -> vec3<f32> {
    const THRESHOLD = 1.0;
    const KNEE = 0.1;
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
