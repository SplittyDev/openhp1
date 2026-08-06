@group(0) @binding(0)
var frame_texture: texture_2d<f32>;

@group(0) @binding(1)
var frame_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex_fullscreen(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0),
    );
    var output: VertexOutput;
    output.position = vec4(positions[vertex_index], 0.0, 1.0);
    output.uv = positions[vertex_index] * vec2(0.5, -0.5) + vec2(0.5);
    return output;
}

@fragment
fn fragment_true_color(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, input.uv);
}

@fragment
fn fragment_rgb565(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(frame_texture, frame_sampler, input.uv);
    let levels = vec3(31.0, 63.0, 31.0);
    return vec4(round(clamp(color.rgb, vec3(0.0), vec3(1.0)) * levels) / levels, color.a);
}
