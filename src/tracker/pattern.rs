/// A musical note pitch (C, C#, D, ... B)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
                let midi = (*octave as u16) * 12 + value.to_index() as u16;
                if midi <= 127 {
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
}

/// A single cell in the tracker grid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    pub note: Option<Note>,
    pub instrument: Option<u8>,
    pub volume: Option<u8>,
    pub effect: Option<u8>,
    pub effect_value: Option<u8>,
}

impl Cell {
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

/// A pattern is a grid of rows x channels
#[derive(Debug, Clone)]
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
        assert_eq!(got.note, Some(Note::On { value: NoteValue::E, octave: 3 }));
        assert_eq!(got.instrument, Some(1));
        assert_eq!(got.volume, Some(0x40));
    }
}
