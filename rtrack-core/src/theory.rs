//! Scale masks for note-entry quantization.
//!
//! A scale is a 12-bit mask over semitones from its root: bit `n` is set when
//! semitone `n` belongs to the scale. Masks here are written relative to C and
//! rotated to the song's root by [`ScaleSetting::mask`].
//!
//! Nothing in this module quantizes by itself. Snapping is applied at note
//! entry by the frontends via [`crate::tracker::Song::snap_entry`]; the engine,
//! file loader, MIDI input and clipboard paste all stay unquantized.

use serde::{Deserialize, Serialize};

use crate::constants::{MIDI_MAX_NOTE, SEMITONES_PER_OCTAVE};
use crate::tracker::NoteValue;

/// A scale, stored as a pitch-class mask relative to its root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Chromatic,
    Major,
    Minor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    WholeTone,
    HalfWhole,
    WholeHalf,
    PhrygianDominant,
    HungarianMinor,
    DoubleHarmonic,
    Persian,
    Hirajoshi,
    InSen,
    Iwato,
    Kumoi,
    Egyptian,
    Ryukyu,
}

/// Build a 12-bit mask from semitone offsets.
const fn mask_of(degrees: &[u8]) -> u16 {
    let mut mask = 0u16;
    let mut i = 0;
    while i < degrees.len() {
        mask |= 1 << degrees[i];
        i += 1;
    }
    mask
}

impl Scale {
    /// Every scale, in menu order.
    pub const ALL: [Scale; 26] = [
        Self::Chromatic,
        Self::Major,
        Self::Minor,
        Self::HarmonicMinor,
        Self::MelodicMinor,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Locrian,
        Self::MajorPentatonic,
        Self::MinorPentatonic,
        Self::Blues,
        Self::WholeTone,
        Self::HalfWhole,
        Self::WholeHalf,
        Self::PhrygianDominant,
        Self::HungarianMinor,
        Self::DoubleHarmonic,
        Self::Persian,
        Self::Hirajoshi,
        Self::InSen,
        Self::Iwato,
        Self::Kumoi,
        Self::Egyptian,
        Self::Ryukyu,
    ];

    /// Pitch-class mask relative to the scale's own root.
    pub const fn mask(self) -> u16 {
        match self {
            Self::Chromatic => 0x0FFF,
            Self::Major => mask_of(&[0, 2, 4, 5, 7, 9, 11]),
            Self::Minor => mask_of(&[0, 2, 3, 5, 7, 8, 10]),
            Self::HarmonicMinor => mask_of(&[0, 2, 3, 5, 7, 8, 11]),
            Self::MelodicMinor => mask_of(&[0, 2, 3, 5, 7, 9, 11]),
            Self::Dorian => mask_of(&[0, 2, 3, 5, 7, 9, 10]),
            Self::Phrygian => mask_of(&[0, 1, 3, 5, 7, 8, 10]),
            Self::Lydian => mask_of(&[0, 2, 4, 6, 7, 9, 11]),
            Self::Mixolydian => mask_of(&[0, 2, 4, 5, 7, 9, 10]),
            Self::Locrian => mask_of(&[0, 1, 3, 5, 6, 8, 10]),
            Self::MajorPentatonic => mask_of(&[0, 2, 4, 7, 9]),
            Self::MinorPentatonic => mask_of(&[0, 3, 5, 7, 10]),
            Self::Blues => mask_of(&[0, 3, 5, 6, 7, 10]),
            Self::WholeTone => mask_of(&[0, 2, 4, 6, 8, 10]),
            Self::HalfWhole => mask_of(&[0, 1, 3, 4, 6, 7, 9, 10]),
            Self::WholeHalf => mask_of(&[0, 2, 3, 5, 6, 8, 9, 11]),
            Self::PhrygianDominant => mask_of(&[0, 1, 4, 5, 7, 8, 10]),
            Self::HungarianMinor => mask_of(&[0, 2, 3, 6, 7, 8, 11]),
            Self::DoubleHarmonic => mask_of(&[0, 1, 4, 5, 7, 8, 11]),
            Self::Persian => mask_of(&[0, 1, 4, 5, 6, 8, 11]),
            Self::Hirajoshi => mask_of(&[0, 2, 3, 7, 8]),
            Self::InSen => mask_of(&[0, 1, 5, 7, 10]),
            Self::Iwato => mask_of(&[0, 1, 5, 6, 10]),
            Self::Kumoi => mask_of(&[0, 2, 3, 7, 9]),
            Self::Egyptian => mask_of(&[0, 2, 5, 7, 10]),
            Self::Ryukyu => mask_of(&[0, 4, 5, 7, 11]),
        }
    }

