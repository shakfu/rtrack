use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Pattern;

/// A song is a collection of patterns with an order list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub title: String,
    pub bpm: u16,
    pub speed: u8, // ticks per row
    pub patterns: Vec<Pattern>,
    pub order: Vec<usize>, // indices into patterns
    pub channels: usize,
    pub rows_per_pattern: usize,
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
            channels,
            rows_per_pattern,
        }
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

    pub fn get_pattern(&self, index: usize) -> Option<&Pattern> {
        self.patterns.get(index)
    }

    pub fn get_pattern_mut(&mut self, index: usize) -> Option<&mut Pattern> {
        self.patterns.get_mut(index)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize song")?;
        std::fs::write(path, json)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let song: Song = serde_json::from_str(&data)
            .context("Failed to parse song file")?;
        Ok(song)
    }

    /// Seconds per row based on BPM and speed
    pub fn seconds_per_row(&self) -> f64 {
        // Classic tracker timing: BPM defines ticks per minute / 24
        // Each row takes `speed` ticks
        let ticks_per_second = (self.bpm as f64 * 24.0) / 60.0;
        self.speed as f64 / ticks_per_second
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
}
