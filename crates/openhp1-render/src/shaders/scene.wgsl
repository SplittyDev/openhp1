struct Camera {
    view_projection: mat4x4<f32>,
    world_to_view: mat4x4<f32>,
    camera_position: vec4<f32>,
    auto_uv: vec4<f32>,
    clip_plane: vec4<f32>,
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

struct RealtimeLightmap {
    ambient: vec4<f32>,
    light_range: vec4<u32>,
};

struct RealtimeLight {
    position_radius: vec4<f32>,
    direction_outer: vec4<f32>,
    color: vec4<f32>,
    visibility: vec4<f32>,
    effect: vec4<u32>,
};

@group(2) @binding(0)
var<storage, read> realtime_lightmaps: array<RealtimeLightmap>;

@group(2) @binding(1)
var<storage, read> realtime_lights: array<RealtimeLight>;

@group(2) @binding(2)
var visibility_texture: texture_2d<f32>;

@group(2) @binding(3)
var visibility_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
    @location(1) lightmap_coordinates: vec2<f32>,
    @location(2) has_lightmap: f32,
    @location(3) vertex_color: vec4<f32>,
    @location(4) world_position: vec3<f32>,
    @location(5) world_normal: vec3<f32>,
    @location(6) @interpolate(flat) lighting_index: u32,
    @location(7) lighting_coordinates: vec2<f32>,
    @location(8) environment_color: vec4<f32>,
};

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) texture_coordinates: vec2<f32>,
    @location(2) texture_pan_speeds: vec4<f32>,
    @location(3) lightmap_coordinates: vec2<f32>,
    @location(4) has_lightmap: f32,
    @location(5) vertex_color: vec4<f32>,
    @location(6) normal: vec3<f32>,
    @location(7) environment_map: f32,
    @location(8) lighting_coordinates: vec2<f32>,
    @location(9) lighting_index: u32,
    @location(10) uv_effect_scale: vec2<f32>,
    @location(11) node_plane_normal: vec3<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4(position, 1.0);
    output.environment_color = vec4(-1.0);
    if environment_map > 0.5 {
        let incident_offset = position - camera.camera_position.xyz;
        let incident = incident_offset * inverseSqrt(max(dot(incident_offset, incident_offset), 0.00000001));
        let world_normal = normal * inverseSqrt(max(dot(normal, normal), 0.00000001));
        let reflection = reflect(incident, world_normal);
        let world_to_view = mat3x3<f32>(
            camera.world_to_view[0].xyz,
            camera.world_to_view[1].xyz,
            camera.world_to_view[2].xyz,
        );
        let view_reflection = world_to_view * reflection;
        output.texture_coordinates = (view_reflection.xy + vec2(1.0))
            * 0.5 * uv_effect_scale.x;
        let environment_light = pow(max(view_reflection.z, 0.0), 0.25);
        output.environment_color = vec4(vec3(environment_light), 0.0);
    } else {
        let texture_pan_speed = select(
            texture_pan_speeds.xy,
            texture_pan_speeds.zw,
            dot(camera.camera_position.xyz - position, node_plane_normal) > 0.0,
        );
        output.texture_coordinates = texture_coordinates + texture_pan_speed * camera.auto_uv.x;
        if any(uv_effect_scale != vec2(0.0)) {
            let time = camera.auto_uv.x / 64.0;
            let small_wavy_offset = vec2(
                8.0 * sin(time) + 4.0 * cos(2.3 * time),
                8.0 * cos(time) + 4.0 * sin(2.3 * time),
            );
            output.texture_coordinates = output.texture_coordinates
                + uv_effect_scale * small_wavy_offset;
        }
    }
    output.lightmap_coordinates = lightmap_coordinates;
    output.has_lightmap = has_lightmap;
    output.vertex_color = vertex_color;
    output.world_position = position;
    output.world_normal = normal;
    output.lighting_index = lighting_index;
    output.lighting_coordinates = lighting_coordinates;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_lightmap(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_lightmap(input, texture_color);
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_unlit(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_vertex_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_unlit_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_vertex_light(input, texture_color);
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_lightmap(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return apply_opacity(input, color);
}

@fragment
fn fragment_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_opacity(input, apply_lightmap(input, texture_color));
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return color;
}

@fragment
fn fragment_unlit_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_vertex_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return apply_opacity(input, color);
}

