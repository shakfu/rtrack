use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::{MAX_CHANNELS, MAX_ROWS_PER_PATTERN, MIDI_CLOCKS_PER_BEAT};

use super::{Cell, Note, Pattern, Phrase, PhraseLink, PhraseTiming, Table};
use crate::audio::effects::SendBusParams;
use crate::audio::synth::SynthParams;
use crate::fs::write_atomic;
use crate::theory::ScaleSetting;
use crate::types::{ChannelConfig, Instrument, InstrumentKind, MidiCcMapping};

/// Where a cell is stored: in a pattern, or in a phrase a pattern column
/// links to. Undo records edits by this, so an edit to a phrase undoes
/// wherever it was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellAddr {
    Pattern {
        pattern: usize,
        row: usize,
        channel: usize,
    },
    Phrase {
        phrase: usize,
        row: usize,
    },
}

/// What an editor's grid shows. Cell accessors take `impl Into<Sheet>`, and an
/// order position converts to `Order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// The pattern at an order position.
    Order(usize),
    /// One phrase, in column `track` alone; the other columns do not resolve.
    /// Notes entered there preview through that track.
    Phrase { phrase: usize, track: usize },
}

impl From<usize> for Sheet {
    fn from(order_idx: usize) -> Self {
        Sheet::Order(order_idx)
    }
}

/// The furthest a phrase link transposes, in steps either way: four
/// octaves of semitones, or more than six of scale degrees.
pub const MAX_LINK_TRANSPOSE: i8 = 48;

/// Most steps in a groove.
pub const MAX_GROOVE_STEPS: usize = 16;

/// What an unresolvable coordinate reads as.
static EMPTY_CELL: Cell = Cell {
    note: None,
    instrument: None,
    volume: None,
    fx: [None, None],
};

/// A tempo change point in the song
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoPoint {
    pub order: usize,
    pub row: usize,
    pub bpm: f64,
}

/// Serializable instrument definition (stored in .rtrk files)
///
/// Version 2 writes `kind`. Versions 0 and 1 wrote the untyped
/// `midi_program`/`sample_index`/`synth_params`, which are read but no longer
/// written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentDef {
    pub name: String,
    // Every optional field needs `default` as well as `skip_serializing_if`:
    // without it, serde treats an omitted field as an error, so a file we
    // wrote ourselves would fail to load back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InstrumentKind>,
    #[serde(default, skip_serializing)]
    pub midi_program: Option<u8>,
    #[serde(default, skip_serializing)]
    pub sample_index: Option<usize>,
    #[serde(default, skip_serializing)]
    pub synth_params: Option<SynthParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_bend_range: Option<f64>,
}

impl InstrumentDef {
    pub fn from_instrument(inst: &Instrument) -> Self {
        Self {
            name: inst.name.clone(),
            kind: Some(inst.kind.clone()),
            midi_program: None,
            sample_index: None,
            synth_params: None,
            pitch_bend_range: inst.pitch_bend_range,
        }
    }

    /// The instrument this defines in `slot`, converting the untyped form of
    /// versions 0 and 1. Returns a repair note for each setting the old
    /// routing never used.
    pub fn to_instrument(&self, slot: usize) -> (Instrument, Vec<String>) {
        let (kind, repairs) = match &self.kind {
            Some(kind) => (kind.clone(), Vec::new()),
            None => InstrumentKind::from_legacy(
                slot,
                self.sample_index,
                self.synth_params.clone(),
                self.midi_program,
            ),
        };
        let inst = Instrument {
            name: self.name.clone(),
            kind,
            pitch_bend_range: self.pitch_bend_range,
        };
        (inst, repairs)
    }
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
/// Files predating the field deserialize as version 0. Version 2 stores
/// named effect columns (`Cell.fx`); the reader still accepts the
/// single-digit effects of versions 0 and 1, so it does not branch on this.
pub const FORMAT_VERSION: u32 = 2;

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
    /// See `crate::fs::write_atomic` for the durability and temp-file handling this
    /// relies on.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        write_atomic(path, json.as_bytes())
    }

    /// Read a song file.
    ///
    /// Parsing is not validation. A `.rtrk` is user-editable text and may also
    /// have been written by an older version, so the result can be
    /// structurally impossible -- an order list naming patterns that are not
    /// there, a pattern whose declared geometry does not match its cells, a
    /// channel count larger than the editor can show. **Call
    /// [`Song::repair`] on `song` before using it**, and show what it returns:
    /// it corrects those and reports each correction.
    ///
    /// ```no_run
    /// use rtrack_core::tracker::SongFile;
    ///
    /// let mut file = SongFile::load(std::path::Path::new("song.rtrk"))?;
    /// for repair in file.song.repair() {
    ///     eprintln!("repaired: {repair}");
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let mut song_file: SongFile =
            serde_json::from_str(&data).context("Failed to parse song file")?;
        song_file.song.adopt_legacy_swing();
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
    /// Relative row lengths, repeating by pattern row: row `r` lasts
    /// `len * groove[r % len] / sum` of a straight row. Empty is straight.
    /// `[67, 33]` is the old swing 67 (`docs/dev/g9-groove.md`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groove: Vec<u8>,
    /// The swing amount files before groove stored (50 = none), read only
    /// to become a groove; see [`Song::adopt_legacy_swing`].
    #[serde(default, rename = "swing", skip_serializing)]
    pub(crate) legacy_swing: Option<u8>,
    /// Tempo automation points (order, row, bpm)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_map: Vec<TempoPoint>,
    /// Scale constraining keyboard note entry. `None` means chromatic entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleSetting>,
    /// Single-channel row runs that pattern columns link to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phrases: Vec<Phrase>,
    /// Tables by number. Instrument N plays table N, if there is one.
    /// Saved as a list of `{"slot": N, ...}` entries: `Song` is flattened
    /// into `SongFile`, and through a flatten serde cannot read a map's
    /// string keys back as numbers.
    #[serde(
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        with = "super::table::by_slot"
    )]
    pub tables: std::collections::BTreeMap<u8, Table>,
}

