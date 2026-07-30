struct Camera {
    view_projection: mat4x4<f32>,
    world_to_view: mat4x4<f32>,
    camera_position: vec4<f32>,
    display_gamma: vec4<f32>,
    auto_uv: vec4<f32>,
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var color_texture: texture_2d<f32>;

@group(1) @binding(1)
var color_sampler: sampler;

@group(1) @binding(2)
var lightmap_texture: texture_2d<f32>;

@group(1) @binding(3)
var lightmap_sampler: sampler;

// UE1 corona actors have authored color and size but no physical luminance.
// This modern-only gain gives their Skin texture enough HDR range to bloom.
const CORONA_HDR_GAIN = 4.0;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) lightmap_coordinates: vec2<f32>,
    @location(2) has_lightmap: f32,
    @location(3) vertex_color: vec4<f32>,
};

struct CoronaOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) texture_coordinates: vec2<f32>,
    @location(2) texture_pan_speed: vec2<f32>,
    @location(3) lightmap_coordinates: vec2<f32>,
    @location(4) has_lightmap: f32,
    @location(5) vertex_color: vec4<f32>,
    @location(6) normal: vec3<f32>,
    @location(7) environment_map: f32,
) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    if environment_map > 0.5 {
        let incident_offset = position - camera.camera_position.xyz;
        let incident = incident_offset * inverseSqrt(max(dot(incident_offset, incident_offset), 0.00000001));
        let world_normal = normal * inverseSqrt(max(dot(normal, normal), 0.00000001));
        let reflection = reflect(incident, world_normal);
        let world_to_view = mat3x3<f32>(
            camera.world_to_view[0].xyz,
            camera.world_to_view[1].xyz,
            camera.world_to_view[2].xyz,
        );
        let view_reflection = world_to_view * reflection;
        output.texture_coordinates = (view_reflection.xy + vec2(1.0)) * (128.0 / 255.0);
    } else {
        output.texture_coordinates = texture_coordinates + texture_pan_speed * camera.auto_uv.x;
    }
    output.lightmap_coordinates = lightmap_coordinates;
    output.has_lightmap = has_lightmap;
    output.vertex_color = vertex_color;
    return output;
}

@vertex
fn vertex_corona(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color_and_scale: vec4<f32>,
) -> CoronaOutput {
    let corners = array<vec2<f32>, 6>(
        vec2(-0.5, -0.5),
        vec2(0.5, -0.5),
        vec2(-0.5, 0.5),
        vec2(-0.5, 0.5),
        vec2(0.5, -0.5),
        vec2(0.5, 0.5),
    );
    let corner = corners[vertex_index];
    var output: CoronaOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    let aspect = camera.viewport.x / max(camera.viewport.y, 1.0);
    // ponytail: UE1 used a fixed viewport fraction. Modern mode attenuates
    // after 512 units and clamps the near size until physical emitters exist.
    let distance_scale = clamp(512.0 / max(output.clip_position.w, 1.0), 0.1, 1.25);
    let screen_offset =
        corner * vec2(1.6, 1.6 * aspect) * color_and_scale.w * distance_scale;
    output.clip_position.x += screen_offset.x * output.clip_position.w;
    output.clip_position.y += screen_offset.y * output.clip_position.w;
    output.texture_coordinates = corner + vec2(0.5);
    output.color = color_and_scale.rgb;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return apply_display_gamma(apply_lightmap(input, textureSample(color_texture, color_sampler, input.texture_coordinates)));
}

@fragment
fn fragment_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return apply_display_gamma(apply_lightmap(input, color));
}

@fragment
fn fragment_unlit(input: VertexOutput) -> @location(0) vec4<f32> {
    return apply_display_gamma(apply_vertex_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates)));
}

@fragment
fn fragment_unlit_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return apply_display_gamma(apply_vertex_light(input, color));
}

@fragment
fn fragment_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_lightmap(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return apply_display_gamma(apply_opacity(input, color));
}

@fragment
fn fragment_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return apply_display_gamma(apply_opacity(input, apply_lightmap(input, color)));
}

@fragment
fn fragment_backdrop(input: VertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(color_texture));
    return textureSample(color_texture, color_sampler, input.clip_position.xy / dimensions);
}

@fragment
fn fragment_corona(input: CoronaOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    return vec4(color.rgb * input.color * color.a * CORONA_HDR_GAIN, color.a);
}

fn apply_lightmap(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    let light = textureSample(
        lightmap_texture,
        lightmap_sampler,
        input.lightmap_coordinates,
    ).rgb * 2.0;
    return vec4(color.rgb * mix(input.vertex_color.rgb, light, input.has_lightmap), color.a);
}

fn apply_vertex_light(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    return vec4(color.rgb * input.vertex_color.rgb, color.a);
}

fn apply_opacity(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    // ponytail: HP's Opacity has no licensed renderer reference. Treat it as
    // a clamped multiplier of the full translucent source color until traces
    // disprove it; UE1's One/OneMinusSrcColor blend ignores alpha for RGB.
    return color * input.vertex_color.a;
}

fn apply_display_gamma(color: vec4<f32>) -> vec4<f32> {
    return vec4(pow(max(color.rgb, vec3(0.0)), vec3(camera.display_gamma.x)), color.a);
}
