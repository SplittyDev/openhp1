struct Camera {
    view_projection: mat4x4<f32>,
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

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) lightmap_coordinates: vec2<f32>,
    @location(2) has_lightmap: f32,
};

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) texture_coordinates: vec2<f32>,
    @location(2) lightmap_coordinates: vec2<f32>,
    @location(3) has_lightmap: f32,
) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    output.texture_coordinates = texture_coordinates;
    output.lightmap_coordinates = lightmap_coordinates;
    output.has_lightmap = has_lightmap;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return apply_lightmap(input, linear_color(textureSample(color_texture, color_sampler, input.texture_coordinates)));
}

@fragment
fn fragment_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return apply_lightmap(input, linear_color(color));
}

@fragment
fn fragment_unlit(input: VertexOutput) -> @location(0) vec4<f32> {
    return linear_color(textureSample(color_texture, color_sampler, input.texture_coordinates));
}

@fragment
fn fragment_unlit_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return linear_color(color);
}

@fragment
fn fragment_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    return apply_lightmap(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
}

@fragment
fn fragment_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    if color.a < 0.5 {
        discard;
    }
    return apply_lightmap(input, color);
}

@fragment
fn fragment_backdrop(input: VertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(color_texture));
    return textureSample(color_texture, color_sampler, input.clip_position.xy / dimensions);
}

fn linear_color(color: vec4<f32>) -> vec4<f32> {
    let low = color.rgb / 12.92;
    let high = pow((color.rgb + 0.055) / 1.055, vec3(2.4));
    return vec4(select(low, high, color.rgb > vec3(0.04045)), color.a);
}

fn apply_lightmap(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    let light = textureSample(
        lightmap_texture,
        lightmap_sampler,
        input.lightmap_coordinates,
    ).rgb * 2.0;
    return vec4(color.rgb * mix(vec3(1.0), light, input.has_lightmap), color.a);
}
