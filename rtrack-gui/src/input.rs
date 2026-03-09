use egui::Key;
use rtrack_core::constants::*;
use rtrack_core::midi::MidiInputEvent;
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
                if self.show_midi_ports {
                    self.show_midi_ports = false;
                } else if self.show_help {
                    self.show_help = false;
                } else if self.show_pattern_matrix {
                    actions.push(Action::TogglePatternMatrix);
                } else {
                    actions.push(Action::SetMode(Mode::Normal));
                }
            }
            if input.key_pressed(Key::F1) {
                actions.push(Action::ToggleHelp);
            }
            if input.key_pressed(Key::F2) {
                actions.push(Action::ToggleMidiPorts);
            }
            if input.key_pressed(Key::F3) {
                actions.push(Action::ToggleLink);
            }
            if input.key_pressed(Key::F7) {
                actions.push(Action::ToggleInstrumentList);
            }
            if input.key_pressed(Key::F8) {
                actions.push(Action::CycleTheme);
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

            // Additional Ctrl+ shortcuts
            if ctrl && input.key_pressed(Key::F) {
                actions.push(Action::ToggleFollow);
            }
            if ctrl && input.key_pressed(Key::R) {
                actions.push(Action::ToggleRecording);
            }
            if ctrl && input.key_pressed(Key::E) {
                actions.push(Action::ExportMidi);
            }
            if ctrl && input.key_pressed(Key::W) {
                actions.push(Action::ExportWav);
            }
            if ctrl && input.key_pressed(Key::L) {
                actions.push(Action::ExportFlac);
            }
            if ctrl && input.key_pressed(Key::M) {
                actions.push(Action::ToggleMidiClock);
            }
            if ctrl && input.key_pressed(Key::B) {
                actions.push(Action::ToggleBlockSelect);
            }
            if ctrl && input.key_pressed(Key::I) {
                actions.push(Action::BlockInterpolate);
            }

            // Transpose (Shift+Up/Down, both modes)
            if shift && !ctrl && input.key_pressed(Key::ArrowUp) {
                actions.push(Action::TransposeUp);
            }
            if shift && !ctrl && input.key_pressed(Key::ArrowDown) {
                actions.push(Action::TransposeDown);
            }

            // Octave
            if input.key_pressed(Key::Minus) {
                actions.push(Action::OctaveDown);
            }
            if input.key_pressed(Key::Plus) || (shift && input.key_pressed(Key::Equals)) {
                actions.push(Action::OctaveUp);
            }

            // Pattern matrix keys (when matrix is open)
            if self.show_pattern_matrix {
                if input.key_pressed(Key::ArrowUp) && !shift {
                    actions.push(Action::MatrixUp);
                }
                if input.key_pressed(Key::ArrowDown) && !shift {
                    actions.push(Action::MatrixDown);
                }
                if input.key_pressed(Key::Home) {
                    actions.push(Action::MatrixHome);
                }
                if input.key_pressed(Key::End) {
                    actions.push(Action::MatrixEnd);
                }
                if input.key_pressed(Key::PageUp) {
                    actions.push(Action::MatrixPageUp);
                }
                if input.key_pressed(Key::PageDown) {
                    actions.push(Action::MatrixPageDown);
                }
                if input.key_pressed(Key::Enter) {
                    actions.push(Action::MatrixSelect);
                }
                if input.key_pressed(Key::Insert) {
                    actions.push(Action::MatrixInsert);
                }
                if input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace) {
                    actions.push(Action::MatrixDelete);
                }
                if input.key_pressed(Key::ArrowLeft) || input.key_pressed(Key::Minus) {
                    actions.push(Action::MatrixPrevPattern);
                }
                if input.key_pressed(Key::ArrowRight) || (input.key_pressed(Key::Plus) || (shift && input.key_pressed(Key::Equals))) {
                    actions.push(Action::MatrixNextPattern);
                }
                if ctrl && input.key_pressed(Key::N) {
                    actions.push(Action::MatrixNewPattern);
                }
                if ctrl && input.key_pressed(Key::D) {
                    actions.push(Action::MatrixClonePattern);
                }
                if input.key_pressed(Key::OpenBracket) {
                    actions.push(Action::MatrixDecRepeat);
                }
                if input.key_pressed(Key::CloseBracket) {
                    actions.push(Action::MatrixIncRepeat);
                }
                // Skip normal/insert mode processing
            } else {

            // Ctrl+N / Ctrl+D (pattern ops, outside matrix)
            if ctrl && input.key_pressed(Key::N) {
                actions.push(Action::NewPattern);
            }
            if ctrl && input.key_pressed(Key::D) {
                actions.push(Action::ClonePattern);
            }
            if ctrl && input.key_pressed(Key::P) {
                actions.push(Action::TogglePatternMatrix);
            }

            match self.mode {
                Mode::Normal => {
                    if input.key_pressed(Key::I) && !ctrl {
                        actions.push(Action::SetMode(Mode::Insert));
                    }
                    // Navigation
                    if input.key_pressed(Key::ArrowUp) && !shift {
                        actions.push(Action::CursorUp(1));
                    }
                    if input.key_pressed(Key::ArrowDown) && !shift {
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
                    // Row insert/delete (Normal mode only)
                    if input.key_pressed(Key::Insert) {
                        actions.push(Action::InsertRow);
                    }
                    if input.key_pressed(Key::Backspace) {
                        actions.push(Action::DeleteRow);
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
                    if input.key_pressed(Key::Enter) {
                        actions.push(Action::OpenTrackConfig);
                    }
                }
                Mode::Insert => {
                    // Navigation
                    if input.key_pressed(Key::ArrowUp) && !shift {
                        actions.push(Action::CursorUp(1));
                    }
                    if input.key_pressed(Key::ArrowDown) && !shift {
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
            } // end else (not pattern matrix)
        });

        for action in actions {
            match action {
                Action::CycleTheme => {
                    let new_theme = self.theme.toggle();
                    self.set_theme(ctx, new_theme);
                }
                other => self.execute_action(other),
            }
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
                if mode == Mode::Normal && self.show_help {
                    self.show_help = false;
                } else if mode == Mode::Normal && self.show_instrument_list {
                    self.show_instrument_list = false;
                } else if mode == Mode::Normal && self.show_track_config.is_some() {
                    self.show_track_config = None;
                } else if mode == Mode::Normal && self.show_song_settings {
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
                if self.block_start.is_some() {
                    self.block_end = Some((self.cursor_row, self.cursor_channel));
                }
            }
            Action::CursorDown(n) => {
                let max_row = self.current_pattern_rows().saturating_sub(1);
                self.cursor_row = (self.cursor_row + n).min(max_row);
                if self.block_start.is_some() {
                    self.block_end = Some((self.cursor_row, self.cursor_channel));
                }
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
                if self.block_start.is_some() {
                    self.block_end = Some((self.cursor_row, self.cursor_channel));
                }
            }
            Action::PrevChannel => {
                if self.cursor_channel == 0 {
                    self.cursor_channel = self.core.song.channels - 1;
                } else {
                    self.cursor_channel -= 1;
                }
                if self.block_start.is_some() {
                    self.block_end = Some((self.cursor_row, self.cursor_channel));
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
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    // Block copy
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let mut block = Vec::new();
                    for row in min_r..=max_r {
                        let mut row_cells = Vec::new();
                        for ch in min_c..=max_c {
                            row_cells.push(*self.core.song.patterns[pattern_idx].get(row, ch));
                        }
                        block.push(row_cells);
                    }
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.block_clipboard = Some(block);
                    self.status_message = Some(format!("Block copied ({}x{})", rows, chs));
                } else {
                    // Single cell copy
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let cell = *self.core.song.patterns[pattern_idx]
                        .get(self.cursor_row, self.cursor_channel);
                    self.clipboard = Some(cell);
                    self.status_message = Some("Copied".to_string());
                }
            }
            Action::Cut => {
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    // Block cut
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let mut block = Vec::new();
                    let mut edits = Vec::new();
                    for row in min_r..=max_r {
                        let mut row_cells = Vec::new();
                        for ch in min_c..=max_c {
                            let old_cell = *self.core.song.patterns[pattern_idx].get(row, ch);
                            row_cells.push(old_cell);
                            let cell = self.core.song.patterns[pattern_idx].get_mut(row, ch);
                            *cell = Cell::default();
                            edits.push(CellEdit {
                                pattern_idx,
                                row,
                                channel: ch,
                                old_cell,
                                new_cell: Cell::default(),
                            });
                        }
                        block.push(row_cells);
                    }
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.block_clipboard = Some(block);
                    self.history.push(edits);
                    self.core.dirty = true;
                    self.block_start = None;
                    self.block_end = None;
                    self.status_message = Some(format!("Block cut ({}x{})", rows, chs));
                } else {
                    // Single cell cut
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
            }
            Action::Paste => {
                if let Some(ref block) = self.block_clipboard.clone() {
                    // Block paste
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let pattern = &self.core.song.patterns[pattern_idx];
                    let max_rows = pattern.rows;
                    let max_chs = pattern.channels;
                    let mut edits = Vec::new();
                    for (dr, row_cells) in block.iter().enumerate() {
                        let row = self.cursor_row + dr;
                        if row >= max_rows {
                            break;
                        }
                        for (dc, clip_cell) in row_cells.iter().enumerate() {
                            let ch = self.cursor_channel + dc;
                            if ch >= max_chs {
                                break;
                            }
                            let old_cell = *self.core.song.patterns[pattern_idx].get(row, ch);
                            let cell = self.core.song.patterns[pattern_idx].get_mut(row, ch);
                            *cell = *clip_cell;
                            edits.push(CellEdit {
                                pattern_idx,
                                row,
                                channel: ch,
                                old_cell,
                                new_cell: *clip_cell,
                            });
                        }
                    }
                    self.history.push(edits);
                    self.core.dirty = true;
                    self.status_message = Some("Block pasted".to_string());
                } else if let Some(clip) = self.clipboard {
                    // Single cell paste
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
            Action::InsertRow => {
                let pattern_idx = self.core.song.order[self.edit_order];
                let pattern = &mut self.core.song.patterns[pattern_idx];
                if pattern.rows < 256 {
                    let channels = pattern.channels;
                    pattern.data.insert(self.cursor_row, vec![Cell::default(); channels]);
                    pattern.rows += 1;
                    self.core.dirty = true;
                }
            }
            Action::DeleteRow => {
                let pattern_idx = self.core.song.order[self.edit_order];
                let pattern = &mut self.core.song.patterns[pattern_idx];
                if pattern.rows > 1 {
                    pattern.data.remove(self.cursor_row);
                    pattern.rows -= 1;
                    if self.cursor_row >= pattern.rows {
                        self.cursor_row = pattern.rows - 1;
                    }
                    self.core.dirty = true;
                }
            }
            Action::TransposeUp => {
                self.transpose_notes(1);
            }
            Action::TransposeDown => {
                self.transpose_notes(-1);
            }
            Action::OpenTrackConfig => {
                self.show_track_config = Some(self.cursor_channel);
            }
            Action::NewPattern => {
                let idx = self.core.song.add_pattern();
                self.core.song.order.insert(self.edit_order + 1, idx);
                self.core.song.sync_order_repeats();
                self.edit_order += 1;
                self.cursor_row = 0;
                self.core.dirty = true;
                self.status_message = Some(format!("New pattern {:02X}", idx));
            }
            Action::ClonePattern => {
                let src_idx = self.core.song.order[self.edit_order];
                let cloned = self.core.song.patterns[src_idx].clone();
                let new_idx = self.core.song.patterns.len();
                self.core.song.patterns.push(cloned);
                self.core.song.order.insert(self.edit_order + 1, new_idx);
                self.core.song.sync_order_repeats();
                self.edit_order += 1;
                self.cursor_row = 0;
                self.core.dirty = true;
                self.status_message = Some(format!("Cloned {:02X} -> {:02X}", src_idx, new_idx));
            }
            Action::TogglePatternMatrix => {
                self.show_pattern_matrix = !self.show_pattern_matrix;
                if self.show_pattern_matrix {
                    self.matrix_cursor = self.edit_order;
                }
            }
            Action::MatrixUp => {
                self.matrix_cursor = self.matrix_cursor.saturating_sub(1);
            }
            Action::MatrixDown => {
                let max = self.core.song.order.len().saturating_sub(1);
                self.matrix_cursor = (self.matrix_cursor + 1).min(max);
            }
            Action::MatrixHome => {
                self.matrix_cursor = 0;
            }
            Action::MatrixEnd => {
                self.matrix_cursor = self.core.song.order.len().saturating_sub(1);
            }
            Action::MatrixPageUp => {
                self.matrix_cursor = self.matrix_cursor.saturating_sub(8);
            }
            Action::MatrixPageDown => {
                let max = self.core.song.order.len().saturating_sub(1);
                self.matrix_cursor = (self.matrix_cursor + 8).min(max);
            }
            Action::MatrixSelect => {
                self.edit_order = self.matrix_cursor;
                self.cursor_row = 0;
                self.show_pattern_matrix = false;
            }
            Action::MatrixInsert => {
                let pat = self.core.song.order[self.matrix_cursor];
                self.core.song.order.insert(self.matrix_cursor + 1, pat);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
            }
            Action::MatrixDelete => {
                if self.core.song.order.len() > 1 {
                    self.core.song.order.remove(self.matrix_cursor);
                    self.core.song.sync_order_repeats();
                    if self.matrix_cursor >= self.core.song.order.len() {
                        self.matrix_cursor = self.core.song.order.len() - 1;
                    }
                    if self.edit_order >= self.core.song.order.len() {
                        self.edit_order = self.core.song.order.len() - 1;
                    }
                    self.core.dirty = true;
                }
            }
            Action::MatrixPrevPattern => {
                let cur = self.core.song.order[self.matrix_cursor];
                if cur > 0 {
                    self.core.song.order[self.matrix_cursor] = cur - 1;
                    self.core.dirty = true;
                }
            }
            Action::MatrixNextPattern => {
                let cur = self.core.song.order[self.matrix_cursor];
                if cur + 1 < self.core.song.patterns.len() {
                    self.core.song.order[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            Action::MatrixNewPattern => {
                let idx = self.core.song.add_pattern();
                self.core.song.order.insert(self.matrix_cursor + 1, idx);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.status_message = Some(format!("New pattern {:02X}", idx));
            }
            Action::MatrixClonePattern => {
                let src_idx = self.core.song.order[self.matrix_cursor];
                let cloned = self.core.song.patterns[src_idx].clone();
                let new_idx = self.core.song.patterns.len();
                self.core.song.patterns.push(cloned);
                self.core.song.order.insert(self.matrix_cursor + 1, new_idx);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.status_message = Some(format!("Cloned {:02X} -> {:02X}", src_idx, new_idx));
            }
            Action::MatrixDecRepeat => {
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur > 0 {
                    self.core.song.order_repeats[self.matrix_cursor] = cur - 1;
                    self.core.dirty = true;
                }
            }
            Action::MatrixIncRepeat => {
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur < 99 {
                    self.core.song.order_repeats[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            Action::ToggleFollow => {
                self.follow_playback = !self.follow_playback;
                let state = if self.follow_playback { "on" } else { "off" };
                self.status_message = Some(format!("Follow {}", state));
            }
            Action::ToggleRecording => {
                self.core.recording = !self.core.recording;
                let state = if self.core.recording { "on" } else { "off" };
                self.status_message = Some(format!("Recording {}", state));
            }
            Action::ExportMidi => {
                match self.core.export_midi_to_default() {
                    Ok(msg) => self.status_message = Some(msg),
                    Err(msg) => self.status_message = Some(msg),
                }
            }
            Action::ExportWav => {
                match self.core.export_wav_to_default() {
                    Ok(msg) => self.status_message = Some(msg),
                    Err(msg) => self.status_message = Some(msg),
                }
            }
            Action::ExportFlac => {
                match self.core.export_flac_to_default() {
                    Ok(msg) => self.status_message = Some(msg),
                    Err(msg) => self.status_message = Some(msg),
                }
            }
            Action::ToggleMidiClock => {
                let msg = self.core.toggle_midi_clock();
                self.status_message = Some(msg);
            }
            Action::ToggleInstrumentList => {
                self.show_instrument_list = !self.show_instrument_list;
            }
            Action::ToggleHelp => {
                self.show_help = !self.show_help;
            }
            Action::ToggleLink => {
                self.core.toggle_link();
                let msg = if self.core.link.is_enabled() {
                    "Link enabled"
                } else {
                    "Link disabled"
                };
                self.status_message = Some(msg.to_string());
            }
            Action::ToggleMidiPorts => {
                if self.show_midi_ports {
                    self.show_midi_ports = false;
                } else {
                    self.midi_port_list = rtrack_core::midi::MidiEngine::list_ports()
                        .unwrap_or_default();
                    self.midi_input_port_list = rtrack_core::midi::MidiInputEngine::list_ports()
                        .unwrap_or_default();
                    self.show_midi_ports = true;
                }
            }
            Action::CycleTheme => unreachable!(),
            Action::ToggleBlockSelect => {
                if self.block_start.is_some() {
                    self.block_start = None;
                    self.block_end = None;
                    self.status_message = Some("Block cleared".to_string());
                } else {
                    self.block_start = Some((self.cursor_row, self.cursor_channel));
                    self.block_end = Some((self.cursor_row, self.cursor_channel));
                    self.status_message = Some("Block select started".to_string());
                }
            }
            Action::BlockInterpolate => {
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let span = max_r - min_r;
                    if span < 2 {
                        self.status_message = Some("Need at least 3 rows to interpolate".to_string());
                        return;
                    }
                    let mut edits = Vec::new();
                    for ch in min_c..=max_c {
                        let first = *self.core.song.patterns[pattern_idx].get(min_r, ch);
                        let last = *self.core.song.patterns[pattern_idx].get(max_r, ch);

                        // Interpolate volume
                        if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                            for row in min_r..=max_r {
                                let t = (row - min_r) as f64 / span as f64;
                                let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                                let old_cell = *self.core.song.patterns[pattern_idx].get(row, ch);
                                let cell = self.core.song.patterns[pattern_idx].get_mut(row, ch);
                                cell.volume = Some(v.round() as u8);
                                edits.push(CellEdit {
                                    pattern_idx,
                                    row,
                                    channel: ch,
                                    old_cell,
                                    new_cell: *cell,
                                });
                            }
                        }

                        // Interpolate effect_value
                        if let (Some(e0), Some(e1)) = (first.effect_value, last.effect_value) {
                            // Only interpolate if effect command matches at both ends
                            if first.effect == last.effect && first.effect.is_some() {
                                for row in min_r..=max_r {
                                    let t = (row - min_r) as f64 / span as f64;
                                    let e = e0 as f64 + (e1 as f64 - e0 as f64) * t;
                                    let old_cell = *self.core.song.patterns[pattern_idx].get(row, ch);
                                    let cell = self.core.song.patterns[pattern_idx].get_mut(row, ch);
                                    cell.effect = first.effect;
                                    cell.effect_value = Some(e.round() as u8);
                                    // Check if we already have an edit for this cell from volume interpolation
                                    let existing = edits.iter_mut().find(|ed: &&mut CellEdit| {
                                        ed.pattern_idx == pattern_idx && ed.row == row && ed.channel == ch
                                    });
                                    if let Some(existing) = existing {
                                        existing.new_cell = *cell;
                                    } else {
                                        edits.push(CellEdit {
                                            pattern_idx,
                                            row,
                                            channel: ch,
                                            old_cell,
                                            new_cell: *cell,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if edits.is_empty() {
                        self.status_message = Some("Nothing to interpolate".to_string());
                    } else {
                        self.history.push(edits);
                        self.core.dirty = true;
                        self.status_message = Some("Interpolated".to_string());
                    }
                } else {
                    self.status_message = Some("No block selected".to_string());
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

    fn transpose_notes(&mut self, semitones: i8) {
        let pattern_idx = self.core.song.order[self.edit_order];

        if let (Some(start), Some(end)) = (self.block_start, self.block_end) {
            let r0 = start.0.min(end.0);
            let r1 = start.0.max(end.0);
            let c0 = start.1.min(end.1);
            let c1 = start.1.max(end.1);
            let pattern = &mut self.core.song.patterns[pattern_idx];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    transpose_cell_note(pattern.get_mut(r, c), semitones);
                }
            }
            self.core.dirty = true;
            self.status_message = Some(format!("Transposed block by {} semitone(s)", semitones));
        } else {
            let cell = self.core.song.patterns[pattern_idx]
                .get_mut(self.cursor_row, self.cursor_channel);
            transpose_cell_note(cell, semitones);
            self.core.dirty = true;
        }
    }

    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.core.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    fn handle_midi_input(&mut self, event: MidiInputEvent) {
        match event {
            MidiInputEvent::NoteOn { channel: _, note, velocity } => {
                let ch = self.cursor_channel;
                let midi_ch = self.core.midi_channel_for(ch);

                // Punch-in recording: playing + recording + Insert mode
                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    let order = self.core.engine.order;
                    let row = self.core.engine.row;
                    self.core.record_note_at(order, row, ch, note, velocity);
                    self.core.preview_note(midi_ch, note, velocity);
                    return;
                }

                // Step recording: Insert mode + stopped
                if self.mode == Mode::Insert && !self.core.playing {
                    self.core.preview_note(midi_ch, note, velocity);
                    let order = self.current_order_position();
                    let row = self.cursor_row;
                    if self.core.record_note_at(order, row, ch, note, velocity) {
                        let step = self.edit_step;
                        let max_row = self.current_pattern_rows().saturating_sub(1);
                        self.cursor_row = (self.cursor_row + step).min(max_row);
                    }
                    return;
                }

                // Preview only
                self.core.preview_note(midi_ch, note, velocity);
            }
            MidiInputEvent::NoteOff { channel: _, note } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_note_off(midi_ch, note);

                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    self.core.record_note_off_at(
                        self.core.engine.order,
                        self.core.engine.row,
                        self.cursor_channel,
                    );
                }
            }
            MidiInputEvent::CC { channel: _, controller, value } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                if let Some(msg) = self.core.handle_midi_cc(controller, value, midi_ch) {
                    self.status_message = Some(msg);
                }
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_pitch_bend(midi_ch, value);
            }
            MidiInputEvent::ProgramChange { channel: _, program } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_program_change(midi_ch, program);
            }
            MidiInputEvent::ChannelPressure { channel: _, pressure } => {
                self.core.apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::PolyPressure { channel: _, note: _, pressure } => {
                self.core.apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::Clock => {
                self.core.handle_external_clock();
            }
            MidiInputEvent::Start => {
                self.core.handle_external_start();
            }
            MidiInputEvent::Stop => {
                self.core.handle_external_stop();
            }
            MidiInputEvent::Continue => {
                self.core.handle_external_continue();
            }
        }
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
    InsertRow,
    DeleteRow,
    TransposeUp,
    TransposeDown,
    OpenTrackConfig,
    NewPattern,
    ClonePattern,
    TogglePatternMatrix,
    // Pattern matrix actions
    MatrixUp,
    MatrixDown,
    MatrixHome,
    MatrixEnd,
    MatrixPageUp,
    MatrixPageDown,
    MatrixSelect,
    MatrixInsert,
    MatrixDelete,
    MatrixPrevPattern,
    MatrixNextPattern,
    MatrixDecRepeat,
    MatrixIncRepeat,
    MatrixNewPattern,
    MatrixClonePattern,
    ToggleFollow,
    ToggleRecording,
    ExportMidi,
    ExportWav,
    ExportFlac,
    ToggleMidiClock,
    ToggleInstrumentList,
    ToggleBlockSelect,
    BlockInterpolate,
    ToggleHelp,
    ToggleLink,
    ToggleMidiPorts,
    CycleTheme,
}

fn transpose_cell_note(cell: &mut Cell, semitones: i8) {
    if let Some(Note::On { ref value, ref octave }) = cell.note {
        let semi = SEMITONES_PER_OCTAVE as i16;
        let midi = (*octave as i16) * semi + value.to_index() as i16 + semitones as i16;
        if midi >= 0 && midi <= MIDI_MAX_NOTE as i16 {
            let new_octave = (midi / semi) as u8;
            let new_note_idx = (midi % semi) as u8;
            if let Some(nv) = NoteValue::from_index(new_note_idx) {
                cell.note = Some(Note::On { value: nv, octave: new_octave });
            }
        }
    }
}
