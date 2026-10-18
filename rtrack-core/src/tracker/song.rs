use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::{MAX_CHANNELS, MAX_PATTERNS, MAX_ROWS_PER_PATTERN, MIDI_CLOCKS_PER_BEAT};

use super::{Cell, ColumnAlias, Note, Pattern, Table, MAX_TABLES};
use crate::audio::channel_effects::InstrumentFx;
use crate::audio::effects::SendBusParams;
use crate::audio::synth::SynthParams;
use crate::fs::write_atomic;
use crate::modulation::{ModSlot, MOD_SLOTS};
use crate::theory::ScaleSetting;
use crate::types::{ChannelConfig, Instrument, InstrumentKind, MidiCcMapping};

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
    /// Absent in format version 1 files, which carry the legacy fields below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InstrumentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_bend_range: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<u8>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fixed_pitch: bool,
    #[serde(default, skip_serializing_if = "is_default_fx")]
    pub fx: InstrumentFx,
    #[serde(default, skip_serializing_if = "no_mods")]
    pub mods: [Option<ModSlot>; MOD_SLOTS],
    #[serde(default, rename = "midi_program", skip_serializing)]
    legacy_midi_program: Option<u8>,
    #[serde(default, rename = "sample_index", skip_serializing)]
    legacy_sample_index: Option<usize>,
    #[serde(default, rename = "synth_params", skip_serializing)]
    legacy_synth_params: Option<SynthParams>,
}

fn no_mods(mods: &[Option<ModSlot>; MOD_SLOTS]) -> bool {
    mods.iter().all(Option::is_none)
}

fn is_default_fx(fx: &InstrumentFx) -> bool {
    *fx == InstrumentFx::default()
}

impl InstrumentDef {
    /// Describe an instrument for saving.
    pub fn from_instrument(inst: &Instrument) -> Self {
        Self {
            name: inst.name.clone(),
            kind: Some(inst.kind.clone()),
            pitch_bend_range: inst.pitch_bend_range,
            table: inst.table,
            fixed_pitch: inst.fixed_pitch,
            fx: inst.fx,
            mods: inst.mods,
            legacy_midi_program: None,
            legacy_sample_index: None,
            legacy_synth_params: None,
        }
    }

    /// Build the instrument, migrating version 1 fields when `kind` is absent.
    ///
    /// Version 1 routed by precedence: a sample, then synth parameters, then
    /// the slot's preset patch. A MIDI program alone never reached a device,
    /// so it migrates to `Preset` and is returned as a note for the load
    /// report.
    pub fn into_instrument(self) -> (Instrument, Option<String>) {
        let mut note = None;
        let kind = match self.kind {
            Some(kind) => kind,
            None => {
                if let Some(slot) = self.legacy_sample_index {
                    InstrumentKind::Sample { slot }
                } else if let Some(params) = self.legacy_synth_params {
                    InstrumentKind::Synth { params }
                } else {
                    if let Some(program) = self.legacy_midi_program {
                        note = Some(format!(
                            "instrument \"{}\": MIDI program {} dropped; set the instrument type to MIDI to use it",
                            self.name, program
                        ));
                    }
                    InstrumentKind::Preset
                }
            }
        };
        (
            Instrument {
                name: self.name,
                kind,
                pitch_bend_range: self.pitch_bend_range,
                table: self.table,
                fixed_pitch: self.fixed_pitch,
                fx: self.fx,
                mods: self.mods,
            },
            note,
        )
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
/// (serde default kicks in for the optional fields), and unknown fields are
/// ignored. A file from a newer format version is refused; see
/// [`SongFile::load`].
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

/// Most entries a groove can hold.
pub const MAX_GROOVE_STEPS: usize = 16;

/// Current .rtrk format version, written into every saved file.
///
/// Files predating the field deserialize as version 0.
///
/// - 2: typed instruments (`kind`); channels lose `channel_type` and
///   `midi_channel`; cells hold two named effects (`fx`) in place of
///   `effect` and `effect_value`.
/// - 3: `groove` replaces `swing`.
pub const FORMAT_VERSION: u32 = 3;

impl SongFile {
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
    ///
    /// A file from a newer format version is refused rather than loaded in
    /// part: what this build cannot read would be lost on the next save.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        // Checked before the full parse, which may fail on newer content
        // with a less useful message.
        #[derive(Deserialize)]
        struct Version {
            #[serde(default)]
            version: u32,
        }
        if let Ok(Version { version }) = serde_json::from_str::<Version>(&data) {
            if version > FORMAT_VERSION {
                anyhow::bail!(
                    "written by a newer rtrack (format {version}; this version reads up to {FORMAT_VERSION}). Update rtrack to open it"
                );
            }
        }
        let song_file: SongFile =
            serde_json::from_str(&data).context("Failed to parse song file")?;
        Ok(song_file)
    }
}

/// What [`Song::edit_cell`] stored, and where: the stored cells either side,
/// for an undo record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellWrite {
    pub pattern: usize,
    pub row: usize,
    pub channel: usize,
    pub old: Cell,
    pub new: Cell,
}

