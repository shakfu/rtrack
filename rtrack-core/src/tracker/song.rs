use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::{MAX_CHANNELS, MAX_ROWS_PER_PATTERN, MIDI_CLOCKS_PER_BEAT};

use super::{Cell, Pattern};
use crate::audio::effects::SendBusParams;
use crate::audio::synth::SynthParams;
use crate::fs::write_atomic;
use crate::types::{ChannelConfig, MidiCcMapping};

/// A tempo change point in the song
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoPoint {
    pub order: usize,
    pub row: usize,
    pub bpm: f64,
}

/// Serializable instrument definition (stored in .rtrk files)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentDef {
    pub name: String,
    // Every optional field needs `default` as well as `skip_serializing_if`:
    // without it, serde treats an omitted field as an error, so a file we
    // wrote ourselves would fail to load back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_program: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synth_params: Option<SynthParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_bend_range: Option<f64>,
}

/// Serializable sample reference (metadata only, no audio data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleRef {
    pub name: String,
    /// Path to the source audio file (relative to the .rtrk file)
    pub path: String,
    pub base_note: u8,
    #[serde(default)]
    pub trim_start: usize,
    #[serde(default)]
    pub trim_end: usize,
    #[serde(default)]
    pub loop_enabled: bool,
    #[serde(default)]
    pub loop_start: usize,
    #[serde(default)]
    pub loop_end: usize,
}

/// Extended song file format that includes instrument, sample, mixer and
/// MIDI-learn state.
///
/// Backwards-compatible: old .rtrk files without these fields still load fine
/// (serde default kicks in for the optional fields), and unknown fields
/// written by newer versions are ignored rather than rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongFile {
    /// Format version this file was written with. See [`FORMAT_VERSION`].
    #[serde(default)]
    pub version: u32,
    #[serde(flatten)]
    pub song: Song,
    /// Instrument definitions (only non-empty ones are stored)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruments: Vec<InstrumentEntry>,
    /// Sample file references with metadata
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_refs: Vec<SampleRefEntry>,
    /// Per-channel mixer state: type, routing, volume, pan, effects.
    /// Indexed positionally, one entry per song channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels_config: Vec<ChannelConfig>,
    /// Send/return bus settings, indexed positionally.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub send_buses: Vec<SendBusParams>,
    /// MIDI CC -> parameter mappings created via MIDI learn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub midi_cc_mappings: Vec<MidiCcMapping>,
}

/// Instrument entry keyed by slot index (for sparse storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentEntry {
    pub slot: usize,
    #[serde(flatten)]
    pub def: InstrumentDef,
}

/// Sample reference entry keyed by slot index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleRefEntry {
    pub slot: usize,
    #[serde(flatten)]
    pub sample_ref: SampleRef,
}

/// Current .rtrk format version, written into every saved file.
///
/// Files predating the field deserialize as version 0. Nothing branches on
/// this yet; it exists so that a future format change has something to
/// branch on, which is not something that can be added retroactively.
pub const FORMAT_VERSION: u32 = 1;

impl SongFile {
    /// True if this file was written by a newer version of rtrack than this
    /// one understands. Such a file is still loaded -- unknown fields are
    /// ignored -- but the caller should say so rather than silently dropping
    /// whatever it could not represent.
    pub fn is_from_newer_version(&self) -> bool {
        self.version > FORMAT_VERSION
    }

    /// Wrap a song with no instrument, sample, mixer or MIDI-learn state.
    /// Use struct-update syntax to fill in the parts you care about:
    /// `SongFile { instruments, ..SongFile::from_song(song) }`.
    pub fn from_song(song: Song) -> Self {
        Self {
            version: FORMAT_VERSION,
            song,
            instruments: Vec::new(),
            sample_refs: Vec::new(),
            channels_config: Vec::new(),
            send_buses: Vec::new(),
            midi_cc_mappings: Vec::new(),
        }
    }

