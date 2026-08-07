struct Camera {
    view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var color_texture: texture_2d<f32>;

@group(1) @binding(1)
var color_sampler: sampler;

// UE1 corona actors have authored color and size but no physical luminance.
// This modern-only gain gives their Skin texture enough HDR range to bloom.
const CORONA_HDR_GAIN = 4.0;

struct CoronaOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) color: vec3<f32>,
};

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
fn fragment_corona(input: CoronaOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    return vec4(color.rgb * input.color * color.a * CORONA_HDR_GAIN, color.a);
}
