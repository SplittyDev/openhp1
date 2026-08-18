struct ShadowSettings {
    light_view_projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    direction_density: vec4<f32>,
    distance_intensity_pixel: vec4<f32>,
    haze: vec4<f32>,
    dust: vec4<f32>,
    shaft: vec4<f32>,
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
    @location(8) @interpolate(flat) center_scale: vec4<f32>,
    @location(9) @interpolate(flat) uv_bounds: vec4<f32>,
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
    @location(8) center_scale: vec4<f32>,
    @location(9) uv_bounds: vec4<f32>,
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
    let end_scale = max(center_scale.w, 1.0);
    let end_a = center_scale.xyz + (a.xyz - center_scale.xyz) * end_scale + extrusion;
    let end_b = center_scale.xyz + (b.xyz - center_scale.xyz) * end_scale + extrusion;
    let end_c = center_scale.xyz + (c.xyz - center_scale.xyz) * end_scale + extrusion;
    let points = array<vec3<f32>, 6>(
        a.xyz, b.xyz, c.xyz, end_a, end_b, end_c,
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
    output.center_scale = center_scale;
    output.uv_bounds = uv_bounds;
    return output;
}

@vertex
fn vertex_projection(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) direction: vec4<f32>,
    @location(5) uv_a: vec4<f32>,
    @location(6) uv_b: vec4<f32>,
    @location(7) uv_c: vec4<f32>,
    @location(8) center_scale: vec4<f32>,
    @location(9) uv_bounds: vec4<f32>,
) -> PortalVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    var output: PortalVertex;
    output.position = vec4(corners[vertex_index], 0.0, 1.0);
    output.a = a;
    output.b = b;
    output.c = c;
    output.color = color;
    output.direction = direction;
    output.uv_a = uv_a;
    output.uv_b = uv_b;
    output.uv_c = uv_c;
    output.center_scale = center_scale;
    output.uv_bounds = uv_bounds;
    return output;
}

fn aperture_value(uv: vec2<f32>, layer: f32) -> f32 {
    return textureSampleLevel(
        aperture_masks,
        aperture_sampler,
        uv,
        i32(layer),
        0.0,
    ).a;
}

fn aperture_color(uv: vec2<f32>, layer: f32) -> vec3<f32> {
    return textureSampleLevel(aperture_masks, aperture_sampler, uv, i32(layer), 0.0).rgb;
}

fn aperture_transmission(uv: vec2<f32>, layer: f32) -> f32 {
    return aperture_value(uv, layer);
}

fn soft_aperture_transmission(uv: vec2<f32>, layer: f32, blur_texels: f32) -> f32 {
    let offset = blur_texels / vec2<f32>(textureDimensions(aperture_masks));
    var value = aperture_value(uv, layer) * 4.0;
    value += aperture_value(uv + vec2(offset.x, 0.0), layer) * 2.0;
    value += aperture_value(uv - vec2(offset.x, 0.0), layer) * 2.0;
    value += aperture_value(uv + vec2(0.0, offset.y), layer) * 2.0;
    value += aperture_value(uv - vec2(0.0, offset.y), layer) * 2.0;
    value += aperture_value(uv + offset, layer);
    value += aperture_value(uv - offset, layer);
    value += aperture_value(uv + vec2(offset.x, -offset.y), layer);
    value += aperture_value(uv + vec2(-offset.x, offset.y), layer);
    return value / 16.0;
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
    center: vec3<f32>,
    end_scale: f32,
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
    let center_prism = prism_coordinates(
        center - a,
        edge_ab,
        edge_ac,
        direction,
        inverse_determinant,
    );
    let growth = (max(end_scale, 1.0) - 1.0) / extrusion_length;
    var interval = vec2(0.0, ray_length);
    interval = clip_lower_bound(
        interval,
        origin.x + center_prism.x * growth * origin.z,
        slope.x + center_prism.x * growth * slope.z,
    );
    interval = clip_lower_bound(
        interval,
        origin.y + center_prism.y * growth * origin.z,
        slope.y + center_prism.y * growth * slope.z,
    );
    interval = clip_lower_bound(
        interval,
        1.0 - origin.x - origin.y
            + (1.0 - center_prism.x - center_prism.y) * growth * origin.z,
        -slope.x - slope.y
            + (1.0 - center_prism.x - center_prism.y) * growth * slope.z,
    );
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

fn soft_sun_visibility(position: vec3<f32>, radius_texels: f32) -> f32 {
    let clip = settings.light_view_projection * vec4(position, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2(0.5, -0.5) + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || ndc.z < 0.0 || ndc.z > 1.0 {
        return 0.0;
    }
    let offset = radius_texels / vec2<f32>(textureDimensions(sun_shadow));
    let depth = ndc.z - 0.001;
    var visibility = textureSampleCompareLevel(sun_shadow, shadow_sampler, uv, depth) * 4.0;
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv + vec2(offset.x, 0.0), depth) * 2.0;
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv - vec2(offset.x, 0.0), depth) * 2.0;
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv + vec2(0.0, offset.y), depth) * 2.0;
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv - vec2(0.0, offset.y), depth) * 2.0;
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv + offset, depth);
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv - offset, depth);
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv + vec2(offset.x, -offset.y), depth);
    visibility += textureSampleCompareLevel(sun_shadow, shadow_sampler, uv + vec2(-offset.x, offset.y), depth);
    return visibility / 16.0;
}

