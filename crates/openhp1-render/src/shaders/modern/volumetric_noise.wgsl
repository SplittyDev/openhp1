fn volumetric_hash(cell: vec3<f32>) -> f32 {
    return fract(sin(dot(cell, vec3(127.1, 311.7, 74.7))) * 43758.5453);
}

fn volumetric_noise(point: vec3<f32>) -> f32 {
    let cell = floor(point);
    let fraction = fract(point);
    let blend = fraction * fraction * (vec3(3.0) - 2.0 * fraction);
    let x00 = mix(volumetric_hash(cell), volumetric_hash(cell + vec3(1.0, 0.0, 0.0)), blend.x);
    let x10 = mix(volumetric_hash(cell + vec3(0.0, 1.0, 0.0)), volumetric_hash(cell + vec3(1.0, 1.0, 0.0)), blend.x);
    let x01 = mix(volumetric_hash(cell + vec3(0.0, 0.0, 1.0)), volumetric_hash(cell + vec3(1.0, 0.0, 1.0)), blend.x);
    let x11 = mix(volumetric_hash(cell + vec3(0.0, 1.0, 1.0)), volumetric_hash(cell + vec3(1.0, 1.0, 1.0)), blend.x);
    return mix(mix(x00, x10, blend.y), mix(x01, x11, blend.y), blend.z);
}

fn volumetric_dust(position: vec3<f32>, time: f32, settings: vec4<f32>) -> f32 {
    let direction = normalize(vec3(3.2, -1.4, 2.2));
    let drift = direction * time * settings.w;
    let point = (position + drift) / max(settings.x, 0.001);
    let haze = volumetric_noise(point) * 0.72
        + volumetric_noise(point * 2.17 + vec3(19.0, 43.0, 71.0)) * 0.28;
    return mix(1.0, haze * 2.0, settings.z);
}

fn volumetric_henyey_greenstein(cosine: f32, anisotropy: f32) -> f32 {
    let g = clamp(anisotropy, -0.95, 0.95);
    let denominator = max(1.0 + g * g - 2.0 * g * cosine, 0.0001);
    return (1.0 - g * g) / (12.5663706 * denominator * sqrt(denominator));
}

fn volumetric_segment_transmittance(extinction: f32, distance: f32) -> f32 {
    return exp(-max(extinction, 0.0) * max(distance, 0.0));
}
