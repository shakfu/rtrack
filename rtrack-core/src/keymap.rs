//! Keyboard-to-note mapping for tracker-style note entry.
//!
//! This is pure data with no UI dependency, so it lives here rather than in a
//! frontend: both the TUI and the GUI need exactly the same layout, and when
//! each kept its own copy the two tables had to be maintained in parallel.

use crate::tracker::NoteValue;

/// Highest octave a note may be entered at.
pub const MAX_ENTRY_OCTAVE: u8 = 9;

/// Map a key to a note and an octave offset relative to the current octave.
///
/// The layout is the classic two-row tracker piano:
///
/// ```text
///   lower octave:  z=C  s=C#  x=D  d=D#  c=E  v=F  g=F#  b=G  h=G#  n=A  j=A#  m=B
///   upper octave:  q=C  2=C#  w=D  3=D#  e=E  r=F  5=F#  t=G  6=G#  y=A  7=A#  u=B
/// ```
///
/// Returns `None` for any key that is not part of the layout.
pub fn piano_key(c: char) -> Option<(NoteValue, u8)> {
    let mapping = match c {
        'z' => (NoteValue::C, 0),
        's' => (NoteValue::Cs, 0),
        'x' => (NoteValue::D, 0),
        'd' => (NoteValue::Ds, 0),
        'c' => (NoteValue::E, 0),
        'v' => (NoteValue::F, 0),
        'g' => (NoteValue::Fs, 0),
        'b' => (NoteValue::G, 0),
        'h' => (NoteValue::Gs, 0),
        'n' => (NoteValue::A, 0),
        'j' => (NoteValue::As, 0),
        'm' => (NoteValue::B, 0),
        'q' => (NoteValue::C, 1),
        '2' => (NoteValue::Cs, 1),
        'w' => (NoteValue::D, 1),
        '3' => (NoteValue::Ds, 1),
        'e' => (NoteValue::E, 1),
        'r' => (NoteValue::F, 1),
        '5' => (NoteValue::Fs, 1),
        't' => (NoteValue::G, 1),
        '6' => (NoteValue::Gs, 1),
        'y' => (NoteValue::A, 1),
        '7' => (NoteValue::As, 1),
        'u' => (NoteValue::B, 1),
        _ => return None,
    };
    Some(mapping)
}

/// Resolve a key press to an absolute octave, or `None` if the key is not a
/// piano key or the result would exceed [`MAX_ENTRY_OCTAVE`].
pub fn piano_key_at_octave(c: char, current_octave: u8) -> Option<(NoteValue, u8)> {
    PianoLayout::Qwerty.piano_key_at_octave(c, current_octave)
}

/// A keyboard layout for note entry: which characters sit where QWERTY's
/// piano keys are (`docs/dev/keybindings.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PianoLayout {
    #[default]
    Qwerty,
    /// German: `z` and `y` swapped.
    Qwertz,
    /// French: the letters of the top-left keys move, and the number row
    /// types `\u{e9} " ( - \u{e8}` unshifted where QWERTY types `2 3 5 6 7`.
    Azerty,
}

impl PianoLayout {
    pub const ALL: [Self; 3] = [Self::Qwerty, Self::Qwertz, Self::Azerty];

