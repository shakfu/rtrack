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
    /// The track type versions 0 and 1 stored, read only to apply
    /// [`ChannelConfig::drop_legacy_type`].
    #[serde(default, rename = "channel_type", skip_serializing)]
    legacy_type: Option<String>,
    /// Instrument filled in for a note entered or played without one.
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

fn default_channel_volume() -> f32 {
    1.0
}

impl ChannelConfig {
    /// Create a new channel config with defaults for the given MIDI channel.
    pub fn new(midi_channel: u8) -> Self {
        Self {
            muted: false,
            name: String::new(),
            legacy_type: None,
            default_instrument: None,
            volume: 1.0,
            pan: 0.0,
            effects_params: ChannelEffectsParams::default(),
            midi_channel,
        }
    }

    /// Apply the old track type, then forget it. Only `Synth` and `Sample`
    /// tracks filled in their default instrument, so a `Midi` track's is
    /// dropped to keep the song sounding as it did.
    pub fn drop_legacy_type(&mut self) {
        if self.legacy_type.take().as_deref() == Some("Midi") {
            self.default_instrument = None;
        }
    }
}

/// An instrument: a name bound to one sound source.
#[derive(Debug, Clone, PartialEq)]
pub struct Instrument {
    /// Display name shown in the instrument list.
    pub name: String,
    pub kind: InstrumentKind,
    /// Pitch bend range in semitones (None = use default of 2).
    pub pitch_bend_range: Option<f64>,
}

impl Instrument {
    /// An unset instrument in `slot`.
    pub fn for_slot(slot: usize) -> Self {
        Self {
            name: String::new(),
            kind: InstrumentKind::default_for(slot),
            pitch_bend_range: None,
        }
    }

    /// True if nothing about the instrument in `slot` has been set.
    pub fn is_default(&self, slot: usize) -> bool {
        *self == Self::for_slot(slot)
    }

    /// The sample bank slot this instrument plays, if it is a sample.
    pub fn sample_index(&self) -> Option<usize> {
        match self.kind {
            InstrumentKind::Sample { slot, .. } => Some(slot),
            _ => None,
        }
    }

    /// Make this instrument play bank `slot`. A sample instrument keeps its
    /// drive and filter, so replacing its sample keeps its sound.
    pub fn play_sample(&mut self, slot: usize) {
        let fx = match self.kind {
            InstrumentKind::Sample { fx, .. } => fx,
            _ => SampleFx::default(),
        };
        self.kind = InstrumentKind::Sample { slot, fx };
    }

    pub fn synth_params(&self) -> Option<&crate::audio::synth::SynthParams> {
        match &self.kind {
            InstrumentKind::Synth(params) => Some(params),
            _ => None,
        }
    }

    pub fn synth_params_mut(&mut self) -> Option<&mut crate::audio::synth::SynthParams> {
        match &mut self.kind {
            InstrumentKind::Synth(params) => Some(params),
            _ => None,
        }
    }

    /// The program of a MIDI instrument.
    pub fn program(&self) -> Option<u8> {
        match self.kind {
            InstrumentKind::Midi { program, .. } => Some(program),
            _ => None,
        }
    }
}

/// What an instrument plays: the G4 instrument format
/// (`docs/dev/g4-g8-format.md`). Each note plays in exactly one place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InstrumentKind {
    /// A MIDI voice, played by the built-in synth unless routed out.
    Midi {
        /// For the built-in synth, any value selects a preset. Sent to a
        /// port as `program & 0x7F`.
        program: u8,
        /// `None` uses the track's `ChannelConfig.midi_channel`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<u8>,
        #[serde(default, skip_serializing_if = "MidiOutput::is_builtin")]
        output: MidiOutput,
    },
    Synth(crate::audio::synth::SynthParams),
    Sample {
        slot: usize,
        #[serde(default, skip_serializing_if = "SampleFx::is_off")]
        fx: SampleFx,
    },
}

