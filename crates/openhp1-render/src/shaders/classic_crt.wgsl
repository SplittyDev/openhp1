// OpenHP1's high-resolution PC CRT model. This is an original implementation
// of the physical effect, not a port of a RetroArch shader.

@group(0) @binding(0)
var crt_source: texture_2d<f32>;

@group(0) @binding(1)
var crt_glow: texture_2d<f32>;

@group(0) @binding(2)
var crt_sampler: sampler;

fn bright_signal(uv: vec2<f32>) -> vec3<f32> {
    let color = textureSampleLevel(crt_source, crt_sampler, uv, 0.0).rgb;
    let peak = max(color.r, max(color.g, color.b));
    return color * smoothstep(0.3, 0.8, peak);
}

fn blur_step() -> f32 {
    return max(f32(textureDimensions(crt_source).y) / 384.0, 1.0);
}

@fragment
fn fragment_crt_halation_horizontal(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = vec2(blur_step() / f32(textureDimensions(crt_source).x), 0.0);
    var color = bright_signal(input.uv) * 70.0;
    color += (bright_signal(input.uv - texel) + bright_signal(input.uv + texel)) * 56.0;
    color += (bright_signal(input.uv - texel * 2.0) + bright_signal(input.uv + texel * 2.0)) * 28.0;
    color += (bright_signal(input.uv - texel * 3.0) + bright_signal(input.uv + texel * 3.0)) * 8.0;
    color += bright_signal(input.uv - texel * 4.0) + bright_signal(input.uv + texel * 4.0);
    return vec4(color / 256.0, 1.0);
}

@fragment
fn fragment_crt_halation_vertical(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = vec2(0.0, blur_step() / f32(textureDimensions(crt_source).y));
    var color = textureSampleLevel(crt_source, crt_sampler, input.uv, 0.0).rgb * 70.0;
    color += (
        textureSampleLevel(crt_source, crt_sampler, input.uv - texel, 0.0).rgb
        + textureSampleLevel(crt_source, crt_sampler, input.uv + texel, 0.0).rgb
    ) * 56.0;
    color += (
        textureSampleLevel(crt_source, crt_sampler, input.uv - texel * 2.0, 0.0).rgb
        + textureSampleLevel(crt_source, crt_sampler, input.uv + texel * 2.0, 0.0).rgb
    ) * 28.0;
    color += (
        textureSampleLevel(crt_source, crt_sampler, input.uv - texel * 3.0, 0.0).rgb
        + textureSampleLevel(crt_source, crt_sampler, input.uv + texel * 3.0, 0.0).rgb
    ) * 8.0;
    color += textureSampleLevel(crt_source, crt_sampler, input.uv - texel * 4.0, 0.0).rgb;
    color += textureSampleLevel(crt_source, crt_sampler, input.uv + texel * 4.0, 0.0).rgb;
    return vec4(color / 256.0, 1.0);
}

fn curved_uv(uv: vec2<f32>) -> vec2<f32> {
    let centered = uv * 2.0 - 1.0;
    return centered * (1.0 + dot(centered, centered) * 0.0075) * 0.5 + 0.5;
}

fn aperture_grille(pixel_x: u32) -> vec3<f32> {
    switch pixel_x % 3u {
        case 0u: { return vec3(1.06, 0.97, 0.97); }
        case 1u: { return vec3(0.97, 1.06, 0.97); }
        default: { return vec3(0.97, 0.97, 1.06); }
    }
}

fn reference_pixel(position: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(textureDimensions(crt_source));
    let scale = max(size / vec2(1024.0, 768.0), vec2(1.0));
    return (position - 0.5) / scale;
}

@fragment
fn fragment_crt_composite(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let uv = curved_uv(input.uv);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    let base = textureSampleLevel(crt_source, crt_sampler, uv, 0.0).rgb;
    let diffused = textureSampleLevel(crt_glow, crt_sampler, uv, 0.0).rgb;
    let local_bright = bright_signal(uv);
    let halation = diffused * 0.08 + max(diffused - local_bright, vec3(0.0)) * 0.45;

    // A 768-line PC monitor resolves a much finer raster than a 240p console.
    let raster = reference_pixel(input.position.xy);
    let scanline = 0.955 + 0.045 * cos(3.14159265 * raster.y);
    let mask = aperture_grille(u32(floor(raster.x)));
    let centered = input.uv * 2.0 - 1.0;
    let vignette = 1.0 - 0.06 * smoothstep(0.55, 1.0, max(abs(centered.x), abs(centered.y)));

    return vec4(max((base + halation) * mask * scanline * vignette, vec3(0.0)), 1.0);
}