    pub fn name(self) -> &'static str {
        match self {
            Self::Qwerty => "qwerty",
            Self::Qwertz => "qwertz",
            Self::Azerty => "azerty",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.name() == name)
    }

    /// The character at each QWERTY piano position: the lower row, C to B,
    /// then the upper row.
    fn keys(self) -> [char; 24] {
        let rows = match self {
            Self::Qwerty => "zsxdcvgbhnjmq2w3er5t6y7u",
            Self::Qwertz => "ysxdcvgbhnjmq2w3er5t6z7u",
            Self::Azerty => "wsxdcvgbhnj,a\u{e9}z\"er(t-y\u{e8}u",
        };
        let mut keys = ['\0'; 24];
        for (slot, c) in keys.iter_mut().zip(rows.chars()) {
            *slot = c;
        }
        keys
    }

    /// [`piano_key`] in this layout.
    pub fn piano_key(self, c: char) -> Option<(NoteValue, u8)> {
        let at = self.keys().iter().position(|&k| k == c)?;
        // The QWERTY character at that position gives the note.
        piano_key(PianoLayout::Qwerty.keys()[at])
    }

    /// [`piano_key_at_octave`] in this layout.
    pub fn piano_key_at_octave(self, c: char, current_octave: u8) -> Option<(NoteValue, u8)> {
        let (value, offset) = self.piano_key(c)?;
        let octave = current_octave.checked_add(offset)?;
        (octave <= MAX_ENTRY_OCTAVE).then_some((value, octave))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both rows must cover a full chromatic octave, in order, with no gaps
    /// and no key doing double duty.
    #[test]
    fn each_row_covers_a_chromatic_octave_in_order() {
        const CHROMATIC: [NoteValue; 12] = [
            NoteValue::C,
            NoteValue::Cs,
            NoteValue::D,
            NoteValue::Ds,
            NoteValue::E,
            NoteValue::F,
            NoteValue::Fs,
            NoteValue::G,
            NoteValue::Gs,
            NoteValue::A,
            NoteValue::As,
            NoteValue::B,
        ];
        let lower = "zsxdcvgbhnjm";
        let upper = "q2w3er5t6y7u";

        for (row, offset) in [(lower, 0u8), (upper, 1u8)] {
            for (key, expected) in row.chars().zip(CHROMATIC) {
                assert_eq!(
                    piano_key(key),
                    Some((expected, offset)),
                    "key '{key}' in row offset {offset}"
                );
            }
        }
    }

    #[test]
    fn no_key_appears_in_both_rows() {
        let mut seen = std::collections::HashSet::new();
        for c in "zsxdcvgbhnjmq2w3er5t6y7u".chars() {
            assert!(seen.insert(c), "key '{c}' is mapped twice");
        }
    }

    #[test]
    fn unmapped_keys_return_none() {
        for c in ['k', 'l', 'p', '1', '4', '8', '9', '0', ' ', 'Z'] {
            assert_eq!(piano_key(c), None, "'{c}' should not be a piano key");
        }
    }

    #[test]
    fn every_layout_puts_the_same_notes_at_the_same_positions() {
        let qwerty = PianoLayout::Qwerty.keys();
        for layout in PianoLayout::ALL {
            let keys = layout.keys();
            let mut seen = std::collections::HashSet::new();
            for (key, at) in keys.iter().zip(qwerty) {
                assert!(seen.insert(*key), "{layout:?} maps '{key}' twice");
                assert_eq!(layout.piano_key(*key), piano_key(at), "{layout:?} '{key}'");
            }
            assert_eq!(PianoLayout::from_name(layout.name()), Some(layout));
        }
        assert_eq!(PianoLayout::Qwertz.piano_key('y'), Some((NoteValue::C, 0)));
        assert_eq!(PianoLayout::Qwertz.piano_key('z'), Some((NoteValue::A, 1)));
        assert_eq!(PianoLayout::Azerty.piano_key('-'), Some((NoteValue::Gs, 1)));
        assert_eq!(PianoLayout::Azerty.piano_key('m'), None);
        assert_eq!(
            PianoLayout::from_name(" QWERTZ "),
            Some(PianoLayout::Qwertz)
        );
        assert_eq!(PianoLayout::from_name("dvorak"), None);
    }

    #[test]
    fn octave_offsets_are_applied() {
        assert_eq!(piano_key_at_octave('z', 4), Some((NoteValue::C, 4)));
        assert_eq!(piano_key_at_octave('q', 4), Some((NoteValue::C, 5)));
    }

    #[test]
    fn entry_above_the_top_octave_is_refused() {
        assert_eq!(
            piano_key_at_octave('z', MAX_ENTRY_OCTAVE),
            Some((NoteValue::C, 9))
        );
        // The upper row would land on octave 10.
        assert_eq!(piano_key_at_octave('q', MAX_ENTRY_OCTAVE), None);
    }

    #[test]
    fn an_absurd_current_octave_does_not_overflow() {
        assert_eq!(piano_key_at_octave('q', u8::MAX), None);
    }
}
