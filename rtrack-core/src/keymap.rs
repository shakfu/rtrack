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
    let (value, offset) = piano_key(c)?;
    let octave = current_octave.checked_add(offset)?;
    (octave <= MAX_ENTRY_OCTAVE).then_some((value, octave))
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
