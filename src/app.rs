use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::audio::AudioEngine;
use crate::link::LinkEngine;
use crate::midi::{MidiEngine, MidiInputEngine, MidiInputEvent};
use crate::sample::SampleBank;
use crate::tracker::{Note, NoteValue, Song, SongFile, InstrumentDef, InstrumentEntry, SampleRef, SampleRefEntry};
use crate::ui::pattern_editor::SubColumn;

// Effect commands (single hex digit, stored in Cell.effect)
const EFFECT_ARPEGGIO: u8 = 0x0;      // 0xy: cycle note, note+x, note+y
const EFFECT_PORTA_UP: u8 = 0x1;      // 1xx: slide pitch up by xx per tick
const EFFECT_PORTA_DOWN: u8 = 0x2;    // 2xx: slide pitch down by xx per tick
const EFFECT_TONE_PORTA: u8 = 0x3;    // 3xx: slide toward target note at speed xx
const EFFECT_VIBRATO: u8 = 0x4;       // 4xy: vibrato speed x, depth y
const EFFECT_VOLUME_SLIDE: u8 = 0x5;  // 5xy: volume slide up x, down y per tick
const EFFECT_NOTE_DELAY: u8 = 0x6;    // 6xx: delay note trigger by xx ticks
const EFFECT_POSITION_JUMP: u8 = 0xB; // Bxx: jump to order position xx
const EFFECT_MIDI_CC: u8 = 0xC;       // Cxx: send MIDI CC (controller from instrument col, value xx)
const EFFECT_PATTERN_BREAK: u8 = 0xD; // Dxx: break to row xx of next pattern
const EFFECT_PROGRAM_CHANGE: u8 = 0xE; // Exx: program change to program xx
const EFFECT_SET_SPEED: u8 = 0xF;     // Fxx: xx<0x20 = set speed, xx>=0x20 = set BPM

/// Pitch bend center (no bend) = 0x2000 = 8192
const PITCH_BEND_CENTER: u16 = 0x2000;
/// Pitch bend range in semitones (standard MIDI default = 2)
const PITCH_BEND_RANGE: f64 = 2.0;
/// Pitch bend units per semitone
const PITCH_BEND_PER_SEMITONE: f64 = (PITCH_BEND_CENTER as f64) / PITCH_BEND_RANGE;

/// Per-channel state for continuous effects (arpeggio, portamento, vibrato, volume slide)
#[derive(Debug, Clone)]
pub struct ChannelState {
    /// Current base MIDI note being played
    pub note: Option<u8>,
    /// Current pitch offset in pitch-bend units (fractional semitones * PITCH_BEND_PER_SEMITONE)
    pub pitch_offset: f64,
    /// Current volume (0-127)
    pub volume: u8,
    /// Active effect command for this channel
    pub effect: Option<u8>,
    /// Active effect parameter
    pub effect_param: u8,
    /// Target note for tone portamento (3xx)
    pub porta_target: Option<u8>,
    /// Vibrato phase (0.0 .. 1.0)
    pub vibrato_phase: f64,
    /// Note delay: pending note trigger (note, velocity, is_note_off)
    pub delayed_note: Option<(u8, u8, bool)>,
    /// Note delay: tick on which to trigger
    pub delay_tick: u8,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            note: None,
            pitch_offset: 0.0,
            volume: 0x7F,
            effect: None,
            effect_param: 0,
            porta_target: None,
            vibrato_phase: 0.0,
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
}

/// Which field is being edited in the song settings dialog
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

/// Which field is active in the sample editor
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

/// Instrument definition for the instrument list
#[derive(Debug, Clone)]
pub struct Instrument {
    pub name: String,
    pub midi_program: Option<u8>,
    /// Index into SampleBank (None = use synth)
    pub sample_index: Option<usize>,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            name: String::new(),
            midi_program: None,
            sample_index: None,
        }
    }
}

pub struct App {
    pub song: Song,
    pub midi: MidiEngine,
    pub midi_input: MidiInputEngine,
    pub link: LinkEngine,
    pub mode: Mode,
    pub should_quit: bool,

    // Cursor state
    pub cursor_row: usize,
    pub cursor_channel: usize,
    pub cursor_sub: SubColumn,
    pub current_octave: u8,

    // Playback state
    pub playing: bool,
    pub playback_row: usize,
    pub playback_order: usize,
    last_tick: Option<Instant>,
    tick_accumulator: f64,
    /// Current sub-tick within a row (0..speed-1). Tick 0 = new row, ticks 1+ = effect processing.
    playback_tick: u8,

    // Per-channel effect state
    channel_states: Vec<ChannelState>,

    // Edit step: how many rows to advance after entering a note
    pub edit_step: usize,

    // File
    pub file_path: Option<PathBuf>,
    pub status_message: Option<String>,

    // Undo/redo
    undo_stack: Vec<Song>,
    redo_stack: Vec<Song>,

    // Clipboard
    pub clipboard: Option<Vec<crate::tracker::Cell>>,

    // Order list editing position (when not playing)
    pub edit_order: usize,

    // Channel mute/solo state
    pub muted_channels: Vec<bool>,
    pub solo_channel: Option<usize>,

    // Per-channel MIDI channel mapping (tracker channel -> MIDI channel 0-15)
    pub midi_channel_map: Vec<u8>,

    // MIDI port selection
    pub midi_port_list: Vec<String>,
    pub midi_port_cursor: usize,
    /// The mode to return to after closing the port selector
    prev_mode: Mode,

    // Song settings dialog
    pub settings_field: SettingsField,
    pub settings_edit_buf: String,

    // Instrument list
    pub instruments: Vec<Instrument>,
    pub instrument_cursor: usize,

    // Color theme
    pub theme_index: usize,

    // MIDI clock
    clock_tick_accumulator: f64,

    // Audio engine (SF2 via RustySynth and/or fundsp synth + effects, via cpal)
    pub audio: Option<AudioEngine>,

    // Sample bank
    pub sample_bank: Arc<SampleBank>,

    // Sample editor state
    pub sample_editor_slot: usize,
    pub sample_editor_field: SampleField,

