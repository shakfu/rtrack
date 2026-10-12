use serde::{Deserialize, Serialize};

use super::fx::{Fx, FxCmd};
use crate::constants::{EFFECT_SET_SPEED, MIDI_MAX_NOTE, SEMITONES_PER_OCTAVE};
use crate::theory::ScaleSetting;

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

/// Pitch names by semitone: plain, then padded to two columns for the pattern
/// grid. One table so the two spellings cannot drift apart.
const PITCH_NAMES: [(&str, &str); 12] = [
    ("C", "C-"),
    ("C#", "C#"),
    ("D", "D-"),
    ("D#", "D#"),
    ("E", "E-"),
    ("F", "F-"),
    ("F#", "F#"),
    ("G", "G-"),
    ("G#", "G#"),
    ("A", "A-"),
    ("A#", "A#"),
    ("B", "B-"),
];

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

    /// Plain name, e.g. `C` or `C#`.
    pub fn name(self) -> &'static str {
        PITCH_NAMES[self.to_index() as usize].0
    }

    /// Name padded to two columns for the pattern grid, e.g. `C-` or `C#`.
    pub fn display_name(self) -> &'static str {
        PITCH_NAMES[self.to_index() as usize].1
    }

    /// Parse a plain name. Case-insensitive; `Db`-style flats are not accepted.
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_uppercase();
        PITCH_NAMES
            .iter()
            .position(|(plain, _)| *plain == s)
            .and_then(|i| Self::from_index(i as u8))
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

    /// Build a note from a MIDI note number, or `None` above 127.
    pub fn from_midi(midi: u8) -> Option<Note> {
        if midi > MIDI_MAX_NOTE {
            return None;
        }
        NoteValue::from_index(midi % SEMITONES_PER_OCTAVE).map(|value| Note::On {
            value,
            octave: midi / SEMITONES_PER_OCTAVE,
        })
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
                Note::from_midi(midi as u8).unwrap_or(self)
            }
            Note::Off => Note::Off,
        }
    }
}

/// Name a MIDI note number for display, e.g. `60` -> `C4`. Out-of-range input
/// is named by its pitch class rather than refused, so callers holding a raw
/// `u8` (sample base note, MIDI mapping) need no fallback.
pub fn midi_note_name(midi: u8) -> String {
    let value = NoteValue::from_index(midi % SEMITONES_PER_OCTAVE).unwrap_or(NoteValue::C);
    format!("{}{}", value.name(), midi / SEMITONES_PER_OCTAVE)
}

/// Effect columns per cell.
pub const FX_COLUMNS: usize = 2;

/// A single cell in the tracker grid.
///
/// Unset fields are omitted when serialized. Tracker patterns are mostly
/// empty, and spelling out five nulls per cell dominated the file: a 4x16
/// pattern with one note cost about 5.9 KB before this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "CellRepr", into = "CellRepr")]
pub struct Cell {
    pub note: Option<Note>,
    pub instrument: Option<u8>,
    pub volume: Option<u8>,
    pub fx: [Option<Fx>; FX_COLUMNS],
}

/// A cell as stored: the effect columns as a list trimmed of trailing empty
/// ones, or the single-digit `effect`/`effect_value` of versions 0 and 1.
#[derive(Serialize, Deserialize)]
struct CellRepr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instrument: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fx: Vec<Option<Fx>>,
    #[serde(default, skip_serializing)]
    effect: Option<u8>,
    #[serde(default, skip_serializing)]
    effect_value: Option<u8>,
}

impl TryFrom<CellRepr> for Cell {
    type Error = String;

    fn try_from(r: CellRepr) -> Result<Self, Self::Error> {
        if r.fx.len() > FX_COLUMNS {
            return Err(format!(
                "a cell has {} effect columns; at most {FX_COLUMNS} are supported",
                r.fx.len()
            ));
        }
        let mut cell = Cell {
            note: r.note,
            instrument: r.instrument,
            volume: r.volume,
            fx: [None; FX_COLUMNS],
        };
        if r.fx.is_empty() {
            let (fx, consumed) = Fx::from_legacy(r.effect, r.effect_value, r.instrument);
            cell.fx[0] = fx;
            if consumed {
                cell.instrument = None;
            }
        } else {
            cell.fx[..r.fx.len()].copy_from_slice(&r.fx);
        }
        Ok(cell)
    }
}

