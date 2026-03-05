pub mod effects;
pub mod synth;

use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use fundsp::audiounit::AudioUnit;
use fundsp::realseq::SequencerBackend;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::sample::playback::SamplePlaybackEngine;
use crate::sample::SampleBank;

use effects::EffectsChain;
use synth::FundspSynth;

/// Shared state for the audio callback thread.
struct AudioState {
    sf2_synth: Option<Synthesizer>,
    fundsp_backend: SequencerBackend,
    effects: EffectsChain,
    sample_engine: SamplePlaybackEngine,
    sample_bank: Arc<SampleBank>,
}

/// Unified audio engine. Supports:
/// - SF2 playback via RustySynth (when --sf2 is provided)
/// - Built-in fundsp synth (always available)
/// - Sample playback engine (when instruments have samples assigned)
/// - Effects chain (reverb) applied to mixed output
pub struct AudioEngine {
    state: Arc<Mutex<AudioState>>,
    fundsp_synth: Arc<Mutex<FundspSynth>>,
    has_sf2: bool,
    sample_rate: f64,
    _stream: Stream,
}

impl AudioEngine {
    /// Create audio engine with optional SF2 file. If sf2_path is None,
    /// only the built-in fundsp synth is used.
    pub fn new(sf2_path: Option<&Path>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No audio output device available")?;

        let config = device
            .default_output_config()
            .context("Failed to get default audio output config")?;

        let sample_rate = config.sample_rate().0 as i32;
        let channels = config.channels() as usize;
        let sr_f64 = sample_rate as f64;

        // Load SF2 if provided
        let sf2_synth = if let Some(path) = sf2_path {
            let mut file = File::open(path)
                .with_context(|| format!("Failed to open SF2 file: {}", path.display()))?;
            let sound_font = Arc::new(
                SoundFont::new(&mut file)
                    .map_err(|e| anyhow::anyhow!("Failed to load SoundFont: {:?}", e))?,
            );
            let settings = SynthesizerSettings::new(sample_rate);
            let synth = Synthesizer::new(&sound_font, &settings)
                .map_err(|e| anyhow::anyhow!("Failed to create synthesizer: {:?}", e))?;
            Some(synth)
        } else {
            None
        };
        let has_sf2 = sf2_synth.is_some();

        // Create fundsp synth
        let mut fundsp_synth = FundspSynth::new(sr_f64);
        let fundsp_backend = fundsp_synth.backend();
        let fundsp_synth = Arc::new(Mutex::new(fundsp_synth));

        // Create effects chain
        let effects = EffectsChain::new(sr_f64);

        // Create sample playback engine
        let sample_engine = SamplePlaybackEngine::new(32);
        let sample_bank = Arc::new(SampleBank::new());

        let state = Arc::new(Mutex::new(AudioState {
            sf2_synth,
            fundsp_backend,
            effects,
            sample_engine,
            sample_bank: Arc::clone(&sample_bank),
        }));

        let state_for_callback = Arc::clone(&state);
        let stream_config: cpal::StreamConfig = config.into();

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;
                    let mut left = vec![0f32; frames];
                    let mut right = vec![0f32; frames];

                    if let Ok(mut state) = state_for_callback.try_lock() {
                        // Render SF2 synth
                        if let Some(ref mut sf2) = state.sf2_synth {
                            sf2.render(&mut left, &mut right);
                        }

                        // Render fundsp synth and mix in
                        for i in 0..frames {
                            let mut output = [0f32; 2];
                            state.fundsp_backend.tick(&[], &mut output);
                            left[i] += output[0];
                            right[i] += output[1];
                        }

                        // Render sample playback
                        {
                            let bank = Arc::clone(&state.sample_bank);
                            state.sample_engine.render(&bank, &mut left, &mut right);
                        }

                        // Apply effects chain
                        state.effects.process(&mut left, &mut right);
                    }

                    // Interleave into output buffer
                    for i in 0..frames {
                        let base = i * channels;
                        data[base] = left[i];
                        if channels > 1 {
                            data[base + 1] = right[i];
                        }
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
            state,
            fundsp_synth,
            has_sf2,
            sample_rate: sr_f64,
            _stream: stream,
        })
    }

    pub fn has_sf2(&self) -> bool {
        self.has_sf2
    }

    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) {
        // SF2 synth
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.note_on(channel as i32, note as i32, velocity as i32);
                }
            }
        }
        // fundsp synth (when no SF2, or could mix both -- for now, only when no SF2)
        if !self.has_sf2 {
            if let Ok(mut synth) = self.fundsp_synth.lock() {
                synth.note_on(channel, note, velocity);
            }
        }
    }

    #[allow(dead_code)]
    pub fn note_off(&self, channel: u8, note: u8) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.note_off(channel as i32, note as i32);
                }
            }
        }
        if !self.has_sf2 {
            if let Ok(mut synth) = self.fundsp_synth.lock() {
                synth.note_off(channel, note);
            }
        }
    }

    pub fn note_off_all_channel(&self, channel: u8) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.note_off_all_channel(channel as i32, false);
                }
            }
        }
        if !self.has_sf2 {
            if let Ok(mut synth) = self.fundsp_synth.lock() {
                synth.note_off_all_channel(channel);
            }
        }
    }

    pub fn note_off_all(&self) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.note_off_all(false);
                }
            }
        }
        if let Ok(mut synth) = self.fundsp_synth.lock() {
            synth.note_off_all();
        }
    }

    pub fn send_cc(&self, channel: u8, controller: u8, value: u8) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.process_midi_message(channel as i32, 0xB0, controller as i32, value as i32);
                }
            }
        }
    }

    pub fn program_change(&self, channel: u8, program: u8) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.process_midi_message(channel as i32, 0xC0, program as i32, 0);
                }
            }
        }
        // Also update fundsp synth patch
        if let Ok(mut synth) = self.fundsp_synth.lock() {
            synth.program_change(channel, program);
        }
    }

    pub fn pitch_bend(&self, channel: u8, value: u16) {
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    let lsb = (value & 0x7F) as i32;
                    let msb = ((value >> 7) & 0x7F) as i32;
                    sf2.process_midi_message(channel as i32, 0xE0, lsb, msb);
                }
            }
        }
        // fundsp pitch bend: not yet supported (voices are fixed frequency)
    }

    /// Toggle effects chain on/off
    #[allow(dead_code)]
    pub fn toggle_effects(&self) -> bool {
        if let Ok(mut state) = self.state.lock() {
            state.effects.enabled = !state.effects.enabled;
            state.effects.enabled
        } else {
            false
        }
    }

    pub fn effects_enabled(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.effects.enabled
        } else {
            false
        }
    }

    /// Update the sample bank (called when samples are loaded/modified)
    pub fn set_sample_bank(&self, bank: Arc<SampleBank>) {
        if let Ok(mut state) = self.state.lock() {
            state.sample_bank = bank;
        }
    }

    /// Trigger a sample voice
    pub fn sample_note_on(&self, sample_index: usize, note: u8, velocity: u8, channel: u8) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(sample) = state.sample_bank.get(sample_index) {
                // We need the sample data but can't hold both borrows, so clone the needed info
                let base_note = sample.base_note;
                let sample_rate = sample.sample_rate;
                let trim_start = sample.trim_start;
                let pitch_ratio = 2.0_f64.powf((note as f64 - base_note as f64) / 12.0);
                let rate_ratio = sample_rate / self.sample_rate;
                let rate = pitch_ratio * rate_ratio;
                let vel = velocity as f32 / 127.0;

                // Kill existing voice for same channel+note
                state.sample_engine.note_off(channel, note);

                // Evict oldest voice if at capacity
                if state.sample_engine.voices.len() >= state.sample_engine.max_voices {
                    if let Some(idx) = state.sample_engine.voices.iter().position(|v| !v.active) {
                        state.sample_engine.voices.remove(idx);
                    } else {
                        state.sample_engine.voices.remove(0);
                    }
                }

                use crate::sample::playback::SampleVoice;
                state.sample_engine.voices.push(SampleVoice {
                    sample_index,
                    position: trim_start as f64,
                    rate,
                    velocity: vel,
                    channel,
                    note,
                    active: true,
                });
            }
        }
    }

    /// Stop a sample voice
    #[allow(dead_code)]
    pub fn sample_note_off(&self, channel: u8, note: u8) {
        if let Ok(mut state) = self.state.lock() {
            state.sample_engine.note_off(channel, note);
        }
    }

    /// Stop all sample voices for a channel
    pub fn sample_note_off_channel(&self, channel: u8) {
        if let Ok(mut state) = self.state.lock() {
            state.sample_engine.note_off_channel(channel);
        }
    }

    /// Stop all sample voices
    pub fn sample_note_off_all(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.sample_engine.note_off_all();
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_requires_valid_sf2() {
        let result = AudioEngine::new(Some(Path::new("/nonexistent/file.sf2")));
        assert!(result.is_err());
    }
}
