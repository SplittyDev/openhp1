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
    @location(4) @interpolate(flat) direction: vec4<f32>,
};

@vertex
fn vertex_fullscreen(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) direction: vec4<f32>,
) -> PortalVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    let extrusion = direction.xyz * min(settings.distance_intensity_phase.x * 0.5, 1500.0);
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
    output.direction = direction;
    return output;
}

fn clip_lower_bound(interval: vec2<f32>, origin: f32, slope: f32) -> vec2<f32> {
    if abs(slope) < 0.00001 {
        if origin < 0.0 {
            return vec2(1.0, -1.0);
        }
        return interval;
    }
    let crossing = -origin / slope;
    if slope > 0.0 {
        return vec2(max(interval.x, crossing), interval.y);
    }
    return vec2(interval.x, min(interval.y, crossing));
}

fn prism_coordinates(
    vector: vec3<f32>,
    edge_ab: vec3<f32>,
    edge_ac: vec3<f32>,
    direction: vec3<f32>,
    inverse_determinant: f32,
) -> vec3<f32> {
    return vec3(
        dot(vector, cross(edge_ac, direction)),
        dot(edge_ab, cross(vector, direction)),
        dot(edge_ab, cross(edge_ac, vector)),
    ) * inverse_determinant;
}

fn portal_interval(
    ray_origin: vec3<f32>,
    ray_direction: vec3<f32>,
    ray_length: f32,
    a: vec3<f32>,
    b: vec3<f32>,
    c: vec3<f32>,
    direction: vec3<f32>,
) -> vec2<f32> {
    let edge_ab = b - a;
    let edge_ac = c - a;
    let determinant = dot(edge_ab, cross(edge_ac, direction));
    if abs(determinant) < 0.0001 {
        return vec2(1.0, -1.0);
    }

    let inverse_determinant = 1.0 / determinant;
    let origin = prism_coordinates(
        ray_origin - a,
        edge_ab,
        edge_ac,
        direction,
        inverse_determinant,
    );
    let slope = prism_coordinates(
        ray_direction,
        edge_ab,
        edge_ac,
        direction,
        inverse_determinant,
    );
    let extrusion_length = min(settings.distance_intensity_phase.x * 0.5, 1500.0);
    var interval = vec2(0.0, ray_length);
    interval = clip_lower_bound(interval, origin.x, slope.x);
    interval = clip_lower_bound(interval, origin.y, slope.y);
    interval = clip_lower_bound(interval, 1.0 - origin.x - origin.y, -slope.x - slope.y);
    interval = clip_lower_bound(interval, origin.z, slope.z);
    interval = clip_lower_bound(interval, extrusion_length - origin.z, -slope.z);
    return interval;
}

fn sun_visibility(position: vec3<f32>) -> f32 {
    let clip = settings.light_view_projection * vec4(position, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2(0.5, -0.5) + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || ndc.z < 0.0 || ndc.z > 1.0 {
        return 0.0;
    }
    return textureSampleCompareLevel(sun_shadow, shadow_sampler, uv, ndc.z - 0.001);
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

    let ray_direction = normalize(ray);
    let interval = portal_interval(
        settings.camera_position.xyz,
        ray_direction,
        ray_length,
        input.a.xyz,
        input.b.xyz,
        input.c.xyz,
        input.direction.xyz,
    );
    if interval.y <= interval.x {
        return vec4(0.0);
    }

    const STEP_COUNT = 8;
    let step_length = (interval.y - interval.x) / f32(STEP_COUNT);
    let extrusion_length = min(settings.distance_intensity_phase.x * 0.5, 1500.0);
    let edge_ab = input.b.xyz - input.a.xyz;
    let edge_ac = input.c.xyz - input.a.xyz;
    let inverse_determinant = 1.0 / dot(edge_ab, cross(edge_ac, input.direction.xyz));
    var lit_length = 0.0;
    for (var index = 0; index < STEP_COUNT; index += 1) {
        let distance = interval.x + (f32(index) + 0.5) * step_length;
        let position = settings.camera_position.xyz + ray_direction * distance;
        let prism = prism_coordinates(
            position - input.a.xyz,
            edge_ab,
            edge_ac,
            input.direction.xyz,
            inverse_determinant,
        );
        let along_shaft = clamp(prism.z / extrusion_length, 0.0, 1.0);
        let end_fade = 1.0 - smoothstep(0.65, 1.0, along_shaft);
        lit_length += sun_visibility(position) * end_fade * step_length;
    }
    let beam = 1.0 - exp(-lit_length * settings.direction_density.w);
    return vec4(input.color.rgb * beam * settings.distance_intensity_phase.y, 0.0);
}
