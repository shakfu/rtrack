use egui::Key;
use rtrack_core::constants::*;
use rtrack_core::midi::MidiInputEvent;
use rtrack_core::tracker::{Cell, Note};

use crate::app::RtrackApp;
use crate::history::{CellEdit, Edit};
use crate::menu::ExportFormat;
use crate::state::{Mode, SubColumn};

impl RtrackApp {
    pub fn process_keys(&mut self, ctx: &egui::Context) {
        // Keys typed into a text field are not editor commands. Save is the
        // one shortcut that still applies.
        if ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| (i.modifiers.ctrl || i.modifiers.mac_cmd) && i.key_pressed(Key::S)) {
                self.execute_action(Action::Save);
            }
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

                        // Piano keys and hex digits via text events
                        for event in &input.events {
                            if let egui::Event::Text(text) = event {
                                for c in text.chars() {
                                    if self.cursor_sub == SubColumn::Note {
                                        actions.push(Action::TryEnterNote(c));
                                    } else if c.is_ascii_hexdigit()
                                        || (self.cursor_sub.fx_slot().is_some()
                                            && c.is_ascii_alphabetic())
                                    {
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
                    self.break_edit_coalescing();
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
                if self.cursor_sub == SubColumn::LAST && self.cursor_channel > 0 {
                    self.cursor_channel -= 1;
                }
            }
            Action::CursorRight => {
                let old = self.cursor_sub;
                self.cursor_sub = self.cursor_sub.next();
                if old == SubColumn::LAST && self.cursor_channel + 1 < self.core.song.channels {
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
                self.apply_undo();
            }
            Action::Redo => {
                self.apply_redo();
            }
            Action::Copy => {
                if let (Some((r1, c1)), Some((r2, c2))) = (self.block_start, self.block_end) {
                    // Block copy
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let min_r = r1.min(r2);
                    let max_r = r1.max(r2);
                    let min_c = c1.min(c2);
                    let max_c = c1.max(c2);
                    let song = &self.core.song;
                    let block: Vec<Vec<Cell>> = (min_r..=max_r)
                        .map(|row| {
                            (min_c..=max_c)
                                .map(|ch| song.cell(pattern_idx, row, ch))
                                .collect()
                        })
                        .collect();
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.clipboard.set_block(block);
                    self.status_message = Some(format!("Block copied ({}x{})", rows, chs));
                } else {
                    // Single cell copy
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let cell =
                        self.core
                            .song
                            .cell(pattern_idx, self.cursor_row, self.cursor_channel);
                    self.clipboard.set_cell(cell);
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
                            row_cells.push(self.core.song.cell(pattern_idx, row, ch));
                            edits.extend(
                                self.edit_view(pattern_idx, row, ch, |c| *c = Cell::default()),
                            );
                        }
                        block.push(row_cells);
                    }
                    let rows = block.len();
                    let chs = block.first().map_or(0, |r| r.len());
                    self.clipboard.set_block(block);
                    self.history.push(edits);
                    self.core.dirty = true;
                    self.block_start = None;
                    self.block_end = None;
                    self.status_message = Some(format!("Block cut ({}x{})", rows, chs));
                } else {
                    // Single cell cut
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let (row, ch) = (self.cursor_row, self.cursor_channel);
                    self.clipboard
                        .set_cell(self.core.song.cell(pattern_idx, row, ch));
                    if let Some(e) = self.edit_view(pattern_idx, row, ch, |c| *c = Cell::default())
                    {
                        self.history.push(vec![e]);
                    }
                    self.core.dirty = true;
                    self.status_message = Some("Cut".to_string());
                }
            }
            Action::Paste => {
                if let Some(block) = self.clipboard.block().cloned() {
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
                            edits.extend(self.edit_view(pattern_idx, row, ch, |c| *c = *clip_cell));
                        }
                    }
                    self.history.push(edits);
                    self.core.dirty = true;
                    self.status_message = Some("Block pasted".to_string());
                } else if let Some(clip) = self.clipboard.cell() {
                    // Single cell paste
                    let pattern_idx = self.core.song.order[self.edit_order];
                    let (row, ch) = (self.cursor_row, self.cursor_channel);
                    if let Some(e) = self.edit_view(pattern_idx, row, ch, |c| *c = clip) {
                        self.history.push(vec![e]);
                    }
                    self.core.dirty = true;

                    // Advance cursor
                    let step = self.edit_step;
                    let max_row = self.current_pattern_rows().saturating_sub(1);
                    self.cursor_row = (self.cursor_row + step).min(max_row);
                    self.status_message = Some("Pasted".to_string());
                }
            }
            // Structural: later cell edits name rows by index, so a row
            // shift outside the history would make their undo hit the wrong
            // rows.
            Action::InsertRow => {
                let pattern_idx = self.core.song.order[self.edit_order];
                if self.core.song.patterns[pattern_idx].rows < 256 {
                    let before = self.begin_structural_edit();
                    let pattern = &mut self.core.song.patterns[pattern_idx];
                    let channels = pattern.channels;
                    pattern
                        .data
                        .insert(self.cursor_row, vec![Cell::default(); channels]);
                    pattern.rows += 1;
                    self.end_structural_edit(before);
                }
            }
            Action::DeleteRow => {
                let pattern_idx = self.core.song.order[self.edit_order];
                if self.core.song.patterns[pattern_idx].rows > 1 {
                    let before = self.begin_structural_edit();
                    let pattern = &mut self.core.song.patterns[pattern_idx];
                    pattern.data.remove(self.cursor_row);
                    pattern.rows -= 1;
                    if self.cursor_row >= pattern.rows {
                        self.cursor_row = pattern.rows - 1;
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
            Action::NewPattern => {
                if self.refuse_at_pattern_limit() {
                    return;
                }
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
                if self.refuse_at_pattern_limit() {
                    return;
                }
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
                    let before = self.begin_structural_edit();
                    self.core.song.order[self.matrix_cursor] = cur - 1;
                    self.end_structural_edit(before);
                }
            }
            Action::MatrixNextPattern => {
                let cur = self.core.song.order[self.matrix_cursor];
                if cur + 1 < self.core.song.patterns.len() {
                    let before = self.begin_structural_edit();
                    self.core.song.order[self.matrix_cursor] = cur + 1;
                    self.end_structural_edit(before);
                }
            }
            Action::MatrixNewPattern => {
                if self.refuse_at_pattern_limit() {
                    return;
                }
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
                if self.refuse_at_pattern_limit() {
                    return;
                }
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
                    let before = self.begin_structural_edit();
                    self.core.song.order_repeats[self.matrix_cursor] = cur - 1;
                    self.end_structural_edit(before);
                }
            }
            Action::MatrixIncRepeat => {
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur < 99 {
                    let before = self.begin_structural_edit();
                    self.core.song.order_repeats[self.matrix_cursor] = cur + 1;
                    self.end_structural_edit(before);
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
            Action::ExportMidi => self.export(ExportFormat::Midi),
            Action::ExportWav => self.export(ExportFormat::Wav),
            Action::ExportFlac => self.export(ExportFormat::Flac),
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
                    let pattern_idx = self.core.song.order[self.edit_order];
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
                    let mut edits = Vec::new();
                    for ch in min_c..=max_c {
                        let first = self.core.song.cell(pattern_idx, min_r, ch);
                        let last = self.core.song.cell(pattern_idx, max_r, ch);
                        for row in min_r..=max_r {
                            let t = (row - min_r) as f64 / span as f64;
                            let edit = self.edit_view(pattern_idx, row, ch, |cell| {
                                // Volume, and each effect slot whose endpoints
                                // hold the same command
                                if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                                    let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                                    cell.volume = Some(v.round() as u8);
                                }
                                for slot in 0..rtrack_core::tracker::FX_COLUMNS {
                                    if let Some(fx) =
                                        rtrack_core::editor::interpolate_fx(&first, &last, slot, t)
                                    {
                                        cell.fx[slot] = Some(fx);
                                    }
                                }
                            });
                            edits.extend(edit.filter(|e| e.old_cell != e.new_cell));
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

    /// Roll one edit group back. Shared by the Edit menu and Ctrl+Z so the
    /// two entry points cannot drift apart.
    pub(crate) fn apply_undo(&mut self) {
        if let Some(edit) = self.history.undo() {
            self.apply_edit_backward(edit);
            self.core.dirty = true;
            self.status_message = Some("Undo".to_string());
        }
    }

    /// Put back the "before" side of one step.
    fn apply_edit_backward(&mut self, edit: Edit) {
        match edit {
            // In reverse: an aliased column can store two edits in one cell.
            Edit::Cells(edits) => {
                for e in edits.iter().rev() {
                    self.write_recorded_cell(e.pattern_idx, e.row, e.channel, e.old_cell);
                }
            }
            Edit::Bank(bank) => self.core.restore_samples(bank.before),
            Edit::Channel(c) => self.core.restore_channel(c.index, c.before),
            Edit::Structure(s) => self.restore_song(s.before),
            // Undone in reverse, so the parts come apart the way they were
            // put together.
            Edit::Group(parts) => {
                for part in parts.into_iter().rev() {
                    self.apply_edit_backward(part);
                }
            }
        }
    }

    /// Re-apply one edit group. Counterpart to [`RtrackApp::apply_undo`].
    pub(crate) fn apply_redo(&mut self) {
        if let Some(edit) = self.history.redo() {
            self.apply_edit_forward(edit);
            self.core.dirty = true;
            self.status_message = Some("Redo".to_string());
        }
    }

    /// Re-apply the "after" side of one step.
    fn apply_edit_forward(&mut self, edit: Edit) {
        match edit {
            Edit::Cells(edits) => {
                for e in &edits {
                    self.write_recorded_cell(e.pattern_idx, e.row, e.channel, e.new_cell);
                }
            }
            Edit::Bank(bank) => self.core.restore_samples(bank.after),
            Edit::Channel(c) => self.core.restore_channel(c.index, c.after),
            Edit::Structure(s) => self.restore_song(s.after),
            Edit::Group(parts) => {
                for part in parts {
                    self.apply_edit_forward(part);
                }
            }
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
        self.history.push_from(
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
        self.history.break_coalescing();
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
        self.history.push_structure(before, self.core.song.clone());
        self.core.dirty = true;
    }

    /// Say so and return true if the song cannot take another pattern.
    pub(crate) fn refuse_at_pattern_limit(&mut self) -> bool {
        let full = self.core.song.at_pattern_limit();
        if full {
            self.status_message = Some(format!("Pattern limit reached ({MAX_PATTERNS})"));
        }
        full
    }

    /// Put back a whole song recorded by a structural edit.
    ///
    /// The cursor has to be brought back inside it: the song being restored
    /// may have fewer patterns, channels, or rows than the one on screen --
    /// undoing "add pattern" is exactly that case -- and a cursor left past
    /// the end would index out of range on the next draw.
    fn restore_song(&mut self, song: rtrack_core::tracker::Song) {
        self.core.restore_song(song);
        self.clamp_cursor_to_song();
    }

    /// Bring the cursor and the visible channel window inside the song.
    pub(crate) fn clamp_cursor_to_song(&mut self) {
        let order_len = self.core.song.order.len().max(1);
        if self.edit_order >= order_len {
            self.edit_order = order_len - 1;
        }
        let rows = self
            .core
            .song
            .pattern_at(self.edit_order)
            .map(|p| p.rows)
            .unwrap_or(self.core.song.rows_per_pattern)
            .max(1);
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

    /// Write a cell recorded in the history. History entries can outlive the
    /// pattern they name (a song can be loaded while the stacks still hold
    /// edits), so an entry that no longer resolves is skipped, not indexed.
    fn write_recorded_cell(&mut self, pattern_idx: usize, row: usize, channel: usize, cell: Cell) {
        if let Some(pattern) = self.core.song.patterns.get_mut(pattern_idx) {
            if row < pattern.rows && channel < pattern.channels {
                pattern.data[row][channel] = cell;
            }
        }
    }

    /// Edit a cell as it plays, through any column alias. Returns the undo
    /// record: the stored cells where they are stored.
    pub(crate) fn edit_view(
        &mut self,
        pattern_idx: usize,
        row: usize,
        channel: usize,
        f: impl FnOnce(&mut Cell),
    ) -> Option<CellEdit> {
        self.core
            .song
            .edit_cell(pattern_idx, row, channel, f)
            .map(CellEdit::from)
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
            .resolve_edit_instrument(self.edit_order, self.cursor_row, ch);

        if let Some(midi_note) = note.to_midi_note() {
            self.core.preview_note_for_cell(
                self.edit_order,
                self.cursor_row,
                ch,
                midi_note,
                MIDI_DEFAULT_VELOCITY,
            );
        }

        let Some(pattern_idx) = self.current_pattern_idx() else {
            return;
        };
        let Some(e) = self.edit_view(pattern_idx, self.cursor_row, ch, |cell| {
            cell.note = Some(note);
            if let Some(inst) = track_inst {
                cell.instrument = Some(inst);
            }
        }) else {
            return;
        };
        self.history.push(vec![e]);
        self.core.dirty = true;

        // Advance cursor
        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    /// Type into the instrument, volume or an effect slot at the cursor.
    ///
    /// Instrument and volume advance the cursor by the edit step after each
    /// digit. An effect slot does not: its command name and parameter take
    /// several keys.
    fn enter_hex_digit(&mut self, c: char) {
        let pattern_idx = self.core.song.order[self.edit_order];
        let (row, ch, sub) = (self.cursor_row, self.cursor_channel, self.cursor_sub);
        let pos = (
            self.edit_order,
            row,
            ch,
            sub.fx_slot().unwrap_or(usize::MAX),
        );
        let mut new_cell = self.core.song.cell(pattern_idx, row, ch);
        let key = rtrack_core::editor::type_into(&mut new_cell, sub, c, self.fx_typing.at(pos));
        if key != rtrack_core::tracker::FxKey::Changed {
            return;
        }
        match self.edit_view(pattern_idx, row, ch, |cell| *cell = new_cell) {
            Some(e) if e.old_cell != e.new_cell => self.history.push(vec![e]),
            _ => return,
        }
        self.core.dirty = true;

        if sub.fx_slot().is_none() {
            let step = self.edit_step;
            let max_row = self.current_pattern_rows().saturating_sub(1);
            self.cursor_row = (self.cursor_row + step).min(max_row);
        }
    }

    fn clear_cell(&mut self) {
        let pattern_idx = self.core.song.order[self.edit_order];
        let (row, ch, sub) = (self.cursor_row, self.cursor_channel, self.cursor_sub);
        let edit = self.edit_view(pattern_idx, row, ch, |cell| match sub {
            SubColumn::Note => {
                cell.note = None;
                cell.instrument = None;
            }
            sub => rtrack_core::editor::clear_field(cell, sub),
        });
        self.fx_typing.reset();
        if let Some(e) = edit {
            self.history.push(vec![e]);
        }
        self.core.dirty = true;
    }

    fn enter_note_off(&mut self) {
        let Some(pattern_idx) = self.current_pattern_idx() else {
            return;
        };
        let (row, ch) = (self.cursor_row, self.cursor_channel);
        let Some(e) = self.edit_view(pattern_idx, row, ch, |cell| cell.note = Some(Note::Off))
        else {
            return;
        };
        self.history.push(vec![e]);
        self.core.dirty = true;

        let step = self.edit_step;
        let max_row = self.current_pattern_rows().saturating_sub(1);
        self.cursor_row = (self.cursor_row + step).min(max_row);
    }

    fn current_pattern_rows(&self) -> usize {
        self.core.song.rows_at(self.edit_order)
    }

    /// Index of the pattern the edit cursor currently points at, or None if
    /// the order position does not resolve to one.
    fn current_pattern_idx(&self) -> Option<usize> {
        let pattern_idx = *self.core.song.order.get(self.edit_order)?;
        (pattern_idx < self.core.song.patterns.len()).then_some(pattern_idx)
    }

    fn transpose_notes(&mut self, steps: i8) {
        // Copied out before the pattern is borrowed mutably.
        let scale = self.core.song.scale;
        let pattern_idx = self.core.song.order[self.edit_order];

        if let (Some(start), Some(end)) = (self.block_start, self.block_end) {
            let r0 = start.0.min(end.0);
            let r1 = start.0.max(end.0);
            let c0 = start.1.min(end.1);
            let c1 = start.1.max(end.1);
            let mut edits = Vec::new();
            for r in r0..=r1 {
                for c in c0..=c1 {
                    edits.extend(
                        self.edit_view(pattern_idx, r, c, |cell| cell.transpose_note(steps, scale)),
                    );
                }
            }
            self.history.push(edits);
            self.core.dirty = true;
            let unit = if scale.is_some() {
                "scale degree(s)"
            } else {
                "semitone(s)"
            };
            self.status_message = Some(format!("Transposed block by {} {}", steps, unit));
        } else {
            let (row, ch) = (self.cursor_row, self.cursor_channel);
            if let Some(e) = self.edit_view(pattern_idx, row, ch, |cell| {
                cell.transpose_note(steps, scale)
            }) {
                self.history.push(vec![e]);
            }
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
                    let order = self.current_order_position();
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
                let order = self.current_order_position();
                self.core
                    .preview_note_for_cell(order, self.cursor_row, ch, note, velocity);
            }
            MidiInputEvent::NoteOff { channel: _, note } => {
                self.core.send_note_off(self.cursor_channel, note);

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
                if let Some(msg) = self
                    .core
                    .handle_midi_cc(controller, value, self.cursor_channel)
                {
                    self.status_message = Some(msg);
                }
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                self.core.send_pitch_bend(self.cursor_channel, value);
            }
            MidiInputEvent::ProgramChange {
                channel: _,
                program,
            } => {
                self.core.send_program_change(self.cursor_channel, program);
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
        a.core.instruments[3].kind = rtrack_core::InstrumentKind::Sample { slot: 3 };
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
        let depth = app.history.undo_depth();

        for bpm in [121, 130, 145, 160] {
            let before = app.core.song.clone();
            app.core.song.bpm = bpm;
            app.record_settings_edit("song.bpm", before);
        }
        assert_eq!(
            app.history.undo_depth(),
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
        let depth = app.history.undo_depth();

        let before = app.core.song.clone();
        app.core.song.bpm = 160;
        app.record_settings_edit("song.bpm", before);

        let before = app.core.song.clone();
        app.core.song.groove = vec![7, 5];
        app.record_settings_edit("song.groove", before);

        assert_eq!(app.history.undo_depth(), depth + 2);
    }

    #[test]
    fn a_channel_slider_drag_is_one_step_and_mute_alone_is_none() {
        let mut app = app();
        for volume in [0.9, 0.7, 0.5] {
            let before = app.core.channels[1].clone();
            app.core.channels[1].volume = volume;
            app.record_channel_edit(1, before);
        }
        let before = app.core.channels[1].clone();
        app.core.channels[1].muted = true;
        app.record_channel_edit(1, before);
        assert_eq!(app.history.undo_depth(), 1);

        app.apply_undo();
        assert_eq!(app.core.channels[1].volume, 1.0);
        assert!(app.core.channels[1].muted);
    }

    #[test]
    fn the_field_hint_follows_the_cursor_and_can_be_off() {
        let mut a = app();
        a.cursor_channel = 1;
        a.cursor_sub = SubColumn::Fx2;
        assert_eq!(a.field_hint().as_deref(), Some("2 Effect 2"));
        a.field_hints = false;
        assert_eq!(a.field_hint(), None);
    }

    #[test]
    fn closing_the_dialog_ends_the_run() {
        let mut app = app();
        let before = app.core.song.clone();
        app.core.song.bpm = 160;
        app.record_settings_edit("song.bpm", before);
        let depth = app.history.undo_depth();

        app.break_edit_coalescing();

        let before = app.core.song.clone();
        app.core.song.bpm = 170;
        app.record_settings_edit("song.bpm", before);
        assert_eq!(app.history.undo_depth(), depth + 1);
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

        app.core.channels[3].name = "Keep".to_string();

        // Shrink to two channels, as the settings dialog does.
        let before = app.core.song.clone();
        app.core.set_channel_count(2);
        app.cursor_channel = 1;
        app.record_settings_edit("song.channels", before);

        app.apply_undo();
        assert_eq!(app.core.song.channels, 4);
        assert_eq!(
            app.core.song.cell_at(app.edit_order, 0, 3).note,
            note,
            "the truncated cells should have come back"
        );
        // The pattern matrix indexes `channels` by song channel.
        assert!(app.core.channels.len() >= 4);
        assert_eq!(app.core.channels[3].name, "Keep");
    }

    #[test]
    fn row_insert_and_delete_are_undoable_and_keep_later_undos_aligned() {
        let mut app = app();
        app.mode = Mode::Insert;
        app.cursor_row = 2;
        app.try_enter_note('z');
        let entered = note_at(&app, 2, 0);
        app.mode = Mode::Normal;

        app.cursor_row = 0;
        app.execute_action(Action::DeleteRow);
        assert_eq!(note_at(&app, 1, 0), entered);
        app.apply_undo();
        assert_eq!(note_at(&app, 2, 0), entered, "the deleted row came back");

        app.cursor_row = 0;
        app.execute_action(Action::InsertRow);
        assert_eq!(note_at(&app, 3, 0), entered);
        app.apply_undo();
        assert_eq!(note_at(&app, 2, 0), entered);
        app.apply_undo();
        assert_eq!(
            note_at(&app, 2, 0),
            None,
            "the note entry undid in its own row"
        );
    }

    #[test]
    fn matrix_pattern_assignment_and_repeats_are_undoable_and_mark_the_song() {
        let mut app = app();
        app.core.song.add_pattern();
        app.matrix_cursor = 0;
        app.execute_action(Action::MatrixNextPattern);
        app.execute_action(Action::MatrixIncRepeat);
        assert!(app.core.dirty);
        assert_eq!(app.core.song.order[0], 1);
        app.apply_undo();
        app.apply_undo();
        assert_eq!(app.core.song.order[0], 0);
        assert_eq!(app.core.song.order_repeats[0], 1);
    }

    #[test]
    fn no_pattern_is_added_past_the_limit() {
        let mut app = app();
        while !app.core.song.at_pattern_limit() {
            app.core.song.add_pattern();
        }
        let order = app.core.song.order.clone();
        app.execute_action(Action::NewPattern);
        app.execute_action(Action::ClonePattern);
        assert_eq!(app.core.song.patterns.len(), MAX_PATTERNS);
        assert_eq!(app.core.song.order, order);
    }

    #[test]
    fn opening_over_unsaved_work_waits_for_an_answer() {
        let mut app = app();
        app.core.dirty = true;
        let title = app.core.song.title.clone();
        app.request_song_change(crate::menu::PendingSongChange::New);
        assert!(app.pending_song_change.is_some());
        assert_eq!(app.core.song.title, title);
        assert!(app.core.dirty, "nothing was replaced yet");
    }

    #[test]
    fn typing_in_a_text_field_does_not_drive_the_editor() {
        let mut app = app();
        let ctx = egui::Context::default();
        let mut text = String::new();
        let id = egui::Id::new("field");
        let mut frame = |events: Vec<egui::Event>, app: &mut RtrackApp| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id))
                    .request_focus();
                app.process_keys(ui.ctx());
            });
            out.textures_delta.clear();
        };
        frame(Vec::new(), &mut app);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![key(Key::I), key(Key::Space)], &mut app);
        assert_eq!(app.mode, Mode::Normal, "'i' was typed, not a mode switch");
        assert!(!app.core.playing);
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
        a.history.push(vec![rtrack_core::editor::CellEdit {
            pattern_idx: 99,
            row: 0,
            channel: 0,
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
        let pattern_idx = a.current_pattern_idx().unwrap();
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
    fn transpose_stays_chromatic_without_a_scale() {
        let mut a = app();
        assert_eq!(a.core.song.scale, None);
        let pattern_idx = a.current_pattern_idx().unwrap();
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
            SubColumn::Fx1,
            SubColumn::Fx2,
        ];
        for (i, sub) in order.iter().enumerate() {
            assert_eq!(sub.next(), order[(i + 1) % order.len()]);
            assert_eq!(sub.prev(), order[(i + order.len() - 1) % order.len()]);
        }
    }

    #[test]
    fn typing_in_an_effect_slot_spells_a_command_and_stays_on_the_row() {
        use rtrack_core::tracker::{Fx, FxCmd};
        let mut a = app();
        a.cursor_sub = SubColumn::Fx1;
        for c in "ccv7f".chars() {
            a.enter_hex_digit(c);
        }
        assert_eq!(a.cursor_row, 0, "effect typing does not advance");
        assert_eq!(
            a.core.song.cell_at(a.edit_order, 0, 0).fx,
            Fx::only(FxCmd::CcValue, 0x7F)
        );
    }

    #[test]
    fn edits_through_an_alias_land_in_the_source_and_undo_there() {
        use rtrack_core::tracker::ColumnAlias;
        let mut a = app();
        let src = a.core.song.add_pattern();
        let here = a.core.song.order[a.edit_order];
        a.core
            .song
            .set_alias(
                here,
                0,
                ColumnAlias {
                    source: src,
                    transpose: 0,
                },
            )
            .unwrap();
        a.enter_note_off();
        assert_eq!(a.core.song.patterns[src].data[0][0].note, Some(Note::Off));
        a.apply_undo();
        assert_eq!(a.core.song.patterns[src].data[0][0].note, None);
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
