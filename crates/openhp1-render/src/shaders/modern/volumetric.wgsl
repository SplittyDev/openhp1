struct VolumetricSettings {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    camera_forward: vec4<f32>,
    projection: vec4<f32>,
};

@group(0) @binding(0)
var scene_depth: texture_depth_2d;

@group(0) @binding(1)
var<uniform> settings: VolumetricSettings;

struct VolumeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) position_radius: vec4<f32>,
    @location(1) @interpolate(flat) color_fog: vec4<f32>,
    @location(2) @interpolate(flat) profile: vec4<f32>,
};

@vertex
fn vertex_volume(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position_radius: vec4<f32>,
    @location(1) color_fog: vec4<f32>,
    @location(2) profile: vec4<f32>,
) -> VolumeOutput {
    const CORNERS = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2(1.0, -1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
    );
    let radius = position_radius.w;
    let camera_offset = position_radius.xyz - settings.camera_position.xyz;
    let camera_distance = dot(camera_offset, settings.camera_forward.xyz);
    var center = vec2(3.0);
    var extent = vec2(0.0);
    if camera_distance + radius > settings.projection.z {
        if camera_distance <= radius {
            center = vec2(0.0);
            extent = vec2(2.0);
        } else {
            let clip = settings.view_projection * vec4(position_radius.xyz, 1.0);
            center = clip.xy / clip.w;
            extent = radius / (camera_distance * settings.projection.xy);
        }
    }
    var output: VolumeOutput;
    output.position = vec4(center + CORNERS[vertex_index] * extent, 0.0, 1.0);
    output.position_radius = position_radius;
    output.color_fog = color_fog;
    output.profile = profile;
    return output;
}

@fragment
fn fragment_volume(input: VolumeOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp(vec2<i32>(input.position.xy), vec2(0), dimensions - vec2(1));
    let depth = textureLoad(scene_depth, pixel, 0);
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(dimensions);
    let clip = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), depth, 1.0);
    let world_h = settings.inverse_view_projection * clip;
    let world = world_h.xyz / world_h.w;
    let ray = world - settings.camera_position.xyz;
    let ray_length = length(ray);
    if ray_length <= 0.0001 {
        discard;
    }
    let ray_direction = ray / ray_length;
    let radius = input.position_radius.w;
    let normalized_depth = ray_length / radius;
    let ray_origin = (settings.camera_position.xyz - input.position_radius.xyz) / radius;
    let b = dot(ray_direction, ray_origin);
    let c = dot(ray_origin, ray_origin) - 1.0;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        discard;
    }
    let half_chord = sqrt(discriminant);
    var start = -b - half_chord;
    var end = -b + half_chord;
    if end < 0.0 || start > normalized_depth {
        discard;
    }
    start = max(start, 0.0);
    end = min(end, normalized_depth);

    var density = 0.0;
    if input.profile.x > 0.5 {
        let perpendicular_squared = max(dot(ray_origin, ray_origin) - b * b, 0.0);
        let softened_distance = sqrt(perpendicular_squared + 0.0036);
        density = max(
            (atan((end + b) / softened_distance)
                - atan((start + b) / softened_distance))
                / softened_distance,
            0.0,
        );
    } else {
        let integral_start = -(c * start + b * start * start + start * start * start / 3.0);
        let integral_end = -(c * end + b * end * end + end * end * end / 3.0);
        density = max((integral_end - integral_start) * 0.75, 0.0);
    }
    return vec4(
        input.color_fog.rgb * density,
        min(density * input.color_fog.a, 1.0),
    );
}
