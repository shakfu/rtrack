//! Shared types used by TrackerCore and both frontends.

use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::channel_effects::ChannelEffectsParams;

/// Auto-save interval in seconds
pub const AUTOSAVE_INTERVAL_SECS: u64 = 60;

/// Timing accumulators for the playback loop.
///
/// Tracks elapsed time, sub-tick fractions, MIDI clock counters, and
/// Ableton Link beat position to drive deterministic tick-based playback.
pub struct PlaybackTiming {
    /// Timestamp of the last tick (None when stopped).
    pub last_tick: Option<Instant>,
    /// Fractional tick accumulator (seconds).
    pub tick_accumulator: f64,
    /// Fractional accumulator for outgoing MIDI clock messages (seconds).
    pub clock_tick_accumulator: f64,
    /// Total elapsed playback time (seconds).
    pub playback_elapsed: f64,
    /// Counter for incoming external MIDI clock ticks.
    pub ext_clock_count: u32,
    /// Last polled Ableton Link beat position.
    pub last_link_beat: f64,

    /// Audio frame at which the next tick should sound. Only used when the
    /// sequencer is running off the audio clock.
    pub next_tick_frame: u64,
    /// Audio frame observed at the previous `tick_playback` call, used to
    /// derive elapsed time without consulting the wall clock.
    pub last_clock_frame: u64,
    /// Row positions already scheduled but not yet audible, oldest first.
    /// Lets the UI show the row the listener is actually hearing rather than
    /// the row the sequencer has run ahead to.
    pub scheduled_positions: std::collections::VecDeque<ScheduledPosition>,
}

/// A song position paired with the audio frame at which it becomes audible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledPosition {
    pub frame: u64,
    pub order: usize,
    pub row: usize,
}

impl PlaybackTiming {
    /// Create a new timing state with all accumulators zeroed.
    pub fn new() -> Self {
        Self {
            last_tick: None,
            tick_accumulator: 0.0,
            clock_tick_accumulator: 0.0,
            playback_elapsed: 0.0,
            ext_clock_count: 0,
            last_link_beat: 0.0,
            next_tick_frame: 0,
            last_clock_frame: 0,
            scheduled_positions: std::collections::VecDeque::new(),
        }
    }
}

impl Default for PlaybackTiming {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackTiming {
    /// Reset all timing accumulators to zero (called on playback start).
    pub fn reset(&mut self) {
        self.last_tick = None;
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;
        self.playback_elapsed = 0.0;
        self.ext_clock_count = 0;
        self.last_link_beat = 0.0;
        self.next_tick_frame = 0;
        self.last_clock_frame = 0;
        self.scheduled_positions.clear();
    }
}

/// Re-export ChannelState from the engine module.
pub use crate::engine::ChannelState;

/// The sound source type for a tracker channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    /// Route notes to an external MIDI device.
    Midi,
    /// Use the built-in synthesizer.
    Synth,
    /// Play loaded audio samples.
    Sample,
}

impl ChannelType {
    /// Short display label for the channel type header (e.g. "[MID]").
    pub fn label(self) -> &'static str {
        match self {
            Self::Midi => "[MID]",
            Self::Synth => "[SYN]",
            Self::Sample => "[SMP]",
        }
    }

    /// Cycle to the next channel type (Midi -> Synth -> Sample -> Midi).
    pub fn next(self) -> Self {
        match self {
            Self::Midi => Self::Synth,
            Self::Synth => Self::Sample,
            Self::Sample => Self::Midi,
        }
    }

    /// Cycle to the previous channel type.
    pub fn prev(self) -> Self {
        match self {
            Self::Midi => Self::Sample,
            Self::Synth => Self::Midi,
            Self::Sample => Self::Synth,
        }
    }
}