    /// Serialize to the JSON text that [`SongFile::save`] would write.
    ///
    /// Split out so callers that need the bytes without touching the
    /// filesystem -- comparing against a committed file, for instance -- do
    /// not have to reproduce the formatting.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to serialize song file")
    }

    /// Write the song, replacing any existing file atomically.
    ///
    /// See [`write_atomic`] for the durability and temp-file handling this
    /// relies on.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        write_atomic(path, json.as_bytes())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let song_file: SongFile =
            serde_json::from_str(&data).context("Failed to parse song file")?;
        Ok(song_file)
    }
}

/// A song is a collection of patterns with an order list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub title: String,
    pub bpm: u16,
    pub speed: u8, // ticks per row
    pub patterns: Vec<Pattern>,
    pub order: Vec<usize>, // indices into patterns
    /// Per-order-entry repeat count: 0=skip, 1=play once (default), 2+=repeat
    #[serde(default)]
    pub order_repeats: Vec<u8>,
    pub channels: usize,
    pub rows_per_pattern: usize,
    /// Row highlight interval for beats (default 4)
    #[serde(default = "default_highlight_beat")]
    pub highlight_beat: usize,
    /// Row highlight interval for bars (default 16)
    #[serde(default = "default_highlight_bar")]
    pub highlight_bar: usize,
    /// Swing amount: 50 = none, 0-100 (even rows get swing% of pair time, odd rows get rest)
    #[serde(default = "default_swing")]
    pub swing: u8,
    /// Tempo automation points (order, row, bpm)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_map: Vec<TempoPoint>,
}

fn default_highlight_beat() -> usize {
    4
}
fn default_highlight_bar() -> usize {
    16
}
fn default_swing() -> u8 {
    50
}

impl Song {
    pub fn new(channels: usize, rows_per_pattern: usize) -> Self {
        let initial_pattern = Pattern::new(rows_per_pattern, channels);
        Self {
            title: "Untitled".to_string(),
            bpm: 120,
            speed: 6,
            patterns: vec![initial_pattern],
            order: vec![0],
            order_repeats: vec![1],
            channels,
            rows_per_pattern,
            highlight_beat: 4,
            highlight_bar: 16,
            swing: 50,
            tempo_map: Vec::new(),
        }
    }

    /// Ensure order_repeats matches order length (for backwards compat with old files)
    pub fn sync_order_repeats(&mut self) {
        self.order_repeats.resize(self.order.len(), 1);
    }

    pub fn current_pattern_count(&self) -> usize {
        self.patterns.len()
    }

    pub fn add_pattern(&mut self) -> usize {
        let idx = self.patterns.len();
        self.patterns
            .push(Pattern::new(self.rows_per_pattern, self.channels));
        idx
    }

    #[allow(dead_code)]
    pub fn get_pattern(&self, index: usize) -> Option<&Pattern> {
        self.patterns.get(index)
    }

    #[allow(dead_code)]
    pub fn get_pattern_mut(&mut self, index: usize) -> Option<&mut Pattern> {
        self.patterns.get_mut(index)
    }

