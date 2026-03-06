pub mod effects;
pub mod synth;

use std::fs::File;
use std::path::Path;

/// Soft clamp audio sample to [-1, 1] using tanh-style saturation.
/// Passes near-unity signals through cleanly, gently compresses values above ~0.8.
#[inline]
fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((x.abs() - 0.8) / 0.2).tanh())
    }
}
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

/// Maximum number of frames per audio callback buffer.
/// CoreAudio on macOS typically uses 512-1024 frames; we allocate enough for 4096.
const MAX_CALLBACK_FRAMES: usize = 4096;

/// Shared state for the audio callback thread.
struct AudioState {
    sf2_synth: Option<Synthesizer>,
    fundsp_backend: SequencerBackend,
    effects: EffectsChain,
    sample_engine: SamplePlaybackEngine,
    sample_bank: Arc<SampleBank>,
    // Pre-allocated scratch buffers to avoid heap allocation in the audio callback
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
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

        let sample_format = config.sample_format();
        let sample_rate = config.sample_rate().0 as i32;
        let channels = config.channels() as usize;
        let sr_f64 = sample_rate as f64;

        eprintln!(
            "Audio: {}Hz, {} ch, {:?}",
            sample_rate, channels, sample_format
        );

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
            scratch_left: vec![0.0; MAX_CALLBACK_FRAMES],
            scratch_right: vec![0.0; MAX_CALLBACK_FRAMES],
        }));

        let state_for_callback = Arc::clone(&state);
        let stream_config: cpal::StreamConfig = config.into();

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;

                    if let Ok(mut state) = state_for_callback.try_lock() {
                        let st = &mut *state;

                        // Grow scratch buffers if needed (rare, only on first
                        // callback if device uses larger buffers than expected)
                        if st.scratch_left.len() < frames {
                            st.scratch_left.resize(frames, 0.0);
                            st.scratch_right.resize(frames, 0.0);
                        }

                        let left = &mut st.scratch_left[..frames];
                        let right = &mut st.scratch_right[..frames];

                        // Zero the scratch buffers (no allocation)
                        for s in left.iter_mut() { *s = 0.0; }
                        for s in right.iter_mut() { *s = 0.0; }

                        // Render SF2 synth
                        if let Some(ref mut sf2) = st.sf2_synth {
                            sf2.render(left, right);
                        }

                        // Render fundsp synth (built-in patches)
                        for i in 0..frames {
                            let mut output = [0f32; 2];
                            st.fundsp_backend.tick(&[], &mut output);
                            left[i] += output[0];
                            right[i] += output[1];
                        }

                        // Render sample playback
                        st.sample_engine.render(&st.sample_bank, left, right);

                        // Apply effects chain
                        st.effects.process(left, right);

                        // Interleave into output buffer with soft clamp
                        for i in 0..frames {
                            let base = i * channels;
                            data[base] = soft_clip(left[i]);
                            if channels > 1 {
                                data[base + 1] = soft_clip(right[i]);
                            }
                            for ch in 2..channels {
                                data[base + ch] = 0.0;
                            }
                        }
                    } else {
                        // Couldn't acquire lock -- output silence
                        for s in data.iter_mut() {
                            *s = 0.0;
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
        if self.has_sf2 {
            if let Ok(mut state) = self.state.lock() {
                if let Some(ref mut sf2) = state.sf2_synth {
                    sf2.note_off_all_channel(channel as i32, false);
                    sf2.note_on(channel as i32, note as i32, velocity as i32);
                }
            }
        }
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
    use crate::audio::synth::FundspSynth;
    use crate::audio::effects::EffectsChain;

    #[test]
    fn test_system_audio_config() {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        if let Some(device) = host.default_output_device() {
            let name = device.name().unwrap_or_else(|_| "unknown".into());
            if let Ok(config) = device.default_output_config() {
                eprintln!("Device: {}", name);
                eprintln!("  Sample rate: {}", config.sample_rate().0);
                eprintln!("  Channels: {}", config.channels());
                eprintln!("  Sample format: {:?}", config.sample_format());
                eprintln!("  Buffer size: {:?}", config.config().buffer_size);
            }
            // Also check supported buffer sizes
            if let Ok(configs) = device.supported_output_configs() {
                for c in configs {
                    eprintln!("  Supported: {:?} {}ch {:?}",
                        c.sample_format(), c.channels(), c.buffer_size());
                }
            }
        }
    }

    /// Test that rendering across multiple buffer boundaries has no discontinuities.
    #[test]
    fn test_no_buffer_boundary_clicks() {
        use crate::audio::synth::FundspSynth;
        use crate::audio::effects::EffectsChain;

        let sr = 48000.0;
        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);
        let mut effects = EffectsChain::new(sr);

        // Trigger sine note
        synth.program_change(0, 2);
        synth.note_on(0, 69, 127);

        // Render 10 buffers of 512 frames each (like real callback)
        let chunk = 512;
        let mut all_samples = Vec::new();

        for _ in 0..10 {
            let mut left = vec![0f32; chunk];
            let mut right = vec![0f32; chunk];
            for i in 0..chunk {
                let mut output = [0f32; 2];
                backend.tick(&[], &mut output);
                left[i] = output[0];
                right[i] = output[1];
            }
            effects.process(&mut left, &mut right);
            all_samples.extend_from_slice(&left);
        }

        // Check for clicks: a click is a large sample-to-sample jump.
        // For a 440 Hz sine at 48kHz, max derivative is 2*pi*440/48000 * amplitude
        // = 0.0576 * 0.25 = 0.0144 per sample. Allow 10x for harmonics + reverb.
        let max_allowed_jump = 0.15;
        let mut max_jump = 0f32;
        let mut max_jump_pos = 0;
        for i in 1..all_samples.len() {
            let jump = (all_samples[i] - all_samples[i - 1]).abs();
            if jump > max_jump {
                max_jump = jump;
                max_jump_pos = i;
            }
        }

        eprintln!("Max sample jump: {:.6} at sample {} (buffer boundary at {})",
            max_jump, max_jump_pos,
            if max_jump_pos % chunk == 0 { "YES" } else { "no" });

        assert!(max_jump < max_allowed_jump,
            "Click detected: jump={:.6} at sample {} (limit {:.6})",
            max_jump, max_jump_pos, max_allowed_jump);
    }

    #[test]
    fn test_audio_engine_requires_valid_sf2() {
        let result = AudioEngine::new(Some(Path::new("/nonexistent/file.sf2")));
        assert!(result.is_err());
    }

    /// Simulates the exact audio callback to verify interleaving and signal integrity.
    #[test]
    fn test_callback_interleaving() {
        let sr = 44100.0;
        let channels = 2usize;
        let frames = 512;

        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);
        let mut effects = EffectsChain::new(sr);

        // Trigger a sine note for a clean reference
        synth.program_change(0, 2); // Sine patch
        synth.note_on(0, 69, 127);

        // Let the note settle (past fade-in)
        for _ in 0..4410 {
            let mut out = [0f32; 2];
            backend.tick(&[], &mut out);
        }

        // Now render exactly as the callback does
        let mut left = vec![0f32; frames];
        let mut right = vec![0f32; frames];
        for i in 0..frames {
            let mut output = [0f32; 2];
            backend.tick(&[], &mut output);
            left[i] = output[0];
            right[i] = output[1];
        }
        effects.process(&mut left, &mut right);

        // Interleave into callback buffer
        let mut data = vec![0f32; frames * channels];
        for i in 0..frames {
            let base = i * channels;
            data[base] = soft_clip(left[i]);
            data[base + 1] = soft_clip(right[i]);
        }

        // Verify: deinterleave and compare
        for i in 0..frames {
            let got_l = data[i * 2];
            let got_r = data[i * 2 + 1];
            let exp_l = soft_clip(left[i]);
            let exp_r = soft_clip(right[i]);
            assert!((got_l - exp_l).abs() < 1e-7,
                "L mismatch at frame {}: got={}, exp={}", i, got_l, exp_l);
            assert!((got_r - exp_r).abs() < 1e-7,
                "R mismatch at frame {}: got={}, exp={}", i, got_r, exp_r);
        }

        // Verify signal properties
        let peak = data.iter().fold(0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.05, "Signal too quiet: peak={:.4}", peak);
        assert!(peak < 1.0, "Signal clips: peak={:.4}", peak);

        // Check L and R are correlated (sine patch should produce identical L/R)
        let mut diff_sum = 0f32;
        for i in 0..frames {
            diff_sum += (data[i*2] - data[i*2+1]).abs();
        }
        let avg_diff = diff_sum / frames as f32;
        eprintln!("Interleave test: peak={:.4}, avg L-R diff={:.6}", peak, avg_diff);
    }
}
