use rtrack_core::TrackerCore;
use rtrack_core::tracker::Cell;

use crate::app::RtrackApp;
use crate::state::SubColumn;

impl RtrackApp {
    pub fn draw_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        self.core = TrackerCore::with_song_size(8, 64);
                        // Re-init audio on the new core
                        match rtrack_core::audio::AudioEngine::new(None) {
                            Ok(engine) => {
                                self.core.audio = Some(engine);
                            }
                            Err(e) => {
                                eprintln!("Audio warning: {}", e);
                            }
                        }
                        self.reset_cursor_state();
                        self.history.clear();
                        self.clipboard = None;
                        self.status_message = Some("New song created".to_string());
                        ui.close_menu();
                    }

                    if ui.button("Open...").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("rtrack", &["rtrk"])
                            .pick_file()
                        {
                            match self.core.load_file(&path) {
                                Ok(msg) => {
                                    self.reset_cursor_state();
                                    self.history.clear();
                                    self.clipboard = None;
                                    self.status_message = Some(msg);
                                }
                                Err(msg) => {
                                    self.status_message = Some(msg);
                                }
                            }
                        }
                    }

                    if ui.button("Save  (Ctrl+S)").clicked() {
                        ui.close_menu();
                        self.do_save();
                    }

                    if ui.button("Save As...").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("rtrack", &["rtrk"])
                            .save_file()
                        {
                            self.core.file_path = Some(path);
                            self.do_save();
                        }
                    }

                    ui.separator();

                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(self.history.can_undo(), egui::Button::new("Undo  (Ctrl+Z)")).clicked() {
                        if let Some(edits) = self.history.undo() {
                            for edit in &edits {
                                let cell = self.core.song.patterns[edit.pattern_idx]
                                    .get_mut(edit.row, edit.channel);
                                *cell = edit.old_cell;
                            }
                            self.core.dirty = true;
                            self.status_message = Some("Undo".to_string());
                        }
                        ui.close_menu();
                    }
                    if ui.add_enabled(self.history.can_redo(), egui::Button::new("Redo  (Ctrl+Shift+Z)")).clicked() {
                        if let Some(edits) = self.history.redo() {
                            for edit in &edits {
                                let cell = self.core.song.patterns[edit.pattern_idx]
                                    .get_mut(edit.row, edit.channel);
                                *cell = edit.new_cell;
                            }
                            self.core.dirty = true;
                            self.status_message = Some("Redo".to_string());
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(true, egui::Button::new("Copy  (Ctrl+C)")).clicked() {
                        let pattern_idx = self.core.song.order[self.edit_order];
                        let cell = *self.core.song.patterns[pattern_idx]
                            .get(self.cursor_row, self.cursor_channel);
                        self.clipboard = Some(cell);
                        self.status_message = Some("Copied".to_string());
                        ui.close_menu();
                    }
                    if ui.add_enabled(true, egui::Button::new("Cut  (Ctrl+X)")).clicked() {
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
                        ui.close_menu();
                    }
                    if ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste  (Ctrl+V)")).clicked() {
                        if let Some(clip) = self.clipboard {
                            let pattern_idx = self.core.song.order[self.edit_order];
                            let old_cell = *self.core.song.patterns[pattern_idx]
                                .get(self.cursor_row, self.cursor_channel);
                            let cell = self.core.song.patterns[pattern_idx]
                                .get_mut(self.cursor_row, self.cursor_channel);
                            *cell = clip;
                            self.record_cell_edit(pattern_idx, self.cursor_row, self.cursor_channel, old_cell, clip);
                            self.core.dirty = true;
                            self.status_message = Some("Pasted".to_string());
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Song Settings").clicked() {
                        self.show_song_settings = !self.show_song_settings;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    pub fn do_save(&mut self) {
        match self.core.save() {
            Ok(msg) => {
                self.status_message = Some(msg);
            }
            Err(msg) => {
                self.status_message = Some(msg);
            }
        }
    }

    fn reset_cursor_state(&mut self) {
        self.cursor_row = 0;
        self.cursor_channel = 0;
        self.cursor_sub = SubColumn::Note;
        self.edit_order = 0;
    }
}