    // Track page: which group of 4 tracks is visible (0 = tracks 0-3, 1 = tracks 4-7)
    pub track_page: usize,
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
            last_tick: None,
            tick_accumulator: 0.0,
            edit_step: 1,
            file_path: None,
            status_message: None,
            undo_stack: Vec::new(),
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
            instruments: (0..256).map(|_| Instrument::default()).collect(),
            instrument_cursor: 0,
            theme_index: 0,
            clock_tick_accumulator: 0.0,
            playback_tick: 0,
            channel_states: vec![ChannelState::default(); 4],
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            track_page: 0,
        }
    }

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
        let instruments: Vec<(Option<usize>, u8)> = self.instruments.iter()
            .map(|i| (i.sample_index, i.midi_program.unwrap_or(0)))
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

    fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref audio) = self.audio {
            audio.note_on(channel, note, velocity);
        }
    }

    /// Note-on with instrument awareness: routes to sample engine if instrument has a sample
    fn send_note_on_with_instrument(&mut self, channel: u8, note: u8, velocity: u8, instrument: Option<u8>) {
        let inst_idx = instrument.unwrap_or(0) as usize;
        let sample_idx = self.instruments.get(inst_idx).and_then(|i| i.sample_index);
        if let Some(sid) = sample_idx {
            if self.sample_bank.get(sid).is_some() {
                // Route to sample engine
                let _ = self.midi.note_on(channel, note, velocity);
                if let Some(ref audio) = self.audio {
                    audio.sample_note_on(sid, note, velocity, channel);
                }
                return;
            }
        }
        // Fall through to synth
        self.send_note_on(channel, note, velocity);
    }

    fn send_channel_note_off(&mut self, channel: u8) {
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref audio) = self.audio {
            audio.note_off_all_channel(channel);
            audio.sample_note_off_channel(channel);
        }
    }

    fn send_all_notes_off(&mut self) {
        let _ = self.midi.all_notes_off();
        if let Some(ref audio) = self.audio {
            audio.note_off_all();
            audio.sample_note_off_all();
        }
    }

    fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        let _ = self.midi.send_cc(channel, controller, value);
        if let Some(ref audio) = self.audio {
            audio.send_cc(channel, controller, value);
        }
    }

    fn send_program_change(&mut self, channel: u8, program: u8) {
        let _ = self.midi.program_change(channel, program);
        if let Some(ref audio) = self.audio {
            audio.program_change(channel, program);
        }
    }

    fn send_pitch_bend(&mut self, channel: u8, value: u16) {
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

    fn close_port_selector(&mut self) {
        self.mode = self.prev_mode;
    }

    fn select_midi_port(&mut self) {
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
        self.midi_channel_map.get(tracker_channel).copied().unwrap_or(tracker_channel as u8)
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
            .filter(|(_, inst)| !inst.name.is_empty() || inst.sample_index.is_some() || inst.midi_program.is_some())
            .map(|(slot, inst)| InstrumentEntry {
                slot,
                def: InstrumentDef {
                    name: inst.name.clone(),
                    midi_program: inst.midi_program,
                    sample_index: inst.sample_index,
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
        self.undo_stack.push(self.song.clone());
        self.redo_stack.clear();
        // Cap undo history
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.song.clone());
            self.song = prev;
            self.status_message = Some("Undo".to_string());
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.song.clone());
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

    // -- Playback --

    pub fn toggle_link(&mut self) {
        if self.link.is_enabled() {
            self.link.disable();
        } else {
            self.link.enable();
        }
    }

    pub fn toggle_playback(&mut self) {
        if self.playing {
            self.stop();
        } else {
            self.play();
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.playback_row = self.cursor_row;
        self.playback_order = 0;
        self.playback_tick = 0;
        self.last_tick = Some(Instant::now());
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;
        // Reset channel states and ensure we have enough for all channels
        let ch_count = self.song.channels;
        self.channel_states = vec![ChannelState::default(); ch_count];

        if self.link.is_enabled() {
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.last_tick = None;
        // Reset pitch bends to center before killing notes
        for ch in 0..self.channel_states.len() {
            let midi_ch = self.midi_channel_for(ch);
            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
        }
        self.send_all_notes_off();
        let _ = self.midi.send_stop();

        if self.link.is_enabled() {
            self.link.request_stop();
        }
    }

    /// Sync tempo from Link peers if changed externally
    pub fn sync_link(&mut self) {
        if !self.link.is_enabled() {
            return;
        }

        if let Some(new_tempo) = self.link.poll_tempo_change() {
            let new_bpm = new_tempo.round() as u16;
            if new_bpm != self.song.bpm && new_bpm >= 32 && new_bpm <= 300 {
                self.song.bpm = new_bpm;
            }
        }
    }

    pub fn tick_playback(&mut self) {
        if !self.playing {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_tick {
            let elapsed = now.duration_since(last).as_secs_f64();
            self.tick_accumulator += elapsed;

            // Send MIDI clock: 24 ppqn (pulses per quarter note)
            if self.midi.clock_enabled {
                self.clock_tick_accumulator += elapsed;
                let clock_interval = 60.0 / (self.song.bpm as f64 * 24.0);
                while self.clock_tick_accumulator >= clock_interval {
                    self.clock_tick_accumulator -= clock_interval;
                    let _ = self.midi.send_clock();
                }
            }

            let spt = self.song.seconds_per_tick();
            while self.tick_accumulator >= spt {
                self.tick_accumulator -= spt;
                self.process_tick();
            }
        }
        self.last_tick = Some(now);
    }

    /// Process a single sub-tick. Tick 0 = new row (notes + row effects). Ticks 1+ = continuous effects.
    fn process_tick(&mut self) {
        if self.playback_tick == 0 {
            self.advance_playback();
        } else {
            self.process_effects_tick();
        }
        self.playback_tick += 1;
        if self.playback_tick >= self.song.speed {
            self.playback_tick = 0;
        }
    }

    /// Tick 0: process the new row -- trigger notes, set up channel effect state, advance row pointer.
    fn advance_playback(&mut self) {
        let pattern_idx = self.song.order[self.playback_order];
        let pattern_rows = self.song.patterns[pattern_idx].rows;
        let channels = self.song.patterns[pattern_idx].channels;

        // Ensure channel_states has enough entries
        while self.channel_states.len() < channels {
            self.channel_states.push(ChannelState::default());
        }

        // Collect cell data we need before mutating self
        let cells: Vec<(Option<Note>, Option<u8>, Option<u8>, Option<u8>, Option<u8>)> = (0..channels)
            .map(|ch| {
                let cell = self.song.patterns[pattern_idx].get(self.playback_row, ch);
                (cell.note, cell.volume, cell.effect, cell.effect_value, cell.instrument)
            })
            .collect();

        // Scan for pattern-level effects (first one wins)
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        for &(_, _, effect, effect_value, _) in &cells {
            match effect {
                Some(EFFECT_POSITION_JUMP) => {
                    jump_order = Some(effect_value.unwrap_or(0) as usize);
                }
                Some(EFFECT_PATTERN_BREAK) => {
                    break_row = Some(effect_value.unwrap_or(0) as usize);
                }
                Some(EFFECT_SET_SPEED) => {
                    let val = effect_value.unwrap_or(0);
                    if val > 0 && val < 0x20 {
                        self.song.speed = val;
                    } else if val >= 0x20 {
                        self.song.bpm = val as u16;
                    }
                }
                _ => {}
            }
        }

        // Play the current row and set up per-channel effect state
        for (ch, (note, volume, effect, effect_value, instrument)) in cells.into_iter().enumerate() {
            let midi_ch = self.midi_channel_for(ch);
            let param = effect_value.unwrap_or(0);

            // For tone portamento (3xx), a new note sets the target instead of triggering
            let is_tone_porta = effect == Some(EFFECT_TONE_PORTA);

            if !self.is_channel_audible(ch) {
                // Still update channel state for muted channels so effects resume correctly
                if let Some(Note::On { .. }) = note {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else {
                            self.channel_states[ch].note = Some(midi_note);
                        }
                    }
                }
                self.channel_states[ch].effect = effect;
                self.channel_states[ch].effect_param = param;
                continue;
            }

            // Clear any previous delayed note
            self.channel_states[ch].delayed_note = None;

            // Note delay (6xx): defer note trigger to tick xx
            let is_note_delay = effect == Some(EFFECT_NOTE_DELAY) && param > 0;

            // Process notes
            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            // Tone portamento: set target, don't retrigger
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else if is_note_delay {
                            // Defer note trigger to the specified tick
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            self.channel_states[ch].delayed_note = Some((midi_note, vel, false));
                            self.channel_states[ch].delay_tick = param;
                        } else {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            // Reset pitch bend on new note
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                            self.send_note_on_with_instrument(midi_ch, midi_note, vel, instrument);
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = vel;
                        }
                    }
                }
                Some(Note::Off) => {
                    if is_note_delay {
                        // Defer note-off to the specified tick
                        self.channel_states[ch].delayed_note = Some((0, 0, true));
                        self.channel_states[ch].delay_tick = param;
                    } else {
                        self.send_channel_note_off(midi_ch);
                        self.channel_states[ch].note = None;
                        self.channel_states[ch].pitch_offset = 0.0;
                        self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                    }
                }
                None => {
                    // No note: update volume if specified
                    if let Some(vol) = volume {
                        self.channel_states[ch].volume = vol;
                    }
                }
            }

            // Store effect state for subsequent ticks
            self.channel_states[ch].effect = effect;
            self.channel_states[ch].effect_param = param;

            // Process immediate (tick 0) effects
            match effect {
                Some(EFFECT_MIDI_CC) => {
                    let controller = instrument.unwrap_or(0);
                    self.send_cc(midi_ch, controller, param);
                }
                Some(EFFECT_PROGRAM_CHANGE) => {
                    self.send_program_change(midi_ch, param);
                }
                _ => {}
            }
        }

        // Process position jump (Bxx)
        if let Some(target_order) = jump_order {
            let target = target_order.min(self.song.order.len() - 1);
            self.playback_order = target;
            let target_pattern = self.song.order[self.playback_order];
            let target_rows = self.song.patterns[target_pattern].rows;
            self.playback_row = break_row.unwrap_or(0).min(target_rows - 1);
            return;
        }

        // Process pattern break (Dxx)
        if let Some(target_row) = break_row {
            self.playback_order += 1;
            if self.playback_order >= self.song.order.len() {
                self.playback_order = 0;
            }
            let target_pattern = self.song.order[self.playback_order];
            let target_rows = self.song.patterns[target_pattern].rows;
            self.playback_row = target_row.min(target_rows - 1);
            return;
        }

        // Normal advance
        self.playback_row += 1;
        if self.playback_row >= pattern_rows {
            self.playback_row = 0;
            self.playback_order += 1;
            if self.playback_order >= self.song.order.len() {
                self.playback_order = 0;
            }
        }
    }

    /// Ticks 1..speed-1: process continuous effects (arpeggio, portamento, vibrato, volume slide).
    fn process_effects_tick(&mut self) {
        let channels = self.channel_states.len();
        for ch in 0..channels {
            if !self.is_channel_audible(ch) {
                continue;
            }
            let midi_ch = self.midi_channel_for(ch);

            // Process note delay before other effects (note may not exist yet)
            if self.channel_states[ch].effect == Some(EFFECT_NOTE_DELAY) {
                if let Some((midi_note, vel, is_off)) = self.channel_states[ch].delayed_note {
                    if self.playback_tick == self.channel_states[ch].delay_tick {
                        if is_off {
                            self.send_channel_note_off(midi_ch);
                            self.channel_states[ch].note = None;
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                        } else {
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                            self.send_note_on(midi_ch, midi_note, vel);
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = vel;
                        }
                        self.channel_states[ch].delayed_note = None;
                    }
                }
                continue;
            }

            let effect = self.channel_states[ch].effect;
            let param = self.channel_states[ch].effect_param;
            let base_note = match self.channel_states[ch].note {
                Some(n) => n,
                None => continue,
            };

            match effect {
                Some(EFFECT_ARPEGGIO) if param != 0 => {
                    let x = (param >> 4) as u8;
                    let y = (param & 0x0F) as u8;
                    // Cycle through base, base+x, base+y on ticks 1, 2, 3...
                    let phase = self.playback_tick % 3;
                    let offset = match phase {
                        0 => 0.0,
                        1 => x as f64,
                        _ => y as f64,
                    };
                    let bend = (offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_PORTA_UP) => {
                    // Slide pitch up by param units per tick (param in 16ths of a semitone)
                    self.channel_states[ch].pitch_offset += param as f64 / 16.0;
                    let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_PORTA_DOWN) => {
                    self.channel_states[ch].pitch_offset -= param as f64 / 16.0;
                    let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_TONE_PORTA) => {
                    if let Some(target) = self.channel_states[ch].porta_target {
                        let current = base_note as f64 + self.channel_states[ch].pitch_offset;
                        let target_f = target as f64;
                        let speed = param as f64 / 16.0;
                        if current < target_f {
                            self.channel_states[ch].pitch_offset += speed.min(target_f - current);
                        } else if current > target_f {
                            self.channel_states[ch].pitch_offset -= speed.min(current - target_f);
                        }
                        let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                        let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                        self.send_pitch_bend(midi_ch, value);
                    }
                }
                Some(EFFECT_VIBRATO) => {
                    let speed = (param >> 4) as f64;
                    let depth = (param & 0x0F) as f64;
                    self.channel_states[ch].vibrato_phase += speed / 64.0;
                    if self.channel_states[ch].vibrato_phase >= 1.0 {
                        self.channel_states[ch].vibrato_phase -= 1.0;
                    }
                    let sine = (self.channel_states[ch].vibrato_phase * std::f64::consts::TAU).sin();
                    let offset = sine * depth / 16.0; // depth in 16ths of a semitone
                    let total = self.channel_states[ch].pitch_offset + offset;
                    let bend = (total * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_VOLUME_SLIDE) => {
                    let up = (param >> 4) as i16;
                    let down = (param & 0x0F) as i16;
                    let delta = up - down;
                    let new_vol = (self.channel_states[ch].volume as i16 + delta).clamp(0, 127) as u8;
                    self.channel_states[ch].volume = new_vol;
                    // Send volume as CC 7
                    self.send_cc(midi_ch, 7, new_vol);
                }
                _ => {}
            }
        }
    }

    // -- MIDI input handling --

    /// Process incoming MIDI note events from external controllers
    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    fn handle_midi_input(&mut self, event: MidiInputEvent) {
        // Only enter notes in Insert mode when not playing
        if self.mode != Mode::Insert || self.playing {
            // Still preview the note
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.send_note_on(midi_ch, event.note, event.velocity);
            return;
        }

        // Convert MIDI note number to Note
        let octave = event.note / 12;
        let note_index = event.note % 12;
        if let Some(note_val) = NoteValue::from_index(note_index) {
            let note = Note::On {
                value: note_val,
                octave,
            };

            self.push_undo();

            // Preview via MIDI output
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.send_note_on(midi_ch, event.note, event.velocity);

            // Write to pattern
            let pattern_idx = self.song.order[self.current_order_position()];
            let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
            cell.note = Some(note);
            cell.volume = Some(event.velocity);

            // Advance cursor
            self.move_cursor_down(self.edit_step);
        }
    }

    // -- Input handling --

    // -- Song settings dialog --

    fn open_song_settings(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::SongSettings;
        self.settings_field = SettingsField::Title;
        self.settings_edit_buf = self.song.title.clone();
    }

    fn close_song_settings(&mut self) {
        self.mode = self.prev_mode;
    }

    fn settings_select_field(&mut self, field: SettingsField) {
        self.settings_field = field;
        self.settings_edit_buf = match field {
            SettingsField::Title => self.song.title.clone(),
            SettingsField::Bpm => self.song.bpm.to_string(),
            SettingsField::Speed => self.song.speed.to_string(),
            SettingsField::Channels => self.song.channels.to_string(),
            SettingsField::Rows => self.song.rows_per_pattern.to_string(),
        };
    }

    fn settings_apply_field(&mut self) {
        match self.settings_field {
            SettingsField::Title => {
                if !self.settings_edit_buf.is_empty() {
                    self.push_undo();
                    self.song.title = self.settings_edit_buf.clone();
                }
            }
            SettingsField::Bpm => {
                if let Ok(v) = self.settings_edit_buf.parse::<u16>() {
                    let v = v.clamp(32, 300);
                    self.push_undo();
                    self.song.bpm = v;
                    if self.link.is_enabled() {
                        self.link.set_tempo(v as f64);
                    }
                }
            }
            SettingsField::Speed => {
                if let Ok(v) = self.settings_edit_buf.parse::<u8>() {
                    let v = v.clamp(1, 31);
                    self.push_undo();
                    self.song.speed = v;
                }
            }
            SettingsField::Channels => {
                if let Ok(v) = self.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 16);
                    if v != self.song.channels {
                        self.push_undo();
                        self.song.channels = v;
                        // Resize all patterns
                        for pat in &mut self.song.patterns {
                            for row in &mut pat.data {
                                row.resize(v, crate::tracker::Cell::default());
                            }
                            pat.channels = v;
                        }
                        self.muted_channels.resize(v, false);
                        self.midi_channel_map = (0..v).map(|i| i as u8).collect();
                        if self.cursor_channel >= v {
                            self.cursor_channel = v - 1;
                        }
                    }
                }
            }
            SettingsField::Rows => {
                if let Ok(v) = self.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    if v != self.song.rows_per_pattern {
                        self.push_undo();
                        self.song.rows_per_pattern = v;
                    }
                }
            }
        }
    }

    fn handle_song_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(6) => {
                self.settings_apply_field();
                self.close_song_settings();
            }
            KeyCode::Tab | KeyCode::Down => {
                self.settings_apply_field();
                let next = self.settings_field.next();
                self.settings_select_field(next);
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.settings_apply_field();
                let prev = self.settings_field.prev();
                self.settings_select_field(prev);
            }
            KeyCode::Enter => {
                self.settings_apply_field();
                self.close_song_settings();
            }
            KeyCode::Char(c) => {
                self.settings_edit_buf.push(c);
            }
            KeyCode::Backspace => {
                self.settings_edit_buf.pop();
            }
            _ => {}
        }
    }

    // -- Instrument list --

    fn open_instrument_list(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::InstrumentList;
    }

    fn close_instrument_list(&mut self) {
        self.mode = self.prev_mode;
    }

    fn handle_instrument_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(7) => self.close_instrument_list(),
            KeyCode::Up => {
                if self.instrument_cursor > 0 {
                    self.instrument_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.instrument_cursor < 255 {
                    self.instrument_cursor += 1;
                }
            }
            KeyCode::PageUp => {
                self.instrument_cursor = self.instrument_cursor.saturating_sub(16);
            }
            KeyCode::PageDown => {
                self.instrument_cursor = (self.instrument_cursor + 16).min(255);
            }
            KeyCode::Enter => {
                // Open sample editor for current instrument
                self.open_sample_editor();
            }
            KeyCode::Char(c) => {
                self.instruments[self.instrument_cursor].name.push(c);
            }
            KeyCode::Backspace => {
                self.instruments[self.instrument_cursor].name.pop();
            }
            _ => {}
        }
    }

    fn handle_sample_editor_key(&mut self, key: KeyEvent) {
        let slot = self.sample_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
            }
            KeyCode::Tab => {
                self.sample_editor_field = self.sample_editor_field.next();
            }
            KeyCode::BackTab => {
                self.sample_editor_field = self.sample_editor_field.prev();
            }
            KeyCode::Up => {
                self.adjust_sample_field(slot, 1);
            }
            KeyCode::Down => {
                self.adjust_sample_field(slot, -1);
            }
            KeyCode::Right => {
                self.adjust_sample_field(slot, 10);
            }
            KeyCode::Left => {
                self.adjust_sample_field(slot, -10);
            }
            _ => {}
        }
    }

    fn adjust_sample_field(&mut self, slot: usize, delta: i64) {
        let mut bank = (*self.sample_bank).clone();
        if let Some(ref mut sample) = bank.samples.get_mut(slot).and_then(|s| s.as_mut()) {
            match self.sample_editor_field {
                SampleField::BaseNote => {
                    sample.base_note = (sample.base_note as i64 + delta).clamp(0, 127) as u8;
                }
                SampleField::TrimStart => {
                    sample.trim_start = (sample.trim_start as i64 + delta * 100)
                        .clamp(0, sample.data.len() as i64 - 1) as usize;
                }
                SampleField::TrimEnd => {
                    let max = sample.data.len();
                    sample.trim_end = if sample.trim_end == 0 {
                        (max as i64 + delta * 100).clamp(0, max as i64) as usize
                    } else {
                        (sample.trim_end as i64 + delta * 100).clamp(0, max as i64) as usize
                    };
                }
                SampleField::LoopEnabled => {
                    sample.loop_enabled = !sample.loop_enabled;
                }
                SampleField::LoopStart => {
                    let max = sample.effective_loop_end();
                    sample.loop_start = (sample.loop_start as i64 + delta * 100)
                        .clamp(0, max as i64) as usize;
                }
                SampleField::LoopEnd => {
                    let max = sample.end();
                    sample.loop_end = if sample.loop_end == 0 {
                        (max as i64 + delta * 100).clamp(0, max as i64) as usize
                    } else {
                        (sample.loop_end as i64 + delta * 100).clamp(0, max as i64) as usize
                    };
                }
            }
            self.sample_bank = Arc::new(bank);
            if let Some(ref audio) = self.audio {
                audio.set_sample_bank(Arc::clone(&self.sample_bank));
            }
        } else {
            self.status_message = Some("No sample loaded in this slot".to_string());
        }
    }

    // -- Theme cycling --

    pub fn cycle_theme(&mut self) {
        self.theme_index = (self.theme_index + 1) % crate::ui::theme::THEME_NAMES.len();
        let name = crate::ui::theme::THEME_NAMES[self.theme_index];
        self.status_message = Some(format!("Theme: {}", name));
    }

    // -- MIDI clock toggle --

    pub fn toggle_midi_clock(&mut self) {
        self.midi.clock_enabled = !self.midi.clock_enabled;
        let state = if self.midi.clock_enabled { "on" } else { "off" };
        self.status_message = Some(format!("MIDI clock {}", state));
    }

    // -- Mouse handling --

    pub fn handle_mouse(&mut self, event: MouseEvent, pattern_area_y: u16, pattern_area_x: u16) {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_mouse_click(event.column, event.row, pattern_area_y, pattern_area_x);
            }
            MouseEventKind::ScrollUp => {
                self.move_cursor_up(3);
            }
            MouseEventKind::ScrollDown => {
                self.move_cursor_down(3);
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16, area_y: u16, area_x: u16) {
        // Check if click is in the pattern editor area
        if y < area_y {
            return;
        }
        let screen_y = (y - area_y) as usize;

        // Calculate which row was clicked
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &self.song.patterns[pattern_idx];
        let visible_rows = 40usize; // approximate
        let center_offset = visible_rows / 2;
        let focus_row = self.cursor_row;
        let start_row = if focus_row >= center_offset {
            focus_row - center_offset
        } else {
            0
        };
        let clicked_row = start_row + screen_y;
        if clicked_row < pattern.rows {
            self.cursor_row = clicked_row;
        }

        // Calculate which channel/sub-column was clicked
        let row_num_width: u16 = 3;
        let sep_width: u16 = 3;
        let channel_width: u16 = 14;

        if x < area_x + row_num_width + sep_width {
            return;
        }
        let col_x = x - area_x - row_num_width - sep_width;

        // Each channel is channel_width + separator_width (except first)
        let stride = channel_width + sep_width;
        let ch = (col_x / stride) as usize;
        let within = col_x % stride;

        // Map visible channel index to actual channel (offset by page)
        let actual_ch = self.track_page * 4 + ch as usize;
        if actual_ch < pattern.channels {
            self.cursor_channel = actual_ch;
            // note=0..3, gap=3, inst=4..6, gap=6, vol=7..9, gap=9, fx=10..13
            if within < 3 {
                self.cursor_sub = SubColumn::Note;
            } else if within < 6 {
                self.cursor_sub = SubColumn::Instrument;
            } else if within < 9 {
                self.cursor_sub = SubColumn::Volume;
            } else if within < 14 {
                self.cursor_sub = SubColumn::Effect;
            }
        }
    }

    // -- MIDI file export/import --

    pub fn export_wav_file(&mut self) {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("wav"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.wav", name))
            });
        let instruments: Vec<(Option<usize>, u8)> = self.instruments.iter()
            .map(|i| (i.sample_index, i.midi_program.unwrap_or(0)))
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

    // -- Input handling --

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status_message = None;
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            Mode::MidiPortSelect => self.handle_port_select_key(key),
            Mode::Help => self.handle_help_key(key),
            Mode::SongSettings => self.handle_song_settings_key(key),
            Mode::InstrumentList => self.handle_instrument_list_key(key),
            Mode::SampleEditor => self.handle_sample_editor_key(key),
        }
    }

    fn handle_common_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => { self.save(); return true; }
                KeyCode::Char('z') => { self.undo(); return true; }
                KeyCode::Char('y') => { self.redo(); return true; }
                KeyCode::Char('c') => { self.copy_row(); return true; }
                KeyCode::Char('v') => { self.paste_row(); return true; }
                KeyCode::Char('x') => { self.cut_row(); return true; }
                KeyCode::Right => { self.next_order_position(); return true; }
                KeyCode::Left => { self.prev_order_position(); return true; }
                KeyCode::Char('e') => { self.export_midi(); return true; }
                KeyCode::Char('w') => { self.export_wav_file(); return true; }
                KeyCode::Char('m') => { self.toggle_midi_clock(); return true; }
                // Ctrl+1..8 select specific tracks
                KeyCode::Char('1') => { self.select_track(0); return true; }
                KeyCode::Char('2') => { self.select_track(1); return true; }
                KeyCode::Char('3') => { self.select_track(2); return true; }
                KeyCode::Char('4') => { self.select_track(3); return true; }
                KeyCode::Char('5') => { self.select_track(4); return true; }
                KeyCode::Char('6') => { self.select_track(5); return true; }
                KeyCode::Char('7') => { self.select_track(6); return true; }
                KeyCode::Char('8') => { self.select_track(7); return true; }
                // Ctrl+F9-F12: solo channels on current page
                KeyCode::F(9) => { let ch = self.track_page * 4; self.toggle_solo(ch); return true; }
                KeyCode::F(10) => { let ch = self.track_page * 4 + 1; self.toggle_solo(ch); return true; }
                KeyCode::F(11) => { let ch = self.track_page * 4 + 2; self.toggle_solo(ch); return true; }
                KeyCode::F(12) => { let ch = self.track_page * 4 + 3; self.toggle_solo(ch); return true; }
                _ => {}
            }
        }
        // F9-F12: mute channels on current page
        match key.code {
            KeyCode::F(9) => { let ch = self.track_page * 4; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(10) => { let ch = self.track_page * 4 + 1; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(11) => { let ch = self.track_page * 4 + 2; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(12) => { let ch = self.track_page * 4 + 3; self.toggle_channel_mute(ch); return true; }
            _ => {}
        }
        false
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) { return; }

        // Ctrl combos specific to Normal mode
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => { self.add_new_pattern_to_order(); return; }
                KeyCode::Char('d') => { self.clone_current_pattern(); return; }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.mode = Mode::Insert,
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_port_selector(),
            KeyCode::F(3) => self.toggle_link(),
            KeyCode::F(4) => self.insert_order_entry(),
            KeyCode::F(5) => self.remove_order_entry(),
            KeyCode::F(6) => self.open_song_settings(),
            KeyCode::F(7) => self.open_instrument_list(),
            KeyCode::F(8) => self.cycle_theme(),

            // Track page navigation
            KeyCode::Tab => self.toggle_track_page(),
            KeyCode::BackTab => {
                // Reverse page toggle
                let max_pages = (self.song.channels + 3) / 4;
                if max_pages > 1 {
                    self.track_page = if self.track_page == 0 { max_pages - 1 } else { self.track_page - 1 };
                    let page_start = self.track_page * 4;
                    let page_end = (page_start + 4).min(self.song.channels);
                    if self.cursor_channel < page_start || self.cursor_channel >= page_end {
                        self.cursor_channel = page_start;
                        self.cursor_sub = SubColumn::Note;
                    }
                    self.status_message = Some(format!("Track page {} (ch {}-{})", self.track_page + 1, page_start + 1, page_end));
                }
            }

            // Navigation
            KeyCode::Up => self.move_cursor_up(1),
            KeyCode::Down => self.move_cursor_down(1),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::PageUp => self.move_cursor_up(16),
            KeyCode::PageDown => self.move_cursor_down(16),
            KeyCode::Home => self.cursor_row = 0,
            KeyCode::End => self.cursor_row = self.current_pattern_rows() - 1,

            // Octave
            KeyCode::Char('+') => {
                if self.current_octave < 9 {
                    self.current_octave += 1;
                }
            }
            KeyCode::Char('-') => {
                if self.current_octave > 0 {
                    self.current_octave -= 1;
                }
            }

            // BPM
            KeyCode::Char(']') => self.change_bpm(1),
            KeyCode::Char('[') => self.change_bpm(-1),

            // Edit step
            KeyCode::Char(')') => self.change_edit_step(1),
            KeyCode::Char('(') => self.change_edit_step(-1),

            // Row insert/delete
            KeyCode::Insert => self.insert_row_at_cursor(),
            KeyCode::Backspace => self.delete_row_at_cursor(),

            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) { return; }
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_port_selector(),
            KeyCode::F(3) => self.toggle_link(),
            KeyCode::F(6) => self.open_song_settings(),
            KeyCode::F(7) => self.open_instrument_list(),
            KeyCode::F(8) => self.cycle_theme(),

            // Track page navigation
            KeyCode::Tab => self.toggle_track_page(),
            KeyCode::BackTab => {
                let max_pages = (self.song.channels + 3) / 4;
                if max_pages > 1 {
                    self.track_page = if self.track_page == 0 { max_pages - 1 } else { self.track_page - 1 };
                    let page_start = self.track_page * 4;
                    let page_end = (page_start + 4).min(self.song.channels);
                    if self.cursor_channel < page_start || self.cursor_channel >= page_end {
                        self.cursor_channel = page_start;
                        self.cursor_sub = SubColumn::Note;
                    }
                    self.status_message = Some(format!("Track page {} (ch {}-{})", self.track_page + 1, page_start + 1, page_end));
                }
            }

            // Navigation
            KeyCode::Up => self.move_cursor_up(1),
            KeyCode::Down => self.move_cursor_down(1),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::PageUp => self.move_cursor_up(16),
            KeyCode::PageDown => self.move_cursor_down(16),
            KeyCode::Home => self.cursor_row = 0,
            KeyCode::End => self.cursor_row = self.current_pattern_rows() - 1,

            // Octave
            KeyCode::Char('+') => {
                if self.current_octave < 9 {
                    self.current_octave += 1;
                }
            }
            KeyCode::Char('-') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.current_octave > 0 {
                    self.current_octave -= 1;
                }
            }

            // Delete current cell content
            KeyCode::Delete | KeyCode::Backspace => {
                self.delete_at_cursor();
            }

            // Note off (= key in Insert mode on Note sub-column)
            KeyCode::Char('=') if self.cursor_sub == SubColumn::Note => {
                self.enter_note_off();
            }

            // Piano keyboard note entry
            KeyCode::Char(c) => {
                match self.cursor_sub {
                    SubColumn::Note => self.try_enter_note(c),
                    SubColumn::Instrument => self.try_enter_hex_digit(c, SubColumn::Instrument),
                    SubColumn::Volume => self.try_enter_hex_digit(c, SubColumn::Volume),
                    SubColumn::Effect => self.try_enter_hex_digit(c, SubColumn::Effect),
                }
            }

            _ => {}
        }
    }

    fn open_help(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::Help;
    }

    fn close_help(&mut self) {
        self.mode = self.prev_mode;
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') => self.close_help(),
            _ => {}
        }
    }

    fn handle_port_select_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(2) => self.close_port_selector(),
            KeyCode::Up => {
                if self.midi_port_cursor > 0 {
                    self.midi_port_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.midi_port_cursor + 1 < self.midi_port_list.len() {
                    self.midi_port_cursor += 1;
                }
            }
            KeyCode::Enter => self.select_midi_port(),
            _ => {}
        }
    }

    fn try_enter_note(&mut self, c: char) {
        // Piano keyboard layout (lowercase):
        // z=C, s=C#, x=D, d=D#, c=E, v=F, g=F#, b=G, h=G#, n=A, j=A#, m=B
        // Upper octave:
        // q=C, 2=C#, w=D, 3=D#, e=E, r=F, 5=F#, t=G, 6=G#, y=A, 7=A#, u=B
        let (note_val, octave_offset) = match c {
            'z' => (NoteValue::C, 0),
            's' => (NoteValue::Cs, 0),
            'x' => (NoteValue::D, 0),
            'd' => (NoteValue::Ds, 0),
            'c' => (NoteValue::E, 0),
            'v' => (NoteValue::F, 0),
            'g' => (NoteValue::Fs, 0),
            'b' => (NoteValue::G, 0),
            'h' => (NoteValue::Gs, 0),
            'n' => (NoteValue::A, 0),
            'j' => (NoteValue::As, 0),
            'm' => (NoteValue::B, 0),
            'q' => (NoteValue::C, 1),
            '2' => (NoteValue::Cs, 1),
            'w' => (NoteValue::D, 1),
            '3' => (NoteValue::Ds, 1),
            'e' => (NoteValue::E, 1),
            'r' => (NoteValue::F, 1),
            '5' => (NoteValue::Fs, 1),
            't' => (NoteValue::G, 1),
            '6' => (NoteValue::Gs, 1),
            'y' => (NoteValue::A, 1),
            '7' => (NoteValue::As, 1),
            'u' => (NoteValue::B, 1),
            _ => return,
        };

        let octave = self.current_octave + octave_offset;
        if octave > 9 {
            return;
        }

        let note = Note::On {
            value: note_val,
            octave,
        };

        self.push_undo();

        // Preview the note via MIDI
        if let Some(midi_note) = note.to_midi_note() {
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.send_note_on(midi_ch, midi_note, 0x7F);
        }

        // Write to pattern
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        cell.note = Some(note);

        // Advance cursor
        self.move_cursor_down(self.edit_step);
    }

    fn try_enter_hex_digit(&mut self, c: char, sub: SubColumn) {
        let digit = match c {
            '0'..='9' => c as u8 - b'0',
            'a'..='f' => c as u8 - b'a' + 10,
            _ => return,
        };

        self.push_undo();

        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);

        match sub {
            SubColumn::Instrument => {
                let current = cell.instrument.unwrap_or(0);
                // Shift left and add new digit (2 hex digits max)
                cell.instrument = Some(((current << 4) | digit) & 0xFF);
            }
            SubColumn::Volume => {
                let current = cell.volume.unwrap_or(0);
                cell.volume = Some(((current << 4) | digit) & 0xFF);
            }
            SubColumn::Effect => {
                // Effect is 1 hex digit for command + 2 hex digits for value
                // Simple approach: rotate through effect then effect_value
                if cell.effect.is_none() {
                    cell.effect = Some(digit);
                } else {
                    let current_val = cell.effect_value.unwrap_or(0);
                    cell.effect_value = Some(((current_val << 4) | digit) & 0xFF);
                }
            }
            SubColumn::Note => {} // handled separately
        }
    }

    fn enter_note_off(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        cell.note = Some(Note::Off);
        self.move_cursor_down(self.edit_step);
    }

    fn delete_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        match self.cursor_sub {
            SubColumn::Note => cell.note = None,
            SubColumn::Instrument => cell.instrument = None,
            SubColumn::Volume => cell.volume = None,
            SubColumn::Effect => {
                cell.effect = None;
                cell.effect_value = None;
            }
        }
    }

    // -- Cursor movement --

    fn move_cursor_up(&mut self, amount: usize) {
        if self.cursor_row >= amount {
            self.cursor_row -= amount;
        } else {
            self.cursor_row = 0;
        }
    }

    fn move_cursor_down(&mut self, amount: usize) {
        let max = self.current_pattern_rows() - 1;
        self.cursor_row = (self.cursor_row + amount).min(max);
    }

    /// Get the number of rows in the current pattern (per-pattern length)
    fn current_pattern_rows(&self) -> usize {
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].rows
    }

    fn change_bpm(&mut self, delta: i16) {
        let new_bpm = (self.song.bpm as i16 + delta).clamp(32, 300) as u16;
        self.song.bpm = new_bpm;
        if self.link.is_enabled() {
            self.link.set_tempo(new_bpm as f64);
        }
    }

    fn change_edit_step(&mut self, delta: i16) {
        let new_step = (self.edit_step as i16 + delta).clamp(0, 16) as usize;
        self.edit_step = new_step;
        self.status_message = Some(format!("Edit step: {}", self.edit_step));
    }

    fn insert_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].insert_row(self.cursor_row);
        self.status_message = Some(format!("Inserted row at {:02X}", self.cursor_row));
    }

    fn delete_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].delete_row(self.cursor_row);
        self.status_message = Some(format!("Deleted row at {:02X}", self.cursor_row));
    }

    /// Toggle track page (0 = tracks 1-4, 1 = tracks 5-8, etc.)
    fn toggle_track_page(&mut self) {
        let max_pages = (self.song.channels + 3) / 4;
        if max_pages <= 1 {
            return;
        }
        self.track_page = (self.track_page + 1) % max_pages;
        // Move cursor to same relative position on the new page
        let page_start = self.track_page * 4;
        let page_end = (page_start + 4).min(self.song.channels);
        if self.cursor_channel < page_start || self.cursor_channel >= page_end {
            self.cursor_channel = page_start;
            self.cursor_sub = SubColumn::Note;
        }
        let page_display = self.track_page + 1;
        self.status_message = Some(format!("Track page {} (ch {}-{})", page_display, page_start + 1, page_end));
    }

    /// Select a specific track by number (0-indexed)
    fn select_track(&mut self, track: usize) {
        if track < self.song.channels {
            self.cursor_channel = track;
            self.cursor_sub = SubColumn::Note;
            self.track_page = track / 4;
        }
    }

    /// Get the range of visible channels for the current track page
    pub fn visible_channels(&self) -> std::ops::Range<usize> {
        let start = self.track_page * 4;
        let end = (start + 4).min(self.song.channels);
        start..end
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_sub == SubColumn::Note {
            if self.cursor_channel > 0 {
                self.cursor_channel -= 1;
                self.cursor_sub = SubColumn::Effect;
                self.track_page = self.cursor_channel / 4;
            }
        } else {
            self.cursor_sub = self.cursor_sub.prev();
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_sub == SubColumn::Effect {
            if self.cursor_channel < self.song.channels - 1 {
                self.cursor_channel += 1;
                self.cursor_sub = SubColumn::Note;
                self.track_page = self.cursor_channel / 4;
            }
        } else {
            self.cursor_sub = self.cursor_sub.next();
        }
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
fn resolve_relative(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            last_tick: None,
            tick_accumulator: 0.0,
            playback_tick: 0,
            channel_states: vec![ChannelState::default(); 4],
            edit_step: 1,
            file_path: None,
            status_message: None,
            undo_stack: Vec::new(),
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
            instruments: (0..256).map(|_| Instrument::default()).collect(),
            instrument_cursor: 0,
            theme_index: 0,
            clock_tick_accumulator: 0.0,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            sample_editor_slot: 0,
            sample_editor_field: SampleField::BaseNote,
            track_page: 0,
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
        app.move_cursor_right();
        assert_eq!(app.cursor_channel, 3); // stays
        assert_eq!(app.cursor_sub, SubColumn::Effect); // stays
    }

    #[test]
    fn test_note_entry() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        app.try_enter_note('z'); // C at current octave
        let pat = &app.song.patterns[0];
        assert_eq!(
            pat.get(0, 0).note,
            Some(Note::On { value: NoteValue::C, octave: 4 })
        );
        // Cursor should have advanced by edit_step
        assert_eq!(app.cursor_row, 1);
    }

    #[test]
    fn test_note_off_entry() {
        let mut app = make_app();
        app.enter_note_off();
        let pat = &app.song.patterns[0];
        assert_eq!(pat.get(0, 0).note, Some(Note::Off));
    }

    #[test]
    fn test_delete_at_cursor() {
        let mut app = make_app();
        // Enter a note first
        app.try_enter_note('z');
        app.cursor_row = 0; // go back
        assert!(app.song.patterns[0].get(0, 0).note.is_some());

        app.delete_at_cursor();
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
    }

    #[test]
    fn test_play_stop() {
        let mut app = make_app();
        assert!(!app.is_playing());

        app.play();
        assert!(app.is_playing());

        app.stop();
        assert!(!app.is_playing());
    }

    #[test]
    fn test_mode_toggle() {
        let mut app = make_app();
        assert_eq!(app.mode, Mode::Normal);

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        app.handle_key(esc);
        assert_eq!(app.mode, Mode::Insert);

        app.handle_key(esc);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_tab_toggles_track_page() {
        let mut app = make_app();
        // With 4 channels, only 1 page, Tab does nothing
        assert_eq!(app.track_page, 0);
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.handle_key(tab);
        assert_eq!(app.track_page, 0);

        // Set up 8 channels for multi-page testing
        app.song.channels = 8;
        for pat in &mut app.song.patterns {
            for row in &mut pat.data {
                row.resize(8, crate::tracker::Cell::default());
            }
            pat.channels = 8;
        }
        app.muted_channels.resize(8, false);

        // Tab toggles to page 1
        app.handle_key(tab);
        assert_eq!(app.track_page, 1);
        assert_eq!(app.cursor_channel, 4); // moved to first channel on page 2

        // Tab wraps back to page 0
        app.handle_key(tab);
        assert_eq!(app.track_page, 0);

        // Shift-Tab goes to page 1 (reverse)
        let stab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        app.handle_key(stab);
        assert_eq!(app.track_page, 1);
    }

    #[test]
    fn test_ctrl_number_selects_track() {
        let mut app = make_app();
        app.song.channels = 8;
        for pat in &mut app.song.patterns {
            for row in &mut pat.data {
                row.resize(8, crate::tracker::Cell::default());
            }
            pat.channels = 8;
        }
        app.muted_channels.resize(8, false);

        // Ctrl+5 selects track 4 (0-indexed) and switches page
        let ctrl5 = KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL);
        app.handle_key(ctrl5);
        assert_eq!(app.cursor_channel, 4);
        assert_eq!(app.track_page, 1);

        // Ctrl+1 selects track 0 and switches back to page 0
        let ctrl1 = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL);
        app.handle_key(ctrl1);
        assert_eq!(app.cursor_channel, 0);
        assert_eq!(app.track_page, 0);
    }

    #[test]
    fn test_octave_change() {
        let mut app = make_app();
        assert_eq!(app.current_octave, 4);

        let key_up = KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE);
        app.handle_key(key_up);
        assert_eq!(app.current_octave, 5);

        let key_down = KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE);
        app.handle_key(key_down);
        assert_eq!(app.current_octave, 4);
    }

    #[test]
    fn test_upper_octave_note_entry() {
        let mut app = make_app();
        app.current_octave = 4;
        app.try_enter_note('q'); // C at octave+1
        let pat = &app.song.patterns[0];
        assert_eq!(
            pat.get(0, 0).note,
            Some(Note::On { value: NoteValue::C, octave: 5 })
        );
    }

    #[test]
    fn test_hex_entry_instrument() {
        let mut app = make_app();
        app.cursor_sub = SubColumn::Instrument;
        app.try_enter_hex_digit('a', SubColumn::Instrument);
        let cell = app.song.patterns[0].get(0, 0);
        assert_eq!(cell.instrument, Some(0x0A));

        app.try_enter_hex_digit('3', SubColumn::Instrument);
        let cell = app.song.patterns[0].get(0, 0);
        assert_eq!(cell.instrument, Some(0xA3));
    }

    #[test]
    fn test_open_port_selector() {
        let mut app = make_app();
        app.mode = Mode::Normal;
        app.open_port_selector();
        assert_eq!(app.mode, Mode::MidiPortSelect);
        assert_eq!(app.prev_mode, Mode::Normal);
        assert_eq!(app.midi_port_cursor, 0);
        // Should have at least the virtual port on unix
        #[cfg(unix)]
        assert!(!app.midi_port_list.is_empty());
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
        app.midi_port_list = vec!["Port A".into(), "Port B".into(), "Port C".into()];
        app.midi_port_cursor = 0;
        app.mode = Mode::MidiPortSelect;

        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(down);
        assert_eq!(app.midi_port_cursor, 1);

        app.handle_key(down);
        assert_eq!(app.midi_port_cursor, 2);

        // Can't go past end
        app.handle_key(down);
        assert_eq!(app.midi_port_cursor, 2);

        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        app.handle_key(up);
        assert_eq!(app.midi_port_cursor, 1);
    }

    #[test]
    fn test_port_select_esc_closes() {
        let mut app = make_app();
        app.prev_mode = Mode::Normal;
        app.mode = Mode::MidiPortSelect;

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        app.handle_key(esc);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_f2_opens_port_selector() {
        let mut app = make_app();
        let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        app.handle_key(f2);
        assert_eq!(app.mode, Mode::MidiPortSelect);
    }

    #[test]
    fn test_undo_redo() {
        let mut app = make_app();
        // Enter a note
        app.try_enter_note('z'); // C-4 at row 0
        assert!(app.song.patterns[0].get(0, 0).note.is_some());
        assert_eq!(app.undo_stack.len(), 1);

        // Undo should restore empty
        app.undo();
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
        assert_eq!(app.redo_stack.len(), 1);

        // Redo should restore the note
        app.redo();
        assert!(app.song.patterns[0].get(0, 0).note.is_some());
    }

    #[test]
    fn test_undo_clears_redo_on_new_edit() {
        let mut app = make_app();
        app.try_enter_note('z');
        app.undo();
        assert_eq!(app.redo_stack.len(), 1);

        // New edit should clear redo
        app.cursor_row = 0;
        app.try_enter_note('x'); // D-4
        assert_eq!(app.redo_stack.len(), 0);
    }

    #[test]
    fn test_copy_paste_row() {
        let mut app = make_app();
        // Enter notes in row 0
        app.try_enter_note('z'); // C-4 at row 0, cursor advances to row 1
        app.cursor_row = 0;
        app.copy_row();
        assert!(app.clipboard.is_some());

        // Paste at row 5
        app.cursor_row = 5;
        app.paste_row();
        let cell = app.song.patterns[0].get(5, 0);
        assert_eq!(
            cell.note,
            Some(Note::On { value: NoteValue::C, octave: 4 })
        );
    }

    #[test]
    fn test_cut_row() {
        let mut app = make_app();
        app.try_enter_note('z');
        app.cursor_row = 0;
        app.cut_row();

        // Row 0 should be cleared
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
        // Clipboard should have the note
        assert!(app.clipboard.is_some());
        let row = app.clipboard.as_ref().unwrap();
        assert_eq!(
            row[0].note,
            Some(Note::On { value: NoteValue::C, octave: 4 })
        );
    }

    #[test]
    fn test_save_load_roundtrip() {
        let mut app = make_app();
        app.song.title = "TestSong".to_string();
        app.song.bpm = 140;
        app.try_enter_note('z'); // C-4 at row 0

        let tmp = std::env::temp_dir().join("rtrack_test.rtrk");
        app.file_path = Some(tmp.clone());
        app.save();
        assert!(tmp.exists());

        // Load into a fresh app
        let mut app2 = make_app();
        app2.load_file(tmp.clone());
        assert_eq!(app2.song.title, "TestSong");
        assert_eq!(app2.song.bpm, 140);
        assert_eq!(
            app2.song.patterns[0].get(0, 0).note,
            Some(Note::On { value: NoteValue::C, octave: 4 })
        );

        // Cleanup
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_ctrl_s_saves() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("rtrack_ctrl_s_test.rtrk");
        app.file_path = Some(tmp.clone());

        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(tmp.exists());

        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_ctrl_z_undoes() {
        let mut app = make_app();
        app.try_enter_note('z');
        assert!(app.song.patterns[0].get(0, 0).note.is_some());

        let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
    }

    #[test]
    fn test_order_navigation() {
        let mut app = make_app();
        // Add a second pattern to order
        app.song.add_pattern();
        app.song.order.push(1);
        assert_eq!(app.edit_order, 0);

        app.next_order_position();
        assert_eq!(app.edit_order, 1);
        assert_eq!(app.current_order_position(), 1);

        // Can't go past end
        app.next_order_position();
        assert_eq!(app.edit_order, 1);

        app.prev_order_position();
        assert_eq!(app.edit_order, 0);

        // Can't go below 0
        app.prev_order_position();
        assert_eq!(app.edit_order, 0);
    }

    #[test]
    fn test_add_new_pattern_to_order() {
        let mut app = make_app();
        assert_eq!(app.song.patterns.len(), 1);
        assert_eq!(app.song.order.len(), 1);

        app.add_new_pattern_to_order();
        assert_eq!(app.song.patterns.len(), 2);
        assert_eq!(app.song.order.len(), 2);
        assert_eq!(app.song.order[1], 1);
        assert_eq!(app.edit_order, 1);
    }

    #[test]
    fn test_clone_current_pattern() {
        let mut app = make_app();
        // Put a note in pattern 0
        app.try_enter_note('z');
        app.cursor_row = 0;

        app.clone_current_pattern();
        assert_eq!(app.song.patterns.len(), 2);
        assert_eq!(app.song.order, vec![0, 1]);
        assert_eq!(app.edit_order, 1);

        // The cloned pattern should have the same note
        assert_eq!(
            app.song.patterns[1].get(0, 0).note,
            Some(Note::On { value: NoteValue::C, octave: 4 })
        );
    }

    #[test]
    fn test_insert_remove_order_entry() {
        let mut app = make_app();
        app.insert_order_entry();
        assert_eq!(app.song.order, vec![0, 0]);
        assert_eq!(app.edit_order, 1);

        app.remove_order_entry();
        assert_eq!(app.song.order, vec![0]);
        assert_eq!(app.edit_order, 0);

        // Can't remove the last entry
        app.remove_order_entry();
        assert_eq!(app.song.order, vec![0]);
    }

    #[test]
    fn test_channel_mute() {
        let mut app = make_app();
        assert!(!app.muted_channels[0]);

        app.toggle_channel_mute(0);
        assert!(app.muted_channels[0]);

        app.toggle_channel_mute(0);
        assert!(!app.muted_channels[0]);
    }

    #[test]
    fn test_ctrl_right_navigates_order() {
        let mut app = make_app();
        app.song.add_pattern();
        app.song.order.push(1);

        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
        app.handle_key(key);
        assert_eq!(app.edit_order, 1);

        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
        app.handle_key(key);
        assert_eq!(app.edit_order, 0);
    }

    #[test]
    fn test_f9_toggles_mute() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE);
        app.handle_key(key);
        assert!(app.muted_channels[0]);

        app.handle_key(key);
        assert!(!app.muted_channels[0]);
    }

    #[test]
    fn test_midi_channel_mapping() {
        let mut app = make_app();
        // Default: tracker ch 0 -> MIDI ch 0, etc.
        assert_eq!(app.midi_channel_for(0), 0);
        assert_eq!(app.midi_channel_for(3), 3);

        // Remap tracker ch 0 to MIDI ch 9 (drums)
        app.midi_channel_map[0] = 9;
        assert_eq!(app.midi_channel_for(0), 9);

        // Out of bounds falls back to tracker channel index
        assert_eq!(app.midi_channel_for(99), 99);
    }

    // -- Solo tests --

    #[test]
    fn test_solo_channel() {
        let mut app = make_app();
        assert!(app.is_channel_audible(0));
        assert!(app.is_channel_audible(1));

        app.toggle_solo(0);
        assert_eq!(app.solo_channel, Some(0));
        assert!(app.is_channel_audible(0));
        assert!(!app.is_channel_audible(1));
        assert!(!app.is_channel_audible(3));

        // Toggle same channel off
        app.toggle_solo(0);
        assert_eq!(app.solo_channel, None);
        assert!(app.is_channel_audible(0));
        assert!(app.is_channel_audible(1));

        // Toggle different channel
        app.toggle_solo(2);
        assert_eq!(app.solo_channel, Some(2));
        assert!(!app.is_channel_audible(0));
        assert!(app.is_channel_audible(2));
    }

    #[test]
    fn test_solo_overrides_mute() {
        let mut app = make_app();
        app.muted_channels[1] = true;
        assert!(!app.is_channel_audible(1));

        // Solo on ch 1 should make it audible despite mute
        app.solo_channel = Some(1);
        assert!(app.is_channel_audible(1));
        assert!(!app.is_channel_audible(0));
    }

    #[test]
    fn test_mute_clears_solo() {
        let mut app = make_app();
        app.toggle_solo(0);
        assert_eq!(app.solo_channel, Some(0));

        // Toggling mute should clear solo
        app.toggle_channel_mute(1);
        assert_eq!(app.solo_channel, None);
    }

    #[test]
    fn test_ctrl_f9_toggles_solo() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::F(9), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert_eq!(app.solo_channel, Some(0));

        app.handle_key(key);
        assert_eq!(app.solo_channel, None);
    }

    // -- Pattern break/jump effect tests --

    #[test]
    fn test_pattern_break_effect() {
        use crate::tracker::Cell;

        let mut app = make_app();
        // Create two patterns in order
        app.song.add_pattern();
        app.song.order.push(1);

        // Put a pattern break (D05) at row 2 of pattern 0
        app.song.patterns[0].set_cell(2, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_PATTERN_BREAK),
            effect_value: Some(0x05),
        });

        app.play();
        // Advance through rows 0, 1, 2
        app.advance_playback(); // row 0 -> row 1
        app.advance_playback(); // row 1 -> row 2
        app.advance_playback(); // row 2 hits Dxx -> jumps to pattern 1, row 5

        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 5);
    }

    #[test]
    fn test_position_jump_effect() {
        use crate::tracker::Cell;

        let mut app = make_app();
        // Create three patterns in order
        app.song.add_pattern();
        app.song.add_pattern();
        app.song.order = vec![0, 1, 2];

        // Put a position jump (B02) at row 1 of pattern 0
        app.song.patterns[0].set_cell(1, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_POSITION_JUMP),
            effect_value: Some(0x02),
        });

        app.play();
        app.advance_playback(); // row 0 -> row 1
        app.advance_playback(); // row 1 hits Bxx -> jumps to order pos 2, row 0

        assert_eq!(app.playback_order, 2);
        assert_eq!(app.playback_row, 0);
    }

    #[test]
    fn test_position_jump_with_break() {
        use crate::tracker::Cell;

        let mut app = make_app();
        app.song.add_pattern();
        app.song.order = vec![0, 1];

        // Bxx and Dxx on same row: jump to order 1, row 3
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_POSITION_JUMP),
            effect_value: Some(0x01),
        });
        app.song.patterns[0].set_cell(0, 1, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_PATTERN_BREAK),
            effect_value: Some(0x03),
        });

        app.play();
        app.advance_playback(); // row 0 hits B01 + D03 -> order 1, row 3

        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 3);
    }

    #[test]
    fn test_pattern_break_wraps_order() {
        use crate::tracker::Cell;

        let mut app = make_app();
        // Single pattern in order, pattern break should wrap to order 0
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_PATTERN_BREAK),
            effect_value: Some(0x00),
        });

        app.play();
        app.advance_playback(); // row 0 hits D00 -> next pattern (wraps to 0), row 0

        assert_eq!(app.playback_order, 0);
        assert_eq!(app.playback_row, 0);
    }

    #[test]
    fn test_position_jump_clamps_to_max() {
        use crate::tracker::Cell;

        let mut app = make_app();
        // Jump to order 0xFF but only 1 entry exists -> clamp to 0
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_POSITION_JUMP),
            effect_value: Some(0xFF),
        });

        app.play();
        app.advance_playback();

        assert_eq!(app.playback_order, 0); // clamped to max valid
    }

    // -- Edit step tests --

    #[test]
    fn test_edit_step_change() {
        let mut app = make_app();
        assert_eq!(app.edit_step, 1);

        app.change_edit_step(1);
        assert_eq!(app.edit_step, 2);

        app.change_edit_step(-1);
        assert_eq!(app.edit_step, 1);

        app.change_edit_step(-2);
        assert_eq!(app.edit_step, 0); // clamped to 0

        app.edit_step = 15;
        app.change_edit_step(5);
        assert_eq!(app.edit_step, 16); // clamped to 16
    }

    #[test]
    fn test_edit_step_affects_note_entry() {
        let mut app = make_app();
        app.edit_step = 4;
        app.try_enter_note('z'); // C-4
        assert_eq!(app.cursor_row, 4); // advanced by 4
    }

    #[test]
    fn test_edit_step_zero_no_advance() {
        let mut app = make_app();
        app.edit_step = 0;
        app.try_enter_note('z');
        assert_eq!(app.cursor_row, 0); // no advance
    }

    // -- Row insert/delete tests --

    #[test]
    fn test_insert_row() {
        let mut app = make_app();
        // Enter notes at rows 0, 1, 2
        app.try_enter_note('z'); // C-4 at row 0, cursor to 1
        app.try_enter_note('x'); // D-4 at row 1, cursor to 2
        app.try_enter_note('c'); // E-4 at row 2

        // Insert at row 1 -> should push D-4 and E-4 down
        app.cursor_row = 1;
        app.insert_row_at_cursor();

        let pat = &app.song.patterns[0];
        assert_eq!(pat.get(0, 0).note, Some(Note::On { value: NoteValue::C, octave: 4 }));
        assert!(pat.get(1, 0).note.is_none()); // new empty row
        assert_eq!(pat.get(2, 0).note, Some(Note::On { value: NoteValue::D, octave: 4 }));
        assert_eq!(pat.get(3, 0).note, Some(Note::On { value: NoteValue::E, octave: 4 }));
        assert_eq!(pat.rows, 64); // length unchanged
    }

    #[test]
    fn test_delete_row() {
        let mut app = make_app();
        app.try_enter_note('z'); // C-4 at row 0
        app.try_enter_note('x'); // D-4 at row 1
        app.try_enter_note('c'); // E-4 at row 2

        // Delete row 1 -> D-4 removed, E-4 shifts up
        app.cursor_row = 1;
        app.delete_row_at_cursor();

        let pat = &app.song.patterns[0];
        assert_eq!(pat.get(0, 0).note, Some(Note::On { value: NoteValue::C, octave: 4 }));
        assert_eq!(pat.get(1, 0).note, Some(Note::On { value: NoteValue::E, octave: 4 }));
        assert_eq!(pat.rows, 64); // length unchanged
    }

    // -- Per-pattern length tests --

    #[test]
    fn test_per_pattern_length_cursor_bounds() {
        let mut app = make_app();
        // Resize pattern 0 to 32 rows
        app.song.patterns[0].resize_rows(32);
        assert_eq!(app.current_pattern_rows(), 32);

        // Cursor should be clamped to 31
        app.move_cursor_down(100);
        assert_eq!(app.cursor_row, 31);
    }

    #[test]
    fn test_per_pattern_length_playback_advance() {
        let mut app = make_app();
        // Create a short pattern (8 rows) and a normal one
        app.song.patterns[0].resize_rows(8);
        app.song.add_pattern();
        app.song.order.push(1);

        app.play();
        // Advance through all 8 rows of pattern 0
        for _ in 0..8 {
            app.advance_playback();
        }
        // Should have wrapped to pattern 1
        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 0);
    }

    // -- MIDI CC effect test --

    #[test]
    fn test_midi_cc_effect_in_playback() {
        use crate::tracker::Cell;

        let mut app = make_app();
        // Put a MIDI CC effect at row 0: C40 with instrument=0x07 (volume controller)
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: Some(0x07),
            volume: None,
            effect: Some(EFFECT_MIDI_CC),
            effect_value: Some(0x40),
        });

        app.play();
        app.advance_playback(); // row 0 plays, CC should be sent
        // No crash = success (we can't easily verify MIDI output without a mock)
    }

    // -- Program change effect test --

    #[test]
    fn test_program_change_effect_in_playback() {
        use crate::tracker::Cell;

        let mut app = make_app();
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_PROGRAM_CHANGE),
            effect_value: Some(0x05),
        });

        app.play();
        app.advance_playback();
        // No crash = success
    }

    #[test]
    fn test_arpeggio_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 6;
        // Place C-4 with arpeggio 037 (minor chord: +3, +7 semitones)
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(0x7F),
            effect: Some(EFFECT_ARPEGGIO),
            effect_value: Some(0x37),
        });

        app.play();
        // Tick 0: note triggers, channel state set up
        app.process_tick();
        assert_eq!(app.channel_states[0].note, Some(48)); // C-4 = MIDI 48
        assert_eq!(app.channel_states[0].effect, Some(EFFECT_ARPEGGIO));

        // Tick 1: arpeggio processes -- pitch should shift
        app.process_tick();
        // Tick 2: different arpeggio phase
        app.process_tick();
        // No crash, effect processes correctly
    }

    #[test]
    fn test_portamento_up_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 4;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(0x7F),
            effect: Some(EFFECT_PORTA_UP),
            effect_value: Some(0x10), // slide up by 1 semitone per tick
        });

        app.play();
        app.process_tick(); // tick 0: note on
        assert_eq!(app.channel_states[0].pitch_offset, 0.0);

        app.process_tick(); // tick 1: pitch slides up
        assert!(app.channel_states[0].pitch_offset > 0.0);
        let after_one = app.channel_states[0].pitch_offset;

        app.process_tick(); // tick 2: pitch slides up more
        assert!(app.channel_states[0].pitch_offset > after_one);
    }

    #[test]
    fn test_portamento_down_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 4;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(0x7F),
            effect: Some(EFFECT_PORTA_DOWN),
            effect_value: Some(0x10),
        });

        app.play();
        app.process_tick(); // tick 0
        app.process_tick(); // tick 1: pitch slides down
        assert!(app.channel_states[0].pitch_offset < 0.0);
    }

    #[test]
    fn test_tone_portamento_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 6;
        // Row 0: trigger C-4
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(0x7F),
            effect: None,
            effect_value: None,
        });
        // Row 1: tone porta to E-4 (4 semitones up)
        app.song.patterns[0].set_cell(1, 0, Cell {
            note: Some(Note::On { value: NoteValue::E, octave: 4 }),
            instrument: None,
            volume: None,
            effect: Some(EFFECT_TONE_PORTA),
            effect_value: Some(0x20), // speed
        });

        app.play();
        // Process all ticks of row 0
        for _ in 0..6 { app.process_tick(); }
        assert_eq!(app.channel_states[0].note, Some(48)); // C-4

        // Row 1 tick 0: target set to E-4 (52), no retrigger
        app.process_tick();
        assert_eq!(app.channel_states[0].porta_target, Some(52));
        assert_eq!(app.channel_states[0].note, Some(48)); // still C-4

        // Tick 1+: pitch slides toward target
        app.process_tick();
        assert!(app.channel_states[0].pitch_offset > 0.0);
    }

    #[test]
    fn test_vibrato_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 6;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(0x7F),
            effect: Some(EFFECT_VIBRATO),
            effect_value: Some(0x48), // speed=4, depth=8
        });

        app.play();
        app.process_tick(); // tick 0: note on
        assert_eq!(app.channel_states[0].vibrato_phase, 0.0);

        app.process_tick(); // tick 1: vibrato advances
        assert!(app.channel_states[0].vibrato_phase > 0.0);
    }

    #[test]
    fn test_volume_slide_effect() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 4;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(100),
            effect: Some(EFFECT_VOLUME_SLIDE),
            effect_value: Some(0x02), // slide down by 2 per tick
        });

        app.play();
        app.process_tick(); // tick 0: note on at volume 100
        assert_eq!(app.channel_states[0].volume, 100);

        app.process_tick(); // tick 1: volume slides down by 2
        assert_eq!(app.channel_states[0].volume, 98);

        app.process_tick(); // tick 2
        assert_eq!(app.channel_states[0].volume, 96);
    }

    #[test]
    fn test_volume_slide_up() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 4;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(100),
            effect: Some(EFFECT_VOLUME_SLIDE),
            effect_value: Some(0x30), // slide up by 3 per tick
        });

        app.play();
        app.process_tick(); // tick 0
        app.process_tick(); // tick 1
        assert_eq!(app.channel_states[0].volume, 103);
    }

    #[test]
    fn test_volume_slide_clamps() {
        use crate::tracker::{Cell, Note, NoteValue};

        let mut app = make_app();
        app.song.speed = 4;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 4 }),
            instrument: None,
            volume: Some(2),
            effect: Some(EFFECT_VOLUME_SLIDE),
            effect_value: Some(0x0F), // slide down by 15 per tick
        });

        app.play();
        app.process_tick(); // tick 0
        app.process_tick(); // tick 1: 2 - 15 = clamped to 0
        assert_eq!(app.channel_states[0].volume, 0);
    }

    #[test]
    fn test_set_speed_effect() {
        use crate::tracker::Cell;

        let mut app = make_app();
        assert_eq!(app.song.speed, 6);
        app.song.patterns[0].set_cell(0, 0, Cell {
            effect: Some(EFFECT_SET_SPEED),
            effect_value: Some(3),
            ..Cell::default()
        });

        app.play();
        app.process_tick(); // tick 0 processes row with Fxx
        assert_eq!(app.song.speed, 3);
    }

    #[test]
    fn test_set_tempo_effect() {
        use crate::tracker::Cell;

        let mut app = make_app();
        assert_eq!(app.song.bpm, 120);
        app.song.patterns[0].set_cell(0, 0, Cell {
            effect: Some(EFFECT_SET_SPEED),
            effect_value: Some(0x80), // >= 0x20, sets BPM to 128
            ..Cell::default()
        });

        app.play();
        app.process_tick();
        assert_eq!(app.song.bpm, 0x80); // 128
    }

    #[test]
    fn test_sub_tick_timing() {
        let mut app = make_app();
        app.song.speed = 3;
        app.play();
        assert_eq!(app.playback_tick, 0);

        app.process_tick(); // tick 0: advances row
        assert_eq!(app.playback_tick, 1);

        app.process_tick(); // tick 1: effects only
        assert_eq!(app.playback_tick, 2);

        app.process_tick(); // tick 2: effects only, wraps to 0
        assert_eq!(app.playback_tick, 0);
    }

    #[test]
    fn test_note_delay_effect() {
        use crate::tracker::Cell;

        let mut app = make_app();
        app.song.speed = 6;
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(100),
            effect: Some(EFFECT_NOTE_DELAY),
            effect_value: Some(3),
            ..Cell::default()
        });

        app.play();
        app.process_tick(); // tick 0: note deferred, not triggered yet
        assert!(app.channel_states[0].note.is_none());
        assert!(app.channel_states[0].delayed_note.is_some());

        app.process_tick(); // tick 1: not yet
        assert!(app.channel_states[0].note.is_none());

        app.process_tick(); // tick 2: not yet
        assert!(app.channel_states[0].note.is_none());

        app.process_tick(); // tick 3: trigger!
        assert_eq!(app.channel_states[0].note, Some(60));
        assert!(app.channel_states[0].delayed_note.is_none());
    }

    #[test]
    fn test_note_delay_off() {
        use crate::tracker::Cell;

        let mut app = make_app();
        app.song.speed = 6;

        // Row 0: normal note on
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(100),
            ..Cell::default()
        });
        // Row 1: delayed note-off
        app.song.patterns[0].set_cell(1, 0, Cell {
            note: Some(Note::Off),
            effect: Some(EFFECT_NOTE_DELAY),
            effect_value: Some(2),
            ..Cell::default()
        });

        app.play();
        // Process all ticks for row 0 to trigger note
        for _ in 0..6 {
            app.process_tick();
        }
        assert_eq!(app.channel_states[0].note, Some(60));

        // Row 1: tick 0 -- note-off is deferred
        app.process_tick();
        assert_eq!(app.channel_states[0].note, Some(60)); // still on

        app.process_tick(); // tick 1: not yet
        assert_eq!(app.channel_states[0].note, Some(60));

        app.process_tick(); // tick 2: note-off fires
        assert!(app.channel_states[0].note.is_none());
    }

    // -- MIDI input tests --

    #[test]
    fn test_midi_input_in_insert_mode() {
        let mut app = make_app();
        app.mode = Mode::Insert;

        let event = MidiInputEvent {
            channel: 0,
            note: 60, // C-5 (MIDI note 60 = C5 since octave = 60/12 = 5)
            velocity: 100,
        };

        app.handle_midi_input(event);

        let cell = app.song.patterns[0].get(0, 0);
        assert_eq!(cell.note, Some(Note::On { value: NoteValue::C, octave: 5 }));
        assert_eq!(cell.volume, Some(100));
        assert_eq!(app.cursor_row, 1); // advanced by edit_step
    }

    #[test]
    fn test_midi_input_ignored_in_normal_mode() {
        let mut app = make_app();
        app.mode = Mode::Normal;

        let event = MidiInputEvent {
            channel: 0,
            note: 60,
            velocity: 100,
        };

        app.handle_midi_input(event);
        // Should not write to pattern
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
    }

    #[test]
    fn test_midi_input_ignored_during_playback() {
        let mut app = make_app();
        app.mode = Mode::Insert;
        app.play();

        let event = MidiInputEvent {
            channel: 0,
            note: 60,
            velocity: 100,
        };

        app.handle_midi_input(event);
        // Should not write to pattern during playback
        assert!(app.song.patterns[0].get(0, 0).note.is_none());
    }

    #[test]
    fn test_pattern_break_clamps_row() {
        use crate::tracker::Cell;

        let mut app = make_app();
        app.song.add_pattern();
        app.song.order.push(1);

        // Break to row 0xFF but pattern only has 64 rows -> clamp to 63
        app.song.patterns[0].set_cell(0, 0, Cell {
            note: None,
            instrument: None,
            volume: None,
            effect: Some(EFFECT_PATTERN_BREAK),
            effect_value: Some(0xFF),
        });

        app.play();
        app.advance_playback();

        assert_eq!(app.playback_order, 1);
        assert_eq!(app.playback_row, 63); // clamped to rows_per_pattern - 1
    }

    // -- Tier 4: Song settings tests --

    #[test]
    fn test_song_settings_open_close() {
        let mut app = make_app();
        app.mode = Mode::Normal;
        app.open_song_settings();
        assert_eq!(app.mode, Mode::SongSettings);
        assert_eq!(app.settings_field, SettingsField::Title);
        assert_eq!(app.settings_edit_buf, "Untitled");

        app.close_song_settings();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_song_settings_edit_title() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_edit_buf.clear();
        app.settings_edit_buf.push_str("NewTitle");
        app.settings_apply_field();
        assert_eq!(app.song.title, "NewTitle");
    }

    #[test]
    fn test_song_settings_edit_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_select_field(SettingsField::Bpm);
        app.settings_edit_buf.clear();
        app.settings_edit_buf.push_str("140");
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 140);
    }

    #[test]
    fn test_song_settings_edit_channels() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_select_field(SettingsField::Channels);
        app.settings_edit_buf.clear();
        app.settings_edit_buf.push_str("8");
        app.settings_apply_field();
        assert_eq!(app.song.channels, 8);
        assert_eq!(app.song.patterns[0].channels, 8);
        assert_eq!(app.muted_channels.len(), 8);
    }

    #[test]
    fn test_song_settings_clamps_bpm() {
        let mut app = make_app();
        app.open_song_settings();
        app.settings_select_field(SettingsField::Bpm);
        app.settings_edit_buf.clear();
        app.settings_edit_buf.push_str("9999");
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 300); // clamped

        app.settings_edit_buf.clear();
        app.settings_edit_buf.push_str("1");
        app.settings_apply_field();
        assert_eq!(app.song.bpm, 32); // clamped
    }

    // -- Tier 4: Instrument list tests --

    #[test]
    fn test_instrument_list_open_close() {
        let mut app = make_app();
        app.open_instrument_list();
        assert_eq!(app.mode, Mode::InstrumentList);
        app.close_instrument_list();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_instrument_list_navigation() {
        let mut app = make_app();
        app.open_instrument_list();
        assert_eq!(app.instrument_cursor, 0);

        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(down);
        assert_eq!(app.instrument_cursor, 1);

        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        app.handle_key(up);
        assert_eq!(app.instrument_cursor, 0);

        // Can't go below 0
        app.handle_key(up);
        assert_eq!(app.instrument_cursor, 0);
    }

    #[test]
    fn test_instrument_name_edit() {
        let mut app = make_app();
        app.open_instrument_list();
        assert!(app.instruments[0].name.is_empty());

        // Type a name
        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.instruments[0].name, "Pad");

        // Backspace
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.instruments[0].name, "Pa");
    }

    // -- Tier 4: Theme cycling test --

    #[test]
    fn test_theme_cycling() {
        let mut app = make_app();
        assert_eq!(app.theme_index, 0);

        app.cycle_theme();
        assert_eq!(app.theme_index, 1);

        app.cycle_theme();
        assert_eq!(app.theme_index, 2);

        app.cycle_theme();
        assert_eq!(app.theme_index, 0); // wraps
    }

    // -- Tier 4: MIDI clock toggle test --

    #[test]
    fn test_midi_clock_toggle() {
        let mut app = make_app();
        assert!(!app.midi.clock_enabled);

        app.toggle_midi_clock();
        assert!(app.midi.clock_enabled);

        app.toggle_midi_clock();
        assert!(!app.midi.clock_enabled);
    }

    // -- Tier 4: Mouse click test --

    #[test]
    fn test_mouse_scroll() {
        let mut app = make_app();
        app.cursor_row = 10;

        let scroll_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(scroll_up, 3, 7);
        assert_eq!(app.cursor_row, 7); // moved up 3

        let scroll_down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(scroll_down, 3, 7);
        assert_eq!(app.cursor_row, 10); // moved down 3
    }

    // -- Tier 4: F6 opens song settings --

    #[test]
    fn test_f6_opens_settings() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.mode, Mode::SongSettings);
    }

    // -- Tier 4: F7 opens instrument list --

    #[test]
    fn test_f7_opens_instruments() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.mode, Mode::InstrumentList);
    }

    // -- Tier 4: F8 cycles theme --

    #[test]
    fn test_f8_cycles_theme() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.theme_index, 1);
    }

    // -- Tier 4: Ctrl+E exports MIDI --

    #[test]
    fn test_ctrl_e_exports_midi() {
        let mut app = make_app();
        let tmp = std::env::temp_dir().join("rtrack_test_export.mid");
        app.file_path = Some(tmp.with_extension("rtrk"));
        app.try_enter_note('z'); // put a note so there's content

        let key = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(tmp.exists());
        let _ = std::fs::remove_file(tmp);
    }

    // -- Tier 4: Ctrl+M toggles MIDI clock --

    #[test]
    fn test_ctrl_m_toggles_clock() {
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(app.midi.clock_enabled);

        app.handle_key(key);
        assert!(!app.midi.clock_enabled);
    }
}
