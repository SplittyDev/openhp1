struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    direction_density: vec4<f32>,
    distance_intensity_pixel: vec4<f32>,
    haze: vec4<f32>,
    dust: vec4<f32>,
};

@group(0) @binding(0)
var scene_depth: texture_depth_2d;

@group(0) @binding(1)
var sun_shadow: texture_depth_2d;

@group(0) @binding(2)
var shadow_sampler: sampler_comparison;

@group(0) @binding(3)
var<uniform> settings: ShadowSettings;

@group(0) @binding(4)
var aperture_masks: texture_2d_array<f32>;

@group(0) @binding(5)
var aperture_sampler: sampler;

struct PortalVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) a: vec4<f32>,
    @location(1) @interpolate(flat) b: vec4<f32>,
    @location(2) @interpolate(flat) c: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) direction: vec4<f32>,
    @location(5) @interpolate(flat) uv_a: vec4<f32>,
    @location(6) @interpolate(flat) uv_b: vec4<f32>,
    @location(7) @interpolate(flat) uv_c: vec4<f32>,
};

@vertex
fn vertex_fullscreen(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) direction: vec4<f32>,
    @location(5) uv_a: vec4<f32>,
    @location(6) uv_b: vec4<f32>,
    @location(7) uv_c: vec4<f32>,
) -> PortalVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    let extrusion = direction.xyz * min(settings.distance_intensity_pixel.x * 0.5, 1500.0);
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
    output.uv_a = uv_a;
    output.uv_b = uv_b;
    output.uv_c = uv_c;
    return output;
}

fn aperture_transmission(uv: vec2<f32>, layer: f32) -> f32 {
    let value = textureSampleLevel(
        aperture_masks,
        aperture_sampler,
        uv,
        i32(layer),
        0.0,
    ).r;
    return smoothstep(0.1, 0.5, value);
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
    let extrusion_length = min(settings.distance_intensity_pixel.x * 0.5, 1500.0);
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

fn directional_phase(light_direction: vec3<f32>, view_direction: vec3<f32>) -> f32 {
    let cosine = dot(light_direction, view_direction);
    return 0.35 + 0.65 * pow(max(cosine, 0.0), 4.0);
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
    let ray_length = min(length(ray), settings.distance_intensity_pixel.x);
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

    const STEP_COUNT = 32;
    let step_length = (interval.y - interval.x) / f32(STEP_COUNT);
    let extrusion_length = min(settings.distance_intensity_pixel.x * 0.5, 1500.0);
    let edge_ab = input.b.xyz - input.a.xyz;
    let edge_ac = input.c.xyz - input.a.xyz;
    let inverse_determinant = 1.0 / dot(edge_ab, cross(edge_ac, input.direction.xyz));
    var lit_length = 0.0;
    var aperture_sum = 0.0;
    var visibility_sum = 0.0;
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
        let aperture_uv = input.uv_a.xy
            + (input.uv_b.xy - input.uv_a.xy) * prism.x
            + (input.uv_c.xy - input.uv_a.xy) * prism.y;
        let visibility = sun_visibility(position);
        let transmission = aperture_transmission(aperture_uv, input.direction.w);
        visibility_sum += visibility;
        aperture_sum += transmission;
        lit_length += visibility
            * transmission
            * end_fade
            * step_length;
    }
    let debug_mode = u32(settings.dust.w + 0.5);
    if debug_mode == 2u {
        let transmission = aperture_sum / f32(STEP_COUNT);
        return vec4(vec3(transmission), 0.0);
    }
    if debug_mode == 3u {
        let visibility = visibility_sum / f32(STEP_COUNT);
        return vec4(1.0 - visibility, visibility, 0.0, 0.0);
    }
    let beam = 1.0 - exp(-lit_length * settings.direction_density.w);
    let midpoint = settings.camera_position.xyz
        + ray_direction * ((interval.x + interval.y) * 0.5);
    let haze = volumetric_dust(midpoint, settings.camera_position.w, settings.haze);
    let phase = directional_phase(input.direction.xyz, -ray_direction);
    return vec4(input.color.rgb * beam * haze * phase * settings.distance_intensity_pixel.y, 0.0);
}

struct DustMoteVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) world_position: vec3<f32>,
    @location(2) @interpolate(flat) color_fade: vec4<f32>,
    @location(3) @interpolate(flat) aperture_uv_layer: vec3<f32>,
};

@vertex
fn vertex_dust_mote(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) direction: vec4<f32>,
    @location(5) uv_a: vec4<f32>,
    @location(6) uv_b: vec4<f32>,
    @location(7) uv_c: vec4<f32>,
) -> DustMoteVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let mote_index = vertex_index / 6u;
    let corner = corners[vertex_index % 6u];
    let seed = floor((a.xyz + b.xyz + c.xyz) * 0.03125)
        + vec3(f32(mote_index) * 17.0, f32(mote_index) * 31.0, f32(mote_index) * 47.0);
    let root = sqrt(volumetric_hash(seed));
    let split = volumetric_hash(seed + vec3(11.0, 23.0, 37.0));
    let base = a.xyz * (1.0 - root)
        + b.xyz * (root * (1.0 - split))
        + c.xyz * (root * split);
    let extrusion_length = min(settings.distance_intensity_pixel.x * 0.5, 1500.0);
    let speed = settings.dust.z
        * mix(0.7, 1.3, volumetric_hash(seed + vec3(41.0, 53.0, 67.0)));
    let phase = fract(
        volumetric_hash(seed + vec3(71.0, 83.0, 97.0))
            + settings.camera_position.w * speed / extrusion_length,
    );
    let world_position = base + direction.xyz * (phase * extrusion_length);
    let clip = settings.view_projection * vec4(world_position, 1.0);
    let radius = settings.dust.x
        * mix(0.25, 0.5, volumetric_hash(seed + vec3(101.0, 113.0, 127.0)));
    var position = vec4(2.0, 2.0, 1.0, 1.0);
    if clip.w > 0.001 {
        let pixel_to_ndc = settings.distance_intensity_pixel.zw * 2.0;
        let offset = (corner * 2.0 - vec2(1.0)) * radius * pixel_to_ndc * clip.w;
        position = vec4(clip.xy + offset, clip.zw);
    }

    var output: DustMoteVertex;
    output.position = position;
    output.uv = corner;
    output.world_position = world_position;
    output.color_fade = vec4(color.rgb, 1.0 - smoothstep(0.65, 1.0, phase));
    output.aperture_uv_layer = vec3(
        uv_a.xy * (1.0 - root)
            + uv_b.xy * (root * (1.0 - split))
            + uv_c.xy * (root * split),
        direction.w,
    );
    return output;
}

@fragment
fn fragment_dust_mote(input: DustMoteVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    if input.position.z > textureLoad(scene_depth, pixel, 0) + 0.0005 {
        discard;
    }
    let circle = 1.0 - smoothstep(0.2, 0.5, distance(input.uv, vec2(0.5)));
    let brightness = circle
        * input.color_fade.a
        * sun_visibility(input.world_position)
        * aperture_transmission(input.aperture_uv_layer.xy, input.aperture_uv_layer.z)
        * settings.dust.y;
    return vec4(
        input.color_fade.rgb * brightness * settings.distance_intensity_pixel.y,
        0.0,
    );
}
