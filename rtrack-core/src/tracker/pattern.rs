use serde::{Deserialize, Serialize};

use crate::constants::{MIDI_MAX_NOTE, SEMITONES_PER_OCTAVE};

/// A musical note pitch (C, C#, D, ... B)
///
/// Sharps serialize as `Cs`, `Ds`, ... . The `#`-spelled aliases are accepted
/// on load so that song files written by earlier versions still parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteValue {
    C,
    #[serde(alias = "C#")]
    Cs,
    D,
    #[serde(alias = "D#")]
    Ds,
    E,
    F,
    #[serde(alias = "F#")]
    Fs,
    G,
    #[serde(alias = "G#")]
    Gs,
    A,
    #[serde(alias = "A#")]
    As,
    B,
}

impl NoteValue {
    pub fn from_index(i: u8) -> Option<Self> {
        match i {
            0 => Some(Self::C),
            1 => Some(Self::Cs),
            2 => Some(Self::D),
            3 => Some(Self::Ds),
            4 => Some(Self::E),
            5 => Some(Self::F),
            6 => Some(Self::Fs),
            7 => Some(Self::G),
            8 => Some(Self::Gs),
            9 => Some(Self::A),
            10 => Some(Self::As),
            11 => Some(Self::B),
            _ => None,
        }
    }

    pub fn to_index(self) -> u8 {
        match self {
            Self::C => 0,
            Self::Cs => 1,
            Self::D => 2,
            Self::Ds => 3,
            Self::E => 4,
            Self::F => 5,
            Self::Fs => 6,
            Self::G => 7,
            Self::Gs => 8,
            Self::A => 9,
            Self::As => 10,
            Self::B => 11,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::C => "C-",
            Self::Cs => "C#",
            Self::D => "D-",
            Self::Ds => "D#",
            Self::E => "E-",
            Self::F => "F-",
            Self::Fs => "F#",
            Self::G => "G-",
            Self::Gs => "G#",
            Self::A => "A-",
            Self::As => "A#",
            Self::B => "B-",
        }
    }
}

/// Represents a note event in a tracker cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Note {
    /// A note with pitch and octave (e.g., C-4)
    On { value: NoteValue, octave: u8 },
    /// Note off event
    Off,
}

impl Note {
    /// Convert to MIDI note number (0-127)
    pub fn to_midi_note(&self) -> Option<u8> {
        match self {
            Note::On { value, octave } => {
                let midi = (*octave as u16) * SEMITONES_PER_OCTAVE as u16 + value.to_index() as u16;
                if midi <= MIDI_MAX_NOTE as u16 {
                    Some(midi as u8)
                } else {
                    None
                }
            }
            Note::Off => None,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Note::On { value, octave } => format!("{}{}", value.display_name(), octave),
            Note::Off => "===".to_string(),
        }
    }

    /// Return a new Note transposed by the given number of semitones.
    /// Note::Off is returned unchanged. Clamps to valid MIDI range (0-127).
    /// Transpose by a number of semitones.
    ///
    /// A transpose that would leave the MIDI range is refused: the note is
    /// returned unchanged rather than clamped. Clamping would silently
    /// collapse the top or bottom of a transposed selection onto one pitch,
    /// which is worse than leaving those notes where the user put them.
    /// `Note::Off` is unaffected.
    pub fn transposed(self, semitones: i8) -> Note {
        match self {
            Note::On { value, octave } => {
                let midi = (octave as i16) * SEMITONES_PER_OCTAVE as i16
                    + value.to_index() as i16
                    + semitones as i16;
                if midi < 0 || midi > MIDI_MAX_NOTE as i16 {
                    return self;
                }
                let midi = midi as u8;
                let new_octave = midi / SEMITONES_PER_OCTAVE;
                let new_index = midi % SEMITONES_PER_OCTAVE;
                match NoteValue::from_index(new_index) {
                    Some(nv) => Note::On {
                        value: nv,
                        octave: new_octave,
                    },
                    None => self, // shouldn't happen
                }
            }
            Note::Off => Note::Off,
        }
    }
}

/// A single cell in the tracker grid.
///
/// Unset fields are omitted when serialized. Tracker patterns are mostly
/// empty, and spelling out five nulls per cell dominated the file: a 4x16
/// pattern with one note cost about 5.9 KB before this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cell {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_value: Option<u8>,
}

