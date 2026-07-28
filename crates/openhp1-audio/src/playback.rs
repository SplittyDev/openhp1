use std::{io::Cursor, sync::Arc};

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::{AudioClip, Error, Result};

pub struct AudioPlayer {
    stream: MixerDeviceSink,
    music: Option<Player>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let stream = DeviceSinkBuilder::open_default_sink()
            .map_err(|error| Error::Playback(error.to_string()))?;
        Ok(Self {
            stream,
            music: None,
        })
    }

    pub fn play_sound(&self, clip: &AudioClip, volume: f32, pitch: f32) -> Result<()> {
        let sink = Player::connect_new(self.stream.mixer());
        sink.set_volume(volume);
        sink.set_speed(pitch);
        sink.append(decoder(clip)?);
        sink.detach();
        Ok(())
    }

    pub fn play_music(&mut self, clip: &AudioClip, volume: f32) -> Result<()> {
        self.stop_music();
        let sink = Player::connect_new(self.stream.mixer());
        sink.set_volume(volume);
        sink.append(decoder(clip)?.repeat_infinite());
        self.music = Some(sink);
        Ok(())
    }

    pub fn stop_music(&mut self) {
        if let Some(sink) = self.music.take() {
            sink.stop();
        }
    }
}

fn decoder(clip: &AudioClip) -> Result<Decoder<Cursor<Arc<[u8]>>>> {
    Decoder::builder()
        .with_data(Cursor::new(Arc::clone(&clip.data)))
        .with_hint(clip.format())
        .build()
        .map_err(|error| Error::Playback(error.to_string()))
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

        assert!(decoder(&clip).unwrap().next().is_some());
    }
}