    /// Display name, also the form accepted by [`Scale::from_name`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chromatic => "chromatic",
            Self::Major => "major",
            Self::Minor => "minor",
            Self::HarmonicMinor => "harmonic minor",
            Self::MelodicMinor => "melodic minor",
            Self::Dorian => "dorian",
            Self::Phrygian => "phrygian",
            Self::Lydian => "lydian",
            Self::Mixolydian => "mixolydian",
            Self::Locrian => "locrian",
            Self::MajorPentatonic => "major pentatonic",
            Self::MinorPentatonic => "minor pentatonic",
            Self::Blues => "blues",
            Self::WholeTone => "whole tone",
            Self::HalfWhole => "half-whole",
            Self::WholeHalf => "whole-half",
            Self::PhrygianDominant => "phrygian dominant",
            Self::HungarianMinor => "hungarian minor",
            Self::DoubleHarmonic => "double harmonic",
            Self::Persian => "persian",
            Self::Hirajoshi => "hirajoshi",
            Self::InSen => "in sen",
            Self::Iwato => "iwato",
            Self::Kumoi => "kumoi",
            Self::Egyptian => "egyptian",
            Self::Ryukyu => "ryukyu",
        }
    }

    /// Parse a scale name. Case-insensitive; `_` and `-` count as spaces.
    pub fn from_name(s: &str) -> Option<Scale> {
        let want = normalize(s);
        Self::ALL
            .iter()
            .copied()
            .find(|sc| normalize(sc.name()) == want)
    }
}

fn normalize(s: &str) -> String {
    s.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A scale anchored to a root pitch class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleSetting {
    pub root: NoteValue,
    pub scale: Scale,
}

impl ScaleSetting {
    pub fn new(root: NoteValue, scale: Scale) -> Self {
        Self { root, scale }
    }

    /// Absolute pitch-class mask, the scale's mask rotated to `root`.
    pub fn mask(self) -> u16 {
        let r = self.root.to_index() as u32;
        let m = self.scale.mask() as u32;
        (((m << r) | (m >> (SEMITONES_PER_OCTAVE as u32 - r))) & 0x0FFF) as u16
    }

    /// Snap a MIDI note to the nearest pitch in the scale.
    pub fn snap(self, midi: u8) -> u8 {
        snap_to_mask(midi, self.mask())
    }

    /// Move `midi` by `degrees` steps along the scale.
    ///
    /// An off-scale note is snapped first, so transposing a chromatic pattern
    /// by one degree lands it in the scale rather than preserving the offset.
    /// A step that would leave the MIDI range returns the note unchanged,
    /// matching `Note::transposed`.
    pub fn transpose(self, midi: u8, degrees: i16) -> u8 {
        let mask = self.mask();
        let from = snap_to_mask(midi, mask);
        if degrees == 0 {
            return from;
        }
        let step: i16 = if degrees > 0 { 1 } else { -1 };
        let mut current = from as i16;
        for _ in 0..degrees.abs() {
            let mut next = current + step;
            while (0..=MIDI_MAX_NOTE as i16).contains(&next)
                && mask & (1 << (next % SEMITONES_PER_OCTAVE as i16)) == 0
            {
                next += step;
            }
            if !(0..=MIDI_MAX_NOTE as i16).contains(&next) {
                return midi;
            }
            current = next;
        }
        current as u8
    }

    /// `"C# dorian"`.
    pub fn display(self) -> String {
        format!("{} {}", self.root.name(), self.scale.name())
    }

    /// Parse `"C# dorian"`: a root note then a scale name. `off`, `none` and
    /// the empty string parse to `Ok(None)`, meaning no quantization.
    ///
    /// A typo must not read as `off`: collapsing both to `None` would turn
    /// quantization off on a misspelling.
    pub fn parse(s: &str) -> Result<Option<ScaleSetting>, ScaleParseError> {
        let s = normalize(s);
        if s.is_empty() || s == "off" || s == "none" {
            return Ok(None);
        }
        let (root, rest) = s.split_once(' ').ok_or(ScaleParseError)?;
        let root = NoteValue::from_name(root).ok_or(ScaleParseError)?;
        let scale = Scale::from_name(rest).ok_or(ScaleParseError)?;
        Ok(Some(ScaleSetting::new(root, scale)))
    }
}

/// A scale field that is neither a `<root> <scale>` pair nor `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleParseError;

impl std::fmt::Display for ScaleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expected a root note and a scale name, or \"off\"")
    }
}

