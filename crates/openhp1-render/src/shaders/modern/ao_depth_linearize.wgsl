@group(0) @binding(0)
var scene_depth: texture_depth_2d;

@group(0) @binding(1)
var<uniform> settings: AoSettings;

@fragment
fn fragment_linearize_depth(input: FullscreenVertex) -> @location(0) f32 {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), dimensions);
    return linear_depth(textureLoad(scene_depth, pixel, 0));
}
