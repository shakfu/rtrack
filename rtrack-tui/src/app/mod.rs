mod input;
mod playback;

use std::path::PathBuf;
use std::time::Instant;

use rtrack_core::constants::*;
use rtrack_core::editor::Edit;
use rtrack_core::midi::MidiEngine;

pub use rtrack_core::{
    autosave_path_for, default_channel_configs, make_relative, resolve_relative, ChannelConfig,
    ChannelState, ClockMode, Instrument, LearnableParam, MidiCcMapping, PlaybackTiming,
    AUTOSAVE_INTERVAL_SECS,
};

// -- Constants (module-private; TUI-specific) --

/// Dialog and popup state for all modal UIs.
pub struct DialogState {
    pub settings_field: SettingsField,
    pub settings_edit_buf: String,
    /// Set while the buffer still holds the value the field was seeded with.
    /// The first character typed then replaces it instead of appending.
    pub settings_buf_untouched: bool,
    pub instrument_cursor: usize,
    pub phrase_cursor: usize,
    /// The table open in the table editor, its cursor row and field.
    pub table_number: u8,
    pub table_row: usize,
    pub table_field: TableField,
    pub sample_editor_slot: usize,
    pub sample_editor_field: SampleField,
    pub sample_slice_count: usize,
    pub sample_slice_sensitivity: f32,
    /// Whether a slice action divides the whole sample or just this slot.
    pub sample_slice_range: rtrack_core::sample::SliceRange,
    /// Set when a slice was refused for want of free slots: the next Enter
    /// on the same action goes ahead anyway.
    pub sample_slice_overwrite_armed: bool,
    /// Spans the slice settings would produce, reused across redraws. A
    /// `RefCell` because drawing borrows the app immutably.
    pub sample_slice_plan: std::cell::RefCell<rtrack_core::sample::SlicePlanCache>,
    pub synth_editor_slot: usize,
    pub synth_editor_field: SynthField,
    pub midi_port_list: Vec<String>,
    pub midi_port_cursor: usize,
    pub help_scroll: usize,
    pub file_browser: FileBrowserState,
    pub recent_cursor: usize,
}

impl DialogState {
    fn new() -> Self {
        Self {
            settings_field: SettingsField::Title,
            settings_edit_buf: String::new(),
            settings_buf_untouched: true,
            instrument_cursor: 0,
            phrase_cursor: 0,
            table_number: 0,
            table_row: 0,
            table_field: TableField::Transpose,
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            sample_slice_count: 8,
            sample_slice_sensitivity: 0.5,
            sample_slice_range: rtrack_core::sample::SliceRange::Source,
            sample_slice_overwrite_armed: false,
            sample_slice_plan: Default::default(),
            synth_editor_slot: 0,
            synth_editor_field: SynthField::Waveform,
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            help_scroll: 0,
            file_browser: FileBrowserState::new(),
            recent_cursor: 0,
        }
    }
}

/// Block-selection state: where the selection was anchored, if one is active.
pub type BlockAnchor = Option<(usize, usize)>;

// The cursor's sub-column is shared with the GUI: both frontends walk the
// same four fields with the same wrapping, and keeping two copies is how they
// drift.
pub use rtrack_core::editor::SubColumn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    MidiPortSelect,
    Help,
    SongSettings,
    InstrumentList,
    SampleEditor,
    SynthEditor,
    QuitConfirm,
    TrackConfig,
    PatternMatrix,
    Command,
    FileBrowser,
    RecentFiles,
    PhraseList,
    TableEditor,
}

/// The table editor's field under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableField {
    #[default]
    Transpose,
    Volume,
    Effect,
}

/// What action to perform when a file is selected in the file browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileBrowserAction {
    /// Load a sample into a specific slot
    LoadSample(usize),
    /// Open/load a song file
    OpenSong,
}

/// An entry in the file browser listing.
#[derive(Debug, Clone)]
pub struct FileBrowserEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Self-contained file browser dialog state.
pub struct FileBrowserState {
    pub dir: PathBuf,
    pub entries: Vec<FileBrowserEntry>,
    pub cursor: usize,
    pub action: FileBrowserAction,
    pub filter: Vec<String>,
    pub scroll: usize,
}

impl Default for FileBrowserState {
    fn default() -> Self {
        Self::new()
    }
}

impl FileBrowserState {
    pub fn new() -> Self {
        Self {
            dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            entries: Vec::new(),
            cursor: 0,
            action: FileBrowserAction::OpenSong,
            filter: Vec::new(),
            scroll: 0,
        }
    }

