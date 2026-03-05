use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

/// Audio engine that renders MIDI through a SoundFont using RustySynth + cpal.
/// All synth mutations go through a shared Mutex so the cpal callback thread
/// can call render() while the main thread sends note_on/note_off.
pub struct AudioEngine {
    synth: Arc<Mutex<Synthesizer>>,
    _stream: Stream, // held to keep the audio stream alive
}

impl AudioEngine {
    /// Load an SF2 file and open the default audio output device.
    pub fn new(sf2_path: &Path) -> Result<Self> {
        let mut file = File::open(sf2_path)
            .with_context(|| format!("Failed to open SF2 file: {}", sf2_path.display()))?;
        let sound_font = Arc::new(
            SoundFont::new(&mut file)
                .map_err(|e| anyhow::anyhow!("Failed to load SoundFont: {:?}", e))?,
        );

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No audio output device available")?;

        let config = device
            .default_output_config()
            .context("Failed to get default audio output config")?;

        let sample_rate = config.sample_rate().0 as i32;
        let channels = config.channels() as usize;

        let settings = SynthesizerSettings::new(sample_rate);
        let synth = Synthesizer::new(&sound_font, &settings)
            .map_err(|e| anyhow::anyhow!("Failed to create synthesizer: {:?}", e))?;
        let synth = Arc::new(Mutex::new(synth));

        let synth_for_callback = Arc::clone(&synth);
        let stream_config: cpal::StreamConfig = config.into();

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;
                    let mut left = vec![0f32; frames];
                    let mut right = vec![0f32; frames];

                    if let Ok(mut synth) = synth_for_callback.try_lock() {
                        synth.render(&mut left, &mut right);
                    }

                    // Interleave into output buffer
                    for i in 0..frames {
                        let base = i * channels;
                        data[base] = left[i];
                        if channels > 1 {
                            data[base + 1] = right[i];
                        }
                        // Zero any extra channels
                        for ch in 2..channels {
                            data[base + ch] = 0.0;
                        }
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
            .context("Failed to build audio output stream")?;

        stream.play().context("Failed to start audio stream")?;

        Ok(Self {
            synth,
            _stream: stream,
        })
    }

    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) {
        if let Ok(mut synth) = self.synth.lock() {
            synth.note_on(channel as i32, note as i32, velocity as i32);
        }
    }

    pub fn note_off(&self, channel: u8, note: u8) {
        if let Ok(mut synth) = self.synth.lock() {
            synth.note_off(channel as i32, note as i32);
        }
    }

    pub fn note_off_all_channel(&self, channel: u8) {
        if let Ok(mut synth) = self.synth.lock() {
            synth.note_off_all_channel(channel as i32, false);
        }
    }

    pub fn note_off_all(&self) {
        if let Ok(mut synth) = self.synth.lock() {
            synth.note_off_all(false);
        }
    }

    pub fn send_cc(&self, channel: u8, controller: u8, value: u8) {
        if let Ok(mut synth) = self.synth.lock() {
            // MIDI CC command = 0xB0
            synth.process_midi_message(channel as i32, 0xB0, controller as i32, value as i32);
        }
    }

    pub fn program_change(&self, channel: u8, program: u8) {
        if let Ok(mut synth) = self.synth.lock() {
            // MIDI program change command = 0xC0
            synth.process_midi_message(channel as i32, 0xC0, program as i32, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_requires_valid_sf2() {
        let result = AudioEngine::new(Path::new("/nonexistent/file.sf2"));
        assert!(result.is_err());
    }
}
