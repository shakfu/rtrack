pub mod channel_effects;
pub mod effects;
pub mod envelope;
pub mod synth;

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use rtrb::{Consumer, Producer, RingBuffer};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::sample::playback::{NewNoteAction, SamplePlaybackEngine};
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

/// Ring buffer capacity for visualization samples (mono, L+R averaged).
/// 8192 samples at 48kHz ~ 170ms, enough for several GUI frames.
const VIS_BUFFER_CAPACITY: usize = 8192;

/// Snapshot of a single active sample voice for UI visualization.
#[derive(Clone, Debug)]
pub struct VoiceSnapshot {
    pub sample_index: usize,
    pub position: f64,
    pub channel: u8,
    pub note: u8,
    pub velocity: f32,
    pub active: bool,
}

/// Commands sent from the UI thread to the audio thread via lock-free ring buffer.
enum AudioCommand {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOnWithParams {
        channel: u8,
        note: u8,
        velocity: u8,
        params: Box<SynthParams>,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    NoteOffAllChannel {
        channel: u8,
    },
    NoteOffAll,
    SendCC {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ToggleEffects,
    SetSampleBank {
        bank: Arc<SampleBank>,
    },
    SampleNoteOn {
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        action: NewNoteAction,
    },
    SampleNoteOff {
        channel: u8,
        note: u8,
    },
    SampleNoteOffChannel {
        channel: u8,
    },
    SampleNoteOffAll,
    SetChannelEffects {
        channel: u8,
        params: Box<ChannelEffectsParams>,
    },
    SetSendBusParams {
        bus: u8,
        params: Box<effects::SendBusParams>,
    },
}

/// Capacity of the audio-to-UI reclaim queue. Values pushed here are dropped
/// by the UI thread; see [`Reclaimed`].
const RECLAIM_QUEUE_CAPACITY: usize = 64;

/// A value the audio thread has finished with, shipped back to the UI thread
/// to be dropped there.
///
/// Freeing memory takes a lock inside the allocator, which the audio callback
/// cannot afford: dropping a `SampleBank` holding the last reference to a few
/// megabytes of PCM is exactly the kind of thing that produces an occasional
/// unexplained click.
// The payloads are never read: each variant exists purely to carry ownership
// across the thread boundary so that `drop` runs on the UI side.
#[allow(dead_code)]
enum Reclaimed {
    Bank(Arc<SampleBank>),
    SynthParams(Box<SynthParams>),
    ChannelEffects(Box<ChannelEffectsParams>),
    SendBus(Box<effects::SendBusParams>),
}

/// A command tagged with the audio-clock frame at which it should take
/// effect. Frame 0 means "as soon as this callback sees it".
///
/// Timestamping is what decouples note timing from the UI frame rate: the
/// sequencer runs slightly ahead of the audio clock and stamps each event
/// with its exact target frame, so a late or jittery UI frame still produces
/// audio at the right instant, as long as it is not later than the lookahead.
struct TimedCommand {
    frame: u64,
    cmd: AudioCommand,
}

/// Everything the audio thread owns. Bundled into one struct so the callback
/// can render the buffer in segments, applying commands between them, without
/// threading a dozen `&mut` parameters through every call.
struct RenderState {
    sf2_synth: Option<Synthesizer>,
    builtin_synth: BuiltinSynth,
    effects: EffectsChain,
    channel_effects: Vec<ChannelEffects>,
    send_buses: Vec<effects::SendBus>,
    sample_engine: SamplePlaybackEngine,
    sample_bank: Arc<SampleBank>,
    has_sf2: bool,
    sample_rate: f64,

    // Master mix scratch, sized to the largest callback seen so far.
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    // Per-channel scratch for the channel-effects path.
    ch_buf_left: Vec<Vec<f32>>,
    ch_buf_right: Vec<Vec<f32>>,
}

impl RenderState {
    /// Build a render state with no SF2 synth, for tests and offline checks.
    /// Mirrors what `AudioEngine::new` assembles, minus the output device.
    #[cfg(test)]
    fn for_test(sample_rate: f64) -> Self {
        Self {
            sf2_synth: None,
            builtin_synth: BuiltinSynth::new(sample_rate),
            effects: EffectsChain::new(sample_rate),
            channel_effects: (0..MAX_EFFECT_CHANNELS)
                .map(|_| ChannelEffects::new(sample_rate))
                .collect(),
            send_buses: (0..effects::MAX_SEND_BUSES)
                .map(|_| effects::SendBus::new(sample_rate))
                .collect(),
            sample_engine: SamplePlaybackEngine::new(32),
            sample_bank: Arc::new(SampleBank::new()),
            has_sf2: false,
            sample_rate,
            scratch_left: vec![0.0f32; MAX_CALLBACK_FRAMES],
            scratch_right: vec![0.0f32; MAX_CALLBACK_FRAMES],
            ch_buf_left: (0..MAX_EFFECT_CHANNELS)
                .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                .collect(),
            ch_buf_right: (0..MAX_EFFECT_CHANNELS)
                .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                .collect(),
        }
    }

