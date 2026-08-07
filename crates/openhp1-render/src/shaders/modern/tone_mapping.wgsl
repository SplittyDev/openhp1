fn tone_map(color: vec3<f32>) -> vec3<f32> {
    switch settings.tone_mapper {
        case 0u: {
            return agx(color);
        }
        case 1u: {
            return reinhard(color);
        }
        default: {
            return aces(color);
        }
    }
}

fn reinhard(color: vec3<f32>) -> vec3<f32> {
    // Keep nominal UE1 white bright while retaining a short overbright shoulder.
    const WHITE = 1.25;
    const LUMINANCE = vec3(0.2126, 0.7152, 0.0722);
    let luminance = dot(color, LUMINANCE);
    if luminance <= 0.0 {
        return vec3(0.0);
    }
    let mapped_luminance =
        luminance * (1.0 + luminance / (WHITE * WHITE)) / (1.0 + luminance);
    return color * (mapped_luminance / luminance);
}

fn aces(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3(0.0),
        vec3(1.0),
    );
}

fn agx(color: vec3<f32>) -> vec3<f32> {
    var value = vec3(
        0.8424790623 * color.r + 0.0784336000 * color.g + 0.0791661275 * color.b,
        0.0423282423 * color.r + 0.8784686365 * color.g + 0.0791661275 * color.b,
        0.0423756549 * color.r + 0.0784336000 * color.g + 0.8791429738 * color.b,
    );
    const MIN_EV = -12.47393;
    const MAX_EV = 4.026069;
    value = clamp(
        (log2(max(value, vec3(1e-10))) - MIN_EV) / (MAX_EV - MIN_EV),
        vec3(0.0),
        vec3(1.0),
    );
    value = agx_contrast(value);
    value = vec3(
        1.1968790051 * value.r - 0.0980208811 * value.g - 0.0990297441 * value.b,
        -0.0528968518 * value.r + 1.1519031299 * value.g - 0.0989611768 * value.b,
        -0.0529716355 * value.r - 0.0980434501 * value.g + 1.1510736726 * value.b,
    );
    return clamp(pow(max(value, vec3(0.0)), vec3(2.2)), vec3(0.0), vec3(1.0));
}

fn agx_contrast(value: vec3<f32>) -> vec3<f32> {
    let value2 = value * value;
    let value4 = value2 * value2;
    return 15.5 * value4 * value2
        - 40.14 * value4 * value
        + 31.96 * value4
        - 6.868 * value2 * value
        + 0.4298 * value2
        + 0.1191 * value
        - 0.00232;
}
