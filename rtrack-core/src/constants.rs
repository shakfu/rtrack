//! Shared constants used across the rtrack codebase.
//!
//! Centralises magic numbers so they are defined once and documented in one place.

// ---------------------------------------------------------------------------
// MIDI protocol
// ---------------------------------------------------------------------------

/// Maximum valid MIDI note number.
pub const MIDI_MAX_NOTE: u8 = 127;

/// Maximum valid MIDI velocity / CC value (7-bit).
pub const MIDI_MAX_VALUE: u8 = 0x7F;

/// Default MIDI velocity when none is specified.
pub const MIDI_DEFAULT_VELOCITY: u8 = MIDI_MAX_VALUE;

/// Number of MIDI channels.
pub const MIDI_CHANNELS: usize = 16;

/// MIDI pitch bend center (no bend). 14-bit midpoint = 0x2000 = 8192.
pub const PITCH_BEND_CENTER: u16 = 0x2000;

/// Maximum 14-bit pitch bend value.
pub const PITCH_BEND_MAX: u16 = 0x3FFF;

/// Default pitch bend range in semitones (standard MIDI GM default).
pub const DEFAULT_PITCH_BEND_RANGE: f64 = 2.0;

/// Pitch bend units per semitone at the default range.
#[allow(clippy::cast_lossless)]
pub const PITCH_BEND_PER_SEMITONE: f64 = (PITCH_BEND_CENTER as f64) / DEFAULT_PITCH_BEND_RANGE;

/// MIDI clocks (pulses) per quarter note (24 ppqn standard).
pub const MIDI_CLOCKS_PER_BEAT: f64 = 24.0;

// ---------------------------------------------------------------------------
// Music theory
// ---------------------------------------------------------------------------

/// Number of semitones in one octave.
pub const SEMITONES_PER_OCTAVE: u8 = 12;

// ---------------------------------------------------------------------------
// Tracker effect commands (single hex digit, stored in Cell.effect)
// ---------------------------------------------------------------------------

pub const EFFECT_ARPEGGIO: u8 = 0x0; // 0xy: cycle note, note+x, note+y
pub const EFFECT_PORTA_UP: u8 = 0x1; // 1xx: slide pitch up by xx per tick
pub const EFFECT_PORTA_DOWN: u8 = 0x2; // 2xx: slide pitch down by xx per tick
pub const EFFECT_TONE_PORTA: u8 = 0x3; // 3xx: slide toward target note at speed xx
pub const EFFECT_VIBRATO: u8 = 0x4; // 4xy: vibrato speed x, depth y
pub const EFFECT_VOLUME_SLIDE: u8 = 0x5; // 5xy: volume slide up x, down y per tick
pub const EFFECT_NOTE_DELAY: u8 = 0x6; // 6xx: delay note trigger by xx ticks
pub const EFFECT_POSITION_JUMP: u8 = 0xB; // Bxx: jump to order position xx
pub const EFFECT_MIDI_CC: u8 = 0xC; // Cxx: send MIDI CC (controller from instrument col, value xx)
pub const EFFECT_PATTERN_BREAK: u8 = 0xD; // Dxx: break to row xx of next pattern
pub const EFFECT_PROGRAM_CHANGE: u8 = 0xE; // Exx: program change to program xx
pub const EFFECT_SET_SPEED: u8 = 0xF; // Fxx: xx<0x20 = set speed, xx>=0x20 = set BPM

// ---------------------------------------------------------------------------
// Tracker defaults and limits
// ---------------------------------------------------------------------------

/// Default pattern length (rows).
pub const DEFAULT_ROWS_PER_PATTERN: usize = 64;

/// Maximum number of instruments.
pub const MAX_INSTRUMENTS: usize = 256;

/// Maximum number of tracker channels.
pub const MAX_CHANNELS: usize = 16;

/// Number of channels displayed per track page.
pub const CHANNELS_PER_PAGE: usize = 4;

/// Maximum length of a channel name in the pattern editor header.
pub const MAX_CHANNEL_NAME: usize = 8;

/// Preview note auto-off timeout in milliseconds.
pub const PREVIEW_NOTE_TIMEOUT_MS: u64 = 250;

/// How far ahead of the audio clock the sequencer schedules note events,
/// in seconds.
///
/// Events are stamped with the audio frame at which they should sound, so
/// this only has to cover the worst-case gap between two `tick_playback`
/// calls: roughly one display refresh in the GUI (~16.7ms at 60Hz). Larger
/// values absorb more UI stalls at the cost of a longer delay before a
/// transport change (start, stop, tempo edit) is heard.
pub const SCHEDULER_LOOKAHEAD_SECS: f64 = 0.025;

/// Upper bound on scheduled-but-not-yet-audible positions retained for the
/// UI playback cursor. One entry per row is pushed, and a row is never
/// shorter than a tick, so a handful covers any realistic lookahead.
pub const MAX_SCHEDULED_POSITIONS: usize = 64;

/// Largest audio file that will be loaded into a sample slot, in bytes.
///
/// Samples are decoded into memory in full as stereo `f32`, so a file this
/// size can occupy several times as much RAM. The limit is a guard against
/// a mistyped path or a corrupt header, not a judgement about what makes a
/// reasonable sample.
pub const MAX_SAMPLE_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// How long a one-shot sample preview is tracked before the record is
/// dropped.
///
/// One-shot samples end on their own, so they are not cut off at
/// `PREVIEW_NOTE_TIMEOUT_MS`. This is only an upper bound on how long the
/// core remembers that a preview happened.
pub const PREVIEW_ONE_SHOT_MAX_MS: u64 = 10_000;

/// Length of the de-click ramp applied to a sample voice, in seconds.
///
/// A slice begins and ends at an arbitrary frame, so its first and last
/// frames are almost never at a zero crossing. Stopping a voice by simply
/// dropping it leaves a step in the output -- a full-scale one for a loud
/// slice -- which is heard as a click. Voices therefore fade over this
/// window instead: at the end of a one-shot, and when a voice is stolen to
/// make room for a new one.
pub const SAMPLE_DECLICK_SECS: f32 = 0.005;

/// Envelope level below which a voice is inaudible and can be dropped
/// outright rather than faded.
pub const SAMPLE_INAUDIBLE_LEVEL: f32 = 0.001;
