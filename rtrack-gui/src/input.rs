use egui::Key;
use rtrack_core::constants::*;
use rtrack_core::midi::MidiInputEvent;
use rtrack_core::tracker::{Cell, Fx, Note, Sheet};

use crate::app::RtrackApp;
use crate::history::{CellEdit, Edit};
use crate::state::{Mode, SubColumn};

impl RtrackApp {
    pub fn process_keys(&mut self, ctx: &egui::Context) {
        // The effect prompt has the keyboard while it is open, as does any
        // focused text field: typing a name must not enter notes or delete rows.
        if self.effect_prompt.is_some() || ctx.text_edit_focused() {
            return;
        }
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
            if input.key_pressed(Key::F4) {
                actions.push(Action::ToggleVisualization);
            }
            if input.key_pressed(Key::F5) {
                actions.push(Action::TogglePhraseView);
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
                if input.key_pressed(Key::ArrowRight)
                    || (input.key_pressed(Key::Plus) || (shift && input.key_pressed(Key::Equals)))
                {
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
                        // Edit an effect by name
                        if input.key_pressed(Key::Equals)
                            && !shift
                            && self.cursor_sub.fx_column().is_some()
                        {
                            actions.push(Action::OpenEffectPrompt);
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
                self.core
                    .toggle_playback(self.edit_order, self.pattern_row());
            }
            Action::TogglePhraseView => {
                if self.phrase_view.is_some() {
                    self.close_phrase_view();
                } else {
                    self.open_phrase_view(None);
                }
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
                    self.break_edit_coalescing();
                } else if mode == Mode::Normal && self.block_start.is_some() {
                    self.block_start = None;
                    self.block_end = None;
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
                let old = self.cursor_sub;
                self.cursor_sub = old.prev(self.fx_columns());
                // The phrase view shows one track's column.
                if old == SubColumn::Note && self.cursor_channel > 0 && self.phrase_view.is_none() {
                    self.cursor_channel -= 1;
                }
            }
            Action::CursorRight => {
                self.cursor_sub = self.cursor_sub.next(self.fx_columns());
                if self.cursor_sub == SubColumn::Note
                    && self.cursor_channel + 1 < self.core.song.channels
                    && self.phrase_view.is_none()
                {
                    self.cursor_channel += 1;
                }
            }
            Action::CursorHome => {
                self.cursor_row = 0;
            }
            Action::CursorEnd => {
                self.cursor_row = self.current_pattern_rows().saturating_sub(1);
            }
            Action::NextChannel | Action::PrevChannel if self.phrase_view.is_some() => {}
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
            Action::NextOrder if self.phrase_view.is_some() => self.step_phrase_view(true),
            Action::PrevOrder if self.phrase_view.is_some() => self.step_phrase_view(false),
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
            Action::OpenEffectPrompt => self.open_effect_prompt(),
            Action::NoteOff => {
                self.enter_note_off();
            }
            Action::Save => {
                self.do_save();
            }
            Action::Undo => {
                self.apply_undo();
            }
            Action::Redo => {
                self.apply_redo();
            }
            Action::Copy => {
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    // Block copy
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let mut block = Vec::new();
                    for row in min_r..=max_r {
                        let mut row_cells = Vec::new();
                        for ch in min_c..=max_c {
                            row_cells.push(self.shown_cell(row, ch));
                        }
                        block.push(row_cells);
                    }
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.clipboard.set_block(block);
                    self.status_message = Some(format!("Block copied ({}x{})", rows, chs));
                } else {
                    // Single cell copy
                    let cell = self.shown_cell(self.cursor_row, self.cursor_channel);
                    self.clipboard.set_cell(cell);
                    self.status_message = Some("Copied".to_string());
                }
            }
            Action::Cut => {
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    // Block cut
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let mut block = Vec::new();
                    let mut edits = Vec::new();
                    for row in min_r..=max_r {
                        let mut row_cells = Vec::new();
                        for ch in min_c..=max_c {
                            row_cells.push(self.shown_cell(row, ch));
                            self.write_cell(row, ch, Cell::default(), &mut edits);
                        }
                        block.push(row_cells);
                    }
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.clipboard.set_block(block);
                    self.core.history.push(edits);
                    self.core.dirty = true;
                    self.block_start = None;
                    self.block_end = None;
                    self.status_message = Some(format!("Block cut ({}x{})", rows, chs));
                } else {
                    // Single cell cut
                    if let Some(old_cell) = self.write_cursor_cell(Cell::default()) {
                        self.clipboard.set_cell(old_cell);
                    }
                    self.status_message = Some("Cut".to_string());
                }
            }
            Action::Paste => {
                if let Some(block) = self.clipboard.block().cloned() {
                    // Block paste; cells past the pattern's edge do not resolve
                    let mut edits = Vec::new();
                    for (dr, row_cells) in block.iter().enumerate() {
                        for (dc, clip_cell) in row_cells.iter().enumerate() {
                            let (row, ch) = (self.cursor_row + dr, self.cursor_channel + dc);
                            self.write_cell(row, ch, *clip_cell, &mut edits);
                        }
                    }
                    self.core.history.push(edits);
                    self.core.dirty = true;
                    self.status_message = Some("Block pasted".to_string());
                } else if let Some(clip) = self.clipboard.cell() {
                    // Single cell paste
                    self.write_cursor_cell(clip);

                    // Advance cursor
                    let step = self.edit_step;
                    let max_row = self.current_pattern_rows().saturating_sub(1);
                    self.cursor_row = (self.cursor_row + step).min(max_row);
                    self.status_message = Some("Pasted".to_string());
                }
            }
            Action::InsertRow | Action::DeleteRow if self.phrase_view.is_some() => {
                let phrase = self.phrase_view.map_or(0, |v| v.phrase);
                let before = self.begin_structural_edit();
                if matches!(action, Action::InsertRow) {
                    self.core.song.insert_phrase_row(phrase, self.cursor_row);
                } else {
                    self.core.song.delete_phrase_row(phrase, self.cursor_row);
                }
                self.end_structural_edit(before);
            }
            Action::InsertRow => {
                let before = self.begin_structural_edit();
                let pattern_idx = self.core.song.order[self.edit_order];
                let pattern = &mut self.core.song.patterns[pattern_idx];
                if pattern.rows < 256 {
                    let channels = pattern.channels;
                    pattern
                        .data
                        .insert(self.cursor_row, vec![Cell::default(); channels]);
                    pattern.rows += 1;
                    if pattern.links.iter().any(Option::is_some) {
                        self.status_message =
                            Some("Row inserted (linked phrases unchanged)".into());
                    }
                    self.end_structural_edit(before);
                }
            }
            Action::DeleteRow => {
                let before = self.begin_structural_edit();
                let pattern_idx = self.core.song.order[self.edit_order];
                let pattern = &mut self.core.song.patterns[pattern_idx];
                if pattern.rows > 1 {
                    pattern.data.remove(self.cursor_row);
                    pattern.rows -= 1;
                    if self.cursor_row >= pattern.rows {
                        self.cursor_row = pattern.rows - 1;
                    }
                    if pattern.links.iter().any(Option::is_some) {
                        self.status_message = Some("Row deleted (linked phrases unchanged)".into());
                    }
                    self.end_structural_edit(before);
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
            Action::NewPattern if self.phrase_view.is_some() => self.add_phrase_in_view(false),
            Action::ClonePattern if self.phrase_view.is_some() => self.add_phrase_in_view(true),
            Action::NewPattern => {
                let before = self.begin_structural_edit();
                let idx = self.core.song.add_pattern();
                self.core.song.order.insert(self.edit_order + 1, idx);
                self.core.song.sync_order_repeats();
                self.edit_order += 1;
                self.cursor_row = 0;
                self.core.dirty = true;
                self.status_message = Some(format!("New pattern {:02X}", idx));
                self.end_structural_edit(before);
            }
            Action::ClonePattern => {
                let before = self.begin_structural_edit();
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
                self.end_structural_edit(before);
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
                let before = self.begin_structural_edit();
                let pat = self.core.song.order[self.matrix_cursor];
                self.core.song.order.insert(self.matrix_cursor + 1, pat);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.end_structural_edit(before);
            }
            Action::MatrixDelete => {
                let before = self.begin_structural_edit();
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
                self.end_structural_edit(before);
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
                let before = self.begin_structural_edit();
                let idx = self.core.song.add_pattern();
                self.core.song.order.insert(self.matrix_cursor + 1, idx);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.status_message = Some(format!("New pattern {:02X}", idx));
                self.end_structural_edit(before);
            }
            Action::MatrixClonePattern => {
                let before = self.begin_structural_edit();
                let src_idx = self.core.song.order[self.matrix_cursor];
                let cloned = self.core.song.patterns[src_idx].clone();
                let new_idx = self.core.song.patterns.len();
                self.core.song.patterns.push(cloned);
                self.core.song.order.insert(self.matrix_cursor + 1, new_idx);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.status_message = Some(format!("Cloned {:02X} -> {:02X}", src_idx, new_idx));
                self.end_structural_edit(before);
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
            Action::ExportMidi => match self.core.export_midi_to_default() {
                Ok(path) => {
                    self.status_message = Some(format!("Exported MIDI: {}", path.display()))
                }
                Err(e) => self.status_message = Some(format!("MIDI export failed: {}", e)),
            },
            Action::ExportWav => match self.core.export_wav_to_default() {
                Ok(path) => self.status_message = Some(format!("Exported WAV: {}", path.display())),
                Err(e) => self.status_message = Some(format!("WAV export failed: {}", e)),
            },
            Action::ExportFlac => match self.core.export_flac_to_default() {
                Ok(path) => {
                    self.status_message = Some(format!("Exported FLAC: {}", path.display()))
                }
                Err(e) => self.status_message = Some(format!("FLAC export failed: {}", e)),
            },
            Action::ToggleMidiClock => {
                let msg = format!(
                    "MIDI clock {}",
                    if self.core.toggle_midi_clock() {
                        "on"
                    } else {
                        "off"
                    }
                );
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
                    self.midi_port_list =
                        rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                    self.midi_input_port_list =
                        rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                    self.show_midi_ports = true;
                }
            }
            Action::ToggleVisualization => {
                self.show_visualization = !self.show_visualization;
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
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let span = max_r - min_r;
                    if span < 2 {
                        self.status_message =
                            Some("Need at least 3 rows to interpolate".to_string());
                        return;
                    }
                    let lerp = |a: u8, b: u8, row: usize| {
                        let t = (row - min_r) as f64 / span as f64;
                        (a as f64 + (b as f64 - a as f64) * t).round() as u8
                    };
                    let mut edits = Vec::new();
                    for ch in min_c..=max_c {
                        let first = self.shown_cell(min_r, ch);
                        let last = self.shown_cell(max_r, ch);
                        let volumes = first.volume.zip(last.volume);
                        // The effect parameter, when both ends have the same command
                        let fx = first.fx[0].zip(last.fx[0]).filter(|(a, b)| a.cmd == b.cmd);
                        if volumes.is_none() && fx.is_none() {
                            continue;
                        }
                        for row in min_r..=max_r {
                            let mut cell = self.shown_cell(row, ch);
                            if let Some((v0, v1)) = volumes {
                                cell.volume = Some(lerp(v0, v1, row));
                            }
                            if let Some((f0, f1)) = fx {
                                cell.fx[0] = Some(Fx::new(f0.cmd, lerp(f0.param, f1.param, row)));
                            }
                            self.write_cell(row, ch, cell, &mut edits);
                        }
                    }
                    if edits.is_empty() {
                        self.status_message = Some("Nothing to interpolate".to_string());
                    } else {
                        self.core.history.push(edits);
                        self.core.dirty = true;
                        self.status_message = Some("Interpolated".to_string());
                    }
                } else {
                    self.status_message = Some("No block selected".to_string());
                }
            }
        }
    }