impl From<Cell> for CellRepr {
    fn from(cell: Cell) -> Self {
        let used = cell
            .fx
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |i| i + 1);
        CellRepr {
            note: cell.note,
            instrument: cell.instrument,
            volume: cell.volume,
            fx: cell.fx[..used].to_vec(),
            effect: None,
            effect_value: None,
        }
    }
}

impl Cell {
    /// Transpose this cell's note in place, if it has one.
    ///
    /// `steps` counts scale degrees when a scale is given and semitones
    /// otherwise. Shared by both frontends' transpose commands so they cannot
    /// drift apart on edge cases like the ends of the MIDI range.
    pub fn transpose_note(&mut self, steps: i8, scale: Option<ScaleSetting>) {
        let Some(note) = self.note else { return };
        self.note = Some(match (scale, note.to_midi_note()) {
            (Some(scale), Some(midi)) => {
                Note::from_midi(scale.transpose(midi, steps as i16)).unwrap_or(note)
            }
            _ => note.transposed(steps),
        });
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.note.is_none()
            && self.instrument.is_none()
            && self.volume.is_none()
            && self.fx.iter().all(Option::is_none)
    }

    /// Type one hex digit into effect column `col`.
    ///
    /// An empty column takes the digit as an old single-digit command (`4`
    /// is `PVB`); later digits shift into the parameter. `C` takes its
    /// controller from the instrument column, as `Cxx` did. Shared by both
    /// frontends so their entry cannot drift.
    pub fn enter_effect_digit(&mut self, col: usize, digit: u8) {
        match &mut self.fx[col] {
            None => {
                let Some(cmd) = FxCmd::from_digit(digit, 0, self.instrument) else {
                    return;
                };
                if matches!(cmd, FxCmd::Cc(_)) {
                    self.instrument = None;
                }
                self.fx[col] = Some(Fx::new(cmd, 0));
            }
            Some(fx) => {
                fx.param = (fx.param << 4) | (digit & 0x0F);
                // `F` meant speed below 0x20 and tempo from it.
                if matches!(fx.cmd, FxCmd::Ticks | FxCmd::Tempo) {
                    fx.cmd = FxCmd::from_digit(EFFECT_SET_SPEED, fx.param, None).unwrap_or(fx.cmd);
                }
            }
        }
    }

    /// Format the cell for display: "C-4 01 80 PVB44"
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

    /// Effect column `col`, e.g. "PVB44".
    pub fn display_effect(&self, col: usize) -> String {
        match self.fx[col] {
            Some(fx) => fx.to_string(),
            None => "-----".to_string(),
        }
    }

    /// Set effect column `col` from its text form, e.g. "PVB44" or "C0740".
    /// Empty text clears it.
    pub fn set_effect_text(&mut self, col: usize, text: &str) -> Result<(), String> {
        let text = text.trim();
        self.fx[col] = if text.is_empty() {
            None
        } else {
            Some(text.parse()?)
        };
        Ok(())
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
    /// Per channel, the phrase that column plays instead of its own cells.
    /// May be shorter than `channels`; a missing entry is no link.
    #[serde(default, skip_serializing_if = "no_links")]
    pub links: Vec<Option<super::PhraseLink>>,
}

fn no_links(links: &[Option<super::PhraseLink>]) -> bool {
    links.iter().all(Option::is_none)
}

impl Pattern {
    pub fn new(rows: usize, channels: usize) -> Self {
        Self {
            rows,
            channels,
            data: vec![vec![Cell::default(); channels]; rows],
            links: Vec::new(),
        }
    }

    /// The phrase link of `channel`, if it has one.
    pub fn link(&self, channel: usize) -> Option<super::PhraseLink> {
        self.links.get(channel).copied().flatten()
    }

    /// Roughly how much heap this pattern holds.
    ///
    /// Used to budget the undo history by bytes. An estimate on purpose: it
    /// counts the cell storage and the per-row `Vec` headers, which together
    /// are all but a rounding error of the real figure, and does not chase
    /// allocator overhead.
    pub fn approx_heap_bytes(&self) -> usize {
        let row_header = std::mem::size_of::<Vec<Cell>>();
        let cells: usize = self
            .data
            .iter()
            .map(|row| row.capacity() * std::mem::size_of::<Cell>())
            .sum();
        cells
            + self.data.capacity() * row_header
            + self.links.capacity() * std::mem::size_of::<Option<super::PhraseLink>>()
    }