impl InstrumentKind {
    /// A sample instrument playing bank `slot`, with no filter or drive.
    pub fn sample(slot: usize) -> Self {
        Self::Sample {
            slot,
            fx: SampleFx::default(),
        }
    }
}

/// Drive and filter applied to each voice of a sample instrument, before
/// its envelope (`docs/dev/g5-instrument-fx.md`). Off by default.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleFx {
    /// `tanh` drive gain; 1.0 is off.
    #[serde(default = "SampleFx::no_drive")]
    pub drive: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<VoiceFilter>,
}

impl Default for SampleFx {
    fn default() -> Self {
        Self {
            drive: Self::no_drive(),
            filter: None,
        }
    }
}

impl SampleFx {
    /// Highest drive, as for a track's distortion.
    pub const MAX_DRIVE: f32 = 20.0;

    fn no_drive() -> f32 {
        1.0
    }

    pub fn is_off(&self) -> bool {
        *self == Self::default()
    }
}

/// A sample instrument's filter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VoiceFilter {
    #[serde(default)]
    pub kind: crate::audio::synth::FilterType,
    /// 20..20000 Hz.
    pub cutoff_hz: f32,
    /// 0..1.
    #[serde(default)]
    pub resonance: f32,
}

impl Default for VoiceFilter {
    fn default() -> Self {
        Self {
            kind: crate::audio::synth::FilterType::LowPass,
            cutoff_hz: 8000.0,
            resonance: 0.0,
        }
    }
}

/// Where a MIDI instrument's notes go.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "lowercase")]
pub enum MidiOutput {
    /// SF2 if one is loaded, else the built-in preset synth.
    #[default]
    Builtin,
    /// The MIDI port the user selected. Plays as `Builtin` when none is
    /// selected or the selected one is absent. `port` names a specific port
    /// instead of the session's; unused until per-instrument ports exist.
    External {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
}

impl MidiOutput {
    pub fn is_builtin(&self) -> bool {
        *self == Self::Builtin
    }
}

impl InstrumentKind {
    /// The kind of an unset slot: built-in preset `slot`, as an instrument
    /// number with nothing assigned played before typed instruments.
    pub fn default_for(slot: usize) -> Self {
        Self::Midi {
            program: slot as u8,
            channel: None,
            output: MidiOutput::Builtin,
        }
    }