/// A channel effects parameter that can be targeted by MIDI learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnableParam {
    /// Low-pass filter cutoff frequency (20-20000 Hz, exponential).
    FilterCutoff,
    /// Filter resonance (0.0-1.0).
    FilterResonance,
    /// Distortion drive amount (1.0-20.0).
    DistortionDrive,
    /// Chorus LFO rate in Hz (0.1-10.0).
    ChorusRate,
    /// Chorus depth in samples (0.5-20.0).
    ChorusDepth,
    /// Chorus wet/dry mix (0.0-1.0).
    ChorusMix,
    /// Delay time in milliseconds (1-2000).
    DelayTime,
    /// Delay feedback amount (0.0-0.95).
    DelayFeedback,
    /// Delay wet/dry mix (0.0-1.0).
    DelayMix,
    /// Reverb room size (0.0-1.0).
    ReverbSize,
    /// Reverb high-frequency damping (0.0-1.0).
    ReverbDamp,
    /// Reverb wet/dry mix (0.0-1.0).
    ReverbMix,
}

impl LearnableParam {
    /// Map a MIDI CC value (0-127) to the parameter's native range.
    pub fn map_cc(self, value: u8) -> f32 {
        let t = value as f32 / 127.0;
        match self {
            Self::FilterCutoff => 20.0 * (1000.0_f32).powf(t), // 20..20000 Hz (exp)
            Self::FilterResonance => t,                        // 0..1
            Self::DistortionDrive => 1.0 + t * 19.0,           // 1..20
            Self::ChorusRate => 0.1 + t * 9.9,                 // 0.1..10
            Self::ChorusDepth => 0.5 + t * 19.5,               // 0.5..20
            Self::ChorusMix => t,                              // 0..1
            Self::DelayTime => 1.0 + t * 1999.0,               // 1..2000 ms
            Self::DelayFeedback => t * 0.95,                   // 0..0.95
            Self::DelayMix => t,                               // 0..1
            Self::ReverbSize => t,                             // 0..1
            Self::ReverbDamp => t,                             // 0..1
            Self::ReverbMix => t,                              // 0..1
        }
    }

    /// Apply a CC value to the given effects params.
    pub fn apply(self, params: &mut ChannelEffectsParams, value: u8) {
        let v = self.map_cc(value);
        match self {
            Self::FilterCutoff => params.filter_cutoff = v,
            Self::FilterResonance => params.filter_resonance = v,
            Self::DistortionDrive => params.distortion_drive = v,
            Self::ChorusRate => params.chorus_rate = v,
            Self::ChorusDepth => params.chorus_depth = v,
            Self::ChorusMix => params.chorus_mix = v,
            Self::DelayTime => params.delay_time = v,
            Self::DelayFeedback => params.delay_feedback = v,
            Self::DelayMix => params.delay_mix = v,
            Self::ReverbSize => params.reverb_size = v,
            Self::ReverbDamp => params.reverb_damp = v,
            Self::ReverbMix => params.reverb_mix = v,
        }
    }

    /// Human-readable display name for this parameter.
    pub fn name(self) -> &'static str {
        match self {
            Self::FilterCutoff => "Filter Cutoff",
            Self::FilterResonance => "Filter Resonance",
            Self::DistortionDrive => "Distortion Drive",
            Self::ChorusRate => "Chorus Rate",
            Self::ChorusDepth => "Chorus Depth",
            Self::ChorusMix => "Chorus Mix",
            Self::DelayTime => "Delay Time",
            Self::DelayFeedback => "Delay Feedback",
            Self::DelayMix => "Delay Mix",
            Self::ReverbSize => "Reverb Size",
            Self::ReverbDamp => "Reverb Damp",
            Self::ReverbMix => "Reverb Mix",
        }
    }

    /// Try to convert a track config field index (relative to fx_off) to a learnable param.
    pub fn from_fx_field(offset: usize) -> Option<Self> {
        match offset {
            1 => Some(Self::FilterCutoff),
            2 => Some(Self::FilterResonance),
            4 => Some(Self::DistortionDrive),
            6 => Some(Self::ChorusRate),
            7 => Some(Self::ChorusDepth),
            8 => Some(Self::ChorusMix),
            10 => Some(Self::DelayTime),
            11 => Some(Self::DelayFeedback),
            12 => Some(Self::DelayMix),
            14 => Some(Self::ReverbSize),
            15 => Some(Self::ReverbDamp),
            16 => Some(Self::ReverbMix),
            _ => None,
        }
    }
}

