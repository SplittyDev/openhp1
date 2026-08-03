use std::{io::Cursor, sync::Arc};

use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
    listener::ListenerHandle,
    sound::{
        PlaybackState,
        static_sound::{StaticSoundData, StaticSoundHandle},
    },
    track::{SpatialTrackBuilder, SpatialTrackHandle},
};

use crate::{AudioClip, Error, Result};

struct ActiveSound {
    actor: usize,
    clip: AudioClip,
    slot: u8,
    volume: f32,
    radius: f32,
    track: SpatialTrackHandle,
    sound: StaticSoundHandle,
}

pub struct AudioPlayer {
    manager: AudioManager,
    listener: ListenerHandle,
    listener_position: [f32; 3],
    music_volume: f32,
    sound_volume: f32,
    sounds: Vec<ActiveSound>,
    music: Option<StaticSoundHandle>,
}

impl AudioPlayer {
    pub fn new(music_volume: f32, sound_volume: f32) -> Result<Self> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| Error::Playback(error.to_string()))?;
        let listener = manager
            .add_listener([0.0; 3], mint::Quaternion::from([0.0, 0.0, 0.0, 1.0]))
            .map_err(|error| Error::Playback(error.to_string()))?;
        Ok(Self {
            manager,
            listener,
            listener_position: [0.0; 3],
            music_volume,
            sound_volume,
            sounds: Vec::new(),
            music: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play_sound(
        &mut self,
        actor: usize,
        clip: &AudioClip,
        position: [f32; 3],
        slot: u8,
        volume: f32,
        no_override: bool,
        radius: f32,
        pitch: f32,
    ) -> Result<()> {
        self.sounds
            .retain(|sound| sound.sound.state() != PlaybackState::Stopped);
        if slot != 0
            && let Some(index) = self
                .sounds
                .iter()
                .position(|sound| sound.actor == actor && sound.slot == slot)
        {
            if no_override {
                return Ok(());
            }
            let mut sound = self.sounds.swap_remove(index);
            sound.sound.stop(Tween::default());
        }

        let radius = if radius > 0.0 { radius } else { 1500.0 };
        let mut track = self
            .manager
            .add_spatial_sub_track(
                &self.listener,
                position,
                SpatialTrackBuilder::new()
                    .attenuation_function(None)
                    .spatialization_strength(1.0),
            )
            .map_err(|error| Error::Playback(error.to_string()))?;
        let sound = track
            .play(decoder(clip)?.playback_rate(f64::from(pitch)))
            .map_err(|error| Error::Playback(error.to_string()))?;
        track.set_volume(
            linear_volume(
                attenuated_volume(self.listener_position, position, radius)
                    * volume
                    * self.sound_volume,
            ),
            Tween::default(),
        );
        self.sounds.push(ActiveSound {
            actor,
            clip: clip.clone(),
            slot,
            volume,
            radius,
            track,
            sound,
        });
        Ok(())
    }

    pub fn stop_sound(&mut self, actor: usize, clip: Option<&AudioClip>, slot: Option<u8>) {
        for sound in self.sounds.iter_mut().filter(|sound| {
            sound.actor == actor
                && clip.is_none_or(|clip| sound.clip == *clip)
                && slot.is_none_or(|slot| sound.slot == slot)
        }) {
            sound.sound.stop(Tween::default());
        }
        self.sounds
            .retain(|sound| sound.sound.state() != PlaybackState::Stopped);
    }

    pub fn modify_sound(&mut self, actor: usize, slot: u8, parameter: u8, value: f32) -> bool {
        if slot == 0 {
            return false;
        }
        self.sounds
            .retain(|sound| sound.sound.state() != PlaybackState::Stopped);
        let Some(sound) = self
            .sounds
            .iter_mut()
            .find(|sound| sound.actor == actor && sound.slot == slot)
        else {
            return false;
        };
        match parameter {
            0 => sound.volume = value,
            1 => sound.radius = value,
            2 => {
                sound
                    .sound
                    .set_playback_rate(f64::from(value), Tween::default());
            }
            _ => {}
        }
        true
    }

    pub fn update(
        &mut self,
        listener_position: [f32; 3],
        listener_orientation: [f32; 4],
        actor_positions: &[[f32; 3]],
    ) {
        self.listener_position = listener_position;
        self.listener
            .set_position(listener_position, Tween::default());
        self.listener.set_orientation(
            mint::Quaternion::from(listener_orientation),
            Tween::default(),
        );
        self.sounds
            .retain(|sound| sound.sound.state() != PlaybackState::Stopped);
        for sound in &mut self.sounds {
            if let Some(position) = actor_positions.get(sound.actor).copied() {
                sound.track.set_position(position, Tween::default());
                sound.track.set_volume(
                    linear_volume(
                        attenuated_volume(listener_position, position, sound.radius)
                            * sound.volume
                            * self.sound_volume,
                    ),
                    Tween::default(),
                );
            }
        }
    }

    pub fn play_music(&mut self, clip: &AudioClip, volume: f32) -> Result<()> {
        self.stop_music();
        self.music = Some(
            self.manager
                .play(
                    decoder(clip)?
                        .volume(linear_volume(volume * self.music_volume))
                        .loop_region(..),
                )
                .map_err(|error| Error::Playback(error.to_string()))?,
        );
        Ok(())
    }

    pub fn stop_music(&mut self) {
        if let Some(mut music) = self.music.take() {
            music.stop(Tween::default());
        }
    }

    pub fn set_music_volume(&mut self, volume: f32) {
        self.music_volume = volume.clamp(0.0, 1.0);
        if let Some(music) = &mut self.music {
            music.set_volume(linear_volume(self.music_volume), Tween::default());
        }
    }

    pub fn set_sound_volume(&mut self, volume: f32) {
        self.sound_volume = volume.clamp(0.0, 1.0);
    }
}

fn decoder(clip: &AudioClip) -> Result<StaticSoundData> {
    StaticSoundData::from_cursor(Cursor::new(Arc::clone(&clip.data)))
        .map_err(|error| Error::Playback(error.to_string()))
}

fn linear_volume(volume: f32) -> Decibels {
    if volume <= 0.001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * volume.log10())
    }
}

fn attenuated_volume(listener: [f32; 3], source: [f32; 3], radius: f32) -> f32 {
    let distance = listener
        .into_iter()
        .zip(source)
        .map(|(listener, source)| (listener - source).powi(2))
        .sum::<f32>()
        .sqrt();
    (1.0 - distance / radius).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_wav_for_playback() {
        let clip = AudioClip {
            format: Arc::from("wav"),
            data: Arc::from(
                &b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x40\x1f\0\0\x01\0\x08\0data\x04\0\0\0\x80\x80\x80\x80"[..],
            ),
        };

        assert_eq!(decoder(&clip).unwrap().num_frames(), 4);
    }

    #[test]
    fn converts_linear_volume_to_decibels() {
        assert_eq!(linear_volume(0.0), Decibels::SILENCE);
        assert_eq!(linear_volume(1.0), Decibels::IDENTITY);
        assert!((linear_volume(0.5).0 - -6.0206).abs() < 0.0001);
    }

    #[test]
    fn attenuation_matches_unreal_radius() {
        assert_eq!(attenuated_volume([0.0; 3], [0.0, 0.0, 50.0], 100.0), 0.5);
        assert_eq!(attenuated_volume([0.0; 3], [0.0, 0.0, 100.0], 100.0), 0.0);
    }
}
