use std::{io::Cursor, sync::Arc, time::Duration};

use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
    sound::{
        PlaybackState,
        static_sound::{StaticSoundData, StaticSoundHandle},
    },
};

use crate::{AudioClip, Error, Result};

struct ActiveSound {
    actor: usize,
    clip: AudioClip,
    slot: u8,
    volume: f32,
    radius: f32,
    sound: StaticSoundHandle,
}

pub struct AudioPlayer {
    manager: AudioManager,
    listener_position: [f32; 3],
    listener_orientation: [f32; 4],
    music_volume: f32,
    sound_volume: f32,
    sound_latency: Duration,
    sounds: Vec<ActiveSound>,
    music: Option<StaticSoundHandle>,
}

impl AudioPlayer {
    pub fn new(music_volume: f32, sound_volume: f32, sound_latency: Duration) -> Result<Self> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| Error::Playback(error.to_string()))?;
        Ok(Self {
            manager,
            listener_position: [0.0; 3],
            listener_orientation: [0.0, 0.0, 0.0, 1.0],
            music_volume,
            sound_volume,
            sound_latency,
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
            sound.sound.stop(immediate_tween());
        }

        let radius = if radius > 0.0 { radius } else { 1500.0 };
        let mut data = decoder(clip)?
            .start_time(self.sound_latency)
            .playback_rate(f64::from(pitch))
            .volume(linear_volume(sound_gain(
                self.listener_position,
                position,
                radius,
                volume,
                self.sound_volume,
            )))
            .panning(source_panning(
                self.listener_position,
                self.listener_orientation,
                position,
            ));
        if clip.looping() {
            data = data.loop_region(..);
        }
        let sound = self
            .manager
            .play(data)
            .map_err(|error| Error::Playback(error.to_string()))?;
        self.sounds.push(ActiveSound {
            actor,
            clip: clip.clone(),
            slot,
            volume,
            radius,
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
            sound.sound.stop(immediate_tween());
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
        self.listener_orientation = listener_orientation;
        self.sounds
            .retain(|sound| sound.sound.state() != PlaybackState::Stopped);
        for sound in &mut self.sounds {
            if let Some(position) = actor_positions.get(sound.actor).copied() {
                sound.sound.set_volume(
                    linear_volume(sound_gain(
                        listener_position,
                        position,
                        sound.radius,
                        sound.volume,
                        self.sound_volume,
                    )),
                    Tween::default(),
                );
                sound.sound.set_panning(
                    source_panning(listener_position, listener_orientation, position),
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

fn immediate_tween() -> Tween {
    Tween {
        duration: Duration::ZERO,
        ..Tween::default()
    }
}

fn linear_volume(volume: f32) -> Decibels {
    if volume <= 0.001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * volume.log10())
    }
}

fn sound_gain(
    listener: [f32; 3],
    source: [f32; 3],
    radius: f32,
    volume: f32,
    master_volume: f32,
) -> f32 {
    (attenuated_volume(listener, source, radius) * volume).clamp(0.0, 1.0) * master_volume
}

fn source_panning(listener: [f32; 3], orientation: [f32; 4], source: [f32; 3]) -> f32 {
    let direction = [
        source[0] - listener[0],
        source[1] - listener[1],
        source[2] - listener[2],
    ];
    let length = direction
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if length <= f32::EPSILON {
        return 0.0;
    }
    let [x, y, z, w] = orientation;
    let right = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
    ];
    direction
        .into_iter()
        .zip(right)
        .map(|(direction, right)| direction / length * right)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
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
            looping: false,
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
    fn galaxy_clamps_each_attenuated_voice_before_master_volume() {
        assert_eq!(sound_gain([0.0; 3], [0.0; 3], 1_500.0, 3.2, 0.5), 0.5);
        assert_eq!(
            sound_gain([0.0; 3], [0.0, 0.0, 20.0], 1_500.0, 1.0, 0.5),
            0.49333334
        );
    }

    #[test]
    fn rear_sources_stay_centered_instead_of_losing_both_channels() {
        let orientation = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(source_panning([0.0; 3], orientation, [0.0, 0.0, 20.0]), 0.0);
        assert_eq!(source_panning([0.0; 3], orientation, [20.0, 0.0, 0.0]), 1.0);
        assert_eq!(
            source_panning([0.0; 3], orientation, [-20.0, 0.0, 0.0]),
            -1.0
        );
    }

    #[test]
    fn attenuation_matches_unreal_radius() {
        assert_eq!(attenuated_volume([0.0; 3], [0.0, 0.0, 50.0], 100.0), 0.5);
        assert_eq!(attenuated_volume([0.0; 3], [0.0, 0.0, 100.0], 100.0), 0.0);
    }
}
