//! Phrases: reusable single-channel row runs that pattern columns link to
//! (`docs/dev/g1-g3-sequencing.md`).

use serde::{Deserialize, Serialize};

use super::Cell;

/// A single-channel run of rows, usable by any column of any pattern.
/// Editing it changes every column that links to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phrase {
    #[serde(default)]
    pub name: String,
    pub rows: Vec<Cell>,
}

/// A pattern column's link to a phrase, played instead of its own cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhraseLink {
    pub phrase: usize,
    /// Semitones added at playback (G2). Never written back to the phrase.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub transpose: i8,
    #[serde(default)]
    pub timing: PhraseTiming,
}

fn is_zero(v: &i8) -> bool {
    *v == 0
}

/// How a linked phrase lines up with the song.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhraseTiming {
    /// Pattern row `r` plays phrase row `r % len`: the phrase restarts with
    /// the pattern and loops inside it.
    #[default]
    Aligned,
    /// Plays phrase row `n % len`, where `n` counts rows played since the
    /// song started. Continues across patterns, repeats and jumps.
    Running,
}
