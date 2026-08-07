struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertex {
    let positions = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0),
    );
    var output: FullscreenVertex;
    output.position = vec4(positions[vertex_index], 0.0, 1.0);
    output.uv = positions[vertex_index] * vec2(0.5, -0.5) + vec2(0.5);
    return output;
}