    /// Whether any cell uses the second effect column.
    pub fn uses_second_fx(&self) -> bool {
        self.data.iter().flatten().any(|c| c.fx[1].is_some())
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
        self.links.truncate(channels);
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
    fn from_midi_inverts_to_midi_note() {
        for midi in 0..=MIDI_MAX_NOTE {
            let note = Note::from_midi(midi).expect("in range");
            assert_eq!(note.to_midi_note(), Some(midi));
        }
        assert_eq!(Note::from_midi(128), None);
    }

    #[test]
    fn midi_note_names_match_the_grid_spelling() {
        assert_eq!(midi_note_name(60), "C5");
        assert_eq!(midi_note_name(61), "C#5");
        assert_eq!(midi_note_name(0), "C0");
        // Same pitch class as the padded grid name, minus the padding.
        for midi in 0..=MIDI_MAX_NOTE {
            let value = NoteValue::from_index(midi % SEMITONES_PER_OCTAVE).unwrap();
            assert!(midi_note_name(midi).starts_with(value.name()));
            assert_eq!(value.display_name().trim_end_matches('-'), value.name());
        }
    }

    #[test]
    fn test_cell_default_is_empty() {
        let cell = Cell::default();
        assert!(cell.is_empty());
        assert_eq!(cell.display_note(), "---");
        assert_eq!(cell.display_instrument(), "--");
        assert_eq!(cell.display_volume(), "--");
        assert_eq!(cell.display_effect(0), "-----");
    }

    #[test]
    fn effect_columns_save_trimmed_and_load_back() {
        let mut cell = Cell::default();
        assert_eq!(serde_json::to_string(&cell).unwrap(), "{}");

        cell.fx[1] = Some(Fx::new(FxCmd::Arp, 0x37));
        let json = serde_json::to_string(&cell).unwrap();
        assert_eq!(json, r#"{"fx":[null,"ARP37"]}"#);
        assert_eq!(serde_json::from_str::<Cell>(&json).unwrap(), cell);

        cell.fx[1] = None;
        cell.fx[0] = Some(Fx::new(FxCmd::Vibrato, 0x44));
        assert_eq!(serde_json::to_string(&cell).unwrap(), r#"{"fx":["PVB44"]}"#);
    }

    #[test]
    fn a_cell_with_too_many_effect_columns_is_refused() {
        let err = serde_json::from_str::<Cell>(r#"{"fx":["ARP37",null,"PVB44"]}"#).unwrap_err();
        assert!(err.to_string().contains("effect columns"), "{err}");
    }

    #[test]
    fn new_effect_columns_win_over_legacy_fields() {
        let cell: Cell =
            serde_json::from_str(r#"{"fx":["ARP37"],"effect":4,"effect_value":68}"#).unwrap();
        assert_eq!(cell.fx[0], Some(Fx::new(FxCmd::Arp, 0x37)));
    }

    #[test]
    fn effect_digits_enter_as_before() {
        let mut cell = Cell::default();
        for d in [0x4, 0x4, 0x2] {
            cell.enter_effect_digit(0, d);
        }
        assert_eq!(cell.display_effect(0), "PVB42");

        // `F` picks speed or tempo from the parameter as it is typed.
        let mut cell = Cell::default();
        cell.enter_effect_digit(0, 0xF);
        cell.enter_effect_digit(0, 0x1);
        assert_eq!(cell.display_effect(0), "TIC01");
        cell.enter_effect_digit(0, 0x8);
        assert_eq!(cell.display_effect(0), "TIC18");
        cell.enter_effect_digit(0, 0x0);
        assert_eq!(cell.display_effect(0), "TPO80");

        // `C` takes its controller from the instrument column.
        let mut cell = Cell {
            instrument: Some(7),
            ..Cell::default()
        };
        cell.enter_effect_digit(0, 0xC);
        assert_eq!(cell.display_effect(0), "C0700");
        assert_eq!(cell.instrument, None);

        // `A` was never a command.
        let mut cell = Cell::default();
        cell.enter_effect_digit(0, 0xA);
        assert_eq!(cell.fx[0], None);
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
            fx: [None, None],
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
