pub mod channel_effects;
pub mod effects;
pub mod envelope;
pub mod synth;

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use rtrb::{Producer, RingBuffer};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::audio::envelope::Envelope;
use crate::sample::playback::{SamplePlaybackEngine, SampleVoice};
use crate::sample::SampleBank;

use channel_effects::{ChannelEffects, ChannelEffectsParams, MAX_EFFECT_CHANNELS};
use effects::EffectsChain;
use synth::{BuiltinSynth, SynthParams};

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

/// Maximum number of frames per audio callback buffer.
/// CoreAudio on macOS typically uses 512-1024 frames; we allocate enough for 4096.
const MAX_CALLBACK_FRAMES: usize = 4096;

/// Ring buffer capacity for audio commands. Must be large enough to hold all
/// commands between audio callbacks (~5-10ms at typical buffer sizes).
const COMMAND_QUEUE_CAPACITY: usize = 256;

/// Commands sent from the UI thread to the audio thread via lock-free ring buffer.
enum AudioCommand {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOnWithParams { channel: u8, note: u8, velocity: u8, params: Box<SynthParams> },
    NoteOff { channel: u8, note: u8 },
    NoteOffAllChannel { channel: u8 },
    NoteOffAll,
    SendCC { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    PitchBend { channel: u8, value: u16 },
    ToggleEffects,
    SetSampleBank { bank: Arc<SampleBank> },
    SampleNoteOn { sample_index: usize, note: u8, velocity: u8, channel: u8 },
    SampleNoteOff { channel: u8, note: u8 },
    SampleNoteOffChannel { channel: u8 },
    SampleNoteOffAll,
    SetChannelEffects { channel: u8, params: Box<ChannelEffectsParams> },
    SetSendBusParams { bus: u8, params: Box<effects::SendBusParams> },
}

/// Unified audio engine. Supports:
/// - SF2 playback via RustySynth (when --sf2 is provided)
/// - Built-in subtractive synth with ADSR + SVF filter (always available)
/// - Sample playback engine (when instruments have samples assigned)
/// - Effects chain (delay) applied to mixed output
///
/// Uses a lock-free command queue: the UI thread sends commands via a ring buffer,
/// and the audio callback thread owns all synthesis state, draining commands at
/// the start of each callback. No mutex is held during audio rendering.
pub struct AudioEngine {
    producer: Producer<AudioCommand>,
    has_sf2: bool,
    effects_enabled: Arc<AtomicBool>,
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

        // Create built-in synth
        let builtin_synth = BuiltinSynth::new(sr_f64);

        // Create effects chain (master)
        let effects = EffectsChain::new(sr_f64);

        // Create per-channel effects
        let channel_effects: Vec<ChannelEffects> = (0..MAX_EFFECT_CHANNELS)
            .map(|_| ChannelEffects::new(sr_f64))
            .collect();

        // Create send/return buses
        let send_buses: Vec<effects::SendBus> = (0..effects::MAX_SEND_BUSES)
            .map(|_| effects::SendBus::new(sr_f64))
            .collect();

        // Create sample playback engine
        let sample_engine = SamplePlaybackEngine::new(32);
        let sample_bank = Arc::new(SampleBank::new());

        // Lock-free command queue
        let (producer, consumer) = RingBuffer::new(COMMAND_QUEUE_CAPACITY);

        // Shared effects-enabled flag (read by UI for status bar, toggled via command)
        let effects_enabled = Arc::new(AtomicBool::new(true));
        let effects_flag = Arc::clone(&effects_enabled);

        let stream_config: cpal::StreamConfig = config.into();
        let callback_has_sf2 = has_sf2;
        let callback_sr = sr_f64;

