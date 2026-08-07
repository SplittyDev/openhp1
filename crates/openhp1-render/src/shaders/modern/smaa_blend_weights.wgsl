// Ported from the SMAA v2.8 medium preset; see SMAA-LICENSE.txt.

@group(0) @binding(0)
var edges_texture: texture_2d<f32>;

@group(0) @binding(1)
var area_texture: texture_2d<f32>;

@group(0) @binding(2)
var search_texture: texture_2d<f32>;

@group(0) @binding(3)
var linear_sampler: sampler;

fn smaa_edges(uv: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(edges_texture, linear_sampler, uv, 0.0).rg;
}

fn smaa_search_length(edges: vec2<f32>, offset: f32) -> f32 {
    var scale = vec2(66.0, 33.0) * vec2(0.5, -1.0);
    var bias = vec2(66.0, 33.0) * vec2(offset, 1.0);
    scale += vec2(-1.0, 1.0);
    bias += vec2(0.5, -0.5);
    return textureSampleLevel(
        search_texture,
        linear_sampler,
        (scale * edges + bias) / vec2(64.0, 16.0),
        0.0,
    ).r;
}

fn smaa_search_x_left(start: vec2<f32>, end: f32, texel: vec2<f32>) -> f32 {
    var uv = start;
    var edges = vec2(0.0, 1.0);
    for (var step = 0u; step < 8u; step++) {
        if !(uv.x > end && edges.g > 0.8281 && edges.r == 0.0) {
            break;
        }
        edges = smaa_edges(uv);
        uv -= vec2(2.0 * texel.x, 0.0);
    }
    let offset = -(255.0 / 127.0) * smaa_search_length(edges, 0.0) + 3.25;
    return uv.x + texel.x * offset;
}

fn smaa_search_x_right(start: vec2<f32>, end: f32, texel: vec2<f32>) -> f32 {
    var uv = start;
    var edges = vec2(0.0, 1.0);
    for (var step = 0u; step < 8u; step++) {
        if !(uv.x < end && edges.g > 0.8281 && edges.r == 0.0) {
            break;
        }
        edges = smaa_edges(uv);
        uv += vec2(2.0 * texel.x, 0.0);
    }
    let offset = -(255.0 / 127.0) * smaa_search_length(edges, 0.5) + 3.25;
    return uv.x - texel.x * offset;
}

fn smaa_search_y_up(start: vec2<f32>, end: f32, texel: vec2<f32>) -> f32 {
    var uv = start;
    var edges = vec2(1.0, 0.0);
    for (var step = 0u; step < 8u; step++) {
        if !(uv.y > end && edges.r > 0.8281 && edges.g == 0.0) {
            break;
        }
        edges = smaa_edges(uv);
        uv -= vec2(0.0, 2.0 * texel.y);
    }
    let offset = -(255.0 / 127.0) * smaa_search_length(edges.gr, 0.0) + 3.25;
    return uv.y + texel.y * offset;
}

fn smaa_search_y_down(start: vec2<f32>, end: f32, texel: vec2<f32>) -> f32 {
    var uv = start;
    var edges = vec2(1.0, 0.0);
    for (var step = 0u; step < 8u; step++) {
        if !(uv.y < end && edges.r > 0.8281 && edges.g == 0.0) {
            break;
        }
        edges = smaa_edges(uv);
        uv += vec2(0.0, 2.0 * texel.y);
    }
    let offset = -(255.0 / 127.0) * smaa_search_length(edges.gr, 0.5) + 3.25;
    return uv.y - texel.y * offset;
}

fn smaa_area(distance: vec2<f32>, crossing_left: f32, crossing_right: f32) -> vec2<f32> {
    let lookup = vec2(16.0) * round(4.0 * vec2(crossing_left, crossing_right)) + distance;
    let pixel_size = 1.0 / vec2(160.0, 560.0);
    return textureSampleLevel(area_texture, linear_sampler, pixel_size * (lookup + 0.5), 0.0).rg;
}

@fragment
fn fragment_smaa_blend_weights(input: FullscreenVertex) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(edges_texture));
    let texel = 1.0 / dimensions;
    let pixel = input.uv * dimensions;
    let offset0 = input.uv.xyxy + texel.xyxy * vec4(-0.25, -0.125, 1.25, -0.125);
    let offset1 = input.uv.xyxy + texel.xyxy * vec4(-0.125, -0.25, -0.125, 1.25);
    let ends = vec4(
        offset0.x - 16.0 * texel.x,
        offset0.z + 16.0 * texel.x,
        offset1.y - 16.0 * texel.y,
        offset1.w + 16.0 * texel.y,
    );
    let edges = smaa_edges(input.uv);
    var weights = vec4(0.0);

    if edges.g > 0.0 {
        let left = smaa_search_x_left(offset0.xy, ends.x, texel);
        let crossing_y = offset1.y;
        let crossing_left = smaa_edges(vec2(left, crossing_y)).r;
        let right = smaa_search_x_right(offset0.zw, ends.y, texel);
        let distance = abs(round(vec2(left, right) * dimensions.x - vec2(pixel.x)));
        let crossing_right = smaa_edges(vec2(right + texel.x, crossing_y)).r;
        let horizontal = smaa_area(sqrt(distance), crossing_left, crossing_right);
        weights.r = horizontal.r;
        weights.g = horizontal.g;
    }

    if edges.r > 0.0 {
        let top = smaa_search_y_up(offset1.xy, ends.z, texel);
        let crossing_x = offset0.x;
        let crossing_top = smaa_edges(vec2(crossing_x, top)).g;
        let bottom = smaa_search_y_down(offset1.zw, ends.w, texel);
        let distance = abs(round(vec2(top, bottom) * dimensions.y - vec2(pixel.y)));
        let crossing_bottom = smaa_edges(vec2(crossing_x, bottom + texel.y)).g;
        let vertical = smaa_area(sqrt(distance), crossing_top, crossing_bottom);
        weights.b = vertical.r;
        weights.a = vertical.g;
    }

    return weights;
}