    /// Convert an instrument from the untyped format (versions 0 and 1).
    ///
    /// Follows the old routing order: sample, then synth parameters, then a
    /// preset by slot number. Returns a repair note for each setting the old
    /// routing never used.
    pub fn from_legacy(
        slot: usize,
        sample_index: Option<usize>,
        synth_params: Option<crate::audio::synth::SynthParams>,
        midi_program: Option<u8>,
    ) -> (Self, Vec<String>) {
        let mut repairs = Vec::new();
        let kind = match (sample_index, synth_params) {
            (Some(sample), params) => {
                if params.is_some() {
                    repairs.push(format!(
                        "instrument {slot:02X}: had a sample and synth parameters; kept the sample"
                    ));
                }
                Self::sample(sample)
            }
            (None, Some(params)) => Self::Synth(params),
            (None, None) => {
                let program = midi_program.unwrap_or(slot as u8);
                if program != slot as u8 {
                    repairs.push(format!(
                        "instrument {slot:02X}: now plays its MIDI program {program}, \
                         not preset {slot}"
                    ));
                }
                Self::Midi {
                    program,
                    channel: None,
                    output: MidiOutput::Builtin,
                }
            }
        };
        if midi_program.is_some() && !matches!(kind, Self::Midi { .. }) {
            repairs.push(format!(
                "instrument {slot:02X}: dropped MIDI program, which was never sent"
            ));
        }
        (kind, repairs)
    }
}

/// Compute the auto-save path for a given song file path.
pub fn autosave_path_for(path: &std::path::Path) -> PathBuf {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let name = path.file_name().and_then(|f| f.to_str()).unwrap_or("song");
    dir.join(format!(".{}.autosave", name))
}

/// Where to auto-save a song that has never been saved.
///
/// Beside the song is right once there *is* a song file, and wrong before
/// there is one: the fallback name has no directory, so it resolved against
/// the working directory and dropped a hidden `.untitled.rtrk.autosave` into
/// wherever rtrack happened to be launched from. Run it from `$HOME` and that
/// is where the file appeared -- which is neither what the README described
/// ("a temp file") nor something a user would think to look for or clean up.
///
/// The title is folded into the name so that two unsaved sessions do not
/// overwrite each other's recovery file.
pub fn autosave_path_for_unsaved(title: &str) -> PathBuf {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let slug = slug.trim_matches('_').to_string();
    let slug = if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    };
    std::env::temp_dir().join(format!("rtrack-{}.rtrk.autosave", slug))
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

/// Resolve a sample path recorded in a `.rtrk` file against the song directory.
///
/// Two shapes reach this function, matching what [`make_relative`] writes:
///
/// * A relative path, for a sample that lives under the song directory. It is
///   resolved against `base` and must stay there: a `..` component, a root, or
///   a Windows prefix is rejected rather than silently stripped. Stripping was
///   the old behaviour and it turned `../../etc/passwd` into a *valid* path
///   under `base` -- a file that may well exist and is not the one named.
/// * An absolute path, for a sample from a library outside the song
///   directory. It is returned unchanged. Rewriting it as `base/<file_name>`
///   (the old behaviour) lost the directory it named and could load a
///   same-named file that happened to sit next to the song.
///
/// So a song is free to name any file its own user can read, exactly as if
/// they had opened it: the loader only ever decodes the result as audio. What
/// this rules out is a *relative* reference that claims to stay inside the
/// song directory and does not.
pub fn resolve_relative(base: &std::path::Path, rel: &str) -> crate::error::Result<PathBuf> {
    use std::path::Component;

    let p = std::path::Path::new(rel);
    if rel.is_empty() {
        return Err(crate::error::Error::file(
            base,
            anyhow::anyhow!("sample path is empty"),
        ));
    }
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }

    let mut sanitized = PathBuf::new();
    for component in p.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            // `./x` names `x`; nothing is lost by dropping it.
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(crate::error::Error::file(
                    p,
                    anyhow::anyhow!("sample path leaves the song directory"),
                ))
            }
            // `p` is relative, so a root or prefix here means something like
            // a Windows `C:foo` read on another platform. Refuse to guess.
            Component::RootDir | Component::Prefix(_) => {
                return Err(crate::error::Error::file(
                    p,
                    anyhow::anyhow!("sample path is not a plain relative path"),
                ))
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        return Err(crate::error::Error::file(
            p,
            anyhow::anyhow!("sample path names no file"),
        ));
    }
    Ok(base.join(sanitized))
}