    /// Roll one edit group back. Shared by the Edit menu and Ctrl+Z so the
    /// two entry points cannot drift apart.
    pub(crate) fn apply_undo(&mut self) {
        if self.core.undo() {
            self.clamp_cursor_to_song();
            self.status_message = Some("Undo".to_string());
        }
    }

    /// Re-apply one edit group. Counterpart to [`RtrackApp::apply_undo`].
    pub(crate) fn apply_redo(&mut self) {
        if self.core.redo() {
            self.clamp_cursor_to_song();
            self.status_message = Some("Redo".to_string());
        }
    }

    /// Record a song-settings change attributed to one control, folding it
    /// into the previous step when that came from the same control.
    ///
    /// These are drag values: one change per frame while the pointer moves,
    /// and one per repeat while an arrow key is held. Recording each would
    /// bury the history under a single drag, which is why the settings were
    /// outside undo entirely. Coalescing keeps the first "before", so undo
    /// returns to where the value stood before the drag began.
    pub(crate) fn record_settings_edit(
        &mut self,
        control: &'static str,
        before: rtrack_core::tracker::Song,
    ) {
        self.core.history.push_from(
            rtrack_core::editor::EditSource::new(control, 0),
            Edit::Structure(Box::new(rtrack_core::editor::StructureEdit {
                before,
                after: self.core.song.clone(),
            })),
        );
        self.core.dirty = true;
    }

    /// End a run of coalescing, so the next edit to the same control starts a
    /// new step. Called when a dialog closes.
    pub(crate) fn break_edit_coalescing(&mut self) {
        self.core.history.break_coalescing();
    }

    /// Take the song as it stands, to pair with [`RtrackApp::end_structural_edit`].
    ///
    /// Structural changes -- adding, cloning or removing patterns and order
    /// entries -- move data around wholesale, so they are recorded as a
    /// before/after pair rather than a diff. Before this the GUI could not
    /// undo any of them.
    pub(crate) fn begin_structural_edit(&self) -> rtrack_core::tracker::Song {
        self.core.song.clone()
    }

