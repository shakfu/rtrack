mod playback;
mod input;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::AudioEngine;
use crate::link::LinkEngine;
use crate::midi::{MidiEngine, MidiInputEngine};
use crate::sample::SampleBank;
use crate::tracker::{Song, SongFile, InstrumentDef, InstrumentEntry, SampleRef, SampleRefEntry};
use crate::ui::pattern_editor::SubColumn;

// -- Constants --

// Effect commands (single hex digit, stored in Cell.effect)
pub(crate) const EFFECT_ARPEGGIO: u8 = 0x0;      // 0xy: cycle note, note+x, note+y
pub(crate) const EFFECT_PORTA_UP: u8 = 0x1;      // 1xx: slide pitch up by xx per tick
pub(crate) const EFFECT_PORTA_DOWN: u8 = 0x2;    // 2xx: slide pitch down by xx per tick
pub(crate) const EFFECT_TONE_PORTA: u8 = 0x3;    // 3xx: slide toward target note at speed xx
pub(crate) const EFFECT_VIBRATO: u8 = 0x4;       // 4xy: vibrato speed x, depth y
pub(crate) const EFFECT_VOLUME_SLIDE: u8 = 0x5;  // 5xy: volume slide up x, down y per tick
pub(crate) const EFFECT_NOTE_DELAY: u8 = 0x6;    // 6xx: delay note trigger by xx ticks
pub(crate) const EFFECT_POSITION_JUMP: u8 = 0xB; // Bxx: jump to order position xx
pub(crate) const EFFECT_MIDI_CC: u8 = 0xC;       // Cxx: send MIDI CC (controller from instrument col, value xx)
pub(crate) const EFFECT_PATTERN_BREAK: u8 = 0xD; // Dxx: break to row xx of next pattern
pub(crate) const EFFECT_PROGRAM_CHANGE: u8 = 0xE; // Exx: program change to program xx
pub(crate) const EFFECT_SET_SPEED: u8 = 0xF;     // Fxx: xx<0x20 = set speed, xx>=0x20 = set BPM

/// Pitch bend center (no bend) = 0x2000 = 8192
pub(crate) const PITCH_BEND_CENTER: u16 = 0x2000;
/// Pitch bend range in semitones (standard MIDI default = 2)
const PITCH_BEND_RANGE: f64 = 2.0;
/// Pitch bend units per semitone
pub(crate) const PITCH_BEND_PER_SEMITONE: f64 = (PITCH_BEND_CENTER as f64) / PITCH_BEND_RANGE;

/// Number of channels displayed per track page
pub(crate) const CHANNELS_PER_PAGE: usize = 4;
/// Maximum undo history depth
const MAX_UNDO_HISTORY: usize = 100;
/// Maximum number of instruments
pub(crate) const MAX_INSTRUMENTS: usize = 256;
/// Preview note auto-off timeout in milliseconds
const PREVIEW_NOTE_TIMEOUT_MS: u64 = 250;
/// Maximum number of tracker channels
pub(crate) const MAX_CHANNELS: usize = 16;

/// Per-channel state for continuous effects (arpeggio, portamento, vibrato, volume slide)
#[derive(Debug, Clone)]
pub struct ChannelState {
    /// Last triggered MIDI note on this channel
    pub note: Option<u8>,
    /// Current volume (0-127)
    pub volume: u8,
    /// Accumulated pitch offset in semitones (for portamento)
    pub pitch_offset: f64,
    /// Target note for tone portamento
    pub porta_target: Option<u8>,
    /// Vibrato phase (0.0..1.0)
    pub vibrato_phase: f64,
    /// Current active effect
    pub effect: Option<u8>,
    /// Current effect parameter
    pub effect_param: u8,
    /// Delayed note: (midi_note, velocity, is_off)
    pub delayed_note: Option<(u8, u8, bool)>,
    /// Tick on which to trigger delayed note
    pub delay_tick: u8,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            note: None,
            volume: 100,
            pitch_offset: 0.0,
            porta_target: None,
            vibrato_phase: 0.0,
            effect: None,
            effect_param: 0,
            delayed_note: None,
            delay_tick: 0,
        }
    }
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
    ChannelRename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Title,
    Bpm,
    Speed,
    Channels,
    Rows,
}

