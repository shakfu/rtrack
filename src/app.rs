use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::link::LinkEngine;
use crate::midi::MidiEngine;
use crate::tracker::{Note, NoteValue, Song};
use crate::ui::pattern_editor::SubColumn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    MidiPortSelect,
    Help,
}

pub struct App {
    pub song: Song,
    pub midi: MidiEngine,
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

    // MIDI port selection
    pub midi_port_list: Vec<String>,
    pub midi_port_cursor: usize,
    /// The mode to return to after closing the port selector
    prev_mode: Mode,
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

        let song = Song::new(4, 64);
        let link = LinkEngine::new(song.bpm as f64);

        Self {
            song,
            midi,
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
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            prev_mode: Mode::Normal,
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
            // In non-playing mode, use order position 0 for now
            // TODO: allow navigating order list
            0
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

        if self.link.is_enabled() {
            self.link.request_play();
        }
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.last_tick = None;
        let _ = self.midi.all_notes_off();

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
        let channels = self.song.patterns[pattern_idx].channels;

        // Collect cell data we need before mutating self
        let cells: Vec<(Option<Note>, Option<u8>)> = (0..channels)
            .map(|ch| {
                let cell = self.song.patterns[pattern_idx].get(self.playback_row, ch);
                (cell.note, cell.volume)
            })
            .collect();

        // Play the current row
        for (ch, (note, volume)) in cells.into_iter().enumerate() {
            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        let velocity = volume.unwrap_or(0x7F);
                        let _ = self.midi.note_on(ch as u8, midi_note, velocity);
                    }
                }
                Some(Note::Off) => {
                    let _ = self.midi.channel_note_off(ch as u8);
                }
                None => {}
            }
        }

        // Advance
        self.playback_row += 1;
        if self.playback_row >= self.song.rows_per_pattern {
            self.playback_row = 0;
            self.playback_order += 1;
            if self.playback_order >= self.song.order.len() {
                self.playback_order = 0;
            }
        }
    }

    // -- Input handling --

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            Mode::MidiPortSelect => self.handle_port_select_key(key),
            Mode::Help => self.handle_help_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.mode = Mode::Insert,
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_port_selector(),
            KeyCode::F(3) => self.toggle_link(),

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
            KeyCode::End => self.cursor_row = self.song.rows_per_pattern - 1,

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

            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_port_selector(),
            KeyCode::F(3) => self.toggle_link(),

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
            KeyCode::End => self.cursor_row = self.song.rows_per_pattern - 1,

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

        // Preview the note via MIDI
        if let Some(midi_note) = note.to_midi_note() {
            let _ = self.midi.note_on(self.cursor_channel as u8, midi_note, 0x7F);
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
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        cell.note = Some(Note::Off);
        self.move_cursor_down(self.edit_step);
    }

    fn delete_at_cursor(&mut self) {
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
        let max = self.song.rows_per_pattern - 1;
        self.cursor_row = (self.cursor_row + amount).min(max);
    }

    fn change_bpm(&mut self, delta: i16) {
        let new_bpm = (self.song.bpm as i16 + delta).clamp(32, 300) as u16;
        self.song.bpm = new_bpm;
        if self.link.is_enabled() {
            self.link.set_tempo(new_bpm as f64);
        }
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
            midi_port_list: Vec::new(),
            midi_port_cursor: 0,
            prev_mode: Mode::Normal,
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
}