    /// Record a structural change against the song `begin_structural_edit`
    /// returned.
    pub(crate) fn end_structural_edit(&mut self, before: rtrack_core::tracker::Song) {
        self.core
            .history
            .push_structure(before, self.core.song.clone());
        self.core.dirty = true;
    }

    /// Bring the cursor and the visible channel window inside the song.
    pub(crate) fn clamp_cursor_to_song(&mut self) {
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
        let rows = self.current_pattern_rows();
        if self.cursor_row >= rows {
            self.cursor_row = rows - 1;
        }
        let channels = self.core.song.channels.max(1);
        if self.cursor_channel >= channels {
            self.cursor_channel = channels - 1;
        }
        if self.first_visible_channel >= channels {
            self.first_visible_channel = 0;
        }
    }

    /// What the grid edits: the phrase view's phrase, or the pattern at the
    /// edit position.
    pub(crate) fn sheet(&self) -> Sheet {
        match self.phrase_view {
            Some(view) => view.sheet(self.cursor_channel),
            None => self.edit_order.into(),
        }
    }

    /// The pattern row playback starts from and the phrase view returns to.
    pub(crate) fn pattern_row(&self) -> usize {
        self.phrase_view.map_or(self.cursor_row, |v| v.pattern_row)
    }

    /// Show a phrase in the grid in place of the pattern: `phrase`, or the
    /// one the cursor's track plays.
    pub(crate) fn open_phrase_view(&mut self, phrase: Option<usize>) {
        use rtrack_core::editor::PhraseView;
        let pattern_row = self.pattern_row();
        let opened = match phrase {
            Some(i) if i < self.core.song.phrases.len() => Some((
                PhraseView {
                    phrase: i,
                    pattern_row,
                },
                0,
            )),
            Some(_) => None,
            None => PhraseView::open(
                &self.core.song,
                self.edit_order,
                pattern_row,
                self.cursor_channel,
            ),
        };
        match opened {
            Some((view, row)) => {
                self.phrase_view = Some(view);
                self.cursor_row = row;
                self.block_start = None;
                self.block_end = None;
            }
            None if phrase.is_some() => self.status_message = Some("No such phrase".to_string()),
            None => {
                self.status_message =
                    Some("No phrases yet: View > Phrases makes one from a track".to_string())
            }
        }
    }

    /// Return the grid to the pattern, at the row it showed before.
    pub(crate) fn close_phrase_view(&mut self) {
        if let Some(view) = self.phrase_view.take() {
            self.block_start = None;
            self.block_end = None;
            self.cursor_row = view.pattern_row.min(self.current_pattern_rows() - 1);
        }
    }

    /// Show the next or previous phrase.
    pub(crate) fn step_phrase_view(&mut self, forward: bool) {
        let Some(mut view) = self.phrase_view else {
            return;
        };
        if view.step(&self.core.song, forward) {
            self.phrase_view = Some(view);
            self.block_start = None;
            self.block_end = None;
            self.cursor_row = self.cursor_row.min(self.current_pattern_rows() - 1);
        }
    }

    /// Add an empty phrase as long as the pattern, or a copy of the phrase
    /// on view, and show it.
    pub(crate) fn add_phrase_in_view(&mut self, duplicate: bool) {
        let Some(view) = self.phrase_view else {
            return;
        };
        let before = self.begin_structural_edit();
        let added = if duplicate {
            self.core.song.duplicate_phrase(view.phrase)
        } else {
            let rows = self.core.song.rows_at(self.edit_order);
            Some(self.core.song.add_phrase(rows))
        };
        if let Some(i) = added {
            self.end_structural_edit(before);
            self.open_phrase_view(Some(i));
        }
    }

    /// The cell shown at (row, channel) of the sheet being edited.
    pub(crate) fn shown_cell(&self, row: usize, channel: usize) -> Cell {
        *self.core.song.cell_at(self.sheet(), row, channel)
    }

    /// Write the cell shown at (row, channel) of the pattern being edited,
    /// wherever it is stored, and record the change in `edits`. Returns the
    /// old cell, or `None` if the coordinate does not resolve.
    pub(crate) fn write_cell(
        &mut self,
        row: usize,
        channel: usize,
        new: Cell,
        edits: &mut Vec<CellEdit>,
    ) -> Option<Cell> {
        let at = self.core.song.address(self.sheet(), row, channel)?;
        let old = std::mem::replace(self.core.song.cell_mut(at)?, new);
        edits.push(CellEdit {
            at,
            old_cell: old,
            new_cell: new,
        });
        Some(old)
    }

    /// [`RtrackApp::write_cell`] at the cursor, as one undo step.
    pub(crate) fn write_cursor_cell(&mut self, new: Cell) -> Option<Cell> {
        let mut edits = Vec::new();
        let old = self.write_cell(self.cursor_row, self.cursor_channel, new, &mut edits);
        self.core.history.push(edits);
        self.core.dirty = true;
        old
    }

    fn try_enter_note(&mut self, c: char) {
        let Some((value, octave)) =
            rtrack_core::keymap::piano_key_at_octave(c, self.current_octave)
        else {
            return;
        };
        let note = self.core.song.snap_entry(Note::On { value, octave });

        let ch = self.cursor_channel;
        // One resolution for both the preview and the written cell, so what
        // you hear while typing is what plays back.
        let track_inst = self
            .core
            .resolve_edit_instrument(self.sheet(), self.cursor_row, ch);

        if let Some(midi_note) = note.to_midi_note() {
            self.core.preview_note_for_cell(
                self.sheet(),
                self.cursor_row,
                ch,
                midi_note,
                MIDI_DEFAULT_VELOCITY,
            );
        }

        let mut cell = self.shown_cell(self.cursor_row, ch);
        cell.note = Some(note);
        if let Some(inst) = track_inst {
            cell.instrument = Some(inst);
        }
        if self.write_cursor_cell(cell).is_none() {
            return;
        }

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

        let mut cell = self.shown_cell(self.cursor_row, self.cursor_channel);
        match self.cursor_sub {
            SubColumn::Instrument => {
                let current = cell.instrument.unwrap_or(0);
                cell.instrument = Some((current << 4) | digit);
            }
            SubColumn::Volume => {
                let current = cell.volume.unwrap_or(0);
                cell.volume = Some((current << 4) | digit);
            }
            SubColumn::Effect => cell.enter_effect_digit(0, digit),
            SubColumn::Effect2 => cell.enter_effect_digit(1, digit),
            SubColumn::Note => {}
        }
        // The cursor stays, so the next digit completes the value, as in the TUI.
        self.write_cursor_cell(cell);
    }

    fn clear_cell(&mut self) {
        let mut cell = self.shown_cell(self.cursor_row, self.cursor_channel);
        match self.cursor_sub {
            SubColumn::Note => {
                cell.note = None;
                cell.instrument = None;
            }
            SubColumn::Instrument => cell.instrument = None,
            SubColumn::Volume => cell.volume = None,
            SubColumn::Effect => cell.fx[0] = None,
            SubColumn::Effect2 => cell.fx[1] = None,
        }
        self.write_cursor_cell(cell);
    }

