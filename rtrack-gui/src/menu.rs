use rtrack_core::tracker::Cell;
use rtrack_core::TrackerCore;

use crate::app::RtrackApp;
use crate::state::SubColumn;

impl RtrackApp {
    pub fn draw_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        let config = rtrack_core::config::load_config();
                        self.core = TrackerCore::with_song_size(8, 64);
                        // Re-init audio on the new core with SF2 from config
                        match rtrack_core::audio::AudioEngine::new(config.sf2.as_deref()) {
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
                                Ok(report) => {
                                    self.reset_cursor_state();
                                    self.history.clear();
                                    self.clipboard = None;
                                    rtrack_core::config::push_recent_file(
                                        &mut self.recent_files,
                                        &path,
                                    );
                                    rtrack_core::config::save_recent_files(&self.recent_files);
                                    self.status_message = Some(Self::describe_load(&report));
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("Load failed: {}", e));
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

                    if ui.button("Load SF2...").clicked() {
                        ui.close_menu();
                        self.draw_load_sf2();
                    }

                    if ui.button("Load Sample Dir...").clicked() {
                        ui.close_menu();
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.status_message =
                                Some(match self.core.load_sample_directory(&dir) {
                                    Ok(count) => {
                                        format!("Loaded {} sample(s) from {}", count, dir.display())
                                    }
                                    Err(e) => format!("Sample directory failed: {}", e),
                                });
                        }
                    }

                    if !self.recent_files.is_empty() {
                        ui.separator();
                        ui.menu_button("Recent Files", |ui| {
                            let files: Vec<_> = self.recent_files.clone();
                            for path in &files {
                                let label =
                                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                                if ui.button(label).clicked() {
                                    match self.core.load_file(path) {
                                        Ok(report) => {
                                            self.reset_cursor_state();
                                            self.history.clear();
                                            self.clipboard = None;
                                            self.status_message =
                                                Some(Self::describe_load(&report));
                                        }
                                        Err(e) => {
                                            self.status_message =
                                                Some(format!("Load failed: {}", e));
                                        }
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    }

                    ui.separator();

                    if ui.button("Export WAV").clicked() {
                        ui.close_menu();
                        self.status_message = Some(match self.core.export_wav_to_default() {
                            Ok(path) => format!("Exported WAV: {}", path.display()),
                            Err(e) => format!("WAV export failed: {}", e),
                        });
                    }
                    if ui.button("Export FLAC").clicked() {
                        ui.close_menu();
                        self.status_message = Some(match self.core.export_flac_to_default() {
                            Ok(path) => format!("Exported FLAC: {}", path.display()),
                            Err(e) => format!("FLAC export failed: {}", e),
                        });
                    }
                    if ui.button("Export MIDI").clicked() {
                        ui.close_menu();
                        self.status_message = Some(match self.core.export_midi_to_default() {
                            Ok(path) => format!("Exported MIDI: {}", path.display()),
                            Err(e) => format!("MIDI export failed: {}", e),
                        });
                    }

                    ui.separator();

                    if ui.button("Quit").clicked() {
                        ui.close_menu();
                        if self.core.dirty {
                            self.show_quit_confirm = true;
                        } else {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.history.can_undo(), egui::Button::new("Undo  (Ctrl+Z)"))
                        .clicked()
                    {
                        self.apply_undo();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.history.can_redo(),
                            egui::Button::new("Redo  (Ctrl+Shift+Z)"),
                        )
                        .clicked()
                    {
                        self.apply_redo();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(true, egui::Button::new("Copy  (Ctrl+C)"))
                        .clicked()
                    {
                        let pattern_idx = self.core.song.order[self.edit_order];
                        let cell = *self.core.song.patterns[pattern_idx]
                            .get(self.cursor_row, self.cursor_channel);
                        self.clipboard = Some(cell);
                        self.status_message = Some("Copied".to_string());
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(true, egui::Button::new("Cut  (Ctrl+X)"))
                        .clicked()
                    {
                        let pattern_idx = self.core.song.order[self.edit_order];
                        let old_cell = *self.core.song.patterns[pattern_idx]
                            .get(self.cursor_row, self.cursor_channel);
                        self.clipboard = Some(old_cell);
                        let cell = self.core.song.patterns[pattern_idx]
                            .get_mut(self.cursor_row, self.cursor_channel);
                        *cell = Cell::default();
                        let new_cell = *cell;
                        self.record_cell_edit(
                            pattern_idx,
                            self.cursor_row,
                            self.cursor_channel,
                            old_cell,
                            new_cell,
                        );
                        self.core.dirty = true;
                        self.status_message = Some("Cut".to_string());
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.clipboard.is_some(),
                            egui::Button::new("Paste  (Ctrl+V)"),
                        )
                        .clicked()
                    {
                        if let Some(clip) = self.clipboard {
                            let pattern_idx = self.core.song.order[self.edit_order];
                            let old_cell = *self.core.song.patterns[pattern_idx]
                                .get(self.cursor_row, self.cursor_channel);
                            let cell = self.core.song.patterns[pattern_idx]
                                .get_mut(self.cursor_row, self.cursor_channel);
                            *cell = clip;
                            self.record_cell_edit(
                                pattern_idx,
                                self.cursor_row,
                                self.cursor_channel,
                                old_cell,
                                clip,
                            );
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

                ui.menu_button("View", |ui| {
                    if ui.button("Instruments  (F7)").clicked() {
                        self.show_instrument_list = !self.show_instrument_list;
                        ui.close_menu();
                    }
                    let matrix_label = if self.show_pattern_matrix {
                        "Close Pattern Matrix  (Ctrl+P)"
                    } else {
                        "Pattern Matrix  (Ctrl+P)"
                    };
                    if ui.button(matrix_label).clicked() {
                        self.show_pattern_matrix = !self.show_pattern_matrix;
                        if self.show_pattern_matrix {
                            self.matrix_cursor = self.edit_order;
                        }
                        ui.close_menu();
                    }
                    if ui.button("Help  (F1)").clicked() {
                        self.show_help = !self.show_help;
                        ui.close_menu();
                    }
                    if ui.button("MIDI Ports  (F2)").clicked() {
                        self.midi_port_list =
                            rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                        self.midi_input_port_list =
                            rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                        self.show_midi_ports = true;
                        ui.close_menu();
                    }
                    let vis_label = if self.show_visualization {
                        "Spectrum  (F4) [on]"
                    } else {
                        "Spectrum  (F4)"
                    };
                    if ui.button(vis_label).clicked() {
                        self.show_visualization = !self.show_visualization;
                        ui.close_menu();
                    }
                    ui.separator();
                    let theme_label = format!("Theme: {} (F8)", self.theme.label());
                    if ui.button(theme_label).clicked() {
                        let new_theme = self.theme.toggle();
                        self.set_theme(ctx, new_theme);
                        ui.close_menu();
                    }
                });
            });
        });
    }

    pub fn do_save(&mut self) {
        match self.core.save() {
            Ok(path) => {
                rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                rtrack_core::config::save_recent_files(&self.recent_files);
                self.status_message = Some(format!("Saved: {}", path.display()));
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
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
