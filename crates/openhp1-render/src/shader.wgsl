struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world_position: vec3<f32>,
};

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    output.color = color;
    output.world_position = position;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(cross(dpdx(input.world_position), dpdy(input.world_position)));
    let light = normalize(vec3(0.45, 0.8, 0.3));
    let diffuse = 0.3 + 0.7 * abs(dot(normal, light));
    return vec4(input.color * diffuse, 1.0);
}