impl Cell {
    /// Transpose this cell's note in place, if it has one.
    ///
    /// Shared by both frontends' transpose commands so they cannot drift
    /// apart on edge cases like the ends of the MIDI range.
    pub fn transpose_note(&mut self, semitones: i8) {
        if let Some(note) = self.note {
            self.note = Some(note.transposed(semitones));
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.note.is_none()
            && self.instrument.is_none()
            && self.volume.is_none()
            && self.effect.is_none()
    }

    /// Format the cell for display: "C-4 01 80 000"
    pub fn display_note(&self) -> String {
        match &self.note {
            Some(n) => n.display(),
            None => "---".to_string(),
        }
    }

    pub fn display_instrument(&self) -> String {
        match self.instrument {
            Some(i) => format!("{:02X}", i),
            None => "--".to_string(),
        }
    }

    pub fn display_volume(&self) -> String {
        match self.volume {
            Some(v) => format!("{:02X}", v),
            None => "--".to_string(),
        }
    }

    pub fn display_effect(&self) -> String {
        let cmd = match self.effect {
            Some(e) => format!("{:01X}", e & 0x0F),
            None => "-".to_string(),
        };
        let val = match self.effect_value {
            Some(v) => format!("{:02X}", v),
            None => "--".to_string(),
        };
        format!("{}{}", cmd, val)
    }
}

// ---------------------------------------------------------------------------
// Pattern (multi-channel grid -- the song's only note storage)
// ---------------------------------------------------------------------------

/// A pattern is a grid of rows x channels. Patterns are addressed
/// indirectly through `Song::order`, so one pattern may appear at several
/// positions in the song.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub rows: usize,
    pub channels: usize,
    pub data: Vec<Vec<Cell>>, // data[row][channel]
}

impl Pattern {
    pub fn new(rows: usize, channels: usize) -> Self {
        Self {
            rows,
            channels,
            data: vec![vec![Cell::default(); channels]; rows],
        }
    }

    pub fn get(&self, row: usize, channel: usize) -> &Cell {
        &self.data[row][channel]
    }

    pub fn get_mut(&mut self, row: usize, channel: usize) -> &mut Cell {
        &mut self.data[row][channel]
    }

    pub fn set_cell(&mut self, row: usize, channel: usize, cell: Cell) {
        self.data[row][channel] = cell;
    }

    /// Insert an empty row at the given index, pushing rows below down.
    /// The last row is discarded to keep the pattern length constant.
    pub fn insert_row(&mut self, at: usize) {
        if at >= self.rows {
            return;
        }
        self.data.insert(at, vec![Cell::default(); self.channels]);
        self.data.truncate(self.rows);
    }

    /// Delete the row at the given index, shifting rows below up.
    /// An empty row is appended at the end to keep the pattern length constant.
    pub fn delete_row(&mut self, at: usize) {
        if at >= self.rows {
            return;
        }
        self.data.remove(at);
        self.data.push(vec![Cell::default(); self.channels]);
    }

    /// Force the backing storage to match `rows` x `channels` exactly,
    /// padding with empty cells and truncating as needed. Used when loading
    /// a file whose declared geometry disagrees with its cell data.
    pub fn conform(&mut self, rows: usize, channels: usize) {
        self.rows = rows;
        self.channels = channels;
        self.data.truncate(rows);
        for row in self.data.iter_mut() {
            row.truncate(channels);
            row.resize(channels, Cell::default());
        }
        while self.data.len() < rows {
            self.data.push(vec![Cell::default(); channels]);
        }
    }

