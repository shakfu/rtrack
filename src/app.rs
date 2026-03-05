use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::audio::AudioEngine;
use crate::link::LinkEngine;
use crate::midi::{MidiEngine, MidiInputEngine, MidiInputEvent};
use crate::tracker::{Note, NoteValue, Song};
use crate::ui::pattern_editor::SubColumn;

// Effect commands (single hex digit, stored in Cell.effect)
const EFFECT_POSITION_JUMP: u8 = 0xB; // Bxx: jump to order position xx
const EFFECT_MIDI_CC: u8 = 0xC;       // Cxx: send MIDI CC (controller from instrument col, value xx)
const EFFECT_PATTERN_BREAK: u8 = 0xD; // Dxx: break to row xx of next pattern
const EFFECT_PROGRAM_CHANGE: u8 = 0xE; // Exx: program change to program xx

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    MidiPortSelect,
    Help,
    SongSettings,
    InstrumentList,
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

/// Instrument definition for the instrument list
#[derive(Debug, Clone)]
pub struct Instrument {
    pub name: String,
    pub midi_program: Option<u8>,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            name: String::new(),
            midi_program: None,
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

    // Audio engine (SF2 playback via RustySynth + cpal)
    pub audio: Option<AudioEngine>,
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
            audio: None,
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

    // -- Sound output helpers (dispatch to MIDI + optional audio engine) --

    fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref audio) = self.audio {
            audio.note_on(channel, note, velocity);
        }
    }

    fn send_channel_note_off(&mut self, channel: u8) {
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref audio) = self.audio {
            audio.note_off_all_channel(channel);
        }
    }

    fn send_all_notes_off(&mut self) {
        let _ = self.midi.all_notes_off();
        if let Some(ref audio) = self.audio {
            audio.note_off_all();
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
        match self.song.save(&path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
                self.status_message = Some(format!("Saved: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
            }
        }
    }

    pub fn load_file(&mut self, path: PathBuf) {
        match Song::load(&path) {
            Ok(song) => {
                self.muted_channels = vec![false; song.channels];
                self.solo_channel = None;
                self.midi_channel_map = (0..song.channels).map(|i| i as u8).collect();
                self.song = song;
                self.file_path = Some(path.clone());
                self.cursor_row = 0;
                self.cursor_channel = 0;
                self.cursor_sub = SubColumn::Note;
                self.edit_order = 0;
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.status_message = Some(format!("Loaded: {}", path.display()));
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
        self.last_tick = Some(Instant::now());
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;

        if self.link.is_enabled() {
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.last_tick = None;
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

            let spr = self.song.seconds_per_row();
            while self.tick_accumulator >= spr {
                self.tick_accumulator -= spr;
                self.advance_playback();
            }
        }
        self.last_tick = Some(now);
    }

    fn advance_playback(&mut self) {
        let pattern_idx = self.song.order[self.playback_order];
        let pattern_rows = self.song.patterns[pattern_idx].rows;
        let channels = self.song.patterns[pattern_idx].channels;

        // Collect cell data we need before mutating self
        let cells: Vec<(Option<Note>, Option<u8>, Option<u8>, Option<u8>, Option<u8>)> = (0..channels)
            .map(|ch| {
                let cell = self.song.patterns[pattern_idx].get(self.playback_row, ch);
                (cell.note, cell.volume, cell.effect, cell.effect_value, cell.instrument)
            })
            .collect();

        // Scan for pattern effects (first one wins)
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
                _ => {}
            }
        }

        // Play the current row
        for (ch, (note, volume, effect, effect_value, instrument)) in cells.into_iter().enumerate() {
            if !self.is_channel_audible(ch) {
                continue;
            }
            let midi_ch = self.midi_channel_for(ch);

            // Process notes
            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        let velocity = volume.unwrap_or(0x7F);
                        self.send_note_on(midi_ch, midi_note, velocity);
                    }
                }
                Some(Note::Off) => {
                    self.send_channel_note_off(midi_ch);
                }
                None => {}
            }

            // Process MIDI CC effect (Cxx: controller from instrument column, value xx)
            if effect == Some(EFFECT_MIDI_CC) {
                let controller = instrument.unwrap_or(0);
                let value = effect_value.unwrap_or(0);
                self.send_cc(midi_ch, controller, value);
            }

            // Process program change effect (Exx: change to program xx)
            if effect == Some(EFFECT_PROGRAM_CHANGE) {
                let program = effect_value.unwrap_or(0);
                self.send_program_change(midi_ch, program);
            }
        }

        // Process position jump (Bxx) -- jump to order position xx
        if let Some(target_order) = jump_order {
            let target = target_order.min(self.song.order.len() - 1);
            self.playback_order = target;
            let target_pattern = self.song.order[self.playback_order];
            let target_rows = self.song.patterns[target_pattern].rows;
            self.playback_row = break_row.unwrap_or(0).min(target_rows - 1);
            return;
        }

        // Process pattern break (Dxx) -- break to row xx of next pattern
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
                // Start editing the instrument name
                let inst = &self.instruments[self.instrument_cursor];
                self.settings_edit_buf = inst.name.clone();
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

        if ch < pattern.channels {
            self.cursor_channel = ch;
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
                KeyCode::Char('m') => { self.toggle_midi_clock(); return true; }
                KeyCode::F(9) => { self.toggle_solo(0); return true; }
                KeyCode::F(10) => { self.toggle_solo(1); return true; }
                KeyCode::F(11) => { self.toggle_solo(2); return true; }
                KeyCode::F(12) => { self.toggle_solo(3); return true; }
                _ => {}
            }
        }
        match key.code {
            KeyCode::F(9) => { self.toggle_channel_mute(0); return true; }
            KeyCode::F(10) => { self.toggle_channel_mute(1); return true; }
            KeyCode::F(11) => { self.toggle_channel_mute(2); return true; }
            KeyCode::F(12) => { self.toggle_channel_mute(3); return true; }
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

            // Track navigation
            KeyCode::Tab => self.next_channel(),
            KeyCode::BackTab => self.prev_channel(),

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
            KeyCode::Char('+') | KeyCode::Char('=') => {
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

            // Track navigation
            KeyCode::Tab => self.next_channel(),
            KeyCode::BackTab => self.prev_channel(),

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
            KeyCode::Char('+') | KeyCode::Char('=') => {
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

            // Note off
            KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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

    fn next_channel(&mut self) {
        if self.cursor_channel < self.song.channels - 1 {
            self.cursor_channel += 1;
            self.cursor_sub = SubColumn::Note;
        }
    }

    fn prev_channel(&mut self) {
        if self.cursor_channel > 0 {
            self.cursor_channel -= 1;
            self.cursor_sub = SubColumn::Note;
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_sub == SubColumn::Note {
            if self.cursor_channel > 0 {
                self.cursor_channel -= 1;
                self.cursor_sub = SubColumn::Effect;
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
            }
        } else {
            self.cursor_sub = self.cursor_sub.next();
        }
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
    fn test_tab_moves_to_next_channel() {
        let mut app = make_app();
        assert_eq!(app.cursor_channel, 0);

        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.handle_key(tab);
        assert_eq!(app.cursor_channel, 1);
        assert_eq!(app.cursor_sub, SubColumn::Note);

        // Shift-Tab goes back
        let stab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        app.handle_key(stab);
        assert_eq!(app.cursor_channel, 0);
        assert_eq!(app.cursor_sub, SubColumn::Note);
    }

    #[test]
    fn test_tab_channel_bounds() {
        let mut app = make_app();
        // Can't go below 0
        let stab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        app.handle_key(stab);
        assert_eq!(app.cursor_channel, 0);

        // Can't go past last channel
        app.cursor_channel = 3;
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.handle_key(tab);
        assert_eq!(app.cursor_channel, 3);
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