impl SettingsField {
    pub fn next(self) -> Self {
        match self {
            Self::Title => Self::Bpm,
            Self::Bpm => Self::Speed,
            Self::Speed => Self::Channels,
            Self::Channels => Self::Rows,
            Self::Rows => Self::Title,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Title => Self::Rows,
            Self::Bpm => Self::Title,
            Self::Speed => Self::Bpm,
            Self::Channels => Self::Speed,
            Self::Rows => Self::Channels,
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
}

impl SampleField {
    pub fn next(self) -> Self {
        match self {
            Self::BaseNote => Self::TrimStart,
            Self::TrimStart => Self::TrimEnd,
            Self::TrimEnd => Self::LoopEnabled,
            Self::LoopEnabled => Self::LoopStart,
            Self::LoopStart => Self::LoopEnd,
            Self::LoopEnd => Self::BaseNote,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::BaseNote => Self::LoopEnd,
            Self::TrimStart => Self::BaseNote,
            Self::TrimEnd => Self::TrimStart,
            Self::LoopEnabled => Self::TrimEnd,
            Self::LoopStart => Self::LoopEnabled,
            Self::LoopEnd => Self::LoopStart,
        }
    }
}

pub struct Instrument {
    pub name: String,
    pub midi_program: Option<u8>,
    pub sample_index: Option<usize>,
    pub synth_params: Option<crate::audio::synth::SynthParams>,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            name: String::new(),
            midi_program: None,
            sample_index: None,
            synth_params: None,
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
    FilterCutoff,
    FilterResonance,
    FilterEnv,
    Detune,
}

impl SynthField {
    pub fn next(self) -> Self {
        match self {
            Self::Waveform => Self::Attack,
            Self::Attack => Self::Decay,
            Self::Decay => Self::Sustain,
            Self::Sustain => Self::Release,
            Self::Release => Self::FilterCutoff,
            Self::FilterCutoff => Self::FilterResonance,
            Self::FilterResonance => Self::FilterEnv,
            Self::FilterEnv => Self::Detune,
            Self::Detune => Self::Waveform,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Waveform => Self::Detune,
            Self::Attack => Self::Waveform,
            Self::Decay => Self::Attack,
            Self::Sustain => Self::Decay,
            Self::Release => Self::Sustain,
            Self::FilterCutoff => Self::Release,
            Self::FilterResonance => Self::FilterCutoff,
            Self::FilterEnv => Self::FilterResonance,
            Self::Detune => Self::FilterEnv,
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
    // Fields: playing, playback_row, playback_order, playback_generation,
    //         last_tick, tick_accumulator, playback_tick,
    //         clock_tick_accumulator, channel_states
    // -----------------------------------------------------------------------
    pub playing: bool,
    pub playback_row: usize,
    pub playback_order: usize,
    /// Incremented each time playback wraps past the end of the order list
    pub playback_generation: u32,
    pub(crate) last_tick: Option<Instant>,
    pub(crate) tick_accumulator: f64,
    /// Current sub-tick within a row (0..speed-1). Tick 0 = new row, ticks 1+ = effect processing.
    pub(crate) playback_tick: u8,
    /// MIDI clock tick accumulator
    pub(crate) clock_tick_accumulator: f64,
    /// Elapsed playback time in seconds
    pub playback_elapsed: f64,
    /// Per-channel effect state
    pub(crate) channel_states: Vec<ChannelState>,

    // -----------------------------------------------------------------------
    // Editor State
    // Fields: dirty, clipboard, block_anchor, block_clipboard,
    //         undo_stack, redo_stack, rename_buf
    // -----------------------------------------------------------------------
    /// Dirty flag: set when song is modified, cleared on save/load
    pub dirty: bool,
    /// Single-row clipboard
    pub clipboard: Option<Vec<crate::tracker::Cell>>,
    /// Block selection: anchor point (row, channel) when selection is active
    pub block_anchor: Option<(usize, usize)>,
    /// Block clipboard: 2D grid of cells (rows x channels)
    pub block_clipboard: Option<Vec<Vec<crate::tracker::Cell>>>,
    /// Undo stack
    pub(crate) undo_stack: VecDeque<Song>,
    /// Redo stack
    pub(crate) redo_stack: Vec<Song>,
    /// Channel rename edit buffer
    pub rename_buf: String,

    // -----------------------------------------------------------------------
    // Dialog State
    // Fields: settings_field, settings_edit_buf, instrument_cursor,
    //         sample_editor_slot, sample_editor_field, synth_editor_slot,
    //         synth_editor_field, midi_port_list, midi_port_cursor, help_scroll
    // -----------------------------------------------------------------------
    pub settings_field: SettingsField,
    pub settings_edit_buf: String,
    pub instrument_cursor: usize,
    pub sample_editor_slot: usize,
    pub sample_editor_field: SampleField,
    pub synth_editor_slot: usize,
    pub synth_editor_field: SynthField,
    pub midi_port_list: Vec<String>,
    pub midi_port_cursor: usize,
    pub help_scroll: usize,

    // -----------------------------------------------------------------------
    // Other state (file, audio, instruments, channels)
    // -----------------------------------------------------------------------
    pub file_path: Option<PathBuf>,
    pub status_message: Option<String>,
    pub edit_order: usize,
    pub muted_channels: Vec<bool>,
    pub solo_channel: Option<usize>,
    pub midi_channel_map: Vec<u8>,
    /// The mode to return to after closing the port selector
    pub(crate) prev_mode: Mode,
    pub instruments: Vec<Instrument>,
    pub theme_index: usize,
    pub audio: Option<AudioEngine>,
    pub sample_bank: Arc<SampleBank>,
    /// Preview note: (channel, note, timestamp) -- auto note-off after timeout
    pub(crate) preview_note: Option<(u8, u8, Instant)>,
    pub channel_names: Vec<String>,
    /// Per-channel volume (0.0..1.0, default 1.0)
    pub channel_volumes: Vec<f32>,
    /// Per-channel pan (-1.0=left, 0.0=center, 1.0=right)
    pub channel_pans: Vec<f32>,
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
            playback_row: 0,
            playback_order: 0,
            playback_generation: 0,
            last_tick: None,
            tick_accumulator: 0.0,
            edit_step: 1,
            file_path: None,
            status_message: None,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            edit_order: 0,
            muted_channels: vec![false; 4],
            solo_channel: None,
            midi_channel_map: (0..4).map(|i| i as u8).collect(),
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            prev_mode: Mode::Normal,
            settings_field: SettingsField::Title,
            settings_edit_buf: String::new(),
            instruments: (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect(),
            instrument_cursor: 0,
            theme_index: 0,
            clock_tick_accumulator: 0.0,
            playback_elapsed: 0.0,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            synth_editor_slot: 0,
            synth_editor_field: SynthField::Waveform,
            track_page: 0,
            preview_note: None,
            playback_tick: 0,
            channel_states: vec![ChannelState::default(); 4],
            help_scroll: 0,
            dirty: false,
            block_anchor: None,
            block_clipboard: None,
            follow_playback: true,
            channel_names: vec![String::new(); 4],
            rename_buf: String::new(),
            channel_volumes: vec![1.0; 4],
            channel_pans: vec![0.0; 4],
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
    pub fn toggle_audio_effects(&self) -> bool {
        self.audio.as_ref().map_or(false, |a| a.toggle_effects())
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
                if let Some(ref audio) = self.audio {
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
                if let Some(ref audio) = self.audio {
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
        let slot = self.instrument_cursor;
        self.synth_editor_slot = slot;
        self.synth_editor_field = SynthField::Waveform;
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
        self.sample_editor_slot = self.instrument_cursor;
        self.sample_editor_field = SampleField::BaseNote;
        self.prev_mode = self.mode;
        self.mode = Mode::SampleEditor;
    }

    /// Export the song to a WAV file
    #[allow(dead_code)]
    pub fn export_wav(&self, path: std::path::PathBuf) {
        let instruments: Vec<crate::sample::export::ExportInstrument> = self.instruments.iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect();
        let sample_rate = self.audio.as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100);
        match crate::sample::export::render_to_wav(
            &path, &self.song, &self.sample_bank, &instruments, sample_rate,
        ) {
            Ok(()) => {
                // status_message is not &mut self here; caller should set it
            }
            Err(_e) => {}
        }
    }

    // -- Sound output helpers (dispatch to MIDI + optional audio engine) --

    pub(crate) fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref audio) = self.audio {
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
                if let Some(ref audio) = self.audio {
                    audio.sample_note_on(sid, note, velocity, channel);
                }
                return;
            }
        }

        // Route 2: custom synth params
        if let Some(ref params) = inst.and_then(|i| i.synth_params.as_ref()) {
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref audio) = self.audio {
                audio.note_on_with_params(channel, note, velocity, params);
            }
            return;
        }

        // Route 3: default synth (channel program)
        self.send_note_on(channel, note, velocity);
    }

    pub(crate) fn send_channel_note_off(&mut self, channel: u8) {
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref audio) = self.audio {
            audio.note_off_all_channel(channel);
            audio.sample_note_off_channel(channel);
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
        if let Some(ref audio) = self.audio {
            audio.note_off_all();
            audio.sample_note_off_all();
        }
    }

    pub(crate) fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        let _ = self.midi.send_cc(channel, controller, value);
        if let Some(ref audio) = self.audio {
            audio.send_cc(channel, controller, value);
        }
    }

    pub(crate) fn send_program_change(&mut self, channel: u8, program: u8) {
        let _ = self.midi.program_change(channel, program);
        if let Some(ref audio) = self.audio {
            audio.program_change(channel, program);
        }
    }

    pub(crate) fn send_pitch_bend(&mut self, channel: u8, value: u16) {
        let _ = self.midi.pitch_bend(channel, value);
        if let Some(ref audio) = self.audio {
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

        self.midi_port_list = ports;
        self.midi_port_cursor = 0;
        self.prev_mode = self.mode;
        self.mode = Mode::MidiPortSelect;
    }

    pub(crate) fn close_port_selector(&mut self) {
        self.mode = self.prev_mode;
    }

    pub(crate) fn select_midi_port(&mut self) {
        if self.midi_port_cursor >= self.midi_port_list.len() {
            return;
        }

        let _selected = &self.midi_port_list[self.midi_port_cursor];

        // Index 0 on unix is the virtual port
        #[cfg(unix)]
        {
            if self.midi_port_cursor == 0 {
                let _ = self.midi.create_virtual_port();
                self.close_port_selector();
                return;
            }
            // Hardware ports start at index 1 in our list, but index 0 in midir
            let hw_index = self.midi_port_cursor - 1;
            let _ = self.midi.connect(hw_index);
        }

        #[cfg(not(unix))]
        {
            let _ = self.midi.connect(self.midi_port_cursor);
        }

        self.close_port_selector();
    }

    pub fn current_order_position(&self) -> usize {
        if self.playing {
            self.playback_order
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
        self.edit_order += 1;
        self.cursor_row = 0;
        self.status_message = Some(format!("Cloned pattern {:02X} -> {:02X}", src_idx, new_idx));
    }

    pub fn insert_order_entry(&mut self) {
        self.push_undo();
        let current_pattern = self.song.order[self.edit_order];
        self.song.order.insert(self.edit_order + 1, current_pattern);
        self.edit_order += 1;
        self.status_message = Some(format!("Inserted order entry {:02X}", self.edit_order));
    }

    pub fn remove_order_entry(&mut self) {
        if self.song.order.len() <= 1 {
            self.status_message = Some("Cannot remove last order entry".to_string());
            return;
        }
        self.push_undo();
        self.song.order.remove(self.edit_order);
        if self.edit_order >= self.song.order.len() {
            self.edit_order = self.song.order.len() - 1;
        }
        self.cursor_row = 0;
        self.status_message = Some("Removed order entry".to_string());
    }

    pub fn midi_channel_for(&self, tracker_channel: usize) -> u8 {
        let ch = self.midi_channel_map.get(tracker_channel).copied().unwrap_or(tracker_channel as u8);
        ch & 0x0F // clamp to valid MIDI channel range 0-15
    }

    pub fn is_channel_audible(&self, channel: usize) -> bool {
        if let Some(solo) = self.solo_channel {
            return channel == solo;
        }
        if channel < self.muted_channels.len() && self.muted_channels[channel] {
            return false;
        }
        true
    }

    pub fn toggle_channel_mute(&mut self, channel: usize) {
        if channel < self.muted_channels.len() {
            self.solo_channel = None;
            self.muted_channels[channel] = !self.muted_channels[channel];
            let state = if self.muted_channels[channel] { "muted" } else { "unmuted" };
            self.status_message = Some(format!("Ch {} {}", channel + 1, state));
            if self.muted_channels[channel] {
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
            for ch in 0..self.muted_channels.len() {
                if ch != channel {
                    let midi_ch = self.midi_channel_for(ch);
                    self.send_channel_note_off(midi_ch);
                }
            }
        }
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
                self.status_message = Some(format!("Saved: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
            }
        }
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
                self.muted_channels = vec![false; song.channels];
                self.solo_channel = None;
                self.channel_names = vec![String::new(); song.channels];
                self.channel_volumes = vec![1.0; song.channels];
                self.channel_pans = vec![0.0; song.channels];
                self.midi_channel_map = (0..song.channels).map(|i| i as u8).collect();
                self.song = song;
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.track_page = 0;
                self.undo_stack.clear();
                self.redo_stack.clear();

                // Restore instruments
                for entry in &song_file.instruments {
                    if entry.slot < self.instruments.len() {
                        self.instruments[entry.slot].name = entry.def.name.clone();
                        self.instruments[entry.slot].midi_program = entry.def.midi_program;
                        self.instruments[entry.slot].sample_index = entry.def.sample_index;
                        self.instruments[entry.slot].synth_params = entry.def.synth_params.clone();
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
                if let Some(ref audio) = self.audio {
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
        self.undo_stack.push_back(self.song.clone());
        self.redo_stack.clear();
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.pop_front();
        }
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop_back() {
            self.redo_stack.push(self.song.clone());
            self.song = prev;
            self.status_message = Some("Undo".to_string());
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push_back(self.song.clone());
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
        self.clipboard = Some(row);
        self.status_message = Some(format!("Copied row {:02X}", self.cursor_row));
    }

    pub fn paste_row(&mut self) {
        if let Some(ref row) = self.clipboard.clone() {
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
        let instruments: Vec<crate::sample::export::ExportInstrument> = self.instruments.iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect();
        let sample_rate = self.audio.as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100);
        match crate::sample::export::render_to_wav(
            &path, &self.song, &self.sample_bank, &instruments, sample_rate,
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
        let instruments: Vec<crate::sample::export::ExportInstrument> = self.instruments.iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect();
        let sample_rate = self.audio.as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100);
        match crate::sample::export::render_to_flac(
            &path, &self.song, &self.sample_bank, &instruments, sample_rate,
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
                self.muted_channels = vec![false; song.channels];
                self.solo_channel = None;
                self.channel_names = vec![String::new(); song.channels];
                self.channel_volumes = vec![1.0; song.channels];
                self.channel_pans = vec![0.0; song.channels];
                self.midi_channel_map = (0..song.channels).map(|i| i as u8).collect();
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
        App {
            song: Song::new(4, 64),
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
            playback_row: 0,
            playback_order: 0,
            playback_generation: 0,
            last_tick: None,
            tick_accumulator: 0.0,
            playback_tick: 0,
            channel_states: vec![ChannelState::default(); 4],
            edit_step: 1,
            file_path: None,
            status_message: None,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            edit_order: 0,
            muted_channels: vec![false; 4],
            solo_channel: None,
            midi_channel_map: vec![0, 1, 2, 3],
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            prev_mode: Mode::Normal,
            settings_field: SettingsField::Title,
            settings_edit_buf: String::new(),
            instruments: (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect(),
            instrument_cursor: 0,
            theme_index: 0,
            clock_tick_accumulator: 0.0,
            playback_elapsed: 0.0,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            synth_editor_slot: 0,
            synth_editor_field: SynthField::Waveform,
            track_page: 0,
            preview_note: None,
            help_scroll: 0,
            dirty: false,
            block_anchor: None,
            block_clipboard: None,
            follow_playback: true,
            channel_names: vec![String::new(); 4],
            rename_buf: String::new(),
            channel_volumes: vec![1.0; 4],
            channel_pans: vec![0.0; 4],
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
    fn test_tab_toggles_track_page() {
        let mut app = make_app();
        // 4 channels, 1 page => Tab should not change page
        assert_eq!(app.track_page, 0);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.track_page, 0);

        // 8 channels => should have 2 pages
        app.song.channels = 8;
        for pat in &mut app.song.patterns {
            pat.channels = 8;
            for row in &mut pat.data {
                row.resize(8, crate::tracker::Cell::default());
            }
        }
        app.muted_channels = vec![false; 8];
        app.midi_channel_map = (0..8).map(|i| i as u8).collect();

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.track_page, 1);

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.track_page, 0);
    }

    #[test]
    fn test_ctrl_number_selects_track() {
        let mut app = make_app();
        // 8 channels
        app.song.channels = 8;
        for pat in &mut app.song.patterns {
            pat.channels = 8;
            for row in &mut pat.data {
                row.resize(8, crate::tracker::Cell::default());
            }
        }
        app.muted_channels = vec![false; 8];
        app.midi_channel_map = (0..8).map(|i| i as u8).collect();

        // Ctrl+5 selects track 4 (0-indexed)
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert_eq!(app.cursor_channel, 4);
        assert_eq!(app.track_page, 1); // channels 4-7 are page 1
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
        app.midi_port_list = vec![
            "Port A".to_string(),
            "Port B".to_string(),
            "Port C".to_string(),
        ];
        app.midi_port_cursor = 0;
        app.mode = Mode::MidiPortSelect;

        // Down
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.midi_port_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.midi_port_cursor, 2);

        // Can't go past end
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.midi_port_cursor, 2);

        // Up
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.midi_port_cursor, 1);
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
        assert!(!app.redo_stack.is_empty());

        // New edit should clear redo
        app.push_undo();
        assert!(app.redo_stack.is_empty());
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
        assert!(app.clipboard.is_some());

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
        assert!(app.clipboard.is_some());
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
    fn test_insert_remove_order_entry() {
        let mut app = make_app();
        assert_eq!(app.song.order.len(), 1);

        app.insert_order_entry();
        assert_eq!(app.song.order.len(), 2);

        app.remove_order_entry();
        assert_eq!(app.song.order.len(), 1);

        // Can't remove last entry
        app.remove_order_entry();
        assert_eq!(app.song.order.len(), 1);
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

        // Start playback
        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        // Advance one row -- should trigger pattern break
        app.advance_playback();
        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 8);
    }

    #[test]
    fn test_position_jump_effect() {
        let mut app = make_app();
        // Need 3 patterns in order
        let pat2 = app.song.add_pattern();
        let pat3 = app.song.add_pattern();
        app.song.order.push(pat2);
        app.song.order.push(pat3);

        // Set position jump (B02) at row 0 of pattern 0
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_POSITION_JUMP);
        cell.effect_value = Some(2);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
        assert_eq!(app.playback_order, 2);
        assert_eq!(app.playback_row, 0);
    }

    #[test]
    fn test_position_jump_with_break() {
        let mut app = make_app();
        let pat2 = app.song.add_pattern();
        let pat3 = app.song.add_pattern();
        app.song.order.push(pat2);
        app.song.order.push(pat3);

        // Channel 0: position jump to order 2
        let pat_idx = app.song.order[0];
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 0);
            cell.effect = Some(EFFECT_POSITION_JUMP);
            cell.effect_value = Some(2);
        }
        // Channel 1: pattern break to row 4
        {
            let cell = app.song.patterns[pat_idx].get_mut(0, 1);
            cell.effect = Some(EFFECT_PATTERN_BREAK);
            cell.effect_value = Some(4);
        }

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
        assert_eq!(app.playback_order, 2);
        assert_eq!(app.playback_row, 4);
    }

    #[test]
    fn test_pattern_break_wraps_order() {
        let mut app = make_app();
        // Single pattern in order
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PATTERN_BREAK);
        cell.effect_value = Some(0);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
        // Should wrap back to order 0
        assert_eq!(app.playback_order, 0);
        assert_eq!(app.playback_row, 0);
        assert_eq!(app.playback_generation, 1);
    }

    #[test]
    fn test_position_jump_clamps_to_max() {
        let mut app = make_app();
        // Jump to order 99, but we only have 1 entry
        let pat_idx = app.song.order[0];
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_POSITION_JUMP);
        cell.effect_value = Some(99);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
        assert_eq!(app.playback_order, 0); // clamped to max
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
        app.playback_row = 3; // last row

        // Advance past end -> should wrap
        app.advance_playback();
        assert_eq!(app.playback_row, 0);
    }