    /// Resize the pattern to a new number of rows, truncating or padding with empty rows.
    #[allow(dead_code)]
    pub fn resize_rows(&mut self, new_rows: usize) {
        if new_rows > self.rows {
            for _ in self.rows..new_rows {
                self.data.push(vec![Cell::default(); self.channels]);
            }
        } else {
            self.data.truncate(new_rows);
        }
        self.rows = new_rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_value_roundtrip() {
        for i in 0..12u8 {
            let nv = NoteValue::from_index(i).unwrap();
            assert_eq!(nv.to_index(), i);
        }
        assert!(NoteValue::from_index(12).is_none());
    }

    #[test]
    fn test_note_to_midi() {
        let note = Note::On {
            value: NoteValue::C,
            octave: 4,
        };
        assert_eq!(note.to_midi_note(), Some(48));

        let note = Note::On {
            value: NoteValue::A,
            octave: 4,
        };
        assert_eq!(note.to_midi_note(), Some(57));

        assert_eq!(Note::Off.to_midi_note(), None);
    }

    #[test]
    fn test_note_display() {
        let note = Note::On {
            value: NoteValue::Cs,
            octave: 5,
        };
        assert_eq!(note.display(), "C#5");
        assert_eq!(Note::Off.display(), "===");
    }

    #[test]
    fn test_cell_default_is_empty() {
        let cell = Cell::default();
        assert!(cell.is_empty());
        assert_eq!(cell.display_note(), "---");
        assert_eq!(cell.display_instrument(), "--");
        assert_eq!(cell.display_volume(), "--");
        assert_eq!(cell.display_effect(), "---");
    }

    #[test]
    fn test_pattern_new() {
        let pat = Pattern::new(64, 4);
        assert_eq!(pat.rows, 64);
        assert_eq!(pat.channels, 4);
        assert_eq!(pat.data.len(), 64);
        assert_eq!(pat.data[0].len(), 4);
        assert!(pat.get(0, 0).is_empty());
    }

    #[test]
    fn test_pattern_insert_row() {
        let mut pat = Pattern::new(4, 2);
        pat.set_cell(
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        pat.set_cell(
            1,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::D,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );

        pat.insert_row(1);
        assert_eq!(pat.rows, 4); // length unchanged
        assert_eq!(pat.data.len(), 4);
        assert_eq!(
            pat.get(0, 0).note,
            Some(Note::On {
                value: NoteValue::C,
                octave: 4
            })
        );
        assert!(pat.get(1, 0).note.is_none()); // new empty row
        assert_eq!(
            pat.get(2, 0).note,
            Some(Note::On {
                value: NoteValue::D,
                octave: 4
            })
        );
    }

    #[test]
    fn test_pattern_delete_row() {
        let mut pat = Pattern::new(4, 2);
        pat.set_cell(
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        pat.set_cell(
            1,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::D,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        pat.set_cell(
            2,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::E,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );

        pat.delete_row(1);
        assert_eq!(pat.rows, 4); // length unchanged
        assert_eq!(pat.data.len(), 4);
        assert_eq!(
            pat.get(0, 0).note,
            Some(Note::On {
                value: NoteValue::C,
                octave: 4
            })
        );
        assert_eq!(
            pat.get(1, 0).note,
            Some(Note::On {
                value: NoteValue::E,
                octave: 4
            })
        );
        assert!(pat.get(3, 0).note.is_none()); // appended empty row
    }

    #[test]
    fn test_pattern_resize_rows() {
        let mut pat = Pattern::new(8, 2);
        pat.set_cell(
            5,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::A,
                    octave: 3,
                }),
                ..Cell::default()
            },
        );

        // Shrink
        pat.resize_rows(4);
        assert_eq!(pat.rows, 4);
        assert_eq!(pat.data.len(), 4);

        // Grow
        pat.resize_rows(10);
        assert_eq!(pat.rows, 10);
        assert_eq!(pat.data.len(), 10);
        assert!(pat.get(9, 0).note.is_none());
    }

    #[test]
    fn test_pattern_set_and_get() {
        let mut pat = Pattern::new(64, 4);
        let cell = Cell {
            note: Some(Note::On {
                value: NoteValue::E,
                octave: 3,
            }),
            instrument: Some(1),
            volume: Some(0x40),
            effect: None,
            effect_value: None,
        };
        pat.set_cell(10, 2, cell);
        let got = pat.get(10, 2);
        assert_eq!(
            got.note,
            Some(Note::On {
                value: NoteValue::E,
                octave: 3
            })
        );
        assert_eq!(got.instrument, Some(1));
        assert_eq!(got.volume, Some(0x40));
    }

    #[test]
    fn test_note_transposed() {
        let c4 = Note::On {
            value: NoteValue::C,
            octave: 4,
        };
        // Transpose up 7 semitones: C4 -> G4
        let g4 = c4.transposed(7);
        assert_eq!(g4.to_midi_note(), Some(55));
        match g4 {
            Note::On { value, octave } => {
                assert_eq!(value, NoteValue::G);
                assert_eq!(octave, 4);
            }
            _ => panic!("expected Note::On"),
        }

        // Transpose down 3 semitones: C4 -> A3
        let a3 = c4.transposed(-3);
        assert_eq!(a3.to_midi_note(), Some(45));

        // Transpose Note::Off is a no-op
        assert_eq!(Note::Off.transposed(5), Note::Off);

        // Out of range: refused, not clamped.
        let high = Note::On {
            value: NoteValue::G,
            octave: 10,
        };
        assert_eq!(high.transposed(12), high, "should not clamp onto 127");
        let low = Note::On {
            value: NoteValue::C,
            octave: 0,
        };
        assert_eq!(low.transposed(-1), low, "should not clamp onto 0");

        // A transpose that lands exactly on the boundary is allowed.
        let g9 = Note::On {
            value: NoteValue::G,
            octave: 9,
        };
        assert_eq!(g9.transposed(12).to_midi_note(), Some(127));
    }
}