@fragment
fn fragment_unlit_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_opacity(input, apply_vertex_light(input, texture_color));
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return color;
}

@fragment
fn fragment_backdrop(input: VertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(color_texture));
    let color = textureSample(color_texture, color_sampler, input.clip_position.xy / dimensions);
    // The modern composite uses alpha to keep sky pixels out of ambient occlusion.
    return vec4(color.rgb, 0.0);
}

@fragment
fn fragment_mirror(input: VertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(color_texture));
    return textureSample(color_texture, color_sampler, input.clip_position.xy / dimensions);
}

@fragment
fn fragment_modern(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_realtime_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_modern_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_realtime_light(input, texture_color);
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_modern_unlit(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_modern_vertex_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_modern_unlit_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = apply_modern_vertex_light(input, texture_color);
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return preserve_environment_coverage(input, color);
}

@fragment
fn fragment_modern_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_realtime_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    // UE1 clamps the lit source before blending; the modern target does not.
    return clamp(apply_opacity(input, color), vec4(0.0), vec4(1.0));
}

@fragment
fn fragment_modern_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = clamp(
        apply_opacity(input, apply_realtime_light(input, texture_color)),
        vec4(0.0),
        vec4(1.0),
    );
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return color;
}

@fragment
fn fragment_modern_unlit_blended(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = apply_modern_vertex_light(input, textureSample(color_texture, color_sampler, input.texture_coordinates));
    return clamp(apply_opacity(input, color), vec4(0.0), vec4(1.0));
}

@fragment
fn fragment_modern_unlit_blended_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_color = textureSample(color_texture, color_sampler, input.texture_coordinates);
    let color = clamp(
        apply_opacity(input, apply_modern_vertex_light(input, texture_color)),
        vec4(0.0),
        vec4(1.0),
    );
    if masked_alpha(input, texture_color.a, color.a) < 0.5 {
        discard;
    }
    return color;
}

@fragment
fn fragment_backdrop_modern(input: VertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(color_texture));
    let color = textureSample(color_texture, color_sampler, input.clip_position.xy / dimensions);
    return vec4(color.rgb, 0.0);
}

fn apply_lightmap(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    clip_to_portal(input);
    let light = textureSample(
        lightmap_texture,
        lightmap_sampler,
        input.lightmap_coordinates,
    ).rgb * 2.0;
    let vertex_light = select(
        input.vertex_color.rgb,
        input.environment_color.rgb,
        input.environment_color.r >= 0.0,
    );
    let alpha = select(
        color.a,
        color.a * input.environment_color.a,
        input.environment_color.r >= 0.0,
    );
    return vec4(color.rgb * mix(vertex_light, light, input.has_lightmap), alpha);
}

fn apply_vertex_light(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    clip_to_portal(input);
    let alpha = select(
        color.a,
        color.a * input.environment_color.a,
        input.environment_color.r >= 0.0,
    );
    return vec4(color.rgb * input.vertex_color.rgb, alpha);
}

fn apply_modern_vertex_light(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    clip_to_portal(input);
    let alpha = select(
        color.a,
        color.a * input.environment_color.a,
        input.environment_color.r >= 0.0,
    );
    return vec4(srgb_to_linear(color.rgb * input.vertex_color.rgb), alpha);
}