/// A single MIDI CC -> parameter mapping, created via MIDI learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiCcMapping {
    /// MIDI CC number (0-127).
    pub cc: u8,
    /// Tracker channel index this mapping targets.
    pub channel: usize,
    /// The effects parameter controlled by this CC.
    pub param: LearnableParam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockMode {
    /// rtrack is the clock master (internal timing)
    Internal,
    /// rtrack slaves to external MIDI clock
    ExternalMidi,
}

/// Per-channel configuration (audio routing, effects, naming).
///
/// Serialized into the .rtrk file so that mixer state survives a save/load
/// cycle. Every field carries a serde default so that files written by
/// earlier versions, which stored no channel data at all, still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_channel_type")]
    pub channel_type: ChannelType,
    /// Default instrument for this track (Synth tracks auto-fill on note entry)
    #[serde(default)]
    pub default_instrument: Option<u8>,
    #[serde(default = "default_channel_volume")]
    pub volume: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub effects_params: ChannelEffectsParams,
    /// MIDI channel this tracker channel maps to (0-15)
    #[serde(default)]
    pub midi_channel: u8,
}

fn default_channel_type() -> ChannelType {
    ChannelType::Midi
}

fn default_channel_volume() -> f32 {
    1.0
}

impl ChannelConfig {
    /// Create a new channel config with defaults for the given MIDI channel.
    pub fn new(midi_channel: u8) -> Self {
        Self {
            muted: false,
            name: String::new(),
            channel_type: ChannelType::Midi,
            default_instrument: None,
            volume: 1.0,
            pan: 0.0,
            effects_params: ChannelEffectsParams::default(),
            midi_channel,
        }
    }
}

/// An instrument definition binding a name to a sound source.
///
/// Each instrument can route to one of: a loaded sample (via `sample_index`),
/// custom synth parameters, a MIDI program number, or the default built-in synth.
#[derive(Default, Clone)]
pub struct Instrument {
    /// Display name shown in the instrument list.
    pub name: String,
    /// MIDI program change number (0-127).
    pub midi_program: Option<u8>,
    /// Index into the sample bank (if this instrument plays a sample).
    pub sample_index: Option<usize>,
    /// Custom synthesizer parameters (overrides preset patches).
    pub synth_params: Option<crate::audio::synth::SynthParams>,
    /// Pitch bend range in semitones (None = use default of 2).
    pub pitch_bend_range: Option<f64>,
}

/// Compute the auto-save path for a given song file path.
pub fn autosave_path_for(path: &std::path::Path) -> PathBuf {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let name = path.file_name().and_then(|f| f.to_str()).unwrap_or("song");
    dir.join(format!(".{}.autosave", name))
}

/// Make a path relative to a base directory. Falls back to absolute if no common prefix.
pub fn make_relative(base: &std::path::Path, target: &std::path::Path) -> String {
    let base_abs = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let target_abs = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());

    if let Ok(rel) = target_abs.strip_prefix(&base_abs) {
        return rel.to_string_lossy().to_string();
    }

    target.to_string_lossy().to_string()
}

/// Resolve a (possibly relative) path against a base directory.
/// Rejects path traversal (`..` components) and absolute paths to prevent
/// a malicious .rtrk file from referencing files outside the song directory.
pub fn resolve_relative(base: &std::path::Path, rel: &str) -> PathBuf {
    let p = std::path::Path::new(rel);
    if p.is_absolute() {
        return base.join(p.file_name().unwrap_or_default());
    }
    let sanitized: PathBuf = p
        .components()
        .filter(|c| !matches!(c, std::path::Component::ParentDir))
        .collect();
    base.join(sanitized)
}

/// Create default channel configs for N channels.
pub fn default_channel_configs(n: usize) -> Vec<ChannelConfig> {
    (0..n).map(|i| ChannelConfig::new(i as u8)).collect()
}
