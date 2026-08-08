struct FroxelSettings {
    inverse_view_projection: mat4x4<f32>,
    light_view_projections: array<mat4x4<f32>, 4>,
    camera_position_time: vec4<f32>,
    volume_size_portals: vec4<u32>,
    distance_density: vec4<f32>,
    haze: vec4<f32>,
};

struct PortalTriangle {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    color: vec4<f32>,
    direction: vec4<f32>,
    uv_a: vec4<f32>,
    uv_b: vec4<f32>,
    uv_c: vec4<f32>,
    center_scale: vec4<f32>,
    uv_bounds: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> settings: FroxelSettings;

@group(0) @binding(1)
var<storage, read> portals: array<PortalTriangle>;

@group(0) @binding(2)
var aperture_masks: texture_2d_array<f32>;

@group(0) @binding(3)
var aperture_sampler: sampler;

@group(0) @binding(4)
var sun_shadows: texture_depth_2d_array;

@group(0) @binding(5)
var sun_shadow_sampler: sampler_comparison;

@group(0) @binding(6)
var integrated_volume: texture_storage_3d<rgba16float, write>;

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

fn aperture_transmission(uv: vec2<f32>, layer: i32) -> f32 {
    return textureSampleLevel(aperture_masks, aperture_sampler, uv, layer, 0.0).r;
}

fn sun_visibility(position: vec3<f32>, layer: i32) -> f32 {
    let clip = settings.light_view_projections[layer] * vec4(position, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2(0.5, -0.5) + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || ndc.z < 0.0 || ndc.z > 1.0 {
        return 0.0;
    }
    return textureSampleCompareLevel(
        sun_shadows,
        sun_shadow_sampler,
        uv,
        layer,
        ndc.z - 0.001,
    );
}

fn portal_light(portal: PortalTriangle, position: vec3<f32>, view_direction: vec3<f32>) -> vec3<f32> {
    let edge_ab = portal.b.xyz - portal.a.xyz;
    let edge_ac = portal.c.xyz - portal.a.xyz;
    let direction = normalize(portal.direction.xyz);
    let determinant = dot(edge_ab, cross(edge_ac, direction));
    if abs(determinant) < 0.0001 {
        return vec3(0.0);
    }
    let inverse_determinant = 1.0 / determinant;
    let prism = prism_coordinates(
        position - portal.a.xyz,
        edge_ab,
        edge_ac,
        direction,
        inverse_determinant,
    );
    let extrusion_length = settings.distance_density.y;
    if prism.z <= 0.0 || prism.z >= extrusion_length {
        return vec3(0.0);
    }
    let center_prism = prism_coordinates(
        portal.center_scale.xyz - portal.a.xyz,
        edge_ab,
        edge_ac,
        direction,
        inverse_determinant,
    );
    let along_shaft = prism.z / extrusion_length;
    let cross_section_scale = mix(1.0, max(portal.center_scale.w, 1.0), along_shaft);
    let source_coordinates = center_prism.xy
        + (prism.xy - center_prism.xy) / cross_section_scale;
    if source_coordinates.x < 0.0
        || source_coordinates.y < 0.0
        || source_coordinates.x + source_coordinates.y > 1.0 {
        return vec3(0.0);
    }
    let aperture_uv = portal.uv_a.xy
        + (portal.uv_b.xy - portal.uv_a.xy) * source_coordinates.x
        + (portal.uv_c.xy - portal.uv_a.xy) * source_coordinates.y;
    let surface_edge = min(
        min(aperture_uv.x - portal.uv_bounds.x, aperture_uv.y - portal.uv_bounds.y),
        min(portal.uv_bounds.z - aperture_uv.x, portal.uv_bounds.w - aperture_uv.y),
    );
    let surface_coverage = smoothstep(
        0.0,
        mix(1.5, 6.0, along_shaft) / 128.0,
        surface_edge,
    );
    if surface_coverage <= 0.0 {
        return vec3(0.0);
    }
    let shadow_position = portal.a.xyz
        + edge_ab * source_coordinates.x
        + edge_ac * source_coordinates.y
        + direction * prism.z;
    let layer = i32(portal.uv_a.z + 0.5);
    let phase = volumetric_henyey_greenstein(dot(direction, -view_direction), 0.25);
    let end_fade = 1.0 - smoothstep(0.65, 1.0, along_shaft);
    return portal.color.rgb
        * aperture_transmission(aperture_uv, i32(portal.direction.w + 0.5))
        * sun_visibility(shadow_position, layer)
        * surface_coverage
        * end_fade
        * phase
        * portal.uv_a.w;
}

fn slice_distance(slice: f32) -> f32 {
    let near = settings.distance_density.x;
    let far = settings.distance_density.y;
    return near * pow(far / near, slice / f32(settings.volume_size_portals.z));
}

@compute @workgroup_size(8, 8, 1)
fn compute_froxel(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= settings.volume_size_portals.xy) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2(0.5)) / vec2<f32>(settings.volume_size_portals.xy);
    let clip = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 1.0, 1.0);
    let far_h = settings.inverse_view_projection * clip;
    let ray_direction = normalize(
        far_h.xyz / far_h.w - settings.camera_position_time.xyz,
    );
    var scattering = vec3(0.0);
    var path_transmittance = 1.0;
    for (var z = 0u; z < settings.volume_size_portals.z; z += 1u) {
        let segment_start = slice_distance(f32(z));
        let segment_end = slice_distance(f32(z + 1u));
        let step_length = segment_end - segment_start;
        let position = settings.camera_position_time.xyz
            + ray_direction * ((segment_start + segment_end) * 0.5);
        var incident_light = vec3(0.0);
        for (var portal_index = 0u; portal_index < settings.volume_size_portals.w; portal_index += 1u) {
            incident_light += portal_light(portals[portal_index], position, ray_direction);
        }
        let extinction = settings.distance_density.z
            * volumetric_dust(position, settings.camera_position_time.w, settings.haze);
        let segment_transmittance = volumetric_segment_transmittance(extinction, step_length);
        scattering += path_transmittance
            * incident_light
            * 0.92
            * (1.0 - segment_transmittance)
            * settings.distance_density.w;
        path_transmittance *= segment_transmittance;
        textureStore(
            integrated_volume,
            vec3<i32>(vec2<i32>(id.xy), i32(z)),
            vec4(scattering, 1.0 - path_transmittance),
        );
    }
}

@group(1) @binding(0)
var scene_depth: texture_depth_2d;

@group(1) @binding(1)
var integrated_scattering: texture_3d<f32>;

@group(1) @binding(2)
var volume_sampler: sampler;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex_froxel_composite(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertex {
    const POSITIONS = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0),
    );
    let position = POSITIONS[vertex_index];
    var output: FullscreenVertex;
    output.position = vec4(position, 0.0, 1.0);
    output.uv = position * vec2(0.5, -0.5) + vec2(0.5);
    return output;
}

@fragment
fn fragment_froxel_composite(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    let depth = textureLoad(scene_depth, pixel, 0);
    let clip = vec4(input.uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), depth, 1.0);
    let world_h = settings.inverse_view_projection * clip;
    let world = world_h.xyz / world_h.w;
    let distance = clamp(
        length(world - settings.camera_position_time.xyz),
        settings.distance_density.x,
        settings.distance_density.y,
    );
    let volume_z = log(distance / settings.distance_density.x)
        / log(settings.distance_density.y / settings.distance_density.x);
    let value = textureSampleLevel(
        integrated_scattering,
        volume_sampler,
        vec3(input.uv, clamp(volume_z, 0.0, 1.0)),
        0.0,
    );
    return vec4(value.rgb, 0.0);
}