        let stream = {
            // All audio state moves into the closure
            let mut sf2_synth = sf2_synth;
            let mut builtin_synth = builtin_synth;
            let mut effects = effects;
            let mut channel_effects = channel_effects;
            let mut sample_engine = sample_engine;
            let mut sample_bank = sample_bank;
            let mut send_buses = send_buses;
            let mut consumer = consumer;
            let mut scratch_left = vec![0.0f32; MAX_CALLBACK_FRAMES];
            let mut scratch_right = vec![0.0f32; MAX_CALLBACK_FRAMES];
            // Per-channel scratch buffers for channel effects
            let mut ch_buf_left: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
                .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                .collect();
            let mut ch_buf_right: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
                .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                .collect();

            device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let frames = data.len() / channels;

                        // Drain command queue (lock-free, no allocation)
                        while let Ok(cmd) = consumer.pop() {
                            process_command(
                                cmd,
                                &mut sf2_synth,
                                &mut builtin_synth,
                                &mut effects,
                                &mut channel_effects,
                                &mut send_buses,
                                &mut sample_engine,
                                &mut sample_bank,
                                callback_has_sf2,
                                callback_sr,
                                &effects_flag,
                            );
                        }

                        // Grow scratch buffers if needed (rare)
                        if scratch_left.len() < frames {
                            scratch_left.resize(frames, 0.0);
                            scratch_right.resize(frames, 0.0);
                            for ch in 0..MAX_EFFECT_CHANNELS {
                                ch_buf_left[ch].resize(frames, 0.0);
                                ch_buf_right[ch].resize(frames, 0.0);
                            }
                        }

                        let left = &mut scratch_left[..frames];
                        let right = &mut scratch_right[..frames];

                        // Zero the master scratch buffers
                        for s in left.iter_mut() { *s = 0.0; }
                        for s in right.iter_mut() { *s = 0.0; }

                        // Check if any channel has effects enabled
                        let any_ch_fx = channel_effects.iter().any(|fx| fx.any_enabled());

                        // Render SF2 synth (always to master -- can't separate by channel)
                        if let Some(ref mut sf2) = sf2_synth {
                            sf2.render(left, right);
                        }

                        // Check if any send bus is enabled
                        let any_send_bus = send_buses.iter().any(|b| b.params.enabled);

                        if any_ch_fx || any_send_bus {
                            // Per-channel rendering path (needed for channel effects or send buses)
                            // Zero per-channel buffers
                            for ch in 0..MAX_EFFECT_CHANNELS {
                                for s in ch_buf_left[ch][..frames].iter_mut() { *s = 0.0; }
                                for s in ch_buf_right[ch][..frames].iter_mut() { *s = 0.0; }
                            }

                            // Render built-in synth per-channel
                            for i in 0..frames {
                                let mut ch_out = [[0.0f32; 2]; MAX_EFFECT_CHANNELS];
                                builtin_synth.render_sample_per_channel(&mut ch_out);
                                for ch in 0..MAX_EFFECT_CHANNELS {
                                    ch_buf_left[ch][i] += ch_out[ch][0];
                                    ch_buf_right[ch][i] += ch_out[ch][1];
                                }
                            }

                            // Render samples per-channel
                            {
                                let mut slices: Vec<(&mut [f32], &mut [f32])> = Vec::with_capacity(MAX_EFFECT_CHANNELS);
                                let (ch_l_slices, ch_r_slices) = (&mut ch_buf_left, &mut ch_buf_right);
                                for ch in 0..MAX_EFFECT_CHANNELS {
                                    let l = &mut ch_l_slices[ch][..frames] as *mut [f32];
                                    let r = &mut ch_r_slices[ch][..frames] as *mut [f32];
                                    // SAFETY: each channel index is unique, no aliasing
                                    unsafe { slices.push((&mut *l, &mut *r)); }
                                }
                                sample_engine.render_per_channel(&sample_bank, &mut slices);
                            }

                            // Clear send bus inputs
                            for bus in send_buses.iter_mut() {
                                bus.ensure_size(frames);
                                bus.clear_inputs(frames);
                            }

                            // Apply per-channel effects, feed send buses, sum to master
                            for ch in 0..MAX_EFFECT_CHANNELS {
                                channel_effects[ch].process(
                                    &mut ch_buf_left[ch][..frames],
                                    &mut ch_buf_right[ch][..frames],
                                );

                                // Feed send buses (post-channel-effects)
                                let send_levels = channel_effects[ch].params.send_levels;
                                for (bus_idx, bus) in send_buses.iter_mut().enumerate() {
                                    if bus.params.enabled && send_levels[bus_idx] > 0.0 {
                                        bus.add_send(
                                            &ch_buf_left[ch][..frames],
                                            &ch_buf_right[ch][..frames],
                                            send_levels[bus_idx],
                                        );
                                    }
                                }

                                for i in 0..frames {
                                    left[i] += ch_buf_left[ch][i];
                                    right[i] += ch_buf_right[ch][i];
                                }
                            }

                            // Process send buses to master
                            for bus in send_buses.iter_mut() {
                                bus.process_to_master(left, right, frames);
                            }
                        } else {
                            // Fast path: no per-channel effects, render directly to master
                            for i in 0..frames {
                                let (l, r) = builtin_synth.render_sample();
                                left[i] += l;
                                right[i] += r;
                            }
                            sample_engine.render(&sample_bank, left, right);
                        }

                        // Apply master effects chain
                        effects.process(left, right);

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
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
                .context("Failed to build audio output stream")?
        };