    /// Refresh the entries from the current directory.
    pub fn refresh(&mut self) {
        let mut entries = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(&self.dir) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    entries.push(FileBrowserEntry { name, is_dir: true });
                } else if self.filter.is_empty() {
                    entries.push(FileBrowserEntry {
                        name,
                        is_dir: false,
                    });
                } else {
                    let ext = std::path::Path::new(&name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if self.filter.iter().any(|f| f == &ext) {
                        entries.push(FileBrowserEntry {
                            name,
                            is_dir: false,
                        });
                    }
                }
            }
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        self.entries = entries;
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Open the browser with a specific action and extension filter.
    pub fn open(&mut self, action: FileBrowserAction, extensions: Vec<String>) {
        self.action = action;
        self.filter = extensions;
        self.cursor = 0;
        self.scroll = 0;
        self.refresh();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Title,
    Bpm,
    Speed,
    Channels,
    Rows,
    HighlightBeat,
    HighlightBar,
    Groove,
    Scale,
}

impl SettingsField {
    pub fn next(self) -> Self {
        match self {
            Self::Title => Self::Bpm,
            Self::Bpm => Self::Speed,
            Self::Speed => Self::Channels,
            Self::Channels => Self::Rows,
            Self::Rows => Self::HighlightBeat,
            Self::HighlightBeat => Self::HighlightBar,
            Self::HighlightBar => Self::Groove,
            Self::Groove => Self::Scale,
            Self::Scale => Self::Title,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Title => Self::Scale,
            Self::Bpm => Self::Title,
            Self::Speed => Self::Bpm,
            Self::Channels => Self::Speed,
            Self::Rows => Self::Channels,
            Self::HighlightBeat => Self::Rows,
            Self::HighlightBar => Self::HighlightBeat,
            Self::Groove => Self::HighlightBar,
            Self::Scale => Self::Groove,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleField {
    BaseNote,
    TrimStart,
    TrimEnd,
    LoopEnabled,
    LoopStart,
    LoopEnd,
    Drive,
    Filter,
    Cutoff,
    Resonance,
    SliceCount,
    SliceSensitivity,
    SliceRange,
    SliceEqual,
    SliceTransient,
}

impl SampleField {
    /// Name used to tie a run of edits to this field together for undo.
    ///
    /// A `&'static str` rather than the enum itself so that `rtrack-core`,
    /// which owns the history, needs to know nothing about the TUI's dialogs.
    pub fn undo_control(self) -> &'static str {
        match self {
            Self::BaseNote => "sample.base_note",
            Self::TrimStart => "sample.trim_start",
            Self::TrimEnd => "sample.trim_end",
            Self::LoopEnabled => "sample.loop_enabled",
            Self::LoopStart => "sample.loop_start",
            Self::LoopEnd => "sample.loop_end",
            Self::Drive => "instrument.drive",
            Self::Filter => "instrument.filter",
            Self::Cutoff => "instrument.cutoff",
            Self::Resonance => "instrument.resonance",
            Self::SliceCount => "sample.slice_count",
            Self::SliceSensitivity => "sample.slice_sensitivity",
            Self::SliceRange => "sample.slice_range",
            Self::SliceEqual => "sample.slice_equal",
            Self::SliceTransient => "sample.slice_transient",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::BaseNote => Self::TrimStart,
            Self::TrimStart => Self::TrimEnd,
            Self::TrimEnd => Self::LoopEnabled,
            Self::LoopEnabled => Self::LoopStart,
            Self::LoopStart => Self::LoopEnd,
            Self::LoopEnd => Self::Drive,
            Self::Drive => Self::Filter,
            Self::Filter => Self::Cutoff,
            Self::Cutoff => Self::Resonance,
            Self::Resonance => Self::SliceCount,
            Self::SliceCount => Self::SliceSensitivity,
            Self::SliceSensitivity => Self::SliceRange,
            Self::SliceRange => Self::SliceEqual,
            Self::SliceEqual => Self::SliceTransient,
            Self::SliceTransient => Self::BaseNote,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::BaseNote => Self::SliceTransient,
            Self::TrimStart => Self::BaseNote,
            Self::TrimEnd => Self::TrimStart,
            Self::LoopEnabled => Self::TrimEnd,
            Self::LoopStart => Self::LoopEnabled,
            Self::LoopEnd => Self::LoopStart,
            Self::Drive => Self::LoopEnd,
            Self::Filter => Self::Drive,
            Self::Cutoff => Self::Filter,
            Self::Resonance => Self::Cutoff,
            Self::SliceCount => Self::Resonance,
            Self::SliceSensitivity => Self::SliceCount,
            Self::SliceRange => Self::SliceSensitivity,
            Self::SliceEqual => Self::SliceRange,
            Self::SliceTransient => Self::SliceEqual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthField {
    Waveform,
    Attack,
    Decay,
    Sustain,
    Release,
    Drive,
    FilterType,
    FilterCutoff,
    FilterResonance,
    FilterEnv,
    Detune,
    SubOsc,
    FmRatio,
    FmIndex,
    PulseWidth,
}

impl SynthField {
    pub fn next(self) -> Self {
        match self {
            Self::Waveform => Self::Attack,
            Self::Attack => Self::Decay,
            Self::Decay => Self::Sustain,
            Self::Sustain => Self::Release,
            Self::Release => Self::Drive,
            Self::Drive => Self::FilterType,
            Self::FilterType => Self::FilterCutoff,
            Self::FilterCutoff => Self::FilterResonance,
            Self::FilterResonance => Self::FilterEnv,
            Self::FilterEnv => Self::Detune,
            Self::Detune => Self::SubOsc,
            Self::SubOsc => Self::FmRatio,
            Self::FmRatio => Self::FmIndex,
            Self::FmIndex => Self::PulseWidth,
            Self::PulseWidth => Self::Waveform,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Waveform => Self::PulseWidth,
            Self::Attack => Self::Waveform,
            Self::Decay => Self::Attack,
            Self::Sustain => Self::Decay,
            Self::Release => Self::Sustain,
            Self::Drive => Self::Release,
            Self::FilterType => Self::Drive,
            Self::FilterCutoff => Self::FilterType,
            Self::FilterResonance => Self::FilterCutoff,
            Self::FilterEnv => Self::FilterResonance,
            Self::Detune => Self::FilterEnv,
            Self::SubOsc => Self::Detune,
            Self::FmRatio => Self::SubOsc,
            Self::FmIndex => Self::FmRatio,
            Self::PulseWidth => Self::FmIndex,
        }
    }
}

pub struct App {
    // -----------------------------------------------------------------------
    // Headless core (song, engine, audio, MIDI, channels, instruments)
    // -----------------------------------------------------------------------
    pub core: rtrack_core::TrackerCore,

    // -----------------------------------------------------------------------
    // UI mode
    // -----------------------------------------------------------------------
    pub mode: Mode,
    pub should_quit: bool,

    // -----------------------------------------------------------------------
    // Cursor State
    // -----------------------------------------------------------------------
    pub cursor_row: usize,
    pub cursor_channel: usize,
    pub cursor_sub: SubColumn,
    pub current_octave: u8,
    /// Which group of 4 tracks is visible (0 = tracks 0-3, 1 = tracks 4-7)
    pub track_page: usize,
    /// Cursor follows playback position
    pub follow_playback: bool,
    /// How many rows to advance after entering a note
    pub edit_step: usize,
    /// Show the second effect column even where the pattern does not use it.
    pub show_second_fx: bool,

    // -----------------------------------------------------------------------
    // Editor State
    // -----------------------------------------------------------------------
    /// Copied cells. Shared with the GUI so the two cannot drift.
    pub clipboard: rtrack_core::editor::Clipboard,
    /// Where a block selection was anchored, if one is active.
    pub block_anchor: BlockAnchor,
    /// The phrase shown in the grid in place of the pattern, if any.
    pub phrase_view: Option<rtrack_core::editor::PhraseView>,
    /// Channel rename edit buffer
    pub rename_buf: String,
    /// Channel effects editor: currently focused field index
    pub ch_fx_field: usize,
    /// Pattern matrix view: cursor row (order position)
    pub matrix_cursor: usize,
    /// Command-line buffer for :command mode
    pub command_buf: String,

    // -----------------------------------------------------------------------
    // Dialog State
    // -----------------------------------------------------------------------
    pub dialogs: DialogState,

    // -----------------------------------------------------------------------
    // UI-specific state
    // -----------------------------------------------------------------------
    pub status_message: Option<String>,
    /// Last auto-save timestamp
    pub(crate) last_autosave: Instant,
    pub edit_order: usize,
    /// The mode to return to after closing the port selector
    pub(crate) prev_mode: Mode,
    pub theme_index: usize,

    // -----------------------------------------------------------------------
    // Recent Files
    // -----------------------------------------------------------------------
    pub recent_files: Vec<PathBuf>,

    // -----------------------------------------------------------------------
    // Visualization
    // -----------------------------------------------------------------------
    pub vis: crate::tui::visualization::VisualizationState,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            core: rtrack_core::TrackerCore::new(),
            mode: Mode::Normal,
            should_quit: false,
            cursor_row: 0,
            cursor_channel: 0,
            cursor_sub: SubColumn::Note,
            current_octave: 4,
            edit_step: 1,
            show_second_fx: false,
            status_message: None,
            last_autosave: Instant::now(),
            clipboard: rtrack_core::editor::Clipboard::default(),
            block_anchor: None,
            phrase_view: None,
            edit_order: 0,
            prev_mode: Mode::Normal,
            theme_index: 0,
            track_page: 0,
            follow_playback: true,
            rename_buf: String::new(),
            ch_fx_field: 0,
            matrix_cursor: 0,
            command_buf: String::new(),
            dialogs: DialogState::new(),
            recent_files: rtrack_core::config::load_recent_files(),
            vis: crate::tui::visualization::VisualizationState::new(),
        }
    }

    // -- Sample loading --

    pub fn load_sample(&mut self, slot: usize, path: std::path::PathBuf) {
        self.status_message = Some(match self.core.load_sample(slot, &path) {
            Ok(name) => format!("Loaded sample: {}", name),
            Err(e) => format!("Sample load failed: {}", e),
        });
    }

    pub fn load_sample_directory(&mut self, dir: &std::path::Path) {
        self.status_message = Some(match self.core.load_sample_directory(dir) {
            Ok(count) => format!("Loaded {} sample(s) from {}", count, dir.display()),
            Err(e) => format!("Sample directory failed: {}", e),
        });
    }

    /// Render what a load reported into a one-line status message.
    ///
    /// `verb` names what happened -- "Loaded" for a `.rtrk`, "Imported" for a
    /// MIDI file -- so both paths report their repairs the same way rather
    /// than one of them dropping them.
    fn describe_load(verb: &str, report: &rtrack_core::core::LoadReport) -> String {
        if report.is_clean() {
            return format!("{}: {}", verb, report.path.display());
        }
        let mut notes = Vec::new();
        if report.from_newer_version {
            notes.push("written by a newer rtrack; some settings may be missing".to_string());
        }
        if !report.repairs.is_empty() {
            notes.push(format!("repaired: {}", report.repairs.join("; ")));
        }
        if !report.missing_samples.is_empty() {
            let names: Vec<&str> = report
                .missing_samples
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            notes.push(format!("missing samples: {}", names.join(", ")));
        }
        format!(
            "{} ({}): {}",
            verb,
            notes.join(" | "),
            report.path.display()
        )
    }

    pub fn open_synth_editor(&mut self) {
        let slot = self.dialogs.instrument_cursor;
        // A MIDI instrument becomes a synth starting from its preset. A
        // sample instrument is left alone: an instrument plays one source.
        let inst = &mut self.core.instruments[slot];
        match inst.kind {
            rtrack_core::InstrumentKind::Synth(_) => {}
            rtrack_core::InstrumentKind::Midi { program, .. } => {
                inst.kind = rtrack_core::InstrumentKind::Synth(
                    rtrack_core::audio::synth::SynthParams::from_patch(program),
                );
            }
            rtrack_core::InstrumentKind::Sample { .. } => {
                self.status_message = Some(format!(
                    "Instrument {slot:02X} plays a sample; clear it to make it a synth"
                ));
                return;
            }
        }
        self.dialogs.synth_editor_slot = slot;
        self.dialogs.synth_editor_field = SynthField::Waveform;
        self.prev_mode = self.mode;
        self.mode = Mode::SynthEditor;
    }

    pub fn open_sample_editor(&mut self) {
        self.dialogs.sample_editor_slot = self.dialogs.instrument_cursor;
        self.dialogs.sample_editor_field = SampleField::BaseNote;
        self.prev_mode = self.mode;
        self.mode = Mode::SampleEditor;
    }

    /// Open the file browser on song files. Shared by `:open` and Ctrl+O so
    /// the two entry points cannot drift apart.
    pub fn open_song_browser(&mut self) {
        self.open_file_browser(FileBrowserAction::OpenSong, vec!["rtrk".to_string()]);
    }

    /// Open the file browser on audio files, loading into the cursor's
    /// channel. Reached by `:load`.
    pub fn open_sample_browser(&mut self) {
        self.open_file_browser(
            FileBrowserAction::LoadSample(self.cursor_channel),
            vec!["wav".to_string(), "aif".to_string(), "aiff".to_string()],
        );
    }

    /// Show the recent-files list, or say so if there are none.
    pub fn open_recent_files(&mut self) {
        if self.recent_files.is_empty() {
            self.mode = Mode::Normal;
            self.status_message = Some("No recent files".to_string());
        } else {
            self.dialogs.recent_cursor = 0;
            self.prev_mode = self.mode;
            self.mode = Mode::RecentFiles;
        }
    }

    pub fn open_file_browser(&mut self, action: FileBrowserAction, extensions: Vec<String>) {
        self.prev_mode = self.mode;
        self.dialogs.file_browser.open(action, extensions);
        self.mode = Mode::FileBrowser;
    }

    pub fn on_file_selected(&mut self, path: PathBuf) {
        match self.dialogs.file_browser.action {
            FileBrowserAction::LoadSample(slot) => {
                self.status_message = Some(match self.core.load_sample_into_slot(slot, &path) {
                    Ok(()) => format!("Loaded sample into slot {:02X}", slot),
                    Err(e) => format!("Failed to load: {}", e),
                });
            }
            FileBrowserAction::OpenSong => {
                self.load_file(path);
            }
        }
    }

    /// Word a slice failure, including what a second Enter would do.
    pub(crate) fn describe_slice_error(&self, e: &rtrack_core::error::Error) -> String {
        match e {
            rtrack_core::error::Error::SlotsOccupied { .. } => {
                format!("{e}; Enter again to overwrite")
            }
            _ => format!("Slice failed: {e}"),
        }
    }

    pub fn slice_sample(&mut self, use_transients: bool) -> rtrack_core::error::Result<usize> {
        let range = self.dialogs.sample_slice_range;
        // `Source` replaces the whole set, so it writes from the set's first
        // slot rather than the one being edited. The GUI does the same.
        let slot = match range {
            rtrack_core::sample::SliceRange::Source => self
                .core
                .sample_bank
                .slice_set_start(self.dialogs.sample_editor_slot),
            rtrack_core::sample::SliceRange::Span => self.dialogs.sample_editor_slot,
        };
        let count = self.dialogs.sample_slice_count;
        let sensitivity = self.dialogs.sample_slice_sensitivity;
        // Recorded before the write, so Ctrl+Z puts back whatever the
        // slices landed on.
        self.core.begin_edit_with_samples();
        // Slicing overwrites consecutive slots, so the first attempt refuses
        // when anything unrelated is in the way; the arming flag is what a
        // second Enter sets.
        let overwrite = if self.dialogs.sample_slice_overwrite_armed {
            rtrack_core::sample::SliceOverwrite::Allow
        } else {
            rtrack_core::sample::SliceOverwrite::Refuse
        };
        let result =
            self.core
                .slice_sample(slot, count, sensitivity, use_transients, range, overwrite);
        self.dialogs.sample_slice_overwrite_armed =
            matches!(result, Err(rtrack_core::error::Error::SlotsOccupied { .. }));
        if result.is_err() {
            // Nothing was written, so there is nothing to undo.
            self.core.cancel_edit();
        } else {
            // Slicing is one step on its own. Committing here rather than
            // leaving it to the entry point keeps a later edit from being
            // folded into it, which would make an unrelated undo revert the
            // whole slicing.
            self.core.commit_edit();
        }
        result
    }

    // -- MIDI port selection --

    pub fn open_port_selector(&mut self) {
        let mut ports = Vec::new();
        #[cfg(unix)]
        ports.push(rtrack_core::midi::VIRTUAL_PORT_NAME.to_string());

        if let Ok(hw_ports) = MidiEngine::list_ports() {
            ports.extend(hw_ports);
        }

        self.dialogs.midi_port_list = ports;
        self.dialogs.midi_port_cursor = 0;
        self.prev_mode = self.mode;
        self.mode = Mode::MidiPortSelect;
    }

    pub(crate) fn close_port_selector(&mut self) {
        self.mode = self.prev_mode;
    }

    pub(crate) fn select_midi_port(&mut self) {
        if self.dialogs.midi_port_cursor >= self.dialogs.midi_port_list.len() {
            return;
        }

        let name = self.dialogs.midi_port_list[self.dialogs.midi_port_cursor].clone();
        self.status_message = Some(match self.core.select_midi_port(&name) {
            Ok(()) => format!("MIDI out: {name}"),
            Err(e) => e.to_string(),
        });
        self.close_port_selector();
    }

    /// Show any error the audio stream reported since the last frame.
    ///
    /// The cpal error callback cannot print (it would land on the alternate
    /// screen) and cannot return, so it records and we poll.
    pub fn report_audio_errors(&mut self) {
        if let Some(ref audio) = self.core.audio {
            if let Some(err) = audio.take_stream_error() {
                self.status_message = Some(format!("Audio error: {}", err));
            }
        }
        if let Some(notice) = self.core.take_notice() {
            self.status_message = Some(notice);
        }
    }

    /// Effect columns shown: two when toggled on with `:fx2`, or when the
    /// pattern on screen uses the second, so its data is never hidden.
    pub fn fx_columns(&self) -> usize {
        let used = self.core.song.uses_second_fx(self.sheet());
        if self.show_second_fx || used {
            2
        } else {
            1
        }
    }

    pub fn current_order_position(&self) -> usize {
        if self.core.playing {
            self.core.playback_position().0
        } else {
            self.edit_order
        }
    }

    /// What the grid shows and edits: the phrase view's phrase, or the
    /// pattern at the current order position.
    pub fn sheet(&self) -> rtrack_core::tracker::Sheet {
        match self.phrase_view {
            Some(view) => view.sheet(self.cursor_channel),
            None => self.current_order_position().into(),
        }
    }

    /// The pattern row playback starts from and the phrase view returns to.
    pub fn pattern_row(&self) -> usize {
        self.phrase_view.map_or(self.cursor_row, |v| v.pattern_row)
    }

    /// Whether the grid highlights and scrolls with playback. The phrase
    /// view does not: the phrase shown need not be the one playing.
    pub fn grid_follows_playback(&self) -> bool {
        self.core.is_playing() && self.phrase_view.is_none()
    }

    /// Row currently being heard, for the playback row highlight.
    pub fn playback_row(&self) -> usize {
        self.core.playback_position().1
    }

    // -- Pattern / Order management --

    pub fn next_order_position(&mut self) {
        if self.edit_order + 1 < self.core.song.order.len() {
            self.edit_order += 1;
            self.cursor_row = 0;
        }
    }

    pub fn prev_order_position(&mut self) {
        if self.edit_order > 0 {
            self.edit_order -= 1;
            self.cursor_row = 0;
        }
    }

    pub fn add_new_pattern_to_order(&mut self) {
        self.core.begin_edit();
        let idx = self.core.song.add_pattern();
        self.core.song.order.push(idx);
        self.core.song.order_repeats.push(1);
        self.edit_order = self.core.song.order.len() - 1;
        self.cursor_row = 0;
        self.status_message = Some(format!(
            "New pattern {:02X}, order pos {:02X}",
            idx, self.edit_order
        ));
    }

    pub fn clone_current_pattern(&mut self) {
        self.core.begin_edit();
        let src_idx = self.core.song.order[self.edit_order];
        let cloned = self.core.song.patterns[src_idx].clone();
        let new_idx = self.core.song.patterns.len();
        self.core.song.patterns.push(cloned);
        self.core.song.order.insert(self.edit_order + 1, new_idx);
        self.core.song.order_repeats.insert(self.edit_order + 1, 1);
        self.edit_order += 1;
        self.cursor_row = 0;
        self.status_message = Some(format!("Cloned pattern {:02X} -> {:02X}", src_idx, new_idx));
    }

    pub fn toggle_channel_mute(&mut self, channel: usize) {
        if let Some(muted) = self.core.toggle_channel_mute(channel) {
            let state = if muted { "muted" } else { "unmuted" };
            self.status_message = Some(format!("Ch {} {}", channel + 1, state));
        }
    }

    pub fn toggle_solo(&mut self, channel: usize) {
        self.status_message = Some(match self.core.toggle_solo(channel) {
            Some(ch) => format!("Solo ch {}", ch + 1),
            None => "Solo off".to_string(),
        });
    }

    // -- File I/O --

    pub fn save(&mut self) {
        match self.core.save() {
            Ok(path) => {
                self.last_autosave = Instant::now();
                rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                rtrack_core::config::save_recent_files(&self.recent_files);
                self.status_message = Some(format!("Saved: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
            }
        }
    }

    pub fn auto_save(&mut self) {
        if let Err(e) = self.core.auto_save(&mut self.last_autosave) {
            self.status_message = Some(format!("Auto-save failed: {}", e));
        }
    }

    pub fn load_file(&mut self, path: PathBuf) {
        match self.core.load_file(&path) {
            Ok(report) => {
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.track_page = 0;
                self.block_anchor = None;
                self.phrase_view = None;
                rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                rtrack_core::config::save_recent_files(&self.recent_files);
                self.status_message = Some(Self::describe_load("Loaded", &report));
            }
            Err(e) => {
                self.status_message = Some(format!("Load failed: {}", e));
            }
        }
    }

    // -- Undo/Redo --

    /// Record a sample-bank change attributed to one control, folding it into
    /// the previous step if that came from the same one.
    ///
    /// The sample editor's fields are adjusted by held arrow keys, so each
    /// press is one change. Recording every press would bury the history, and
    /// avoiding that is why these fields were outside undo altogether. `before`
    /// is the bank as it stood before this press; coalescing keeps the first
    /// one, so undo returns to where the field stood before the run began.
    pub fn record_sample_field_edit(
        &mut self,
        source: rtrack_core::editor::EditSource,
        before: rtrack_core::core::SampleSnapshot,
    ) {
        // A field edit is its own step, so anything the current event had
        // pending is committed first rather than being swallowed by it.
        self.core.commit_edit();
        let after = self.core.snapshot_samples();
        self.core.history.push_from(
            source,
            Edit::Bank(Box::new(rtrack_core::editor::BankEdit { before, after })),
        );
        self.core.dirty = true;
    }

    /// End a run of coalescing, so the next edit to the same control starts a
    /// new step. Called when the field or the dialog changes.
    pub fn break_edit_coalescing(&mut self) {
        self.core.history.break_coalescing();
    }

    /// Bring the cursor and the visible track page inside the song.
    pub fn clamp_cursor_to_song(&mut self) {
        let order_len = self.core.song.order.len().max(1);
        if self.edit_order >= order_len {
            self.edit_order = order_len - 1;
        }
        // An undo can remove the phrase on view.
        let phrases = self.core.song.phrases.len();
        match self.phrase_view.as_mut() {
            Some(_) if phrases == 0 => self.close_phrase_view(),
            Some(view) => view.phrase = view.phrase.min(phrases - 1),
            None => {}
        }
        let rows = self.core.song.rows_at(self.sheet());
        if self.cursor_row >= rows {
            self.cursor_row = rows - 1;
        }
        let channels = self.core.song.channels.max(1);
        if self.cursor_channel >= channels {
            self.cursor_channel = channels - 1;
        }
        if self.track_page * CHANNELS_PER_PAGE >= channels {
            self.track_page = 0;
        }
    }

    pub fn undo(&mut self) {
        if self.core.undo() {
            self.clamp_cursor_to_song();
            self.status_message = Some("Undo".to_string());
        }
    }

    pub fn redo(&mut self) {
        if self.core.redo() {
            self.clamp_cursor_to_song();
            self.status_message = Some("Redo".to_string());
        }
    }

    // -- Clipboard --

    pub fn copy_row(&mut self) {
        let pos = self.sheet();
        let song = &self.core.song;
        let channels = song.channels;
        let row: Vec<rtrack_core::tracker::Cell> = (0..channels)
            .map(|ch| *song.cell_at(pos, self.cursor_row, ch))
            .collect();
        self.clipboard.set_run(row);
        self.status_message = Some(format!("Copied row {:02X}", self.cursor_row));
    }

    pub fn paste_row(&mut self) {
        if let Some(row) = self.clipboard.run().map(|r| r.to_vec()) {
            self.core.begin_edit();
            let pos = self.sheet();
            for (ch, cell) in row.iter().enumerate() {
                self.core.song.set_cell(pos, self.cursor_row, ch, *cell);
            }
            self.status_message = Some(format!("Pasted at row {:02X}", self.cursor_row));
        }
    }

    pub fn cut_row(&mut self) {
        self.copy_row();
        self.core.begin_edit();
        let pos = self.sheet();
        for ch in 0..self.core.song.channels {
            self.core.song.set_cell(
                pos,
                self.cursor_row,
                ch,
                rtrack_core::tracker::Cell::default(),
            );
        }
    }

    // -- Theme cycling --

    pub fn cycle_theme(&mut self) {
        self.theme_index = (self.theme_index + 1) % crate::tui::theme::THEME_NAMES.len();
        let name = crate::tui::theme::THEME_NAMES[self.theme_index];
        self.status_message = Some(format!("Theme: {}", name));
    }

    pub fn theme(&self) -> crate::tui::theme::Theme {
        let name = crate::tui::theme::THEME_NAMES
            .get(self.theme_index)
            .copied()
            .unwrap_or("dark");
        crate::tui::theme::theme_by_name(name)
    }

    // -- MIDI clock toggle --

    pub fn toggle_midi_clock(&mut self) {
        let state = if self.core.toggle_midi_clock() {
            "on"
        } else {
            "off"
        };
        self.status_message = Some(format!("MIDI clock {}", state));
    }

    // -- Export/Import --

    pub fn export_wav_file(&mut self) {
        self.status_message = Some(match self.core.export_wav_to_default() {
            Ok(path) => format!("Exported WAV: {}", path.display()),
            Err(e) => format!("WAV export failed: {}", e),
        });
    }

    pub fn export_flac_file(&mut self) {
        self.status_message = Some(match self.core.export_flac_to_default() {
            Ok(path) => format!("Exported FLAC: {}", path.display()),
            Err(e) => format!("FLAC export failed: {}", e),
        });
    }

    pub fn export_midi(&mut self) {
        self.status_message = Some(match self.core.export_midi_to_default() {
            Ok(path) => format!("Exported MIDI: {}", path.display()),
            Err(e) => format!("MIDI export failed: {}", e),
        });
    }

    pub fn import_midi_file(&mut self, path: PathBuf) {
        self.core.begin_edit();
        match self.core.import_midi_file(&path) {
            Ok(report) => {
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.status_message = Some(Self::describe_load("Imported", &report));
            }
            Err(e) => {
                self.status_message = Some(format!("Import failed: {}", e));
            }
        }
    }

    /// Get the range of visible channels for the current track page
    pub fn visible_channels(&self) -> std::ops::Range<usize> {
        if self.phrase_view.is_some() {
            return self.cursor_channel..self.cursor_channel + 1;
        }
        let start = self.track_page * CHANNELS_PER_PAGE;
        let end = (start + CHANNELS_PER_PAGE).min(self.core.song.channels);
        start..end
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rtrack_core::tracker::{Fx, FxCmd, Note, Song};
    use std::sync::Arc;

    /// Point rtrack's config directory at a scratch path for this process.
    ///
    /// Saving a song records it in the recent-files list, so without this the
    /// tests below rewrite the developer's real `~/.config/rtrack/recent.json`
    /// -- which they were observed doing, leaving `/tmp` paths in a menu the
    /// user actually sees. Idempotent, and safe to call from every test.
    fn isolate_config() {
        rtrack_core::config::set_config_dir_for_test(
            std::env::temp_dir().join(format!("rtrack-test-config-{}", std::process::id())),
        );
    }

    pub(crate) fn make_app() -> App {
        isolate_config();
        // Use the builder rather than a struct literal: it is the supported
        // way to get a core with no hardware attached, and it does not need
        // updating every time TrackerCore gains a field.
        let core = rtrack_core::core::TrackerCoreBuilder::new()
            .song_size(4, 64)
            .headless()
            .build();
        App {
            core,
            mode: Mode::Normal,
            should_quit: false,
            cursor_row: 0,
            cursor_channel: 0,
            cursor_sub: SubColumn::Note,
            current_octave: 4,
            edit_step: 1,
            show_second_fx: false,
            status_message: None,
            last_autosave: Instant::now(),
            clipboard: rtrack_core::editor::Clipboard::default(),
            block_anchor: None,
            phrase_view: None,
            edit_order: 0,
            prev_mode: Mode::Normal,
            theme_index: 0,
            track_page: 0,
            dialogs: DialogState {
                midi_port_list: Vec::new(),
                midi_port_cursor: 0,
                settings_field: SettingsField::Title,
                settings_edit_buf: String::new(),
                settings_buf_untouched: true,
                instrument_cursor: 0,
                phrase_cursor: 0,
                table_number: 0,
                table_row: 0,
                table_field: TableField::Transpose,
                sample_editor_slot: 0,
                sample_editor_field: SampleField::BaseNote,
                sample_slice_count: 8,
                sample_slice_sensitivity: 0.5,
                sample_slice_range: rtrack_core::sample::SliceRange::Source,
                sample_slice_overwrite_armed: false,
                sample_slice_plan: Default::default(),
                synth_editor_slot: 0,
                synth_editor_field: SynthField::Waveform,
                help_scroll: 0,
                file_browser: FileBrowserState {
                    dir: PathBuf::from("/tmp"),
                    entries: Vec::new(),
                    cursor: 0,
                    action: FileBrowserAction::OpenSong,
                    filter: Vec::new(),
                    scroll: 0,
                },
                recent_cursor: 0,
            },
            follow_playback: true,
            rename_buf: String::new(),
            ch_fx_field: 0,
            matrix_cursor: 0,
            command_buf: String::new(),
            recent_files: Vec::new(),
            vis: crate::tui::visualization::VisualizationState::new(),
        }
    }

    #[test]
    fn test_cursor_movement() {
        let mut app = make_app();
        app.move_cursor_down(5);
        assert_eq!(app.cursor_row, 5);

        app.move_cursor_up(3);
        assert_eq!(app.cursor_row, 2);

        // Don't go below 0
        app.move_cursor_up(100);
        assert_eq!(app.cursor_row, 0);

        // Don't exceed max
        app.move_cursor_down(1000);
        assert_eq!(app.cursor_row, 63);
    }

    #[test]
    fn test_cursor_left_right() {
        let mut app = make_app();
        assert_eq!(app.cursor_sub, SubColumn::Note);
        assert_eq!(app.cursor_channel, 0);

        app.move_cursor_right();
        assert_eq!(app.cursor_sub, SubColumn::Instrument);

        app.move_cursor_right();
        assert_eq!(app.cursor_sub, SubColumn::Volume);

        app.move_cursor_right();
        assert_eq!(app.cursor_sub, SubColumn::Effect);

        // Moving right at Effect moves to next channel
        app.move_cursor_right();
        assert_eq!(app.cursor_channel, 1);
        assert_eq!(app.cursor_sub, SubColumn::Note);

        // Moving left at Note moves to prev channel's Effect
        app.move_cursor_left();
        assert_eq!(app.cursor_channel, 0);
        assert_eq!(app.cursor_sub, SubColumn::Effect);
    }

    #[test]
    fn test_cursor_bounds() {
        let mut app = make_app();

        // Can't go left past channel 0, Note
        app.move_cursor_left();
        assert_eq!(app.cursor_channel, 0);
        assert_eq!(app.cursor_sub, SubColumn::Note);

        // Go to last channel, last sub
        app.cursor_channel = 3;
        app.cursor_sub = SubColumn::Effect;

        // Can't go right past last channel
        app.move_cursor_right();
        assert_eq!(app.cursor_channel, 3);
        assert_eq!(app.cursor_sub, SubColumn::Effect);
    }

    #[test]
    fn test_note_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Note;

        // Press 'z' = C at current octave (4)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        // Cursor should have advanced by edit_step (1)
        assert_eq!(app.cursor_row, 1);
    }

    #[test]
    fn keyboard_entry_snaps_to_the_song_scale() {
        use rtrack_core::theory::{Scale, ScaleSetting};
        use rtrack_core::tracker::NoteValue;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Note;
        app.core.song.scale = Some(ScaleSetting::new(NoteValue::C, Scale::Major));

        // 's' is C# at the current octave, which C major does not contain.
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

        let pattern_idx = app.core.song.order[0];
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(0, 0).note,
            Some(Note::On {
                value: NoteValue::C,
                octave: 4,
            })
        );
    }

    #[test]
    fn a_scale_does_not_rewrite_notes_already_in_the_pattern() {
        use rtrack_core::theory::{Scale, ScaleSetting};
        use rtrack_core::tracker::NoteValue;

        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        let off_scale = Note::On {
            value: NoteValue::Cs,
            octave: 4,
        };
        app.core.song.patterns[pattern_idx].get_mut(0, 0).note = Some(off_scale);

        app.core.song.scale = Some(ScaleSetting::new(NoteValue::C, Scale::Major));

        assert_eq!(
            app.core.song.patterns[pattern_idx].get(0, 0).note,
            Some(off_scale)
        );
    }

    #[test]
    fn test_note_off_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE));
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(cell.note, Some(Note::Off));
    }

    #[test]
    fn test_delete_at_cursor() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        // Enter a note first
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.cursor_row = 0;

        // Delete it
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_play_stop() {
        let mut app = make_app();

        app.play();
        assert!(app.core.playing);

        app.core.stop();
        assert!(!app.core.playing);
    }

    #[test]
    fn test_play_starts_from_edit_order() {
        let mut app = make_app();
        app.core.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.play();
        assert_eq!(app.core.engine.order, 2);
        assert_eq!(app.core.engine.row, 5);
    }

    #[test]
    fn test_play_from_start() {
        let mut app = make_app();
        app.core.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.play_from_start();
        assert_eq!(app.core.engine.order, 0);
        assert_eq!(app.core.engine.row, 0);
        assert_eq!(app.edit_order, 0);
    }

    #[test]
    fn test_ctrl_space_plays_from_start() {
        let mut app = make_app();
        app.core.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(app.core.playing);
        assert_eq!(app.core.engine.order, 0);
        assert_eq!(app.core.engine.row, 0);
    }

    #[test]
    fn test_mode_toggle() {
        let mut app = make_app();
        assert_eq!(app.mode, Mode::Normal);

        // Esc in Normal -> Insert
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert);

        // Esc in Insert -> Normal
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_tab_cycles_tracks() {
        let mut app = make_app();
        // 4 channels: Tab cycles 0 -> 1 -> 2 -> 3 -> 0
        assert_eq!(app.cursor_channel, 0);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.cursor_channel, 1);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.cursor_channel, 2);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.cursor_channel, 3);
        // Wraps around
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.cursor_channel, 0);

        // Shift+Tab goes backward: 0 -> 3
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.cursor_channel, 3);
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.cursor_channel, 2);

        // 8 channels: Tab updates track_page when crossing page boundary
        app.core.song.channels = 8;
        for pat in &mut app.core.song.patterns {
            pat.channels = 8;
            for row in &mut pat.data {
                row.resize(8, rtrack_core::tracker::Cell::default());
            }
        }
        app.core.channels = rtrack_core::default_channel_configs(8);

        app.cursor_channel = 3;
        app.track_page = 0;
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.cursor_channel, 4);
        assert_eq!(app.track_page, 1); // auto-switched to page 1
    }

    #[test]
    fn test_octave_change() {
        let mut app = make_app();
        assert_eq!(app.current_octave, 4);

        // '+' increases octave (in Normal mode, shared key)
        app.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        assert_eq!(app.current_octave, 5);

        // '-' decreases (Normal mode specific)
        app.handle_key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE));
        assert_eq!(app.current_octave, 4);
    }

    #[test]
    fn test_upper_octave_note_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        // 'q' = C at octave+1 (5)
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(
            cell.note,
            Some(Note::On {
                value: rtrack_core::tracker::NoteValue::C,
                octave: 5
            })
        );
    }

    #[test]
    fn test_hex_entry_instrument() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Instrument;

        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(cell.instrument, Some(0x01));
    }

    #[test]
    fn test_open_port_selector() {
        let mut app = make_app();
        app.mode = Mode::Normal;

        app.open_port_selector();
        assert_eq!(app.mode, Mode::MidiPortSelect);
        assert_eq!(app.prev_mode, Mode::Normal);
    }

    #[test]
    fn test_close_port_selector_restores_mode() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.open_port_selector();
        assert_eq!(app.mode, Mode::MidiPortSelect);
        app.close_port_selector();
        assert_eq!(app.mode, Mode::Insert);
    }

    #[test]
    fn test_port_select_navigation() {
        let mut app = make_app();
        app.dialogs.midi_port_list = vec![
            "Port A".to_string(),
            "Port B".to_string(),
            "Port C".to_string(),
        ];
        app.dialogs.midi_port_cursor = 0;
        app.mode = Mode::MidiPortSelect;

        // Down
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.midi_port_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.midi_port_cursor, 2);

        // Can't go past end
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.midi_port_cursor, 2);

        // Up
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.dialogs.midi_port_cursor, 1);
    }

    #[test]
    fn test_port_select_esc_closes() {
        let mut app = make_app();
        app.mode = Mode::Normal;
        app.open_port_selector();
        assert_eq!(app.mode, Mode::MidiPortSelect);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_f2_opens_port_selector() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::MidiPortSelect);
    }

    #[test]
    fn test_undo_redo() {
        let mut app = make_app();
        let original_title = app.core.song.title.clone();

        app.core.begin_edit();
        app.core.song.title = "Modified".to_string();

        app.undo();
        assert_eq!(app.core.song.title, original_title);

        app.redo();
        assert_eq!(app.core.song.title, "Modified");
    }

    // -- Sample editor fields under undo --

    /// These fields were outside undo entirely, because one step per arrow
    /// key would bury the history.
    #[test]
    fn a_sample_field_edit_can_be_undone() {
        let (mut app, _dir) = app_with_amen();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_editor_field = SampleField::BaseNote;
        let original = app.core.sample_bank.get(0).expect("a sample").base_note;

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let raised = app.core.sample_bank.get(0).expect("a sample").base_note;
        assert_eq!(raised, original + 1);

        app.undo();
        assert_eq!(
            app.core.sample_bank.get(0).expect("a sample").base_note,
            original
        );
    }

    /// A held arrow key is one edit, not one per repeat, and undo returns to
    /// where the field stood before the run began.
    #[test]
    fn a_run_of_sample_field_edits_is_one_undo_step() {
        let (mut app, _dir) = app_with_amen();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_editor_field = SampleField::BaseNote;
        let original = app.core.sample_bank.get(0).expect("a sample").base_note;

        let before_depth = app.core.history.undo_depth();
        for _ in 0..10 {
            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        }
        assert_eq!(
            app.core.history.undo_depth(),
            before_depth + 1,
            "ten presses should be one step"
        );
        assert_eq!(
            app.core.sample_bank.get(0).expect("a sample").base_note,
            original + 10
        );

        app.undo();
        assert_eq!(
            app.core.sample_bank.get(0).expect("a sample").base_note,
            original,
            "undo should return to before the run, not one press back"
        );
    }

    /// Moving to another field ends the run: the two are separate edits.
    #[test]
    fn moving_between_sample_fields_ends_the_run() {
        let (mut app, _dir) = app_with_amen();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_editor_field = SampleField::BaseNote;

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let after_first = app.core.history.undo_depth();

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));

        assert_eq!(
            app.core.history.undo_depth(),
            after_first + 1,
            "a different field is a different step"
        );
    }

    /// A drag or held key must not grow the history frame by frame.
    #[test]
    fn a_long_run_of_field_edits_does_not_grow_the_history() {
        let (mut app, _dir) = app_with_amen();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_editor_field = SampleField::TrimStart;

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let after_one = app.core.history.approx_bytes();
        for _ in 0..200 {
            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        }

        assert_eq!(
            app.core.history.approx_bytes(),
            after_one,
            "200 presses should weigh the same as one step"
        );
    }

    // -- Undo step boundaries under the shared history --

    /// One keypress is one undo step, however many times the handlers
    /// snapshotted along the way.
    #[test]
    fn a_single_keypress_is_a_single_undo_step() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        let before = app.core.history.undo_depth();
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(
            app.core.history.undo_depth(),
            before + 1,
            "one key should leave exactly one step"
        );
    }

    #[test]
    fn a_key_that_edits_nothing_leaves_no_step() {
        let mut app = make_app();
        let before = app.core.history.undo_depth();
        // Plain cursor movement in Normal mode.
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.core.history.undo_depth(), before);
    }

    /// Slicing rewrites the bank and the instrument names together, so its
    /// step has to put both back -- and a later, unrelated edit must not be
    /// folded into it.
    #[test]
    fn a_later_edit_is_not_folded_into_the_slicing_step() {
        let (mut app, _dir) = app_with_amen();
        app.dialogs.sample_slice_count = 4;
        app.slice_sample(false).expect("slice");
        let after_slicing = app.core.history.undo_depth();

        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(
            app.core.history.undo_depth(),
            after_slicing + 1,
            "the note should be its own step, not merged into the slicing"
        );

        // Undoing the note leaves the slices alone.
        app.undo();
        assert!(
            app.core.sample_bank.get(3).is_some(),
            "undoing a note reverted the slicing"
        );
    }

    /// The history is bounded by bytes, so a step is dropped from the far end
    /// rather than the near one.
    #[test]
    fn the_history_stays_within_its_memory_budget() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        for _ in 0..300 {
            app.cursor_row = 0;
            app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        }
        assert!(
            app.core.history.approx_bytes() <= rtrack_core::constants::MAX_UNDO_BYTES,
            "history holds {} bytes, budget is {}",
            app.core.history.approx_bytes(),
            rtrack_core::constants::MAX_UNDO_BYTES
        );
        assert!(
            app.core.history.can_undo(),
            "the most recent edit is still there"
        );
    }

    /// The clipboard is the shared type now; a copied row must survive a
    /// round trip through it unchanged.
    #[test]
    fn a_copied_row_round_trips_through_the_shared_clipboard() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;
        app.cursor_row = 0;

        app.copy_row();
        let copied = app.clipboard.run().expect("a copied row").to_vec();
        assert_eq!(copied.len(), app.core.song.channels);
        assert!(copied[0].note.is_some(), "the note should have been copied");

        app.cursor_row = 4;
        app.paste_row();
        assert_eq!(app.core.song.cell_at(0, 4, 0).note, copied[0].note);
    }

    #[test]
    fn test_undo_clears_redo_on_new_edit() {
        let mut app = make_app();
        app.core.begin_edit();
        app.core.song.title = "Edit 1".to_string();

        app.undo();
        assert!(app.core.history.can_redo());

        // New edit should clear redo
        app.core.begin_edit();
        assert!(!app.core.history.can_redo());
    }

    #[test]
    fn test_copy_paste_row() {
        let mut app = make_app();
        // Enter a note at row 0, ch 0
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;
        app.cursor_row = 0;

        // Copy
        app.copy_row();
        assert!(!app.clipboard.is_empty());

        // Move and paste
        app.cursor_row = 5;
        app.paste_row();

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(5, 0);
        assert!(cell.note.is_some());
    }

    #[test]
    fn test_cut_row() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;
        app.cursor_row = 0;

        app.cut_row();

        // Original should be cleared
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());

        // Clipboard should have the data
        assert!(!app.clipboard.is_empty());
    }

    /// The importer caps how long a song it will build from a MIDI file.
    /// The cap only helps if the person who ran the import is told about it,
    /// so the status line has to carry it the way a repaired `.rtrk` does.
    #[test]
    fn importing_an_over_long_midi_file_says_it_was_truncated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("far.mid");

        // One note 30.7 million ticks in: 37 bytes asking for a song of
        // nearly 20,000 patterns.
        let mut track = Vec::new();
        let vlq = |buf: &mut Vec<u8>, mut v: u32| {
            let mut tmp = Vec::new();
            loop {
                tmp.push((v & 0x7F) as u8);
                v >>= 7;
                if v == 0 {
                    break;
                }
            }
            for (i, b) in tmp.iter().enumerate().rev() {
                buf.push(if i == 0 { *b } else { *b | 0x80 });
            }
        };
        vlq(&mut track, 30_700_000);
        track.extend_from_slice(&[0x90, 60, 100]);
        vlq(&mut track, 10);
        track.extend_from_slice(&[0x80, 60, 0]);
        vlq(&mut track, 0);
        track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let mut data = Vec::new();
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&6u32.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&96u16.to_be_bytes());
        data.extend_from_slice(b"MTrk");
        data.extend_from_slice(&(track.len() as u32).to_be_bytes());
        data.extend_from_slice(&track);
        std::fs::write(&path, data).expect("write");

        let mut app = make_app();
        app.import_midi_file(path);

        let status = app.status_message.clone().expect("a status message");
        assert!(status.starts_with("Imported"), "{status}");
        assert!(
            status.contains("truncated"),
            "the truncation must reach the status line: {status}"
        );
    }

    #[test]
    fn loading_a_song_clears_the_block_selection() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");
        let mut app = make_app();
        app.core.file_path = Some(path.clone());
        app.save();

        app.block_anchor = Some((2, 1));
        app.load_file(path);
        assert!(app.block_anchor.is_none());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let mut app = make_app();
        app.core.song.title = "Test Song".to_string();

        // Enter a note
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;

        // Save
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_roundtrip.rtrk");
        app.core.file_path = Some(path.clone());
        app.save();

        // Modify the song
        app.core.song.title = "Modified".to_string();

        // Load
        app.load_file(path.clone());
        assert_eq!(app.core.song.title, "Test Song");

        // Clean up
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_s_saves() {
        let mut app = make_app();
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_ctrl_s.rtrk");
        app.core.file_path = Some(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_z_undoes() {
        let mut app = make_app();
        app.core.begin_edit();
        app.core.song.title = "Changed".to_string();

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_ne!(app.core.song.title, "Changed");
    }

    #[test]
    fn test_order_navigation() {
        let mut app = make_app();

        // Add a second pattern to order
        let new_pat = app.core.song.add_pattern();
        app.core.song.order.push(new_pat);

        assert_eq!(app.edit_order, 0);

        app.next_order_position();
        assert_eq!(app.edit_order, 1);

        app.next_order_position();
        assert_eq!(app.edit_order, 1); // can't go past end

        app.prev_order_position();
        assert_eq!(app.edit_order, 0);

        app.prev_order_position();
        assert_eq!(app.edit_order, 0); // can't go below 0
    }

    #[test]
    fn test_add_new_pattern_to_order() {
        let mut app = make_app();
        let original_patterns = app.core.song.patterns.len();
        let original_order = app.core.song.order.len();

        app.add_new_pattern_to_order();

        assert_eq!(app.core.song.patterns.len(), original_patterns + 1);
        assert_eq!(app.core.song.order.len(), original_order + 1);
    }

    #[test]
    fn test_clone_current_pattern() {
        let mut app = make_app();
        // Enter a note in pattern 0
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;

        app.clone_current_pattern();

        // Should have 2 patterns now
        assert_eq!(app.core.song.patterns.len(), 2);
        // The cloned pattern should have the same note
        let cell = app.core.song.patterns[1].get(0, 0);
        assert!(cell.note.is_some());
    }

    #[test]
    fn test_channel_mute() {
        let mut app = make_app();
        assert!(app.core.is_channel_audible(0));

        app.toggle_channel_mute(0);
        assert!(!app.core.is_channel_audible(0));

        app.toggle_channel_mute(0);
        assert!(app.core.is_channel_audible(0));
    }

    #[test]
    fn test_ctrl_right_navigates_order() {
        let mut app = make_app();
        let new_pat = app.core.song.add_pattern();
        app.core.song.order.push(new_pat);

        // Ctrl+Right in Normal mode
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(app.edit_order, 1);

        // Ctrl+Left
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.edit_order, 0);
    }

    #[test]
    fn test_f9_toggles_mute() {
        let mut app = make_app();
        assert!(app.core.is_channel_audible(0));

        app.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE));
        assert!(!app.core.is_channel_audible(0));
    }

    #[test]
    fn test_midi_channel_mapping() {
        let app = make_app();
        assert_eq!(app.core.midi_channel_for(0), 0);
        assert_eq!(app.core.midi_channel_for(1), 1);
        assert_eq!(app.core.midi_channel_for(3), 3);
        // Out of range returns clamped
        assert_eq!(app.core.midi_channel_for(99), 99 & 0x0F);
    }

    #[test]
    fn test_solo_channel() {
        let mut app = make_app();

        app.toggle_solo(1);
        assert!(!app.core.is_channel_audible(0));
        assert!(app.core.is_channel_audible(1));
        assert!(!app.core.is_channel_audible(2));

        // Toggle same channel off
        app.toggle_solo(1);
        assert!(app.core.is_channel_audible(0));
        assert!(app.core.is_channel_audible(1));
    }

    #[test]
    fn test_solo_overrides_mute() {
        let mut app = make_app();

        app.toggle_channel_mute(1);
        assert!(!app.core.is_channel_audible(1));

        // Solo channel 1 should override the mute
        app.toggle_solo(1);
        assert!(app.core.is_channel_audible(1));
    }

    #[test]
    fn test_mute_clears_solo() {
        let mut app = make_app();
        app.toggle_solo(1);
        assert_eq!(app.core.solo_channel, Some(1));

        // Toggling mute clears solo
        app.toggle_channel_mute(0);
        assert_eq!(app.core.solo_channel, None);
    }

    #[test]
    fn test_ctrl_f9_toggles_solo() {
        let mut app = make_app();
        assert!(app.core.is_channel_audible(0));
        assert!(app.core.is_channel_audible(1));

        // Ctrl+F9 solos channel 0
        app.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::CONTROL));
        assert!(app.core.is_channel_audible(0));
        assert!(!app.core.is_channel_audible(1));
    }

    #[test]
    fn test_pattern_break_effect() {
        let mut app = make_app();
        // Add a second pattern
        let pat2 = app.core.song.add_pattern();
        app.core.song.order.push(pat2);

        // Set pattern break (D08) at row 0 of pattern 0
        let pat_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Break, 8));

        app.play();
        // Tick 0: advance row -> pattern break fires
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 1);
        assert_eq!(app.core.engine.row, 8);
    }

    #[test]
    fn test_position_jump_effect() {
        let mut app = make_app();
        let pat2 = app.core.song.add_pattern();
        let pat3 = app.core.song.add_pattern();
        app.core.song.order.push(pat2);
        app.core.song.order.push(pat3);

        let pat_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Jump, 2));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 2);
        assert_eq!(app.core.engine.row, 0);
    }

    #[test]
    fn test_position_jump_with_break() {
        let mut app = make_app();
        let pat2 = app.core.song.add_pattern();
        let pat3 = app.core.song.add_pattern();
        app.core.song.order.push(pat2);
        app.core.song.order.push(pat3);

        let pat_idx = app.core.song.order[0];
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
            cell.fx[0] = Some(Fx::new(FxCmd::Jump, 2));
        }
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(0, 1);
            cell.fx[0] = Some(Fx::new(FxCmd::Break, 4));
        }

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 2);
        assert_eq!(app.core.engine.row, 4);
    }

    #[test]
    fn test_pattern_break_wraps_order() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Break, 0));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 0);
        assert_eq!(app.core.engine.row, 0);
        assert_eq!(app.core.engine.generation, 1);
    }

    #[test]
    fn test_position_jump_clamps_to_max() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Jump, 99));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 0); // clamped to max
    }

    #[test]
    fn test_edit_step_change() {
        let mut app = make_app();
        assert_eq!(app.edit_step, 1);

        // ')' increases edit step
        app.handle_key(KeyEvent::new(KeyCode::Char(')'), KeyModifiers::NONE));
        assert_eq!(app.edit_step, 2);

        // '(' decreases
        app.handle_key(KeyEvent::new(KeyCode::Char('('), KeyModifiers::NONE));
        assert_eq!(app.edit_step, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('('), KeyModifiers::NONE));
        assert_eq!(app.edit_step, 0);

        // Can't go below 0
        app.handle_key(KeyEvent::new(KeyCode::Char('('), KeyModifiers::NONE));
        assert_eq!(app.edit_step, 0);
    }

    #[test]
    fn test_edit_step_affects_note_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.edit_step = 4;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(app.cursor_row, 4);
    }

    #[test]
    fn test_edit_step_zero_no_advance() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.edit_step = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(app.cursor_row, 0);
    }

    #[test]
    fn test_insert_row() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        let original_rows = app.core.song.patterns[pattern_idx].rows;

        // Insert a row at cursor (pattern length stays constant -- row is inserted, last row dropped)
        app.insert_row_at_cursor();
        assert_eq!(app.core.song.patterns[pattern_idx].rows, original_rows);
        // The inserted row should be empty
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_delete_row() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        let original_rows = app.core.song.patterns[pattern_idx].rows;

        // Enter a note at row 0
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;
        app.cursor_row = 0;

        app.delete_row_at_cursor();
        assert_eq!(app.core.song.patterns[pattern_idx].rows, original_rows); // +1 from insert, -1 from delete
    }

    #[test]
    fn test_per_pattern_length_cursor_bounds() {
        let mut app = make_app();
        // Change first pattern to 32 rows
        let pattern_idx = app.core.song.order[0];
        app.core.song.patterns[pattern_idx].rows = 32;
        app.core.song.patterns[pattern_idx].data.truncate(32);

        // Try to move past the end
        app.move_cursor_down(100);
        assert_eq!(app.cursor_row, 31);
    }

    #[test]
    fn test_per_pattern_length_playback_advance() {
        let mut app = make_app();
        // Set pattern to 4 rows
        let pattern_idx = app.core.song.order[0];
        app.core.song.patterns[pattern_idx].rows = 4;
        app.core.song.patterns[pattern_idx].data.truncate(4);

        app.play();
        app.core.engine.row = 3; // last row

        // Advance past end -> should wrap
        app.core.process_tick();
        assert_eq!(app.core.engine.row, 0);
    }

    #[test]
    fn test_midi_cc_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Cc(1), 0x40));

        app.play();
        // process_tick dispatches MidiCC event to MIDI output
        app.core.process_tick();
    }

    #[test]
    fn test_program_change_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Program, 5));

        app.play();
        app.core.process_tick();
    }

    #[test]
    fn test_arpeggio_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::Arp, 0x37));

        app.play();
        // Tick 0: triggers note
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48)); // C-4

        // Tick 1: arpeggio pitch bend
        app.core.process_tick();
    }

    #[test]
    fn test_portamento_up_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::PitchUp, 0x10));

        app.play();
        // Tick 0: note on
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].pitch_offset, 0.0);

        // Tick 1: pitch should increase
        app.core.process_tick();
        assert!(app.core.engine.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_portamento_down_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::PitchDown, 0x10));

        app.play();
        app.core.process_tick(); // tick 0: note on
        app.core.process_tick(); // tick 1: effects
        assert!(app.core.engine.channel_states[0].pitch_offset < 0.0);
    }

    #[test]
    fn test_tone_portamento_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        // Row 0: C-4 note (no effect)
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
            cell.note = Some(Note::On {
                value: rtrack_core::tracker::NoteValue::C,
                octave: 4,
            });
            cell.volume = Some(100);
        }
        // Row 1: E-4 with tone porta (3xx)
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(1, 0);
            cell.note = Some(Note::On {
                value: rtrack_core::tracker::NoteValue::E,
                octave: 4,
            });
            cell.fx[0] = Some(Fx::new(FxCmd::PitchSlide, 0x10));
        }

        app.play();
        // Row 0, tick 0: triggers C-4
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48));

        // Advance through remaining ticks of row 0 (ticks 1-5)
        for _ in 1..app.core.song.speed {
            app.core.process_tick();
        }

        // Row 1, tick 0: sets target to E-4 but keeps C-4 playing
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48));
        assert_eq!(app.core.engine.channel_states[0].porta_target, Some(52));

        // Tick 1: pitch should start sliding up
        app.core.process_tick();
        assert!(app.core.engine.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_vibrato_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::Vibrato, 0x42));

        app.play();
        app.core.process_tick(); // tick 0: note on
        app.core.process_tick(); // tick 1: vibrato
        assert!(app.core.engine.channel_states[0].vibrato_phase[0] > 0.0);
    }

    #[test]
    fn test_volume_slide_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::VolSlide, 0x02));

        app.play();
        app.core.process_tick(); // tick 0
        assert_eq!(app.core.engine.channel_states[0].volume, 100);

        app.core.process_tick(); // tick 1: slide
        assert_eq!(app.core.engine.channel_states[0].volume, 98);
    }

    #[test]
    fn test_volume_slide_up() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::VolSlide, 0x30));

        app.play();
        app.core.process_tick(); // tick 0
        app.core.process_tick(); // tick 1: slide up
        assert_eq!(app.core.engine.channel_states[0].volume, 103);
    }

    #[test]
    fn test_volume_slide_clamps() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(5);
        cell.fx[0] = Some(Fx::new(FxCmd::VolSlide, 0x0F));

        app.play();
        app.core.process_tick(); // tick 0
        app.core.process_tick(); // tick 1: slide down
        assert_eq!(app.core.engine.channel_states[0].volume, 0);
    }

    #[test]
    fn test_set_speed_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Ticks, 3));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.song.speed, 3);
    }

    #[test]
    fn test_set_tempo_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Tempo, 0x80));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.song.bpm, 0x80);
    }

    #[test]
    fn test_sub_tick_timing() {
        let mut app = make_app();
        app.core.song.speed = 6;

        app.play();

        // Tick 0
        app.core.process_tick();
        assert_eq!(app.core.engine.tick, 1);

        // Ticks 1-5
        for _ in 0..5 {
            app.core.process_tick();
        }
        // After tick 5, should reset to 0
        assert_eq!(app.core.engine.tick, 0);
    }

    #[test]
    fn test_note_delay_effect() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        // Note C-4 with delay 3 ticks
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On {
            value: rtrack_core::tracker::NoteValue::C,
            octave: 4,
        });
        cell.volume = Some(100);
        cell.fx[0] = Some(Fx::new(FxCmd::Delay, 3));

        app.play();

        // Tick 0: note should be deferred, not triggered
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, None);
        assert!(app.core.engine.channel_states[0].delayed_note.is_some());

        // Tick 1: still waiting
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, None);

        // Tick 2: still waiting
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, None);

        // Tick 3: should trigger
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48));
    }

    #[test]
    fn test_note_delay_off() {
        let mut app = make_app();
        let pat_idx = app.core.song.order[0];

        // First row: trigger C-4
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
            cell.note = Some(Note::On {
                value: rtrack_core::tracker::NoteValue::C,
                octave: 4,
            });
            cell.volume = Some(100);
        }
        // Second row: note-off with delay 2
        {
            let cell = app.core.song.patterns[pat_idx].get_mut(1, 0);
            cell.note = Some(Note::Off);
            cell.fx[0] = Some(Fx::new(FxCmd::Delay, 2));
        }

        app.play();

        // Row 0, tick 0: trigger the note
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48));

        // Advance through remaining ticks of row 0 (ticks 1-5)
        for _ in 1..app.core.song.speed {
            app.core.process_tick();
        }

        // Row 1, tick 0: note-off should be deferred
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48)); // still active
        assert!(app.core.engine.channel_states[0].delayed_note.is_some());

        // Tick 1: waiting
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, Some(48));

        // Tick 2: note-off triggers
        app.core.process_tick();
        assert_eq!(app.core.engine.channel_states[0].note, None);
    }

    #[test]
    fn a_sample_picked_in_the_file_browser_is_undoable() {
        let (mut app, dir) = app_with_amen();
        let snare = dir.path().join("snare.wav");
        std::fs::copy(dir.path().join("amen.wav"), &snare).unwrap();
        app.core.history.clear();
        app.core.dirty = false;

        app.dialogs.file_browser.action = FileBrowserAction::LoadSample(0);
        app.on_file_selected(snare);
        assert!(app.core.dirty, "a load is an unsaved change");
        assert_eq!(app.core.sample_bank.get(0).unwrap().name, "snare");

        app.undo();
        assert_eq!(app.core.sample_bank.get(0).unwrap().name, "amen");
        app.redo();
        assert_eq!(app.core.sample_bank.get(0).unwrap().name, "snare");
    }

    fn app_with_amen() -> (App, tempfile::TempDir) {
        // A private directory per test: these run in parallel, and a shared
        // path had one test deleting the fixture another was loading.
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("amen.wav");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("examples/data/amen.wav"),
            &wav,
        )
        .expect("fixture missing");

        let mut app = make_app();
        app.core.load_sample(0, &wav).unwrap();
        (app, dir)
    }

    /// Enable looping on `slot` over `start..end`.
    fn set_loop(app: &mut App, slot: usize, start: usize, end: usize) {
        let mut bank = (*app.core.sample_bank).clone();
        let s = Arc::make_mut(bank.samples[slot].as_mut().unwrap());
        s.loop_enabled = true;
        s.loop_start = start;
        s.loop_end = end;
        app.core.sample_bank = Arc::new(bank);
    }

    fn is_rising_crossing(app: &App, slot: usize, frame: usize) -> bool {
        let s = app.core.sample_bank.get(slot).unwrap();
        let mono = |i: usize| s.data[i][0] + s.data[i][1];
        mono(frame - 1) < 0.0 && mono(frame) >= 0.0
    }

    /// The amen break in 8 slices, with the sample editor on slot 2's trim end.
    fn app_on_slice_two(field: SampleField) -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with_amen();
        app.dialogs.sample_slice_count = 8;
        app.slice_sample(false).unwrap();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 2;
        app.dialogs.sample_editor_field = field;
        (app, dir)
    }

    #[test]
    fn test_trim_keys_move_a_shared_edge_and_shift_moves_one_slice() {
        let (mut app, _dir) = app_on_slice_two(SampleField::TrimEnd);
        let (edge, next) = (slot_span(&app, 2).1, slot_span(&app, 3));

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(slot_span(&app, 2).1, edge + 100);
        assert_eq!(
            slot_span(&app, 3).0,
            edge + 100,
            "the next slice did not follow"
        );

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
        assert_eq!(slot_span(&app, 2).1, edge + 100 - 1000);
        assert_eq!(
            slot_span(&app, 3).0,
            edge + 100,
            "Shift moved the next slice"
        );
        assert_eq!(slot_span(&app, 3).1, next.1);
    }

    #[test]
    fn test_loop_start_keys_stay_inside_the_played_span() {
        let (mut app, _dir) = app_on_slice_two(SampleField::LoopStart);
        let start = slot_span(&app, 2).0;
        for _ in 0..50 {
            app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        }
        assert_eq!(app.core.sample_bank.get(2).unwrap().loop_start, start);
    }

    #[test]
    fn test_z_snaps_the_loop_to_zero_crossings_and_undoes() {
        let (mut app, _dir) = app_with_amen();
        set_loop(&mut app, 0, 20000, 60000);
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_editor_field = SampleField::LoopStart;

        app.handle_key(KeyEvent::from(KeyCode::Char('z')));
        let s = app.core.sample_bank.get(0).unwrap();
        let (start, end) = (s.loop_start, s.loop_end);
        assert_ne!((start, end), (20000, 60000), "nothing moved");
        assert!(is_rising_crossing(&app, 0, start) && is_rising_crossing(&app, 0, end));

        app.undo();
        let s = app.core.sample_bank.get(0).unwrap();
        assert_eq!((s.loop_start, s.loop_end), (20000, 60000));
    }

    fn slot_span(app: &App, slot: usize) -> (usize, usize) {
        let s = app.core.sample_bank.get(slot).unwrap();
        (s.trim_start, s.end())
    }

    #[test]
    fn test_slice_range_field_is_reachable_by_tabbing() {
        // Both directions, or the field exists but cannot be got at.
        let mut f = SampleField::SliceSensitivity;
        f = f.next();
        assert_eq!(f, SampleField::SliceRange);
        assert_eq!(f.next(), SampleField::SliceEqual);
        assert_eq!(SampleField::SliceEqual.prev(), SampleField::SliceRange);
        assert_eq!(
            SampleField::SliceRange.prev(),
            SampleField::SliceSensitivity
        );
    }

    #[test]
    fn test_slicing_the_source_re_derives_from_the_whole_sample() {
        // Bound, not dropped: the directory lives until the test ends.
        let (mut app, _dir) = app_with_amen();
        let total = app.core.sample_bank.get(0).unwrap().len();
        app.dialogs.sample_slice_range = rtrack_core::sample::SliceRange::Source;

        app.dialogs.sample_slice_count = 4;
        app.slice_sample(false).unwrap();
        app.dialogs.sample_slice_count = 8;
        app.slice_sample(false).unwrap();

        assert_eq!(slot_span(&app, 0).0, 0);
        assert_eq!(slot_span(&app, 7).1, total, "the second pass lost the tail");
    }

    #[test]
    fn test_slicing_the_source_from_a_later_slice_replaces_the_whole_set() {
        // Starting from the slot being edited left slots 0-2 holding the old
        // division, overlapping the new slices written from slot 3.
        let (mut app, _dir) = app_with_amen();
        let total = app.core.sample_bank.get(0).unwrap().len();
        app.dialogs.sample_slice_range = rtrack_core::sample::SliceRange::Source;
        app.dialogs.sample_slice_count = 8;
        app.slice_sample(false).unwrap();

        app.dialogs.sample_editor_slot = 3;
        app.dialogs.sample_slice_count = 4;
        app.slice_sample(false).unwrap();

        assert_eq!(
            slot_span(&app, 0),
            (0, total / 4),
            "slot 0 kept the old slice"
        );
        assert_eq!(slot_span(&app, 3).1, total);
        assert!(
            app.core.sample_bank.get(4).is_none(),
            "old slices remain past the new set"
        );
    }

    #[test]
    fn test_subdividing_a_slice_stays_inside_it() {
        // Bound, not dropped: the directory lives until the test ends.
        let (mut app, _dir) = app_with_amen();

        app.dialogs.sample_slice_range = rtrack_core::sample::SliceRange::Source;
        app.dialogs.sample_slice_count = 4;
        app.slice_sample(false).unwrap();
        let (outer_start, outer_end) = slot_span(&app, 1);
        assert!(outer_start > 0);

        // Subdividing slice 1 writes its pieces into slots 1 and 2.
        app.dialogs.sample_editor_slot = 1;
        app.dialogs.sample_slice_range = rtrack_core::sample::SliceRange::Span;
        app.dialogs.sample_slice_count = 2;
        app.slice_sample(false).unwrap();

        assert_eq!(slot_span(&app, 1).0, outer_start);
        assert_eq!(slot_span(&app, 2).1, outer_end);
        assert!(
            slot_span(&app, 1).1 < outer_end,
            "the slice was not actually divided"
        );
    }

    #[test]
    fn test_slicing_over_an_instrument_needs_a_second_enter() {
        let (mut app, _dir) = app_with_amen();
        app.core.instruments[3].name = "Bass".to_string();
        app.dialogs.sample_slice_count = 8;

        // First attempt refuses, and arms the confirmation.
        let first = app.slice_sample(false);
        assert!(first.is_err(), "the first attempt should have been refused");
        assert!(app.dialogs.sample_slice_overwrite_armed);
        assert_eq!(app.core.instruments[3].name, "Bass");

        // Second goes ahead, and disarms again.
        let second = app.slice_sample(false);
        assert!(second.is_ok(), "the second attempt should have gone ahead");
        assert!(!app.dialogs.sample_slice_overwrite_armed);
        assert_eq!(app.core.instruments[3].name, "amen_S03");
    }

    #[test]
    fn test_moving_off_the_slice_action_disarms_the_confirmation() {
        let (mut app, _dir) = app_with_amen();
        app.core.instruments[3].name = "Bass".to_string();
        app.mode = Mode::SampleEditor;
        app.dialogs.sample_editor_field = SampleField::SliceEqual;
        let _ = app.slice_sample(false);
        assert!(app.dialogs.sample_slice_overwrite_armed);

        app.handle_key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Tab,
        ));
        assert!(
            !app.dialogs.sample_slice_overwrite_armed,
            "tabbing away left the overwrite armed"
        );
    }

    #[test]
    fn test_undo_puts_back_what_slicing_overwrote() {
        let (mut app, _dir) = app_with_amen();
        app.core.instruments[3].name = "Bass".to_string();
        app.dialogs.sample_slice_count = 8;
        app.dialogs.sample_slice_overwrite_armed = true; // user confirmed

        app.slice_sample(false).unwrap();
        assert_eq!(app.core.instruments[3].name, "amen_S03");
        assert!(app.core.sample_bank.get(3).is_some());

        app.undo();
        assert_eq!(
            app.core.instruments[3].name, "Bass",
            "undo did not restore the overwritten instrument"
        );
        assert!(
            app.core.sample_bank.get(3).is_none(),
            "undo left a slice in a slot that had none"
        );

        app.redo();
        assert_eq!(app.core.instruments[3].name, "amen_S03");
        assert!(app.core.sample_bank.get(3).is_some());
    }

    #[test]
    fn test_a_refused_slice_leaves_no_undo_step() {
        let (mut app, _dir) = app_with_amen();
        app.core.instruments[3].name = "Bass".to_string();
        let before = app.core.history.undo_depth();

        assert!(app.slice_sample(false).is_err());
        assert_eq!(
            app.core.history.undo_depth(),
            before,
            "a slice that wrote nothing should not be undoable"
        );
    }

    #[test]
    fn test_pattern_undo_does_not_disturb_the_sample_bank() {
        // A cell edit records no samples, so undoing it must leave the bank
        // where it is rather than reverting to some earlier snapshot.
        let (mut app, _dir) = app_with_amen();
        app.dialogs.sample_slice_count = 4;
        app.slice_sample(false).unwrap();
        let sliced: Vec<_> = (0..4)
            .map(|s| app.core.sample_bank.get(s).map(|x| (x.trim_start, x.end())))
            .collect();

        app.core.begin_edit();
        app.core
            .song
            .set_cell(0, 0, 0, rtrack_core::tracker::Cell::default());
        app.undo();

        let after: Vec<_> = (0..4)
            .map(|s| app.core.sample_bank.get(s).map(|x| (x.trim_start, x.end())))
            .collect();
        assert_eq!(sliced, after, "a pattern undo changed the sample bank");
    }

    #[test]
    fn test_midi_input_in_insert_mode() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;

        // Simulate MIDI note C-4 (note 60)
        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.volume, Some(100));
    }

    #[test]
    fn test_midi_input_ignored_in_normal_mode() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Normal;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        // Should not have written to pattern
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_midi_input_ignored_during_playback() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = true;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_recording_toggle() {
        let mut app = make_app();
        assert!(!app.core.recording);
        app.toggle_recording();
        assert!(app.core.recording);
        app.toggle_recording();
        assert!(!app.core.recording);
    }

    #[test]
    fn test_punch_in_records_at_engine_position() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = true;
        app.core.recording = true;
        // Position engine at row 5
        app.core.engine.row = 5;
        app.core.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 110,
        });

        let pattern_idx = app.core.song.order[0];
        // Written at engine row (5), not cursor_row (0)
        let cell = app.core.song.patterns[pattern_idx].get(5, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.volume, Some(110));
        // Cursor row should not have advanced
        assert_eq!(app.cursor_row, 0);
        // Pattern at cursor row should be untouched
        let cell0 = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell0.note.is_none());
    }

    #[test]
    fn test_punch_in_no_record_without_flag() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = true;
        app.core.recording = false;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        // Should not record (preview only)
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_punch_in_noteoff_recorded() {
        use rtrack_core::midi::MidiInputEvent;
        use rtrack_core::tracker::Note;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = true;
        app.core.recording = true;
        app.core.engine.row = 3;
        app.core.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOff {
            channel: 0,
            note: 60,
        });

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(3, 0);
        assert_eq!(cell.note, Some(Note::Off));
    }

    #[test]
    fn test_noteoff_not_recorded_in_step_mode() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = false;

        app.handle_midi_input(MidiInputEvent::NoteOff {
            channel: 0,
            note: 60,
        });

        // Step mode should NOT record note-off from MIDI
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_punch_in_auto_fills_instrument() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].default_instrument = Some(7);
        app.mode = Mode::Insert;
        app.core.playing = true;
        app.core.recording = true;
        app.core.engine.row = 2;
        app.core.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 64,
            velocity: 100,
        });

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(2, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(7));
    }

    #[test]
    fn test_step_record_auto_fills_instrument() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].default_instrument = Some(3);
        app.mode = Mode::Insert;
        app.core.playing = false;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(3));
    }

    #[test]
    fn test_punch_in_sets_dirty() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.core.playing = true;
        app.core.recording = true;
        app.core.dirty = false;

        app.handle_midi_input(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });

        assert!(app.core.dirty);
    }

    #[test]
    fn test_aftertouch_modulates_filter_cutoff() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;
        app.core.channels[0].effects_params.filter_cutoff = 1000.0;

        // Channel pressure at max should set cutoff to ~20kHz
        app.handle_midi_input(MidiInputEvent::ChannelPressure {
            channel: 0,
            pressure: 127,
        });
        assert!((app.core.channels[0].effects_params.filter_cutoff - 20000.0).abs() < 1.0);

        // Channel pressure at 0 should set cutoff to 20 Hz
        app.handle_midi_input(MidiInputEvent::ChannelPressure {
            channel: 0,
            pressure: 0,
        });
        assert!((app.core.channels[0].effects_params.filter_cutoff - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_aftertouch_ignored_when_filter_disabled() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = false;
        app.core.channels[0].effects_params.filter_cutoff = 1000.0;

        app.handle_midi_input(MidiInputEvent::ChannelPressure {
            channel: 0,
            pressure: 127,
        });
        // Filter cutoff should be unchanged
        assert!((app.core.channels[0].effects_params.filter_cutoff - 1000.0).abs() < 0.1);
    }

    #[test]
    fn test_poly_pressure_modulates_filter() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;

        // Poly pressure should also modulate filter
        app.handle_midi_input(MidiInputEvent::PolyPressure {
            channel: 0,
            note: 60,
            pressure: 64,
        });
        // Midpoint: 20 * 1000^(64/127) ~= 632 Hz
        let cutoff = app.core.channels[0].effects_params.filter_cutoff;
        assert!(
            cutoff > 500.0 && cutoff < 800.0,
            "Expected ~632 Hz, got {}",
            cutoff
        );
    }

    #[test]
    fn test_midi_learn_binds_cc_to_param() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;

        // Arm learn for filter cutoff on channel 0
        app.core.midi_learn_pending = Some((0, LearnableParam::FilterCutoff));

        // Send a CC -- should bind CC7 to filter cutoff
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 7,
            value: 64,
        });

        // Learn pending should be consumed
        assert!(app.core.midi_learn_pending.is_none());
        assert_eq!(app.core.midi_cc_mappings.len(), 1);
        assert_eq!(app.core.midi_cc_mappings[0].cc, 7);
        assert_eq!(app.core.midi_cc_mappings[0].channel, 0);
        assert_eq!(
            app.core.midi_cc_mappings[0].param,
            LearnableParam::FilterCutoff
        );
    }

    #[test]
    fn test_midi_learn_cc_applies_to_param() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;
        app.core.channels[0].effects_params.filter_cutoff = 1000.0;

        // Set up a mapping: CC1 -> filter cutoff ch0
        app.core.midi_cc_mappings.push(MidiCcMapping {
            cc: 1,
            channel: 0,
            param: LearnableParam::FilterCutoff,
        });

        // Send CC1 at max value -> should set cutoff to ~20000
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 1,
            value: 127,
        });
        assert!((app.core.channels[0].effects_params.filter_cutoff - 20000.0).abs() < 1.0);

        // Send CC1 at zero -> cutoff to 20 Hz
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 1,
            value: 0,
        });
        assert!((app.core.channels[0].effects_params.filter_cutoff - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_midi_learn_unmapped_cc_thru() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        // No mappings, CC should just pass through (no crash, no effect on params)
        let cutoff_before = app.core.channels[0].effects_params.filter_cutoff;
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 74,
            value: 100,
        });
        assert_eq!(
            app.core.channels[0].effects_params.filter_cutoff,
            cutoff_before
        );
    }

    #[test]
    fn test_midi_learn_replaces_existing_mapping() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;

        // Map CC1 -> filter cutoff
        app.core.midi_cc_mappings.push(MidiCcMapping {
            cc: 1,
            channel: 0,
            param: LearnableParam::FilterCutoff,
        });

        // Now learn again: CC2 -> filter cutoff (same param, should replace)
        app.core.midi_learn_pending = Some((0, LearnableParam::FilterCutoff));
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 2,
            value: 64,
        });

        assert_eq!(app.core.midi_cc_mappings.len(), 1);
        assert_eq!(app.core.midi_cc_mappings[0].cc, 2);
    }

    #[test]
    fn test_midi_learn_from_track_config() {
        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;

        // Open track config, navigate to filter cutoff (Name=0, Inst=1, Filter=2, Cutoff=3)
        run_command(&mut app, "fx");
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.ch_fx_field, 3);

        // Press 'l' to arm learn
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert!(app.core.midi_learn_pending.is_some());
        let (ch, param) = app.core.midi_learn_pending.unwrap();
        assert_eq!(ch, 0);
        assert_eq!(param, LearnableParam::FilterCutoff);
    }

    #[test]
    fn test_midi_unlearn_from_track_config() {
        let mut app = make_app();

        // Add a mapping
        app.core.midi_cc_mappings.push(MidiCcMapping {
            cc: 1,
            channel: 0,
            param: LearnableParam::FilterCutoff,
        });
        assert_eq!(app.core.midi_cc_mappings.len(), 1);

        // Open track config, navigate to cutoff field
        run_command(&mut app, "fx");
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }

        // Press 'u' to unlearn
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert!(app.core.midi_cc_mappings.is_empty());
    }

    #[test]
    fn test_learnable_param_map_cc_ranges() {
        // Filter cutoff: exponential 20..20000
        assert!((LearnableParam::FilterCutoff.map_cc(0) - 20.0).abs() < 0.1);
        assert!((LearnableParam::FilterCutoff.map_cc(127) - 20000.0).abs() < 1.0);

        // Linear 0..1 params
        assert!((LearnableParam::ReverbMix.map_cc(0)).abs() < 0.001);
        assert!((LearnableParam::ReverbMix.map_cc(127) - 1.0).abs() < 0.001);

        // Delay feedback: 0..0.95
        assert!((LearnableParam::DelayFeedback.map_cc(127) - 0.95).abs() < 0.01);

        // Distortion drive: 1..20
        assert!((LearnableParam::DistortionDrive.map_cc(0) - 1.0).abs() < 0.01);
        assert!((LearnableParam::DistortionDrive.map_cc(127) - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_midi_learn_multiple_params_same_cc() {
        use rtrack_core::midi::MidiInputEvent;

        let mut app = make_app();
        app.core.channels[0].effects_params.filter_enabled = true;
        app.core.channels[0].effects_params.chorus_enabled = true;

        // Map CC1 to both filter cutoff and chorus rate (different params, same CC)
        app.core.midi_cc_mappings.push(MidiCcMapping {
            cc: 1,
            channel: 0,
            param: LearnableParam::FilterCutoff,
        });
        app.core.midi_cc_mappings.push(MidiCcMapping {
            cc: 1,
            channel: 0,
            param: LearnableParam::ChorusRate,
        });

        // Send CC1 at 127 -- both should update
        app.handle_midi_input(MidiInputEvent::CC {
            channel: 0,
            controller: 1,
            value: 127,
        });
        assert!((app.core.channels[0].effects_params.filter_cutoff - 20000.0).abs() < 1.0);
        assert!((app.core.channels[0].effects_params.chorus_rate - 10.0).abs() < 0.1);
    }

    #[test]
    fn test_pattern_break_clamps_row() {
        let mut app = make_app();
        // Second pattern with only 16 rows
        let pat2 = app.core.song.add_pattern();
        app.core.song.patterns[pat2].rows = 16;
        app.core.song.patterns[pat2].data.truncate(16);
        app.core.song.order.push(pat2);

        // Pattern break to row 99 (beyond bounds of pattern 2)
        let pat_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pat_idx].get_mut(0, 0);
        cell.fx[0] = Some(Fx::new(FxCmd::Break, 99));

        app.play();
        app.core.process_tick();
        assert_eq!(app.core.engine.order, 1);
        assert_eq!(app.core.engine.row, 15); // clamped to max row
    }

    #[test]
    fn test_song_settings_open_close() {
        let mut app = make_app();
        app.open_song_settings();
        assert_eq!(app.mode, Mode::SongSettings);
        assert_eq!(app.dialogs.settings_field, SettingsField::Title);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_song_settings_edit_title() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_edit_buf = "New Title".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.core.song.title, "New Title");
    }

    #[test]
    fn tabbing_through_settings_without_editing_leaves_the_song_clean() {
        let mut app = make_app();
        app.core.dirty = false;
        app.open_song_settings();

        // A full cycle of every field, back to Title.
        for _ in 0..9 {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!app.core.dirty, "no field changed, so nothing was edited");
    }

    #[test]
    fn editing_a_settings_field_still_marks_the_song_dirty() {
        let mut app = make_app();
        app.core.dirty = false;
        app.open_song_settings();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for c in "140".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.core.song.bpm, 140);
        assert!(app.core.dirty);
    }

    #[test]
    fn typing_in_a_settings_field_replaces_the_seeded_value() {
        use rtrack_core::theory::{Scale, ScaleSetting};
        use rtrack_core::tracker::NoteValue;

        let mut app = make_app();
        app.open_song_settings();
        // Tab to Scale, whose buffer is seeded with "off".
        for _ in 0..8 {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.dialogs.settings_field, SettingsField::Scale);
        assert_eq!(app.dialogs.settings_edit_buf, "off");

        for c in "c minor".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.dialogs.settings_edit_buf, "c minor");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.core.song.scale,
            Some(ScaleSetting::new(NoteValue::C, Scale::Minor))
        );
    }

    #[test]
    fn backspace_edits_the_seeded_value_instead_of_replacing_it() {
        let mut app = make_app();
        app.core.song.bpm = 140;
        app.open_song_settings();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.dialogs.settings_edit_buf, "140");

        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        assert_eq!(app.dialogs.settings_edit_buf, "145");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.core.song.bpm, 145);
    }

    #[test]
    fn test_song_settings_edit_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Bpm;
        app.dialogs.settings_edit_buf = "140".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.core.song.bpm, 140);
    }

    #[test]
    fn test_song_settings_edit_channels() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Channels;
        app.dialogs.settings_edit_buf = "8".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.core.song.channels, 8);
        assert_eq!(app.core.channels.len(), 8);
    }

    #[test]
    fn reducing_the_channel_count_unlinks_the_removed_tracks() {
        let mut app = make_app();
        app.cursor_channel = 3;
        run_command(&mut app, "phrase");
        let set_channels = |app: &mut App, n: &str| {
            app.open_song_settings();
            app.dialogs.settings_field = SettingsField::Channels;
            app.dialogs.settings_edit_buf = n.to_string();
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        };
        app.core.channels[3].name = "bass".into();
        set_channels(&mut app, "2");
        assert_eq!(app.cursor_channel, 1, "the cursor stays on a track");
        set_channels(&mut app, "4");
        let pos = app.current_order_position();
        assert!(app.core.song.pattern_at(pos).unwrap().link(3).is_none());

        // Undo returns to two tracks, then to the four with their config.
        app.undo();
        app.undo();
        assert_eq!(app.core.channels[3].name, "bass");
        assert!(app.core.song.pattern_at(pos).unwrap().link(3).is_some());
    }

    #[test]
    fn test_song_settings_clamps_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Bpm;

        // Too low
        app.dialogs.settings_edit_buf = "10".to_string();
        app.settings_apply_field();
        assert_eq!(app.core.song.bpm, 32);

        // Too high
        app.dialogs.settings_edit_buf = "500".to_string();
        app.settings_apply_field();
        assert_eq!(app.core.song.bpm, 300);
    }

    #[test]
    fn test_instrument_list_open_close() {
        let mut app = make_app();
        app.open_instrument_list();
        assert_eq!(app.mode, Mode::InstrumentList);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_instrument_list_navigation() {
        let mut app = make_app();
        app.open_instrument_list();
        assert_eq!(app.dialogs.instrument_cursor, 0);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.instrument_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.dialogs.instrument_cursor, 17);

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.dialogs.instrument_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.dialogs.instrument_cursor, 0);
    }

    #[test]
    fn test_instrument_name_edit() {
        let mut app = make_app();
        app.open_instrument_list();

        app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));

        assert_eq!(app.core.instruments[0].name, "Test");

        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.core.instruments[0].name, "Tes");
    }

    #[test]
    fn test_theme_cycling() {
        let mut app = make_app();
        let initial = app.theme_index;

        app.cycle_theme();
        assert_ne!(app.theme_index, initial);

        // Cycle through all themes back to start
        let count = crate::tui::theme::THEME_NAMES.len();
        for _ in 0..count - 1 {
            app.cycle_theme();
        }
        assert_eq!(app.theme_index, initial);
    }

    #[test]
    fn test_midi_clock_toggle() {
        let mut app = make_app();
        let initial = app.core.midi.clock_enabled;

        app.toggle_midi_clock();
        assert_ne!(app.core.midi.clock_enabled, initial);

        app.toggle_midi_clock();
        assert_eq!(app.core.midi.clock_enabled, initial);
    }

    #[test]
    fn a_click_lands_on_the_field_drawn_under_it() {
        use crate::tui::pattern_editor::{CHANNEL_WIDTH, ROW_NUM_WIDTH, SEPARATOR_WIDTH};
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = make_app();
        let click = |app: &mut App, column: u16| {
            app.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                },
                0,
                0,
            );
        };
        let channel_start =
            |ch: u16| ROW_NUM_WIDTH + SEPARATOR_WIDTH + ch * (CHANNEL_WIDTH + SEPARATOR_WIDTH);

        // The last character of the fourth channel's effect field.
        click(&mut app, channel_start(3) + CHANNEL_WIDTH - 1);
        assert_eq!((app.cursor_channel, app.cursor_sub), (3, SubColumn::Effect));

        // The first character of the fourth channel's note.
        click(&mut app, channel_start(3));
        assert_eq!((app.cursor_channel, app.cursor_sub), (3, SubColumn::Note));
    }

    #[test]
    fn test_mouse_scroll() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let mut app = make_app();
        app.cursor_row = 10;

        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            0,
            0,
        );
        assert_eq!(app.cursor_row, 7);

        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            0,
            0,
        );
        assert_eq!(app.cursor_row, 10);
    }

    #[test]
    fn test_f6_opens_settings() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::SongSettings);
    }

    #[test]
    fn test_f7_opens_instruments() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::InstrumentList);
    }

    #[test]
    fn test_synth_editor_open_close() {
        let mut app = make_app();
        // Open instrument list
        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::InstrumentList);
        // Tab opens synth editor
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::SynthEditor);
        // Should have initialized synth params
        assert!(app.core.instruments[0].synth_params().is_some());
        // Navigate fields
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.dialogs.synth_editor_field, SynthField::Attack);
        // Adjust value
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let attack = app.core.instruments[0].synth_params().unwrap().attack;
        assert!(attack > 0.005); // Saw default is 0.005, +0.001
                                 // Esc closes
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_ne!(app.mode, Mode::SynthEditor);
    }

    #[test]
    fn test_synth_editor_delete_clears_params() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(app.core.instruments[0].synth_params().is_some());
        // Delete clears params
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(app.core.instruments[0].is_default(0));
    }

    #[test]
    fn the_instrument_list_sets_midi_output_program_and_channel() {
        use rtrack_core::{InstrumentKind, MidiOutput};
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
        app.handle_key(ctrl(KeyCode::Char('o')));
        app.handle_key(ctrl(KeyCode::Right));
        app.handle_key(ctrl(KeyCode::Up));
        assert_eq!(
            app.core.instruments[0].kind,
            InstrumentKind::Midi {
                program: 1,
                channel: Some(0),
                output: MidiOutput::External { port: None },
            }
        );
        assert!(
            app.core.instruments[0].name.is_empty(),
            "a binding typed into the name"
        );

        app.undo();
        app.undo();
        app.undo();
        assert!(app.core.instruments[0].is_default(0));
    }

    #[test]
    fn the_sample_editor_sets_the_instruments_filter_and_drive() {
        use rtrack_core::audio::synth::FilterType;
        use rtrack_core::{InstrumentKind, SampleFx};
        let mut app = make_app();
        app.core.instruments[0].kind = InstrumentKind::sample(0);
        app.open_sample_editor();
        let key = |app: &mut App, code| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let fx = |app: &App| match app.core.instruments[0].kind {
            InstrumentKind::Sample { fx, .. } => fx,
            _ => panic!("not a sample"),
        };

        app.dialogs.sample_editor_field = SampleField::Cutoff;
        key(&mut app, KeyCode::Up);
        assert_eq!(app.status_message.as_deref(), Some("Filter is off"));

        app.dialogs.sample_editor_field = SampleField::Filter;
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Up);
        assert_eq!(fx(&app).filter.unwrap().kind, FilterType::HighPass);
        app.dialogs.sample_editor_field = SampleField::Cutoff;
        key(&mut app, KeyCode::Right);
        let cutoff = fx(&app).filter.unwrap().cutoff_hz;
        assert!(
            (cutoff - 8000.0 * 2f32.powf(10.0 / 12.0)).abs() < 1.0,
            "{cutoff}"
        );
        app.dialogs.sample_editor_field = SampleField::Drive;
        key(&mut app, KeyCode::Right);
        assert!((fx(&app).drive - 2.0).abs() < 1e-6);

        app.undo();
        app.undo();
        app.undo();
        assert_eq!(fx(&app), SampleFx::default(), "each field is one undo step");

        app.core.instruments[1].kind = InstrumentKind::default_for(1);
        app.dialogs.sample_editor_slot = 1;
        key(&mut app, KeyCode::Up);
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("does not play a sample")));
    }

    #[test]
    fn the_synth_editor_leaves_a_sample_instrument_alone() {
        let mut app = make_app();
        app.core.instruments[0].kind = rtrack_core::InstrumentKind::sample(0);
        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_ne!(app.mode, Mode::SynthEditor);
        assert_eq!(app.core.instruments[0].sample_index(), Some(0));
    }

    #[test]
    fn test_f8_cycles_theme() {
        let mut app = make_app();
        let initial = app.theme_index;
        app.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
        assert_ne!(app.theme_index, initial);
    }

    #[test]
    fn test_ctrl_e_exports_midi() {
        let mut app = make_app();
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_export.rtrk");
        app.core.file_path = Some(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));

        let midi_path = path.with_extension("mid");
        assert!(midi_path.exists());
        let _ = std::fs::remove_file(midi_path);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_m_toggles_clock() {
        let mut app = make_app();
        let initial = app.core.midi.clock_enabled;
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL));
        assert_ne!(app.core.midi.clock_enabled, initial);
    }

    #[test]
    fn test_dirty_flag_on_edit() {
        let mut app = make_app();
        assert!(!app.core.dirty);
        app.mode = Mode::Insert;
        // Enter a note (triggers push_undo -> dirty)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(app.core.dirty);
    }

    #[test]
    fn test_dirty_flag_cleared_on_save() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(app.core.dirty);
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_dirty.rtrk");
        app.core.file_path = Some(path.clone());
        app.save();
        assert!(!app.core.dirty);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_quit_confirm_when_dirty() {
        let mut app = make_app();
        app.core.dirty = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        // Should enter quit confirm mode, not quit
        assert!(!app.should_quit);
        assert_eq!(app.mode, Mode::QuitConfirm);
        // Press 'n' (any key other than y/s) cancels
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(!app.should_quit);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_quit_confirm_yes() {
        let mut app = make_app();
        app.core.dirty = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::QuitConfirm);
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn test_quit_no_confirm_when_clean() {
        let mut app = make_app();
        assert!(!app.core.dirty);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn test_note_transpose_up() {
        let mut app = make_app();
        // Place a C-4 note
        let pattern_idx = app.core.song.order[0];
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::C,
                    octave: 4,
                }),
                ..Default::default()
            },
        );
        // Shift+Up transposes up 1 semitone
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(
            cell.note,
            Some(Note::On {
                value: rtrack_core::tracker::NoteValue::Cs,
                octave: 4
            })
        );
    }

    #[test]
    fn test_note_transpose_down() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::C,
                    octave: 4,
                }),
                ..Default::default()
            },
        );
        // Shift+Down transposes down 1 semitone (C-4 -> B-3)
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(
            cell.note,
            Some(Note::On {
                value: rtrack_core::tracker::NoteValue::B,
                octave: 3
            })
        );
    }

    #[test]
    fn test_block_select_toggle() {
        let mut app = make_app();
        assert!(app.block_anchor.is_none());
        // Ctrl+B toggles block selection
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.block_anchor, Some((0, 0)));
        // Toggle off
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(app.block_anchor.is_none());
    }

    #[test]
    fn test_block_copy_paste() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        // Place notes in rows 0-1, channels 0-1
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::C,
                    octave: 4,
                }),
                ..Default::default()
            },
        );
        app.core.song.patterns[pattern_idx].set_cell(
            1,
            1,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::E,
                    octave: 4,
                }),
                ..Default::default()
            },
        );
        // Start block at (0,0)
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        // Move cursor to (1,1)
        app.cursor_row = 1;
        app.cursor_channel = 1;
        // Copy block
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.clipboard.has_block());
        let clip = app.clipboard.block().expect("a copied block");
        assert_eq!(clip.len(), 2); // 2 rows
        assert_eq!(clip[0].len(), 2); // 2 channels
                                      // Paste at (4,0)
        app.cursor_row = 4;
        app.cursor_channel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        let cell = app.core.song.patterns[pattern_idx].get(4, 0);
        assert_eq!(
            cell.note,
            Some(Note::On {
                value: rtrack_core::tracker::NoteValue::C,
                octave: 4
            })
        );
        let cell2 = app.core.song.patterns[pattern_idx].get(5, 1);
        assert_eq!(
            cell2.note,
            Some(Note::On {
                value: rtrack_core::tracker::NoteValue::E,
                octave: 4
            })
        );
    }

    #[test]
    fn test_block_cut_clears_selection() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::C,
                    octave: 4,
                }),
                ..Default::default()
            },
        );
        // Start block at (0,0), cursor at (0,0)
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        // Cut
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        // Original cell should be cleared
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
        // Block anchor should be cleared
        assert!(app.block_anchor.is_none());
    }

    #[test]
    fn test_atomic_save() {
        let mut app = make_app();
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_atomic.rtrk");
        app.core.file_path = Some(path.clone());
        app.save();
        assert!(path.exists());
        // Temp file should not exist
        let temp_path = tmp_dir.join(format!(".rtrack_save_{}.tmp", std::process::id()));
        assert!(!temp_path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_follow_mode_toggle() {
        let mut app = make_app();
        assert!(app.follow_playback); // on by default
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(!app.follow_playback);
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(app.follow_playback);
    }

    #[test]
    fn test_track_config_open_via_enter() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::TrackConfig);
        // Enter again closes
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        // Esc also closes
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::TrackConfig);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_channel_rename() {
        let mut app = make_app();
        assert_eq!(app.core.channels[0].name, "");
        // Enter opens track config (name field focused)
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::TrackConfig);
        assert_eq!(app.ch_fx_field, 0); // Name field
                                        // Type a name
        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.rename_buf, "Kick");
        // Esc confirms and closes
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].name, "Kick");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_channel_rename_backspace() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.rename_buf, "A");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].name, "A");
    }

    #[test]
    fn test_interpolate_volume() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        // Set volume at row 0 and row 4
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                volume: Some(0),
                ..Default::default()
            },
        );
        app.core.song.patterns[pattern_idx].set_cell(
            4,
            0,
            rtrack_core::tracker::Cell {
                volume: Some(100),
                ..Default::default()
            },
        );
        // Select block from (0,0) to (4,0)
        app.block_anchor = Some((0, 0));
        app.cursor_row = 4;
        app.cursor_channel = 0;
        // Interpolate
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        // Check intermediate values
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(0, 0).volume,
            Some(0)
        );
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(1, 0).volume,
            Some(25)
        );
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(2, 0).volume,
            Some(50)
        );
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(3, 0).volume,
            Some(75)
        );
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(4, 0).volume,
            Some(100)
        );
    }

    #[test]
    fn a_track_is_linked_edited_shared_and_unlinked() {
        use rtrack_core::tracker::PhraseTiming;
        let mut app = make_app();
        let pos = app.current_order_position();
        let link = |app: &App, ch| app.core.song.pattern_at(pos).unwrap().link(ch);

        run_command(&mut app, "phrase");
        assert_eq!(link(&app, 0).map(|l| l.phrase), Some(0));
        assert_eq!(app.core.song.phrases.len(), 1);

        // Typing into the linked track writes the phrase.
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Volume;
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        assert_eq!(app.core.song.phrases[0].rows[0].volume, Some(7));

        // A second track picks the phrase from the list.
        app.mode = Mode::Normal;
        app.cursor_channel = 1;
        run_command(&mut app, "phrases");
        assert_eq!(app.mode, Mode::PhraseList);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(link(&app, 1).map(|l| l.phrase), Some(0));
        assert_eq!(app.core.song.cell_at(pos, 0, 1).volume, Some(7));

        run_command(&mut app, "run");
        assert_eq!(link(&app, 1).map(|l| l.timing), Some(PhraseTiming::Running));

        run_command(&mut app, "unlink");
        assert_eq!(link(&app, 1), None);
        assert_eq!(
            app.core.song.cell_at(pos, 0, 1).volume,
            Some(7),
            "unlinking keeps the rows"
        );

        app.undo();
        assert!(link(&app, 1).is_some(), "unlink is undoable");
    }

    #[test]
    fn tr_sets_a_linked_tracks_transpose() {
        let mut app = make_app();
        let pos = app.current_order_position();
        run_command(&mut app, "tr 3");
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("not linked")));
        run_command(&mut app, "phrase");
        run_command(&mut app, "tr -5");
        let link = app.core.song.pattern_at(pos).unwrap().link(0).unwrap();
        assert_eq!(link.transpose, -5);
        run_command(&mut app, "tr +2");
        let link = app.core.song.pattern_at(pos).unwrap().link(0).unwrap();
        assert_eq!(link.transpose, 2);
        app.undo();
        let link = app.core.song.pattern_at(pos).unwrap().link(0).unwrap();
        assert_eq!(link.transpose, -5);
    }

    #[test]
    fn the_table_editor_builds_a_table() {
        use rtrack_core::tracker::{FxCmd, Table};
        let mut app = make_app();
        let key = |app: &mut App, code: KeyCode, mods: KeyModifiers| {
            app.handle_key(KeyEvent::new(code, mods));
        };
        let none = KeyModifiers::NONE;
        let table = |app: &App| app.core.song.tables.get(&5).cloned();

        run_command(&mut app, "table 05");
        assert_eq!(app.mode, Mode::TableEditor);
        assert!(table(&app).is_none(), "opening creates nothing");

        // Three rows: 0, +4, +7, looping from row 0, 2 ticks a row.
        key(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('l'), none);
        key(&mut app, KeyCode::Char(']'), none);
        key(&mut app, KeyCode::Down, none);
        for _ in 0..4 {
            key(&mut app, KeyCode::Char('+'), none);
        }
        key(&mut app, KeyCode::Down, none);
        for _ in 0..7 {
            key(&mut app, KeyCode::Char('+'), none);
        }
        // Row 2: volume 30, effect PVB44.
        key(&mut app, KeyCode::Right, none);
        key(&mut app, KeyCode::Char('3'), none);
        key(&mut app, KeyCode::Char('0'), none);
        key(&mut app, KeyCode::Right, none);
        for c in ['4', '4', '4'] {
            key(&mut app, KeyCode::Char(c), none);
        }
        let t = table(&app).unwrap();
        let transposes: Vec<i8> = t.rows.iter().map(|r| r.transpose).collect();
        assert_eq!(transposes, vec![0, 4, 7]);
        assert_eq!((t.ticks_per_row, t.loop_from), (2, Some(0)));
        assert_eq!(t.rows[2].volume, Some(0x30));
        assert_eq!(t.rows[2].fx.unwrap().cmd, FxCmd::Vibrato);

        // A song command is refused, by digit and by name.
        key(&mut app, KeyCode::Delete, none);
        key(&mut app, KeyCode::Char('b'), none);
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("JMP")));
        assert_eq!(table(&app).unwrap().rows[2].fx, None);
        key(&mut app, KeyCode::Char('='), none);
        app.command_buf = "tfx JMP00".to_string();
        key(&mut app, KeyCode::Enter, none);
        assert_eq!(app.mode, Mode::TableEditor);
        assert_eq!(table(&app).unwrap().rows[2].fx, None);
        key(&mut app, KeyCode::Char('='), none);
        app.command_buf = "tfx C4AA0".to_string();
        key(&mut app, KeyCode::Enter, none);
        assert_eq!(
            table(&app).unwrap().rows[2].fx.unwrap().cmd,
            FxCmd::Cc(0x4A)
        );

        key(&mut app, KeyCode::Char(':'), none);
        app.command_buf = "tname arp".to_string();
        key(&mut app, KeyCode::Enter, none);
        assert_eq!(table(&app).unwrap().name, "arp");

        app.undo();
        assert_eq!(table(&app).unwrap().name, "");
        assert!(Table::allows(table(&app).unwrap().rows[2].fx.unwrap().cmd));

        run_command(&mut app, "tdel");
        assert!(table(&app).is_none());
    }

    #[test]
    fn table_opens_the_cursor_instruments_table() {
        let mut app = make_app();
        app.core.channels[0].default_instrument = Some(0x0C);
        run_command(&mut app, "table");
        assert_eq!(app.dialogs.table_number, 0x0C);
    }

    #[test]
    fn the_phrase_list_renames_and_resizes() {
        let mut app = make_app();
        run_command(&mut app, "phrase");
        run_command(&mut app, "phrases");
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(app.core.song.phrases[0].name.ends_with('x'));
        let len = app.core.song.phrases[0].rows.len();
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.core.song.phrases[0].rows.len(), len - 1);
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.handle_key(KeyEvent::new(code, mods));
    }

    #[test]
    fn the_phrase_view_edits_the_tracks_phrase_and_returns_to_the_pattern() {
        let mut app = make_app();
        let pos = app.current_order_position();
        app.cursor_channel = 1;
        run_command(&mut app, "phrase");
        app.cursor_row = 5;

        press(&mut app, KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(app.phrase_view.map(|v| v.phrase), Some(0));
        assert_eq!(app.cursor_row, 5, "opens at the phrase row shown");

        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Volume;
        press(&mut app, KeyCode::Char('7'), KeyModifiers::NONE);
        assert_eq!(app.core.song.phrases[0].rows[5].volume, Some(7));
        assert_eq!(app.core.song.cell_at(pos, 5, 1).volume, Some(7));

        // The cursor stays in the phrase's column.
        app.cursor_sub = SubColumn::Effect;
        press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.cursor_channel, 1);
        assert_eq!(app.visible_channels(), 1..2);

        app.mode = Mode::Normal;
        app.cursor_row = 0;
        assert_eq!(app.pattern_row(), 5, "playback starts from the pattern row");
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE);
        assert!(app.phrase_view.is_none());
        assert_eq!(app.cursor_row, 5);

        app.undo();
        assert_eq!(app.core.song.phrases[0].rows[5].volume, None);
    }

    #[test]
    fn the_phrase_view_steps_adds_and_survives_undo() {
        let mut app = make_app();
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE);
        assert!(app.phrase_view.is_none(), "nothing to show yet");
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("No phrases")));

        app.core.song.add_phrase(4);
        app.core.song.phrases[0].rows[1].volume = Some(9);
        run_command(&mut app, "pedit");
        let shown = |app: &App| app.phrase_view.map(|v| v.phrase);
        assert_eq!(shown(&app), Some(0), "an unlinked track opens phrase 00");

        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(shown(&app), Some(1));
        assert_eq!(app.core.song.phrases[1].rows[1].volume, Some(9));
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(shown(&app), Some(2));
        let pattern_rows = app.core.song.rows_at(app.current_order_position());
        assert_eq!(app.current_pattern_rows(), pattern_rows);

        press(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(shown(&app), Some(2), "stops at the last phrase");
        press(&mut app, KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(shown(&app), Some(1));

        run_command(&mut app, "pedit 2");
        app.undo();
        assert_eq!(app.core.song.phrases.len(), 2);
        assert_eq!(shown(&app), Some(1), "the undone phrase is no longer shown");

        run_command(&mut app, "pedit 9");
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("No such")));
        assert_eq!(shown(&app), Some(1));
    }

    #[test]
    fn rows_insert_and_delete_in_the_phrase_not_the_pattern() {
        let mut app = make_app();
        run_command(&mut app, "phrase");
        let pattern_idx = app.core.song.order[app.current_order_position()];
        app.core.song.patterns[pattern_idx].links[0] = None;
        app.core.song.patterns[pattern_idx].data[0][0].volume = Some(1);
        app.core.song.phrases[0].rows[0].volume = Some(2);
        let len = app.core.song.phrases[0].rows.len();

        run_command(&mut app, "pedit 0");
        press(&mut app, KeyCode::Insert, KeyModifiers::NONE);
        assert_eq!(app.core.song.phrases[0].rows[1].volume, Some(2));
        assert_eq!(app.core.song.phrases[0].rows.len(), len);
        assert_eq!(
            app.core.song.patterns[pattern_idx].data[0][0].volume,
            Some(1)
        );

        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.core.song.phrases[0].rows[0].volume, Some(2));
    }

    #[test]
    fn the_phrase_list_shows_a_phrase_while_the_view_is_open() {
        let mut app = make_app();
        app.core.song.add_phrase(4);
        app.core.song.add_phrase(4);
        run_command(&mut app, "pedit 1");
        run_command(&mut app, "phrases");
        assert_eq!(
            app.dialogs.phrase_cursor, 1,
            "the list starts on the phrase shown"
        );
        press(&mut app, KeyCode::Up, KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.phrase_view.map(|v| v.phrase), Some(0));
        let pos = app.current_order_position();
        assert!(
            app.core.song.pattern_at(pos).unwrap().link(0).is_none(),
            "nothing is linked"
        );
    }

    #[test]
    fn loading_a_song_closes_the_phrase_view() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");
        let mut app = make_app();
        app.core.file_path = Some(path.clone());
        app.save();
        app.core.song.add_phrase(4);
        run_command(&mut app, "pedit 0");
        app.load_file(path);
        assert!(app.phrase_view.is_none());
    }

    #[test]
    fn linking_to_a_missing_phrase_changes_nothing() {
        let mut app = make_app();
        let depth = app.core.history.undo_depth();
        run_command(&mut app, "phrase 5");
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("No such")));
        assert_eq!(app.core.history.undo_depth(), depth);
        assert!(app.core.song.phrases.is_empty());
    }

    #[test]
    fn the_second_effect_column_is_reached_once_shown() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Effect;
        app.move_cursor_right();
        assert_eq!((app.cursor_channel, app.cursor_sub), (1, SubColumn::Note));

        app.mode = Mode::Normal;
        run_command(&mut app, "fx2");
        app.mode = Mode::Insert;
        app.cursor_channel = 0;
        app.cursor_sub = SubColumn::Effect;
        app.move_cursor_right();
        assert_eq!(app.cursor_sub, SubColumn::Effect2);
        for c in ['4', '4', '2'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let order = app.current_order_position();
        assert_eq!(
            app.core.song.cell_at(order, 0, 0).fx[1],
            Some(Fx::new(FxCmd::Vibrato, 0x42))
        );
    }

    #[test]
    fn a_pattern_using_the_second_column_shows_it() {
        let mut app = make_app();
        assert_eq!(app.fx_columns(), 1);
        let order = app.current_order_position();
        app.core.song.set_cell(
            order,
            3,
            2,
            rtrack_core::tracker::Cell {
                fx: [None, Some(Fx::new(FxCmd::Arp, 0x37))],
                ..Default::default()
            },
        );
        assert_eq!(app.fx_columns(), 2);
    }

    #[test]
    fn effects_are_edited_by_name_through_the_prompt() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Effect;
        app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.command_buf, "e ");
        for c in "c0740".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert);
        let order = app.current_order_position();
        assert_eq!(
            app.core.song.cell_at(order, 0, 0).fx[0],
            Some(Fx::new(FxCmd::Cc(7), 0x40))
        );

        // The prompt opens on the current text; bad text leaves it alone.
        app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE));
        assert_eq!(app.command_buf, "e C0740");
        app.command_buf = "e XYZ00".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.status_message.is_some());
        assert_eq!(
            app.core.song.cell_at(order, 0, 0).fx[0],
            Some(Fx::new(FxCmd::Cc(7), 0x40))
        );

        app.undo();
        assert_eq!(app.core.song.cell_at(order, 0, 0).fx[0], None);
    }

    #[test]
    fn test_interpolate_effect_value() {
        let mut app = make_app();
        let pattern_idx = app.core.song.order[0];
        // Set effect at row 0 and row 2 (same effect command)
        app.core.song.patterns[pattern_idx].set_cell(
            0,
            0,
            rtrack_core::tracker::Cell {
                fx: [Some(Fx::new(FxCmd::VolSlide, 0)), None],
                ..Default::default()
            },
        );
        app.core.song.patterns[pattern_idx].set_cell(
            2,
            0,
            rtrack_core::tracker::Cell {
                fx: [Some(Fx::new(FxCmd::VolSlide, 80)), None],
                ..Default::default()
            },
        );
        app.block_anchor = Some((0, 0));
        app.cursor_row = 2;
        app.cursor_channel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert_eq!(
            app.core.song.patterns[pattern_idx].get(1, 0).fx[0],
            Some(Fx::new(FxCmd::VolSlide, 40))
        );
    }

    #[test]
    fn test_interpolate_no_block() {
        let mut app = make_app();
        // No block selected -- should show error message, not panic
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert!(app.status_message.as_ref().unwrap().contains("No block"));
    }

    #[test]
    fn test_resolve_relative_blocks_traversal() {
        let base = std::path::Path::new("/home/user/songs");
        // Normal relative path
        let normal = resolve_relative(base, "samples/kick.wav").expect("plain relative path");
        assert_eq!(
            normal,
            std::path::PathBuf::from("/home/user/songs/samples/kick.wav")
        );
        // Path traversal -- rejected, not silently rewritten into a path
        // under the base that may name a real, unrelated file.
        assert!(resolve_relative(base, "../../etc/passwd").is_err());
        // An absolute path is what `make_relative` writes for a sample from a
        // library outside the song directory, so it resolves unchanged.
        let absolute = resolve_relative(base, "/srv/samples/kick.wav").expect("absolute path");
        assert_eq!(absolute, std::path::PathBuf::from("/srv/samples/kick.wav"));
    }

    /// Helper: enter command mode and execute a command via :cmd<Enter>
    fn run_command(app: &mut App, cmd: &str) {
        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Command);
        for c in cmd.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_command_mode_open_close() {
        let mut app = make_app();
        // ':' enters command mode
        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Command);

        // Esc cancels
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_command_mode_unknown() {
        let mut app = make_app();
        run_command(&mut app, "nonsense");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app
            .status_message
            .as_ref()
            .unwrap()
            .contains("Unknown command"));
    }

    #[test]
    fn test_command_mode_backspace_cancels() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
        // Type a char then backspace twice (second empties buffer -> cancel)
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Command); // still in command mode, buffer empty
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal); // cancelled
    }

    #[test]
    fn test_pattern_matrix_open_close() {
        let mut app = make_app();
        assert_eq!(app.mode, Mode::Normal);

        // :p opens pattern matrix
        run_command(&mut app, "p");
        assert_eq!(app.mode, Mode::PatternMatrix);
        assert_eq!(app.matrix_cursor, 0);

        // Esc returns to Normal
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_pattern_matrix_navigation() {
        let mut app = make_app();
        app.core.song.order.push(0);
        app.core.song.order.push(0);

        run_command(&mut app, "p");
        assert_eq!(app.mode, Mode::PatternMatrix);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 2);

        // Can't go past end
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 2);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 2);

        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 0);
    }

    #[test]
    fn test_pattern_matrix_enter_jumps() {
        let mut app = make_app();
        app.core.song.order.push(0);
        app.core.song.order.push(0);

        run_command(&mut app, "p");
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.matrix_cursor, 2);

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.edit_order, 2);
        assert_eq!(app.cursor_row, 0);
    }

    #[test]
    fn test_pattern_matrix_insert_delete() {
        let mut app = make_app();
        assert_eq!(app.core.song.order.len(), 1);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE));
        assert_eq!(app.core.song.order.len(), 2);
        assert_eq!(app.core.song.order[0], 0);
        assert_eq!(app.core.song.order[1], 0);
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.core.song.order.len(), 1);
        assert_eq!(app.matrix_cursor, 0);

        // Can't delete the last entry
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.core.song.order.len(), 1);
    }

    #[test]
    fn test_pattern_matrix_new_clone() {
        let mut app = make_app();
        assert_eq!(app.core.song.patterns.len(), 1);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.core.song.patterns.len(), 2);
        assert_eq!(app.core.song.order.len(), 2);
        assert_eq!(app.core.song.order[1], 1);
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.core.song.patterns.len(), 3);
        assert_eq!(app.core.song.order.len(), 3);
        assert_eq!(app.matrix_cursor, 2);
    }

    #[test]
    fn test_pattern_matrix_change_pattern() {
        let mut app = make_app();
        app.core.song.add_pattern();
        assert_eq!(app.core.song.order[0], 0);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.core.song.order[0], 1);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.core.song.order[0], 0);

        // Can't go below 0
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.core.song.order[0], 0);
    }

    #[test]
    fn test_pattern_matrix_repeat() {
        let mut app = make_app();
        run_command(&mut app, "p");

        // Default repeat is 1
        assert_eq!(app.core.song.order_repeats[0], 1);

        // ] increases repeat
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 2);

        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 3);

        // [ decreases repeat
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 2);

        // Can go to 0 (skip)
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 0);

        // Can't go below 0
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 0);
    }

    #[test]
    fn test_order_repeats_sync_on_insert_delete() {
        let mut app = make_app();
        run_command(&mut app, "p");

        // Set repeat to 3
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats[0], 3);

        // Insert: new entry gets repeat=1
        app.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats.len(), 2);
        assert_eq!(app.core.song.order_repeats[0], 3); // original preserved
        assert_eq!(app.core.song.order_repeats[1], 1); // new entry

        // Delete: removes the entry's repeat too
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.core.song.order_repeats.len(), 1);
        assert_eq!(app.core.song.order_repeats[0], 3); // original still there
    }

    #[test]
    fn test_track_config_via_command() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        assert_eq!(app.mode, Mode::TrackConfig);
        assert_eq!(app.ch_fx_field, 0);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_track_config_toggle_filter() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        // Navigate to filter enabled (field 2: Name, Inst, Filter)
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Instrument
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 2=Filter
        assert_eq!(app.ch_fx_field, 2);
        assert!(!app.core.channels[0].effects_params.filter_enabled);
        // Toggle with Right arrow
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.core.channels[0].effects_params.filter_enabled);
        // Toggle back
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(!app.core.channels[0].effects_params.filter_enabled);
    }

    #[test]
    fn test_track_config_adjust_cutoff() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        // Navigate to cutoff (field 3: Name, Inst, Filter, Cutoff)
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.ch_fx_field, 3);
        let initial = app.core.channels[0].effects_params.filter_cutoff;
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.core.channels[0].effects_params.filter_cutoff > initial);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!((app.core.channels[0].effects_params.filter_cutoff - initial).abs() < 0.01);
    }

    #[test]
    fn test_track_config_navigate_all_fields() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        // 19 fields total (name, instrument, filter x3, distortion x2, chorus x4, delay x4, reverb x4)
        for i in 0..19 {
            assert_eq!(app.ch_fx_field, i);
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.ch_fx_field, 0);
    }

    #[test]
    fn test_track_config_instrument_steps_back_to_none() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Instrument
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].default_instrument, Some(0));
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].default_instrument, None);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].default_instrument, None);
    }

    #[test]
    fn test_track_config_instrument_select() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        // Navigate to Instrument field (field 1)
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Instrument
        assert_eq!(app.ch_fx_field, 1);
        assert_eq!(app.core.channels[0].default_instrument, None);
        // Right sets to 00
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].default_instrument, Some(0));
        // Right again increments
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.core.channels[0].default_instrument, Some(1));
    }

    #[test]
    fn test_synth_track_auto_fills_instrument() {
        let mut app = make_app();
        app.core.channels[0].default_instrument = Some(5);
        app.mode = Mode::Insert;
        // Enter a note (z = C in current octave)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(5));
    }

    #[test]
    fn test_note_entry_inherits_sample_instrument_from_column_above() {
        // The sliced-sample case: each slice is its own instrument and the
        // track has no default instrument. A note typed on an empty row used to fall through
        // to the built-in synth instead of playing the slice.
        let mut app = make_app();
        app.core.instruments[3].kind = rtrack_core::InstrumentKind::sample(3);
        let order = app.current_order_position();
        app.core.song.set_cell(
            order,
            0,
            0,
            rtrack_core::tracker::Cell {
                note: Some(Note::On {
                    value: rtrack_core::tracker::NoteValue::C,
                    octave: 5,
                }),
                instrument: Some(3),
                ..Default::default()
            },
        );

        app.cursor_row = 4;
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));

        let cell = app.core.song.cell_at(order, 4, 0);
        assert!(cell.note.is_some());
        assert_eq!(
            cell.instrument,
            Some(3),
            "the new note should sound like the sample above it"
        );
    }

    #[test]
    fn test_sample_track_auto_fills_instrument() {
        let mut app = make_app();
        app.core.channels[0].default_instrument = Some(3);
        app.mode = Mode::Insert;
        // Enter a note (z = C in current octave)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let pattern_idx = app.core.song.order[0];
        let cell = app.core.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(3));
    }

    #[test]
    fn test_ctrl_o_opens_the_song_browser() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::FileBrowser);
        assert_eq!(app.dialogs.file_browser.action, FileBrowserAction::OpenSong);
        assert_eq!(app.dialogs.file_browser.filter, vec!["rtrk"]);
    }

    #[test]
    fn test_ctrl_o_works_from_insert_mode_and_returns_there() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::FileBrowser);
        // Cancelling returns to where the user was, rather than dumping them
        // in Normal mode mid-entry.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert);
    }

    #[test]
    fn test_open_command_and_ctrl_o_reach_the_same_place() {
        let mut by_key = make_app();
        by_key.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

        let mut by_command = make_app();
        for c in ":open".chars() {
            by_command.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        by_command.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(by_key.mode, by_command.mode);
        assert_eq!(
            by_key.dialogs.file_browser.action,
            by_command.dialogs.file_browser.action
        );
        assert_eq!(
            by_key.dialogs.file_browser.filter,
            by_command.dialogs.file_browser.filter
        );
    }

    #[test]
    fn test_file_browser_open() {
        let mut app = make_app();
        app.open_file_browser(
            FileBrowserAction::LoadSample(0),
            vec!["wav".to_string(), "aiff".to_string()],
        );
        assert_eq!(app.mode, Mode::FileBrowser);
        assert_eq!(
            app.dialogs.file_browser.action,
            FileBrowserAction::LoadSample(0)
        );
        assert_eq!(app.dialogs.file_browser.filter, vec!["wav", "aiff"]);
        assert_eq!(app.dialogs.file_browser.cursor, 0);
    }

    #[test]
    fn test_file_browser_navigate() {
        let mut app = make_app();
        // Use temp dir which should exist
        app.dialogs.file_browser.dir = std::env::temp_dir();
        app.open_file_browser(FileBrowserAction::OpenSong, vec![]);
        // Navigate down
        let initial = app.dialogs.file_browser.cursor;
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        if !app.dialogs.file_browser.entries.is_empty() {
            assert!(app.dialogs.file_browser.cursor >= initial);
        }
        // Esc closes
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_ne!(app.mode, Mode::FileBrowser);
    }

    #[test]
    fn test_file_browser_enter_directory() {
        let mut app = make_app();
        let dir = std::env::temp_dir().join("rtrack_fb_test_dir");
        let sub = dir.join("subdir");
        let _ = std::fs::create_dir_all(&sub);

        app.dialogs.file_browser.dir = dir.clone();
        app.open_file_browser(FileBrowserAction::OpenSong, vec![]);

        // Find the subdir entry
        let subdir_idx = app
            .dialogs
            .file_browser
            .entries
            .iter()
            .position(|e| e.name == "subdir" && e.is_dir);
        if let Some(idx) = subdir_idx {
            app.dialogs.file_browser.cursor = idx;
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            // Should have navigated into subdir
            assert_eq!(app.dialogs.file_browser.dir, sub);
            assert_eq!(app.mode, Mode::FileBrowser);
        }

        // Backspace goes up
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.dialogs.file_browser.dir, dir);
    }

    #[test]
    fn test_file_browser_filter() {
        let mut app = make_app();
        let dir = std::env::temp_dir().join("rtrack_fb_filter_test");
        let _ = std::fs::create_dir_all(&dir);
        // Create files with different extensions
        std::fs::write(dir.join("sample.wav"), b"fake").unwrap();
        std::fs::write(dir.join("notes.txt"), b"text").unwrap();
        std::fs::write(dir.join("beat.aiff"), b"fake").unwrap();

        app.dialogs.file_browser.dir = dir.clone();
        app.open_file_browser(
            FileBrowserAction::LoadSample(0),
            vec!["wav".to_string(), "aiff".to_string()],
        );

        // Should only show wav and aiff files (not txt)
        let names: Vec<&str> = app
            .dialogs
            .file_browser
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"sample.wav"));
        assert!(names.contains(&"beat.aiff"));
        assert!(!names.contains(&"notes.txt"));
    }

    #[test]
    fn test_file_browser_select_file_loads_sample() {
        let mut app = make_app();
        // Ensure we have enough channels for slot 5
        while app.core.channels.len() <= 5 {
            app.core
                .channels
                .push(ChannelConfig::new(app.core.channels.len() as u8));
        }
        let dir = std::env::temp_dir().join("rtrack_fb_load_test");
        let _ = std::fs::create_dir_all(&dir);

        // Create a valid WAV file
        let path = dir.join("kick.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..100 {
            writer.write_sample((i * 100) as i16).unwrap();
        }
        writer.finalize().unwrap();

        app.dialogs.file_browser.dir = dir.clone();
        app.open_file_browser(FileBrowserAction::LoadSample(5), vec!["wav".to_string()]);

        // Find the wav file and select it
        let wav_idx = app
            .dialogs
            .file_browser
            .entries
            .iter()
            .position(|e| e.name == "kick.wav");
        assert!(wav_idx.is_some(), "WAV file should appear in browser");
        app.dialogs.file_browser.cursor = wav_idx.unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // Should have loaded the sample
        assert!(app.core.sample_bank.get(5).is_some());
        assert_eq!(app.core.sample_bank.get(5).unwrap().name, "kick");
        assert_eq!(app.core.instruments[5].sample_index(), Some(5));
        // Loading a sample should auto-set default_instrument so preview routes correctly
        assert_eq!(app.core.channels[5].default_instrument, Some(5));
        assert_ne!(app.mode, Mode::FileBrowser);
    }

    #[test]
    fn test_file_browser_dirs_first() {
        let mut app = make_app();
        let dir = std::env::temp_dir().join("rtrack_fb_sort_test");
        let _ = std::fs::create_dir_all(dir.join("aaa_dir"));
        std::fs::write(dir.join("aaa_file.wav"), b"x").unwrap();

        app.dialogs.file_browser.dir = dir.clone();
        app.open_file_browser(FileBrowserAction::OpenSong, vec![]);

        // Directories should come before files
        if let Some(dir_idx) = app
            .dialogs
            .file_browser
            .entries
            .iter()
            .position(|e| e.name == "aaa_dir")
        {
            if let Some(file_idx) = app
                .dialogs
                .file_browser
                .entries
                .iter()
                .position(|e| e.name == "aaa_file.wav")
            {
                assert!(dir_idx < file_idx, "Directories should sort before files");
            }
        }
    }

    #[test]
    fn test_slice_sample_equal() {
        let mut app = make_app();
        // Load a sample into slot 0
        let sample = rtrack_core::sample::Sample {
            name: "kick".into(),
            data: vec![[0.5, 0.5]; 4000].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let mut bank = (*app.core.sample_bank).clone();
        bank.samples[0] = Some(Arc::new(sample));
        app.core.sample_bank = Arc::new(bank);
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_slice_count = 4;

        let result = app.slice_sample(false);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4);

        // Check that slices are in consecutive slots
        assert!(app.core.sample_bank.get(0).is_some());
        assert!(app.core.sample_bank.get(1).is_some());
        assert!(app.core.sample_bank.get(2).is_some());
        assert!(app.core.sample_bank.get(3).is_some());
        // Each slice views a quarter of the shared source buffer.
        let first = app.core.sample_bank.get(0).unwrap();
        assert_eq!(first.played_len(), 1000);
        assert_eq!(first.trim_start, 0);
        assert_eq!(first.name, "kick_S00");
        let last = app.core.sample_bank.get(3).unwrap();
        assert_eq!(last.played_len(), 1000);
        assert_eq!(last.trim_start, 3000);
        assert_eq!(last.name, "kick_S03");

        // Check instruments are set up
        assert_eq!(app.core.instruments[0].sample_index(), Some(0));
        assert_eq!(app.core.instruments[3].sample_index(), Some(3));
    }

    #[test]
    fn test_slice_sample_no_sample() {
        let mut app = make_app();
        app.dialogs.sample_editor_slot = 5;
        let result = app.slice_sample(false);
        assert!(result.is_err());
    }

    #[test]
    fn test_slice_sample_transient() {
        let mut app = make_app();
        // Build sample with silence + burst pattern
        let mut data = vec![[0.0f32; 2]; 44100];
        for frame in &mut data[11025..13000] {
            *frame = [0.8, 0.8];
        }
        for frame in &mut data[26460..28000] {
            *frame = [0.9, 0.9];
        }
        let sample = rtrack_core::sample::Sample {
            name: "breaks".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let mut bank = (*app.core.sample_bank).clone();
        bank.samples[0] = Some(Arc::new(sample));
        app.core.sample_bank = Arc::new(bank);
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_slice_sensitivity = 0.5;

        let result = app.slice_sample(true);
        assert!(result.is_ok());
        let count = result.unwrap();
        assert!(
            count >= 2,
            "Expected at least 2 transient slices, got {}",
            count
        );

        // All slices should exist in consecutive slots
        for i in 0..count {
            assert!(app.core.sample_bank.get(i).is_some(), "Slice {} missing", i);
        }
    }

    // -- Auto-save tests --

    #[test]
    fn test_autosave_only_when_dirty() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("test_autosave_dirty.rtrk");
        app.core.file_path = Some(tmp.clone());
        app.core.dirty = false;
        // Force elapsed time
        app.last_autosave = Instant::now() - std::time::Duration::from_secs(120);
        app.auto_save();
        let autosave = autosave_path_for(&tmp);
        assert!(!autosave.exists(), "Should not auto-save when not dirty");
    }

    #[test]
    fn test_autosave_creates_file() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("test_autosave_creates.rtrk");
        app.core.file_path = Some(tmp.clone());
        app.core.dirty = true;
        app.last_autosave = Instant::now() - std::time::Duration::from_secs(120);
        app.auto_save();
        let autosave = autosave_path_for(&tmp);
        assert!(autosave.exists(), "Auto-save file should exist");
        let _ = std::fs::remove_file(&autosave);
    }

    #[test]
    fn test_autosave_cleanup_on_save() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("test_autosave_cleanup.rtrk");
        app.core.file_path = Some(tmp.clone());
        app.core.dirty = true;
        app.last_autosave = Instant::now() - std::time::Duration::from_secs(120);
        app.auto_save();
        let autosave = autosave_path_for(&tmp);
        assert!(autosave.exists());

        // Manual save should clean up autosave
        app.save();
        assert!(
            !autosave.exists(),
            "Auto-save should be cleaned up after manual save"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    // -- Row highlight tests --

    #[test]
    fn test_highlight_defaults() {
        let song = Song::new(4, 64);
        assert_eq!(song.highlight_beat, 4);
        assert_eq!(song.highlight_bar, 16);
    }

    #[test]
    fn test_highlight_settings() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::HighlightBeat;
        app.dialogs.settings_edit_buf = "3".to_string();
        app.settings_apply_field();
        assert_eq!(app.core.song.highlight_beat, 3);

        app.dialogs.settings_field = SettingsField::HighlightBar;
        app.dialogs.settings_edit_buf = "12".to_string();
        app.settings_apply_field();
        assert_eq!(app.core.song.highlight_bar, 12);
    }

    // -- Groove tests --

    #[test]
    fn a_new_song_is_straight() {
        let song = Song::new(4, 64);
        assert!(song.groove.is_empty());
        assert_eq!(song.groove_factor(0), 1.0);
        assert_eq!(song.groove_factor(1), 1.0);
    }

    #[test]
    fn a_groove_cycle_lasts_as_long_as_straight_rows() {
        let mut song = Song::new(4, 64);
        song.groove = vec![55, 45, 52, 48];
        let cycle: f64 = (0..4).map(|r| song.groove_factor(r)).sum();
        assert!((cycle - 4.0).abs() < 1e-12);
        assert!(song.groove_factor(0) > 1.0 && song.groove_factor(1) < 1.0);
        assert_eq!(song.groove_factor(4), song.groove_factor(0), "it repeats");
    }

    #[test]
    fn test_groove_settings() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Groove;
        let set = |app: &mut App, text: &str| {
            app.dialogs.settings_edit_buf = text.to_string();
            app.settings_apply_field();
        };
        set(&mut app, "67");
        assert_eq!(app.core.song.groove, vec![67, 33], "one number is swing");
        set(&mut app, "55 45 52 48");
        assert_eq!(app.core.song.groove, vec![55, 45, 52, 48]);
        set(&mut app, "150");
        assert_eq!(app.core.song.groove, vec![99, 1], "swing keeps both rows");
        set(&mut app, "3 0");
        assert!(app
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("weight")));
        assert_eq!(
            app.core.song.groove,
            vec![99, 1],
            "a bad groove changes nothing"
        );
        set(&mut app, "off");
        assert!(app.core.song.groove.is_empty());
    }

    // -- Tempo automation tests --

    #[test]
    fn test_tempo_map_lookup() {
        let mut song = Song::new(4, 64);
        song.tempo_map.push(rtrack_core::tracker::TempoPoint {
            order: 0,
            row: 16,
            bpm: 140.0,
        });
        song.tempo_map.push(rtrack_core::tracker::TempoPoint {
            order: 1,
            row: 0,
            bpm: 160.0,
        });

        assert_eq!(song.tempo_at(0, 0), None);
        assert_eq!(song.tempo_at(0, 16), Some(140.0));
        assert_eq!(song.tempo_at(1, 0), Some(160.0));
        assert_eq!(song.tempo_at(1, 1), None);
    }

    #[test]
    fn test_tempo_map_serialization() {
        let mut song = Song::new(4, 16);
        song.tempo_map.push(rtrack_core::tracker::TempoPoint {
            order: 0,
            row: 8,
            bpm: 150.5,
        });
        let json = serde_json::to_string(&song).unwrap();
        let loaded: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.tempo_map.len(), 1);
        assert_eq!(loaded.tempo_map[0].order, 0);
        assert_eq!(loaded.tempo_map[0].row, 8);
        assert!((loaded.tempo_map[0].bpm - 150.5).abs() < 1e-9);
    }

    // -- Pitch bend range tests --

    #[test]
    fn test_pitch_bend_range_default() {
        let app = make_app();
        // Default: no instrument active -> use default range
        let pb = app.core.channel_pitch_bend_per_semitone(0);
        let expected = (PITCH_BEND_CENTER as f64) / DEFAULT_PITCH_BEND_RANGE;
        assert!((pb - expected).abs() < 1e-9);
    }

    #[test]
    fn test_pitch_bend_range_custom() {
        let mut app = make_app();
        // Set instrument 0 with custom pitch bend range of 12 semitones
        app.core.instruments[0].pitch_bend_range = Some(12.0);
        app.core.engine.channel_states[0].active_instrument = Some(0);

        let pb = app.core.channel_pitch_bend_per_semitone(0);
        let expected = (PITCH_BEND_CENTER as f64) / 12.0;
        assert!((pb - expected).abs() < 1e-9);
    }

    #[test]
    fn test_pitch_bend_range_serialization() {
        use rtrack_core::tracker::{InstrumentDef, InstrumentEntry, SongFile};
        let song = Song::new(1, 16);
        let song_file = SongFile {
            instruments: vec![InstrumentEntry {
                slot: 0,
                def: InstrumentDef::from_instrument(&rtrack_core::Instrument {
                    name: "Test".to_string(),
                    pitch_bend_range: Some(7.0),
                    ..rtrack_core::Instrument::for_slot(0)
                }),
            }],
            ..SongFile::from_song(song)
        };
        let json = serde_json::to_string(&song_file).unwrap();
        let loaded: SongFile = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.instruments[0].def.pitch_bend_range, Some(7.0));
    }

    // -- Link beat timeline test --

    #[test]
    fn test_link_beat_at_time() {
        let mut engine = rtrack_core::link::LinkEngine::new(120.0);
        engine.enable();
        let beat = engine.beat_at_time_now();
        // Just verify it returns a number (exact value depends on timing)
        assert!(beat.is_finite());
        engine.disable();
    }

    // -- Backwards compatibility tests --

    #[test]
    fn test_song_backwards_compat_new_fields() {
        // Old format without new fields should load with defaults
        let json = r#"{
            "title": "Old",
            "bpm": 120,
            "speed": 6,
            "patterns": [{"rows": 16, "channels": 4, "data": []}],
            "order": [0],
            "channels": 4,
            "rows_per_pattern": 16
        }"#;
        let song: Song = serde_json::from_str(json).unwrap();
        assert_eq!(song.highlight_beat, 4);
        assert_eq!(song.highlight_bar, 16);
        assert!(song.groove.is_empty());
        assert!(song.tempo_map.is_empty());
    }

    #[test]
    fn test_instrument_def_backwards_compat() {
        // Old format without pitch_bend_range
        let json = r#"{"name": "Test", "sample_index": 0}"#;
        let def: rtrack_core::tracker::InstrumentDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.pitch_bend_range, None);
    }

    #[test]
    fn test_track_config_enter_on_instrument_opens_the_file_browser() {
        let mut app = make_app();
        run_command(&mut app, "fx");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Instrument
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::FileBrowser);
    }

    #[test]
    fn test_sample_bank_loaded_slots() {
        let mut bank = rtrack_core::sample::SampleBank::new();
        assert!(bank.loaded_slots().is_empty());

        bank.samples[3] = Some(Arc::new(rtrack_core::sample::Sample {
            name: "test".to_string(),
            data: vec![[0.0; 2]; 10].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }));
        bank.samples[7] = Some(Arc::new(rtrack_core::sample::Sample {
            name: "test2".to_string(),
            data: vec![[0.0; 2]; 10].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }));

        let slots = bank.loaded_slots();
        assert_eq!(slots, vec![3, 7]);
    }

    #[test]
    fn test_recent_command_opens_popup() {
        let mut app = make_app();
        app.recent_files = vec![PathBuf::from("/tmp/a.rtrk"), PathBuf::from("/tmp/b.rtrk")];
        run_command(&mut app, "recent");
        assert_eq!(app.mode, Mode::RecentFiles);
        assert_eq!(app.dialogs.recent_cursor, 0);
    }

    #[test]
    fn test_recent_command_empty_shows_message() {
        let mut app = make_app();
        app.recent_files = Vec::new();
        run_command(&mut app, "recent");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status_message.as_ref().unwrap().contains("No recent"));
    }

    #[test]
    fn test_recent_files_navigate() {
        let mut app = make_app();
        app.recent_files = vec![
            PathBuf::from("/tmp/a.rtrk"),
            PathBuf::from("/tmp/b.rtrk"),
            PathBuf::from("/tmp/c.rtrk"),
        ];
        run_command(&mut app, "recent");
        assert_eq!(app.dialogs.recent_cursor, 0);

        // Down arrow
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.recent_cursor, 1);

        // Down again
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.recent_cursor, 2);

        // Down at end -- stays at 2
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.dialogs.recent_cursor, 2);

        // Up
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.dialogs.recent_cursor, 1);

        // Esc closes
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }
}
