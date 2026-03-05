use super::Pattern;

/// A song is a collection of patterns with an order list
#[derive(Debug, Clone)]
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
}