    /// Grow the scratch buffers if this callback is larger than any so far.
    /// Allocation on the audio thread is undesirable, hence sizing for
    /// `MAX_CALLBACK_FRAMES` up front and treating this as a rare fallback.
    fn ensure_capacity(&mut self, frames: usize) {
        if self.scratch_left.len() >= frames {
            return;
        }
        self.scratch_left.resize(frames, 0.0);
        self.scratch_right.resize(frames, 0.0);
        for ch in 0..MAX_EFFECT_CHANNELS {
            self.ch_buf_left[ch].resize(frames, 0.0);
            self.ch_buf_right[ch].resize(frames, 0.0);
        }
    }

    /// Render `range` frames of the master mix into the scratch buffers.
    ///
    /// Rendering a sub-range rather than the whole callback is what lets the
    /// caller apply a command exactly at its target sample offset. All the
    /// DSP here is sample-wise or block-transparent, so splitting a buffer
    /// into segments produces the same output as rendering it in one go.
    fn render_segment(&mut self, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let frames = range.len();
        let (start, end) = (range.start, range.end);

        let left = &mut self.scratch_left[start..end];
        let right = &mut self.scratch_right[start..end];
        for s in left.iter_mut() {
            *s = 0.0;
        }
        for s in right.iter_mut() {
            *s = 0.0;
        }

        // Render SF2 synth (always to master -- can't separate by channel)
        if let Some(ref mut sf2) = self.sf2_synth {
            sf2.render(left, right);
        }

        let any_ch_fx = self.channel_effects.iter().any(|fx| fx.any_enabled());
        let any_send_bus = self.send_buses.iter().any(|b| b.params.enabled);

        if any_ch_fx || any_send_bus {
            // Per-channel path (needed for channel effects or send buses)
            for ch in 0..MAX_EFFECT_CHANNELS {
                for s in self.ch_buf_left[ch][start..end].iter_mut() {
                    *s = 0.0;
                }
                for s in self.ch_buf_right[ch][start..end].iter_mut() {
                    *s = 0.0;
                }
            }

            for i in start..end {
                let mut ch_out = [[0.0f32; 2]; MAX_EFFECT_CHANNELS];
                self.builtin_synth.render_sample_per_channel(&mut ch_out);
                for (ch, out) in ch_out.iter().enumerate() {
                    self.ch_buf_left[ch][i] += out[0];
                    self.ch_buf_right[ch][i] += out[1];
                }
            }

            self.sample_engine.render_per_channel(
                &self.sample_bank,
                &mut self.ch_buf_left,
                &mut self.ch_buf_right,
                start..end,
            );

            for bus in self.send_buses.iter_mut() {
                bus.ensure_size(frames);
                bus.clear_inputs(frames);
            }

            let left = &mut self.scratch_left[start..end];
            let right = &mut self.scratch_right[start..end];
            for ch in 0..MAX_EFFECT_CHANNELS {
                self.channel_effects[ch].process(
                    &mut self.ch_buf_left[ch][start..end],
                    &mut self.ch_buf_right[ch][start..end],
                );

                // Feed send buses (post-channel-effects)
                let send_levels = self.channel_effects[ch].params.send_levels;
                for (bus_idx, bus) in self.send_buses.iter_mut().enumerate() {
                    if bus.params.enabled && send_levels[bus_idx] > 0.0 {
                        bus.add_send(
                            &self.ch_buf_left[ch][start..end],
                            &self.ch_buf_right[ch][start..end],
                            send_levels[bus_idx],
                        );
                    }
                }

                for i in 0..frames {
                    left[i] += self.ch_buf_left[ch][start + i];
                    right[i] += self.ch_buf_right[ch][start + i];
                }
            }

            for bus in self.send_buses.iter_mut() {
                bus.process_to_master(left, right, frames);
            }
        } else {
            // Fast path: no per-channel effects, render directly to master
            let left = &mut self.scratch_left[start..end];
            let right = &mut self.scratch_right[start..end];
            for i in 0..frames {
                let (l, r) = self.builtin_synth.render_sample();
                left[i] += l;
                right[i] += r;
            }
            self.sample_engine.render(&self.sample_bank, left, right);
        }

        // Apply master effects chain
        let left = &mut self.scratch_left[start..end];
        let right = &mut self.scratch_right[start..end];
        self.effects.process(left, right);
    }
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
    producer: Producer<TimedCommand>,
    has_sf2: bool,
    effects_enabled: Arc<AtomicBool>,
    sample_rate: f64,
    _stream: Stream,

