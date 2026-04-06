use serde::{Deserialize, Serialize};

use crate::constants::{MIDI_MAX_NOTE, SEMITONES_PER_OCTAVE};

/// A musical note pitch (C, C#, D, ... B)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteValue {
    C,
    Cs,
    D,
    Ds,
    E,
    F,
    Fs,
    G,
    Gs,
    A,
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
    pub fn transposed(self, semitones: i8) -> Note {
        match self {
            Note::On { value, octave } => {
                let midi = (octave as i16) * SEMITONES_PER_OCTAVE as i16
                    + value.to_index() as i16
                    + semitones as i16;
                let midi = midi.clamp(0, MIDI_MAX_NOTE as i16) as u8;
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

/// A single cell in the tracker grid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cell {
    pub note: Option<Note>,
    pub instrument: Option<u8>,
    pub volume: Option<u8>,
    pub effect: Option<u8>,
    pub effect_value: Option<u8>,
}

impl Cell {
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
// Phrase (single-channel note data)
// ---------------------------------------------------------------------------

/// A phrase is a single-channel column of note data.
/// This is the atomic unit in the Song > Chain > Phrase hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phrase {
    pub rows: usize,
    pub data: Vec<Cell>, // data[row], single channel
}

impl Phrase {
    pub fn new(rows: usize) -> Self {
        Self {
            rows,
            data: vec![Cell::default(); rows],
        }
    }

    pub fn get(&self, row: usize) -> &Cell {
        &self.data[row]
    }

    pub fn get_mut(&mut self, row: usize) -> &mut Cell {
        &mut self.data[row]
    }

    pub fn set_cell(&mut self, row: usize, cell: Cell) {
        self.data[row] = cell;
    }

    pub fn insert_row(&mut self, at: usize) {
        if at >= self.rows {
            return;
        }
        self.data.insert(at, Cell::default());
        self.data.truncate(self.rows);
    }

    pub fn delete_row(&mut self, at: usize) {
        if at >= self.rows {
            return;
        }
        self.data.remove(at);
        self.data.push(Cell::default());
    }
}

// ---------------------------------------------------------------------------
// Chain (sequence of phrase references for one channel)
// ---------------------------------------------------------------------------

/// One entry in a chain: play a phrase with optional semitone transpose.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ChainEntry {
    pub phrase: usize, // index into Song::phrases
    pub transpose: i8, // semitones
}

/// A chain is a sequence of phrase references for one channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chain {
    pub entries: Vec<ChainEntry>,
}

// ---------------------------------------------------------------------------
// Pattern (multi-channel grid, kept for rendering and backwards compat)
// ---------------------------------------------------------------------------

/// A pattern is a grid of rows x channels
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

        // Clamp to valid range
        let high = Note::On {
            value: NoteValue::G,
            octave: 10,
        };
        let clamped = high.transposed(12);
        assert_eq!(clamped.to_midi_note(), Some(127));
    }

    #[test]
    fn test_phrase_new() {
        let phrase = Phrase::new(16);
        assert_eq!(phrase.rows, 16);
        assert_eq!(phrase.data.len(), 16);
        assert!(phrase.get(0).is_empty());
    }

    #[test]
    fn test_phrase_set_and_get() {
        let mut phrase = Phrase::new(4);
        let cell = Cell {
            note: Some(Note::On {
                value: NoteValue::E,
                octave: 5,
            }),
            ..Cell::default()
        };
        phrase.set_cell(1, cell);
        assert_eq!(phrase.get(1).note, cell.note);
        assert!(phrase.get(0).is_empty());
    }

    #[test]
    fn test_phrase_insert_delete_row() {
        let mut phrase = Phrase::new(4);
        phrase.set_cell(
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        // Insert pushes row 0 down to row 1
        phrase.insert_row(0);
        assert!(phrase.get(0).is_empty());
        assert!(phrase.get(1).note.is_some());
        assert_eq!(phrase.data.len(), 4); // length preserved

        // Delete row 0 pulls row 1 back to row 0
        phrase.delete_row(0);
        assert!(phrase.get(0).note.is_some());
        assert_eq!(phrase.data.len(), 4);
    }
}
