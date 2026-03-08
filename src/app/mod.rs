mod playback;
mod input;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::AudioEngine;
use crate::constants::*;
use crate::link::LinkEngine;
use crate::midi::{MidiEngine, MidiInputEngine};
use crate::sample::SampleBank;
use crate::tracker::{Song, SongFile, InstrumentDef, InstrumentEntry, SampleRef, SampleRefEntry};
use crate::ui::pattern_editor::SubColumn;

// -- Constants (module-private; shared ones live in crate::constants) --

/// Auto-save interval in seconds
const AUTOSAVE_INTERVAL_SECS: u64 = 60;
/// Maximum undo history depth
const MAX_UNDO_HISTORY: usize = 100;
/// Preview note auto-off timeout in milliseconds
const PREVIEW_NOTE_TIMEOUT_MS: u64 = 250;

/// Timing accumulators for playback (internal to the playback loop).
pub(crate) struct PlaybackTiming {
    pub last_tick: Option<Instant>,
    pub tick_accumulator: f64,
    pub clock_tick_accumulator: f64,
    pub playback_elapsed: f64,
    pub ext_clock_count: u32,
    pub last_link_beat: f64,
}

impl PlaybackTiming {
    fn new() -> Self {
        Self {
            last_tick: None,
            tick_accumulator: 0.0,
            clock_tick_accumulator: 0.0,
            playback_elapsed: 0.0,
            ext_clock_count: 0,
            last_link_beat: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.last_tick = None;
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;
        self.playback_elapsed = 0.0;
        self.ext_clock_count = 0;
        self.last_link_beat = 0.0;
    }
}

/// Dialog and popup state for all modal UIs.
pub struct DialogState {
    pub settings_field: SettingsField,
    pub settings_edit_buf: String,
    pub instrument_cursor: usize,
    pub sample_editor_slot: usize,
    pub sample_editor_field: SampleField,
    pub sample_slice_count: usize,
    pub sample_slice_sensitivity: f32,
    pub synth_editor_slot: usize,
    pub synth_editor_field: SynthField,
    pub midi_port_list: Vec<String>,
    pub midi_port_cursor: usize,
    pub help_scroll: usize,
    pub file_browser: FileBrowserState,
}

impl DialogState {
    fn new() -> Self {
        Self {
            settings_field: SettingsField::Title,
            settings_edit_buf: String::new(),
            instrument_cursor: 0,
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            sample_slice_count: 8,
            sample_slice_sensitivity: 0.5,
            synth_editor_slot: 0,
            synth_editor_field: SynthField::Waveform,
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            help_scroll: 0,
            file_browser: FileBrowserState::new(),
        }
    }
}

/// Undo/redo and clipboard state.
pub struct EditHistory {
    pub undo_stack: VecDeque<Song>,
    pub redo_stack: Vec<Song>,
    pub clipboard: Option<Vec<crate::tracker::Cell>>,
    pub block_clipboard: Option<Vec<Vec<crate::tracker::Cell>>>,
    pub block_anchor: Option<(usize, usize)>,
}

impl EditHistory {
    fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            block_clipboard: None,
            block_anchor: None,
        }
    }
}

/// Re-export ChannelState from the engine module.
pub use crate::engine::ChannelState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    Midi,
    Synth,
    Sample,
}

impl ChannelType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Midi => "[MID]",
            Self::Synth => "[SYN]",
            Self::Sample => "[SMP]",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Midi => Self::Synth,
            Self::Synth => Self::Sample,
            Self::Sample => Self::Midi,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Midi => Self::Sample,
            Self::Synth => Self::Midi,
            Self::Sample => Self::Synth,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockMode {
    /// rtrack is the clock master (internal timing)
    Internal,
    /// rtrack slaves to external MIDI clock
    ExternalMidi,
}

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
                    entries.push(FileBrowserEntry { name, is_dir: false });
                } else {
                    let ext = std::path::Path::new(&name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if self.filter.iter().any(|f| f == &ext) {
                        entries.push(FileBrowserEntry { name, is_dir: false });
                    }
                }
            }
        }

        entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
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

/// Per-channel configuration (audio routing, effects, naming).
/// One entry per tracker channel; the Vec always matches `song.channels` in length.
pub struct ChannelConfig {
    pub muted: bool,
    pub name: String,
    pub channel_type: ChannelType,
    /// Default instrument for this track (Synth tracks auto-fill on note entry)
    pub default_instrument: Option<u8>,
    pub volume: f32,
    pub pan: f32,
    pub effects_params: crate::audio::channel_effects::ChannelEffectsParams,
    /// MIDI channel this tracker channel maps to (0-15)
    pub midi_channel: u8,
}

impl ChannelConfig {
    pub fn new(midi_channel: u8) -> Self {
        Self {
            muted: false,
            name: String::new(),
            channel_type: ChannelType::Midi,
            default_instrument: None,
            volume: 1.0,
            pan: 0.0,
            effects_params: crate::audio::channel_effects::ChannelEffectsParams::default(),
            midi_channel,
        }
    }
}

impl App {
    /// Create default channel configs for `n` channels.
    pub fn default_channel_configs(n: usize) -> Vec<ChannelConfig> {
        (0..n).map(|i| ChannelConfig::new(i as u8)).collect()
    }

    /// Collect a Vec of ChannelEffectsParams refs for passing to export functions.
    pub fn channel_effects_params_slice(&self) -> Vec<crate::audio::channel_effects::ChannelEffectsParams> {
        self.channels.iter().map(|c| c.effects_params.clone()).collect()
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
    Swing,
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
            Self::HighlightBar => Self::Swing,
            Self::Swing => Self::Title,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Title => Self::Swing,
            Self::Bpm => Self::Title,
            Self::Speed => Self::Bpm,
            Self::Channels => Self::Speed,
            Self::Rows => Self::Channels,
            Self::HighlightBeat => Self::Rows,
            Self::HighlightBar => Self::HighlightBeat,
            Self::Swing => Self::HighlightBar,
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
    SliceCount,
    SliceSensitivity,
    SliceEqual,
    SliceTransient,
}

impl SampleField {
    pub fn next(self) -> Self {
        match self {
            Self::BaseNote => Self::TrimStart,
            Self::TrimStart => Self::TrimEnd,
            Self::TrimEnd => Self::LoopEnabled,
            Self::LoopEnabled => Self::LoopStart,
            Self::LoopStart => Self::LoopEnd,
            Self::LoopEnd => Self::SliceCount,
            Self::SliceCount => Self::SliceSensitivity,
            Self::SliceSensitivity => Self::SliceEqual,
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
            Self::SliceCount => Self::LoopEnd,
            Self::SliceSensitivity => Self::SliceCount,
            Self::SliceEqual => Self::SliceSensitivity,
            Self::SliceTransient => Self::SliceEqual,
        }
    }
}

pub struct Instrument {
    pub name: String,
    pub midi_program: Option<u8>,
    pub sample_index: Option<usize>,
    pub synth_params: Option<crate::audio::synth::SynthParams>,
    /// Pitch bend range in semitones (None = use default of 2)
    pub pitch_bend_range: Option<f64>,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            name: String::new(),
            midi_program: None,
            sample_index: None,
            synth_params: None,
            pitch_bend_range: None,
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
            Self::Release => Self::FilterType,
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
            Self::FilterType => Self::Release,
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
    // Core / top-level state
    // -----------------------------------------------------------------------
    pub song: Song,
    pub midi: MidiEngine,
    pub midi_input: MidiInputEngine,
    pub link: LinkEngine,
    pub mode: Mode,
    pub should_quit: bool,

    // -----------------------------------------------------------------------
    // Cursor State
    // Fields: cursor_row, cursor_channel, cursor_sub, current_octave,
    //         track_page, follow_playback, edit_step
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

    // -----------------------------------------------------------------------
    // Playback State
    // -----------------------------------------------------------------------
    pub playing: bool,
    /// Punch-in recording: when true + playing + Insert mode, incoming MIDI writes to pattern
    pub recording: bool,
    /// The deterministic playback engine (owns row/order/generation/tick/channel_states).
    pub engine: crate::engine::TrackerEngine,
    /// Timing accumulators (internal to playback loop)
    pub(crate) timing: PlaybackTiming,
    /// External MIDI clock mode
    pub clock_mode: ClockMode,

