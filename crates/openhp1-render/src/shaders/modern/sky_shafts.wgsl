struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
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

struct PortalVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) a: vec4<f32>,
    @location(1) @interpolate(flat) b: vec4<f32>,
    @location(2) @interpolate(flat) c: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vertex_fullscreen(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) color: vec4<f32>,
) -> PortalVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    let normal = cross(b.xyz - a.xyz, c.xyz - a.xyz);
    var direction = settings.direction_density.xyz;
    if dot(normal, direction) > 0.0 {
        direction = -direction;
    }
    let extrusion = direction * min(settings.distance_intensity_phase.x * 0.5, 1500.0);
    let points = array<vec3<f32>, 6>(
        a.xyz, b.xyz, c.xyz, a.xyz + extrusion, b.xyz + extrusion, c.xyz + extrusion,
    );
    var minimum = vec2(1.0);
    var maximum = vec2(-1.0);
    var crosses_camera = false;
    var front_points = 0u;
    for (var index = 0u; index < 6u; index++) {
        let clip = settings.view_projection * vec4(points[index], 1.0);
        if clip.w <= 0.001 {
            crosses_camera = true;
        } else {
            front_points += 1u;
            let ndc = clip.xy / clip.w;
            minimum = min(minimum, ndc);
            maximum = max(maximum, ndc);
        }
    }
    if front_points == 0u {
        minimum = vec2(2.0);
        maximum = vec2(2.0);
    } else if crosses_camera {
        minimum = vec2(-1.0);
        maximum = vec2(1.0);
    }
    minimum = clamp(minimum, vec2(-1.0), vec2(1.0));
    maximum = clamp(maximum, vec2(-1.0), vec2(1.0));

    var output: PortalVertex;
    output.position = vec4(mix(minimum, maximum, corners[vertex_index] * 0.5 + 0.5), 0.0, 1.0);
    output.a = a;
    output.b = b;
    output.c = c;
    output.color = color;
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

fn inside_portal_volume(world: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> bool {
    let edge_ab = b - a;
    let edge_ac = c - a;
    let normal = cross(edge_ab, edge_ac);
    if dot(normal, normal) < 0.0001 {
        return false;
    }

    var direction = settings.direction_density.xyz;
    if dot(normal, direction) > 0.0 {
        direction = -direction;
    }
    let denominator = dot(normal, direction);
    if abs(denominator) < 0.0001 {
        return false;
    }
    let travel = dot(normal, world - a) / denominator;
    if travel < 0.0 || travel > min(settings.distance_intensity_phase.x * 0.5, 1500.0) {
        return false;
    }

    let point = world - direction * travel - a;
    let d00 = dot(edge_ab, edge_ab);
    let d01 = dot(edge_ab, edge_ac);
    let d11 = dot(edge_ac, edge_ac);
    let d20 = dot(point, edge_ab);
    let d21 = dot(point, edge_ac);
    let inverse = 1.0 / max(d00 * d11 - d01 * d01, 0.0001);
    let v = (d11 * d20 - d01 * d21) * inverse;
    let w = (d00 * d21 - d01 * d20) * inverse;
    return v >= 0.0 && w >= 0.0 && v + w <= 1.0;
}

@fragment
fn fragment_sky_shafts(input: PortalVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    let depth = textureLoad(scene_depth, pixel, 0);
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(dimensions);
    let clip = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), depth, 1.0);
    let world_h = settings.inverse_view_projection * clip;
    let world = world_h.xyz / world_h.w;
    let ray = world - settings.camera_position.xyz;
    let ray_length = min(length(ray), settings.distance_intensity_phase.x);
    if ray_length <= 0.001 {
        return vec4(0.0);
    }

    const STEP_COUNT = 24u;
    let ray_direction = normalize(ray);
    let step_length = ray_length / f32(STEP_COUNT);
    let jitter = hash(vec2<u32>(pixel));
    let phase = phase_henyey_greenstein(
        dot(ray_direction, -settings.direction_density.xyz),
        settings.distance_intensity_phase.z,
    );
    var scattering = vec3(0.0);
    for (var step = 0u; step < STEP_COUNT; step++) {
        let distance = (f32(step) + jitter) * step_length;
        let sample_position = settings.camera_position.xyz + ray_direction * distance;
        if inside_portal_volume(sample_position, input.a.xyz, input.b.xyz, input.c.xyz) {
            scattering += input.color.rgb * sun_visibility(sample_position) * step_length;
        }
    }
    scattering *= settings.direction_density.w * settings.distance_intensity_phase.y * phase;
    return vec4(scattering, 0.0);
}
