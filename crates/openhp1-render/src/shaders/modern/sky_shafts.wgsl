struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    direction_density: vec4<f32>,
    distance_intensity_phase: vec4<f32>,
};

@group(0) @binding(0)
var scene_depth: texture_depth_2d;

@group(0) @binding(1)
var sun_shadow: texture_depth_2d;

@group(0) @binding(2)
var shadow_sampler: sampler_comparison;

@group(0) @binding(3)
var<uniform> settings: ShadowSettings;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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

fn hash(pixel: vec2<u32>) -> f32 {
    var value = pixel.x * 1597334677u ^ pixel.y * 3812015801u;
    value = (value ^ (value >> 16u)) * 2246822519u;
    value = (value ^ (value >> 13u)) * 3266489917u;
    return f32(value ^ (value >> 16u)) / 4294967295.0;
}

fn phase_henyey_greenstein(cosine: f32, anisotropy: f32) -> f32 {
    let g2 = anisotropy * anisotropy;
    let denominator = pow(max(1.0 + g2 - 2.0 * anisotropy * cosine, 0.001), 1.5);
    return (1.0 - g2) / (12.5663706 * denominator);
}

fn sun_visibility(world: vec3<f32>) -> f32 {
    let clip = settings.light_view_projection * vec4(world, 1.0);
    let ndc = clip.xyz / clip.w;
    if any(ndc.xy < vec2(-1.0)) || any(ndc.xy > vec2(1.0)) || ndc.z <= 0.0 || ndc.z >= 1.0 {
        return 0.0;
    }
    let uv = ndc.xy * vec2(0.5, -0.5) + vec2(0.5);
    return textureSampleCompareLevel(sun_shadow, shadow_sampler, uv, ndc.z - 0.0008);
}

@fragment
fn fragment_sky_shafts(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    let depth = textureLoad(scene_depth, pixel, 0);
    let clip = vec4(input.uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), depth, 1.0);
    let world_h = settings.inverse_view_projection * clip;
    let world = world_h.xyz / world_h.w;
    let ray = world - settings.camera_position.xyz;
    let ray_length = min(length(ray), settings.distance_intensity_phase.x);
    if ray_length <= 0.001 {
        return vec4(0.0);
    }

    const STEP_COUNT = 40u;
    let ray_direction = normalize(ray);
    let step_length = ray_length / f32(STEP_COUNT);
    let jitter = hash(vec2<u32>(pixel));
    let phase = phase_henyey_greenstein(
        dot(ray_direction, -settings.direction_density.xyz),
        settings.distance_intensity_phase.z,
    );
    var scattering = 0.0;
    var visible_steps = 0.0;
    for (var step = 0u; step < STEP_COUNT; step++) {
        let distance = (f32(step) + jitter) * step_length;
        let sample_position = settings.camera_position.xyz + ray_direction * distance;
        let visibility = sun_visibility(sample_position);
        visible_steps += visibility;
        scattering += visibility * step_length;
    }
    let lit_fraction = visible_steps / f32(STEP_COUNT);
    let occlusion_contrast = 4.0 * lit_fraction * (1.0 - lit_fraction);
    scattering *= occlusion_contrast * occlusion_contrast;
    scattering *= settings.direction_density.w * settings.distance_intensity_phase.y * phase;
    let sunlight = vec3(1.0, 0.82, 0.62);
    return vec4(sunlight * scattering, 0.0);
}