    // -----------------------------------------------------------------------
    // Editor State
    // -----------------------------------------------------------------------
    /// Dirty flag: set when song is modified, cleared on save/load
    pub dirty: bool,
    /// Undo/redo and clipboard state
    pub history: EditHistory,
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
    // Other state (file, audio, instruments, channels)
    // -----------------------------------------------------------------------
    pub file_path: Option<PathBuf>,
    pub status_message: Option<String>,
    /// Last auto-save timestamp
    pub(crate) last_autosave: Instant,
    pub edit_order: usize,
    pub solo_channel: Option<usize>,
    /// The mode to return to after closing the port selector
    pub(crate) prev_mode: Mode,
    pub instruments: Vec<Instrument>,
    pub theme_index: usize,
    pub audio: Option<AudioEngine>,
    pub sample_bank: Arc<SampleBank>,
    /// Preview note: (channel, note, timestamp) -- auto note-off after timeout
    pub(crate) preview_note: Option<(u8, u8, Instant)>,
    /// Per-channel config (routing, effects, naming) -- always matches song.channels in length
    pub channels: Vec<ChannelConfig>,
    /// Send/return bus parameters
    pub send_bus_params: Vec<crate::audio::effects::SendBusParams>,
}

impl App {
    pub fn new() -> Self {
        let mut midi = MidiEngine::new();
        // Create a virtual MIDI port "RTRACK_MIDI" that DAWs can connect to.
        // Falls back to connecting to the first available port if virtual ports
        // are not supported (Windows).
        if midi.create_virtual_port().is_err() {
            let _ = midi.connect_first_available();
        }

        let mut midi_input = MidiInputEngine::new();
        // Try to create a virtual MIDI input port
        let _ = midi_input.create_virtual_port();

        let song = Song::new(4, 64);
        let link = LinkEngine::new(song.bpm as f64);
        let engine = crate::engine::TrackerEngine::new(&song, true);

        Self {
            song,
            midi,
            midi_input,
            link,
            mode: Mode::Normal,
            should_quit: false,
            cursor_row: 0,
            cursor_channel: 0,
            cursor_sub: SubColumn::Note,
            current_octave: 4,
            playing: false,
            recording: false,
            engine,
            timing: PlaybackTiming::new(),
            edit_step: 1,
            file_path: None,
            status_message: None,
            last_autosave: Instant::now(),
            history: EditHistory::new(),
            edit_order: 0,
            solo_channel: None,
            prev_mode: Mode::Normal,
            instruments: (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect(),
            theme_index: 0,
            clock_mode: ClockMode::Internal,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            track_page: 0,
            preview_note: None,
            dirty: false,
            follow_playback: true,
            rename_buf: String::new(),
            ch_fx_field: 0,
            matrix_cursor: 0,
            command_buf: String::new(),
            dialogs: DialogState::new(),
            channels: Self::default_channel_configs(4),
            send_bus_params: (0..crate::audio::effects::MAX_SEND_BUSES).map(|_| crate::audio::effects::SendBusParams::default()).collect(),
        }
    }

    // -- Accessors --

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn midi_connected(&self) -> bool {
        self.midi.is_connected()
    }

    pub fn midi_port_display_name(&self) -> &str {
        self.midi
            .port_name
            .as_deref()
            .unwrap_or("--")
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    pub fn has_sf2(&self) -> bool {
        self.audio.as_ref().map_or(false, |a| a.has_sf2())
    }

    pub fn audio_effects_enabled(&self) -> bool {
        self.audio.as_ref().map_or(false, |a| a.effects_enabled())
    }

    #[allow(dead_code)]
    pub fn toggle_audio_effects(&mut self) -> bool {
        self.audio.as_mut().map_or(false, |a| a.toggle_effects())
    }

    // -- Sample loading --

    /// Load a sample file into a bank slot and assign it to the instrument
    pub fn load_sample(&mut self, slot: usize, path: std::path::PathBuf) {
        // We need to mutate the bank, so make a mutable copy
        let mut bank = (*self.sample_bank).clone();
        match bank.load(slot, &path) {
            Ok(()) => {
                let name = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sample")
                    .to_string();
                if slot < self.instruments.len() {
                    self.instruments[slot].sample_index = Some(slot);
                    if self.instruments[slot].name.is_empty() {
                        self.instruments[slot].name = name.clone();
                    }
                }
                self.sample_bank = Arc::new(bank);
                // Push updated bank to audio engine
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }
                self.status_message = Some(format!("Loaded sample: {}", name));
            }
            Err(e) => {
                self.status_message = Some(format!("Sample load error: {}", e));
            }
        }
    }

    /// Load samples from a directory (files named <slot>-<name>.wav/.aiff)
    pub fn load_sample_directory(&mut self, dir: &std::path::Path) {
        let mut bank = (*self.sample_bank).clone();
        match bank.load_directory(dir) {
            Ok(meta) => {
                // Assign samples to instrument slots
                for (i, sample) in bank.samples.iter().enumerate() {
                    if sample.is_some() {
                        if i < self.instruments.len() {
                            self.instruments[i].sample_index = Some(i);
                            if self.instruments[i].name.is_empty() {
                                if let Some(ref s) = sample {
                                    self.instruments[i].name = s.name.clone();
                                }
                            }
                        }
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }
                // Apply BPM from metadata if provided
                if let Some(bpm) = meta.bpm {
                    self.song.bpm = bpm;
                    if self.link.is_enabled() {
                        self.link.set_tempo(bpm as f64);
                    }
                }
                self.status_message = Some(format!("Loaded samples from: {}", dir.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Sample dir error: {}", e));
            }
        }
    }

    /// Check if a given instrument slot has a sample assigned
    #[allow(dead_code)]
    pub fn instrument_has_sample(&self, inst: usize) -> bool {
        self.instruments.get(inst)
            .and_then(|i| i.sample_index)
            .and_then(|idx| self.sample_bank.get(idx))
            .is_some()
    }

    /// Open the synth editor for the current instrument
    pub fn open_synth_editor(&mut self) {
        let slot = self.dialogs.instrument_cursor;
        self.dialogs.synth_editor_slot = slot;
        self.dialogs.synth_editor_field = SynthField::Waveform;
        // Initialize synth params from defaults if not already set
        if self.instruments[slot].synth_params.is_none() {
            let program = self.instruments[slot].midi_program.unwrap_or(0);
            self.instruments[slot].synth_params = Some(crate::audio::synth::SynthParams::from_patch(program));
        }
        self.prev_mode = self.mode;
        self.mode = Mode::SynthEditor;
    }

    /// Open the sample editor for the current instrument
    pub fn open_sample_editor(&mut self) {
        self.dialogs.sample_editor_slot = self.dialogs.instrument_cursor;
        self.dialogs.sample_editor_field = SampleField::BaseNote;
        self.prev_mode = self.mode;
        self.mode = Mode::SampleEditor;
    }

    /// Open the file browser with a specific action and extension filter.
    pub fn open_file_browser(&mut self, action: FileBrowserAction, extensions: Vec<String>) {
        self.prev_mode = self.mode;
        self.dialogs.file_browser.open(action, extensions);
        self.mode = Mode::FileBrowser;
    }

    /// Handle file selection from the file browser.
    pub fn on_file_selected(&mut self, path: PathBuf) {
        match self.dialogs.file_browser.action {
            FileBrowserAction::LoadSample(slot) => {
                let mut bank = (*self.sample_bank).clone();
                match bank.load(slot, &path) {
                    Ok(()) => {
                        // Set up instrument to point to this sample
                        if slot < self.instruments.len() {
                            self.instruments[slot].sample_index = Some(slot);
                            if self.instruments[slot].name.is_empty() {
                                self.instruments[slot].name = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("sample")
                                    .to_string();
                            }
                        }
                        // Wire the channel's default instrument so note
                        // preview routes through the sample engine
                        if let Some(ch) = self.channels.get_mut(slot) {
                            if ch.default_instrument.is_none() {
                                ch.default_instrument = Some(slot as u8);
                            }
                        }
                        self.sample_bank = Arc::new(bank);
                        if let Some(ref mut audio) = self.audio {
                            audio.set_sample_bank(Arc::clone(&self.sample_bank));
                        }
                        self.status_message = Some(format!("Loaded sample into slot {:02X}", slot));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Failed to load: {}", e));
                    }
                }
            }
            FileBrowserAction::OpenSong => {
                self.load_file(path);
            }
        }
    }

    /// Slice the sample in the current editor slot and place results into consecutive slots.
    /// Returns the number of slices created, or an error message.
    pub fn slice_sample(&mut self, use_transients: bool) -> Result<usize, String> {
        let slot = self.dialogs.sample_editor_slot;
        let sample = match self.sample_bank.get(slot) {
            Some(s) => s.clone(),
            None => return Err("No sample loaded in this slot".to_string()),
        };

        let slices = if use_transients {
            let points = crate::sample::detect_transients(&sample, self.dialogs.sample_slice_sensitivity);
            crate::sample::slice_at_points(&sample, &points)
        } else {
            crate::sample::slice_equal(&sample, self.dialogs.sample_slice_count)
        };

        if slices.is_empty() {
            return Err("Sample too short to slice".to_string());
        }

        // Check that we have enough slots
        let end_slot = slot + slices.len();
        if end_slot > 256 {
            return Err(format!("Not enough sample slots (need {} from slot {:02X})", slices.len(), slot));
        }

        let mut bank = (*self.sample_bank).clone();
        for (i, s) in slices.iter().enumerate() {
            bank.samples[slot + i] = Some(s.clone());
        }
        let count = slices.len();
        self.sample_bank = Arc::new(bank);
        if let Some(ref mut audio) = self.audio {
            audio.set_sample_bank(Arc::clone(&self.sample_bank));
        }

        // Set up instruments to point to the new sample slots
        for i in 0..count {
            let inst_slot = slot + i;
            if inst_slot < self.instruments.len() {
                self.instruments[inst_slot].sample_index = Some(inst_slot);
                if self.instruments[inst_slot].name.is_empty() {
                    self.instruments[inst_slot].name = slices[i].name.clone();
                }
            }
        }

        Ok(count)
    }

    /// Build export instrument descriptors from the current instrument list.
    pub fn export_instruments(&self) -> Vec<crate::sample::export::ExportInstrument> {
        self.instruments.iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect()
    }

    /// Return the audio sample rate (from the audio engine, or 44100 as default).
    pub fn export_sample_rate(&self) -> u32 {
        self.audio.as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100)
    }

    /// Export the song to a WAV file
    #[allow(dead_code)]
    pub fn export_wav(&self, path: std::path::PathBuf) {
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        match crate::sample::export::render_to_wav(
            &path, &self.song, &self.sample_bank, &instruments, &self.channel_effects_params_slice(), &self.send_bus_params, sample_rate,
        ) {
            Ok(()) => {}
            Err(_e) => {}
        }
    }

    // -- Sound output helpers (dispatch to MIDI + optional audio engine) --

    pub(crate) fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref mut audio) = self.audio {
            audio.note_on(channel, note, velocity);
        }
    }

    /// Note-on with instrument awareness: routes to sample engine, custom synth params, or default
    pub(crate) fn send_note_on_with_instrument(&mut self, channel: u8, note: u8, velocity: u8, instrument: Option<u8>) {
        let inst_idx = instrument.unwrap_or(0) as usize;
        let inst = self.instruments.get(inst_idx);

        // Route 1: sample engine
        if let Some(sid) = inst.and_then(|i| i.sample_index) {
            if self.sample_bank.get(sid).is_some() {
                let _ = self.midi.note_on(channel, note, velocity);
                if let Some(ref mut audio) = self.audio {
                    audio.sample_note_on(sid, note, velocity, channel);
                }
                return;
            }
        }

        // Route 2: custom synth params
        if let Some(ref params) = inst.and_then(|i| i.synth_params.as_ref()) {
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                audio.note_on_with_params(channel, note, velocity, params);
            }
            return;
        }

        // Route 3: instrument number maps to preset patch (when instrument is explicitly set)
        if instrument.is_some() {
            let params = crate::audio::synth::SynthParams::from_patch(inst_idx as u8);
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                audio.note_on_with_params(channel, note, velocity, &params);
            }
            return;
        }

        // Route 4: default synth (channel program, no instrument specified)
        self.send_note_on(channel, note, velocity);
    }

