// Ported from SMAA v2.8; see SMAA-LICENSE.txt.

@group(0) @binding(0)
var color_texture: texture_2d<f32>;

@group(0) @binding(1)
var blend_texture: texture_2d<f32>;

@group(0) @binding(2)
var linear_sampler: sampler;

fn smaa_blend(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(blend_texture, linear_sampler, uv, 0.0);
}

@fragment
fn fragment_smaa_neighborhood(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(blend_texture));
    let right = smaa_blend(input.uv + vec2(texel.x, 0.0)).a;
    let top = smaa_blend(input.uv + vec2(0.0, texel.y)).g;
    let current = smaa_blend(input.uv);
    let weights = vec4(right, top, current.z, current.x);
    if dot(weights, vec4(1.0)) < 0.00001 {
        return textureSampleLevel(color_texture, linear_sampler, input.uv, 0.0);
    }

    let horizontal = max(weights.x, weights.z) > max(weights.y, weights.w);
    var offsets = vec4(0.0, weights.y, 0.0, weights.w);
    var blend = weights.yw;
    if horizontal {
        offsets = vec4(weights.x, 0.0, weights.z, 0.0);
        blend = weights.xz;
    }
    blend /= dot(blend, vec2(1.0));
    let coordinates = input.uv.xyxy + offsets * vec4(texel, -texel);
    return blend.x * textureSampleLevel(color_texture, linear_sampler, coordinates.xy, 0.0)
        + blend.y * textureSampleLevel(color_texture, linear_sampler, coordinates.zw, 0.0);
}
