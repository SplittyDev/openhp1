// Copyright (C) 2016-2021, Intel Corporation
// SPDX-License-Identifier: MIT
// WGSL adaptation of XeGTAO v1.30, without bent normals or temporal noise.

const PI = 3.141592653589793;
const HALF_PI = 1.5707963267948966;
const RADIUS_MULTIPLIER = 1.457;
const FALLOFF_RANGE = 0.615;
const SAMPLE_DISTRIBUTION_POWER = 2.0;
const FINAL_VALUE_POWER = 2.2;
const DEPTH_MIP_SAMPLING_OFFSET = 3.30;

@fragment
fn fragment_xegtao(input: FullscreenVertex) -> AoOutput {
    let dimensions = vec2<i32>(textureDimensions(scene_depth));
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), dimensions);
    if textureLoad(scene_depth, pixel, 0) >= 0.99999 {
        return ao_output(1.0, vec4(0.0));
    }

    let depths = depth_neighborhood(pixel);
    let edges = calculate_edges(depths[0], depths[1], depths[2], depths[3], depths[4]);
    let normal = depth_normal(pixel, depths, edges);
    let center = view_position(pixel, depths[0] * 0.99999);
    let view_vector = normalize(-center);
    let noise = spatial_noise(vec2<u32>(pixel));
    let effect_radius = settings.effect_radius * RADIUS_MULTIPLIER;
    let pixel_view_size = depths[0] * 2.0 * settings.tan_half_fov.x * settings.inverse_viewport.x;
    let screen_radius = effect_radius / max(pixel_view_size, 0.0001);
    let minimum_sample = 1.3 / max(screen_radius, 0.0001);
    let falloff_length = FALLOFF_RANGE * effect_radius;
    let falloff_from = effect_radius - falloff_length;
    let falloff_mul = -1.0 / max(falloff_length, 0.0001);
    let falloff_add = falloff_from / max(falloff_length, 0.0001) + 1.0;

    var visibility = clamp((10.0 - screen_radius) / 100.0, 0.0, 1.0) * 0.5;
    for (var slice = 0u; slice < 3u; slice += 1u) {
        let phi = (f32(slice) + noise.x) / 3.0 * PI;
        let cos_phi = cos(phi);
        let sin_phi = sin(phi);
        let direction = vec3(cos_phi, sin_phi, 0.0);
        let slice_offset = vec2(cos_phi, -sin_phi) * screen_radius;
        let orthogonal = direction - dot(direction, view_vector) * view_vector;
        let axis = normalize(cross(orthogonal, view_vector));
        var projected_normal = normal - axis * dot(normal, axis);
        var projected_length = max(length(projected_normal), 0.0001);
        let sign_normal = sign(dot(orthogonal, projected_normal));
        let cos_normal = clamp(dot(projected_normal, view_vector) / projected_length, 0.0, 1.0);
        let normal_angle = sign_normal * acos(cos_normal);
        let low_horizon0 = cos(normal_angle + HALF_PI);
        let low_horizon1 = cos(normal_angle - HALF_PI);
        var horizon0 = low_horizon0;
        var horizon1 = low_horizon1;

        for (var step = 0u; step < 3u; step += 1u) {
            let base_noise = f32(slice + step * 3u) * 0.6180339887498948;
            let step_noise = fract(noise.y + base_noise);
            var sample_fraction = (f32(step) + step_noise) / 3.0;
            sample_fraction = pow(sample_fraction, SAMPLE_DISTRIBUTION_POWER) + minimum_sample;
            let sample_offset = sample_fraction * slice_offset;
            let offset_length = max(length(sample_offset), 1.0);
            let mip_level = u32(clamp(
                round(log2(offset_length) - DEPTH_MIP_SAMPLING_OFFSET),
                0.0,
                f32(settings.depth_mip_count - 1u),
            ));
            let offset = vec2<i32>(round(sample_offset));
            let pixel0 = clamp_pixel(pixel + offset, dimensions);
            let pixel1 = clamp_pixel(pixel - offset, dimensions);
            let sample0 = view_position(pixel0, sample_view_depth(pixel0, mip_level));
            let sample1 = view_position(pixel1, sample_view_depth(pixel1, mip_level));
            let delta0 = sample0 - center;
            let delta1 = sample1 - center;
            let distance0 = max(length(delta0), 0.0001);
            let distance1 = max(length(delta1), 0.0001);
            let weight0 = clamp(distance0 * falloff_mul + falloff_add, 0.0, 1.0);
            let weight1 = clamp(distance1 * falloff_mul + falloff_add, 0.0, 1.0);
            let sample_horizon0 = dot(delta0 / distance0, view_vector);
            let sample_horizon1 = dot(delta1 / distance1, view_vector);
            horizon0 = max(horizon0, mix(low_horizon0, sample_horizon0, weight0));
            horizon1 = max(horizon1, mix(low_horizon1, sample_horizon1, weight1));
        }

        projected_length = mix(projected_length, 1.0, 0.05);
        let horizon_angle0 = -acos(clamp(horizon1, -1.0, 1.0));
        let horizon_angle1 = acos(clamp(horizon0, -1.0, 1.0));
        let integral0 = (
            cos_normal
            + 2.0 * horizon_angle0 * sin(normal_angle)
            - cos(2.0 * horizon_angle0 - normal_angle)
        ) * 0.25;
        let integral1 = (
            cos_normal
            + 2.0 * horizon_angle1 * sin(normal_angle)
            - cos(2.0 * horizon_angle1 - normal_angle)
        ) * 0.25;
        visibility += projected_length * (integral0 + integral1);
    }

    visibility = max(0.03, pow(visibility / 3.0, FINAL_VALUE_POWER));
    return ao_output(visibility, edges);
}

fn spatial_noise(pixel: vec2<u32>) -> vec2<f32> {
    let index = hilbert_index(pixel.x & 63u, pixel.y & 63u);
    return fract(vec2(0.5) + f32(index) * vec2(0.7548776662466927, 0.5698402909980532));
}

fn hilbert_index(initial_x: u32, initial_y: u32) -> u32 {
    var x = initial_x;
    var y = initial_y;
    var index = 0u;
    var level = 32u;
    loop {
        if level == 0u {
            break;
        }
        let region_x = u32((x & level) != 0u);
        let region_y = u32((y & level) != 0u);
        index += level * level * ((3u * region_x) ^ region_y);
        if region_y == 0u {
            if region_x == 1u {
                x = 63u - x;
                y = 63u - y;
            }
            let temporary = x;
            x = y;
            y = temporary;
        }
        level /= 2u;
    }
    return index;
}
