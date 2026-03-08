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

pub const EFFECT_ARPEGGIO: u8 = 0x0;      // 0xy: cycle note, note+x, note+y
pub const EFFECT_PORTA_UP: u8 = 0x1;      // 1xx: slide pitch up by xx per tick
pub const EFFECT_PORTA_DOWN: u8 = 0x2;    // 2xx: slide pitch down by xx per tick
pub const EFFECT_TONE_PORTA: u8 = 0x3;    // 3xx: slide toward target note at speed xx
pub const EFFECT_VIBRATO: u8 = 0x4;       // 4xy: vibrato speed x, depth y
pub const EFFECT_VOLUME_SLIDE: u8 = 0x5;  // 5xy: volume slide up x, down y per tick
pub const EFFECT_NOTE_DELAY: u8 = 0x6;    // 6xx: delay note trigger by xx ticks
pub const EFFECT_POSITION_JUMP: u8 = 0xB; // Bxx: jump to order position xx
pub const EFFECT_MIDI_CC: u8 = 0xC;       // Cxx: send MIDI CC (controller from instrument col, value xx)
pub const EFFECT_PATTERN_BREAK: u8 = 0xD; // Dxx: break to row xx of next pattern
pub const EFFECT_PROGRAM_CHANGE: u8 = 0xE; // Exx: program change to program xx
pub const EFFECT_SET_SPEED: u8 = 0xF;     // Fxx: xx<0x20 = set speed, xx>=0x20 = set BPM

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