    // Visualization data (read by UI thread)
    peak_l: Arc<AtomicU32>,
    peak_r: Arc<AtomicU32>,
    vis_consumer: Consumer<f32>,
    voice_snapshots: Arc<Mutex<Vec<VoiceSnapshot>>>,

    /// Frames consumed by the device so far. The sequencer schedules against
    /// this clock instead of wall time, so note timing does not depend on
    /// when the UI thread last ran.
    frame_clock: Arc<AtomicU64>,

    /// Values the audio thread has finished with, waiting to be dropped here.
    reclaim: Consumer<Reclaimed>,

    /// Human-readable summary of the output device, for the caller to show.
    device_description: String,

    /// Last error reported by the audio stream, if any.
    ///
    /// The cpal error callback runs on its own thread at arbitrary times, so
    /// it records here instead of printing; the UI picks it up on its next
    /// frame.
    stream_error: Arc<Mutex<Option<String>>>,
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

        // Describe the device for the caller to display. Printing it here
        // would land on the TUI's alternate screen.
        let device_description = format!(
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

        // Visualization: peak levels (f32 stored as u32 bits) + mono sample ring buffer
        let peak_l = Arc::new(AtomicU32::new(0));
        let peak_r = Arc::new(AtomicU32::new(0));
        let peak_l_cb = Arc::clone(&peak_l);
        let peak_r_cb = Arc::clone(&peak_r);
        let (vis_producer, vis_consumer) = RingBuffer::new(VIS_BUFFER_CAPACITY);
        let voice_snapshots: Arc<Mutex<Vec<VoiceSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
        let voice_snapshots_cb = Arc::clone(&voice_snapshots);

        let stream_config: cpal::StreamConfig = config.into();

        // Monotonic count of frames the device has consumed. The UI thread
        // reads this to schedule events in the audio timebase.
        let frame_clock = Arc::new(AtomicU64::new(0));

        // Values the audio thread is finished with, drained and dropped by
        // the UI thread.
        let (reclaim_producer, reclaim_consumer) = RingBuffer::new(RECLAIM_QUEUE_CAPACITY);

        let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let stream_error_cb = Arc::clone(&stream_error);

        let stream = {
            let mut state = RenderState {
                sf2_synth,
                builtin_synth,
                effects,
                channel_effects,
                send_buses,
                sample_engine,
                sample_bank,
                has_sf2,
                sample_rate: sr_f64,
                scratch_left: vec![0.0f32; MAX_CALLBACK_FRAMES],
                scratch_right: vec![0.0f32; MAX_CALLBACK_FRAMES],
                ch_buf_left: (0..MAX_EFFECT_CHANNELS)
                    .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                    .collect(),
                ch_buf_right: (0..MAX_EFFECT_CHANNELS)
                    .map(|_| vec![0.0f32; MAX_CALLBACK_FRAMES])
                    .collect(),
            };
            let mut consumer = consumer;
            let mut vis_producer = vis_producer;
            // Commands drained from the queue but not yet due. Pre-allocated
            // so the callback never has to grow it.
            let mut pending: Vec<TimedCommand> = Vec::with_capacity(COMMAND_QUEUE_CAPACITY);
            let mut reclaim = reclaim_producer;
            let frame_clock_cb = Arc::clone(&frame_clock);

            device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let frames = data.len() / channels;
                        // Frame index of the first sample in this buffer. The
                        // sequencer stamps commands in this timebase.
                        let block_start = frame_clock_cb.load(Ordering::Relaxed);

                        state.ensure_capacity(frames);

                        // Drain the queue into the pending list. Commands are
                        // kept until their frame falls inside a rendered
                        // segment, which is what makes note timing independent
                        // of when the UI thread happened to run.
                        while let Ok(cmd) = consumer.pop() {
                            if pending.len() == pending.capacity() {
                                // Cannot grow on the audio thread: apply the
                                // oldest immediately rather than dropping it.
                                let stale = pending.remove(0);
                                process_command(&mut state, stale.cmd, &effects_flag, &mut reclaim);
                            }
                            pending.push(cmd);
                        }

                        // Render the buffer in segments, applying each command
                        // at its own sample offset.
                        let mut pos = 0usize;
                        while pos < frames {
                            // Apply everything due at or before this offset.
                            let due_at = block_start + pos as u64;
                            let mut i = 0;
                            while i < pending.len() {
                                if pending[i].frame <= due_at {
                                    let cmd = pending.remove(i).cmd;
                                    process_command(&mut state, cmd, &effects_flag, &mut reclaim);
                                } else {
                                    i += 1;
                                }
                            }

                            // Render up to the next command boundary.
                            let next_boundary = pending
                                .iter()
                                .map(|c| c.frame)
                                .filter(|&f| f > due_at)
                                .min()
                                .map(|f| (f - block_start) as usize)
                                .unwrap_or(frames)
                                .clamp(pos + 1, frames);

                            state.render_segment(pos..next_boundary);
                            pos = next_boundary;
                        }

                        let left = &state.scratch_left[..frames];
                        let right = &state.scratch_right[..frames];

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

                        // Visualization: compute peak levels and push mono samples
                        {
                            let mut pl = 0.0f32;
                            let mut pr = 0.0f32;
                            for i in 0..frames {
                                let l = left[i].abs();
                                let r = right[i].abs();
                                if l > pl {
                                    pl = l;
                                }
                                if r > pr {
                                    pr = r;
                                }
                                // Push mono (L+R average) to ring buffer, drop if full
                                let mono = (left[i] + right[i]) * 0.5;
                                let _ = vis_producer.push(mono);
                            }
                            peak_l_cb.store(pl.to_bits(), Ordering::Relaxed);
                            peak_r_cb.store(pr.to_bits(), Ordering::Relaxed);
                        }

                        // Snapshot active sample voices for UI (non-blocking)
                        if let Ok(mut snaps) = voice_snapshots_cb.try_lock() {
                            snaps.clear();
                            for v in &state.sample_engine.voices {
                                if v.active {
                                    snaps.push(VoiceSnapshot {
                                        sample_index: v.sample_index,
                                        position: v.position,
                                        channel: v.channel,
                                        note: v.note,
                                        velocity: v.velocity,
                                        active: v.active,
                                    });
                                }
                            }
                        }

                        // Publish the frame position for the next callback.
                        frame_clock_cb.store(block_start + frames as u64, Ordering::Relaxed);
                    },
                    move |err| {
                        if let Ok(mut slot) = stream_error_cb.lock() {
                            *slot = Some(err.to_string());
                        }
                    },
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
            peak_l,
            peak_r,
            vis_consumer,
            voice_snapshots,
            frame_clock,
            reclaim: reclaim_consumer,
            device_description,
            stream_error,
        })
    }