    #[test]
    fn test_midi_cc_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        // Set CC effect: C (controller=01, value=40)
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_MIDI_CC);
        cell.instrument = Some(1);
        cell.effect_value = Some(0x40);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        // advance_playback processes the CC on tick 0
        app.advance_playback();
        // If we got here without panicking, the CC was sent (we can't easily check MIDI output)
    }

    #[test]
    fn test_program_change_effect_in_playback() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_PROGRAM_CHANGE);
        cell.effect_value = Some(5);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
    }

    #[test]
    fn test_arpeggio_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        // Place note C-4 with arpeggio 037 (minor chord)
        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_ARPEGGIO);
        cell.effect_value = Some(0x37);

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;
        app.playback_tick = 0;

        // Tick 0: advance_playback triggers note and sets up arpeggio state
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, Some(48)); // C-4

        // Tick 1: process_effects_tick should send pitch bend for +3
        app.playback_tick = 1;
        app.process_effects_tick();
    }

    #[test]
    fn test_portamento_up_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_PORTA_UP);
        cell.effect_value = Some(0x10); // 1 semitone per tick

        app.play();
        app.playback_row = 0;
        app.playback_order = 0;
        app.playback_tick = 0;

        app.advance_playback();
        assert_eq!(app.channel_states[0].pitch_offset, 0.0);

        // Tick 1: pitch should increase
        app.playback_tick = 1;
        app.process_effects_tick();
        assert!(app.channel_states[0].pitch_offset > 0.0);
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
        app.playback_tick = 0;
        app.advance_playback();

        app.playback_tick = 1;
        app.process_effects_tick();
        assert!(app.channel_states[0].pitch_offset < 0.0);
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
        app.playback_tick = 0;

        // Row 0: triggers C-4
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, Some(48)); // C-4

        // Row 1: sets target to E-4 (52) but keeps C-4 playing
        app.playback_tick = 0;
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, Some(48)); // still C-4
        assert_eq!(app.channel_states[0].porta_target, Some(52));

        // Tick 1: pitch should start sliding up
        app.playback_tick = 1;
        app.process_effects_tick();
        assert!(app.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_vibrato_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VIBRATO);
        cell.effect_value = Some(0x42); // speed=4, depth=2

        app.play();
        app.playback_tick = 0;
        app.advance_playback();

        // Tick 1: vibrato should modulate
        app.playback_tick = 1;
        app.process_effects_tick();
        assert!(app.channel_states[0].vibrato_phase > 0.0);
    }

    #[test]
    fn test_volume_slide_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x02); // slide down by 2

        app.play();
        app.playback_tick = 0;
        app.advance_playback();
        assert_eq!(app.channel_states[0].volume, 100);

        app.playback_tick = 1;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].volume, 98);
    }

    #[test]
    fn test_volume_slide_up() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(100);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x30); // slide up by 3

        app.play();
        app.playback_tick = 0;
        app.advance_playback();

        app.playback_tick = 1;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].volume, 103);
    }

    #[test]
    fn test_volume_slide_clamps() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.note = Some(Note::On { value: crate::tracker::NoteValue::C, octave: 4 });
        cell.volume = Some(5);
        cell.effect = Some(EFFECT_VOLUME_SLIDE);
        cell.effect_value = Some(0x0F); // slide down by 15

        app.play();
        app.playback_tick = 0;
        app.advance_playback();

        app.playback_tick = 1;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].volume, 0); // clamped, not wrapped
    }

    #[test]
    fn test_set_speed_effect() {
        let mut app = make_app();
        let pat_idx = app.song.order[0];

        let cell = app.song.patterns[pat_idx].get_mut(0, 0);
        cell.effect = Some(EFFECT_SET_SPEED);
        cell.effect_value = Some(3);

        app.play();
        app.playback_tick = 0;
        app.advance_playback();
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
        app.playback_tick = 0;
        app.advance_playback();
        assert_eq!(app.song.bpm, 0x80);
    }

    #[test]
    fn test_sub_tick_timing() {
        let mut app = make_app();
        app.song.speed = 6;

        app.play();

        // Tick 0
        app.process_tick();
        assert_eq!(app.playback_tick, 1);

        // Ticks 1-5
        for _ in 0..5 {
            app.process_tick();
        }
        // After tick 5, should reset to 0
        assert_eq!(app.playback_tick, 0);
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
        app.playback_tick = 0;

        // Tick 0: note should be deferred, not triggered
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, None); // not triggered yet
        assert!(app.channel_states[0].delayed_note.is_some());

        // Tick 1: still waiting
        app.playback_tick = 1;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].note, None);

        // Tick 2: still waiting
        app.playback_tick = 2;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].note, None);

        // Tick 3: should trigger
        app.playback_tick = 3;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].note, Some(48));
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

        // Row 0: trigger the note
        app.playback_tick = 0;
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, Some(48));

        // Row 1 tick 0: note-off should be deferred
        app.playback_tick = 0;
        app.advance_playback();
        assert_eq!(app.channel_states[0].note, Some(48)); // still active
        assert!(app.channel_states[0].delayed_note.is_some());

        // Tick 1: waiting
        app.playback_tick = 1;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].note, Some(48));

        // Tick 2: note-off triggers
        app.playback_tick = 2;
        app.process_effects_tick();
        assert_eq!(app.channel_states[0].note, None);
    }

    #[test]
    fn test_midi_input_in_insert_mode() {
        use crate::midi::MidiInputEvent;

        let mut app = make_app();
        app.mode = Mode::Insert;

        // Simulate MIDI note C-4 (note 60)
        app.handle_midi_input(MidiInputEvent { channel: 0, note: 60, velocity: 100 });

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

        app.handle_midi_input(MidiInputEvent { channel: 0, note: 60, velocity: 100 });

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

        app.handle_midi_input(MidiInputEvent { channel: 0, note: 60, velocity: 100 });

        let pattern_idx = app.song.order[0];
        let cell = app.song.patterns[pattern_idx].get(0, 0);
        assert!(cell.note.is_none());
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
        app.playback_row = 0;
        app.playback_order = 0;

        app.advance_playback();
        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 15); // clamped to max row
    }

    #[test]
    fn test_song_settings_open_close() {
        let mut app = make_app();
        app.open_song_settings();
        assert_eq!(app.mode, Mode::SongSettings);
        assert_eq!(app.settings_field, SettingsField::Title);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_song_settings_edit_title() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_edit_buf = "New Title".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.song.title, "New Title");
    }

    #[test]
    fn test_song_settings_edit_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_field = SettingsField::Bpm;
        app.settings_edit_buf = "140".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.song.bpm, 140);
    }

    #[test]
    fn test_song_settings_edit_channels() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_field = SettingsField::Channels;
        app.settings_edit_buf = "8".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.song.channels, 8);
    }

    #[test]
    fn test_song_settings_clamps_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_field = SettingsField::Bpm;

        // Too low
        app.settings_edit_buf = "10".to_string();
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 32);

        // Too high
        app.settings_edit_buf = "500".to_string();
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
        assert_eq!(app.instrument_cursor, 0);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.instrument_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.instrument_cursor, 17);

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.instrument_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.instrument_cursor, 0);
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
        assert_eq!(app.synth_editor_field, SynthField::Attack);
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
        assert!(app.block_clipboard.is_some());
        let clip = app.block_clipboard.as_ref().unwrap();
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
        assert!(app.block_anchor.is_none());
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
    fn test_channel_rename() {
        let mut app = make_app();
        assert_eq!(app.channel_names[0], "");
        // Ctrl+R opens rename mode
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::ChannelRename);
        // Type a name
        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.rename_buf, "Kick");
        // Enter confirms
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.channel_names[0], "Kick");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_channel_rename_backspace() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.rename_buf, "A");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.channel_names[0], "A");
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
        app.block_anchor = Some((0, 0));
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
        app.block_anchor = Some((0, 0));
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
}