/// Why [`Song::set_alias`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasError {
    NoSuchPattern,
    SelfAlias,
    /// The source column is an alias, or this column is a source.
    Chain,
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AliasError::NoSuchPattern => "no such pattern or channel",
            AliasError::SelfAlias => "a pattern cannot alias itself",
            AliasError::Chain => {
                "aliases cannot chain: the source is an alias, or this column is aliased"
            }
        })
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
    /// Ticks per row, cycling by row within the pattern (M8 groove,
    /// `docs/dev/design-gaps.md` G9). Empty: every row takes `speed` ticks.
    /// `[7, 5]` is a swing; `[4, 4, 4]` against 4 rows of 6 is a triplet feel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groove: Vec<u8>,
    /// `swing` from files before format version 3, converted to a groove by
    /// `repair`.
    #[serde(default, rename = "swing", skip_serializing)]
    pub(crate) legacy_swing: Option<u8>,
    /// Tempo automation points (order, row, bpm)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_map: Vec<TempoPoint>,
    /// Scale constraining keyboard note entry. `None` means chromatic entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleSetting>,
    /// Tables, referenced by index from instruments and `TBL`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
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
            tables: Vec::new(),
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
        patterns
            + self.patterns.capacity() * std::mem::size_of::<Pattern>()
            + self.order.capacity() * std::mem::size_of::<usize>()
            + self.order_repeats.capacity()
            + self.tempo_map.capacity() * std::mem::size_of::<TempoPoint>()
            + self.tables.capacity() * std::mem::size_of::<Table>()
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

    /// True when the song holds [`MAX_PATTERNS`] patterns. Editors check
    /// this before adding one: a song past the cap loses the rest on load.
    pub fn at_pattern_limit(&self) -> bool {
        self.patterns.len() >= MAX_PATTERNS
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

    /// Ticks the given pattern row takes: the groove's entry for it, or
    /// `speed` with no groove.
    pub fn ticks_for_row(&self, row: usize) -> u8 {
        match self.groove.len() {
            0 => self.speed.max(1),
            n => self.groove[row % n].max(1),
        }
    }

    /// The groove as typed in a settings field: ticks separated by spaces,
    /// or `off`.
    pub fn groove_text(&self) -> String {
        if self.groove.is_empty() {
            return "off".to_string();
        }
        let ticks: Vec<String> = self.groove.iter().map(u8::to_string).collect();
        ticks.join(" ")
    }

    /// Parse [`Song::groove_text`]'s form: up to 16 tick counts of 1-255,
    /// or `off` (or nothing) for no groove.
    pub fn parse_groove(text: &str) -> Result<Vec<u8>, String> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("off") {
            return Ok(Vec::new());
        }
        let ticks = text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .map(|t| match t.parse::<u8>() {
                Ok(0) | Err(_) => Err(format!("{t:?} is not 1-255 ticks")),
                Ok(n) => Ok(n),
            })
            .collect::<Result<Vec<u8>, String>>()?;
        if ticks.len() > MAX_GROOVE_STEPS {
            return Err(format!("a groove has at most {MAX_GROOVE_STEPS} steps"));
        }
        Ok(ticks)
    }

    /// The groove nearest an old `swing` setting at `speed`.
    ///
    /// Swing gave even rows `swing`% of a pair of rows. A groove counts
    /// whole ticks, so the pair `2 * speed` ticks long is split as near that
    /// share as it goes, keeping at least one tick on each side. Swing 50 is
    /// no groove.
    pub fn groove_from_swing(speed: u8, swing: u8) -> Vec<u8> {
        if swing == 50 {
            return Vec::new();
        }
        let total = 2 * speed.max(1) as u32;
        let even = ((total * swing.min(100) as u32 + 50) / 100).clamp(1, total.max(2) - 1);
        vec![even as u8, (total - even) as u8]
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

    /// Where the cell at (`pattern`, `row`, `channel`) is stored, and the
    /// transpose a read applies. An aliased column resolves to its source
    /// column, looping a shorter source. `None` if out of range.
    pub fn resolve(
        &self,
        pattern: usize,
        row: usize,
        channel: usize,
    ) -> Option<(usize, usize, i8)> {
        let p = self.patterns.get(pattern)?;
        if row >= p.rows || channel >= p.channels {
            return None;
        }
        match p.alias(channel) {
            Some(alias) => {
                let src = self.patterns.get(alias.source)?;
                if channel >= src.channels || src.rows == 0 {
                    return None;
                }
                Some((alias.source, row % src.rows, alias.transpose))
            }
            None => Some((pattern, row, 0)),
        }
    }

    /// The cell at (`pattern`, `row`, `channel`) as it plays: an aliased
    /// column reads its source, transposed. Empty when out of range.
    pub fn cell(&self, pattern: usize, row: usize, channel: usize) -> Cell {
        match self.resolve(pattern, row, channel) {
            Some((p, r, t)) => self.patterns[p].data[r][channel].transposed(t),
            None => Cell::default(),
        }
    }

    /// Change the cell at (`pattern`, `row`, `channel`) as it plays.
    ///
    /// `f` sees the cell as [`Song::cell`] returns it. An aliased column
    /// stores the change in its source, transposed back. Returns what was
    /// stored where, for undo; `None` if the position is out of range.
    pub fn edit_cell(
        &mut self,
        pattern: usize,
        row: usize,
        channel: usize,
        f: impl FnOnce(&mut Cell),
    ) -> Option<CellWrite> {
        let (p, r, t) = self.resolve(pattern, row, channel)?;
        let old = self.patterns[p].data[r][channel];
        let mut view = old.transposed(t);
        f(&mut view);
        let new = view.transposed(t.wrapping_neg());
        self.patterns[p].data[r][channel] = new;
        Some(CellWrite {
            pattern: p,
            row: r,
            channel,
            old,
            new,
        })
    }

    /// The stored cell at an order position, untransposed, and the transpose
    /// an alias applies to it. The engine uses this so an instrument with
    /// `fixed_pitch` can refuse the transpose.
    pub fn played_cell(&self, order_idx: usize, row: usize, channel: usize) -> (Cell, i8) {
        let resolved = self
            .order
            .get(order_idx)
            .and_then(|&p| self.resolve(p, row, channel));
        match resolved {
            Some((p, r, t)) => (self.patterns[p].data[r][channel], t),
            None => (Cell::default(), 0),
        }
    }

    /// Read a cell by order position. Returns an empty cell for any
    /// out-of-range coordinate rather than panicking.
    pub fn cell_at(&self, order_idx: usize, row: usize, channel: usize) -> Cell {
        match self.order.get(order_idx) {
            Some(&p) => self.cell(p, row, channel),
            None => Cell::default(),
        }
    }

    /// Write a cell by order position. Out-of-range coordinates are ignored.
    pub fn set_cell(&mut self, order_idx: usize, row: usize, channel: usize, cell: Cell) {
        if let Some(&p) = self.order.get(order_idx) {
            self.edit_cell(p, row, channel, |c| *c = cell);
        }
    }

    // -------------------------------------------------------------------
    // Column aliases
    // -------------------------------------------------------------------

    /// Make `channel` of `pattern` play the same channel of `alias.source`.
    ///
    /// The column's own cells are cleared: an aliased column stores nothing,
    /// so the source is the only copy. Refused for a missing or identical
    /// source, and for chains: the source column may not be an alias, and a
    /// column other patterns alias may not become one.
    pub fn set_alias(
        &mut self,
        pattern: usize,
        channel: usize,
        alias: ColumnAlias,
    ) -> Result<(), AliasError> {
        let p = self
            .patterns
            .get(pattern)
            .ok_or(AliasError::NoSuchPattern)?;
        if channel >= p.channels {
            return Err(AliasError::NoSuchPattern);
        }
        let src = self
            .patterns
            .get(alias.source)
            .ok_or(AliasError::NoSuchPattern)?;
        if alias.source == pattern {
            return Err(AliasError::SelfAlias);
        }
        if channel >= src.channels || src.alias(channel).is_some() {
            return Err(AliasError::Chain);
        }
        if self.is_alias_source(pattern, channel) {
            return Err(AliasError::Chain);
        }
        let p = &mut self.patterns[pattern];
        for row in &mut p.data {
            row[channel] = Cell::default();
        }
        if p.aliases.len() <= channel {
            p.aliases.resize(channel + 1, None);
        }
        p.aliases[channel] = Some(alias);
        Ok(())
    }

    /// Give an aliased column its own copy of what it plays, and drop the
    /// alias. Does nothing to a column that is not an alias.
    pub fn unalias(&mut self, pattern: usize, channel: usize) {
        let Some(p) = self.patterns.get(pattern) else {
            return;
        };
        if p.alias(channel).is_none() {
            return;
        }
        let cells: Vec<Cell> = (0..p.rows)
            .map(|r| self.cell(pattern, r, channel))
            .collect();
        let p = &mut self.patterns[pattern];
        p.aliases[channel] = None;
        for (row, cell) in p.data.iter_mut().zip(cells) {
            row[channel] = cell;
        }
    }

    /// Whether any pattern aliases `channel` of `pattern`.
    pub fn is_alias_source(&self, pattern: usize, channel: usize) -> bool {
        self.patterns
            .iter()
            .any(|p| p.alias(channel).is_some_and(|a| a.source == pattern))
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

        // Before the geometry pass below, which allocates every pattern in
        // full. Order entries and aliases naming a dropped pattern are
        // repaired further down.
        if self.patterns.len() > MAX_PATTERNS {
            repairs.push(format!(
                "dropped {} patterns past the maximum of {}",
                self.patterns.len() - MAX_PATTERNS,
                MAX_PATTERNS
            ));
            self.patterns.truncate(MAX_PATTERNS);
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

        // Aliases must name another pattern, whose column is not an alias.
        let snapshot: Vec<Vec<Option<ColumnAlias>>> =
            self.patterns.iter().map(|p| p.aliases.clone()).collect();
        let channels: Vec<usize> = self.patterns.iter().map(|p| p.channels).collect();
        for (i, pattern) in self.patterns.iter_mut().enumerate() {
            for (ch, slot) in pattern.aliases.iter_mut().enumerate() {
                let Some(alias) = *slot else { continue };
                let valid = alias.source != i
                    && alias.source < snapshot.len()
                    && ch < channels[alias.source]
                    && snapshot[alias.source].get(ch).copied().flatten().is_none();
                if !valid {
                    *slot = None;
                    repairs.push(format!(
                        "pattern {i} channel {ch}: alias to pattern {} dropped",
                        alias.source
                    ));
                }
            }
        }

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
        if let Some(swing) = self.legacy_swing.take() {
            if swing != 50 && self.groove.is_empty() {
                self.groove = Song::groove_from_swing(self.speed, swing);
                repairs.push(format!(
                    "swing {swing} converted to groove {:?}",
                    self.groove
                ));
            }
        }
        if self.groove.len() > MAX_GROOVE_STEPS || self.groove.contains(&0) {
            self.groove.truncate(MAX_GROOVE_STEPS);
            for t in &mut self.groove {
                *t = (*t).max(1);
            }
            repairs.push("groove clamped to 16 steps of 1 or more ticks".to_string());
        }

        if self.tables.len() > MAX_TABLES {
            repairs.push(format!(
                "dropped {} tables past the maximum of {}",
                self.tables.len() - MAX_TABLES,
                MAX_TABLES
            ));
            self.tables.truncate(MAX_TABLES);
        }
        for (i, table) in self.tables.iter_mut().enumerate() {
            if table.repair() {
                repairs.push(format!(
                    "table {i}: length or tick rate out of range, clamped"
                ));
            }
        }

        repairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::ScaleSetting;
    use crate::tracker::{AliasError, ColumnAlias, Fx, FxCmd, Pattern};
    use crate::tracker::{Note, NoteValue};

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
                fx: Fx::only(FxCmd::Portamento, 0xFF),
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
        assert_eq!(cell.fx, Fx::only(FxCmd::Portamento, 0xFF));

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
                    def: InstrumentDef::from_instrument(&Instrument {
                        name: "Kick".to_string(),
                        kind: InstrumentKind::Sample { slot: 0 },
                        pitch_bend_range: None,
                        ..Default::default()
                    }),
                },
                InstrumentEntry {
                    slot: 5,
                    def: InstrumentDef::from_instrument(&Instrument {
                        name: "Lead".to_string(),
                        kind: InstrumentKind::Midi(crate::types::MidiOut {
                            channel: 3,
                            program: Some(80),
                        }),
                        pitch_bend_range: None,
                        ..Default::default()
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
        let (kick, _) = loaded.instruments[0].def.clone().into_instrument();
        assert_eq!(kick.sample_slot(), Some(0));
        assert_eq!(loaded.instruments[1].slot, 5);
        let (lead, _) = loaded.instruments[1].def.clone().into_instrument();
        assert_eq!(
            lead.midi_out(),
            Some(crate::types::MidiOut {
                channel: 3,
                program: Some(80)
            })
        );
        assert_eq!(loaded.sample_refs.len(), 1);
        assert_eq!(loaded.sample_refs[0].slot, 0);
        assert_eq!(loaded.sample_refs[0].sample_ref.base_note, 36);
        assert_eq!(loaded.sample_refs[0].sample_ref.path, "samples/0-kick.wav");

        let _ = std::fs::remove_file(tmp);
    }

    fn c(octave: u8) -> Cell {
        Cell {
            note: Some(Note::On {
                value: crate::tracker::NoteValue::C,
                octave,
            }),
            ..Cell::default()
        }
    }

    /// Pattern 0: 4 rows. Pattern 1: 8 rows, channel 1 aliased to pattern 0.
    fn aliased_song(transpose: i8) -> Song {
        let mut song = Song::new(2, 4);
        song.patterns.push(Pattern::new(8, 2));
        song.patterns[0].data[0][1] = c(4);
        song.patterns[0].data[2][1] = c(5);
        song.set_alias(
            1,
            1,
            ColumnAlias {
                source: 0,
                transpose,
            },
        )
        .unwrap();
        song
    }

    #[test]
    fn an_alias_reads_its_source_and_loops_a_shorter_one() {
        let song = aliased_song(0);
        assert_eq!(song.cell(1, 0, 1), c(4));
        assert_eq!(song.cell(1, 2, 1), c(5));
        assert_eq!(song.cell(1, 4, 1), c(4), "row 4 loops to source row 0");
        assert_eq!(song.cell(1, 6, 1), c(5));
        assert_eq!(song.cell(1, 0, 0), Cell::default(), "channel 0 is its own");
    }

    #[test]
    fn an_alias_transposes_reads_and_untransposes_writes() {
        let mut song = aliased_song(12);
        assert_eq!(song.cell(1, 0, 1), c(5));
        let w = song.edit_cell(1, 5, 1, |cell| *cell = c(6)).unwrap();
        assert_eq!((w.pattern, w.row), (0, 1), "stored in the source, looped");
        assert_eq!(song.patterns[0].data[1][1], c(5), "stored an octave down");
        assert_eq!(song.cell(1, 5, 1), c(6));
    }

    #[test]
    fn edits_through_either_side_of_an_alias_are_one_edit() {
        let mut song = aliased_song(0);
        song.edit_cell(0, 3, 1, |cell| *cell = c(3));
        assert_eq!(song.cell(1, 3, 1), c(3));
        assert_eq!(song.cell(1, 7, 1), c(3));
    }

    #[test]
    fn aliases_cannot_chain_or_point_at_themselves() {
        let mut song = aliased_song(0);
        song.patterns.push(Pattern::new(4, 2));
        let to = |source| ColumnAlias {
            source,
            transpose: 0,
        };
        assert_eq!(song.set_alias(2, 1, to(1)), Err(AliasError::Chain));
        assert_eq!(song.set_alias(0, 1, to(2)), Err(AliasError::Chain));
        assert_eq!(song.set_alias(2, 1, to(2)), Err(AliasError::SelfAlias));
        assert_eq!(song.set_alias(2, 1, to(9)), Err(AliasError::NoSuchPattern));
        assert_eq!(song.set_alias(2, 1, to(0)), Ok(()));
    }

    #[test]
    fn unalias_keeps_what_the_column_played() {
        let mut song = aliased_song(12);
        song.unalias(1, 1);
        assert_eq!(song.patterns[1].alias(1), None);
        assert_eq!(song.patterns[1].data[4][1], c(5), "looped and transposed");
        song.edit_cell(0, 0, 1, |cell| *cell = Cell::default());
        assert_eq!(song.cell(1, 0, 1), c(5), "no longer follows the source");
    }

    #[test]
    fn repair_drops_aliases_that_chain_or_dangle() {
        let mut song = aliased_song(0);
        song.patterns[0].aliases = vec![
            None,
            Some(ColumnAlias {
                source: 1,
                transpose: 0,
            }),
        ];
        song.patterns[1].aliases.push(Some(ColumnAlias {
            source: 7,
            transpose: 0,
        }));
        let repairs = song.repair();
        assert_eq!(repairs.len(), 2, "{repairs:?}");
        assert_eq!(
            song.patterns[1].alias(1),
            None,
            "its source became an alias"
        );
    }

    #[test]
    fn aliases_round_trip_and_unaliased_patterns_write_none() {
        let song = aliased_song(-3);
        let json = serde_json::to_string(&song).unwrap();
        assert_eq!(json.matches("aliases").count(), 1, "{json}");
        let loaded: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.patterns[1].alias(1),
            Some(ColumnAlias {
                source: 0,
                transpose: -3
            })
        );
    }

    #[test]
    fn swing_converts_to_the_nearest_tick_pair() {
        assert_eq!(Song::groove_from_swing(6, 50), Vec::<u8>::new());
        assert_eq!(Song::groove_from_swing(6, 75), vec![9, 3]);
        assert_eq!(Song::groove_from_swing(6, 58), vec![7, 5]);
        assert_eq!(
            Song::groove_from_swing(1, 100),
            vec![1, 1],
            "keeps a tick each"
        );
        assert_eq!(Song::groove_from_swing(3, 0), vec![1, 5]);
    }

    #[test]
    fn an_old_swing_setting_becomes_a_groove_on_repair() {
        let json = r#"{"title":"s","bpm":120,"speed":6,"patterns":[],"order":[],
            "channels":1,"rows_per_pattern":4,"swing":67}"#;
        let mut song: Song = serde_json::from_str(json).unwrap();
        let repairs = song.repair();
        assert_eq!(song.groove, vec![8, 4]);
        assert!(
            repairs.iter().any(|r| r.contains("swing 67")),
            "{repairs:?}"
        );
        let json = serde_json::to_string(&song).unwrap();
        assert!(!json.contains("swing"), "{json}");
    }

    fn migrate(json: &str) -> (Instrument, Option<String>) {
        serde_json::from_str::<InstrumentDef>(json)
            .unwrap()
            .into_instrument()
    }

    #[test]
    fn version_1_instruments_migrate_by_routing_precedence() {
        // A sample won over synth parameters.
        let (inst, note) =
            migrate(r#"{"name":"a","sample_index":4,"synth_params":null,"midi_program":9}"#);
        assert_eq!(inst.sample_slot(), Some(4));
        assert_eq!(note, None);

        let (inst, _) = migrate(
            &serde_json::json!({
                "name": "b",
                "synth_params": SynthParams::from_patch(3),
            })
            .to_string(),
        );
        assert!(matches!(inst.kind, InstrumentKind::Synth { .. }));

        let (inst, note) = migrate(r#"{"name":"c"}"#);
        assert!(matches!(inst.kind, InstrumentKind::Preset));
        assert_eq!(note, None);
    }

    #[test]
    fn a_version_1_midi_program_alone_is_reported_not_silently_dropped() {
        let (inst, note) = migrate(r#"{"name":"strings","midi_program":48}"#);
        assert!(matches!(inst.kind, InstrumentKind::Preset));
        let note = note.expect("a dropped program is reported");
        assert!(note.contains("strings") && note.contains("48"), "{note}");
    }

    #[test]
    fn typed_instruments_write_no_legacy_fields() {
        let def = InstrumentDef::from_instrument(&Instrument {
            name: "k".into(),
            kind: InstrumentKind::Sample { slot: 2 },
            pitch_bend_range: None,
            ..Default::default()
        });
        let json = serde_json::to_string(&def).unwrap();
        assert!(!json.contains("sample_index"), "{json}");
        assert!(
            json.contains(r#""kind":{"type":"sample","slot":2}"#),
            "{json}"
        );
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
        assert!(song.edit_cell(99, 0, 0, |_| {}).is_none());
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
