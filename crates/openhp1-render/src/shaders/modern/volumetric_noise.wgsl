fn volumetric_hash(cell: vec3<f32>) -> f32 {
    return fract(sin(dot(cell, vec3(127.1, 311.7, 74.7))) * 43758.5453);
}

fn volumetric_dust(position: vec3<f32>, time: f32) -> f32 {
    let point = position * 0.006 + vec3(time * 0.025, -time * 0.01, time * 0.015);
    let cell = floor(point);
    let fraction = fract(point);
    let blend = fraction * fraction * (vec3(3.0) - 2.0 * fraction);
    let x00 = mix(volumetric_hash(cell), volumetric_hash(cell + vec3(1.0, 0.0, 0.0)), blend.x);
    let x10 = mix(volumetric_hash(cell + vec3(0.0, 1.0, 0.0)), volumetric_hash(cell + vec3(1.0, 1.0, 0.0)), blend.x);
    let x01 = mix(volumetric_hash(cell + vec3(0.0, 0.0, 1.0)), volumetric_hash(cell + vec3(1.0, 0.0, 1.0)), blend.x);
    let x11 = mix(volumetric_hash(cell + vec3(0.0, 1.0, 1.0)), volumetric_hash(cell + vec3(1.0, 1.0, 1.0)), blend.x);
    return mix(0.88, 1.12, mix(mix(x00, x10, blend.y), mix(x01, x11, blend.y), blend.z));
}