/// Create default channel configs for N channels.
pub fn default_channel_configs(n: usize) -> Vec<ChannelConfig> {
    (0..n).map(|i| ChannelConfig::new(i as u8)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::audio::synth::SynthParams;

    #[test]
    fn legacy_instruments_follow_the_old_routing_order() {
        let (kind, repairs) =
            InstrumentKind::from_legacy(3, Some(9), Some(SynthParams::from_patch(1)), None);
        assert!(matches!(kind, InstrumentKind::Sample { slot: 9, .. }));
        assert_eq!(repairs.len(), 1, "the unused synth params are reported");

        let (kind, repairs) =
            InstrumentKind::from_legacy(3, None, Some(SynthParams::from_patch(1)), Some(5));
        assert!(matches!(kind, InstrumentKind::Synth(_)));
        assert_eq!(repairs.len(), 1, "the unsent MIDI program is reported");

        let (kind, repairs) = InstrumentKind::from_legacy(3, None, None, None);
        assert!(matches!(
            kind,
            InstrumentKind::Midi {
                program: 3,
                channel: None,
                output: MidiOutput::Builtin
            }
        ));
        assert!(repairs.is_empty());
    }

    #[test]
    fn a_legacy_midi_program_is_kept_and_its_sound_change_reported() {
        let (kind, repairs) = InstrumentKind::from_legacy(3, None, None, Some(40));
        assert!(matches!(kind, InstrumentKind::Midi { program: 40, .. }));
        assert_eq!(repairs.len(), 1);

        let (_, repairs) = InstrumentKind::from_legacy(3, None, None, Some(3));
        assert!(repairs.is_empty(), "same program as the preset: no change");
    }

    #[test]
    fn instrument_kinds_serialize_compactly_and_round_trip() {
        let builtin = InstrumentKind::default_for(5);
        let json = serde_json::to_string(&builtin).unwrap();
        assert_eq!(json, r#"{"type":"midi","program":5}"#);

        let external = InstrumentKind::Midi {
            program: 5,
            channel: Some(9),
            output: MidiOutput::External { port: None },
        };
        let json = serde_json::to_string(&external).unwrap();
        assert_eq!(
            json,
            r#"{"type":"midi","program":5,"channel":9,"output":{"to":"external"}}"#
        );
        assert!(matches!(
            serde_json::from_str::<InstrumentKind>(&json).unwrap(),
            InstrumentKind::Midi {
                channel: Some(9),
                output: MidiOutput::External { port: None },
                ..
            }
        ));

        let sample: InstrumentKind = serde_json::from_str(r#"{"type":"sample","slot":2}"#).unwrap();
        assert!(matches!(sample, InstrumentKind::Sample { slot: 2, .. }));

        let synth = InstrumentKind::Synth(SynthParams::from_patch(1));
        let json = serde_json::to_string(&synth).unwrap();
        assert!(matches!(
            serde_json::from_str::<InstrumentKind>(&json).unwrap(),
            InstrumentKind::Synth(_)
        ));
    }
    use std::path::Path;

    #[test]
    fn plain_relative_paths_resolve_under_the_base() {
        let base = Path::new("/songs/set");
        assert_eq!(
            resolve_relative(base, "samples/kick.wav").expect("relative"),
            PathBuf::from("/songs/set/samples/kick.wav")
        );
        // `./` is noise, not a directory.
        assert_eq!(
            resolve_relative(base, "./kick.wav").expect("cur dir"),
            PathBuf::from("/songs/set/kick.wav")
        );
    }

    #[test]
    fn traversal_is_rejected_rather_than_stripped() {
        let base = Path::new("/songs/set");
        // Stripping `..` used to turn this into /songs/set/etc/passwd, a path
        // the song never named and one that can exist.
        assert!(resolve_relative(base, "../../etc/passwd").is_err());
        // Even a `..` that would cancel out: `make_relative` never writes one,
        // so its presence says the reference was not produced by a save.
        assert!(resolve_relative(base, "a/../b.wav").is_err());
        assert!(resolve_relative(base, "..").is_err());
    }

    #[test]
    fn absolute_paths_are_kept_as_written() {
        // The counterpart of `make_relative` falling back to an absolute path
        // for a sample outside the song directory. Rewriting it as
        // base/<file_name> lost the directory and could load a same-named
        // neighbour of the song file instead.
        let base = Path::new("/songs/set");
        assert_eq!(
            resolve_relative(base, "/srv/library/kick.wav").expect("absolute"),
            PathBuf::from("/srv/library/kick.wav")
        );
    }

    #[test]
    fn paths_that_name_nothing_are_errors() {
        let base = Path::new("/songs/set");
        // An entry with no path at all, as written for a sample with no
        // source file. It must not resolve to the song directory itself.
        assert!(resolve_relative(base, "").is_err());
        assert!(resolve_relative(base, ".").is_err());
    }

    #[test]
    fn an_unsaved_song_autosaves_outside_the_working_directory() {
        let path = autosave_path_for_unsaved("Untitled");
        assert!(
            path.is_absolute(),
            "an unsaved song's autosave must not be a bare relative name: {}",
            path.display()
        );
        assert!(
            path.starts_with(std::env::temp_dir()),
            "{} is not under the temp directory",
            path.display()
        );
        assert_ne!(
            path.parent(),
            Some(Path::new("")),
            "an empty parent is what put the file in the working directory"
        );
    }

    #[test]
    fn two_unsaved_songs_do_not_share_an_autosave_file() {
        assert_ne!(
            autosave_path_for_unsaved("First Idea"),
            autosave_path_for_unsaved("Second Idea")
        );
    }

    #[test]
    fn an_awkward_title_still_gives_a_usable_file_name() {
        for title in ["", "  ", "///", "My Song!", "../../etc/passwd"] {
            let path = autosave_path_for_unsaved(title);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("a file name");
            assert!(
                name.starts_with("rtrack-") && name.ends_with(".rtrk.autosave"),
                "title {title:?} produced {name:?}"
            );
            assert_eq!(
                path.parent(),
                Some(std::env::temp_dir().as_path()),
                "title {title:?} escaped the temp directory"
            );
        }
    }

    #[test]
    fn sample_fx_is_omitted_when_off_and_round_trips_when_set() {
        let plain = InstrumentKind::sample(2);
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"type":"sample","slot":2}"#
        );
        let fx = SampleFx {
            drive: 4.0,
            filter: Some(VoiceFilter {
                kind: crate::audio::synth::FilterType::HighPass,
                cutoff_hz: 300.0,
                resonance: 0.5,
            }),
        };
        let shaped = InstrumentKind::Sample { slot: 2, fx };
        let json = serde_json::to_string(&shaped).unwrap();
        assert_eq!(
            serde_json::from_str::<InstrumentKind>(&json).unwrap(),
            shaped
        );
        let drive_only: InstrumentKind =
            serde_json::from_str(r#"{"type":"sample","slot":2,"fx":{"drive":3.0}}"#).unwrap();
        assert!(matches!(
            drive_only,
            InstrumentKind::Sample {
                fx: SampleFx {
                    drive: 3.0,
                    filter: None
                },
                ..
            }
        ));
    }

    #[test]
    fn replacing_an_instruments_sample_keeps_its_drive_and_filter() {
        let fx = SampleFx {
            drive: 2.0,
            filter: Some(VoiceFilter::default()),
        };
        let mut inst = Instrument::for_slot(0);
        inst.kind = InstrumentKind::Sample { slot: 0, fx };
        inst.play_sample(7);
        assert_eq!(inst.kind, InstrumentKind::Sample { slot: 7, fx });

        let mut midi = Instrument::for_slot(1);
        midi.play_sample(1);
        assert_eq!(midi.kind, InstrumentKind::sample(1));
    }

    #[test]
    fn a_saved_path_round_trips() {
        let dir = tempfile::tempdir().expect("temp dir");
        let base = dir.path();
        let sample = base.join("samples/kick.wav");
        std::fs::create_dir_all(sample.parent().expect("parent")).expect("mkdir");
        std::fs::write(&sample, b"riff").expect("write");

        let rel = make_relative(base, &sample);
        let resolved = resolve_relative(base, &rel).expect("round trip");
        // Canonicalized on both sides: a temp dir can sit behind a symlink
        // (macOS /var -> /private/var), which is not what this is testing.
        assert_eq!(
            std::fs::canonicalize(&resolved).expect("canonical"),
            std::fs::canonicalize(&sample).expect("canonical")
        );

        // A sample outside the song directory: `make_relative` gives an
        // absolute path, which must resolve back to the same file.
        let outside = tempfile::tempdir().expect("temp dir");
        let far = outside.path().join("snare.wav");
        std::fs::write(&far, b"riff").expect("write");
        let rel = make_relative(base, &far);
        let resolved = resolve_relative(base, &rel).expect("round trip");
        assert_eq!(
            std::fs::canonicalize(&resolved).expect("canonical"),
            std::fs::canonicalize(&far).expect("canonical")
        );
    }
}
