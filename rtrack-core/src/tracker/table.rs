//! Tables: per-note sequences of pitch, volume and one effect, read while a
//! note sounds (`docs/dev/g7-tables.md`).

use serde::{Deserialize, Serialize};

use super::{Fx, FxCmd};

/// Most rows in a table, and most ticks a row lasts.
pub const MAX_TABLE_ROWS: usize = 16;
pub const MAX_TABLE_TICKS: u8 = 16;

/// A table, played from row 0 with each note of the instrument with its
/// number, or of a `TBL` command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    #[serde(default)]
    pub name: String,
    /// Ticks each row lasts, 1 to `MAX_TABLE_TICKS`.
    #[serde(default = "one")]
    pub ticks_per_row: u8,
    /// After the last row, play again from this row; `None` stops at the
    /// last row and holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_from: Option<u8>,
    /// 1 to `MAX_TABLE_ROWS` rows.
    pub rows: Vec<TableRow>,
}

fn one() -> u8 {
    1
}

/// One step of a table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRow {
    /// Semitones added to the note while this row plays.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub transpose: i8,
    /// Volume while this row plays; `None` keeps the current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<Fx>,
}

fn is_zero(v: &i8) -> bool {
    *v == 0
}

impl Table {
    /// A table of `rows` empty rows, one tick each, not looping.
    pub fn new(rows: usize) -> Self {
        Self {
            name: String::new(),
            ticks_per_row: 1,
            loop_from: None,
            rows: vec![TableRow::default(); rows.clamp(1, MAX_TABLE_ROWS)],
        }
    }

    /// Whether a table row may carry `cmd`: the continuous effects, `Cnn`
    /// and `PRG`. Song commands belong to the song; `DEL`, `CHA` and `OFS`
    /// describe how a note starts, which a table cannot change once it runs;
    /// `PSL`, `RND` and `TBL` need a target or history a table does not have.
    pub fn allows(cmd: FxCmd) -> bool {
        matches!(
            cmd,
            FxCmd::Arp
                | FxCmd::PitchUp
                | FxCmd::PitchDown
                | FxCmd::Vibrato
                | FxCmd::VolSlide
                | FxCmd::Cc(_)
                | FxCmd::Program
        )
    }

    /// Set row `row`'s effect from its text form, e.g. "PVB44"; empty text
    /// clears it. Refuses a command a table cannot play.
    pub fn set_row_effect(&mut self, row: usize, text: &str) -> Result<(), String> {
        let text = text.trim();
        let fx = if text.is_empty() {
            None
        } else {
            let fx: Fx = text.parse()?;
            if !Self::allows(fx.cmd) {
                return Err(format!(
                    "{} cannot play in a table; tables take ARP, PUP, PDN, PVB, VSL, Cnn and PRG",
                    fx.cmd.name()
                ));
            }
            Some(fx)
        };
        let slot = self.rows.get_mut(row).ok_or("no such row")?;
        slot.fx = fx;
        Ok(())
    }

    /// Set the number of rows, within 1 to `MAX_TABLE_ROWS`, padding with
    /// empty rows. A loop from a row that no longer exists becomes a stop.
    pub fn resize(&mut self, rows: usize) {
        self.rows
            .resize(rows.clamp(1, MAX_TABLE_ROWS), TableRow::default());
        if self
            .loop_from
            .is_some_and(|r| r as usize >= self.rows.len())
        {
            self.loop_from = None;
        }
    }

    /// Bring a table loaded from a file inside its limits. Returns a note
    /// for each change.
    pub fn repair(&mut self, number: u8) -> Vec<String> {
        let mut repairs = Vec::new();
        if self.rows.is_empty() {
            self.rows.push(TableRow::default());
            repairs.push(format!("table {number:02X} had no rows, given one"));
        }
        if self.rows.len() > MAX_TABLE_ROWS {
            repairs.push(format!(
                "table {number:02X} had {} rows, truncated to {MAX_TABLE_ROWS}",
                self.rows.len()
            ));
            self.rows.truncate(MAX_TABLE_ROWS);
        }
        let ticks = self.ticks_per_row.clamp(1, MAX_TABLE_TICKS);
        if ticks != self.ticks_per_row {
            repairs.push(format!(
                "table {number:02X} rows lasted {} ticks, set to {ticks}",
                self.ticks_per_row
            ));
            self.ticks_per_row = ticks;
        }
        if self
            .loop_from
            .is_some_and(|r| r as usize >= self.rows.len())
        {
            self.loop_from = None;
            repairs.push(format!(
                "table {number:02X} looped from a missing row, now stops"
            ));
        }
        for (i, row) in self.rows.iter_mut().enumerate() {
            if row.fx.is_some_and(|fx| !Self::allows(fx.cmd)) {
                row.fx = None;
                repairs.push(format!(
                    "table {number:02X} row {i:X} had a command tables cannot play, removed"
                ));
            }
        }
        repairs
    }
}