    /// Write just the song data, replacing any existing file atomically.
    ///
    /// Shares [`write_atomic`] with [`SongFile::save`], so both paths get the
    /// same durability and temp-file guarantees.
    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("Failed to serialize song")?;
        write_atomic(path, json.as_bytes())
    }

    #[allow(dead_code)]
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let song: Song = serde_json::from_str(&data).context("Failed to parse song file")?;
        Ok(song)
    }

    /// Seconds per row based on BPM and speed
    #[allow(dead_code)]
    pub fn seconds_per_row(&self) -> f64 {
        self.seconds_per_tick() * self.speed as f64
    }

    /// Seconds per single tick (sub-row). Classic tracker: tick rate = BPM * 24 / 60.
    pub fn seconds_per_tick(&self) -> f64 {
        let ticks_per_second = (self.bpm as f64 * MIDI_CLOCKS_PER_BEAT) / 60.0;
        1.0 / ticks_per_second
    }

    /// Seconds per tick with swing applied. Even rows get swing% of a pair, odd rows get the rest.
    pub fn swing_seconds_per_tick(&self, row: usize) -> f64 {
        let base = self.seconds_per_tick();
        if self.swing == 50 {
            return base;
        }
        let swing_f = self.swing as f64;
        if row.is_multiple_of(2) {
            base * swing_f / 50.0
        } else {
            base * (100.0 - swing_f) / 50.0
        }
    }

    /// Look up a tempo automation point at the given position.
    pub fn tempo_at(&self, order: usize, row: usize) -> Option<f64> {
        self.tempo_map
            .iter()
            .find(|tp| tp.order == order && tp.row == row)
            .map(|tp| tp.bpm)
    }

    // -------------------------------------------------------------------
    // Cell access
    //
    // `patterns` + `order` is the single source of truth. An "order index"
    // addresses a position in the song arrangement; it is resolved through
    // `order` to a pattern, which owns the cells.
    // -------------------------------------------------------------------

    /// Number of positions in the order list.
    pub fn order_len(&self) -> usize {
        self.order.len()
    }

    /// Row count of the pattern at an order position. Falls back to
    /// `rows_per_pattern` for out-of-range positions, and never returns 0
    /// so that callers can safely compute `rows - 1`.
    pub fn rows_at(&self, order_idx: usize) -> usize {
        let rows = self
            .order
            .get(order_idx)
            .and_then(|&p| self.patterns.get(p))
            .map(|p| p.rows)
            .unwrap_or(self.rows_per_pattern);
        rows.max(1)
    }

    /// The pattern played at an order position, if that position resolves.
    /// Prefer this over `song.patterns[song.order[i]]`: both indexes can be
    /// out of range for a file that was hand-edited or written by another
    /// version, and a panic in a draw loop takes the whole editor down.
    pub fn pattern_at(&self, order_idx: usize) -> Option<&Pattern> {
        let pattern_idx = *self.order.get(order_idx)?;
        self.patterns.get(pattern_idx)
    }

    /// Mutable counterpart to [`Song::pattern_at`].
    pub fn pattern_at_mut(&mut self, order_idx: usize) -> Option<&mut Pattern> {
        let pattern_idx = *self.order.get(order_idx)?;
        self.patterns.get_mut(pattern_idx)
    }

    /// Clamp an order position to a valid index. Returns 0 for an empty
    /// order list, which `repair` guarantees cannot happen after a load.
    pub fn clamp_order_position(&self, order_idx: usize) -> usize {
        order_idx.min(self.order.len().saturating_sub(1))
    }

    /// Read a cell by order position. Returns an empty cell for any
    /// out-of-range coordinate rather than panicking.
    pub fn cell_at(&self, order_idx: usize, row: usize, channel: usize) -> &Cell {
        static EMPTY: Cell = Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: None,
            effect_value: None,
        };
        self.order
            .get(order_idx)
            .and_then(|&p| self.patterns.get(p))
            .filter(|p| row < p.rows && channel < p.channels)
            .map(|p| p.get(row, channel))
            .unwrap_or(&EMPTY)
    }

    /// Mutable cell access by order position. Returns None if the
    /// coordinate does not resolve to a pattern cell.
    pub fn cell_at_mut(
        &mut self,
        order_idx: usize,
        row: usize,
        channel: usize,
    ) -> Option<&mut Cell> {
        let pattern_idx = *self.order.get(order_idx)?;
        let pattern = self.patterns.get_mut(pattern_idx)?;
        if row >= pattern.rows || channel >= pattern.channels {
            return None;
        }
        Some(&mut pattern.data[row][channel])
    }

    /// Write a cell by order position. Out-of-range coordinates are ignored.
    pub fn set_cell(&mut self, order_idx: usize, row: usize, channel: usize, cell: Cell) {
        if let Some(slot) = self.cell_at_mut(order_idx, row, channel) {
            *slot = cell;
        }
    }

    // -------------------------------------------------------------------
    // Order list management
    // -------------------------------------------------------------------

    /// Append a fresh pattern and a matching order entry.
    /// Returns the new order position.
    pub fn add_order_entry(&mut self) -> usize {
        let pattern_idx = self.add_pattern();
        self.order.push(pattern_idx);
        self.order_repeats.push(1);
        self.order.len() - 1
    }

    /// Duplicate the pattern referenced at `src` into a new pattern and
    /// append an order entry pointing at it. Returns the new order position.
    pub fn clone_order_entry(&mut self, src: usize) -> usize {
        let pattern = match self.order.get(src).and_then(|&p| self.patterns.get(p)) {
            Some(p) => p.clone(),
            None => return self.add_order_entry(),
        };
        let pattern_idx = self.patterns.len();
        self.patterns.push(pattern);
        self.order.push(pattern_idx);
        self.order_repeats.push(1);
        self.order.len() - 1
    }

    /// Remove an order position. The pattern itself is left in place so
    /// that other order entries referencing it are unaffected. Refuses to
    /// empty the order list. Returns true if removed.
    pub fn remove_order_entry(&mut self, order_idx: usize) -> bool {
        if order_idx >= self.order.len() || self.order.len() <= 1 {
            return false;
        }
        self.order.remove(order_idx);
        if order_idx < self.order_repeats.len() {
            self.order_repeats.remove(order_idx);
        }
        true
    }

    // -------------------------------------------------------------------
    // Validation
    // -------------------------------------------------------------------

    /// Repair structurally invalid data loaded from disk. Returns a list of
    /// human-readable descriptions of what was changed, empty if the song
    /// was already well-formed.
    ///
    /// A song file is user-editable text and may also have been written by
    /// an older version, so loading must never leave the song in a state
    /// that can panic the editor or the engine.
    ///
    /// Sizes are bounded from above as well as below. A pattern's geometry is
    /// declared in the file separately from its cell data, and [`Pattern::conform`]
    /// allocates `rows x channels` cells from the declared figures whatever
    /// the data holds -- so an unbounded pair of integers in a small file is an
    /// unbounded allocation, and a large enough one aborts the process before
    /// anything gets the chance to report it. The ceilings applied here are
    /// [`MAX_CHANNELS`] and [`MAX_ROWS_PER_PATTERN`], which are the same limits
    /// both frontends impose when the values are edited: a song beyond them
    /// could not have been made in rtrack and cannot be shown by it either.
    pub fn repair(&mut self) -> Vec<String> {
        let mut repairs = Vec::new();

        if self.channels == 0 {
            self.channels = 1;
            repairs.push("channel count was 0, set to 1".to_string());
        }
        if self.channels > MAX_CHANNELS {
            repairs.push(format!(
                "channel count was {}, clamped to the maximum of {}",
                self.channels, MAX_CHANNELS
            ));
            self.channels = MAX_CHANNELS;
        }
        if self.rows_per_pattern == 0 {
            self.rows_per_pattern = 64;
            repairs.push("rows per pattern was 0, set to 64".to_string());
        }
        if self.rows_per_pattern > MAX_ROWS_PER_PATTERN {
            repairs.push(format!(
                "rows per pattern was {}, clamped to the maximum of {}",
                self.rows_per_pattern, MAX_ROWS_PER_PATTERN
            ));
            self.rows_per_pattern = MAX_ROWS_PER_PATTERN;
        }
        if self.speed == 0 {
            self.speed = 6;
            repairs.push("speed was 0, set to 6".to_string());
        }
        if self.bpm == 0 {
            self.bpm = 120;
            repairs.push("bpm was 0, set to 120".to_string());
        }

        // Every pattern must have a geometry inside the editable range that
        // matches its own declared dimensions, and must cover all song
        // channels. The "resized" line below reports a clamp as well as a
        // repair, since either way the pattern is not the shape the file
        // claimed.
        for (i, pattern) in self.patterns.iter_mut().enumerate() {
            let declared_rows = pattern.rows.clamp(1, MAX_ROWS_PER_PATTERN);
            let declared_channels = pattern.channels.max(self.channels).clamp(1, MAX_CHANNELS);
            if pattern.rows != declared_rows || pattern.channels != declared_channels {
                repairs.push(format!(
                    "pattern {} resized from {}x{} to {}x{}",
                    i, pattern.rows, pattern.channels, declared_rows, declared_channels
                ));
            }
            if pattern.data.len() != declared_rows
                || pattern.data.iter().any(|r| r.len() != declared_channels)
            {
                repairs.push(format!("pattern {} had ragged cell data", i));
            }
            pattern.conform(declared_rows, declared_channels);
        }

        if self.patterns.is_empty() {
            self.patterns
                .push(Pattern::new(self.rows_per_pattern, self.channels));
            repairs.push("song had no patterns, added an empty one".to_string());
        }

        // Order entries must reference existing patterns.
        let pattern_count = self.patterns.len();
        let mut dropped = 0;
        self.order.retain(|&p| {
            let ok = p < pattern_count;
            if !ok {
                dropped += 1;
            }
            ok
        });
        if dropped > 0 {
            repairs.push(format!(
                "dropped {} order {} referencing missing patterns",
                dropped,
                if dropped == 1 { "entry" } else { "entries" }
            ));
        }
        if self.order.is_empty() {
            self.order.push(0);
            repairs.push("order list was empty, added pattern 0".to_string());
        }
        self.sync_order_repeats();

        // Tempo automation must point at real positions.
        let order_len = self.order.len();
        let before = self.tempo_map.len();
        self.tempo_map
            .retain(|tp| tp.order < order_len && tp.bpm > 0.0);
        if self.tempo_map.len() != before {
            repairs.push(format!(
                "dropped {} out-of-range tempo point(s)",
                before - self.tempo_map.len()
            ));
        }

        if self.highlight_beat == 0 {
            self.highlight_beat = default_highlight_beat();
        }
        if self.highlight_bar == 0 {
            self.highlight_bar = default_highlight_bar();
        }
        if self.swing > 100 {
            self.swing = default_swing();
            repairs.push("swing out of range, reset to 50".to_string());
        }

        repairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{Note, NoteValue};

    #[test]
    fn test_song_new() {
        let song = Song::new(4, 64);
        assert_eq!(song.channels, 4);
        assert_eq!(song.rows_per_pattern, 64);
        assert_eq!(song.patterns.len(), 1);
        assert_eq!(song.order, vec![0]);
        assert_eq!(song.bpm, 120);
        assert_eq!(song.speed, 6);
    }

    #[test]
    fn test_add_pattern() {
        let mut song = Song::new(4, 64);
        let idx = song.add_pattern();
        assert_eq!(idx, 1);
        assert_eq!(song.current_pattern_count(), 2);
    }

    #[test]
    fn test_seconds_per_row() {
        let song = Song::new(4, 64);
        let spr = song.seconds_per_row();
        // At 120 BPM, speed 6: ticks/sec = 120*24/60 = 48, spr = 6/48 = 0.125
        assert!((spr - 0.125).abs() < 1e-9);
    }

    #[test]
    fn test_save_load_roundtrip() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut song = Song::new(4, 32);
        song.title = "RoundtripTest".to_string();
        song.bpm = 155;
        song.speed = 3;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::Fs,
                    octave: 5,
                }),
                instrument: Some(0x0A),
                volume: Some(0x60),
                effect: Some(3),
                effect_value: Some(0xFF),
            },
        );

        let tmp = std::env::temp_dir().join("rtrack_song_roundtrip.rtrk");
        song.save(&tmp).unwrap();

        let loaded = Song::load(&tmp).unwrap();
        assert_eq!(loaded.title, "RoundtripTest");
        assert_eq!(loaded.bpm, 155);
        assert_eq!(loaded.speed, 3);
        assert_eq!(loaded.channels, 4);
        assert_eq!(loaded.rows_per_pattern, 32);

        let cell = loaded.patterns[0].get(0, 0);
        assert_eq!(
            cell.note,
            Some(Note::On {
                value: NoteValue::Fs,
                octave: 5
            })
        );
        assert_eq!(cell.instrument, Some(0x0A));
        assert_eq!(cell.volume, Some(0x60));
        assert_eq!(cell.effect, Some(3));
        assert_eq!(cell.effect_value, Some(0xFF));

        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_songfile_roundtrip_with_instruments() {
        let mut song = Song::new(4, 16);
        song.title = "WithInstruments".to_string();

        let song_file = SongFile {
            instruments: vec![
                InstrumentEntry {
                    slot: 0,
                    def: InstrumentDef {
                        name: "Kick".to_string(),
                        midi_program: None,
                        sample_index: Some(0),
                        synth_params: None,
                        pitch_bend_range: None,
                    },
                },
                InstrumentEntry {
                    slot: 5,
                    def: InstrumentDef {
                        name: "Lead".to_string(),
                        midi_program: Some(80),
                        sample_index: None,
                        synth_params: None,
                        pitch_bend_range: None,
                    },
                },
            ],
            sample_refs: vec![SampleRefEntry {
                slot: 0,
                sample_ref: SampleRef {
                    name: "kick".to_string(),
                    path: "samples/0-kick.wav".to_string(),
                    base_note: 36,
                    trim_start: 0,
                    trim_end: 0,
                    loop_enabled: false,
                    loop_start: 0,
                    loop_end: 0,
                },
            }],
            ..SongFile::from_song(song)
        };

        let tmp = std::env::temp_dir().join("rtrack_songfile_roundtrip.rtrk");
        song_file.save(&tmp).unwrap();

        let loaded = SongFile::load(&tmp).unwrap();
        assert_eq!(loaded.song.title, "WithInstruments");
        assert_eq!(loaded.instruments.len(), 2);
        assert_eq!(loaded.instruments[0].slot, 0);
        assert_eq!(loaded.instruments[0].def.name, "Kick");
        assert_eq!(loaded.instruments[0].def.sample_index, Some(0));
        assert_eq!(loaded.instruments[1].slot, 5);
        assert_eq!(loaded.instruments[1].def.midi_program, Some(80));
        assert_eq!(loaded.sample_refs.len(), 1);
        assert_eq!(loaded.sample_refs[0].slot, 0);
        assert_eq!(loaded.sample_refs[0].sample_ref.base_note, 36);
        assert_eq!(loaded.sample_refs[0].sample_ref.path, "samples/0-kick.wav");

        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_songfile_backwards_compat() {
        // An old-style .rtrk without instruments/sample_refs should still load
        let json = r#"{
            "title": "OldFormat",
            "bpm": 120,
            "speed": 6,
            "patterns": [{"rows": 16, "channels": 4, "data": []}],
            "order": [0],
            "channels": 4,
            "rows_per_pattern": 16
        }"#;

        let tmp = std::env::temp_dir().join("rtrack_compat_test.rtrk");
        std::fs::write(&tmp, json).unwrap();

        let loaded = SongFile::load(&tmp).unwrap();
        assert_eq!(loaded.song.title, "OldFormat");
        assert!(loaded.instruments.is_empty());
        assert!(loaded.sample_refs.is_empty());

        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_add_order_entry() {
        let mut song = Song::new(2, 4);
        assert_eq!(song.order_len(), 1);
        let idx = song.add_order_entry();
        assert_eq!(idx, 1);
        assert_eq!(song.order_len(), 2);
        assert_eq!(song.patterns.len(), 2);
        assert_eq!(song.order, vec![0, 1]);
    }

    #[test]
    fn test_clone_order_entry_copies_cells_into_a_new_pattern() {
        let mut song = Song::new(2, 4);
        song.set_cell(
            0,
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
        let idx = song.clone_order_entry(0);
        assert!(song.cell_at(idx, 0, 0).note.is_some());
        // Distinct patterns: editing the clone must not touch the original.
        assert_ne!(song.order[0], song.order[idx]);
        song.set_cell(idx, 0, 0, Cell::default());
        assert!(song.cell_at(0, 0, 0).note.is_some());
        assert!(song.cell_at(idx, 0, 0).note.is_none());
    }

    #[test]
    fn test_remove_order_entry_keeps_pattern_for_other_users() {
        let mut song = Song::new(2, 4);
        // Two order positions pointing at the same pattern.
        song.order = vec![0, 0];
        song.sync_order_repeats();
        song.set_cell(
            0,
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
        assert!(song.remove_order_entry(1));
        assert_eq!(song.order_len(), 1);
        assert_eq!(song.patterns.len(), 1);
        assert!(song.cell_at(0, 0, 0).note.is_some());
        // Refuses to empty the order list.
        assert!(!song.remove_order_entry(0));
    }

    #[test]
    fn test_pattern_reuse_survives_a_save_load_cycle() {
        // The same pattern used at three order positions must stay one
        // pattern: shared editing is the point of an order list.
        let mut song = Song::new(2, 4);
        song.order = vec![0, 0, 0];
        song.sync_order_repeats();
        song.set_cell(
            1,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::E,
                    octave: 5,
                }),
                ..Cell::default()
            },
        );

        let tmp = std::env::temp_dir().join("rtrack_reuse_roundtrip.rtrk");
        let file = SongFile::from_song(song.clone());
        file.save(&tmp).unwrap();
        let loaded = SongFile::load(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);

        assert_eq!(loaded.song.patterns.len(), 1, "pattern reuse was lost");
        assert_eq!(loaded.song.order, vec![0, 0, 0]);
        // The edit is visible from every position that shares the pattern.
        for pos in 0..3 {
            assert_eq!(
                loaded.song.cell_at(pos, 0, 0).note,
                Some(Note::On {
                    value: NoteValue::E,
                    octave: 5
                })
            );
        }
    }

    #[test]
    fn test_cell_access_is_bounds_safe() {
        let mut song = Song::new(2, 4);
        // Out-of-range reads yield an empty cell rather than panicking.
        assert!(song.cell_at(99, 0, 0).is_empty());
        assert!(song.cell_at(0, 99, 0).is_empty());
        assert!(song.cell_at(0, 0, 99).is_empty());
        // Out-of-range writes are ignored.
        song.set_cell(
            99,
            99,
            99,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        assert!(song.cell_at_mut(99, 0, 0).is_none());
    }

    #[test]
    fn test_rows_at_never_returns_zero() {
        let mut song = Song::new(2, 4);
        assert_eq!(song.rows_at(0), 4);
        // Unknown position falls back to the song default.
        assert_eq!(song.rows_at(99), 4);
        song.patterns[0].conform(0, 2);
        assert_eq!(song.rows_at(0), 1, "callers compute rows - 1");
    }

    #[test]
    fn test_repair_drops_dangling_order_entries() {
        let mut song = Song::new(2, 4);
        song.order = vec![0, 99, 7];
        song.order_repeats = vec![1, 1, 1];
        let repairs = song.repair();
        assert!(!repairs.is_empty());
        assert_eq!(song.order, vec![0]);
        assert_eq!(song.order_repeats.len(), 1);
    }

    #[test]
    fn test_repair_rebuilds_a_degenerate_song() {
        let mut song = Song::new(2, 4);
        song.channels = 0;
        song.rows_per_pattern = 0;
        song.speed = 0;
        song.bpm = 0;
        song.patterns.clear();
        song.order.clear();
        let repairs = song.repair();
        assert!(repairs.len() >= 5, "{repairs:?}");
        assert_eq!(song.channels, 1);
        assert_eq!(song.speed, 6);
        assert_eq!(song.bpm, 120);
        assert_eq!(song.patterns.len(), 1);
        assert_eq!(song.order, vec![0]);
        // The repaired song must be safe to read from.
        assert!(song.cell_at(0, 0, 0).is_empty());
    }

    #[test]
    fn test_repair_conforms_ragged_pattern_data() {
        let mut song = Song::new(2, 4);
        song.patterns[0].data.truncate(1);
        song.patterns[0].data[0].clear();
        let repairs = song.repair();
        assert!(!repairs.is_empty());
        assert_eq!(song.patterns[0].data.len(), 4);
        assert!(song.patterns[0].data.iter().all(|r| r.len() == 2));
    }

    /// A `.rtrk` declares each pattern's geometry separately from its cell
    /// data, and `conform` allocates from the declared figures. Without an
    /// upper bound a handful of characters in the file asks for an unbounded
    /// allocation, so the ceiling matters as much as the floor.
    #[test]
    fn test_repair_clamps_a_channel_count_past_the_maximum() {
        let mut song = Song::new(1, 4);
        song.channels = 9000;
        let repairs = song.repair();

        assert_eq!(song.channels, MAX_CHANNELS);
        assert_eq!(song.patterns[0].channels, MAX_CHANNELS);
        assert!(song.patterns[0]
            .data
            .iter()
            .all(|r| r.len() == MAX_CHANNELS));
        assert!(
            repairs.iter().any(|r| r.contains("channel count was 9000")),
            "the clamp must be reported, not silent: {repairs:?}"
        );
    }

    #[test]
    fn test_repair_clamps_a_row_count_past_the_maximum() {
        let mut song = Song::new(2, 4);
        song.rows_per_pattern = 100_000;
        song.patterns[0].rows = 100_000;
        let repairs = song.repair();

        assert_eq!(song.rows_per_pattern, MAX_ROWS_PER_PATTERN);
        assert_eq!(song.patterns[0].rows, MAX_ROWS_PER_PATTERN);
        assert_eq!(song.patterns[0].data.len(), MAX_ROWS_PER_PATTERN);
        assert!(
            repairs.iter().any(|r| r.contains("rows per pattern")),
            "{repairs:?}"
        );
    }

    /// The case that used to abort the process: `vec![Cell; usize::MAX]`
    /// panics with "capacity overflow" long before it runs out of memory,
    /// inside the very function whose job is to make a file from disk safe.
    #[test]
    fn test_repair_survives_an_absurd_declared_size() {
        let mut song = Song::new(1, 4);
        song.channels = usize::MAX;
        song.patterns[0].channels = usize::MAX;
        song.patterns[0].rows = usize::MAX;
        song.repair();

        assert_eq!(song.channels, MAX_CHANNELS);
        assert_eq!(song.patterns[0].rows, MAX_ROWS_PER_PATTERN);
        assert!(song.cell_at(0, 0, 0).is_empty());
    }

    /// The clamps sit exactly where both frontends already clamp, so a song
    /// built at the limit must come back untouched.
    #[test]
    fn test_repair_leaves_a_song_at_the_limits_alone() {
        let mut song = Song::new(MAX_CHANNELS, MAX_ROWS_PER_PATTERN);
        let repairs = song.repair();

        assert!(repairs.is_empty(), "{repairs:?}");
        assert_eq!(song.channels, MAX_CHANNELS);
        assert_eq!(song.patterns[0].rows, MAX_ROWS_PER_PATTERN);
    }

    #[test]
    fn test_repair_drops_out_of_range_tempo_points() {
        let mut song = Song::new(2, 4);
        song.tempo_map = vec![
            TempoPoint {
                order: 0,
                row: 0,
                bpm: 140.0,
            },
            TempoPoint {
                order: 42,
                row: 0,
                bpm: 140.0,
            },
        ];
        song.repair();
        assert_eq!(song.tempo_map.len(), 1);
    }
}