    fn open_effect_prompt(&mut self) {
        let Some(col) = self.cursor_sub.fx_column() else {
            return;
        };
        let Some(at) = self
            .core
            .song
            .address(self.sheet(), self.cursor_row, self.cursor_channel)
        else {
            return;
        };
        let cell = self.shown_cell(self.cursor_row, self.cursor_channel);
        self.effect_prompt = Some(crate::app::EffectPrompt {
            text: cell.fx[col].map(|fx| fx.to_string()).unwrap_or_default(),
            at,
            col,
        });
    }

    /// Apply the effect prompt's text; empty text clears the effect.
    pub(crate) fn apply_effect_prompt(&mut self) {
        let Some(prompt) = self.effect_prompt.take() else {
            return;
        };
        let Some(slot) = self.core.song.cell_mut(prompt.at) else {
            return;
        };
        let old_cell = *slot;
        let mut new_cell = old_cell;
        match new_cell.set_effect_text(prompt.col, &prompt.text) {
            Ok(()) => {
                *slot = new_cell;
                self.core.history.push(vec![CellEdit {
                    at: prompt.at,
                    old_cell,
                    new_cell,
                }]);
                self.core.dirty = true;
            }
            Err(e) => self.status_message = Some(e),
        }
    }

    fn enter_note_off(&mut self) {
        let mut cell = self.shown_cell(self.cursor_row, self.cursor_channel);
        cell.note = Some(Note::Off);
        if self.write_cursor_cell(cell).is_none() {
            return;
        }

        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    /// Rows on the sheet being edited: the pattern or the phrase.
    pub(crate) fn current_pattern_rows(&self) -> usize {
        self.core.song.rows_at(self.sheet())
    }

    fn transpose_notes(&mut self, steps: i8) {
        let scale = self.core.song.scale;
        let (r0, r1, c0, c1) = match (self.block_start, self.block_end) {
            (Some(start), Some(end)) => (
                start.0.min(end.0),
                start.0.max(end.0),
                start.1.min(end.1),
                start.1.max(end.1),
            ),
            _ => (
                self.cursor_row,
                self.cursor_row,
                self.cursor_channel,
                self.cursor_channel,
            ),
        };
        // Each stored cell once, however many times it shows in the block.
        let mut edits = Vec::new();
        for at in self.core.song.addresses(self.sheet(), r0..=r1, c0..=c1) {
            if let Some(cell) = self.core.song.cell_mut(at) {
                let old_cell = *cell;
                cell.transpose_note(steps, scale);
                edits.push(CellEdit {
                    at,
                    old_cell,
                    new_cell: *cell,
                });
            }
        }
        self.core.history.push(edits);
        self.core.dirty = true;
        if self.block_start.is_some() {
            let unit = if scale.is_some() {
                "scale degree(s)"
            } else {
                "semitone(s)"
            };
            self.status_message = Some(format!("Transposed block by {} {}", steps, unit));
        }
    }

    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.core.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    fn handle_midi_input(&mut self, event: MidiInputEvent) {
        match event {
            MidiInputEvent::NoteOn {
                channel: _,
                note,
                velocity,
            } => {
                let ch = self.cursor_channel;

                // Punch-in recording: playing + recording + Insert mode
                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    let order = self.core.engine.order;
                    let row = self.core.engine.row;
                    self.core.record_note_at(order, row, ch, note, velocity);
                    self.core
                        .preview_note_for_cell(order, row, ch, note, velocity);
                    return;
                }

                // Step recording: Insert mode + stopped
                if self.mode == Mode::Insert && !self.core.playing {
                    let order = self.sheet();
                    let row = self.cursor_row;
                    self.core
                        .preview_note_for_cell(order, row, ch, note, velocity);
                    if self.core.record_note_at(order, row, ch, note, velocity) {
                        let step = self.edit_step;
                        let max_row = self.current_pattern_rows().saturating_sub(1);
                        self.cursor_row = (self.cursor_row + step).min(max_row);
                    }
                    return;
                }

                // Preview only, still through the instrument the cursor's
                // cell would use.
                let order = self.sheet();
                self.core
                    .preview_note_for_cell(order, self.cursor_row, ch, note, velocity);
            }
            MidiInputEvent::NoteOff { channel: _, note } => {
                let track = self.cursor_channel as u8;
                self.core.send_note_off(track, note);

                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    self.core.record_note_off_at(
                        self.core.engine.order,
                        self.core.engine.row,
                        self.cursor_channel,
                    );
                }
            }
            MidiInputEvent::CC {
                channel: _,
                controller,
                value,
            } => {
                let track = self.cursor_channel as u8;
                if let Some(msg) = self.core.handle_midi_cc(controller, value, track) {
                    self.status_message = Some(msg);
                }
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                let track = self.cursor_channel as u8;
                self.core.send_pitch_bend(track, value);
            }
            MidiInputEvent::ProgramChange {
                channel: _,
                program,
            } => {
                let track = self.cursor_channel as u8;
                self.core.send_program_change(track, program);
            }
            MidiInputEvent::ChannelPressure {
                channel: _,
                pressure,
            } => {
                self.core
                    .apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::PolyPressure {
                channel: _,
                note: _,
                pressure,
            } => {
                self.core
                    .apply_aftertouch_to_filter(self.cursor_channel, pressure);
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
    OpenEffectPrompt,
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
    ToggleVisualization,
    TogglePhraseView,
    CycleTheme,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SubColumn;
    use rtrack_core::tracker::NoteValue;

    fn app() -> RtrackApp {
        RtrackApp::headless(4, 16)
    }

    #[test]
    fn escape_clears_a_block_selection_before_leaving_insert() {
        let mut a = app();
        a.mode = Mode::Insert;
        a.block_start = Some((0, 0));
        a.block_end = Some((3, 1));
        a.execute_action(Action::SetMode(Mode::Normal));
        assert_eq!((a.block_start, a.block_end), (None, None));
        assert_eq!(a.mode, Mode::Insert, "first Esc only clears the selection");
        a.execute_action(Action::SetMode(Mode::Normal));
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn the_second_effect_column_is_reached_once_shown() {
        let mut a = app();
        a.cursor_sub = SubColumn::Effect;
        a.execute_action(Action::CursorRight);
        assert_eq!((a.cursor_channel, a.cursor_sub), (1, SubColumn::Note));

        a.show_second_fx = true;
        a.cursor_channel = 0;
        a.cursor_sub = SubColumn::Effect;
        a.execute_action(Action::CursorRight);
        assert_eq!((a.cursor_channel, a.cursor_sub), (0, SubColumn::Effect2));
        a.execute_action(Action::CursorLeft);
        assert_eq!(a.cursor_sub, SubColumn::Effect);
    }

    #[test]
    fn an_effect_is_edited_by_name_and_the_edit_undoes() {
        let mut a = app();
        a.cursor_sub = SubColumn::Effect;
        a.execute_action(Action::OpenEffectPrompt);
        a.effect_prompt.as_mut().unwrap().text = "pvb44".to_string();
        a.apply_effect_prompt();
        let fx = |a: &RtrackApp| a.core.song.cell_at(a.edit_order, 0, 0).fx[0];
        assert_eq!(
            fx(&a),
            Some(Fx::new(rtrack_core::tracker::FxCmd::Vibrato, 0x44))
        );

        a.execute_action(Action::OpenEffectPrompt);
        assert_eq!(a.effect_prompt.as_ref().unwrap().text, "PVB44");
        a.effect_prompt.as_mut().unwrap().text = "nope".to_string();
        a.apply_effect_prompt();
        assert!(a.status_message.is_some());
        assert!(fx(&a).is_some(), "bad text changed the cell");

        a.apply_undo();
        assert_eq!(fx(&a), None);
    }

    fn note_at(app: &RtrackApp, row: usize, channel: usize) -> Option<Note> {
        app.core.song.cell_at(app.edit_order, row, channel).note
    }

    fn on(value: NoteValue, octave: u8) -> Option<Note> {
        Some(Note::On { value, octave })
    }

    // -- Note entry --

    #[test]
    fn piano_keys_map_to_the_current_octave() {
        let mut a = app();
        a.current_octave = 4;
        a.try_enter_note('z');
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::C, 4));

        a.cursor_row = 0;
        a.try_enter_note('m');
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::B, 4));
    }

