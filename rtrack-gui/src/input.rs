use egui::Key;
use rtrack_core::constants::*;
use rtrack_core::tracker::{Cell, Note, NoteValue};
use rtrack_core::ChannelType;

use crate::app::RtrackApp;
use crate::history::CellEdit;
use crate::state::{Mode, SubColumn};

impl RtrackApp {
    pub fn process_keys(&mut self, ctx: &egui::Context) {
        let mut actions: Vec<Action> = Vec::new();

        ctx.input(|input| {
            let ctrl = input.modifiers.ctrl || input.modifiers.mac_cmd;
            let shift = input.modifiers.shift;

            // Global keys (both modes)
            if input.key_pressed(Key::Space) {
                if ctrl {
                    actions.push(Action::PlayFromStart);
                } else {
                    actions.push(Action::TogglePlayback);
                }
            }
            if input.key_pressed(Key::Escape) {
                actions.push(Action::SetMode(Mode::Normal));
            }
            if input.key_pressed(Key::F1) {
                // Help (not implemented yet)
            }
            if ctrl && input.key_pressed(Key::S) {
                actions.push(Action::Save);
            }
            if ctrl && input.key_pressed(Key::Z) {
                if shift {
                    actions.push(Action::Redo);
                } else {
                    actions.push(Action::Undo);
                }
            }
            if ctrl && input.key_pressed(Key::C) {
                actions.push(Action::Copy);
            }
            if ctrl && input.key_pressed(Key::X) {
                actions.push(Action::Cut);
            }
            if ctrl && input.key_pressed(Key::V) {
                actions.push(Action::Paste);
            }

            // Octave
            if input.key_pressed(Key::Minus) {
                actions.push(Action::OctaveDown);
            }
            if input.key_pressed(Key::Plus) || (shift && input.key_pressed(Key::Equals)) {
                actions.push(Action::OctaveUp);
            }

            match self.mode {
                Mode::Normal => {
                    if input.key_pressed(Key::I) && !ctrl {
                        actions.push(Action::SetMode(Mode::Insert));
                    }
                    // Navigation
                    if input.key_pressed(Key::ArrowUp) {
                        actions.push(Action::CursorUp(1));
                    }
                    if input.key_pressed(Key::ArrowDown) {
                        actions.push(Action::CursorDown(1));
                    }
                    if input.key_pressed(Key::ArrowLeft) {
                        if ctrl {
                            actions.push(Action::PrevOrder);
                        } else {
                            actions.push(Action::CursorLeft);
                        }
                    }
                    if input.key_pressed(Key::ArrowRight) {
                        if ctrl {
                            actions.push(Action::NextOrder);
                        } else {
                            actions.push(Action::CursorRight);
                        }
                    }
                    if input.key_pressed(Key::PageUp) {
                        actions.push(Action::CursorUp(16));
                    }
                    if input.key_pressed(Key::PageDown) {
                        actions.push(Action::CursorDown(16));
                    }
                    if input.key_pressed(Key::Home) {
                        actions.push(Action::CursorHome);
                    }
                    if input.key_pressed(Key::End) {
                        actions.push(Action::CursorEnd);
                    }
                    if input.key_pressed(Key::Tab) {
                        if shift {
                            actions.push(Action::PrevChannel);
                        } else {
                            actions.push(Action::NextChannel);
                        }
                    }
                }
                Mode::Insert => {
                    // Navigation
                    if input.key_pressed(Key::ArrowUp) {
                        actions.push(Action::CursorUp(1));
                    }
                    if input.key_pressed(Key::ArrowDown) {
                        actions.push(Action::CursorDown(1));
                    }
                    if input.key_pressed(Key::ArrowLeft) {
                        if ctrl {
                            actions.push(Action::PrevOrder);
                        } else {
                            actions.push(Action::CursorLeft);
                        }
                    }
                    if input.key_pressed(Key::ArrowRight) {
                        if ctrl {
                            actions.push(Action::NextOrder);
                        } else {
                            actions.push(Action::CursorRight);
                        }
                    }
                    if input.key_pressed(Key::Tab) {
                        if shift {
                            actions.push(Action::PrevChannel);
                        } else {
                            actions.push(Action::NextChannel);
                        }
                    }
                    if input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace) {
                        actions.push(Action::ClearCell);
                    }

                    // Note-off
                    if input.key_pressed(Key::Equals) && self.cursor_sub == SubColumn::Note {
                        actions.push(Action::NoteOff);
                    }

                    // Piano keys and hex digits via text events
                    for event in &input.events {
                        if let egui::Event::Text(text) = event {
                            for c in text.chars() {
                                if self.cursor_sub == SubColumn::Note {
                                    actions.push(Action::TryEnterNote(c));
                                } else if c.is_ascii_hexdigit() {
                                    actions.push(Action::EnterHexDigit(c));
                                }
                            }
                        }
                    }
                }
            }
        });

