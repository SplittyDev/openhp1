struct ClassicDisplay {
    gamma: vec4<f32>,
};

@group(0) @binding(0)
var scene_texture: texture_2d<f32>;

@group(0) @binding(1)
var scene_sampler: sampler;

@group(0) @binding(2)
var<uniform> display: ClassicDisplay;

@fragment
fn fragment_classic_display(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(scene_texture, scene_sampler, input.uv, 0.0);
    return vec4(pow(max(scene.rgb, vec3(0.0)), vec3(display.gamma.x)), scene.a);
}