    #[test]
    fn keyboard_entry_snaps_to_the_song_scale() {
        use rtrack_core::theory::{Scale, ScaleSetting};

        let mut a = app();
        a.current_octave = 4;
        a.core.song.scale = Some(ScaleSetting::new(NoteValue::A, Scale::MinorPentatonic));

        // A minor pentatonic is A C D E G; D# and C# are not in it.
        a.try_enter_note('d'); // D# 4
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::D, 4), "D# snaps down to D");

        a.cursor_row = 1;
        a.try_enter_note('s'); // C# 4
        assert_eq!(note_at(&a, 1, 0), on(NoteValue::C, 4), "C# snaps down to C");

        // A note already in the scale is untouched.
        a.cursor_row = 2;
        a.try_enter_note('n'); // A 4
        assert_eq!(note_at(&a, 2, 0), on(NoteValue::A, 4));
    }

    #[test]
    fn upper_row_keys_map_one_octave_higher() {
        let mut a = app();
        a.current_octave = 4;
        a.try_enter_note('q');
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::C, 5));
    }

    #[test]
    fn the_full_piano_row_covers_a_chromatic_octave() {
        let expected = [
            ('z', NoteValue::C),
            ('s', NoteValue::Cs),
            ('x', NoteValue::D),
            ('d', NoteValue::Ds),
            ('c', NoteValue::E),
            ('v', NoteValue::F),
            ('g', NoteValue::Fs),
            ('b', NoteValue::G),
            ('h', NoteValue::Gs),
            ('n', NoteValue::A),
            ('j', NoteValue::As),
            ('m', NoteValue::B),
        ];
        for (key, value) in expected {
            let mut a = app();
            a.current_octave = 3;
            a.try_enter_note(key);
            assert_eq!(note_at(&a, 0, 0), on(value, 3), "key '{key}'");
        }
    }

    #[test]
    fn unmapped_keys_do_not_write_a_note() {
        let mut a = app();
        a.try_enter_note('k');
        assert_eq!(note_at(&a, 0, 0), None);
        assert_eq!(a.cursor_row, 0, "cursor must not advance on a no-op");
    }

    #[test]
    fn note_entry_beyond_octave_nine_is_rejected() {
        let mut a = app();
        a.current_octave = 9;
        a.try_enter_note('q'); // would be octave 10
        assert_eq!(note_at(&a, 0, 0), None);
    }

    #[test]
    fn note_entry_advances_the_cursor_by_the_edit_step() {
        let mut a = app();
        a.edit_step = 4;
        a.try_enter_note('z');
        assert_eq!(a.cursor_row, 4);
    }

    #[test]
    fn the_cursor_stops_at_the_last_row() {
        let mut a = app();
        a.edit_step = 8;
        a.cursor_row = 14;
        a.try_enter_note('z');
        assert_eq!(a.cursor_row, 15, "16-row pattern, last index is 15");
    }

    #[test]
    fn tracks_auto_fill_the_track_instrument() {
        let mut a = app();
        a.core.channels[0].default_instrument = Some(7);
        a.try_enter_note('z');
        let cell = a.core.song.cell_at(a.edit_order, 0, 0);
        assert_eq!(cell.instrument, Some(7));
    }

    #[test]
    fn entering_a_note_below_a_sample_note_inherits_its_instrument() {
        // The sliced-sample case: each slice is its own instrument, the track
        // is Midi-typed because the song predates persisted channel state,
        // and a note typed on an empty row used to fall through to the
        // built-in synth instead of playing the slice.
        let mut a = app();
        a.core.instruments[3].kind = rtrack_core::InstrumentKind::sample(3);
        a.core.song.set_cell(
            a.edit_order,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                instrument: Some(3),
                ..Cell::default()
            },
        );

        a.cursor_row = 4;
        a.try_enter_note('z');

        let cell = a.core.song.cell_at(a.edit_order, 4, 0);
        assert!(cell.note.is_some());
        assert_eq!(
            cell.instrument,
            Some(3),
            "the new note should sound like the sample above it"
        );
    }

    #[test]
    fn re_entering_a_note_keeps_the_instrument_already_in_the_cell() {
        let mut a = app();
        a.core.song.set_cell(
            a.edit_order,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                instrument: Some(1),
                ..Cell::default()
            },
        );
        a.core.song.set_cell(
            a.edit_order,
            4,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                instrument: Some(6),
                ..Cell::default()
            },
        );

        a.cursor_row = 4;
        a.try_enter_note('x');
        assert_eq!(
            a.core.song.cell_at(a.edit_order, 4, 0).instrument,
            Some(6),
            "editing a note must not retune it to the one above"
        );
    }

    #[test]
    fn a_track_without_a_default_fills_no_instrument() {
        let mut a = app();
        a.try_enter_note('z');
        let cell = a.core.song.cell_at(a.edit_order, 0, 0);
        assert_eq!(cell.instrument, None);
    }

    #[test]
    fn note_off_writes_a_note_off_cell() {
        let mut a = app();
        a.enter_note_off();
        assert_eq!(note_at(&a, 0, 0), Some(Note::Off));
    }

    #[test]
    fn editing_marks_the_song_dirty() {
        let mut a = app();
        assert!(!a.core.dirty);
        a.try_enter_note('z');
        assert!(a.core.dirty);
    }

    // -- Undo / redo --

    #[test]
    fn undo_restores_the_previous_cell_and_redo_reapplies_it() {
        let mut a = app();
        a.try_enter_note('z');
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::C, 4));

        a.apply_undo();
        assert_eq!(note_at(&a, 0, 0), None);
        assert_eq!(a.status_message.as_deref(), Some("Undo"));

        a.apply_redo();
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::C, 4));
        assert_eq!(a.status_message.as_deref(), Some("Redo"));
    }

    #[test]
    fn undo_walks_back_through_several_edits() {
        let mut a = app();
        a.try_enter_note('z'); // row 0
        a.try_enter_note('x'); // row 1
        a.try_enter_note('c'); // row 2

        a.apply_undo();
        assert_eq!(note_at(&a, 2, 0), None);
        assert_eq!(note_at(&a, 1, 0), on(NoteValue::D, 4));

        a.apply_undo();
        assert_eq!(note_at(&a, 1, 0), None);
        assert_eq!(note_at(&a, 0, 0), on(NoteValue::C, 4));

        a.apply_undo();
        assert_eq!(note_at(&a, 0, 0), None);
    }

    // -- Song settings undo, which the GUI did not have at all --

    #[test]
    fn a_settings_change_can_be_undone() {
        let mut app = app();
        let original = app.core.song.bpm;

        let before = app.core.song.clone();
        app.core.song.bpm = 160;
        app.record_settings_edit("song.bpm", before);

        app.apply_undo();
        assert_eq!(app.core.song.bpm, original);

        app.apply_redo();
        assert_eq!(app.core.song.bpm, 160);
    }

    /// A drag reports a change per frame. All of them are one step, and undo
    /// returns to where the value stood before the drag began.
    #[test]
    fn a_dragged_setting_is_one_undo_step() {
        let mut app = app();
        let original = app.core.song.bpm;
        let depth = app.core.history.undo_depth();

        for bpm in [121, 130, 145, 160] {
            let before = app.core.song.clone();
            app.core.song.bpm = bpm;
            app.record_settings_edit("song.bpm", before);
        }
        assert_eq!(
            app.core.history.undo_depth(),
            depth + 1,
            "a drag should leave one step"
        );

        app.apply_undo();
        assert_eq!(
            app.core.song.bpm, original,
            "undo should return to before the drag, not one frame back"
        );
    }

    #[test]
    fn two_different_settings_are_two_steps() {
        let mut app = app();
        let depth = app.core.history.undo_depth();

        let before = app.core.song.clone();
        app.core.song.bpm = 160;
        app.record_settings_edit("song.bpm", before);

        let before = app.core.song.clone();
        app.core.song.groove = vec![60, 40];
        app.record_settings_edit("song.groove", before);

        assert_eq!(app.core.history.undo_depth(), depth + 2);
    }

    #[test]
    fn closing_the_dialog_ends_the_run() {
        let mut app = app();
        let before = app.core.song.clone();
        app.core.song.bpm = 160;
        app.record_settings_edit("song.bpm", before);
        let depth = app.core.history.undo_depth();

        app.break_edit_coalescing();

        let before = app.core.song.clone();
        app.core.song.bpm = 170;
        app.record_settings_edit("song.bpm", before);
        assert_eq!(app.core.history.undo_depth(), depth + 1);
    }

    /// Shrinking the channel count truncates every pattern, so undo has to
    /// bring the cells back as well as the count.
    #[test]
    fn undoing_a_channel_count_change_restores_the_cells() {
        let mut app = app();
        app.mode = Mode::Insert;
        app.cursor_channel = 3;
        app.try_enter_note('z');
        let note = app.core.song.cell_at(app.edit_order, 0, 3).note;
        assert!(note.is_some(), "the note should have been entered");

        // Shrink to two channels, as the settings dialog does.
        let before = app.core.song.clone();
        app.core.song.channels = 2;
        for pat in &mut app.core.song.patterns {
            for row in &mut pat.data {
                row.truncate(2);
            }
            pat.channels = 2;
        }
        app.cursor_channel = 1;
        app.record_settings_edit("song.channels", before);

        app.apply_undo();
        assert_eq!(app.core.song.channels, 4);
        assert_eq!(
            app.core.song.cell_at(app.edit_order, 0, 3).note,
            note,
            "the truncated cells should have come back"
        );
    }

    // -- Structural undo, which the GUI could not do at all before --

    #[test]
    fn adding_a_pattern_can_be_undone() {
        let mut app = app();
        assert_eq!(app.core.song.patterns.len(), 1);
        assert_eq!(app.core.song.order.len(), 1);

        app.execute_action(Action::NewPattern);
        assert_eq!(app.core.song.patterns.len(), 2);
        assert_eq!(app.core.song.order.len(), 2);

        app.apply_undo();
        assert_eq!(
            app.core.song.patterns.len(),
            1,
            "the pattern should be gone"
        );
        assert_eq!(app.core.song.order.len(), 1);
    }

    #[test]
    fn adding_a_pattern_can_be_redone() {
        let mut app = app();
        app.execute_action(Action::NewPattern);
        app.apply_undo();
        app.apply_redo();
        assert_eq!(app.core.song.patterns.len(), 2);
        assert_eq!(app.core.song.order.len(), 2);
    }

    #[test]
    fn cloning_a_pattern_can_be_undone() {
        let mut app = app();
        app.execute_action(Action::ClonePattern);
        assert_eq!(app.core.song.patterns.len(), 2);

        app.apply_undo();
        assert_eq!(app.core.song.patterns.len(), 1);
    }

    #[test]
    fn undoing_a_pattern_add_brings_the_cursor_back_inside_the_song() {
        // `NewPattern` moves the edit cursor onto the pattern it made. Undo
        // removes that order entry, so a cursor left where it was would index
        // past the end on the next draw.
        let mut app = app();
        app.execute_action(Action::NewPattern);
        assert_eq!(app.edit_order, 1);

        app.apply_undo();
        assert!(
            app.edit_order < app.core.song.order.len(),
            "edit_order {} is past the end of a {}-entry order list",
            app.edit_order,
            app.core.song.order.len()
        );
    }

    /// Structural and cell steps share one stack, so they have to unwind in
    /// the order they were made.
    #[test]
    fn structural_and_cell_edits_undo_in_the_order_they_were_made() {
        let mut app = app();
        app.mode = Mode::Insert;
        app.execute_action(Action::TryEnterNote('z'));
        let with_note = app.core.song.patterns[0].get(0, 0).note;
        assert!(with_note.is_some(), "the note should have been entered");

        app.execute_action(Action::NewPattern);
        assert_eq!(app.core.song.patterns.len(), 2);

        // Newest first: the pattern, then the note.
        app.apply_undo();
        assert_eq!(app.core.song.patterns.len(), 1);
        assert_eq!(
            app.core.song.patterns[0].get(0, 0).note,
            with_note,
            "undoing the pattern must not touch the note"
        );

        app.apply_undo();
        assert!(app.core.song.patterns[0].get(0, 0).note.is_none());
    }

    #[test]
    fn undo_with_an_empty_history_is_a_no_op() {
        let mut a = app();
        a.apply_undo();
        assert_eq!(a.status_message, None);
        assert!(!a.core.dirty);
    }

    #[test]
    fn history_entries_naming_a_missing_pattern_are_skipped() {
        // Loading a smaller song while the undo stack still refers to
        // patterns from the previous one must not panic.
        let mut a = app();
        a.try_enter_note('z');
        a.core.history.push(vec![CellEdit {
            at: rtrack_core::tracker::CellAddr::Pattern {
                pattern: 99,
                row: 0,
                channel: 0,
            },
            old_cell: Cell::default(),
            new_cell: Cell::default(),
        }]);
        a.apply_undo();
        a.apply_undo();
        // Reached here without panicking; the real edit is still undone.
        assert_eq!(note_at(&a, 0, 0), None);
    }

    // -- Transpose --

    #[test]
    fn transpose_shifts_a_note_up_and_down() {
        let mut cell = Cell {
            note: on(NoteValue::C, 4),
            ..Cell::default()
        };
        cell.transpose_note(12, None);
        assert_eq!(cell.note, on(NoteValue::C, 5));
        cell.transpose_note(-13, None);
        assert_eq!(cell.note, on(NoteValue::B, 3));
    }

    #[test]
    fn transpose_moves_by_scale_degree_when_a_scale_is_set() {
        use rtrack_core::theory::{Scale, ScaleSetting};

        let mut a = app();
        a.core.song.scale = Some(ScaleSetting::new(NoteValue::C, Scale::Major));
        let pattern_idx = a.core.song.order[a.edit_order];
        a.core.song.patterns[pattern_idx].get_mut(0, 0).note = on(NoteValue::E, 4);

        a.transpose_notes(1);
        assert_eq!(
            a.core.song.patterns[pattern_idx].get(0, 0).note,
            on(NoteValue::F, 4),
            "one degree up from E in C major is F, not F#"
        );

        a.transpose_notes(-1);
        assert_eq!(
            a.core.song.patterns[pattern_idx].get(0, 0).note,
            on(NoteValue::E, 4)
        );
    }

    #[test]
    fn the_phrase_window_links_edits_and_unlinks_with_undo() {
        use crate::dialogs::PhraseAction;
        use rtrack_core::tracker::PhraseTiming;
        let mut a = app();
        let link = |a: &RtrackApp| a.core.song.pattern_at(a.edit_order).unwrap().link(0);

        a.apply_phrase_action(PhraseAction::LinkNew);
        assert_eq!(link(&a).map(|l| l.phrase), Some(0));

        // An edit through the grid lands in the phrase, and undoes there.
        a.cursor_sub = SubColumn::Volume;
        a.enter_hex_digit('5');
        assert_eq!(a.core.song.phrases[0].rows[0].volume, Some(5));
        a.apply_undo();
        assert_eq!(a.core.song.phrases[0].rows[0].volume, None);

        a.apply_phrase_action(PhraseAction::Transpose(7));
        assert_eq!(link(&a).map(|l| l.transpose), Some(7));

        a.apply_phrase_action(PhraseAction::Timing(true));
        assert_eq!(link(&a).map(|l| l.timing), Some(PhraseTiming::Running));

        // Typing a name is one undo step, not one per character.
        let depth = a.core.history.undo_depth();
        for name in ["B", "Ba", "Bass"] {
            a.apply_phrase_action(PhraseAction::Rename(0, name.to_string()));
        }
        assert_eq!(a.core.history.undo_depth(), depth + 1);

        a.apply_phrase_action(PhraseAction::Unlink);
        assert_eq!(link(&a), None);
        a.apply_undo();
        assert!(link(&a).is_some());
    }

    /// Run one egui pass in which `focus` is drawn and given keyboard focus,
    /// then one in which `key` is pressed and the app reads the keyboard.
    fn press_after_focusing(
        a: &mut RtrackApp,
        focus: impl Fn(&mut egui::Ui) -> egui::Response,
        key: Key,
    ) {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            focus(ui).request_focus();
        })
        .drop_without_applying_deltas();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            focus(ui);
            a.process_keys(ui.ctx());
        })
        .drop_without_applying_deltas();
    }

    /// Clicking a button or the grid focuses it. That must not take the
    /// keyboard from the grid, which it did when any focused widget counted.
    #[test]
    fn a_focused_button_leaves_the_keyboard_to_the_grid() {
        let mut a = app();
        press_after_focusing(&mut a, |ui| ui.button("Edit"), Key::I);
        assert_eq!(a.mode, Mode::Insert);
    }

    #[test]
    fn a_focused_text_field_keeps_keys_from_the_grid() {
        let mut a = app();
        let mut text = String::new();
        let text = std::cell::RefCell::new(&mut text);
        press_after_focusing(
            &mut a,
            |ui| ui.text_edit_singleline(*text.borrow_mut()),
            Key::I,
        );
        assert_eq!(a.mode, Mode::Normal);
    }

    #[test]
    fn two_hex_digits_make_one_value_in_one_cell() {
        let mut a = app();
        a.cursor_sub = SubColumn::Volume;
        a.enter_hex_digit('7');
        a.enter_hex_digit('0');
        assert_eq!(a.cursor_row, 0, "the cursor stays on the cell");
        assert_eq!(a.shown_cell(0, 0).volume, Some(0x70));
    }

    #[test]
    fn the_phrase_view_edits_the_tracks_phrase_and_returns_to_the_pattern() {
        use crate::dialogs::PhraseAction;
        let mut a = app();
        a.cursor_channel = 1;
        a.apply_phrase_action(PhraseAction::LinkNew);
        a.cursor_row = 5;

        a.execute_action(Action::TogglePhraseView);
        assert_eq!(a.phrase_view.map(|v| v.phrase), Some(0));
        assert_eq!(a.cursor_row, 5, "opens at the phrase row shown");

        a.cursor_sub = SubColumn::Volume;
        a.enter_hex_digit('7');
        assert_eq!(a.core.song.phrases[0].rows[5].volume, Some(7));
        assert_eq!(a.core.song.cell_at(a.edit_order, 5, 1).volume, Some(7));

        // The cursor stays in the phrase's column.
        a.cursor_sub = SubColumn::Effect;
        a.execute_action(Action::CursorRight);
        a.execute_action(Action::NextChannel);
        a.cursor_sub = SubColumn::Note;
        a.execute_action(Action::CursorLeft);
        assert_eq!(a.cursor_channel, 1);

        a.cursor_row = 0;
        assert_eq!(a.pattern_row(), 5, "playback starts from the pattern row");
        a.apply_phrase_action(PhraseAction::Close);
        assert!(a.phrase_view.is_none());
        assert_eq!(a.cursor_row, 5);
    }

    #[test]
    fn the_phrase_view_steps_adds_and_survives_undo() {
        use crate::dialogs::PhraseAction;
        let mut a = app();
        a.execute_action(Action::TogglePhraseView);
        assert!(a.phrase_view.is_none(), "nothing to show yet");

        a.core.song.add_phrase(4);
        a.core.song.phrases[0].rows[1].volume = Some(9);
        a.execute_action(Action::TogglePhraseView);
        let shown = |a: &RtrackApp| a.phrase_view.map(|v| v.phrase);
        assert_eq!(shown(&a), Some(0), "an unlinked track opens phrase 00");

        a.execute_action(Action::ClonePattern);
        assert_eq!(shown(&a), Some(1));
        assert_eq!(a.core.song.phrases[1].rows[1].volume, Some(9));
        a.execute_action(Action::NewPattern);
        assert_eq!(shown(&a), Some(2));
        assert_eq!(a.core.song.order.len(), 1, "no pattern was added");
        assert_eq!(a.current_pattern_rows(), a.core.song.rows_at(a.edit_order));

        a.execute_action(Action::NextOrder);
        assert_eq!(shown(&a), Some(2), "stops at the last phrase");
        a.execute_action(Action::PrevOrder);
        assert_eq!(shown(&a), Some(1));

        a.apply_phrase_action(PhraseAction::Show(2));
        a.apply_undo();
        assert_eq!(a.core.song.phrases.len(), 2);
        assert_eq!(shown(&a), Some(1), "the undone phrase is no longer shown");
    }

    #[test]
    fn pattern_rows_insert_and_delete_with_undo() {
        let mut a = app();
        let pattern_idx = a.core.song.order[a.edit_order];
        a.core.song.patterns[pattern_idx].data[0][0].volume = Some(3);
        let rows = a.core.song.patterns[pattern_idx].rows;

        a.execute_action(Action::InsertRow);
        assert_eq!(a.core.song.patterns[pattern_idx].rows, rows + 1);
        a.apply_undo();
        assert_eq!(a.core.song.patterns[pattern_idx].rows, rows);

        a.execute_action(Action::DeleteRow);
        assert_eq!(a.core.song.patterns[pattern_idx].data[0][0].volume, None);
        a.apply_undo();
        assert_eq!(a.core.song.patterns[pattern_idx].data[0][0].volume, Some(3));
    }

    #[test]
    fn phrase_rows_insert_and_delete_with_undo() {
        let mut a = app();
        let p = a.core.song.add_phrase(4);
        a.core.song.phrases[p].rows[0].volume = Some(2);
        a.open_phrase_view(Some(p));

        a.execute_action(Action::InsertRow);
        assert_eq!(a.core.song.phrases[p].rows[1].volume, Some(2));
        assert_eq!(a.core.song.phrases[p].rows.len(), 4);
        a.apply_undo();
        assert_eq!(a.core.song.phrases[p].rows[0].volume, Some(2));

        a.cursor_row = 0;
        a.execute_action(Action::DeleteRow);
        assert_eq!(a.core.song.phrases[p].rows[0].volume, None);
        a.apply_undo();
        assert_eq!(a.core.song.phrases[p].rows[0].volume, Some(2));
    }

    #[test]
    fn the_table_window_builds_and_deletes_a_table_with_undo() {
        use crate::dialogs::TableAction;
        let mut a = app();
        let table = |a: &RtrackApp| a.core.song.tables.get(&5).cloned();

        a.apply_table_action(TableAction::Select(5));
        assert!(table(&a).is_none(), "selecting creates nothing");
        a.apply_table_action(TableAction::Rows(3));
        a.apply_table_action(TableAction::Transpose(1, 4));
        a.apply_table_action(TableAction::Transpose(2, 7));
        a.apply_table_action(TableAction::Volume(0, Some(0x30)));
        a.apply_table_action(TableAction::Loop(Some(0)));

        // Dragging the ticks is one undo step.
        let depth = a.core.history.undo_depth();
        for t in 2..=4 {
            a.apply_table_action(TableAction::Ticks(t));
        }
        assert_eq!(a.core.history.undo_depth(), depth + 1);

        a.apply_table_action(TableAction::Effect(2, "JMP00".to_string()));
        assert!(a
            .status_message
            .as_deref()
            .is_some_and(|m| m.contains("JMP")));
        a.apply_table_action(TableAction::Effect(2, "pvb44".to_string()));
        let t = table(&a).unwrap();
        let transposes: Vec<i8> = t.rows.iter().map(|r| r.transpose).collect();
        assert_eq!(transposes, vec![0, 4, 7]);
        assert_eq!(
            (t.ticks_per_row, t.loop_from, t.rows[0].volume),
            (4, Some(0), Some(0x30))
        );
        assert!(t.rows[2].fx.is_some());

        a.apply_table_action(TableAction::Delete);
        assert!(table(&a).is_none());
        a.apply_undo();
        assert_eq!(table(&a), Some(t));
    }

    #[test]
    fn transpose_is_undoable() {
        let mut a = app();
        a.try_enter_note('z');
        a.cursor_row = 0;
        let before = note_at(&a, 0, 0);
        a.transpose_notes(1);
        assert_ne!(note_at(&a, 0, 0), before);
        a.apply_undo();
        assert_eq!(note_at(&a, 0, 0), before);
    }

    #[test]
    fn transpose_stays_chromatic_without_a_scale() {
        let mut a = app();
        assert_eq!(a.core.song.scale, None);
        let pattern_idx = a.core.song.order[a.edit_order];
        a.core.song.patterns[pattern_idx].get_mut(0, 0).note = on(NoteValue::E, 4);

        a.transpose_notes(1);
        assert_eq!(
            a.core.song.patterns[pattern_idx].get(0, 0).note,
            on(NoteValue::F, 4)
        );
        a.transpose_notes(1);
        assert_eq!(
            a.core.song.patterns[pattern_idx].get(0, 0).note,
            on(NoteValue::Fs, 4),
            "no scale means semitones"
        );
    }

    #[test]
    fn transpose_leaves_empty_and_note_off_cells_alone() {
        let mut empty = Cell::default();
        empty.transpose_note(5, None);
        assert_eq!(empty.note, None);

        let mut off = Cell {
            note: Some(Note::Off),
            ..Cell::default()
        };
        off.transpose_note(5, None);
        assert_eq!(off.note, Some(Note::Off));
    }

    #[test]
    fn transpose_past_the_midi_range_is_refused() {
        let mut low = Cell {
            note: on(NoteValue::C, 0),
            ..Cell::default()
        };
        low.transpose_note(-1, None);
        assert_eq!(low.note, on(NoteValue::C, 0), "would fall below MIDI 0");

        let mut high = Cell {
            note: on(NoteValue::G, 10),
            ..Cell::default()
        };
        let before = high.note;
        high.transpose_note(12, None);
        assert_eq!(high.note, before, "would exceed MIDI 127");
    }

    // -- Cursor / sub-column navigation --

    #[test]
    fn sub_columns_cycle_in_both_directions() {
        let order = [
            SubColumn::Note,
            SubColumn::Instrument,
            SubColumn::Volume,
            SubColumn::Effect,
        ];
        for (i, sub) in order.iter().enumerate() {
            assert_eq!(sub.next(1), order[(i + 1) % order.len()]);
            assert_eq!(sub.prev(1), order[(i + order.len() - 1) % order.len()]);
        }
    }

    #[test]
    fn editing_an_out_of_range_order_position_does_not_panic() {
        let mut a = app();
        a.edit_order = 99;
        a.try_enter_note('z');
        a.enter_note_off();
        assert_eq!(a.current_pattern_rows(), 16, "falls back to song default");
    }
}