@fragment
fn fragment_window_projection(input: PortalVertex) -> @location(0) vec4<f32> {
    if input.color.a <= 0.0 {
        return vec4(0.0);
    }
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    let depth = textureLoad(scene_depth, pixel, 0);
    if depth >= 1.0 {
        return vec4(0.0);
    }
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(dimensions);
    let clip = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), depth, 1.0);
    let world_h = settings.inverse_view_projection * clip;
    let world = world_h.xyz / world_h.w;
    let edge_ab = input.b.xyz - input.a.xyz;
    let edge_ac = input.c.xyz - input.a.xyz;
    let determinant = dot(edge_ab, cross(edge_ac, input.direction.xyz));
    if abs(determinant) < 0.0001 {
        return vec4(0.0);
    }
    let inverse_determinant = 1.0 / determinant;
    let prism = prism_coordinates(
        world - input.a.xyz,
        edge_ab,
        edge_ac,
        input.direction.xyz,
        inverse_determinant,
    );
    let extrusion_length = min(settings.distance_intensity_pixel.x * 0.5, 1500.0);
    if prism.z <= 2.0 || prism.z >= extrusion_length {
        return vec4(0.0);
    }
    let center_prism = prism_coordinates(
        input.center_scale.xyz - input.a.xyz,
        edge_ab,
        edge_ac,
        input.direction.xyz,
        inverse_determinant,
    );
    let along_shaft = prism.z / extrusion_length;
    let cross_section_scale = mix(1.0, max(input.center_scale.w, 1.0), along_shaft);
    let source_coordinates = center_prism.xy
        + (prism.xy - center_prism.xy) / cross_section_scale;
    let aperture_uv = input.uv_a.xy
        + (input.uv_b.xy - input.uv_a.xy) * source_coordinates.x
        + (input.uv_c.xy - input.uv_a.xy) * source_coordinates.y;
    let surface_edge = min(
        min(aperture_uv.x - input.uv_bounds.x, aperture_uv.y - input.uv_bounds.y),
        min(input.uv_bounds.z - aperture_uv.x, input.uv_bounds.w - aperture_uv.y),
    );
    let edge_width = max(fwidth(surface_edge) * mix(3.0, 12.0, along_shaft), 0.00001);
    let surface_coverage = smoothstep(-edge_width, edge_width, surface_edge);
    if surface_coverage <= 0.001 {
        return vec4(0.0);
    }
    let shadow_position = input.a.xyz
        + edge_ab * source_coordinates.x
        + edge_ac * source_coordinates.y
        + input.direction.xyz * prism.z;
    var receiver_normal = normalize(cross(dpdx(world), dpdy(world)));
    if dot(receiver_normal, settings.camera_position.xyz - world) < 0.0 {
        receiver_normal = -receiver_normal;
    }
    let incidence = max(dot(receiver_normal, -input.direction.xyz), 0.0);
    let brightness = soft_aperture_transmission(
        aperture_uv,
        input.direction.w,
        mix(1.5, 8.0, along_shaft),
    )
        * soft_sun_visibility(shadow_position, mix(2.0, 12.0, along_shaft))
        * incidence
        * surface_coverage
        * input.color.a
        * settings.shaft.z;
    let color = volumetric_saturated_color(
        input.color.rgb * aperture_color(aperture_uv, input.direction.w),
        settings.shaft.y,
    );
    return vec4(color * brightness, 0.0);
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
        input.center_scale.xyz,
        input.center_scale.w,
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
    let center_prism = prism_coordinates(
        input.center_scale.xyz - input.a.xyz,
        edge_ab,
        edge_ac,
        input.direction.xyz,
        inverse_determinant,
    );
    var scattering = vec3(0.0);
    var path_transmittance = 1.0;
    var aperture_sum = 0.0;
    var visibility_sum = 0.0;
    let phase = volumetric_henyey_greenstein(
        dot(input.direction.xyz, -ray_direction),
        settings.shaft.x,
    );
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
        let cross_section_scale = mix(1.0, max(input.center_scale.w, 1.0), along_shaft);
        let source_coordinates = center_prism.xy
            + (prism.xy - center_prism.xy) / cross_section_scale;
        let end_fade = 1.0 - smoothstep(0.65, 1.0, along_shaft);
        let aperture_uv = input.uv_a.xy
            + (input.uv_b.xy - input.uv_a.xy) * source_coordinates.x
            + (input.uv_c.xy - input.uv_a.xy) * source_coordinates.y;
        let surface_edge = min(
            min(aperture_uv.x - input.uv_bounds.x, aperture_uv.y - input.uv_bounds.y),
            min(input.uv_bounds.z - aperture_uv.x, input.uv_bounds.w - aperture_uv.y),
        );
        let edge_width = mix(1.5, 5.0, along_shaft) / 128.0;
        let surface_coverage = smoothstep(0.0, edge_width, surface_edge);
        let shadow_position = input.a.xyz
            + edge_ab * source_coordinates.x
            + edge_ac * source_coordinates.y
            + input.direction.xyz * prism.z;
        let visibility = sun_visibility(shadow_position);
        let transmission = aperture_transmission(aperture_uv, input.direction.w);
        let extinction = settings.direction_density.w
            * volumetric_dust(position, settings.camera_position.w, settings.haze);
        let segment_transmittance = volumetric_segment_transmittance(extinction, step_length);
        let color = volumetric_saturated_color(
            input.color.rgb * aperture_color(aperture_uv, input.direction.w),
            settings.shaft.y,
        );
        let incident_light = color * visibility * transmission * surface_coverage * end_fade;
        visibility_sum += visibility;
        aperture_sum += transmission;
        scattering += path_transmittance
            * incident_light
            * phase
            * 0.92
            * (1.0 - segment_transmittance);
        path_transmittance *= segment_transmittance;
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
    return vec4(scattering * settings.distance_intensity_pixel.y, 0.0);
}

