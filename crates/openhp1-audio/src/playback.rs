use std::{io::Cursor, sync::Arc};

use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
    sound::static_sound::{StaticSoundData, StaticSoundHandle},
};

use crate::{AudioClip, Error, Result};

pub struct AudioPlayer {
    manager: AudioManager,
    music: Option<StaticSoundHandle>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| Error::Playback(error.to_string()))?;
        Ok(Self {
            manager,
            music: None,
        })
    }

    pub fn play_sound(&mut self, clip: &AudioClip, volume: f32, pitch: f32) -> Result<()> {
        self.manager
            .play(
                decoder(clip)?
                    .volume(linear_volume(volume))
                    .playback_rate(f64::from(pitch)),
            )
            .map_err(|error| Error::Playback(error.to_string()))?;
        Ok(())
    }

    pub fn play_music(&mut self, clip: &AudioClip, volume: f32) -> Result<()> {
        self.stop_music();
        self.music = Some(
            self.manager
                .play(decoder(clip)?.volume(linear_volume(volume)).loop_region(..))
                .map_err(|error| Error::Playback(error.to_string()))?,
        );
        Ok(())
    }

    pub fn stop_music(&mut self) {
        if let Some(mut music) = self.music.take() {
            music.stop(Tween::default());
        }
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
}