fn apply_realtime_light(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    clip_to_portal(input);
    if input.environment_color.r >= 0.0 {
        return vec4(
            srgb_to_linear(color.rgb * input.environment_color.rgb),
            color.a * input.environment_color.a,
        );
    }
    if input.lighting_index == 0xffffffffu {
        return apply_modern_vertex_light(input, color);
    }
    let lightmap = realtime_lightmaps[input.lighting_index];
    var illumination = lightmap.ambient.rgb;
    let normal = normalize(input.world_normal);
    let end = lightmap.light_range.x + lightmap.light_range.y;
    for (var index = lightmap.light_range.x; index < end; index++) {
        let light = realtime_lights[index];
        let offset = light.position_radius.xyz - input.world_position;
        let distance_squared = dot(offset, offset);
        let radius_squared = light.position_radius.w * light.position_radius.w;
        let visibility_uv = input.lighting_coordinates * light.visibility.xy + light.visibility.zw;
        var strength = 0.0;
        switch light.effect.x {
            case 13u: {
                if distance_squared < radius_squared {
                    let visibility = textureSample(visibility_texture, visibility_sampler, visibility_uv).r * 2.0;
                    strength = visibility * (1.0 - sqrt(distance_squared) / light.position_radius.w);
                }
            }
            case 14u: {
                let distance = sqrt(distance_squared);
                let normalized = distance / light.position_radius.w;
                if normalized >= 0.8 && normalized < 1.0 {
                    let visibility = textureSample(visibility_texture, visibility_sampler, visibility_uv).r * 2.0;
                    strength = visibility * (1.0 - 10.0 * abs(normalized - 0.9));
                }
            }
            case 17u: {
                let planar = offset.x * offset.x + offset.z * offset.z;
                if planar < radius_squared {
                    let visibility = textureSample(visibility_texture, visibility_sampler, visibility_uv).r * 2.0;
                    strength = visibility * (1.0 - planar / radius_squared);
                }
            }
            case 8u, 12u: {
                if distance_squared < radius_squared && light.direction_outer.w < 1.0 && distance_squared > 0.0 {
                    let distance = sqrt(distance_squared);
                    let normalized_distance = distance_squared / radius_squared;
                    let cosine = dot(offset / distance, light.direction_outer.xyz);
                    let spot = max(1.0 - min((1.0 - cosine) / (1.0 - light.direction_outer.w), 1.0), 0.0);
                    let visibility = textureSample(visibility_texture, visibility_sampler, visibility_uv).r * 2.0;
                    strength = visibility
                        * ue1_distance_falloff(normalized_distance)
                        * abs(dot(offset / distance, normal))
                        * spot * spot;
                }
            }
            case 4u: {}
            default: {
                if distance_squared < radius_squared && distance_squared > 0.0 {
                    let distance = sqrt(distance_squared);
                    let visibility = textureSample(visibility_texture, visibility_sampler, visibility_uv).r * 2.0;
                    strength = visibility
                        * ue1_distance_falloff(distance_squared / radius_squared)
                        * abs(dot(offset / distance, normal));
                }
            }
        }
        let contribution = min(light.color.rgb * strength, vec3(1.0));
        if light.effect.y != 0u {
            illumination = max(illumination - contribution, vec3(0.0));
        } else {
            illumination += contribution;
        }
    }
    return vec4(srgb_to_linear(color.rgb * illumination * 2.0), color.a);
}

fn ue1_distance_falloff(distance_squared: f32) -> f32 {
    let value = sqrt(distance_squared + 0.0001);
    return min((1.0 + 2.0 * value * value * value - 3.0 * value * value) / value, 1.0);
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3(0.055)) / 1.055, vec3(2.4));
    return select(high, low, color <= vec3(0.04045));
}

fn apply_opacity(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    // HP1's mesh path multiplies vertex RGBA by Opacity before alpha blending.
    // Environment/Unlit subsequently overwrite RGB and alpha, so opacity no
    // longer scales the source color in that path.
    if input.environment_color.r >= 0.0 {
        return color;
    }
    return color * input.vertex_color.a;
}

fn preserve_environment_coverage(input: VertexOutput, color: vec4<f32>) -> vec4<f32> {
    // Retail consumes the environment vertex alpha as zero, but its backbuffer
    // alpha is otherwise unused. Modern uses target alpha to distinguish
    // geometry from the backdrop, so opaque environment meshes retain coverage.
    return select(color, vec4(color.rgb, 1.0), input.environment_color.r >= 0.0);
}

fn masked_alpha(input: VertexOutput, texture_alpha: f32, final_alpha: f32) -> f32 {
    // The fixed alpha test sees environment diffuse alpha after texture-stage
    // modulation. Ordinary masked materials retain their texture-alpha test.
    return select(texture_alpha, final_alpha, input.environment_color.r >= 0.0);
}

fn clip_to_portal(input: VertexOutput) {
    if dot(vec4(input.world_position, 1.0), camera.clip_plane) < 0.0 {
        discard;
    }
}