    /// A one-line description of the output device this engine opened.
    pub fn device_description(&self) -> &str {
        &self.device_description
    }

    /// Take the most recent audio stream error, if one has occurred since
    /// this was last called.
    pub fn take_stream_error(&self) -> Option<String> {
        self.stream_error.lock().ok().and_then(|mut e| e.take())
    }

    /// Drop everything the audio thread has handed back.
    ///
    /// Cheap when there is nothing waiting, so it is safe to call from the
    /// UI loop every frame. Called automatically whenever a command is sent.
    pub fn reclaim_garbage(&mut self) {
        while let Ok(value) = self.reclaim.pop() {
            drop(value);
        }
    }

    /// Send a command to be applied as soon as the audio thread sees it.
    /// If the queue is full, the command is dropped.
    #[inline]
    fn send(&mut self, cmd: AudioCommand) {
        self.reclaim_garbage();
        let _ = self.producer.push(TimedCommand { frame: 0, cmd });
    }

    /// Send a command to be applied at a specific audio frame. Frames in the
    /// past are applied immediately; frames beyond the next callback wait in
    /// the audio thread's pending list until they come due.
    #[inline]
    fn send_at(&mut self, frame: u64, cmd: AudioCommand) {
        self.reclaim_garbage();
        let _ = self.producer.push(TimedCommand { frame, cmd });
    }

    /// Frames the output device has consumed so far. Returns 0 before the
    /// first callback runs.
    pub fn frame_clock(&self) -> u64 {
        self.frame_clock.load(Ordering::Relaxed)
    }

    /// Schedule a note-on at a specific audio frame.
    pub fn note_on_at(&mut self, frame: u64, channel: u8, note: u8, velocity: u8) {
        self.send_at(
            frame,
            AudioCommand::NoteOn {
                channel,
                note,
                velocity,
            },
        );
    }

    /// Schedule a note-on with explicit synth parameters at a specific frame.
    pub fn note_on_with_params_at(
        &mut self,
        frame: u64,
        channel: u8,
        note: u8,
        velocity: u8,
        params: &SynthParams,
    ) {
        self.send_at(
            frame,
            AudioCommand::NoteOnWithParams {
                channel,
                note,
                velocity,
                params: Box::new(params.clone()),
            },
        );
    }

