use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::MIDI_CLOCKS_PER_BEAT;

use super::Pattern;
use crate::audio::synth::SynthParams;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub midi_program: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Extended song file format that includes instrument and sample info.
/// Backwards-compatible: old .rtrk files without these fields still load fine
/// (serde default kicks in for the optional fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongFile {
    #[serde(flatten)]
    pub song: Song,
    /// Instrument definitions (only non-empty ones are stored)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instruments: Vec<InstrumentEntry>,
    /// Sample file references with metadata
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_refs: Vec<SampleRefEntry>,
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

impl SongFile {
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize song file")?;
        // Atomic save: write to temp file in same directory, then rename
        let dir = path.parent().unwrap_or(Path::new("."));
        let temp_name = format!(
            ".rtrack_save_{}_{}.tmp",
            std::process::id(),
            path.file_name().and_then(|f| f.to_str()).unwrap_or("song")
        );
        let temp_path = dir.join(temp_name);
        std::fs::write(&temp_path, &json)
            .with_context(|| format!("Failed to write temp file {}", temp_path.display()))?;
        std::fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to rename {} -> {}", temp_path.display(), path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let song_file: SongFile = serde_json::from_str(&data)
            .context("Failed to parse song file")?;
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

fn default_highlight_beat() -> usize { 4 }
fn default_highlight_bar() -> usize { 16 }
fn default_swing() -> u8 { 50 }

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

    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize song")?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let temp_name = format!(
            ".rtrack_save_{}_{}.tmp",
            std::process::id(),
            path.file_name().and_then(|f| f.to_str()).unwrap_or("song")
        );
        let temp_path = dir.join(temp_name);
        std::fs::write(&temp_path, &json)
            .with_context(|| format!("Failed to write temp file {}", temp_path.display()))?;
        std::fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to rename {} -> {}", temp_path.display(), path.display()))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let song: Song = serde_json::from_str(&data)
            .context("Failed to parse song file")?;
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
        self.tempo_map.iter().find(|tp| tp.order == order && tp.row == row).map(|tp| tp.bpm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::Fs, octave: 5 }),
            instrument: Some(0x0A),
            volume: Some(0x60),
            effect: Some(3),
            effect_value: Some(0xFF),
        });

        let tmp = std::env::temp_dir().join("rtrack_song_roundtrip.rtrk");
        song.save(&tmp).unwrap();

        let loaded = Song::load(&tmp).unwrap();
        assert_eq!(loaded.title, "RoundtripTest");
        assert_eq!(loaded.bpm, 155);
        assert_eq!(loaded.speed, 3);
        assert_eq!(loaded.channels, 4);
        assert_eq!(loaded.rows_per_pattern, 32);

        let cell = loaded.patterns[0].get(0, 0);
        assert_eq!(cell.note, Some(Note::On { value: NoteValue::Fs, octave: 5 }));
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
            song,
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
            sample_refs: vec![
                SampleRefEntry {
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
                },
            ],
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
}