/// Serde for `Song.tables`: a list of tables, each with its number.
pub(crate) mod by_slot {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Table;

    #[derive(Serialize, Deserialize)]
    struct Entry {
        slot: u8,
        #[serde(flatten)]
        table: Table,
    }

    pub fn serialize<S: Serializer>(
        tables: &BTreeMap<u8, Table>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let entries: Vec<Entry> = tables
            .iter()
            .map(|(&slot, table)| Entry {
                slot,
                table: table.clone(),
            })
            .collect();
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<u8, Table>, D::Error> {
        let entries = Vec::<Entry>::deserialize(deserializer)?;
        Ok(entries.into_iter().map(|e| (e.slot, e.table)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Song;

    #[test]
    fn tables_save_by_number_and_round_trip() {
        let mut song = Song::new(1, 4);
        let mut table = Table::new(3);
        table.name = "arp".to_string();
        table.loop_from = Some(0);
        table.rows[1].transpose = 4;
        table.rows[2].transpose = 7;
        table.rows[0].volume = Some(0x60);
        table.rows[2].fx = Some(Fx::new(FxCmd::Cc(74), 0x20));
        song.tables.insert(5, table);

        // Through `SongFile`, which flattens the song, as a saved file is.
        let file = crate::tracker::SongFile::from_song(song.clone());
        let json = file.to_json().unwrap();
        assert!(json.contains(r#""slot": 5"#), "{json}");
        let back: crate::tracker::SongFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.song.tables, song.tables);
    }

    #[test]
    fn a_song_without_tables_writes_no_table_field() {
        let json = serde_json::to_string(&Song::new(1, 4)).unwrap();
        assert!(!json.contains("tables"), "{json}");
    }

    #[test]
    fn a_table_from_a_file_is_brought_inside_its_limits() {
        let mut empty = Table::new(1);
        empty.rows.clear();
        empty.ticks_per_row = 0;
        assert_eq!(empty.repair(0).len(), 2);
        assert_eq!((empty.rows.len(), empty.ticks_per_row), (1, 1));

        let mut big = Table::new(1);
        big.rows = vec![TableRow::default(); 40];
        big.ticks_per_row = 99;
        big.loop_from = Some(30);
        big.rows[3].fx = Some(Fx::new(FxCmd::Jump, 0));
        big.rows[4].fx = Some(Fx::new(FxCmd::Vibrato, 0x44));
        let repairs = big.repair(1);
        assert_eq!(repairs.len(), 4, "{repairs:?}");
        assert_eq!(big.rows.len(), MAX_TABLE_ROWS);
        assert_eq!(big.ticks_per_row, MAX_TABLE_TICKS);
        assert_eq!(big.loop_from, None, "row 30 is gone");
        assert_eq!(big.rows[3].fx, None, "a song command");
        assert!(big.rows[4].fx.is_some(), "a note command stays");
    }

    #[test]
    fn a_row_effect_is_set_by_name_and_song_commands_are_refused() {
        let mut t = Table::new(2);
        t.set_row_effect(1, "pvb44").unwrap();
        assert_eq!(t.rows[1].fx, Some(Fx::new(FxCmd::Vibrato, 0x44)));
        let err = t.set_row_effect(1, "JMP00").unwrap_err();
        assert!(err.contains("JMP"), "{err}");
        assert_eq!(
            t.rows[1].fx,
            Some(Fx::new(FxCmd::Vibrato, 0x44)),
            "unchanged"
        );
        t.set_row_effect(1, "").unwrap();
        assert_eq!(t.rows[1].fx, None);
        assert!(t.set_row_effect(5, "ARP37").is_err());
    }

    #[test]
    fn resizing_drops_a_loop_from_a_removed_row() {
        let mut t = Table::new(4);
        t.loop_from = Some(3);
        t.resize(2);
        assert_eq!((t.rows.len(), t.loop_from), (2, None));
        t.resize(99);
        assert_eq!(t.rows.len(), MAX_TABLE_ROWS);
    }

    #[test]
    fn song_repair_repairs_its_tables() {
        let mut song = Song::new(1, 4);
        let mut t = Table::new(1);
        t.ticks_per_row = 0;
        song.tables.insert(9, t);
        assert_eq!(song.repair().len(), 1);
        assert_eq!(song.tables[&9].ticks_per_row, 1);
    }
}