fn default_highlight_beat() -> usize {
    4
}
fn default_highlight_bar() -> usize {
    16
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
            groove: Vec::new(),
            legacy_swing: None,
            tempo_map: Vec::new(),
            scale: None,
            phrases: Vec::new(),
            tables: Default::default(),
        }
    }

    /// Constrain a note played on the keyboard to the song's scale.
    ///
    /// Applied by the frontends at note entry only. Notes arriving from a file,
    /// the clipboard or MIDI input pass through untouched, so setting a scale
    /// never rewrites material that is already in the song.
    pub fn snap_entry(&self, note: Note) -> Note {
        let (Some(scale), Some(midi)) = (self.scale, note.to_midi_note()) else {
            return note;
        };
        Note::from_midi(scale.snap(midi)).unwrap_or(note)
    }

    /// Roughly how much heap this song holds, for budgeting the undo history.
    ///
    /// Dominated by the patterns; the order list, tempo map and title are
    /// counted because they are cheap to count, not because they matter.
    pub fn approx_heap_bytes(&self) -> usize {
        let patterns: usize = self.patterns.iter().map(|p| p.approx_heap_bytes()).sum();
        let phrases: usize = self
            .phrases
            .iter()
            .map(|p| p.rows.capacity() * std::mem::size_of::<Cell>() + p.name.capacity())
            .sum();
        let tables: usize = self
            .tables
            .values()
            .map(|t| {
                std::mem::size_of::<(u8, Table)>()
                    + t.rows.capacity() * std::mem::size_of::<super::TableRow>()
                    + t.name.capacity()
            })
            .sum();
        patterns
            + phrases
            + tables
            + self.phrases.capacity() * std::mem::size_of::<Phrase>()
            + self.patterns.capacity() * std::mem::size_of::<Pattern>()
            + self.order.capacity() * std::mem::size_of::<usize>()
            + self.order_repeats.capacity()
            + self.tempo_map.capacity() * std::mem::size_of::<TempoPoint>()
            + self.title.capacity()
            + std::mem::size_of::<Self>()
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
    /// Shares `crate::fs::write_atomic` with [`SongFile::save`], so both paths
    /// get the same durability and temp-file guarantees.
    ///
    /// # Deprecated
    ///
    /// Writes a song with no format version and none of the instrument,
    /// sample, mixer or MIDI-learn state that makes a `.rtrk` a whole
    /// document. The result loads -- [`SongFile`]'s extra fields all have
    /// serde defaults -- but as a song that has lost everything except its
    /// patterns. Use [`SongFile::save`], which writes the full document.
    #[allow(dead_code)]
    #[deprecated(
        since = "0.1.3",
        note = "writes a song without its instruments, samples or mixer state; use SongFile::save"
    )]
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("Failed to serialize song")?;
        write_atomic(path, json.as_bytes())
    }

    /// Read just the song data from a file.
    ///
    /// # Deprecated
    ///
    /// Two reasons, and the second is the one that matters:
    ///
    /// 1. It reads only the song. Instruments, sample references, mixer state
    ///    and MIDI-learn mappings are in the same file and are silently
    ///    dropped, so a `.rtrk` opened this way comes back missing most of
    ///    what was saved.
    ///
    /// 2. Nothing here checks that what was parsed is structurally possible,
    ///    and the type gives a caller no hint that it should. A song file is
    ///    user-editable text: it can name patterns that do not exist, declare
    ///    a geometry its cells do not match, or ask for more channels than the
    ///    editor can show. [`Song::repair`] exists to correct exactly that and
    ///    is easy to not know about when the call that produced the `Song`
    ///    never mentions it.
    ///
    /// Use [`SongFile::load`] and call [`Song::repair`] on the result, which
    /// is what the editor itself does.
    #[allow(dead_code)]
    #[deprecated(
        since = "0.1.3",
        note = "drops instruments, samples and mixer state, and does not repair the song; use SongFile::load then Song::repair"
    )]
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let mut song: Song = serde_json::from_str(&data).context("Failed to parse song file")?;
        song.adopt_legacy_swing();
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

    /// How much longer than straight pattern row `row` lasts under the
    /// groove: 1.0 when straight. A whole groove cycle lasts as long as the
    /// same rows played straight.
    pub fn groove_factor(&self, row: usize) -> f64 {
        self.grooved(1.0, row)
    }

    /// `length` stretched by the groove for pattern row `row`.
    ///
    /// Computed as `length * weight / mean`, in that order: for a swing
    /// groove `[s, 100 - s]` the mean is exactly 50, so this is the old
    /// swing formula operation for operation, and old songs keep their
    /// timing to the frame.
    pub fn grooved(&self, length: f64, row: usize) -> f64 {
        let sum: u32 = self.groove.iter().map(|&w| w as u32).sum();
        if sum == 0 {
            return length;
        }
        let weight = self.groove[row % self.groove.len()] as f64;
        let mean = sum as f64 / self.groove.len() as f64;
        length * weight / mean
    }

    /// The groove of swing `amount`: even rows get `amount` percent of each
    /// pair. 50, which was straight, is no groove.
    pub fn swing_groove(amount: u8) -> Vec<u8> {
        let amount = amount.clamp(1, 99);
        if amount == 50 {
            Vec::new()
        } else {
            vec![amount, 100 - amount]
        }
    }

    /// The groove as text, for editing: `67 33`, or `off` when straight.
    pub fn groove_text(&self) -> String {
        if self.groove.is_empty() {
            "off".to_string()
        } else {
            self.groove
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    /// Parse a groove typed as text: weights separated by spaces, `off` for
    /// straight, or one number `s` for swing `s` (`s 100-s`).
    pub fn parse_groove(text: &str) -> Result<Vec<u8>, String> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("off") {
            return Ok(Vec::new());
        }
        let weights = text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|w| !w.is_empty())
            .map(|w| match w.parse::<u8>() {
                Ok(0) => Err("a groove step needs a weight of 1 or more".to_string()),
                Ok(v) => Ok(v),
                Err(_) => Err(format!("not a groove weight: {w}")),
            })
            .collect::<Result<Vec<u8>, String>>()?;
        match weights.as_slice() {
            [swing] => Ok(Self::swing_groove(*swing)),
            _ if weights.len() > MAX_GROOVE_STEPS => {
                Err(format!("a groove has at most {MAX_GROOVE_STEPS} steps"))
            }
            _ => Ok(weights),
        }
    }

    /// Turn a swing read from an older file into the equivalent groove,
    /// unless the file also has a groove. Exact: swing stretched ticks by
    /// `swing / 50` and `(100 - swing) / 50`, which `[swing, 100 - swing]`
    /// reproduces.
    pub fn adopt_legacy_swing(&mut self) {
        if let Some(swing) = self.legacy_swing.take() {
            if self.groove.is_empty() {
                self.groove = Self::swing_groove(swing.min(100));
            }
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

    /// Row count of a sheet: the pattern at an order position, or a phrase.
    /// Falls back to `rows_per_pattern` when it does not resolve, and never returns 0
    /// so that callers can safely compute `rows - 1`.
    pub fn rows_at(&self, sheet: impl Into<Sheet>) -> usize {
        let rows = match sheet.into() {
            Sheet::Order(order_idx) => self
                .order
                .get(order_idx)
                .and_then(|&p| self.patterns.get(p))
                .map(|p| p.rows),
            Sheet::Phrase { phrase, .. } => self.phrases.get(phrase).map(|ph| ph.rows.len()),
        };
        rows.unwrap_or(self.rows_per_pattern).max(1)
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

    /// Where the cell shown on a sheet is stored, or `None` if the coordinate
    /// does not resolve. Editors read and write through this so that a linked
    /// column edits its phrase.
    ///
    /// A running link resolves by the first pass through the order list,
    /// which is what the grid shows; playback follows the path it actually
    /// took, through [`Song::playback_cell`].
    pub fn address(&self, sheet: impl Into<Sheet>, row: usize, channel: usize) -> Option<CellAddr> {
        match sheet.into() {
            Sheet::Order(order_idx) => self
                .resolve(order_idx, row, channel, || {
                    self.first_pass_row(order_idx) + row as u64
                })
                .map(|(at, _)| at),
            Sheet::Phrase { phrase, track } => {
                let len = self.phrases.get(phrase)?.rows.len();
                (channel == track && row < len).then_some(CellAddr::Phrase { phrase, row })
            }
        }
    }

    /// The cell playback plays at an order position, when `rows_played`
    /// rows have been played since the song started. Only running links
    /// read the count.
    ///
    /// A linked cell's note is transposed by the link, as the transpose keys
    /// move notes: by scale degree when the song has a scale, by semitone
    /// otherwise. The stored cell, which the grid shows, is unchanged.
    pub fn playback_cell(
        &self,
        order_idx: usize,
        row: usize,
        channel: usize,
        rows_played: u64,
    ) -> Cell {
        let Some((at, transpose)) = self.resolve(order_idx, row, channel, || rows_played) else {
            return EMPTY_CELL;
        };
        let mut cell = self.cell(at).copied().unwrap_or(EMPTY_CELL);
        if transpose != 0 {
            cell.transpose_note(transpose, self.scale);
        }
        cell
    }

    /// Whether a sheet shows anything in the second effect column: a phrase,
    /// or a pattern in its own cells or in a phrase it links to.
    pub fn uses_second_fx(&self, sheet: impl Into<Sheet>) -> bool {
        let order_idx = match sheet.into() {
            Sheet::Order(order_idx) => order_idx,
            Sheet::Phrase { phrase, .. } => {
                return self
                    .phrases
                    .get(phrase)
                    .is_some_and(|ph| ph.rows.iter().any(|c| c.fx[1].is_some()));
            }
        };
        let Some(p) = self.pattern_at(order_idx) else {
            return false;
        };
        p.uses_second_fx()
            || p.links.iter().flatten().any(|l| {
                self.phrases
                    .get(l.phrase)
                    .is_some_and(|ph| ph.rows.iter().any(|c| c.fx[1].is_some()))
            })
    }

    /// Rows played before order entry `order_idx` is first reached, counting
    /// repeats and skipping entries repeated 0 times.
    pub fn first_pass_row(&self, order_idx: usize) -> u64 {
        (0..order_idx.min(self.order.len()))
            .map(|i| {
                let repeats = self.order_repeats.get(i).copied().unwrap_or(1);
                self.rows_at(i) as u64 * repeats as u64
            })
            .sum()
    }

    /// Where a cell is stored, and the transpose of the link it is read
    /// through (0 for none). `running_row` gives the row count a running
    /// link plays by; it is asked for only when one is found.
    fn resolve(
        &self,
        order_idx: usize,
        row: usize,
        channel: usize,
        running_row: impl FnOnce() -> u64,
    ) -> Option<(CellAddr, i8)> {
        let pattern = *self.order.get(order_idx)?;
        let p = self.patterns.get(pattern)?;
        if row >= p.rows || channel >= p.channels {
            return None;
        }
        // A link to a missing phrase, which `repair` removes on load, reads
        // as no link rather than as nothing.
        let linked = p
            .link(channel)
            .and_then(|link| Some((link, self.phrases.get(link.phrase)?.rows.len())))
            .filter(|(_, len)| *len > 0);
        Some(match linked {
            Some((link, len)) => (
                CellAddr::Phrase {
                    phrase: link.phrase,
                    row: match link.timing {
                        PhraseTiming::Aligned => row % len,
                        PhraseTiming::Running => (running_row() % len as u64) as usize,
                    },
                },
                link.transpose,
            ),
            None => (
                CellAddr::Pattern {
                    pattern,
                    row,
                    channel,
                },
                0,
            ),
        })
    }

    /// The distinct addresses of a block of rows and channels on a sheet, in
    /// row-major order. A cell an operation transforms in place must be
    /// visited once even when it shows at several places, as a phrase linked
    /// twice, or looping inside its pattern, does.
    pub fn addresses(
        &self,
        sheet: impl Into<Sheet>,
        rows: std::ops::RangeInclusive<usize>,
        channels: std::ops::RangeInclusive<usize>,
    ) -> Vec<CellAddr> {
        let sheet = sheet.into();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for row in rows {
            for ch in channels.clone() {
                if let Some(at) = self.address(sheet, row, ch) {
                    if seen.insert(at) {
                        out.push(at);
                    }
                }
            }
        }
        out
    }

    /// The cell stored at `at`, if it exists.
    pub fn cell(&self, at: CellAddr) -> Option<&Cell> {
        match at {
            CellAddr::Pattern {
                pattern,
                row,
                channel,
            } => self.patterns.get(pattern)?.data.get(row)?.get(channel),
            CellAddr::Phrase { phrase, row } => self.phrases.get(phrase)?.rows.get(row),
        }
    }

    /// Mutable counterpart to [`Song::cell`].
    pub fn cell_mut(&mut self, at: CellAddr) -> Option<&mut Cell> {
        match at {
            CellAddr::Pattern {
                pattern,
                row,
                channel,
            } => self
                .patterns
                .get_mut(pattern)?
                .data
                .get_mut(row)?
                .get_mut(channel),
            CellAddr::Phrase { phrase, row } => self.phrases.get_mut(phrase)?.rows.get_mut(row),
        }
    }

    /// Read a cell on a sheet. Returns an empty cell for any out-of-range
    /// coordinate rather than panicking.
    pub fn cell_at(&self, sheet: impl Into<Sheet>, row: usize, channel: usize) -> &Cell {
        self.address(sheet, row, channel)
            .and_then(|at| self.cell(at))
            .unwrap_or(&EMPTY_CELL)
    }

    /// Mutable cell access on a sheet. Returns None if the coordinate does
    /// not resolve.
    pub fn cell_at_mut(
        &mut self,
        sheet: impl Into<Sheet>,
        row: usize,
        channel: usize,
    ) -> Option<&mut Cell> {
        let at = self.address(sheet, row, channel)?;
        self.cell_mut(at)
    }

    /// Write a cell on a sheet. Out-of-range coordinates are ignored.
    pub fn set_cell(&mut self, sheet: impl Into<Sheet>, row: usize, channel: usize, cell: Cell) {
        if let Some(slot) = self.cell_at_mut(sheet, row, channel) {
            *slot = cell;
        }
    }

    // -------------------------------------------------------------------
    // Phrase links
    // -------------------------------------------------------------------

    /// Turn a column's cells into a new phrase, as long as the pattern, and
    /// link the column to it. Returns the phrase index.
    pub fn link_new_phrase(
        &mut self,
        order_idx: usize,
        channel: usize,
        timing: PhraseTiming,
    ) -> Option<usize> {
        let p = self
            .pattern_at(order_idx)
            .filter(|p| channel < p.channels)?;
        let rows = (0..p.rows)
            .map(|r| *self.cell_at(order_idx, r, channel))
            .collect();
        let index = self.phrases.len();
        self.phrases.push(Phrase {
            name: format!("Phrase {index:02X}"),
            rows,
        });
        self.link_column(order_idx, channel, index, timing);
        Some(index)
    }

    /// Link a column to an existing phrase. Returns whether it did.
    pub fn link_column(
        &mut self,
        order_idx: usize,
        channel: usize,
        phrase: usize,
        timing: PhraseTiming,
    ) -> bool {
        if phrase >= self.phrases.len() {
            return false;
        }
        let Some(p) = self
            .pattern_at_mut(order_idx)
            .filter(|p| channel < p.channels)
        else {
            return false;
        };
        if p.links.len() <= channel {
            p.links.resize(channel + 1, None);
        }
        p.links[channel] = Some(PhraseLink {
            phrase,
            transpose: 0,
            timing,
        });
        true
    }

    /// Remove a column's link, keeping what the grid showed: the phrase
    /// rows, as on the first pass, become the column's own cells.
    pub fn unlink_column(&mut self, order_idx: usize, channel: usize) -> bool {
        let Some(rows) = self.pattern_at(order_idx).map(|p| p.rows) else {
            return false;
        };
        let shown: Vec<Cell> = (0..rows)
            .map(|r| *self.cell_at(order_idx, r, channel))
            .collect();
        let Some(p) = self.pattern_at_mut(order_idx) else {
            return false;
        };
        if p.link(channel).is_none() {
            return false;
        }
        p.links[channel] = None;
        for (r, cell) in shown.into_iter().enumerate() {
            p.data[r][channel] = cell;
        }
        true
    }

    /// Set how a linked column lines up with the song.
    pub fn set_link_timing(&mut self, order_idx: usize, channel: usize, timing: PhraseTiming) {
        if let Some(Some(link)) = self
            .pattern_at_mut(order_idx)
            .and_then(|p| p.links.get_mut(channel))
        {
            link.timing = timing;
        }
    }

    /// Set how far a linked column transposes its phrase at playback, within
    /// `-MAX_LINK_TRANSPOSE..=MAX_LINK_TRANSPOSE` steps.
    pub fn set_link_transpose(&mut self, order_idx: usize, channel: usize, steps: i8) {
        if let Some(Some(link)) = self
            .pattern_at_mut(order_idx)
            .and_then(|p| p.links.get_mut(channel))
        {
            link.transpose = steps.clamp(-MAX_LINK_TRANSPOSE, MAX_LINK_TRANSPOSE);
        }
    }

    /// Table `number` for editing, created as one empty row if there is none,
    /// so that opening an editor on a table creates nothing until it is edited.
    pub fn table_mut(&mut self, number: u8) -> &mut Table {
        self.tables.entry(number).or_insert_with(|| Table::new(1))
    }

    /// Set a phrase's length, padding with empty rows or truncating, within
    /// 1 to `MAX_ROWS_PER_PATTERN` rows.
    pub fn resize_phrase(&mut self, phrase: usize, rows: usize) {
        if let Some(ph) = self.phrases.get_mut(phrase) {
            ph.rows
                .resize(rows.clamp(1, MAX_ROWS_PER_PATTERN), Cell::default());
        }
    }

    /// Append an empty phrase of `rows` rows, within 1 to
    /// `MAX_ROWS_PER_PATTERN`. Returns its index.
    pub fn add_phrase(&mut self, rows: usize) -> usize {
        let index = self.phrases.len();
        self.phrases.push(Phrase {
            name: format!("Phrase {index:02X}"),
            rows: vec![Cell::default(); rows.clamp(1, MAX_ROWS_PER_PATTERN)],
        });
        index
    }

    /// Append a copy of a phrase. Returns the copy's index.
    pub fn duplicate_phrase(&mut self, phrase: usize) -> Option<usize> {
        let mut copy = self.phrases.get(phrase)?.clone();
        let index = self.phrases.len();
        copy.name = format!("Phrase {index:02X}");
        self.phrases.push(copy);
        Some(index)
    }

    /// Insert an empty row into a phrase, dropping its last row so the
    /// length is unchanged, as [`Pattern::insert_row`] does.
    pub fn insert_phrase_row(&mut self, phrase: usize, at: usize) {
        if let Some(ph) = self.phrases.get_mut(phrase).filter(|ph| at < ph.rows.len()) {
            ph.rows.insert(at, Cell::default());
            ph.rows.pop();
        }
    }

    /// Delete a phrase row, appending an empty one so the length is unchanged.
    pub fn delete_phrase_row(&mut self, phrase: usize, at: usize) {
        if let Some(ph) = self.phrases.get_mut(phrase).filter(|ph| at < ph.rows.len()) {
            ph.rows.remove(at);
            ph.rows.push(Cell::default());
        }
    }

    /// How many pattern columns link to a phrase.
    pub fn phrase_uses(&self, phrase: usize) -> usize {
        self.patterns
            .iter()
            .flat_map(|p| p.links.iter().flatten())
            .filter(|l| l.phrase == phrase)
            .count()
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

        // Phrases hold 1 to MAX_ROWS_PER_PATTERN rows: a running phrase plays
        // row `n % len`, so an empty one cannot be played at all.
        for (i, phrase) in self.phrases.iter_mut().enumerate() {
            if phrase.rows.is_empty() {
                phrase.rows.push(Cell::default());
                repairs.push(format!("phrase {} had no rows, given one empty row", i));
            }
            if phrase.rows.len() > MAX_ROWS_PER_PATTERN {
                repairs.push(format!(
                    "phrase {} had {} rows, truncated to {}",
                    i,
                    phrase.rows.len(),
                    MAX_ROWS_PER_PATTERN
                ));
                phrase.rows.truncate(MAX_ROWS_PER_PATTERN);
            }
        }
        for (number, table) in self.tables.iter_mut() {
            repairs.extend(table.repair(*number));
        }

        // Links must reference existing phrases.
        let phrase_count = self.phrases.len();
        for (i, pattern) in self.patterns.iter_mut().enumerate() {
            for (ch, link) in pattern.links.iter_mut().enumerate() {
                if link.is_some_and(|l| l.phrase >= phrase_count) {
                    *link = None;
                    repairs.push(format!(
                        "pattern {} channel {} linked to a missing phrase, unlinked",
                        i,
                        ch + 1
                    ));
                }
                if let Some(l) = link
                    .as_mut()
                    .filter(|l| !(-MAX_LINK_TRANSPOSE..=MAX_LINK_TRANSPOSE).contains(&l.transpose))
                {
                    l.transpose = l.transpose.clamp(-MAX_LINK_TRANSPOSE, MAX_LINK_TRANSPOSE);
                    repairs.push(format!(
                        "pattern {} channel {} transpose clamped to {}",
                        i,
                        ch + 1,
                        l.transpose
                    ));
                }
            }
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
        self.adopt_legacy_swing();
        if self.groove.len() > MAX_GROOVE_STEPS {
            repairs.push(format!(
                "groove had {} steps, truncated to {MAX_GROOVE_STEPS}",
                self.groove.len()
            ));
            self.groove.truncate(MAX_GROOVE_STEPS);
        }
        if self.groove.contains(&0) {
            self.groove
                .iter_mut()
                .filter(|w| **w == 0)
                .for_each(|w| *w = 1);
            repairs.push("groove step of weight 0 set to 1".to_string());
        }

        repairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::ScaleSetting;
    use crate::tracker::{Fx, FxCmd, Note, NoteValue};

    #[test]
    fn a_song_without_a_scale_leaves_entry_alone() {
        let song = Song::new(4, 64);
        assert_eq!(song.scale, None);
        let cs = Note::On {
            value: NoteValue::Cs,
            octave: 5,
        };
        assert_eq!(song.snap_entry(cs), cs);
    }

    #[test]
    fn entry_snaps_to_the_song_scale_but_note_off_passes_through() {
        let mut song = Song::new(4, 64);
        song.scale = Some(ScaleSetting::new(NoteValue::C, crate::theory::Scale::Major));
        assert_eq!(
            song.snap_entry(Note::On {
                value: NoteValue::Cs,
                octave: 5,
            }),
            Note::On {
                value: NoteValue::C,
                octave: 5,
            }
        );
        assert_eq!(song.snap_entry(Note::Off), Note::Off);
    }

    #[test]
    fn the_scale_survives_a_save_and_an_absent_one_is_not_written() {
        let mut song = Song::new(4, 64);
        let json = serde_json::to_string(&song).unwrap();
        assert!(!json.contains("scale"), "no scale should write no field");
        assert_eq!(
            serde_json::from_str::<Song>(&json).unwrap().scale,
            None,
            "a file predating the field still loads"
        );

        let setting = ScaleSetting::new(NoteValue::Ds, crate::theory::Scale::HarmonicMinor);
        song.scale = Some(setting);
        let json = serde_json::to_string(&song).unwrap();
        assert_eq!(
            serde_json::from_str::<Song>(&json).unwrap().scale,
            Some(setting)
        );
    }

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

    /// The deprecated bare-`Song` pair still has to work: deprecating it says
    /// do not reach for this, not that it stopped functioning.
    #[test]
    #[allow(deprecated)]
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
                fx: [Some(Fx::new(FxCmd::PitchSlide, 0xFF)), None],
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
        assert_eq!(cell.fx[0], Some(Fx::new(FxCmd::PitchSlide, 0xFF)));

        let _ = std::fs::remove_file(tmp);
    }

    fn phrase(rows: usize) -> crate::tracker::Phrase {
        crate::tracker::Phrase {
            name: "riff".to_string(),
            rows: vec![Cell::default(); rows],
        }
    }

    fn link(phrase: usize, timing: crate::tracker::PhraseTiming) -> crate::tracker::PhraseLink {
        crate::tracker::PhraseLink {
            phrase,
            transpose: -3,
            timing,
        }
    }

    #[test]
    fn addresses_cover_a_block_and_skip_what_does_not_resolve() {
        let song = Song::new(2, 4);
        let at = song.addresses(0, 2..=5, 1..=3);
        // Rows 2-3 of channel 1; rows past 3 and channels past 1 do not exist.
        assert_eq!(
            at,
            vec![
                CellAddr::Pattern {
                    pattern: 0,
                    row: 2,
                    channel: 1
                },
                CellAddr::Pattern {
                    pattern: 0,
                    row: 3,
                    channel: 1
                },
            ]
        );
    }

    #[test]
    fn a_phrase_address_reads_and_writes_the_phrase() {
        let mut song = Song::new(1, 4);
        song.phrases.push(phrase(3));
        let at = CellAddr::Phrase { phrase: 0, row: 2 };
        song.cell_mut(at).unwrap().volume = Some(9);
        assert_eq!(song.phrases[0].rows[2].volume, Some(9));
        assert_eq!(song.cell(at).unwrap().volume, Some(9));
        assert!(song.cell(CellAddr::Phrase { phrase: 0, row: 3 }).is_none());
        assert!(song.cell(CellAddr::Phrase { phrase: 1, row: 0 }).is_none());
    }

    #[test]
    fn phrase_rows_insert_and_delete_at_a_fixed_length() {
        let mut song = Song::new(1, 4);
        let p = song.add_phrase(3);
        song.phrases[p].rows[0].volume = Some(1);
        song.phrases[p].rows[2].volume = Some(3);

        song.insert_phrase_row(p, 0);
        let vols = |s: &Song| {
            s.phrases[p]
                .rows
                .iter()
                .map(|c| c.volume)
                .collect::<Vec<_>>()
        };
        assert_eq!(vols(&song), [None, Some(1), None], "the last row drops off");

        song.delete_phrase_row(p, 0);
        assert_eq!(vols(&song), [Some(1), None, None]);

        song.insert_phrase_row(p, 3);
        song.delete_phrase_row(p, 3);
        assert_eq!(
            vols(&song),
            [Some(1), None, None],
            "past the end is ignored"
        );
    }

    #[test]
    fn a_duplicated_phrase_is_a_separate_copy_and_uses_count_links() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(2, 4);
        let p = song.add_phrase(2);
        assert_eq!(song.add_phrase(0), 1, "an empty phrase still has a row");
        assert_eq!(song.phrases[1].rows.len(), 1);
        song.phrases[p].rows[1].volume = Some(7);

        let copy = song.duplicate_phrase(p).unwrap();
        assert_eq!(song.phrases[copy].rows, song.phrases[p].rows);
        song.phrases[copy].rows[1].volume = None;
        assert_eq!(song.phrases[p].rows[1].volume, Some(7));
        assert!(song.duplicate_phrase(99).is_none());

        song.link_column(0, 0, p, PhraseTiming::Aligned);
        song.link_column(0, 1, p, PhraseTiming::Running);
        assert_eq!((song.phrase_uses(p), song.phrase_uses(copy)), (2, 0));
    }

    fn link_channel(song: &mut Song, pattern: usize, ch: usize, l: crate::tracker::PhraseLink) {
        let links = &mut song.patterns[pattern].links;
        links.resize(ch + 1, None);
        links[ch] = Some(l);
    }

    #[test]
    fn an_aligned_link_loops_its_phrase_inside_the_pattern() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(2, 8);
        song.phrases.push(phrase(3));
        link_channel(&mut song, 0, 1, link(0, PhraseTiming::Aligned));
        assert_eq!(
            song.address(0, 4, 1),
            Some(CellAddr::Phrase { phrase: 0, row: 1 })
        );
        // Writing through the grid edits the phrase, so it shows at every
        // row that plays that phrase row.
        song.set_cell(
            0,
            4,
            1,
            Cell {
                volume: Some(7),
                ..Cell::default()
            },
        );
        assert_eq!(song.phrases[0].rows[1].volume, Some(7));
        assert_eq!(song.cell_at(0, 1, 1).volume, Some(7));
        assert_eq!(song.cell_at(0, 7, 1).volume, Some(7));
        // The unlinked channel is untouched.
        assert_eq!(
            song.address(0, 4, 0)
                .map(|a| matches!(a, CellAddr::Pattern { .. })),
            Some(true)
        );
    }

    #[test]
    fn a_running_link_shows_its_first_pass() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(1, 4);
        song.add_order_entry();
        song.order_repeats = vec![2, 1];
        song.phrases.push(phrase(3));
        link_channel(&mut song, 1, 0, link(0, PhraseTiming::Running));
        // Order entry 1 is first reached after 4 rows played twice.
        assert_eq!(song.first_pass_row(1), 8);
        assert_eq!(
            song.address(1, 0, 0),
            Some(CellAddr::Phrase { phrase: 0, row: 2 })
        );
        assert_eq!(
            song.address(1, 1, 0),
            Some(CellAddr::Phrase { phrase: 0, row: 0 })
        );
    }

    #[test]
    fn a_link_to_a_missing_phrase_reads_the_pattern() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(1, 4);
        link_channel(&mut song, 0, 0, link(5, PhraseTiming::Aligned));
        assert!(matches!(
            song.address(0, 0, 0),
            Some(CellAddr::Pattern { .. })
        ));
    }

    #[test]
    fn linking_a_column_keeps_what_it_shows_and_unlinking_keeps_it_too() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(2, 4);
        let vol = |v| Cell {
            volume: Some(v),
            ..Cell::default()
        };
        for r in 0..4 {
            song.set_cell(0, r, 1, vol(r as u8 + 1));
        }
        let phrase = song.link_new_phrase(0, 1, PhraseTiming::Aligned).unwrap();
        assert_eq!(song.phrases[phrase].rows.len(), 4);
        assert_eq!(song.cell_at(0, 2, 1).volume, Some(3));

        // Shorten the phrase: the column now loops two rows.
        song.resize_phrase(phrase, 2);
        assert_eq!(song.cell_at(0, 3, 1).volume, Some(2));

        assert!(song.unlink_column(0, 1));
        assert_eq!(song.patterns[0].link(1), None);
        let shown: Vec<_> = (0..4).map(|r| song.cell_at(0, r, 1).volume).collect();
        assert_eq!(shown, vec![Some(1), Some(2), Some(1), Some(2)]);
        assert!(!song.unlink_column(0, 1), "nothing left to unlink");
    }

    #[test]
    fn a_second_column_can_share_a_phrase() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(2, 4);
        let phrase = song.link_new_phrase(0, 0, PhraseTiming::Aligned).unwrap();
        assert!(song.link_column(0, 1, phrase, PhraseTiming::Running));
        song.set_cell(
            0,
            1,
            0,
            Cell {
                volume: Some(9),
                ..Cell::default()
            },
        );
        assert_eq!(
            song.cell_at(0, 1, 1).volume,
            Some(9),
            "both columns show the edit"
        );
        assert!(
            !song.link_column(0, 1, 7, PhraseTiming::Aligned),
            "no phrase 7"
        );
        song.resize_phrase(phrase, 0);
        assert_eq!(song.phrases[phrase].rows.len(), 1, "a phrase keeps one row");
    }

    #[test]
    fn a_link_transposes_playback_but_not_the_grid() {
        use crate::theory::{Scale, ScaleSetting};
        use crate::tracker::{NoteValue, PhraseTiming};
        let mut song = Song::new(1, 4);
        song.phrases.push(phrase(1));
        song.phrases[0].rows[0].note = Some(Note::On {
            value: NoteValue::E,
            octave: 4,
        });
        link_channel(&mut song, 0, 0, link(0, PhraseTiming::Aligned));
        song.set_link_transpose(0, 0, 1);
        let shown = song.cell_at(0, 0, 0).note;
        let played = |song: &Song| song.playback_cell(0, 0, 0, 0).note;

        // Semitones without a scale: E -> F.
        assert_eq!(
            played(&song),
            Some(Note::On {
                value: NoteValue::F,
                octave: 4
            })
        );
        assert_eq!(
            song.cell_at(0, 0, 0).note,
            shown,
            "the grid shows the stored note"
        );

        // Scale degrees with one, as the transpose keys: E -> F in C major,
        // but E -> F# in G major.
        song.scale = Some(ScaleSetting::new(NoteValue::G, Scale::Major));
        assert_eq!(
            played(&song),
            Some(Note::On {
                value: NoteValue::Fs,
                octave: 4
            })
        );

        song.set_link_transpose(0, 0, 100);
        assert_eq!(
            song.patterns[0].link(0).unwrap().transpose,
            MAX_LINK_TRANSPOSE
        );
    }

    #[test]
    fn an_out_of_range_transpose_is_clamped_on_load() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(1, 4);
        song.phrases.push(phrase(1));
        link_channel(&mut song, 0, 0, link(0, PhraseTiming::Aligned));
        song.patterns[0].links[0].as_mut().unwrap().transpose = i8::MIN;
        assert_eq!(song.repair().len(), 1);
        assert_eq!(
            song.patterns[0].link(0).unwrap().transpose,
            -MAX_LINK_TRANSPOSE
        );
    }

    #[test]
    fn an_old_swing_becomes_the_same_groove() {
        let old = |swing: &str| {
            let json = format!(
                r#"{{"title":"t","bpm":120,"speed":6,"patterns":[],"order":[0],"channels":1,"rows_per_pattern":4{swing}}}"#
            );
            let mut song: Song = serde_json::from_str(&json).unwrap();
            song.repair();
            song
        };
        assert_eq!(old(r#","swing":67"#).groove, vec![67, 33]);
        assert!(old(r#","swing":50"#).groove.is_empty(), "50 was straight");
        assert!(old("").groove.is_empty());
        let both = old(r#","swing":67,"groove":[2,1,1]"#);
        assert_eq!(both.groove, vec![2, 1, 1], "a saved groove wins");
        let json = serde_json::to_string(&old(r#","swing":67"#)).unwrap();
        assert!(
            !json.contains("swing"),
            "swing is no longer written: {json}"
        );
    }

    /// The groove times swung rows as swing did, to the sample: rendering
    /// truncates each tick to whole frames, so equal-but-for-rounding would
    /// still shift notes.
    #[test]
    fn a_swing_groove_times_ticks_as_swing_did() {
        for swing in 1u8..=99 {
            for bpm in [32u16, 90, 120, 128, 175, 300] {
                let mut song = Song::new(1, 4);
                song.bpm = bpm;
                song.groove = Song::swing_groove(swing);
                let base = song.seconds_per_tick();
                for row in 0..2 {
                    // The old engine left swing 50 unscaled.
                    let old = if swing == 50 {
                        base
                    } else if row == 0 {
                        base * swing as f64 / 50.0
                    } else {
                        base * (100.0 - swing as f64) / 50.0
                    };
                    let new = song.grooved(base, row);
                    for sr in [44100.0, 48000.0] {
                        assert_eq!(
                            (old * sr) as usize,
                            (new * sr) as usize,
                            "swing {swing} bpm {bpm} row {row}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn grooves_parse_from_text_and_reject_bad_weights() {
        assert_eq!(Song::parse_groove("67").unwrap(), vec![67, 33]);
        assert_eq!(Song::parse_groove("2, 1, 1").unwrap(), vec![2, 1, 1]);
        assert!(Song::parse_groove("off").unwrap().is_empty());
        assert!(Song::parse_groove("50").unwrap().is_empty());
        assert!(Song::parse_groove("3 0").is_err());
        assert!(Song::parse_groove("3 x").is_err());
        assert!(Song::parse_groove(&"1 ".repeat(17)).is_err());
    }

    #[test]
    fn phrases_and_links_round_trip() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(4, 16);
        song.phrases.push(phrase(3));
        song.phrases.push(phrase(4));
        song.patterns[0].links = vec![
            None,
            Some(link(0, PhraseTiming::Aligned)),
            None,
            Some(link(1, PhraseTiming::Running)),
        ];
        let json = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(back.phrases, song.phrases);
        assert_eq!(back.patterns[0].links, song.patterns[0].links);
        assert_eq!(
            back.patterns[0].link(3).unwrap().timing,
            PhraseTiming::Running
        );
        assert_eq!(back.patterns[0].link(0), None);
    }

    #[test]
    fn a_song_without_phrases_writes_no_phrase_fields() {
        let json = serde_json::to_string(&Song::new(2, 4)).unwrap();
        assert!(!json.contains("phrases"), "{json}");
        assert!(!json.contains("links"), "{json}");
    }

    #[test]
    fn bad_phrases_and_links_are_repaired() {
        use crate::tracker::PhraseTiming;
        let mut song = Song::new(2, 16);
        song.phrases.push(phrase(0));
        song.phrases.push(phrase(MAX_ROWS_PER_PATTERN + 5));
        song.patterns[0].links = vec![
            Some(link(0, PhraseTiming::Running)),
            Some(link(7, PhraseTiming::Aligned)),
            Some(link(1, PhraseTiming::Aligned)),
        ];
        let repairs = song.repair();
        assert_eq!(
            song.phrases[0].rows.len(),
            1,
            "a running phrase cannot be empty"
        );
        assert_eq!(song.phrases[1].rows.len(), MAX_ROWS_PER_PATTERN);
        assert!(song.patterns[0].link(0).is_some());
        assert_eq!(song.patterns[0].link(1), None, "linked to a missing phrase");
        assert_eq!(
            song.patterns[0].links.len(),
            2,
            "a link past the last channel"
        );
        assert_eq!(repairs.len(), 3, "{repairs:?}");
    }

    #[test]
    fn test_songfile_roundtrip_with_instruments() {
        let mut song = Song::new(4, 16);
        song.title = "WithInstruments".to_string();

        let song_file = SongFile {
            instruments: vec![
                InstrumentEntry {
                    slot: 0,
                    def: InstrumentDef::from_instrument(&Instrument {
                        name: "Kick".to_string(),
                        kind: InstrumentKind::sample(0),
                        pitch_bend_range: None,
                    }),
                },
                InstrumentEntry {
                    slot: 5,
                    def: InstrumentDef::from_instrument(&Instrument {
                        name: "Lead".to_string(),
                        kind: InstrumentKind::Midi {
                            program: 80,
                            channel: None,
                            output: crate::types::MidiOutput::Builtin,
                        },
                        pitch_bend_range: None,
                    }),
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
        let (kick, _) = loaded.instruments[0].def.to_instrument(0);
        assert_eq!(kick.sample_index(), Some(0));
        assert_eq!(loaded.instruments[1].slot, 5);
        let (lead, repairs) = loaded.instruments[1].def.to_instrument(5);
        assert_eq!(lead.program(), Some(80));
        assert!(repairs.is_empty(), "a typed instrument needs no conversion");
        assert_eq!(loaded.sample_refs.len(), 1);
        assert_eq!(loaded.sample_refs[0].slot, 0);
        assert_eq!(loaded.sample_refs[0].sample_ref.base_note, 36);
        assert_eq!(loaded.sample_refs[0].sample_ref.path, "samples/0-kick.wav");

        let _ = std::fs::remove_file(tmp);
    }

    /// The replacement has to be able to read what the deprecated one wrote,
    /// or the deprecation note is pointing people at a dead end.
    #[test]
    #[allow(deprecated)]
    fn a_bare_song_file_still_loads_through_songfile() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bare.rtrk");

        let mut song = Song::new(3, 16);
        song.title = "Bare".to_string();
        song.bpm = 145;
        song.save(&path).expect("save");

        let mut file = SongFile::load(&path).expect("SongFile should read a bare song");
        assert_eq!(file.song.title, "Bare");
        assert_eq!(file.song.bpm, 145);
        assert_eq!(file.song.channels, 3);
        // No version field was written, so it reads as the pre-versioning 0.
        assert_eq!(file.version, 0);
        assert!(!file.is_from_newer_version());
        // And the recommended follow-up is safe on it.
        assert!(file.song.repair().is_empty(), "a bare song needs no repair");
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
