use std::sync::Arc;

use glam::Vec3;
use openhp1_audio::AudioClip;
use openhp1_physics::BspCollision;

use crate::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerMusic {
    pub clip: Option<AudioClip>,
    pub section: u8,
    pub transition: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleFloat {
    pub base: f32,
    pub random: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParticleColor {
    pub base: [u8; 4],
    pub random: [u8; 4],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticleTexture {
    pub package: String,
    pub export_index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleWind {
    pub location: [f32; 3],
    pub direction: [f32; 3],
    pub fluctuation: [f32; 3],
    pub speed: f32,
    pub radius: u8,
    pub inner_radius: u8,
    pub source: u8,
    pub permeating: bool,
}

impl ParticleWind {
    pub fn total_at(winds: &[Self], collision: Option<&BspCollision>, location: Vec3) -> Vec3 {
        winds.iter().map(|wind| wind.at(collision, location)).sum()
    }

    fn at(&self, collision: Option<&BspCollision>, location: Vec3) -> Vec3 {
        if self.speed == 0.0 || self.source > 1 {
            return Vec3::ZERO;
        }
        let radius = (self.radius as f32).powi(2);
        if radius == 0.0 {
            return Vec3::ZERO;
        }
        let offset = location - Vec3::from_array(self.location);
        let distance_squared = offset.length_squared();
        // Native GetWind guards with Square(Radius()), where Radius() is WindRadius squared.
        if distance_squared > radius * radius
            || (!self.permeating
                && collision.is_some_and(|collision| {
                    collision
                        .line_trace(Vec3::from_array(self.location), location)
                        .is_some()
                }))
        {
            return Vec3::ZERO;
        }
        let direction = if self.source == 0 {
            let direction = offset.normalize_or_zero();
            if direction == Vec3::ZERO {
                Vec3::from_array(self.direction)
            } else {
                direction
            }
        } else {
            Vec3::from_array(self.direction)
        } + Vec3::from_array(self.fluctuation);
        let attenuation = ((1.0 - distance_squared / radius)
            / (1.0 - self.inner_radius as f32 / 256.0))
            .clamp(0.0, 1.0);
        direction * (self.speed * attenuation)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeObject {
    pub package: Arc<str>,
    pub export_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleEmitter {
    pub actor: usize,
    pub owner: Option<usize>,
    pub emit: bool,
    pub prime: bool,
    pub distribution: u8,
    pub style: u8,
    pub unlit: bool,
    pub particles_alive: usize,
    pub particles_max: usize,
    pub particles_emitted: usize,
    pub particles_per_second: ParticleFloat,
    pub parent_particles_per_second: Option<ParticleFloat>,
    pub period: ParticleFloat,
    pub lifetime: ParticleFloat,
    pub speed: ParticleFloat,
    pub angular_spread_width: ParticleFloat,
    pub angular_spread_height: ParticleFloat,
    pub source_width: ParticleFloat,
    pub source_height: ParticleFloat,
    pub source_depth: ParticleFloat,
    pub size_width: ParticleFloat,
    pub size_length: ParticleFloat,
    pub size_end_scale: ParticleFloat,
    pub color_start: ParticleColor,
    pub color_end: ParticleColor,
    pub alpha_start: ParticleFloat,
    pub alpha_end: ParticleFloat,
    pub alpha_delay: f32,
    pub alpha_grow_period: f32,
    pub color_delay: f32,
    pub size_delay: f32,
    pub size_grow_period: f32,
    pub draw_scale: f32,
    pub system_relative: bool,
    pub wind_per_particle: bool,
    pub damping: f32,
    pub gravity: [f32; 3],
    pub wind: [f32; 3],
    pub winds: Vec<ParticleWind>,
    pub render_primitive: u8,
    pub velocity_relative: bool,
    pub owner_velocity: [f32; 3],
    pub gravity_modifier: f32,
    pub chaos: f32,
    pub chaos_delay: f32,
    pub attraction: [f32; 3],
    pub elasticity: f32,
    pub wind_modifier: f32,
    pub spin_rate: ParticleFloat,
    pub drip_time: ParticleFloat,
    pub parent_blend: f32,
    pub color_palette: bool,
    pub pattern: Vec<[f32; 3]>,
    pub textures: Vec<ParticleTexture>,
}

impl ParticleEmitter {
    pub(crate) fn blend_parent_parameters(&mut self, parent: &Self) {
        let blend = self.parent_blend;
        if blend == 0.0 {
            return;
        }
        self.parent_particles_per_second = Some(parent.particles_per_second);
        if blend < 0.0 {
            return;
        }
        if blend >= 1.0 {
            self.source_width = parent.source_width;
            self.source_height = parent.source_height;
            self.source_depth = parent.source_depth;
            self.angular_spread_width = parent.angular_spread_width;
            self.angular_spread_height = parent.angular_spread_height;
            self.speed = parent.speed;
            self.lifetime = parent.lifetime;
            self.size_width = parent.size_width;
            self.size_length = parent.size_length;
            self.size_end_scale = parent.size_end_scale;
            self.color_start = parent.color_start;
            self.color_end = parent.color_end;
            self.alpha_start = parent.alpha_start;
            self.alpha_end = parent.alpha_end;
            self.spin_rate = parent.spin_rate;
            self.drip_time = parent.drip_time;
            return;
        }
        self.source_width = lerp_particle_float(self.source_width, parent.source_width, blend);
        self.source_height = lerp_particle_float(self.source_height, parent.source_height, blend);
        self.source_depth = lerp_particle_float(self.source_depth, parent.source_depth, blend);
        self.angular_spread_width = lerp_particle_float(
            self.angular_spread_width,
            parent.angular_spread_width,
            blend,
        );
        self.angular_spread_height = lerp_particle_float(
            self.angular_spread_height,
            parent.angular_spread_height,
            blend,
        );
        self.speed = lerp_particle_float(self.speed, parent.speed, blend);
        self.lifetime = lerp_particle_float(self.lifetime, parent.lifetime, blend);
        self.size_width = lerp_particle_float(self.size_width, parent.size_width, blend);
        self.size_length = lerp_particle_float(self.size_length, parent.size_length, blend);
        self.size_end_scale =
            lerp_particle_float(self.size_end_scale, parent.size_end_scale, blend);
        self.color_start = lerp_particle_color(self.color_start, parent.color_start, blend);
        self.color_end = lerp_particle_color(self.color_end, parent.color_end, blend);
        self.alpha_start = lerp_particle_float(self.alpha_start, parent.alpha_start, blend);
        self.alpha_end = lerp_particle_float(self.alpha_end, parent.alpha_end, blend);
        self.spin_rate = lerp_particle_float(self.spin_rate, parent.spin_rate, blend);
        self.drip_time = lerp_particle_float(self.drip_time, parent.drip_time, blend);
    }

    pub fn capability_diagnostics(&self) -> Vec<&'static str> {
        let mut diagnostics = Vec::new();
        if !matches!(self.render_primitive, 1 | 2) {
            diagnostics.push("particle render primitive is unsupported");
        }
        if self.textures.len() > 1 {
            diagnostics.push("particle random texture selection is unsupported");
        }
        if self.color_palette {
            diagnostics.push("particle palette color cycling is unsupported");
        }
        diagnostics
    }
}

fn lerp_particle_float(child: ParticleFloat, parent: ParticleFloat, blend: f32) -> ParticleFloat {
    ParticleFloat {
        base: child.base + (parent.base - child.base) * blend,
        random: child.random + (parent.random - child.random) * blend,
    }
}

fn lerp_particle_color(child: ParticleColor, parent: ParticleColor, blend: f32) -> ParticleColor {
    let component = |child: u8, parent: u8| {
        (f32::from(child) + (f32::from(parent) - f32::from(child)) * blend)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let color = |child: [u8; 4], parent: [u8; 4]| {
        std::array::from_fn(|index| component(child[index], parent[index]))
    };
    ParticleColor {
        base: color(child.base, parent.base),
        random: color(child.random, parent.random),
    }
}

#[cfg(test)]
mod particle_tests {
    use super::*;

    #[test]
    fn reports_only_authored_particle_features_outside_the_supported_subset() {
        let supported = ParticleEmitter {
            render_primitive: 1,
            velocity_relative: true,
            gravity_modifier: -0.5,
            elasticity: 1.0,
            wind_modifier: 1.0,
            ..Default::default()
        };
        assert!(supported.capability_diagnostics().is_empty());

        let unsupported = ParticleEmitter {
            render_primitive: 0,
            chaos: 1.0,
            chaos_delay: 0.5,
            ..Default::default()
        };
        assert_eq!(
            unsupported.capability_diagnostics(),
            ["particle render primitive is unsupported"]
        );
    }

    #[test]
    fn blends_only_the_original_fractional_parent_parameter_set() {
        let mut child = ParticleEmitter {
            parent_blend: 0.25,
            particles_per_second: ParticleFloat {
                base: 4.0,
                random: 8.0,
            },
            source_width: ParticleFloat {
                base: 10.0,
                random: 20.0,
            },
            speed: ParticleFloat {
                base: 100.0,
                random: 200.0,
            },
            size_width: ParticleFloat {
                base: 2.0,
                random: 3.0,
            },
            color_start: ParticleColor {
                base: [20, 40, 60, 80],
                random: [4, 8, 12, 16],
            },
            alpha_start: ParticleFloat {
                base: 0.25,
                random: 0.5,
            },
            alpha_end: ParticleFloat {
                base: 0.5,
                random: 0.75,
            },
            ..Default::default()
        };
        let parent = ParticleEmitter {
            particles_per_second: ParticleFloat {
                base: 12.0,
                random: 16.0,
            },
            source_width: ParticleFloat {
                base: 30.0,
                random: 60.0,
            },
            speed: ParticleFloat {
                base: 300.0,
                random: 600.0,
            },
            size_width: ParticleFloat {
                base: 8.0,
                random: 9.0,
            },
            color_start: ParticleColor {
                base: [100, 120, 140, 160],
                random: [20, 24, 28, 32],
            },
            alpha_start: ParticleFloat {
                base: 0.75,
                random: 1.0,
            },
            alpha_end: ParticleFloat {
                base: 1.0,
                random: 1.25,
            },
            ..Default::default()
        };

        child.blend_parent_parameters(&parent);

        assert_eq!(
            child.parent_particles_per_second,
            Some(parent.particles_per_second)
        );
        assert_eq!(
            child.source_width,
            ParticleFloat {
                base: 15.0,
                random: 30.0
            }
        );
        assert_eq!(
            child.speed,
            ParticleFloat {
                base: 150.0,
                random: 300.0
            }
        );
        assert_eq!(child.color_start.base, [40, 60, 80, 100]);
        assert_eq!(
            child.alpha_start,
            ParticleFloat {
                base: 0.375,
                random: 0.625
            }
        );
        assert_eq!(
            child.alpha_end,
            ParticleFloat {
                base: 0.625,
                random: 0.875
            }
        );
        assert_eq!(child.size_width.base, 3.5);
    }

    #[test]
    fn fractional_parent_color_blend_preserves_identical_colors() {
        let color = ParticleColor {
            base: [1, 3, 5, 7],
            random: [9, 11, 13, 15],
        };
        let mut child = ParticleEmitter {
            parent_blend: 0.5,
            color_start: color,
            ..Default::default()
        };
        let parent = ParticleEmitter {
            color_start: color,
            ..Default::default()
        };

        child.blend_parent_parameters(&parent);

        assert_eq!(child.color_start, color);
    }

    #[test]
    fn negative_parent_blend_keeps_child_parameters_and_retains_parent_rate() {
        let source_width = ParticleFloat {
            base: 10.0,
            random: 2.0,
        };
        let parent_rate = ParticleFloat {
            base: 30.0,
            random: 4.0,
        };
        let mut child = ParticleEmitter {
            parent_blend: -0.5,
            particles_per_second: ParticleFloat {
                base: 20.0,
                random: 3.0,
            },
            source_width,
            ..Default::default()
        };
        let parent = ParticleEmitter {
            particles_per_second: parent_rate,
            source_width: ParticleFloat {
                base: 50.0,
                random: 6.0,
            },
            ..Default::default()
        };

        child.blend_parent_parameters(&parent);

        assert_eq!(child.parent_particles_per_second, Some(parent_rate));
        assert_eq!(child.source_width, source_width);
    }

    #[test]
    fn full_parent_blend_uses_all_parent_spawn_parameters() {
        let child_rate = ParticleFloat {
            base: 2.0,
            random: 3.0,
        };
        let mut child = ParticleEmitter {
            parent_blend: 1.5,
            particles_per_second: child_rate,
            size_width: ParticleFloat {
                base: 2.0,
                random: 3.0,
            },
            alpha_start: ParticleFloat {
                base: 0.25,
                random: 0.5,
            },
            ..Default::default()
        };
        let parent = ParticleEmitter {
            particles_per_second: ParticleFloat {
                base: 6.0,
                random: 7.0,
            },
            size_width: ParticleFloat {
                base: 8.0,
                random: 9.0,
            },
            alpha_start: ParticleFloat {
                base: 0.75,
                random: 1.0,
            },
            ..Default::default()
        };

        child.blend_parent_parameters(&parent);

        assert_eq!(child.particles_per_second, child_rate);
        assert_eq!(
            child.parent_particles_per_second,
            Some(parent.particles_per_second)
        );
        assert_eq!(child.size_width, parent.size_width);
        assert_eq!(child.alpha_start, parent.alpha_start);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponAttachment {
    pub pawn: usize,
    pub weapon: usize,
    pub mesh: RuntimeObject,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActorAction {
    PlayAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
        root_motion: bool,
    },
    LoopAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
        root_motion: bool,
    },
    RestoreAnimation {
        actor: usize,
        sequence: String,
        rate: f32,
        tween_time: f32,
        looping: bool,
        tween_only: bool,
        root_motion: bool,
        phase: f32,
    },
    SetAnimationFrame {
        actor: usize,
        frame: f32,
    },
    AwaitAnimation {
        actor: usize,
    },
    PlaySound {
        actor: usize,
        clip: AudioClip,
        location: [f32; 3],
        slot: u8,
        volume: f32,
        no_override: bool,
        radius: f32,
        pitch: f32,
    },
    ModifySound {
        actor: usize,
        slot: u8,
        parameter: u8,
        value: f32,
    },
    StopSound {
        actor: usize,
        clip: Option<AudioClip>,
        slot: Option<u8>,
    },
    ClientTravel {
        actor: usize,
        url: String,
        travel_type: u8,
        transfer_items: bool,
    },
    StoryBookInterlude {
        actor: usize,
        story: i32,
        event_when_done: String,
    },
    UnlockQuidditch {
        actor: usize,
        level: u8,
    },
    FinishQuidditchMatch {
        actor: usize,
        team0_score: i32,
        opponent_score: i32,
    },
    UpdateUrl {
        actor: usize,
        option: String,
        value: String,
        save_default: bool,
    },
    SpawnActor {
        actor: usize,
        name: String,
        class_package: Arc<str>,
        class_export: usize,
        class_name: String,
        location: [f32; 3],
        rotation: [i32; 3],
    },
    SetLocation {
        actor: usize,
        location: [f32; 3],
    },
    SetRotation {
        actor: usize,
        rotation: [i32; 3],
    },
    SetPrePivot {
        actor: usize,
        pre_pivot: [f32; 3],
    },
    SetHidden {
        actor: usize,
        hidden: bool,
    },
    SetDrawType {
        actor: usize,
        draw_type: u8,
    },
    SetMesh {
        actor: usize,
        mesh: Option<RuntimeObject>,
    },
    SetPhysics {
        actor: usize,
        physics: u8,
    },
    SetDrawScale {
        actor: usize,
        draw_scale: f32,
    },
    SetStyle {
        actor: usize,
        style: u8,
    },
    SetScaleGlow {
        actor: usize,
        scale_glow: f32,
    },
    SetSkin {
        actor: usize,
        skin: Option<RuntimeObject>,
    },
    SetSkelAnim {
        actor: usize,
        skel_anim: Option<RuntimeObject>,
    },
    SetAmbientGlow {
        actor: usize,
        ambient_glow: u8,
    },
    SetLightBrightness {
        actor: usize,
        light_brightness: u8,
    },
    SetOpacity {
        actor: usize,
        opacity: f32,
    },
    SetWarpDestination {
        actor: usize,
        destination: Option<RuntimeObject>,
    },
    UnsupportedSceneProperty {
        actor: usize,
        property: String,
    },
    DestroyActor {
        actor: usize,
    },
    Log {
        actor: usize,
        message: String,
        tag: Option<String>,
    },
    DeferredCall {
        actor: usize,
        message: String,
    },
    DispatchEvent {
        actor: usize,
        event: &'static str,
        arguments: Vec<Value>,
    },
}

impl ActorAction {
    pub fn actor(&self) -> usize {
        match self {
            Self::PlayAnimation { actor, .. }
            | Self::LoopAnimation { actor, .. }
            | Self::RestoreAnimation { actor, .. }
            | Self::SetAnimationFrame { actor, .. }
            | Self::AwaitAnimation { actor }
            | Self::PlaySound { actor, .. }
            | Self::ModifySound { actor, .. }
            | Self::StopSound { actor, .. }
            | Self::ClientTravel { actor, .. }
            | Self::StoryBookInterlude { actor, .. }
            | Self::UnlockQuidditch { actor, .. }
            | Self::FinishQuidditchMatch { actor, .. }
            | Self::UpdateUrl { actor, .. }
            | Self::SpawnActor { actor, .. }
            | Self::SetLocation { actor, .. }
            | Self::SetRotation { actor, .. }
            | Self::SetPrePivot { actor, .. }
            | Self::SetHidden { actor, .. }
            | Self::SetDrawType { actor, .. }
            | Self::SetMesh { actor, .. }
            | Self::SetPhysics { actor, .. }
            | Self::SetDrawScale { actor, .. }
            | Self::SetStyle { actor, .. }
            | Self::SetScaleGlow { actor, .. }
            | Self::SetSkin { actor, .. }
            | Self::SetSkelAnim { actor, .. }
            | Self::SetAmbientGlow { actor, .. }
            | Self::SetLightBrightness { actor, .. }
            | Self::SetOpacity { actor, .. }
            | Self::SetWarpDestination { actor, .. }
            | Self::UnsupportedSceneProperty { actor, .. }
            | Self::DestroyActor { actor }
            | Self::Log { actor, .. }
            | Self::DeferredCall { actor, .. }
            | Self::DispatchEvent { actor, .. } => *actor,
        }
    }
}