impl std::error::Error for ScaleParseError {}

/// Nearest MIDI note whose pitch class is set in `mask` (bit `n` = semitone
/// `n`, absolute, C = 0). Ties resolve downward. An empty mask, or a note with
/// no in-range neighbour, returns `midi` unchanged.
pub fn snap_to_mask(midi: u8, mask: u16) -> u8 {
    if mask == 0 || mask & (1 << (midi % SEMITONES_PER_OCTAVE)) != 0 {
        return midi;
    }
    // A pitch class recurs every 12 semitones, so 12 is always enough.
    for d in 1..=SEMITONES_PER_OCTAVE as i16 {
        for cand in [midi as i16 - d, midi as i16 + d] {
            if (0..=MIDI_MAX_NOTE as i16).contains(&cand)
                && mask & (1 << (cand % SEMITONES_PER_OCTAVE as i16)) != 0
            {
                return cand as u8;
            }
        }
    }
    midi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scale_contains_its_root_and_fits_one_octave() {
        for scale in Scale::ALL {
            let m = scale.mask();
            assert_eq!(m & 1, 1, "{} omits its root", scale.name());
            assert_eq!(m & !0x0FFF, 0, "{} sets a bit above 11", scale.name());
        }
    }

    #[test]
    fn scale_names_round_trip_and_accept_separators() {
        for scale in Scale::ALL {
            assert_eq!(Scale::from_name(scale.name()), Some(scale));
        }
        assert_eq!(
            Scale::from_name("Harmonic_Minor"),
            Some(Scale::HarmonicMinor)
        );
        assert_eq!(Scale::from_name("  MAJOR  "), Some(Scale::Major));
        assert_eq!(Scale::from_name("bogus"), None);
    }

    #[test]
    fn the_root_rotates_the_mask() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        assert_eq!(c_major.mask(), 0b1010_1011_0101);
        // D major adds C# and F#, drops C and F.
        let d_major = ScaleSetting::new(NoteValue::D, Scale::Major);
        assert_eq!(d_major.mask() & 1, 0, "C is not in D major");
        assert_eq!(d_major.mask() & (1 << 1), 1 << 1, "C# is in D major");
        assert_eq!(d_major.mask() & (1 << 6), 1 << 6, "F# is in D major");
        assert_eq!(d_major.mask() & (1 << 5), 0, "F is not in D major");
    }

    #[test]
    fn a_note_already_in_the_scale_is_left_alone() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        for midi in [60, 62, 64, 65, 67, 69, 71] {
            assert_eq!(c_major.snap(midi), midi);
        }
    }

    #[test]
    fn snapping_picks_the_nearest_pitch_and_breaks_ties_downward() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        assert_eq!(c_major.snap(61), 60, "C# -> C");
        assert_eq!(c_major.snap(66), 65, "F# is equidistant, prefer F");
        assert_eq!(c_major.snap(70), 69, "A# -> A");

        // tonal's pcset_nearest example: {C, F, G}
        let mask = mask_of(&[0, 5, 7]);
        assert_eq!(snap_to_mask(1, mask), 0);
        assert_eq!(snap_to_mask(3, mask), 5);
    }

    #[test]
    fn chromatic_never_moves_a_note() {
        let chromatic = ScaleSetting::new(NoteValue::C, Scale::Chromatic);
        for midi in 0..=MIDI_MAX_NOTE {
            assert_eq!(chromatic.snap(midi), midi);
        }
    }

    #[test]
    fn snapping_stays_in_the_midi_range_at_both_ends() {
        for scale in Scale::ALL {
            for root in 0..SEMITONES_PER_OCTAVE {
                let s = ScaleSetting::new(NoteValue::from_index(root).unwrap(), scale);
                for midi in [0, 1, 2, 125, 126, MIDI_MAX_NOTE] {
                    let out = s.snap(midi);
                    assert!(out <= MIDI_MAX_NOTE, "{} left the range", s.display());
                    assert!(
                        s.mask() & (1 << (out % SEMITONES_PER_OCTAVE)) != 0,
                        "{} snapped {} to {}, which is off-scale",
                        s.display(),
                        midi,
                        out
                    );
                }
            }
        }
    }

    #[test]
    fn transposing_by_degree_walks_the_scale() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        // C E G are degrees apart by 2; one step from E is F.
        assert_eq!(c_major.transpose(64, 1), 65, "E -> F");
        assert_eq!(c_major.transpose(64, -1), 62, "E -> D");
        assert_eq!(c_major.transpose(60, 7), 72, "seven degrees is an octave");
        assert_eq!(c_major.transpose(72, -7), 60);
        assert_eq!(c_major.transpose(60, 0), 60);
    }

    #[test]
    fn transposing_an_off_scale_note_snaps_it_first() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        assert_eq!(c_major.transpose(61, 0), 60, "C# snaps to C");
        assert_eq!(c_major.transpose(61, 1), 62, "then one degree up is D");
    }

    #[test]
    fn a_degree_past_the_midi_range_is_refused() {
        let c_major = ScaleSetting::new(NoteValue::C, Scale::Major);
        assert_eq!(c_major.transpose(127, 1), 127, "B9 has nowhere to go");
        assert_eq!(c_major.transpose(0, -1), 0);
        // A long walk that runs out returns the input untouched.
        assert_eq!(c_major.transpose(120, 99), 120);
    }

    #[test]
    fn every_transposed_note_stays_in_the_scale() {
        for scale in Scale::ALL {
            for root in 0..SEMITONES_PER_OCTAVE {
                let s = ScaleSetting::new(NoteValue::from_index(root).unwrap(), scale);
                for midi in [1u8, 40, 60, 61, 100, 126] {
                    for degrees in [-3i16, -1, 1, 3] {
                        let out = s.transpose(midi, degrees);
                        if out == midi && s.snap(midi) != midi {
                            continue; // refused at the range edge
                        }
                        assert!(
                            s.mask() & (1 << (out % SEMITONES_PER_OCTAVE)) != 0,
                            "{} moved {} by {} to off-scale {}",
                            s.display(),
                            midi,
                            degrees,
                            out
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn an_empty_mask_is_a_no_op() {
        assert_eq!(snap_to_mask(61, 0), 61);
    }

    #[test]
    fn settings_parse_round_trips_and_rejects_junk() {
        let s = ScaleSetting::new(NoteValue::Cs, Scale::Dorian);
        assert_eq!(s.display(), "C# dorian");
        assert_eq!(ScaleSetting::parse("C# dorian"), Ok(Some(s)));
        assert_eq!(ScaleSetting::parse("c#_DORIAN"), Ok(Some(s)));
        assert_eq!(ScaleSetting::parse("off"), Ok(None));
        assert_eq!(ScaleSetting::parse("none"), Ok(None));
        assert_eq!(ScaleSetting::parse("   "), Ok(None));
        assert_eq!(ScaleSetting::parse("H major"), Err(ScaleParseError));
        assert_eq!(ScaleSetting::parse("C dorain"), Err(ScaleParseError));
        assert_eq!(ScaleSetting::parse("C"), Err(ScaleParseError));
    }
}