struct DustMoteVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) world_position: vec3<f32>,
    @location(2) @interpolate(flat) color_fade: vec4<f32>,
    @location(3) @interpolate(flat) aperture_uv_layer: vec3<f32>,
    @location(4) @interpolate(flat) shadow_position: vec3<f32>,
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
    @location(8) center_scale: vec4<f32>,
    @location(9) uv_bounds: vec4<f32>,
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
    let phase = volumetric_hash(seed + vec3(71.0, 83.0, 97.0));
    let cross_section_scale = mix(1.0, max(center_scale.w, 1.0), phase);
    let drift_time = settings.camera_position.w * settings.dust.z * 0.12;
    let drift_seed = volumetric_hash(seed + vec3(41.0, 53.0, 67.0)) * 6.2831853;
    let drift_scale = mix(0.5, 2.5, volumetric_hash(seed + vec3(79.0, 89.0, 101.0)));
    let drift = vec3(
        sin(drift_time + drift_seed),
        cos(drift_time * 0.73 + drift_seed * 1.7),
        sin(drift_time * 0.51 + drift_seed * 2.3) * 0.5,
    ) * drift_scale;
    let world_position = center_scale.xyz
        + (base - center_scale.xyz) * cross_section_scale
        + direction.xyz * (phase * extrusion_length)
        + drift;
    let clip = settings.view_projection * vec4(world_position, 1.0);
    let size_random = volumetric_hash(seed + vec3(103.0, 113.0, 127.0));
    let radius = settings.dust.x * mix(0.12, 0.7, size_random * size_random * size_random);
    let aspect = mix(0.25, 0.85, volumetric_hash(seed + vec3(131.0, 137.0, 149.0)));
    let angle = volumetric_hash(seed + vec3(151.0, 163.0, 173.0)) * 6.2831853;
    let rotation = mat2x2<f32>(cos(angle), sin(angle), -sin(angle), cos(angle));
    var position = vec4(2.0, 2.0, 1.0, 1.0);
    if clip.w > 0.001 {
        let pixel_to_ndc = settings.distance_intensity_pixel.zw * 2.0;
        let offset_pixels = rotation
            * ((corner * 2.0 - vec2(1.0)) * vec2(radius, radius * aspect));
        position = vec4(clip.xy + offset_pixels * pixel_to_ndc * clip.w, clip.zw);
    }

    var output: DustMoteVertex;
    output.position = position;
    output.uv = corner;
    output.world_position = world_position;
    output.shadow_position = world_position;
    let brightness = mix(
        0.12,
        1.0,
        pow(volumetric_hash(seed + vec3(181.0, 191.0, 197.0)), 2.0),
    );
    output.color_fade = vec4(
        color.rgb,
        brightness * (1.0 - smoothstep(0.7, 1.0, phase)),
    );
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
    let radial = length(input.uv * 2.0 - vec2(1.0));
    let speck = exp2(-5.0 * radial * radial) * (1.0 - smoothstep(0.72, 1.0, radial));
    let brightness = speck
        * input.color_fade.a
        * sun_visibility(input.shadow_position)
        * aperture_transmission(input.aperture_uv_layer.xy, input.aperture_uv_layer.z)
        * settings.dust.y;
    let color = volumetric_saturated_color(
        input.color_fade.rgb * aperture_color(
            input.aperture_uv_layer.xy,
            input.aperture_uv_layer.z,
        ),
        settings.shaft.y,
    );
    return vec4(color * brightness * 2.0, 0.0);
}
