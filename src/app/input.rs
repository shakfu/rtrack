use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::tracker::Note;
use crate::tracker::NoteValue;
use crate::ui::pattern_editor::SubColumn;

use super::{
    App, Mode, SampleField, SettingsField,
    CHANNELS_PER_PAGE, MAX_CHANNELS, MAX_INSTRUMENTS,
};

impl App {
    // -- Song settings dialog --

    pub(crate) fn open_song_settings(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::SongSettings;
        self.settings_field = SettingsField::Title;
        self.settings_edit_buf = self.song.title.clone();
    }

    fn close_song_settings(&mut self) {
        self.mode = Mode::Normal;
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

    pub(crate) fn settings_apply_field(&mut self) {
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
                    let v = v.clamp(1, MAX_CHANNELS);
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
                        self.channel_names.resize(v, String::new());
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

    pub(crate) fn open_instrument_list(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::InstrumentList;
    }

    fn close_instrument_list(&mut self) {
        self.mode = Mode::Normal;
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
                if self.instrument_cursor < MAX_INSTRUMENTS - 1 {
                    self.instrument_cursor += 1;
                }
            }
            KeyCode::PageUp => {
                self.instrument_cursor = self.instrument_cursor.saturating_sub(16);
            }
            KeyCode::PageDown => {
                self.instrument_cursor = (self.instrument_cursor + 16).min(MAX_INSTRUMENTS - 1);
            }
            KeyCode::Enter => {
                // Open sample editor for current instrument
                self.open_sample_editor();
            }
            KeyCode::Tab => {
                // Open synth editor for current instrument
                self.open_synth_editor();
            }
            KeyCode::Char(c) => {
                self.instruments[self.instrument_cursor].name.push(c);
                self.dirty = true;
            }
            KeyCode::Backspace => {
                self.instruments[self.instrument_cursor].name.pop();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn handle_sample_editor_key(&mut self, key: KeyEvent) {
        let slot = self.sample_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.dirty = true;
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

    // -- Synth editor --

    fn handle_synth_editor_key(&mut self, key: KeyEvent) {
        let slot = self.synth_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.dirty = true;
                self.status_message = Some(format!("Synth params saved for instrument {:02X}", slot));
            }
            KeyCode::Tab => {
                self.synth_editor_field = self.synth_editor_field.next();
            }
            KeyCode::BackTab => {
                self.synth_editor_field = self.synth_editor_field.prev();
            }
            KeyCode::Up => {
                self.adjust_synth_field(slot, 1);
            }
            KeyCode::Down => {
                self.adjust_synth_field(slot, -1);
            }
            KeyCode::Right => {
                self.adjust_synth_field(slot, 10);
            }
            KeyCode::Left => {
                self.adjust_synth_field(slot, -10);
            }
            KeyCode::Delete => {
                // Clear custom synth params (revert to channel default)
                self.instruments[slot].synth_params = None;
                self.dirty = true;
                self.status_message = Some("Synth params cleared (using channel default)".to_string());
                self.mode = self.prev_mode;
            }
            _ => {}
        }
    }

    fn adjust_synth_field(&mut self, slot: usize, delta: i32) {
        use crate::app::SynthField;
        use crate::audio::synth::Patch;

        if let Some(ref mut params) = self.instruments[slot].synth_params {
            match self.synth_editor_field {
                SynthField::Waveform => {
                    let max = Patch::count() as i32;
                    params.waveform = ((params.waveform as i32 + delta).rem_euclid(max)) as u8;
                }
                SynthField::Attack => {
                    params.attack = (params.attack + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::Decay => {
                    params.decay = (params.decay + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::Sustain => {
                    params.sustain = (params.sustain + delta as f32 * 0.01).clamp(0.0, 1.0);
                }
                SynthField::Release => {
                    params.release = (params.release + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::FilterCutoff => {
                    params.filter_cutoff = (params.filter_cutoff + delta as f32 * 0.1).clamp(0.1, 40.0);
                }
                SynthField::FilterResonance => {
                    params.filter_resonance = (params.filter_resonance + delta as f32 * 0.01).clamp(0.0, 0.95);
                }
                SynthField::FilterEnv => {
                    params.filter_env = (params.filter_env + delta as f32 * 0.1).clamp(0.0, 8.0);
                }
                SynthField::Detune => {
                    params.detune = (params.detune + delta as f32 * 0.1).clamp(0.0, 50.0);
                }
            }
        }
    }

    fn handle_quit_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.should_quit = true;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save();
                self.should_quit = true;
            }
            _ => {
                // Any other key cancels
                self.mode = self.prev_mode;
                self.status_message = Some("Quit cancelled".to_string());
            }
        }
    }

    // -- Channel rename --

    fn open_channel_rename(&mut self) {
        let ch = self.cursor_channel;
        self.rename_buf = self.channel_names.get(ch).cloned().unwrap_or_default();
        self.prev_mode = self.mode;
        self.mode = Mode::ChannelRename;
    }

    fn handle_channel_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                let ch = self.cursor_channel;
                if ch < self.channel_names.len() {
                    self.channel_names[ch] = self.rename_buf.clone();
                }
                self.mode = self.prev_mode;
                if self.rename_buf.is_empty() {
                    self.status_message = Some(format!("Ch {} name cleared", ch + 1));
                } else {
                    self.status_message = Some(format!("Ch {} = \"{}\"", ch + 1, self.rename_buf));
                }
            }
            KeyCode::Char(c) => {
                if self.rename_buf.len() < 10 {
                    self.rename_buf.push(c);
                }
            }
            KeyCode::Backspace => {
                self.rename_buf.pop();
            }
            _ => {}
        }
    }

    // -- Note transpose --

    /// Transpose selected notes by the given number of semitones.
    /// If block selection is active, transpose the block; otherwise transpose at cursor.
    fn transpose_notes(&mut self, semitones: i8) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];

        if let Some((anchor_row, anchor_ch)) = self.block_anchor {
            let (r0, r1) = if anchor_row <= self.cursor_row {
                (anchor_row, self.cursor_row)
            } else {
                (self.cursor_row, anchor_row)
            };
            let (c0, c1) = if anchor_ch <= self.cursor_channel {
                (anchor_ch, self.cursor_channel)
            } else {
                (self.cursor_channel, anchor_ch)
            };
            let pattern = &mut self.song.patterns[pattern_idx];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    transpose_cell_note(pattern.get_mut(r, c), semitones);
                }
            }
            self.status_message = Some(format!("Transposed block by {} semitone(s)", semitones));
        } else {
            let pattern = &mut self.song.patterns[pattern_idx];
            transpose_cell_note(pattern.get_mut(self.cursor_row, self.cursor_channel), semitones);
        }
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
        let actual_ch = self.track_page * CHANNELS_PER_PAGE + ch as usize;
        if actual_ch < pattern.channels && actual_ch < self.song.channels {
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
            Mode::SynthEditor => self.handle_synth_editor_key(key),
            Mode::QuitConfirm => self.handle_quit_confirm_key(key),
            Mode::ChannelRename => self.handle_channel_rename_key(key),
        }
    }

    fn handle_common_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => { self.save(); return true; }
                KeyCode::Char('z') => { self.undo(); return true; }
                KeyCode::Char('y') => { self.redo(); return true; }
                KeyCode::Char('b') => { self.toggle_block_select(); return true; }
                KeyCode::Char('f') => { self.toggle_follow(); return true; }
                KeyCode::Char('i') => { self.interpolate_block(); return true; }
                KeyCode::Char('r') => { self.open_channel_rename(); return true; }
                KeyCode::Char('c') => {
                    if self.block_anchor.is_some() {
                        self.copy_block();
                    } else {
                        self.copy_row();
                    }
                    return true;
                }
                KeyCode::Char('v') => {
                    if self.block_clipboard.is_some() {
                        self.paste_block();
                    } else {
                        self.paste_row();
                    }
                    return true;
                }
                KeyCode::Char('x') => {
                    if self.block_anchor.is_some() {
                        self.cut_block();
                    } else {
                        self.cut_row();
                    }
                    return true;
                }
                KeyCode::Right => { self.next_order_position(); return true; }
                KeyCode::Left => { self.prev_order_position(); return true; }
                KeyCode::Char('e') => { self.export_midi(); return true; }
                KeyCode::Char('w') => { self.export_wav_file(); return true; }
                KeyCode::Char('l') => { self.export_flac_file(); return true; }
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
                KeyCode::F(9) => { let ch = self.track_page * CHANNELS_PER_PAGE; self.toggle_solo(ch); return true; }
                KeyCode::F(10) => { let ch = self.track_page * CHANNELS_PER_PAGE + 1; self.toggle_solo(ch); return true; }
                KeyCode::F(11) => { let ch = self.track_page * CHANNELS_PER_PAGE + 2; self.toggle_solo(ch); return true; }
                KeyCode::F(12) => { let ch = self.track_page * CHANNELS_PER_PAGE + 3; self.toggle_solo(ch); return true; }
                _ => {}
            }
        }
        // F9-F12: mute channels on current page
        match key.code {
            KeyCode::F(9) => { let ch = self.track_page * CHANNELS_PER_PAGE; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(10) => { let ch = self.track_page * CHANNELS_PER_PAGE + 1; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(11) => { let ch = self.track_page * CHANNELS_PER_PAGE + 2; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(12) => { let ch = self.track_page * CHANNELS_PER_PAGE + 3; self.toggle_channel_mute(ch); return true; }
            _ => {}
        }
        false
    }

    /// Handle keys shared between Normal and Insert modes: navigation, playback,
    /// track paging, function keys, octave, BPM, edit step.
    fn handle_shared_key(&mut self, key: KeyEvent) -> bool {
        // Shift+Up/Down: note transpose
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => { self.transpose_notes(1); return true; }
                KeyCode::Down => { self.transpose_notes(-1); return true; }
                _ => {}
            }
        }

        match key.code {
            // Playback
            KeyCode::Char(' ') => { self.toggle_playback(); return true; }

            // Function keys shared by both modes
            KeyCode::F(1) => { self.open_help(); return true; }
            KeyCode::F(2) => { self.open_port_selector(); return true; }
            KeyCode::F(3) => { self.toggle_link(); return true; }
            KeyCode::F(6) => { self.open_song_settings(); return true; }
            KeyCode::F(7) => { self.open_instrument_list(); return true; }
            KeyCode::F(8) => { self.cycle_theme(); return true; }

            // Track page navigation
            KeyCode::Tab => { self.toggle_track_page(); return true; }
            KeyCode::BackTab => { self.reverse_track_page(); return true; }

            // Navigation
            KeyCode::Up => { self.move_cursor_up(1); return true; }
            KeyCode::Down => { self.move_cursor_down(1); return true; }
            KeyCode::Left => { self.move_cursor_left(); return true; }
            KeyCode::Right => { self.move_cursor_right(); return true; }
            KeyCode::PageUp => { self.move_cursor_up(16); return true; }
            KeyCode::PageDown => { self.move_cursor_down(16); return true; }
            KeyCode::Home => { self.cursor_row = 0; return true; }
            KeyCode::End => { self.cursor_row = self.current_pattern_rows() - 1; return true; }

            // Octave up
            KeyCode::Char('+') => {
                if self.current_octave < 9 { self.current_octave += 1; }
                return true;
            }

            _ => {}
        }
        false
    }

    /// Reverse track page (Shift-Tab behavior), extracted from duplication.
    fn reverse_track_page(&mut self) {
        let max_pages = (self.song.channels + CHANNELS_PER_PAGE - 1) / CHANNELS_PER_PAGE;
        if max_pages > 1 {
            self.track_page = if self.track_page == 0 { max_pages - 1 } else { self.track_page - 1 };
            let page_start = self.track_page * CHANNELS_PER_PAGE;
            let page_end = (page_start + CHANNELS_PER_PAGE).min(self.song.channels);
            if self.cursor_channel < page_start || self.cursor_channel >= page_end {
                self.cursor_channel = page_start;
                self.cursor_sub = SubColumn::Note;
            }
            self.status_message = Some(format!("Track page {} (ch {}-{})", self.track_page + 1, page_start + 1, page_end));
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) { return; }
        if self.handle_shared_key(key) { return; }

        // Ctrl combos specific to Normal mode
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => { self.add_new_pattern_to_order(); return; }
                KeyCode::Char('d') => { self.clone_current_pattern(); return; }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('q') => {
                if self.dirty {
                    self.prev_mode = self.mode;
                    self.mode = Mode::QuitConfirm;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Esc => self.mode = Mode::Insert,
            KeyCode::F(4) => self.insert_order_entry(),
            KeyCode::F(5) => self.remove_order_entry(),

            // Octave down (plain '-' in Normal mode)
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
        if self.handle_shared_key(key) { return; }

        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,

            // Octave down (Ctrl+- in Insert mode since '-' could be hex)
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
        self.help_scroll = 0;
    }

    fn close_help(&mut self) {
        self.mode = Mode::Normal;
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') => self.close_help(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.help_scroll += 1;
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.help_scroll += 10;
            }
            KeyCode::Home => {
                self.help_scroll = 0;
            }
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

        // Preview the note (check current cell for instrument assignment)
        let pattern_idx = self.song.order[self.current_order_position()];
        let current_inst = self.song.patterns[pattern_idx].get(self.cursor_row, self.cursor_channel).instrument;
        if let Some(midi_note) = note.to_midi_note() {
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.preview_note_with_instrument(midi_ch, midi_note, 0x7F, current_inst);
        }

        // Write to pattern
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

    pub(crate) fn move_cursor_up(&mut self, amount: usize) {
        if self.cursor_row >= amount {
            self.cursor_row -= amount;
        } else {
            self.cursor_row = 0;
        }
    }

    pub(crate) fn move_cursor_down(&mut self, amount: usize) {
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

    pub(crate) fn insert_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].insert_row(self.cursor_row);
        self.status_message = Some(format!("Inserted row at {:02X}", self.cursor_row));
    }

    pub(crate) fn delete_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].delete_row(self.cursor_row);
        self.status_message = Some(format!("Deleted row at {:02X}", self.cursor_row));
    }

    /// Toggle track page (0 = tracks 1-4, 1 = tracks 5-8, etc.)
    fn toggle_track_page(&mut self) {
        let max_pages = (self.song.channels + CHANNELS_PER_PAGE - 1) / CHANNELS_PER_PAGE;
        if max_pages <= 1 {
            return;
        }
        self.track_page = (self.track_page + 1) % max_pages;
        // Move cursor to same relative position on the new page
        let page_start = self.track_page * CHANNELS_PER_PAGE;
        let page_end = (page_start + CHANNELS_PER_PAGE).min(self.song.channels);
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
            self.track_page = track / CHANNELS_PER_PAGE;
        }
    }

    fn toggle_follow(&mut self) {
        self.follow_playback = !self.follow_playback;
        let state = if self.follow_playback { "on" } else { "off" };
        self.status_message = Some(format!("Follow mode {}", state));
    }

    pub(crate) fn move_cursor_left(&mut self) {
        if self.cursor_sub == SubColumn::Note {
            if self.cursor_channel > 0 {
                self.cursor_channel -= 1;
                self.cursor_sub = SubColumn::Effect;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.prev();
        }
    }

    pub(crate) fn move_cursor_right(&mut self) {
        if self.cursor_sub == SubColumn::Effect {
            if self.cursor_channel < self.song.channels - 1 {
                self.cursor_channel += 1;
                self.cursor_sub = SubColumn::Note;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.next();
        }
    }

    // -- Block selection --

    /// Toggle block selection anchor at the current cursor position
    pub fn toggle_block_select(&mut self) {
        if self.block_anchor.is_some() {
            self.block_anchor = None;
            self.status_message = Some("Block selection cleared".to_string());
        } else {
            self.block_anchor = Some((self.cursor_row, self.cursor_channel));
            self.status_message = Some("Block selection started".to_string());
        }
    }

    /// Get the block selection bounds: (row_start, row_end, ch_start, ch_end) inclusive
    pub fn block_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        self.block_anchor.map(|(anchor_row, anchor_ch)| {
            let (r0, r1) = if anchor_row <= self.cursor_row {
                (anchor_row, self.cursor_row)
            } else {
                (self.cursor_row, anchor_row)
            };
            let (c0, c1) = if anchor_ch <= self.cursor_channel {
                (anchor_ch, self.cursor_channel)
            } else {
                (self.cursor_channel, anchor_ch)
            };
            (r0, r1, c0, c1)
        })
    }

    /// Copy the block selection to clipboard (2D grid)
    fn copy_block(&mut self) {
        if let Some((r0, r1, c0, c1)) = self.block_bounds() {
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &self.song.patterns[pattern_idx];
            let mut block = Vec::new();
            for r in r0..=r1 {
                let mut row = Vec::new();
                for c in c0..=c1 {
                    row.push(*pattern.get(r, c));
                }
                block.push(row);
            }
            self.block_clipboard = Some(block);
            let rows = r1 - r0 + 1;
            let cols = c1 - c0 + 1;
            self.status_message = Some(format!("Copied block {}x{}", rows, cols));
        }
    }

    /// Cut block selection (copy + clear)
    fn cut_block(&mut self) {
        self.copy_block();
        if let Some((r0, r1, c0, c1)) = self.block_bounds() {
            self.push_undo();
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &mut self.song.patterns[pattern_idx];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    pattern.set_cell(r, c, crate::tracker::Cell::default());
                }
            }
            self.block_anchor = None;
            self.status_message = Some("Cut block".to_string());
        }
    }

    /// Interpolate volume and effect values across a block selection.
    /// Uses the first and last row values as endpoints and fills intermediate rows linearly.
    fn interpolate_block(&mut self) {
        let bounds = match self.block_bounds() {
            Some(b) => b,
            None => {
                self.status_message = Some("No block selected (Ctrl+B first)".to_string());
                return;
            }
        };
        let (r0, r1, c0, c1) = bounds;
        if r1 <= r0 {
            self.status_message = Some("Need at least 2 rows to interpolate".to_string());
            return;
        }

        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &mut self.song.patterns[pattern_idx];
        let steps = (r1 - r0) as f64;

        for c in c0..=c1 {
            let first = *pattern.get(r0, c);
            let last = *pattern.get(r1, c);

            // Interpolate volume
            if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                for r in r0..=r1 {
                    let t = (r - r0) as f64 / steps;
                    let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                    pattern.get_mut(r, c).volume = Some(v.round() as u8);
                }
            }

            // Interpolate effect_value (when both endpoints have the same effect command)
            if first.effect == last.effect && first.effect.is_some() {
                if let (Some(ev0), Some(ev1)) = (first.effect_value, last.effect_value) {
                    for r in r0..=r1 {
                        let t = (r - r0) as f64 / steps;
                        let ev = ev0 as f64 + (ev1 as f64 - ev0 as f64) * t;
                        pattern.get_mut(r, c).effect = first.effect;
                        pattern.get_mut(r, c).effect_value = Some(ev.round() as u8);
                    }
                }
            }
        }

        self.status_message = Some("Interpolated block".to_string());
    }

    /// Paste block clipboard at cursor position
    fn paste_block(&mut self) {
        if let Some(ref block) = self.block_clipboard.clone() {
            self.push_undo();
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &mut self.song.patterns[pattern_idx];
            for (ri, row) in block.iter().enumerate() {
                let r = self.cursor_row + ri;
                if r >= pattern.rows {
                    break;
                }
                for (ci, cell) in row.iter().enumerate() {
                    let c = self.cursor_channel + ci;
                    if c < pattern.channels {
                        pattern.set_cell(r, c, *cell);
                    }
                }
            }
            self.status_message = Some("Pasted block".to_string());
        }
    }
}

/// Transpose a single cell's note by the given number of semitones, clamping to valid MIDI range.
fn transpose_cell_note(cell: &mut crate::tracker::Cell, semitones: i8) {
    if let Some(Note::On { ref value, ref octave }) = cell.note {
        let midi = (*octave as i16) * 12 + value.to_index() as i16 + semitones as i16;
        if midi >= 0 && midi <= 127 {
            let new_octave = (midi / 12) as u8;
            let new_note_idx = (midi % 12) as u8;
            if let Some(nv) = NoteValue::from_index(new_note_idx) {
                cell.note = Some(Note::On { value: nv, octave: new_octave });
            }
        }
    }
}