        stream.play().context("Failed to start audio stream")?;

        Ok(Self {
            producer,
            has_sf2,
            effects_enabled,
            sample_rate: sr_f64,
            _stream: stream,
        })
    }

    /// Send a command to the audio thread. If the queue is full, the command is dropped.
    #[inline]
    fn send(&mut self, cmd: AudioCommand) {
        let _ = self.producer.push(cmd);
    }

    pub fn has_sf2(&self) -> bool {
        self.has_sf2
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.send(AudioCommand::NoteOn { channel, note, velocity });
    }

    pub fn note_on_with_params(&mut self, channel: u8, note: u8, velocity: u8, params: &SynthParams) {
        self.send(AudioCommand::NoteOnWithParams {
            channel, note, velocity,
            params: Box::new(params.clone()),
        });
    }

    #[allow(dead_code)]
    pub fn note_off(&mut self, channel: u8, note: u8) {
        self.send(AudioCommand::NoteOff { channel, note });
    }

    pub fn note_off_all_channel(&mut self, channel: u8) {
        self.send(AudioCommand::NoteOffAllChannel { channel });
    }

    pub fn note_off_all(&mut self) {
        self.send(AudioCommand::NoteOffAll);
    }

    pub fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        self.send(AudioCommand::SendCC { channel, controller, value });
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.send(AudioCommand::ProgramChange { channel, program });
    }

    pub fn pitch_bend(&mut self, channel: u8, value: u16) {
        self.send(AudioCommand::PitchBend { channel, value });
    }

    /// Toggle effects chain on/off
    #[allow(dead_code)]
    pub fn toggle_effects(&mut self) -> bool {
        self.send(AudioCommand::ToggleEffects);
        // Toggle the local flag too so the UI sees the new state immediately
        let prev = self.effects_enabled.load(Ordering::Relaxed);
        let new = !prev;
        self.effects_enabled.store(new, Ordering::Relaxed);
        new
    }

    pub fn effects_enabled(&self) -> bool {
        self.effects_enabled.load(Ordering::Relaxed)
    }

    /// Update the sample bank (called when samples are loaded/modified)
    pub fn set_sample_bank(&mut self, bank: Arc<SampleBank>) {
        self.send(AudioCommand::SetSampleBank { bank });
    }

    /// Trigger a sample voice
    pub fn sample_note_on(&mut self, sample_index: usize, note: u8, velocity: u8, channel: u8) {
        self.send(AudioCommand::SampleNoteOn { sample_index, note, velocity, channel });
    }

    /// Stop a sample voice
    #[allow(dead_code)]
    pub fn sample_note_off(&mut self, channel: u8, note: u8) {
        self.send(AudioCommand::SampleNoteOff { channel, note });
    }

    /// Stop all sample voices for a channel
    pub fn sample_note_off_channel(&mut self, channel: u8) {
        self.send(AudioCommand::SampleNoteOffChannel { channel });
    }

    /// Stop all sample voices
    pub fn sample_note_off_all(&mut self) {
        self.send(AudioCommand::SampleNoteOffAll);
    }

    /// Set per-channel effects parameters
    pub fn set_channel_effects(&mut self, channel: u8, params: &ChannelEffectsParams) {
        self.send(AudioCommand::SetChannelEffects {
            channel,
            params: Box::new(params.clone()),
        });
    }

    /// Set parameters for a send/return bus
    pub fn set_send_bus_params(&mut self, bus: u8, params: &effects::SendBusParams) {
        self.send(AudioCommand::SetSendBusParams {
            bus,
            params: Box::new(params.clone()),
        });
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

/// Process a single command on the audio thread. Called from inside the audio callback.
fn process_command(
    cmd: AudioCommand,
    sf2_synth: &mut Option<Synthesizer>,
    builtin_synth: &mut BuiltinSynth,
    effects: &mut EffectsChain,
    channel_effects: &mut [ChannelEffects],
    send_buses: &mut [effects::SendBus],
    sample_engine: &mut SamplePlaybackEngine,
    sample_bank: &mut Arc<SampleBank>,
    has_sf2: bool,
    sample_rate: f64,
    effects_flag: &AtomicBool,
) {
    match cmd {
        AudioCommand::NoteOn { channel, note, velocity } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.note_off_all_channel(channel as i32, false);
                sf2.note_on(channel as i32, note as i32, velocity as i32);
            }
            if !has_sf2 {
                builtin_synth.note_on(channel, note, velocity);
            }
        }
        AudioCommand::NoteOnWithParams { channel, note, velocity, params } => {
            builtin_synth.note_on_with_params(channel, note, velocity, &params);
        }
        AudioCommand::NoteOff { channel, note } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.note_off(channel as i32, note as i32);
            }
            if !has_sf2 {
                builtin_synth.note_off(channel, note);
            }
        }
        AudioCommand::NoteOffAllChannel { channel } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.note_off_all_channel(channel as i32, false);
            }
            if !has_sf2 {
                builtin_synth.note_off_all_channel(channel);
            }
        }
        AudioCommand::NoteOffAll => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.note_off_all(false);
            }
            builtin_synth.note_off_all();
        }
        AudioCommand::SendCC { channel, controller, value } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.process_midi_message(channel as i32, 0xB0, controller as i32, value as i32);
            }
        }
        AudioCommand::ProgramChange { channel, program } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.process_midi_message(channel as i32, 0xC0, program as i32, 0);
            }
            builtin_synth.program_change(channel, program);
        }
        AudioCommand::PitchBend { channel, value } => {
            if let Some(ref mut sf2) = sf2_synth {
                let lsb = (value & 0x7F) as i32;
                let msb = ((value >> 7) & 0x7F) as i32;
                sf2.process_midi_message(channel as i32, 0xE0, lsb, msb);
            }
        }
        AudioCommand::ToggleEffects => {
            effects.enabled = !effects.enabled;
            effects_flag.store(effects.enabled, Ordering::Relaxed);
        }
        AudioCommand::SetSampleBank { bank } => {
            *sample_bank = bank;
        }
        AudioCommand::SampleNoteOn { sample_index, note, velocity, channel } => {
            if let Some(sample) = sample_bank.get(sample_index) {
                let base_note = sample.base_note;
                let sr = sample.sample_rate;
                let trim_start = sample.trim_start;
                let pitch_ratio = 2.0_f64.powf((note as f64 - base_note as f64) / 12.0);
                let rate_ratio = sr / sample_rate;
                let rate = pitch_ratio * rate_ratio;
                let vel = velocity as f32 / 127.0;

                sample_engine.note_off(channel, note);

                // Evict quietest voice if at capacity
                if sample_engine.voices.len() >= sample_engine.max_voices {
                    if let Some(idx) = sample_engine.voices.iter().position(|v| !v.active) {
                        sample_engine.voices.remove(idx);
                    } else {
                        let quietest = sample_engine.voices.iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| {
                                let a_level = a.envelope.level * a.velocity;
                                let b_level = b.envelope.level * b.velocity;
                                a_level.partial_cmp(&b_level).unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .map(|(i, _)| i);
                        if let Some(idx) = quietest {
                            sample_engine.voices.remove(idx);
                        }
                    }
                }

                sample_engine.voices.push(SampleVoice {
                    sample_index,
                    position: trim_start as f64,
                    rate,
                    velocity: vel,
                    channel,
                    note,
                    active: true,
                    envelope: Envelope::sample_default(sample_rate as f32),
                });
            }
        }
        AudioCommand::SampleNoteOff { channel, note } => {
            sample_engine.note_off(channel, note);
        }
        AudioCommand::SampleNoteOffChannel { channel } => {
            sample_engine.note_off_channel(channel);
        }
        AudioCommand::SampleNoteOffAll => {
            sample_engine.note_off_all();
        }
        AudioCommand::SetChannelEffects { channel, params } => {
            let ch = channel as usize;
            if ch < channel_effects.len() {
                channel_effects[ch].params = *params;
            }
        }
        AudioCommand::SetSendBusParams { bus, params } => {
            let idx = bus as usize;
            if idx < send_buses.len() {
                send_buses[idx].params = *params;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::synth::BuiltinSynth;
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
            if let Ok(configs) = device.supported_output_configs() {
                for c in configs {
                    eprintln!("  Supported: {:?} {}ch {:?}",
                        c.sample_format(), c.channels(), c.buffer_size());
                }
            }
        }
    }

    #[test]
    fn test_no_buffer_boundary_clicks() {
        let sr = 48000.0;
        let mut synth = BuiltinSynth::new(sr);
        let mut effects = EffectsChain::new(sr);

        synth.program_change(0, 2); // Sine
        synth.note_on(0, 69, 127);

        let chunk = 512;
        let mut all_samples = Vec::new();

        for _ in 0..10 {
            let mut left = vec![0f32; chunk];
            let mut right = vec![0f32; chunk];
            for i in 0..chunk {
                let (l, r) = synth.render_sample();
                left[i] = l;
                right[i] = r;
            }
            effects.process(&mut left, &mut right);
            all_samples.extend_from_slice(&left);
        }

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

    #[test]
    fn test_callback_interleaving() {
        let sr = 44100.0;
        let channels = 2usize;
        let frames = 512;

        let mut synth = BuiltinSynth::new(sr);
        let mut effects = EffectsChain::new(sr);

        synth.program_change(0, 2); // Sine
        synth.note_on(0, 69, 127);

        // Let the note settle (past attack)
        for _ in 0..4410 {
            synth.render_sample();
        }

        let mut left = vec![0f32; frames];
        let mut right = vec![0f32; frames];
        for i in 0..frames {
            let (l, r) = synth.render_sample();
            left[i] = l;
            right[i] = r;
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

        let peak = data.iter().fold(0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.05, "Signal too quiet: peak={:.4}", peak);
        assert!(peak < 1.0, "Signal clips: peak={:.4}", peak);

        // Sine patch: L and R should be identical
        let mut diff_sum = 0f32;
        for i in 0..frames {
            diff_sum += (data[i*2] - data[i*2+1]).abs();
        }
        let avg_diff = diff_sum / frames as f32;
        eprintln!("Interleave test: peak={:.4}, avg L-R diff={:.6}", peak, avg_diff);
    }
}