    /// Schedule "all notes off for this channel" at a specific frame.
    pub fn note_off_all_channel_at(&mut self, frame: u64, channel: u8) {
        self.send_at(frame, AudioCommand::NoteOffAllChannel { channel });
    }

    /// Schedule a sample trigger at a specific frame.
    pub fn sample_note_on_at(
        &mut self,
        frame: u64,
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        action: NewNoteAction,
    ) {
        self.send_at(
            frame,
            AudioCommand::SampleNoteOn {
                sample_index,
                note,
                velocity,
                channel,
                action,
            },
        );
    }

    /// Schedule "stop this channel's sample voices" at a specific frame.
    pub fn sample_note_off_channel_at(&mut self, frame: u64, channel: u8) {
        self.send_at(frame, AudioCommand::SampleNoteOffChannel { channel });
    }

    pub fn has_sf2(&self) -> bool {
        self.has_sf2
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.send(AudioCommand::NoteOn {
            channel,
            note,
            velocity,
        });
    }

    pub fn note_on_with_params(
        &mut self,
        channel: u8,
        note: u8,
        velocity: u8,
        params: &SynthParams,
    ) {
        self.send(AudioCommand::NoteOnWithParams {
            channel,
            note,
            velocity,
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
        self.send(AudioCommand::SendCC {
            channel,
            controller,
            value,
        });
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
    pub fn sample_note_on(
        &mut self,
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        action: NewNoteAction,
    ) {
        self.send(AudioCommand::SampleNoteOn {
            sample_index,
            note,
            velocity,
            channel,
            action,
        });
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

    /// Read peak levels (L, R) from the last audio callback. Values are 0.0..~1.0.
    pub fn peak_levels(&self) -> (f32, f32) {
        let l = f32::from_bits(self.peak_l.load(Ordering::Relaxed));
        let r = f32::from_bits(self.peak_r.load(Ordering::Relaxed));
        (l, r)
    }

    /// Get snapshots of currently active sample voices.
    pub fn voice_snapshots(&self) -> Vec<VoiceSnapshot> {
        self.voice_snapshots
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Drain available visualization samples (mono) into the provided buffer.
    /// Returns the number of samples read.
    pub fn read_vis_samples(&mut self, buf: &mut Vec<f32>) -> usize {
        let available = self.vis_consumer.slots();
        let chunk = self.vis_consumer.read_chunk(available);
        match chunk {
            Ok(chunk) => {
                let n = chunk.len();
                buf.extend_from_slice(chunk.as_slices().0);
                buf.extend_from_slice(chunk.as_slices().1);
                chunk.commit_all();
                n
            }
            Err(_) => 0,
        }
    }
}

/// Process a single command on the audio thread. Called from inside the audio callback.
#[allow(clippy::too_many_arguments)]
fn process_command(
    state: &mut RenderState,
    cmd: AudioCommand,
    effects_flag: &AtomicBool,
    reclaim: &mut Producer<Reclaimed>,
) {
    // If the reclaim queue is full the value is dropped here instead. That
    // costs a free on the audio thread, but only when the UI thread has not
    // drained for 64 commands, and it is preferable to leaking.
    let mut hand_back = |value: Reclaimed| {
        let _ = reclaim.push(value);
    };

    let RenderState {
        sf2_synth,
        builtin_synth,
        effects,
        channel_effects,
        send_buses,
        sample_engine,
        sample_bank,
        has_sf2,
        sample_rate,
        ..
    } = state;
    let has_sf2 = *has_sf2;
    let sample_rate = *sample_rate;

    match cmd {
        AudioCommand::NoteOn {
            channel,
            note,
            velocity,
        } => {
            if let Some(ref mut sf2) = sf2_synth {
                sf2.note_off_all_channel(channel as i32, false);
                sf2.note_on(channel as i32, note as i32, velocity as i32);
            }
            if !has_sf2 {
                builtin_synth.note_on(channel, note, velocity);
            }
        }
        AudioCommand::NoteOnWithParams {
            channel,
            note,
            velocity,
            params,
        } => {
            builtin_synth.note_on_with_params(channel, note, velocity, &params);
            hand_back(Reclaimed::SynthParams(params));
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
        AudioCommand::SendCC {
            channel,
            controller,
            value,
        } => {
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
            // The replaced bank may hold the last reference to every loaded
            // sample, so it must not be dropped here.
            let previous = std::mem::replace(sample_bank, bank);
            hand_back(Reclaimed::Bank(previous));
        }
        AudioCommand::SampleNoteOn {
            sample_index,
            note,
            velocity,
            channel,
            action,
        } => {
            // Same path as the offline renderer takes, rather than a copy of
            // it: voice allocation, de-clicking and loop handling only stay
            // in step between live playback and export if there is one
            // implementation of them.
            if let Some(sample) = sample_bank.get(sample_index) {
                sample_engine.note_on(
                    sample_index,
                    note,
                    velocity,
                    channel,
                    sample,
                    sample_rate,
                    action,
                );
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
            let mut params = params;
            if ch < channel_effects.len() {
                // Swap rather than assign: assigning would drop the previous
                // params here.
                std::mem::swap(&mut channel_effects[ch].params, &mut params);
            }
            hand_back(Reclaimed::ChannelEffects(params));
        }
        AudioCommand::SetSendBusParams { bus, params } => {
            let idx = bus as usize;
            let mut params = params;
            if idx < send_buses.len() {
                // `SendBusParams` owns a `String` label, so the displaced
                // value has a heap allocation to free.
                std::mem::swap(&mut send_buses[idx].params, &mut params);
            }
            hand_back(Reclaimed::SendBus(params));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::effects::EffectsChain;
    use crate::audio::synth::BuiltinSynth;

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
                    eprintln!(
                        "  Supported: {:?} {}ch {:?}",
                        c.sample_format(),
                        c.channels(),
                        c.buffer_size()
                    );
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

        eprintln!(
            "Max sample jump: {:.6} at sample {} (buffer boundary at {})",
            max_jump,
            max_jump_pos,
            if max_jump_pos % chunk == 0 {
                "YES"
            } else {
                "no"
            }
        );

        assert!(
            max_jump < max_allowed_jump,
            "Click detected: jump={:.6} at sample {} (limit {:.6})",
            max_jump,
            max_jump_pos,
            max_allowed_jump
        );
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
            assert!(
                (got_l - exp_l).abs() < 1e-7,
                "L mismatch at frame {}: got={}, exp={}",
                i,
                got_l,
                exp_l
            );
            assert!(
                (got_r - exp_r).abs() < 1e-7,
                "R mismatch at frame {}: got={}, exp={}",
                i,
                got_r,
                exp_r
            );
        }

        let peak = data.iter().fold(0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.05, "Signal too quiet: peak={:.4}", peak);
        assert!(peak < 1.0, "Signal clips: peak={:.4}", peak);

        // Sine patch: L and R should be identical
        let mut diff_sum = 0f32;
        for i in 0..frames {
            diff_sum += (data[i * 2] - data[i * 2 + 1]).abs();
        }
        let avg_diff = diff_sum / frames as f32;
        eprintln!(
            "Interleave test: peak={:.4}, avg L-R diff={:.6}",
            peak, avg_diff
        );
    }
}

#[cfg(test)]
mod scheduler_tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn flag() -> AtomicBool {
        AtomicBool::new(true)
    }

    /// A reclaim queue whose consumer end is dropped: values pushed to it go
    /// nowhere, which is all these tests need.
    fn reclaim_sink() -> Producer<Reclaimed> {
        let (producer, _consumer) = RingBuffer::new(RECLAIM_QUEUE_CAPACITY);
        producer
    }

    /// Apply a command to a render state, as the audio callback would.
    fn apply(state: &mut RenderState, cmd: AudioCommand) {
        process_command(state, cmd, &flag(), &mut reclaim_sink());
    }

    /// Render `frames` in one call.
    fn render_whole(state: &mut RenderState, frames: usize) -> Vec<f32> {
        state.render_segment(0..frames);
        state.scratch_left[..frames].to_vec()
    }

    /// Render `frames` split at the given boundaries.
    fn render_split(state: &mut RenderState, frames: usize, splits: &[usize]) -> Vec<f32> {
        let mut pos = 0;
        for &b in splits {
            state.render_segment(pos..b);
            pos = b;
        }
        state.render_segment(pos..frames);
        state.scratch_left[..frames].to_vec()
    }

    /// The scheduler splits every callback at each command's sample offset.
    /// If segmented rendering did not match whole-buffer rendering, note
    /// timing would be bought at the cost of audible seams.
    #[test]
    fn segmented_rendering_matches_whole_buffer_rendering() {
        let frames = 512;

        let mut a = RenderState::for_test(SR);
        apply(
            &mut a,
            AudioCommand::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        );
        let whole = render_whole(&mut a, frames);

        let mut b = RenderState::for_test(SR);
        apply(
            &mut b,
            AudioCommand::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        );
        let split = render_split(&mut b, frames, &[1, 17, 200, 201, 511]);

        assert_eq!(whole.len(), split.len());
        for (i, (w, s)) in whole.iter().zip(split.iter()).enumerate() {
            assert_eq!(w, s, "sample {i} differs between whole and split render");
        }
        assert!(
            whole.iter().any(|s| s.abs() > 1e-6),
            "test rendered silence, so it would pass trivially"
        );
    }

    #[test]
    fn segmented_rendering_matches_with_channel_effects_engaged() {
        // The per-channel path runs channel effects and send buses, which
        // carry state across segment boundaries.
        let frames = 512;
        let params = ChannelEffectsParams {
            filter_enabled: true,
            delay_enabled: true,
            ..ChannelEffectsParams::default()
        };

        let setup = |state: &mut RenderState| {
            apply(
                state,
                AudioCommand::SetChannelEffects {
                    channel: 0,
                    params: Box::new(params.clone()),
                },
            );
            apply(
                state,
                AudioCommand::NoteOn {
                    channel: 0,
                    note: 55,
                    velocity: 127,
                },
            );
        };

        let mut a = RenderState::for_test(SR);
        setup(&mut a);
        let whole = render_whole(&mut a, frames);

        let mut b = RenderState::for_test(SR);
        setup(&mut b);
        let split = render_split(&mut b, frames, &[64, 65, 300]);

        for (i, (w, s)) in whole.iter().zip(split.iter()).enumerate() {
            assert_eq!(w, s, "sample {i} differs with channel effects engaged");
        }
        assert!(whole.iter().any(|s| s.abs() > 1e-6), "rendered silence");
    }

    #[test]
    fn segmented_rendering_matches_with_send_buses_engaged() {
        // Send buses accumulate per-segment input and carry delay/reverb
        // state across segments; verify that splitting is still transparent.
        let frames = 512;
        let mut send_levels = [0.0f32; effects::MAX_SEND_BUSES];
        send_levels[0] = 0.7;
        let ch_params = ChannelEffectsParams {
            send_levels,
            ..ChannelEffectsParams::default()
        };
        let bus_params = effects::SendBusParams {
            enabled: true,
            effect_type: effects::SendBusType::Delay,
            delay_time: 3.0,
            delay_feedback: 0.5,
            ..effects::SendBusParams::default()
        };

        let setup = |state: &mut RenderState| {
            apply(
                state,
                AudioCommand::SetSendBusParams {
                    bus: 0,
                    params: Box::new(bus_params.clone()),
                },
            );
            apply(
                state,
                AudioCommand::SetChannelEffects {
                    channel: 0,
                    params: Box::new(ch_params.clone()),
                },
            );
            apply(
                state,
                AudioCommand::NoteOn {
                    channel: 0,
                    note: 48,
                    velocity: 127,
                },
            );
        };

        let mut a = RenderState::for_test(SR);
        setup(&mut a);
        let whole = render_whole(&mut a, frames);

        let mut b = RenderState::for_test(SR);
        setup(&mut b);
        let split = render_split(&mut b, frames, &[48, 96, 400]);

        for (i, (w, s)) in whole.iter().zip(split.iter()).enumerate() {
            assert_eq!(w, s, "sample {i} differs with send buses engaged");
        }
        assert!(whole.iter().any(|s| s.abs() > 1e-6), "rendered silence");
    }

    /// A note scheduled part-way through a buffer must be silent before its
    /// frame and sounding from it. This is the property the whole lookahead
    /// design exists to provide.
    #[test]
    fn a_note_scheduled_mid_buffer_starts_at_that_sample() {
        let frames = 512;
        let onset = 300usize;

        let mut state = RenderState::for_test(SR);
        // Segment 1: nothing scheduled yet.
        state.render_segment(0..onset);
        // Command applied exactly at the onset frame.
        apply(
            &mut state,
            AudioCommand::NoteOn {
                channel: 0,
                note: 69,
                velocity: 127,
            },
        );
        state.render_segment(onset..frames);

        let out = &state.scratch_left[..frames];
        assert!(
            out[..onset].iter().all(|s| s.abs() < 1e-9),
            "output before the scheduled frame must be silent"
        );
        assert!(
            out[onset..].iter().any(|s| s.abs() > 1e-6),
            "output after the scheduled frame must be sounding"
        );
    }

    /// Whole-buffer application (the old behaviour) quantises every note to
    /// a buffer boundary. This documents the difference the refactor makes.
    #[test]
    fn without_scheduling_a_note_would_start_at_the_buffer_boundary() {
        let frames = 512;
        let mut state = RenderState::for_test(SR);
        apply(
            &mut state,
            AudioCommand::NoteOn {
                channel: 0,
                note: 69,
                velocity: 127,
            },
        );
        state.render_segment(0..frames);
        assert!(
            state.scratch_left[..8].iter().any(|s| s.abs() > 1e-9),
            "applied up front, the note sounds from sample 0"
        );
    }

    /// Everything the audio thread finishes with must leave via the reclaim
    /// queue rather than being freed on the audio thread.
    #[test]
    fn finished_values_are_handed_back_instead_of_freed() {
        let (mut producer, mut consumer) = RingBuffer::new(RECLAIM_QUEUE_CAPACITY);
        let mut state = RenderState::for_test(SR);

        process_command(
            &mut state,
            AudioCommand::SetSampleBank {
                bank: Arc::new(SampleBank::new()),
            },
            &flag(),
            &mut producer,
        );
        process_command(
            &mut state,
            AudioCommand::NoteOnWithParams {
                channel: 0,
                note: 60,
                velocity: 100,
                params: Box::new(SynthParams::from_patch(0)),
            },
            &flag(),
            &mut producer,
        );
        process_command(
            &mut state,
            AudioCommand::SetChannelEffects {
                channel: 0,
                params: Box::new(ChannelEffectsParams::default()),
            },
            &flag(),
            &mut producer,
        );
        process_command(
            &mut state,
            AudioCommand::SetSendBusParams {
                bus: 0,
                params: Box::new(effects::SendBusParams::default()),
            },
            &flag(),
            &mut producer,
        );

        let mut handed_back = 0;
        while consumer.pop().is_ok() {
            handed_back += 1;
        }
        assert_eq!(
            handed_back, 4,
            "every owning command should hand its value back"
        );
    }

    /// An out-of-range target must still hand the value back rather than
    /// dropping it on the audio thread.
    #[test]
    fn values_for_out_of_range_targets_are_still_handed_back() {
        let (mut producer, mut consumer) = RingBuffer::new(RECLAIM_QUEUE_CAPACITY);
        let mut state = RenderState::for_test(SR);
        process_command(
            &mut state,
            AudioCommand::SetChannelEffects {
                channel: 200,
                params: Box::new(ChannelEffectsParams::default()),
            },
            &flag(),
            &mut producer,
        );
        process_command(
            &mut state,
            AudioCommand::SetSendBusParams {
                bus: 200,
                params: Box::new(effects::SendBusParams::default()),
            },
            &flag(),
            &mut producer,
        );
        assert!(consumer.pop().is_ok());
        assert!(consumer.pop().is_ok());
    }

    /// The swap must actually install the new parameters, not just move the
    /// old ones out of the way.
    #[test]
    fn handing_back_still_installs_the_new_parameters() {
        let (mut producer, _consumer) = RingBuffer::new(RECLAIM_QUEUE_CAPACITY);
        let mut state = RenderState::for_test(SR);
        let params = ChannelEffectsParams {
            filter_enabled: true,
            filter_cutoff: 1234.0,
            ..ChannelEffectsParams::default()
        };
        process_command(
            &mut state,
            AudioCommand::SetChannelEffects {
                channel: 2,
                params: Box::new(params),
            },
            &flag(),
            &mut producer,
        );
        assert!(state.channel_effects[2].params.filter_enabled);
        assert_eq!(state.channel_effects[2].params.filter_cutoff, 1234.0);

        let bus = effects::SendBusParams {
            enabled: true,
            label: "verb".to_string(),
            ..effects::SendBusParams::default()
        };
        process_command(
            &mut state,
            AudioCommand::SetSendBusParams {
                bus: 1,
                params: Box::new(bus),
            },
            &flag(),
            &mut producer,
        );
        assert!(state.send_buses[1].params.enabled);
        assert_eq!(state.send_buses[1].params.label, "verb");
    }

    #[test]
    fn an_empty_segment_renders_nothing_and_does_not_panic() {
        let mut state = RenderState::for_test(SR);
        state.render_segment(0..0);
        state.render_segment(10..10);
    }

    #[test]
    fn ensure_capacity_grows_every_scratch_buffer() {
        let mut state = RenderState::for_test(SR);
        let big = MAX_CALLBACK_FRAMES + 128;
        state.ensure_capacity(big);
        assert!(state.scratch_left.len() >= big);
        assert!(state.scratch_right.len() >= big);
        assert!(state.ch_buf_left.iter().all(|b| b.len() >= big));
        assert!(state.ch_buf_right.iter().all(|b| b.len() >= big));
        // Rendering the full width must not panic.
        state.render_segment(0..big);
    }
}
