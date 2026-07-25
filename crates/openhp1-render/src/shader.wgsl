struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var color_texture: texture_2d<f32>;

@group(1) @binding(1)
var color_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) world_position: vec3<f32>,
};

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) texture_coordinates: vec2<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    output.texture_coordinates = texture_coordinates;
    output.world_position = position;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return shade(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
}

@fragment
fn fragment_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return shade(input, color);
}

fn shade(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    let normal = normalize(cross(dpdx(input.world_position), dpdy(input.world_position)));
    let light = normalize(vec3(0.45, 0.8, 0.3));
    let diffuse = 0.55 + 0.45 * abs(dot(normal, light));
    return vec4(color.rgb * diffuse, 1.0);
}