    pub(crate) fn send_channel_note_off(&mut self, channel: u8) {
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref mut audio) = self.audio {
            audio.note_off_all_channel(channel);
            audio.sample_note_off_channel(channel);
        }
    }

    pub(crate) fn send_note_off(&mut self, channel: u8, note: u8) {
        let _ = self.midi.note_off(channel, note);
        if let Some(ref mut audio) = self.audio {
            audio.note_off(channel, note);
            audio.sample_note_off(channel, note);
        }
    }

    /// Preview a note: kills previous preview, starts new one, tracks it for auto-off.
    /// When instrument is Some, routes through the instrument-aware path (sample engine).
    pub(crate) fn preview_note(&mut self, channel: u8, note: u8, velocity: u8) {
        self.preview_note_with_instrument(channel, note, velocity, None);
    }

    pub(crate) fn preview_note_with_instrument(&mut self, channel: u8, note: u8, velocity: u8, instrument: Option<u8>) {
        // Kill previous preview note if any
        if let Some((prev_ch, _prev_note, _)) = self.preview_note.take() {
            self.send_channel_note_off(prev_ch);
        }
        if instrument.is_some() {
            self.send_note_on_with_instrument(channel, note, velocity, instrument);
        } else {
            self.send_note_on(channel, note, velocity);
        }
        self.preview_note = Some((channel, note, Instant::now()));
    }

    /// Expire preview note after timeout (call from main loop).
    pub fn expire_preview_note(&mut self) {
        if let Some((ch, _note, started)) = self.preview_note {
            if started.elapsed() > std::time::Duration::from_millis(PREVIEW_NOTE_TIMEOUT_MS) {
                self.send_channel_note_off(ch);
                self.preview_note = None;
            }
        }
    }

    pub(crate) fn send_all_notes_off(&mut self) {
        let _ = self.midi.all_notes_off();
        if let Some(ref mut audio) = self.audio {
            audio.note_off_all();
            audio.sample_note_off_all();
        }
    }

    pub(crate) fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        let _ = self.midi.send_cc(channel, controller, value);
        if let Some(ref mut audio) = self.audio {
            audio.send_cc(channel, controller, value);
        }
    }

    pub(crate) fn send_program_change(&mut self, channel: u8, program: u8) {
        let _ = self.midi.program_change(channel, program);
        if let Some(ref mut audio) = self.audio {
            audio.program_change(channel, program);
        }
    }

    pub(crate) fn send_pitch_bend(&mut self, channel: u8, value: u16) {
        let _ = self.midi.pitch_bend(channel, value);
        if let Some(ref mut audio) = self.audio {
            audio.pitch_bend(channel, value);
        }
    }

    // -- MIDI port selection --

    pub fn open_port_selector(&mut self) {
        // Build the port list: virtual port first (on unix), then hardware ports
        let mut ports = Vec::new();
        #[cfg(unix)]
        ports.push("RTRACK_MIDI (virtual)".to_string());

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

        let _selected = &self.dialogs.midi_port_list[self.dialogs.midi_port_cursor];

        // Index 0 on unix is the virtual port
        #[cfg(unix)]
        {
            if self.dialogs.midi_port_cursor == 0 {
                let _ = self.midi.create_virtual_port();
                self.close_port_selector();
                return;
            }
            // Hardware ports start at index 1 in our list, but index 0 in midir
            let hw_index = self.dialogs.midi_port_cursor - 1;
            let _ = self.midi.connect(hw_index);
        }

        #[cfg(not(unix))]
        {
            let _ = self.midi.connect(self.dialogs.midi_port_cursor);
        }

        self.close_port_selector();
    }

    pub fn current_order_position(&self) -> usize {
        if self.playing {
            self.engine.order
        } else {
            self.edit_order
        }
    }

    // -- Pattern / Order management --

    pub fn next_order_position(&mut self) {
        if self.edit_order + 1 < self.song.order.len() {
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
        self.push_undo();
        let idx = self.song.add_pattern();
        self.song.order.push(idx);
        self.song.order_repeats.push(1);
        self.edit_order = self.song.order.len() - 1;
        self.cursor_row = 0;
        self.status_message = Some(format!("New pattern {:02X}, order pos {:02X}", idx, self.edit_order));
    }

    pub fn clone_current_pattern(&mut self) {
        self.push_undo();
        let src_idx = self.song.order[self.edit_order];
        let cloned = self.song.patterns[src_idx].clone();
        let new_idx = self.song.patterns.len();
        self.song.patterns.push(cloned);
        self.song.order.insert(self.edit_order + 1, new_idx);
        self.song.order_repeats.insert(self.edit_order + 1, 1);
        self.edit_order += 1;
        self.cursor_row = 0;
        self.status_message = Some(format!("Cloned pattern {:02X} -> {:02X}", src_idx, new_idx));
    }

    pub fn midi_channel_for(&self, tracker_channel: usize) -> u8 {
        let ch = self.channels.get(tracker_channel)
            .map(|c| c.midi_channel)
            .unwrap_or(tracker_channel as u8);
        ch & 0x0F // clamp to valid MIDI channel range 0-15
    }

    pub fn is_channel_audible(&self, channel: usize) -> bool {
        if let Some(solo) = self.solo_channel {
            return channel == solo;
        }
        self.channels.get(channel).map_or(true, |c| !c.muted)
    }

    pub fn toggle_channel_mute(&mut self, channel: usize) {
        if let Some(ch_cfg) = self.channels.get_mut(channel) {
            self.solo_channel = None;
            ch_cfg.muted = !ch_cfg.muted;
            let muted = ch_cfg.muted;
            let state = if muted { "muted" } else { "unmuted" };
            self.status_message = Some(format!("Ch {} {}", channel + 1, state));
            if muted {
                let midi_ch = self.midi_channel_for(channel);
                self.send_channel_note_off(midi_ch);
            }
        }
    }

    pub fn toggle_solo(&mut self, channel: usize) {
        if self.solo_channel == Some(channel) {
            self.solo_channel = None;
            self.status_message = Some("Solo off".to_string());
        } else {
            self.solo_channel = Some(channel);
            self.status_message = Some(format!("Solo ch {}", channel + 1));
            // Kill notes on all non-solo channels
            for ch in 0..self.channels.len() {
                if ch != channel {
                    let midi_ch = self.midi_channel_for(ch);
                    self.send_channel_note_off(midi_ch);
                }
            }
        }
    }

    /// Get the effective pitch bend range for a channel (checks active instrument).
    pub(crate) fn channel_pitch_bend_per_semitone(&self, ch: usize) -> f64 {
        let range = self.engine.channel_states.get(ch)
            .and_then(|cs| cs.active_instrument)
            .and_then(|idx| self.instruments.get(idx as usize))
            .and_then(|inst| inst.pitch_bend_range)
            .unwrap_or(DEFAULT_PITCH_BEND_RANGE);
        (PITCH_BEND_CENTER as f64) / range
    }

    // -- File I/O --

    pub fn save(&mut self) {
        let path = self.file_path.clone().unwrap_or_else(|| {
            let name = self.song.title.replace(' ', "_").to_lowercase();
            PathBuf::from(format!("{}.rtrk", name))
        });
        let song_file = self.build_song_file(&path);
        match song_file.save(&path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
                self.dirty = false;
                self.last_autosave = Instant::now();
                self.status_message = Some(format!("Saved: {}", path.display()));
                // Clean up autosave file
                let _ = std::fs::remove_file(autosave_path_for(&path));
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
            }
        }
    }

    /// Auto-save to a temporary file if dirty and enough time has elapsed.
    pub fn auto_save(&mut self) {
        if !self.dirty {
            return;
        }
        if self.last_autosave.elapsed().as_secs() < AUTOSAVE_INTERVAL_SECS {
            return;
        }
        self.last_autosave = Instant::now();
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.rtrk", name))
            }
        };
        let autosave_path = autosave_path_for(&path);
        let song_file = self.build_song_file(&path);
        if let Err(e) = song_file.save(&autosave_path) {
            self.status_message = Some(format!("Auto-save failed: {}", e));
        }
    }

    /// Remove the auto-save temp file (called after manual save or on clean quit).
    pub fn cleanup_autosave(&self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.rtrk", name))
            }
        };
        let autosave_path = autosave_path_for(&path);
        let _ = std::fs::remove_file(autosave_path);
    }

    /// Build a SongFile with instrument definitions and sample references
    fn build_song_file(&self, save_path: &std::path::Path) -> SongFile {
        let save_dir = save_path.parent().unwrap_or(std::path::Path::new("."));

        // Collect non-empty instruments
        let instruments: Vec<InstrumentEntry> = self.instruments.iter().enumerate()
            .filter(|(_, inst)| !inst.name.is_empty() || inst.sample_index.is_some() || inst.midi_program.is_some() || inst.synth_params.is_some())
            .map(|(slot, inst)| InstrumentEntry {
                slot,
                def: InstrumentDef {
                    name: inst.name.clone(),
                    midi_program: inst.midi_program,
                    sample_index: inst.sample_index,
                    synth_params: inst.synth_params.clone(),
                    pitch_bend_range: inst.pitch_bend_range,
                },
            })
            .collect();

        // Collect sample references with relative paths
        let sample_refs: Vec<SampleRefEntry> = self.sample_bank.samples.iter().enumerate()
            .filter_map(|(slot, opt)| {
                opt.as_ref().map(|sample| {
                    let rel_path = sample.source_path.as_ref().map(|p| {
                        let abs = std::path::Path::new(p);
                        make_relative(save_dir, abs)
                    }).unwrap_or_default();

                    SampleRefEntry {
                        slot,
                        sample_ref: SampleRef {
                            name: sample.name.clone(),
                            path: rel_path,
                            base_note: sample.base_note,
                            trim_start: sample.trim_start,
                            trim_end: sample.trim_end,
                            loop_enabled: sample.loop_enabled,
                            loop_start: sample.loop_start,
                            loop_end: sample.loop_end,
                        },
                    }
                })
            })
            .collect();

        SongFile {
            song: self.song.clone(),
            instruments,
            sample_refs,
        }
    }

    pub fn load_file(&mut self, path: PathBuf) {
        match SongFile::load(&path) {
            Ok(song_file) => {
                let song = song_file.song;
                self.channels = Self::default_channel_configs(song.channels);
                self.solo_channel = None;
                self.song = song;
                self.song.sync_order_repeats();
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.track_page = 0;
                self.history.undo_stack.clear();
                self.history.redo_stack.clear();

                // Restore instruments
                for entry in &song_file.instruments {
                    if entry.slot < self.instruments.len() {
                        self.instruments[entry.slot].name = entry.def.name.clone();
                        self.instruments[entry.slot].midi_program = entry.def.midi_program;
                        self.instruments[entry.slot].sample_index = entry.def.sample_index;
                        self.instruments[entry.slot].synth_params = entry.def.synth_params.clone();
                        self.instruments[entry.slot].pitch_bend_range = entry.def.pitch_bend_range;
                    }
                }

                // Reload samples from file references
                let load_dir = path.parent().unwrap_or(std::path::Path::new("."));
                let mut bank = (*self.sample_bank).clone();
                let mut sample_errors = Vec::new();
                for entry in &song_file.sample_refs {
                    if entry.slot >= bank.samples.len() {
                        continue;
                    }
                    let sample_path = resolve_relative(load_dir, &entry.sample_ref.path);
                    match bank.load(entry.slot, &sample_path) {
                        Ok(()) => {
                            // Apply saved metadata on top of freshly loaded sample
                            if let Some(ref mut sample) = bank.samples[entry.slot] {
                                sample.base_note = entry.sample_ref.base_note;
                                sample.trim_start = entry.sample_ref.trim_start;
                                sample.trim_end = entry.sample_ref.trim_end;
                                sample.loop_enabled = entry.sample_ref.loop_enabled;
                                sample.loop_start = entry.sample_ref.loop_start;
                                sample.loop_end = entry.sample_ref.loop_end;
                            }
                        }
                        Err(e) => {
                            sample_errors.push(format!("{}: {}", entry.sample_ref.name, e));
                        }
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }

                self.file_path = Some(path.clone());
                self.dirty = false;
                if sample_errors.is_empty() {
                    self.status_message = Some(format!("Loaded: {}", path.display()));
                } else {
                    self.status_message = Some(format!(
                        "Loaded (missing samples: {}): {}",
                        sample_errors.join(", "),
                        path.display()
                    ));
                }
            }
            Err(e) => {
                self.status_message = Some(format!("Load failed: {}", e));
            }
        }
    }

    // -- Undo/Redo --

    pub fn push_undo(&mut self) {
        self.history.undo_stack.push_back(self.song.clone());
        self.history.redo_stack.clear();
        if self.history.undo_stack.len() > MAX_UNDO_HISTORY {
            self.history.undo_stack.pop_front();
        }
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.history.undo_stack.pop_back() {
            self.history.redo_stack.push(self.song.clone());
            self.song = prev;
            self.status_message = Some("Undo".to_string());
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.history.redo_stack.pop() {
            self.history.undo_stack.push_back(self.song.clone());
            self.song = next;
            self.status_message = Some("Redo".to_string());
        }
    }

    // -- Clipboard --

    pub fn copy_row(&mut self) {
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &self.song.patterns[pattern_idx];
        let row: Vec<crate::tracker::Cell> = (0..pattern.channels)
            .map(|ch| *pattern.get(self.cursor_row, ch))
            .collect();
        self.history.clipboard = Some(row);
        self.status_message = Some(format!("Copied row {:02X}", self.cursor_row));
    }

    pub fn paste_row(&mut self) {
        if let Some(ref row) = self.history.clipboard.clone() {
            self.push_undo();
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &mut self.song.patterns[pattern_idx];
            for (ch, cell) in row.iter().enumerate() {
                if ch < pattern.channels {
                    pattern.set_cell(self.cursor_row, ch, *cell);
                }
            }
            self.status_message = Some(format!("Pasted at row {:02X}", self.cursor_row));
        }
    }

    pub fn cut_row(&mut self) {
        self.copy_row();
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &mut self.song.patterns[pattern_idx];
        for ch in 0..pattern.channels {
            pattern.set_cell(self.cursor_row, ch, crate::tracker::Cell::default());
        }
    }

    // -- Theme cycling --

    pub fn cycle_theme(&mut self) {
        self.theme_index = (self.theme_index + 1) % crate::ui::theme::THEME_NAMES.len();
        let name = crate::ui::theme::THEME_NAMES[self.theme_index];
        self.status_message = Some(format!("Theme: {}", name));
    }

    pub fn theme(&self) -> crate::ui::theme::Theme {
        let name = crate::ui::theme::THEME_NAMES.get(self.theme_index).copied().unwrap_or("dark");
        crate::ui::theme::theme_by_name(name)
    }

    // -- MIDI clock toggle --

    pub fn toggle_midi_clock(&mut self) {
        self.midi.clock_enabled = !self.midi.clock_enabled;
        let state = if self.midi.clock_enabled { "on" } else { "off" };
        self.status_message = Some(format!("MIDI clock {}", state));
    }

    // -- MIDI file export/import --

    pub fn export_wav_file(&mut self) {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("wav"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.wav", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        match crate::sample::export::render_to_wav(
            &path, &self.song, &self.sample_bank, &instruments, &self.channel_effects_params_slice(), &self.send_bus_params, sample_rate,
        ) {
            Ok(()) => {
                self.status_message = Some(format!("Exported WAV: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("WAV export failed: {}", e));
            }
        }
    }

    pub fn export_flac_file(&mut self) {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("flac"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.flac", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        match crate::sample::export::render_to_flac(
            &path, &self.song, &self.sample_bank, &instruments, &self.channel_effects_params_slice(), &self.send_bus_params, sample_rate,
        ) {
            Ok(()) => {
                self.status_message = Some(format!("Exported FLAC: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("FLAC export failed: {}", e));
            }
        }
    }

    pub fn export_midi(&mut self) {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("mid"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.mid", name))
            });
        match crate::midi_file::export_midi(&self.song, &path) {
            Ok(()) => {
                self.status_message = Some(format!("Exported: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Export failed: {}", e));
            }
        }
    }

    pub fn import_midi_file(&mut self, path: PathBuf) {
        match crate::midi_file::import_midi(&path) {
            Ok(song) => {
                self.push_undo();
                self.channels = Self::default_channel_configs(song.channels);
                self.solo_channel = None;
                self.song = song;
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.status_message = Some(format!("Imported: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Import failed: {}", e));
            }
        }
    }

    /// Get the range of visible channels for the current track page
    pub fn visible_channels(&self) -> std::ops::Range<usize> {
        let start = self.track_page * CHANNELS_PER_PAGE;
        let end = (start + CHANNELS_PER_PAGE).min(self.song.channels);
        start..end
    }
}

/// Make a path relative to a base directory. Falls back to absolute if no common prefix.
/// Compute the auto-save path for a given song file path.
fn autosave_path_for(path: &std::path::Path) -> std::path::PathBuf {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let name = path.file_name().and_then(|f| f.to_str()).unwrap_or("song");
    dir.join(format!(".{}.autosave", name))
}

fn make_relative(base: &std::path::Path, target: &std::path::Path) -> String {
    // Try to canonicalize both for reliable comparison
    let base_abs = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let target_abs = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());

    if let Ok(rel) = target_abs.strip_prefix(&base_abs) {
        return rel.to_string_lossy().to_string();
    }

    // Fall back to the original path
    target.to_string_lossy().to_string()
}

/// Resolve a (possibly relative) path against a base directory.
/// Rejects path traversal (`..` components) and absolute paths to prevent
/// a malicious .rtrk file from referencing files outside the song directory.
fn resolve_relative(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(rel);
    // Reject absolute paths -- only allow relative references
    if p.is_absolute() {
        return base.join(
            p.file_name().unwrap_or_default(),
        );
    }
    // Strip any `..` components to prevent directory traversal
    let sanitized: std::path::PathBuf = p
        .components()
        .filter(|c| !matches!(c, std::path::Component::ParentDir))
        .collect();
    base.join(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use crate::tracker::Note;

    fn make_app() -> App {
        let song = Song::new(4, 64);
        let engine = crate::engine::TrackerEngine::new(&song, true);
        App {
            song,
            midi: MidiEngine::new(),
            midi_input: MidiInputEngine::new(),
            link: LinkEngine::new(120.0),
            mode: Mode::Normal,
            should_quit: false,
            cursor_row: 0,
            cursor_channel: 0,
            cursor_sub: SubColumn::Note,
            current_octave: 4,
            playing: false,
            recording: false,
            engine,
            timing: PlaybackTiming::new(),
            clock_mode: ClockMode::Internal,
            edit_step: 1,
            file_path: None,
            status_message: None,
            last_autosave: Instant::now(),
            history: EditHistory::new(),
            edit_order: 0,
            solo_channel: None,
            prev_mode: Mode::Normal,
            instruments: (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect(),
            theme_index: 0,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            track_page: 0,
            preview_note: None,
            dialogs: DialogState {
                midi_port_list: Vec::new(),
                midi_port_cursor: 0,
                settings_field: SettingsField::Title,
                settings_edit_buf: String::new(),
                instrument_cursor: 0,
                sample_editor_slot: 0,
                sample_editor_field: SampleField::BaseNote,
                sample_slice_count: 8,
                sample_slice_sensitivity: 0.5,
                synth_editor_slot: 0,
                synth_editor_field: SynthField::Waveform,
                help_scroll: 0,
                file_browser: FileBrowserState { dir: PathBuf::from("/tmp"), entries: Vec::new(), cursor: 0, action: FileBrowserAction::OpenSong, filter: Vec::new(), scroll: 0 },
            },
            dirty: false,
            follow_playback: true,
            rename_buf: String::new(),
            ch_fx_field: 0,
            matrix_cursor: 0,
            command_buf: String::new(),
            channels: App::default_channel_configs(4),
            send_bus_params: (0..crate::audio::effects::MAX_SEND_BUSES).map(|_| crate::audio::effects::SendBusParams::default()).collect(),
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

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        // Cursor should have advanced by edit_step (1)
        assert_eq!(app.cursor_row, 1);
    }

    #[test]
    fn test_note_off_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        app.handle_key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE));
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
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
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_play_stop() {
        let mut app = make_app();

        app.play();
        assert!(app.playing);

        app.stop();
        assert!(!app.playing);
    }

    #[test]
    fn test_play_starts_from_edit_order() {
        let mut app = make_app();
        app.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.play();
        assert_eq!(app.engine.order, 2);
        assert_eq!(app.engine.row, 5);
    }

    #[test]
    fn test_play_from_start() {
        let mut app = make_app();
        app.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.play_from_start();
        assert_eq!(app.engine.order, 0);
        assert_eq!(app.engine.row, 0);
        assert_eq!(app.edit_order, 0);
    }

    #[test]
    fn test_ctrl_space_plays_from_start() {
        let mut app = make_app();
        app.song.order = vec![0, 0, 0];
        app.edit_order = 2;
        app.cursor_row = 5;

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(app.playing);
        assert_eq!(app.engine.order, 0);
        assert_eq!(app.engine.row, 0);
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
        app.song.channels = 8;
        for pat in &mut app.song.patterns {
            pat.channels = 8;
            for row in &mut pat.data {
                row.resize(8, crate::tracker::Cell::default());
            }
        }
        app.channels = App::default_channel_configs(8);

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

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(cell.note, Some(Note::On { value: crate::tracker::NoteValue::C, octave: 5 }));
    }

    #[test]
    fn test_hex_entry_instrument() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.cursor_sub = SubColumn::Instrument;

        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
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
        let original_title = app.song.title.clone();

        app.push_undo();
        app.song.title = "Modified".to_string();

        app.undo();
        assert_eq!(app.song.title, original_title);

        app.redo();
        assert_eq!(app.song.title, "Modified");
    }

    #[test]
    fn test_undo_clears_redo_on_new_edit() {
        let mut app = make_app();
        app.push_undo();
        app.song.title = "Edit 1".to_string();

        app.undo();
        assert!(!app.history.redo_stack.is_empty());

        // New edit should clear redo
        app.push_undo();
        assert!(app.history.redo_stack.is_empty());
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
        assert!(app.history.clipboard.is_some());

        // Move and paste
        app.cursor_row = 5;
        app.paste_row();

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(5, 0);
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
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());

        // Clipboard should have the data
        assert!(app.history.clipboard.is_some());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let mut app = make_app();
        app.song.title = "Test Song".to_string();

        // Enter a note
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;

        // Save
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_roundtrip.rtrk");
        app.file_path = Some(path.clone());
        app.save();

        // Modify the song
        app.song.title = "Modified".to_string();

        // Load
        app.load_file(path.clone());
        assert_eq!(app.song.title, "Test Song");

        // Clean up
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_s_saves() {
        let mut app = make_app();
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_ctrl_s.rtrk");
        app.file_path = Some(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_z_undoes() {
        let mut app = make_app();
        app.push_undo();
        app.song.title = "Changed".to_string();

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_ne!(app.song.title, "Changed");
    }

    #[test]
    fn test_order_navigation() {
        let mut app = make_app();

        // Add a second pattern to order
        let new_pat = app.song.add_pattern();
        app.song.order.push(new_pat);

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
        let original_patterns = app.song.patterns.len();
        let original_order = app.song.order.len();

        app.add_new_pattern_to_order();

        assert_eq!(app.song.patterns.len(), original_patterns + 1);
        assert_eq!(app.song.order.len(), original_order + 1);
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
        assert_eq!(app.song.patterns.len(), 2);
        // The cloned pattern should have the same note
        let cell = app.song.patterns[1].get(0, 0);
        assert!(cell.note.is_some());
    }

    #[test]
    fn test_channel_mute() {
        let mut app = make_app();
        assert!(app.is_channel_audible(0));

        app.toggle_channel_mute(0);
        assert!(!app.is_channel_audible(0));

        app.toggle_channel_mute(0);
        assert!(app.is_channel_audible(0));
    }

    #[test]
    fn test_ctrl_right_navigates_order() {
        let mut app = make_app();
        let new_pat = app.song.add_pattern();
        app.song.order.push(new_pat);

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
        assert!(app.is_channel_audible(0));

        app.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE));
        assert!(!app.is_channel_audible(0));
    }

    #[test]
    fn test_midi_channel_mapping() {
        let app = make_app();
        assert_eq!(app.midi_channel_for(0), 0);
        assert_eq!(app.midi_channel_for(1), 1);
        assert_eq!(app.midi_channel_for(3), 3);
        // Out of range returns clamped
        assert_eq!(app.midi_channel_for(99), 99 & 0x0F);
    }

    #[test]
    fn test_solo_channel() {
        let mut app = make_app();

        app.toggle_solo(1);
        assert!(!app.is_channel_audible(0));
        assert!(app.is_channel_audible(1));
        assert!(!app.is_channel_audible(2));

        // Toggle same channel off
        app.toggle_solo(1);
        assert!(app.is_channel_audible(0));
        assert!(app.is_channel_audible(1));
    }

    #[test]
    fn test_solo_overrides_mute() {
        let mut app = make_app();

        app.toggle_channel_mute(1);
        assert!(!app.is_channel_audible(1));

        // Solo channel 1 should override the mute
        app.toggle_solo(1);
        assert!(app.is_channel_audible(1));
    }

    #[test]
    fn test_mute_clears_solo() {
        let mut app = make_app();
        app.toggle_solo(1);
        assert_eq!(app.solo_channel, Some(1));

        // Toggling mute clears solo
        app.toggle_channel_mute(0);
        assert_eq!(app.solo_channel, None);
    }

    #[test]
    fn test_ctrl_f9_toggles_solo() {
        let mut app = make_app();
        assert!(app.is_channel_audible(0));
        assert!(app.is_channel_audible(1));

        // Ctrl+F9 solos channel 0
        app.handle_key(KeyEvent::new(KeyCode::F(9), KeyModifiers::CONTROL));
        assert!(app.is_channel_audible(0));
        assert!(!app.is_channel_audible(1));
    }

    #[test]
    fn test_pattern_break_effect() {
        let mut app = make_app();
        // Add a second pattern
        let pat2 = app.song.add_pattern();
        app.song.order.push(pat2);

        // Set pattern break (D08) at row 0 of pattern 0
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PATTERN_BREAK);
        cell.effect_value = Some(8);

        app.play();
        // Tick 0: advance row -> pattern break fires
        app.process_tick();
        assert_eq!(app.engine.order, 1);
        assert_eq!(app.engine.row, 8);
    }

    #[test]
    fn test_position_jump_effect() {
        let mut app = make_app();
        let pat2 = app.song.add_pattern();
        let pat3 = app.song.add_pattern();
        app.song.order.push(pat2);
        app.song.order.push(pat3);

        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_POSITION_JUMP);
        cell.effect_value = Some(2);

        app.play();
        app.process_tick();
        assert_eq!(app.engine.order, 2);
        assert_eq!(app.engine.row, 0);
    }

    #[test]
    fn test_position_jump_with_break() {
        let mut app = make_app();
        let pat2 = app.song.add_pattern();
        let pat3 = app.song.add_pattern();
        app.song.order.push(pat2);
        app.song.order.push(pat3);

        let pat_idx = app.song.order[0];
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 0);
            cell.effect = Some(EFFECT_POSITION_JUMP);
            cell.effect_value = Some(2);
        }
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 1);
            cell.effect = Some(EFFECT_PATTERN_BREAK);
            cell.effect_value = Some(4);
        }

        app.play();
        app.process_tick();
        assert_eq!(app.engine.order, 2);
        assert_eq!(app.engine.row, 4);
    }

    #[test]
    fn test_pattern_break_wraps_order() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PATTERN_BREAK);
        cell.effect_value = Some(0);

        app.play();
        app.process_tick();
        assert_eq!(app.engine.order, 0);
        assert_eq!(app.engine.row, 0);
        assert_eq!(app.engine.generation, 1);
    }

    #[test]
    fn test_position_jump_clamps_to_max() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_POSITION_JUMP);
        cell.effect_value = Some(99);

        app.play();
        app.process_tick();
        assert_eq!(app.engine.order, 0); // clamped to max
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
        let pattern_idx = app.song.order[0];
        let original_rows = app.song.patterns[pattern_idx].rows;

        // Insert a row at cursor (pattern length stays constant -- row is inserted, last row dropped)
        app.insert_row_at_cursor();
        assert_eq!(app.song.patterns[pattern_idx].rows, original_rows);
        // The inserted row should be empty
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_delete_row() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        let original_rows = app.song.patterns[pattern_idx].rows;

        // Enter a note at row 0
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        app.mode = Mode::Normal;
        app.cursor_row = 0;

        app.delete_row_at_cursor();
        assert_eq!(app.song.patterns[pattern_idx].rows, original_rows); // +1 from insert, -1 from delete
    }

    #[test]
    fn test_per_pattern_length_cursor_bounds() {
        let mut app = make_app();
        // Change first pattern to 32 rows
        let pattern_idx = app.song.order[0];
        app.song.patterns[pattern_idx].rows = 32;
        app.song.patterns[pattern_idx].data.truncate(32);

        // Try to move past the end
        app.move_cursor_down(100);
        assert_eq!(app.cursor_row, 31);
    }

    #[test]
    fn test_per_pattern_length_playback_advance() {
        let mut app = make_app();
        // Set pattern to 4 rows
        let pattern_idx = app.song.order[0];
        app.song.patterns[pattern_idx].rows = 4;
        app.song.patterns[pattern_idx].data.truncate(4);

        app.play();
        app.engine.row = 3; // last row

        // Advance past end -> should wrap
        app.process_tick();
        assert_eq!(app.engine.row, 0);
    }

    #[test]
    fn test_midi_cc_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_MIDI_CC);
        cell.instrument = Some(1);
        cell.effect_value = Some(0x40);

        app.play();
        // process_tick dispatches MidiCC event to MIDI output
        app.process_tick();
    }

    #[test]
    fn test_program_change_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PROGRAM_CHANGE);
        cell.effect_value = Some(5);

        app.play();
        app.process_tick();
    }

    #[test]
    fn test_arpeggio_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_ARPEGGIO);
        cell.effect_value = Some(0x37);

        app.play();
        // Tick 0: triggers note
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48)); // C-4

        // Tick 1: arpeggio pitch bend
        app.process_tick();
    }

    #[test]
    fn test_portamento_up_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_PORTA_UP);
        cell.effect_value = Some(0x10);

        app.play();
        // Tick 0: note on
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].pitch_offset, 0.0);

        // Tick 1: pitch should increase
        app.process_tick();
        assert!(app.engine.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_portamento_down_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_PORTA_DOWN);
        cell.effect_value = Some(0x10);

        app.play();
        app.process_tick(); // tick 0: note on
        app.process_tick(); // tick 1: effects
        assert!(app.engine.channel_states[0].pitch_offset < 0.0);
    }

    #[test]
    fn test_tone_portamento_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        // Row 0: C-4 note (no effect)
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 0);
            cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
            cell.volume = Some(100);
        }
        // Row 1: E-4 with tone porta (3xx)
        {
            let cell = app.song.patterns[pat_idx].get_mut(1, 0);
            cell.note = Some(Note::On { value: crate::tracker::NoteValue::E, octave: 4 });
            cell.effect = Some(EFFECT_TONE_PORTA);
            cell.effect_value = Some(0x10);
        }

        app.play();
        // Row 0, tick 0: triggers C-4
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48));

        // Advance through remaining ticks of row 0 (ticks 1-5)
        for _ in 1..app.song.speed { app.process_tick(); }

        // Row 1, tick 0: sets target to E-4 but keeps C-4 playing
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48));
        assert_eq!(app.engine.channel_states[0].porta_target, Some(52));

        // Tick 1: pitch should start sliding up
        app.process_tick();
        assert!(app.engine.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_vibrato_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VIBRATO);
        cell.effect_value = Some(0x42);

        app.play();
        app.process_tick(); // tick 0: note on
        app.process_tick(); // tick 1: vibrato
        assert!(app.engine.channel_states[0].vibrato_phase > 0.0);
    }

    #[test]
    fn test_volume_slide_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x02);

        app.play();
        app.process_tick(); // tick 0
        assert_eq!(app.engine.channel_states[0].volume, 100);

        app.process_tick(); // tick 1: slide
        assert_eq!(app.engine.channel_states[0].volume, 98);
    }

    #[test]
    fn test_volume_slide_up() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x30);

        app.play();
        app.process_tick(); // tick 0
        app.process_tick(); // tick 1: slide up
        assert_eq!(app.engine.channel_states[0].volume, 103);
    }

    #[test]
    fn test_volume_slide_clamps() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(5);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x0F);

        app.play();
        app.process_tick(); // tick 0
        app.process_tick(); // tick 1: slide down
        assert_eq!(app.engine.channel_states[0].volume, 0);
    }

    #[test]
    fn test_set_speed_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_SET_SPEED);
        cell.effect_value = Some(3);

        app.play();
        app.process_tick();
        assert_eq!(app.song.speed, 3);
    }

    #[test]
    fn test_set_tempo_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_SET_SPEED);
        cell.effect_value = Some(0x80);

        app.play();
        app.process_tick();
        assert_eq!(app.song.bpm, 0x80);
    }

    #[test]
    fn test_sub_tick_timing() {
        let mut app = make_app();
        app.song.speed = 6;

        app.play();

        // Tick 0
        app.process_tick();
        assert_eq!(app.engine.tick, 1);

        // Ticks 1-5
        for _ in 0..5 {
            app.process_tick();
        }
        // After tick 5, should reset to 0
        assert_eq!(app.engine.tick, 0);
    }

    #[test]
    fn test_note_delay_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        // Note C-4 with delay 3 ticks
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_NOTE_DELAY);
        cell.effect_value = Some(3);

        app.play();

        // Tick 0: note should be deferred, not triggered
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, None);
        assert!(app.engine.channel_states[0].delayed_note.is_some());

        // Tick 1: still waiting
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, None);

        // Tick 2: still waiting
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, None);

        // Tick 3: should trigger
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48));
    }

    #[test]
    fn test_note_delay_off() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        // First row: trigger C-4
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 0);
            cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
            cell.volume = Some(100);
        }
        // Second row: note-off with delay 2
        {
            let cell = app.song.patterns[pat_idx].get_mut(1, 0);
            cell.note = Some(Note::Off);
            cell.effect = Some(EFFECT_NOTE_DELAY);
            cell.effect_value = Some(2);
        }

        app.play();

        // Row 0, tick 0: trigger the note
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48));

        // Advance through remaining ticks of row 0 (ticks 1-5)
        for _ in 1..app.song.speed { app.process_tick(); }

        // Row 1, tick 0: note-off should be deferred
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48)); // still active
        assert!(app.engine.channel_states[0].delayed_note.is_some());

        // Tick 1: waiting
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, Some(48));

        // Tick 2: note-off triggers
        app.process_tick();
        assert_eq!(app.engine.channel_states[0].note, None);
    }

    #[test]
    fn test_midi_input_in_insert_mode() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;

        // Simulate MIDI note C-4 (note 60)
        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.volume, Some(100));
    }

    #[test]
    fn test_midi_input_ignored_in_normal_mode() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Normal;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        // Should not have written to pattern
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_midi_input_ignored_during_playback() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = true;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_recording_toggle() {
        let mut app = make_app();
        assert!(!app.recording);
        app.toggle_recording();
        assert!(app.recording);
        app.toggle_recording();
        assert!(!app.recording);
    }

    #[test]
    fn test_punch_in_records_at_engine_position() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = true;
        app.recording = true;
        // Position engine at row 5
        app.engine.row = 5;
        app.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 110 });

        let pattern_idx = app.song.order[0];
        // Written at engine row (5), not cursor_row (0)
        let cell = app.song.patterns[pattern_idx].get(5, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.volume, Some(110));
        // Cursor row should not have advanced
        assert_eq!(app.cursor_row, 0);
        // Pattern at cursor row should be untouched
        let cell0 = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell0.note.is_none());
    }

    #[test]
    fn test_punch_in_no_record_without_flag() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = true;
        app.recording = false;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        // Should not record (preview only)
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_punch_in_noteoff_recorded() {
        use crate::midi::MidiInputEvent;
        use crate::tracker::Note;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = true;
        app.recording = true;
        app.engine.row = 3;
        app.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOff { channel: 0, note: 60 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(3, 0);
        assert_eq!(cell.note, Some(Note::Off));
    }

    #[test]
    fn test_noteoff_not_recorded_in_step_mode() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = false;

        app.handle_midi_input(MidiInputEvent::NoteOff { channel: 0, note: 60 });

        // Step mode should NOT record note-off from MIDI
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
    }

    #[test]
    fn test_punch_in_auto_fills_instrument() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Synth;
        app.channels[0].default_instrument = Some(7);
        app.mode = Mode::Insert;
        app.playing = true;
        app.recording = true;
        app.engine.row = 2;
        app.engine.order = 0;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 64, velocity: 100 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(2, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(7));
    }

    #[test]
    fn test_step_record_auto_fills_instrument() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Sample;
        app.channels[0].default_instrument = Some(3);
        app.mode = Mode::Insert;
        app.playing = false;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(3));
    }

    #[test]
    fn test_punch_in_sets_dirty() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;
        app.playing = true;
        app.recording = true;
        app.dirty = false;

        app.handle_midi_input(MidiInputEvent::NoteOn { channel: 0, note: 60, velocity: 100 });

        assert!(app.dirty);
    }

    #[test]
    fn test_pattern_break_clamps_row() {
        let mut app = make_app();
        // Second pattern with only 16 rows
        let pat2 = app.song.add_pattern();
        app.song.patterns[pat2].rows = 16;
        app.song.patterns[pat2].data.truncate(16);
        app.song.order.push(pat2);

        // Pattern break to row 99 (beyond bounds of pattern 2)
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PATTERN_BREAK);
        cell.effect_value = Some(99);

        app.play();
        app.process_tick();
        assert_eq!(app.engine.order, 1);
        assert_eq!(app.engine.row, 15); // clamped to max row
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
        assert_eq!(app.song.title, "New Title");
    }

    #[test]
    fn test_song_settings_edit_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Bpm;
        app.dialogs.settings_edit_buf = "140".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.song.bpm, 140);
    }

    #[test]
    fn test_song_settings_edit_channels() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Channels;
        app.dialogs.settings_edit_buf = "8".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.song.channels, 8);
    }

    #[test]
    fn test_song_settings_clamps_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Bpm;

        // Too low
        app.dialogs.settings_edit_buf = "10".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 32);

        // Too high
        app.dialogs.settings_edit_buf = "500".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 300);
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

        assert_eq!(app.instruments[0].name, "Test");

        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.instruments[0].name, "Tes");
    }

    #[test]
    fn test_theme_cycling() {
        let mut app = make_app();
        let initial = app.theme_index;

        app.cycle_theme();
        assert_ne!(app.theme_index, initial);

        // Cycle through all themes back to start
        let count = crate::ui::theme::THEME_NAMES.len();
        for _ in 0..count - 1 {
            app.cycle_theme();
        }
        assert_eq!(app.theme_index, initial);
    }

    #[test]
    fn test_midi_clock_toggle() {
        let mut app = make_app();
        let initial = app.midi.clock_enabled;

        app.toggle_midi_clock();
        assert_ne!(app.midi.clock_enabled, initial);

        app.toggle_midi_clock();
        assert_eq!(app.midi.clock_enabled, initial);
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
        assert!(app.instruments[0].synth_params.is_some());
        // Navigate fields
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.dialogs.synth_editor_field, SynthField::Attack);
        // Adjust value
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let attack = app.instruments[0].synth_params.as_ref().unwrap().attack;
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
        assert!(app.instruments[0].synth_params.is_some());
        // Delete clears params
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(app.instruments[0].synth_params.is_none());
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
        app.file_path = Some(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));

        let midi_path = path.with_extension("mid");
        assert!(midi_path.exists());
        let _ = std::fs::remove_file(midi_path);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_ctrl_m_toggles_clock() {
        let mut app = make_app();
        let initial = app.midi.clock_enabled;
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL));
        assert_ne!(app.midi.clock_enabled, initial);
    }

    #[test]
    fn test_dirty_flag_on_edit() {
        let mut app = make_app();
        assert!(!app.dirty);
        app.mode = Mode::Insert;
        // Enter a note (triggers push_undo -> dirty)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(app.dirty);
    }

    #[test]
    fn test_dirty_flag_cleared_on_save() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(app.dirty);
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_dirty.rtrk");
        app.file_path = Some(path.clone());
        app.save();
        assert!(!app.dirty);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_quit_confirm_when_dirty() {
        let mut app = make_app();
        app.dirty = true;
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
        app.dirty = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::QuitConfirm);
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn test_quit_no_confirm_when_clean() {
        let mut app = make_app();
        assert!(!app.dirty);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn test_note_transpose_up() {
        let mut app = make_app();
        // Place a C-4 note
        let pattern_idx = app.song.order[0];
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            note: Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 }),
            ..Default::default()
        });
        // Shift+Up transposes up 1 semitone
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(cell.note, Some(Note::On { value: crate::tracker::NoteValue::Cs, octave: 4 }));
    }

    #[test]
    fn test_note_transpose_down() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            note: Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 }),
            ..Default::default()
        });
        // Shift+Down transposes down 1 semitone (C-4 -> B-3)
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert_eq!(cell.note, Some(Note::On { value: crate::tracker::NoteValue::B, octave: 3 }));
    }

    #[test]
    fn test_block_select_toggle() {
        let mut app = make_app();
        assert!(app.history.block_anchor.is_none());
        // Ctrl+B toggles block selection
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(app.history.block_anchor, Some((0, 0)));
        // Toggle off
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(app.history.block_anchor.is_none());
    }

    #[test]
    fn test_block_copy_paste() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        // Place notes in rows 0-1, channels 0-1
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            note: Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 }),
            ..Default::default()
        });
        app.song.patterns[pattern_idx].set_cell(1, 1, crate::tracker::Cell {
            note: Some(Note::On { value: crate::tracker::NoteValue::E, octave: 4 }),
            ..Default::default()
        });
        // Start block at (0,0)
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        // Move cursor to (1,1)
        app.cursor_row = 1;
        app.cursor_channel = 1;
        // Copy block
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.history.block_clipboard.is_some());
        let clip = app.history.block_clipboard.as_ref().unwrap();
        assert_eq!(clip.len(), 2); // 2 rows
        assert_eq!(clip[0].len(), 2); // 2 channels
        // Paste at (4,0)
        app.cursor_row = 4;
        app.cursor_channel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        let cell = app.song.patterns[pattern_idx].get(4, 0);
        assert_eq!(cell.note, Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 }));
        let cell2 = app.song.patterns[pattern_idx].get(5, 1);
        assert_eq!(cell2.note, Some(Note::On { value: crate::tracker::NoteValue::E, octave: 4 }));
    }

    #[test]
    fn test_block_cut_clears_selection() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            note: Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 }),
            ..Default::default()
        });
        // Start block at (0,0), cursor at (0,0)
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        // Cut
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        // Original cell should be cleared
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
        // Block anchor should be cleared
        assert!(app.history.block_anchor.is_none());
    }

    #[test]
    fn test_atomic_save() {
        let mut app = make_app();
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("rtrack_test_atomic.rtrk");
        app.file_path = Some(path.clone());
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
        assert_eq!(app.channels[0].name, "");
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
        assert_eq!(app.channels[0].name, "Kick");
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
        assert_eq!(app.channels[0].name, "A");
    }

    #[test]
    fn test_interpolate_volume() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        // Set volume at row 0 and row 4
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            volume: Some(0),
            ..Default::default()
        });
        app.song.patterns[pattern_idx].set_cell(4, 0, crate::tracker::Cell {
            volume: Some(100),
            ..Default::default()
        });
        // Select block from (0,0) to (4,0)
        app.history.block_anchor = Some((0, 0));
        app.cursor_row = 4;
        app.cursor_channel = 0;
        // Interpolate
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        // Check intermediate values
        assert_eq!(app.song.patterns[pattern_idx].get(0, 0).volume, Some(0));
        assert_eq!(app.song.patterns[pattern_idx].get(1, 0).volume, Some(25));
        assert_eq!(app.song.patterns[pattern_idx].get(2, 0).volume, Some(50));
        assert_eq!(app.song.patterns[pattern_idx].get(3, 0).volume, Some(75));
        assert_eq!(app.song.patterns[pattern_idx].get(4, 0).volume, Some(100));
    }

    #[test]
    fn test_interpolate_effect_value() {
        let mut app = make_app();
        let pattern_idx = app.song.order[0];
        // Set effect at row 0 and row 2 (same effect command)
        app.song.patterns[pattern_idx].set_cell(0, 0, crate::tracker::Cell {
            effect: Some(5), effect_value: Some(0),
            ..Default::default()
        });
        app.song.patterns[pattern_idx].set_cell(2, 0, crate::tracker::Cell {
            effect: Some(5), effect_value: Some(80),
            ..Default::default()
        });
        app.history.block_anchor = Some((0, 0));
        app.cursor_row = 2;
        app.cursor_channel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert_eq!(app.song.patterns[pattern_idx].get(1, 0).effect, Some(5));
        assert_eq!(app.song.patterns[pattern_idx].get(1, 0).effect_value, Some(40));
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
        let normal = resolve_relative(base, "samples/kick.wav");
        assert_eq!(normal, std::path::PathBuf::from("/home/user/songs/samples/kick.wav"));
        // Path traversal -- `..` components should be stripped
        let traversal = resolve_relative(base, "../../etc/passwd");
        assert_eq!(traversal, std::path::PathBuf::from("/home/user/songs/etc/passwd"));
        // Absolute path -- should be reduced to just the filename under base
        let absolute = resolve_relative(base, "/etc/passwd");
        assert_eq!(absolute, std::path::PathBuf::from("/home/user/songs/passwd"));
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
        assert!(app.status_message.as_ref().unwrap().contains("Unknown command"));
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
        app.song.order.push(0);
        app.song.order.push(0);

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
        app.song.order.push(0);
        app.song.order.push(0);

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
        assert_eq!(app.song.order.len(), 1);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE));
        assert_eq!(app.song.order.len(), 2);
        assert_eq!(app.song.order[0], 0);
        assert_eq!(app.song.order[1], 0);
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.song.order.len(), 1);
        assert_eq!(app.matrix_cursor, 0);

        // Can't delete the last entry
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.song.order.len(), 1);
    }

    #[test]
    fn test_pattern_matrix_new_clone() {
        let mut app = make_app();
        assert_eq!(app.song.patterns.len(), 1);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.song.patterns.len(), 2);
        assert_eq!(app.song.order.len(), 2);
        assert_eq!(app.song.order[1], 1);
        assert_eq!(app.matrix_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.song.patterns.len(), 3);
        assert_eq!(app.song.order.len(), 3);
        assert_eq!(app.matrix_cursor, 2);
    }

    #[test]
    fn test_pattern_matrix_change_pattern() {
        let mut app = make_app();
        app.song.add_pattern();
        assert_eq!(app.song.order[0], 0);

        run_command(&mut app, "p");

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.song.order[0], 1);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.song.order[0], 0);

        // Can't go below 0
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.song.order[0], 0);
    }

    #[test]
    fn test_pattern_matrix_repeat() {
        let mut app = make_app();
        run_command(&mut app, "p");

        // Default repeat is 1
        assert_eq!(app.song.order_repeats[0], 1);

        // ] increases repeat
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 2);

        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 3);

        // [ decreases repeat
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 2);

        // Can go to 0 (skip)
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 0);

        // Can't go below 0
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 0);
    }

    #[test]
    fn test_order_repeats_sync_on_insert_delete() {
        let mut app = make_app();
        run_command(&mut app, "p");

        // Set repeat to 3
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats[0], 3);

        // Insert: new entry gets repeat=1
        app.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats.len(), 2);
        assert_eq!(app.song.order_repeats[0], 3); // original preserved
        assert_eq!(app.song.order_repeats[1], 1); // new entry

        // Delete: removes the entry's repeat too
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.song.order_repeats.len(), 1);
        assert_eq!(app.song.order_repeats[0], 3); // original still there
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
        app.channels[0].channel_type = ChannelType::Synth;
        run_command(&mut app, "fx");
        // Navigate to filter enabled (field 3 for Synth: Name, Type, Inst, Filter)
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Type
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 2=Instrument
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 3=Filter
        assert_eq!(app.ch_fx_field, 3);
        assert!(!app.channels[0].effects_params.filter_enabled);
        // Toggle with Right arrow
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.channels[0].effects_params.filter_enabled);
        // Toggle back
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(!app.channels[0].effects_params.filter_enabled);
    }

    #[test]
    fn test_track_config_adjust_cutoff() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Synth;
        run_command(&mut app, "fx");
        // Navigate to cutoff (field 4 for Synth: Name, Type, Inst, Filter, Cutoff)
        for _ in 0..4 { app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); }
        assert_eq!(app.ch_fx_field, 4);
        let initial = app.channels[0].effects_params.filter_cutoff;
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.channels[0].effects_params.filter_cutoff > initial);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!((app.channels[0].effects_params.filter_cutoff - initial).abs() < 0.01);
    }

    #[test]
    fn test_track_config_navigate_all_fields() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Synth;
        run_command(&mut app, "fx");
        // 20 fields total (name, type, instrument, filter x3, distortion x2, chorus x4, delay x4, reverb x4)
        for i in 0..20 {
            assert_eq!(app.ch_fx_field, i);
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.ch_fx_field, 0);
    }

    #[test]
    fn test_track_config_type_cycle() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        // Navigate to Type (field 1)
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.ch_fx_field, 1);
        assert_eq!(app.channels[0].channel_type, ChannelType::Midi);
        // Right arrow cycles type
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].channel_type, ChannelType::Synth);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].channel_type, ChannelType::Sample);
    }

    #[test]
    fn test_track_config_instrument_select() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Synth;
        run_command(&mut app, "fx");
        // Navigate to Instrument field (field 2)
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Type
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 2=Instrument
        assert_eq!(app.ch_fx_field, 2);
        assert_eq!(app.channels[0].default_instrument, None);
        // Right sets to 00
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(0));
        // Right again increments
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(1));
    }

    #[test]
    fn test_synth_track_auto_fills_instrument() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Synth;
        app.channels[0].default_instrument = Some(5);
        app.mode = Mode::Insert;
        // Enter a note (z = C in current octave)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(5));
    }

    #[test]
    fn test_sample_track_auto_fills_instrument() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Sample;
        app.channels[0].default_instrument = Some(3);
        app.mode = Mode::Insert;
        // Enter a note (z = C in current octave)
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_some());
        // Sample tracks should auto-fill instrument just like Synth tracks
        assert_eq!(cell.instrument, Some(3));
    }

    #[test]
    fn test_file_browser_open() {
        let mut app = make_app();
        app.open_file_browser(
            FileBrowserAction::LoadSample(0),
            vec!["wav".to_string(), "aiff".to_string()],
        );
        assert_eq!(app.mode, Mode::FileBrowser);
        assert_eq!(app.dialogs.file_browser.action, FileBrowserAction::LoadSample(0));
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
        let subdir_idx = app.dialogs.file_browser.entries.iter().position(|e| e.name == "subdir" && e.is_dir);
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

        let _ = std::fs::remove_dir_all(&dir);
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
        let names: Vec<&str> = app.dialogs.file_browser.entries.iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"sample.wav"));
        assert!(names.contains(&"beat.aiff"));
        assert!(!names.contains(&"notes.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_file_browser_select_file_loads_sample() {
        let mut app = make_app();
        // Ensure we have enough channels for slot 5
        while app.channels.len() <= 5 {
            app.channels.push(ChannelConfig::new(app.channels.len() as u8));
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
        app.open_file_browser(
            FileBrowserAction::LoadSample(5),
            vec!["wav".to_string()],
        );

        // Find the wav file and select it
        let wav_idx = app.dialogs.file_browser.entries.iter().position(|e| e.name == "kick.wav");
        assert!(wav_idx.is_some(), "WAV file should appear in browser");
        app.dialogs.file_browser.cursor = wav_idx.unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // Should have loaded the sample
        assert!(app.sample_bank.get(5).is_some());
        assert_eq!(app.sample_bank.get(5).unwrap().name, "kick");
        assert_eq!(app.instruments[5].sample_index, Some(5));
        // Loading a sample should auto-set default_instrument so preview routes correctly
        assert_eq!(app.channels[5].default_instrument, Some(5));
        assert_ne!(app.mode, Mode::FileBrowser);

        let _ = std::fs::remove_dir_all(&dir);
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
        if let Some(dir_idx) = app.dialogs.file_browser.entries.iter().position(|e| e.name == "aaa_dir") {
            if let Some(file_idx) = app.dialogs.file_browser.entries.iter().position(|e| e.name == "aaa_file.wav") {
                assert!(dir_idx < file_idx, "Directories should sort before files");
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_slice_sample_equal() {
        let mut app = make_app();
        // Load a sample into slot 0
        let sample = crate::sample::Sample {
            name: "kick".into(),
            data: vec![[0.5, 0.5]; 4000],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let mut bank = (*app.sample_bank).clone();
        bank.samples[0] = Some(sample);
        app.sample_bank = Arc::new(bank);
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_slice_count = 4;

        let result = app.slice_sample(false);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4);

        // Check that slices are in consecutive slots
        assert!(app.sample_bank.get(0).is_some());
        assert!(app.sample_bank.get(1).is_some());
        assert!(app.sample_bank.get(2).is_some());
        assert!(app.sample_bank.get(3).is_some());
        assert_eq!(app.sample_bank.get(0).unwrap().data.len(), 1000);
        assert_eq!(app.sample_bank.get(0).unwrap().name, "kick_S00");
        assert_eq!(app.sample_bank.get(3).unwrap().name, "kick_S03");

        // Check instruments are set up
        assert_eq!(app.instruments[0].sample_index, Some(0));
        assert_eq!(app.instruments[3].sample_index, Some(3));
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
        for i in 11025..13000 { data[i] = [0.8, 0.8]; }
        for i in 26460..28000 { data[i] = [0.9, 0.9]; }
        let sample = crate::sample::Sample {
            name: "breaks".into(),
            data,
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0, trim_end: 0,
            loop_enabled: false, loop_start: 0, loop_end: 0,
            source_path: None,
        };
        let mut bank = (*app.sample_bank).clone();
        bank.samples[0] = Some(sample);
        app.sample_bank = Arc::new(bank);
        app.dialogs.sample_editor_slot = 0;
        app.dialogs.sample_slice_sensitivity = 0.5;

        let result = app.slice_sample(true);
        assert!(result.is_ok());
        let count = result.unwrap();
        assert!(count >= 2, "Expected at least 2 transient slices, got {}", count);

        // All slices should exist in consecutive slots
        for i in 0..count {
            assert!(app.sample_bank.get(i).is_some(), "Slice {} missing", i);
        }
    }

    // -- Auto-save tests --

    #[test]
    fn test_autosave_only_when_dirty() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("test_autosave_dirty.rtrk");
        app.file_path = Some(tmp.clone());
        app.dirty = false;
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
        app.file_path = Some(tmp.clone());
        app.dirty = true;
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
        app.file_path = Some(tmp.clone());
        app.dirty = true;
        app.last_autosave = Instant::now() - std::time::Duration::from_secs(120);
        app.auto_save();
        let autosave = autosave_path_for(&tmp);
        assert!(autosave.exists());

        // Manual save should clean up autosave
        app.save();
        assert!(!autosave.exists(), "Auto-save should be cleaned up after manual save");
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
        assert_eq!(app.song.highlight_beat, 3);

        app.dialogs.settings_field = SettingsField::HighlightBar;
        app.dialogs.settings_edit_buf = "12".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.highlight_bar, 12);
    }

    // -- Swing tests --

    #[test]
    fn test_swing_default() {
        let song = Song::new(4, 64);
        assert_eq!(song.swing, 50);
    }

    #[test]
    fn test_swing_seconds_per_tick() {
        let mut song = Song::new(4, 64);
        let base = song.seconds_per_tick();

        // No swing: even and odd rows should be equal
        assert!((song.swing_seconds_per_tick(0) - base).abs() < 1e-12);
        assert!((song.swing_seconds_per_tick(1) - base).abs() < 1e-12);

        // 67% swing: even rows longer, odd rows shorter
        song.swing = 67;
        let even = song.swing_seconds_per_tick(0);
        let odd = song.swing_seconds_per_tick(1);
        assert!(even > base, "Even row should be longer with swing > 50");
        assert!(odd < base, "Odd row should be shorter with swing > 50");
        // Total of a pair should equal 2 * base
        assert!(((even + odd) - 2.0 * base).abs() < 1e-12);
    }

    #[test]
    fn test_swing_settings() {
        let mut app = make_app();
        app.open_song_settings();
        app.dialogs.settings_field = SettingsField::Swing;
        app.dialogs.settings_edit_buf = "67".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.swing, 67);

        // Clamp to 100
        app.dialogs.settings_edit_buf = "150".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.swing, 100);
    }

    // -- Tempo automation tests --

    #[test]
    fn test_tempo_map_lookup() {
        let mut song = Song::new(4, 64);
        song.tempo_map.push(crate::tracker::TempoPoint { order: 0, row: 16, bpm: 140.0 });
        song.tempo_map.push(crate::tracker::TempoPoint { order: 1, row: 0, bpm: 160.0 });

        assert_eq!(song.tempo_at(0, 0), None);
        assert_eq!(song.tempo_at(0, 16), Some(140.0));
        assert_eq!(song.tempo_at(1, 0), Some(160.0));
        assert_eq!(song.tempo_at(1, 1), None);
    }

    #[test]
    fn test_tempo_map_serialization() {
        let mut song = Song::new(4, 16);
        song.tempo_map.push(crate::tracker::TempoPoint { order: 0, row: 8, bpm: 150.5 });
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
        let pb = app.channel_pitch_bend_per_semitone(0);
        let expected = (PITCH_BEND_CENTER as f64) / DEFAULT_PITCH_BEND_RANGE;
        assert!((pb - expected).abs() < 1e-9);
    }

    #[test]
    fn test_pitch_bend_range_custom() {
        let mut app = make_app();
        // Set instrument 0 with custom pitch bend range of 12 semitones
        app.instruments[0].pitch_bend_range = Some(12.0);
        app.engine.channel_states[0].active_instrument = Some(0);

        let pb = app.channel_pitch_bend_per_semitone(0);
        let expected = (PITCH_BEND_CENTER as f64) / 12.0;
        assert!((pb - expected).abs() < 1e-9);
    }

    #[test]
    fn test_pitch_bend_range_serialization() {
        use crate::tracker::{InstrumentDef, InstrumentEntry, SongFile};
        let song = Song::new(1, 16);
        let song_file = SongFile {
            song,
            instruments: vec![InstrumentEntry {
                slot: 0,
                def: InstrumentDef {
                    name: "Test".to_string(),
                    midi_program: None,
                    sample_index: None,
                    synth_params: None,
                    pitch_bend_range: Some(7.0),
                },
            }],
            sample_refs: vec![],
        };
        let json = serde_json::to_string(&song_file).unwrap();
        let loaded: SongFile = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.instruments[0].def.pitch_bend_range, Some(7.0));
    }

    // -- Link beat timeline test --

    #[test]
    fn test_link_beat_at_time() {
        let mut engine = crate::link::LinkEngine::new(120.0);
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
        assert_eq!(song.swing, 50);
        assert!(song.tempo_map.is_empty());
    }

    #[test]
    fn test_instrument_def_backwards_compat() {
        // Old format without pitch_bend_range
        let json = r#"{"name": "Test", "sample_index": 0}"#;
        let def: crate::tracker::InstrumentDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.pitch_bend_range, None);
    }

    #[test]
    fn test_track_config_sample_select_cycles_loaded() {
        let mut app = make_app();
        // Load two samples into the bank
        let mut bank = (*app.sample_bank).clone();
        bank.samples[2] = Some(crate::sample::Sample {
            name: "kick".to_string(),
            data: vec![[0.0; 2]; 100],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        });
        bank.samples[5] = Some(crate::sample::Sample {
            name: "snare".to_string(),
            data: vec![[0.0; 2]; 100],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        });
        app.sample_bank = std::sync::Arc::new(bank);
        app.channels[0].channel_type = ChannelType::Sample;

        // Open track config and navigate to sample field
        run_command(&mut app, "fx");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 1=Type
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // 2=Sample
        assert_eq!(app.ch_fx_field, 2);

        // Right arrow selects first loaded sample (slot 2)
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(2));

        // Right again cycles to slot 5
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(5));

        // Right again wraps to slot 2
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(2));

        // Left goes back to slot 5
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.channels[0].default_instrument, Some(5));
    }

    #[test]
    fn test_track_config_sample_select_no_samples_opens_browser() {
        let mut app = make_app();
        app.channels[0].channel_type = ChannelType::Sample;

        // Open track config and navigate to sample field
        run_command(&mut app, "fx");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        // Right arrow with no samples loaded should open file browser
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::FileBrowser);
    }

    #[test]
    fn test_sample_bank_loaded_slots() {
        let mut bank = crate::sample::SampleBank::new();
        assert!(bank.loaded_slots().is_empty());

        bank.samples[3] = Some(crate::sample::Sample {
            name: "test".to_string(),
            data: vec![[0.0; 2]; 10],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        });
        bank.samples[7] = Some(crate::sample::Sample {
            name: "test2".to_string(),
            data: vec![[0.0; 2]; 10],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        });

        let slots = bank.loaded_slots();
        assert_eq!(slots, vec![3, 7]);
    }
}