        for action in actions {
            self.execute_action(action);
        }
    }

    fn execute_action(&mut self, action: Action) {
        match action {
            Action::TogglePlayback => {
                self.core.toggle_playback(self.edit_order, self.cursor_row);
            }
            Action::PlayFromStart => {
                self.edit_order = 0;
                self.cursor_row = 0;
                self.core.play(0, 0);
            }
            Action::SetMode(mode) => {
                if mode == Mode::Normal && self.show_song_settings {
                    self.show_song_settings = false;
                } else {
                    self.mode = mode;
                }
            }
            Action::OctaveUp => {
                if self.current_octave < 9 {
                    self.current_octave += 1;
                }
            }
            Action::OctaveDown => {
                if self.current_octave > 0 {
                    self.current_octave -= 1;
                }
            }
            Action::CursorUp(n) => {
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            Action::CursorDown(n) => {
                let max_row = self.current_pattern_rows().saturating_sub(1);
                self.cursor_row = (self.cursor_row + n).min(max_row);
            }
            Action::CursorLeft => {
                self.cursor_sub = self.cursor_sub.prev();
                if self.cursor_sub == SubColumn::Effect && self.cursor_channel > 0 {
                    self.cursor_channel -= 1;
                }
            }
            Action::CursorRight => {
                let old = self.cursor_sub;
                self.cursor_sub = self.cursor_sub.next();
                if old == SubColumn::Effect && self.cursor_channel + 1 < self.core.song.channels {
                    self.cursor_channel += 1;
                }
            }
            Action::CursorHome => {
                self.cursor_row = 0;
            }
            Action::CursorEnd => {
                self.cursor_row = self.current_pattern_rows().saturating_sub(1);
            }
            Action::NextChannel => {
                self.cursor_channel = (self.cursor_channel + 1) % self.core.song.channels;
            }
            Action::PrevChannel => {
                if self.cursor_channel == 0 {
                    self.cursor_channel = self.core.song.channels - 1;
                } else {
                    self.cursor_channel -= 1;
                }
            }
            Action::NextOrder => {
                if self.edit_order + 1 < self.core.song.order.len() {
                    self.edit_order += 1;
                    self.cursor_row = 0;
                }
            }
            Action::PrevOrder => {
                if self.edit_order > 0 {
                    self.edit_order -= 1;
                    self.cursor_row = 0;
                }
            }
            Action::TryEnterNote(c) => {
                self.try_enter_note(c);
            }
            Action::EnterHexDigit(c) => {
                self.enter_hex_digit(c);
            }
            Action::ClearCell => {
                self.clear_cell();
            }
            Action::NoteOff => {
                self.enter_note_off();
            }
            Action::Save => {
                self.do_save();
            }
            Action::Undo => {
                if let Some(edits) = self.history.undo() {
                    for edit in &edits {
                        let cell = self.core.song.patterns[edit.pattern_idx]
                            .get_mut(edit.row, edit.channel);
                        *cell = edit.old_cell;
                    }
                    self.core.dirty = true;
                    self.status_message = Some("Undo".to_string());
                }
            }
            Action::Redo => {
                if let Some(edits) = self.history.redo() {
                    for edit in &edits {
                        let cell = self.core.song.patterns[edit.pattern_idx]
                            .get_mut(edit.row, edit.channel);
                        *cell = edit.new_cell;
                    }
                    self.core.dirty = true;
                    self.status_message = Some("Redo".to_string());
                }
            }
            Action::Copy => {
                let pattern_idx = self.core.song.order[self.edit_order];
                let cell = *self.core.song.patterns[pattern_idx]
                    .get(self.cursor_row, self.cursor_channel);
                self.clipboard = Some(cell);
                self.status_message = Some("Copied".to_string());
            }
            Action::Cut => {
                let pattern_idx = self.core.song.order[self.edit_order];
                let old_cell = *self.core.song.patterns[pattern_idx]
                    .get(self.cursor_row, self.cursor_channel);
                self.clipboard = Some(old_cell);

                let cell = self.core.song.patterns[pattern_idx]
                    .get_mut(self.cursor_row, self.cursor_channel);
                *cell = Cell::default();
                let new_cell = *cell;

                self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, new_cell);
                self.core.dirty = true;
                self.status_message = Some("Cut".to_string());
            }
            Action::Paste => {
                if let Some(clip) = self.clipboard {
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let old_cell = *self.core.song.patterns[pattern_idx]
                        .get(self.cursor_row, self.cursor_channel);

                    let cell = self.core.song.patterns[pattern_idx]
                        .get_mut(self.cursor_row, self.cursor_channel);
                    *cell = clip;

                    self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, clip);
                    self.core.dirty = true;

                    // Advance cursor
                    let step = self.edit_step;
                    let max_row = self.current_pattern_rows().saturating_sub(1);
                    self.cursor_row = (self.cursor_row + step).min(max_row);
                    self.status_message = Some("Pasted".to_string());
                }
            }
        }
    }

    pub(crate) fn record_cell_edit(&mut self, pattern_idx: usize, row: usize, channel: usize, old: Cell, new: Cell) {
        self.history.push(vec![CellEdit {
            pattern_idx,
            row,
            channel,
            old_cell: old,
            new_cell: new,
        }]);
    }

    fn try_enter_note(&mut self, c: char) {
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

        // Preview
        if let Some(midi_note) = note.to_midi_note() {
            let ch = self.cursor_channel;
            let midi_ch = self.core.midi_channel_for(ch);
            let ch_type = self.core.channels.get(ch).map(|c| c.channel_type);
            let track_inst = if ch_type == Some(ChannelType::Synth) || ch_type == Some(ChannelType::Sample) {
                self.core.channels.get(ch).and_then(|c| c.default_instrument)
            } else {
                None
            };
            self.core.preview_note_with_instrument(midi_ch, midi_note, MIDI_DEFAULT_VELOCITY, track_inst);
        }

        // Write to pattern
        let pattern_idx = self.core.song.order[self.edit_order];
        let ch = self.cursor_channel;
        let ch_type = self.core.channels.get(ch).map(|c| c.channel_type);
        let track_inst = if ch_type == Some(ChannelType::Synth) || ch_type == Some(ChannelType::Sample) {
            self.core.channels.get(ch).and_then(|c| c.default_instrument)
        } else {
            None
        };

        let old_cell = *self.core.song.patterns[pattern_idx].get(self.cursor_row, ch);
        let cell = self.core.song.patterns[pattern_idx].get_mut(self.cursor_row, ch);
        cell.note = Some(note);
        if let Some(inst) = track_inst {
            cell.instrument = Some(inst);
        }
        let new_cell = *cell;
        self.record_cell_edit(pattern_idx, self.cursor_row, ch, old_cell, new_cell);
        self.core.dirty = true;

        // Advance cursor
        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    fn enter_hex_digit(&mut self, c: char) {
        let digit = match c.to_ascii_uppercase() {
            '0'..='9' => c as u8 - b'0',
            'A'..='F' => c as u8 - b'A' + 10,
            _ => return,
        };

        let pattern_idx = self.core.song.order[self.edit_order];
        let old_cell = *self.core.song.patterns[pattern_idx].get(self.cursor_row, self.cursor_channel);
        let cell = self.core.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);

        match self.cursor_sub {
            SubColumn::Instrument => {
                let current = cell.instrument.unwrap_or(0);
                cell.instrument = Some((current << 4) | digit);
            }
            SubColumn::Volume => {
                let current = cell.volume.unwrap_or(0);
                cell.volume = Some((current << 4) | digit);
            }
            SubColumn::Effect => {
                if cell.effect.is_none() {
                    cell.effect = Some(digit);
                } else {
                    let current_val = cell.effect_value.unwrap_or(0);
                    cell.effect_value = Some((current_val << 4) | digit);
                }
            }
            SubColumn::Note => {}
        }
        let new_cell = *cell;
        self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, new_cell);
        self.core.dirty = true;

        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    fn clear_cell(&mut self) {
        let pattern_idx = self.core.song.order[self.edit_order];
        let old_cell = *self.core.song.patterns[pattern_idx].get(self.cursor_row, self.cursor_channel);
        let cell = self.core.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        match self.cursor_sub {
            SubColumn::Note => {
                cell.note = None;
                cell.instrument = None;
            }
            SubColumn::Instrument => cell.instrument = None,
            SubColumn::Volume => cell.volume = None,
            SubColumn::Effect => {
                cell.effect = None;
                cell.effect_value = None;
            }
        }
        let new_cell = *cell;
        self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, new_cell);
        self.core.dirty = true;
    }

    fn enter_note_off(&mut self) {
        let pattern_idx = self.core.song.order[self.edit_order];
        let old_cell = *self.core.song.patterns[pattern_idx].get(self.cursor_row, self.cursor_channel);
        let cell = self.core.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        cell.note = Some(Note::Off);
        let new_cell = *cell;
        self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, new_cell);
        self.core.dirty = true;

        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    fn current_pattern_rows(&self) -> usize {
        let pattern_idx = self.core.song.order[self.edit_order];
        self.core.song.patterns[pattern_idx].rows
    }
}

enum Action {
    TogglePlayback,
    PlayFromStart,
    SetMode(Mode),
    OctaveUp,
    OctaveDown,
    CursorUp(usize),
    CursorDown(usize),
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
    NextChannel,
    PrevChannel,
    NextOrder,
    PrevOrder,
    TryEnterNote(char),
    EnterHexDigit(char),
    ClearCell,
    NoteOff,
    Save,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
}
