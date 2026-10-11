//! Tables: short per-note sequences that run beside the pattern.
//!
//! A table starts with each note of an instrument that uses it, or of a note
//! with `TBL` on its row, and loops while the note sounds. Each row can
//! transpose the note, scale its volume, and run one effect. The table moves
//! on one row every [`Table::tic`] engine ticks, independent of the pattern's
//! speed (M8 model, `docs/dev/design-gaps.md` G7).

use serde::{Deserialize, Serialize};

use super::fx::{Fx, FxCmd};

/// Rows in a table.
pub const TABLE_ROWS: usize = 16;

/// Most tables a song can hold, so `TBL xx` and an instrument's table index
/// both fit in a byte.
pub const MAX_TABLES: usize = 256;

/// One table row. Every field is optional; an empty row holds the previous
/// row's transpose and volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRow {
    /// Semitones added to the note while this row is current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i8>,
    /// Volume scale, 0x00-0x7F, where 0x7F leaves the note's volume alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    /// Applied when the row is entered. Continuous effects run until the
    /// table moves on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<Fx>,
}

impl TableRow {
    pub fn is_empty(&self) -> bool {
        *self == TableRow::default()
    }
}

/// A table: [`TABLE_ROWS`] rows, of which the first `length` play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub rows: [TableRow; TABLE_ROWS],
    /// Rows played before looping to the first, 1-16.
    #[serde(default = "default_length")]
    pub length: u8,
    /// Engine ticks per table row, 1 or more.
    #[serde(default = "default_tic")]
    pub tic: u8,
}

fn default_length() -> u8 {
    TABLE_ROWS as u8
}

fn default_tic() -> u8 {
    1
}

impl Default for Table {
    fn default() -> Self {
        Self {
            rows: [TableRow::default(); TABLE_ROWS],
            length: default_length(),
            tic: default_tic(),
        }
    }
}

impl Table {
    /// Clamp `length` and `tic` into range. Returns true if either changed.
    pub fn repair(&mut self) -> bool {
        let (length, tic) = (self.length, self.tic);
        self.length = self.length.clamp(1, TABLE_ROWS as u8);
        self.tic = self.tic.max(1);
        (length, tic) != (self.length, self.tic)
    }
}

/// Whether `cmd` can run from a table row.
///
/// Song-structure and note-trigger commands cannot: a table has no row
/// position to jump from, and no note of its own to delay, roll for, offset
/// or glide to.
pub fn allowed_in_table(cmd: FxCmd) -> bool {
    !matches!(
        cmd,
        FxCmd::Jump
            | FxCmd::Break
            | FxCmd::Speed
            | FxCmd::Tempo
            | FxCmd::Delay
            | FxCmd::Chance
            | FxCmd::Random
            | FxCmd::Offset
            | FxCmd::Portamento
            | FxCmd::Table
            | FxCmd::Chord
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_table_serializes_compactly_and_round_trips() {
        let mut table = Table::default();
        table.rows[2] = TableRow {
            transpose: Some(-12),
            volume: None,
            fx: Some(Fx::new(FxCmd::Vibrato, 0x44)),
        };
        let json = serde_json::to_string(&table).unwrap();
        assert!(json.contains(r#"{"transpose":-12,"fx":"VIB44"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Table>(&json).unwrap(), table);
    }

    #[test]
    fn repair_clamps_length_and_tic() {
        let mut table = Table {
            length: 0,
            tic: 0,
            ..Table::default()
        };
        assert!(table.repair());
        assert_eq!((table.length, table.tic), (1, 1));
        table.length = 40;
        assert!(table.repair());
        assert_eq!(table.length, 16);
        assert!(!table.repair());
    }
}
