use std::sync::Arc;

use openhp1_audio::AudioClip;

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
    pub color_delay: f32,
    pub size_delay: f32,
    pub size_grow_period: f32,
    pub draw_scale: f32,
    pub system_relative: bool,
    pub damping: f32,
    pub gravity: [f32; 3],
    pub render_primitive: u8,
    pub velocity_relative: bool,
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
    pub fn capability_diagnostics(&self) -> Vec<&'static str> {
        let mut diagnostics = Vec::new();
        if !matches!(self.render_primitive, 1 | 2) {
            diagnostics.push("particle render primitive is unsupported");
        }
        if self.textures.len() > 1 {
            diagnostics.push("particle random texture selection is unsupported");
        }
        if self.velocity_relative {
            diagnostics.push("particle owner-velocity inheritance is unsupported");
        }
        if self.wind_modifier != 0.0 {
            diagnostics.push("particle wind response is unsupported");
        }
        if self.parent_blend != 0.0 {
            diagnostics.push("particle parent parameter blending is unsupported");
        }
        if self.color_palette {
            diagnostics.push("particle palette color cycling is unsupported");
        }
        diagnostics
    }
}

#[cfg(test)]
mod particle_tests {
    use super::*;

    #[test]
    fn reports_only_authored_particle_features_outside_the_supported_subset() {
        let supported = ParticleEmitter {
            render_primitive: 1,
            gravity_modifier: -0.5,
            elasticity: 1.0,
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
            | Self::AwaitAnimation { actor }
            | Self::PlaySound { actor, .. }
            | Self::StopSound { actor, .. }
            | Self::ClientTravel { actor, .. }
            | Self::SpawnActor { actor, .. }
            | Self::SetLocation { actor, .. }
            | Self::SetRotation { actor, .. }
            | Self::SetPrePivot { actor, .. }
            | Self::SetHidden { actor, .. }
            | Self::SetDrawType { actor, .. }
            | Self::SetMesh { actor, .. }
            | Self::SetDrawScale { actor, .. }
            | Self::SetStyle { actor, .. }
            | Self::SetScaleGlow { actor, .. }
            | Self::SetSkin { actor, .. }
            | Self::SetSkelAnim { actor, .. }
            | Self::SetAmbientGlow { actor, .. }
            | Self::SetLightBrightness { actor, .. }
            | Self::SetOpacity { actor, .. }
            | Self::UnsupportedSceneProperty { actor, .. }
            | Self::DestroyActor { actor }
            | Self::Log { actor, .. }
            | Self::DeferredCall { actor, .. }
            | Self::DispatchEvent { actor, .. } => *actor,
        }
    }
}
