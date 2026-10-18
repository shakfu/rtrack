use rtrack_core::constants::MAX_EXPORT_SECONDS;
use rtrack_core::engine::ExportLength;
use rtrack_core::tracker::Cell;
use rtrack_core::TrackerCore;

use crate::app::RtrackApp;
use crate::state::SubColumn;

/// A replacement of the song waiting on the unsaved-changes prompt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PendingSongChange {
    New,
    Open(std::path::PathBuf),
}

/// What an export writes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExportFormat {
    Wav,
    Flac,
    Midi,
}

impl RtrackApp {
    pub fn draw_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        ui.close();
                        self.request_song_change(PendingSongChange::New);
                    }

                    if ui.button("Open...").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("rtrack", &["rtrk"])
                            .pick_file()
                        {
                            self.request_song_change(PendingSongChange::Open(path));
                        }
                    }

                    if ui.button("Save  (Ctrl+S)").clicked() {
                        ui.close();
                        self.do_save();
                    }

                    if ui.button("Save As...").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("rtrack", &["rtrk"])
                            .save_file()
                        {
                            self.core.file_path = Some(path);
                            self.do_save();
                        }
                    }

                    if ui.button("Load SF2...").clicked() {
                        ui.close();
                        self.draw_load_sf2();
                    }

                    if ui.button("Load Sample Dir...").clicked() {
                        ui.close();
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.load_sample_directory_undoable(&dir);
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
                                    ui.close();
                                    self.request_song_change(PendingSongChange::Open(path.clone()));
                                }
                            }
                        });
                    }

                    ui.separator();

                    self.draw_export_length(ui);
                    if ui.button("Export WAV").clicked() {
                        ui.close();
                        self.export(ExportFormat::Wav);
                    }
                    if ui.button("Export FLAC").clicked() {
                        ui.close();
                        self.export(ExportFormat::Flac);
                    }
                    if ui.button("Export MIDI").clicked() {
                        ui.close();
                        self.export(ExportFormat::Midi);
                    }

                    ui.separator();

                    if ui.button("Quit").clicked() {
                        ui.close();
                        if self.core.dirty {
                            self.show_quit_confirm = true;
                        } else {
                            ui.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.history.can_undo(), egui::Button::new("Undo  (Ctrl+Z)"))
                        .clicked()
                    {
                        self.apply_undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.history.can_redo(),
                            egui::Button::new("Redo  (Ctrl+Shift+Z)"),
                        )
                        .clicked()
                    {
                        self.apply_redo();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(true, egui::Button::new("Copy  (Ctrl+C)"))
                        .clicked()
                    {
                        let pattern_idx = self.core.song.order[self.edit_order];
                        let cell =
                            self.core
                                .song
                                .cell(pattern_idx, self.cursor_row, self.cursor_channel);
                        self.clipboard.set_cell(cell);
                        self.status_message = Some("Copied".to_string());
                        ui.close();
                    }
                    if ui
                        .add_enabled(true, egui::Button::new("Cut  (Ctrl+X)"))
                        .clicked()
                    {
                        let pattern_idx = self.core.song.order[self.edit_order];
                        let (row, ch) = (self.cursor_row, self.cursor_channel);
                        self.clipboard
                            .set_cell(self.core.song.cell(pattern_idx, row, ch));
                        if let Some(e) =
                            self.edit_view(pattern_idx, row, ch, |c| *c = Cell::default())
                        {
                            self.history.push(vec![e]);
                        }
                        self.core.dirty = true;
                        self.status_message = Some("Cut".to_string());
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.clipboard.cell().is_some(),
                            egui::Button::new("Paste  (Ctrl+V)"),
                        )
                        .clicked()
                    {
                        if let Some(clip) = self.clipboard.cell() {
                            let pattern_idx = self.core.song.order[self.edit_order];
                            let (row, ch) = (self.cursor_row, self.cursor_channel);
                            if let Some(e) = self.edit_view(pattern_idx, row, ch, |c| *c = clip) {
                                self.history.push(vec![e]);
                            }
                            self.core.dirty = true;
                            self.status_message = Some("Pasted".to_string());
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Song Settings").clicked() {
                        self.show_song_settings = !self.show_song_settings;
                        ui.close();
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.button("Instruments  (F7)").clicked() {
                        self.show_instrument_list = !self.show_instrument_list;
                        ui.close();
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
                        ui.close();
                    }
                    if ui.button("Help  (F1)").clicked() {
                        self.show_help = !self.show_help;
                        ui.close();
                    }
                    if ui.button("MIDI Ports  (F2)").clicked() {
                        self.midi_port_list =
                            rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                        self.midi_input_port_list =
                            rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                        self.show_midi_ports = true;
                        ui.close();
                    }
                    let vis_label = if self.show_visualization {
                        "Spectrum  (F4) [on]"
                    } else {
                        "Spectrum  (F4)"
                    };
                    if ui.button(vis_label).clicked() {
                        self.show_visualization = !self.show_visualization;
                        ui.close();
                    }
                    ui.separator();
                    let theme_label = format!("Theme: {} (F8)", self.theme.label());
                    if ui.button(theme_label).clicked() {
                        let new_theme = self.theme.toggle();
                        self.set_theme(ui.ctx(), new_theme);
                        ui.close();
                    }
                });
            });
        });
    }

    /// Save the song. Returns whether it was written.
    pub fn do_save(&mut self) -> bool {
        match self.core.save() {
            Ok(path) => {
                rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                rtrack_core::config::save_recent_files(&self.recent_files);
                self.status_message = Some(format!("Saved: {}", path.display()));
                true
            }
            Err(e) => {
                self.status_message = Some(format!("Save failed: {}", e));
                false
            }
        }
    }

    /// Replace the song, asking first if the current one has unsaved
    /// changes. Every path that replaces the song comes through here.
    pub(crate) fn request_song_change(&mut self, change: PendingSongChange) {
        if self.core.dirty {
            self.pending_song_change = Some(change);
        } else {
            self.apply_song_change(change);
        }
    }

    /// Replace the song, dropping any unsaved changes.
    pub(crate) fn apply_song_change(&mut self, change: PendingSongChange) {
        match change {
            PendingSongChange::New => {
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
                self.status_message = Some("New song created".to_string());
            }
            PendingSongChange::Open(path) => match self.core.load_file(&path) {
                Ok(report) => {
                    rtrack_core::config::push_recent_file(&mut self.recent_files, &path);
                    rtrack_core::config::save_recent_files(&self.recent_files);
                    self.status_message = Some(Self::describe_load(&report));
                }
                Err(e) => {
                    self.status_message = Some(format!("Load failed: {}", e));
                    return;
                }
            },
        }
        self.reset_cursor_state();
        self.first_visible_channel = 0;
        self.history.clear();
        self.clipboard.clear();
    }

    /// Loops or seconds for the next export.
    fn draw_export_length(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Export length:");
            let loops = matches!(self.export_length, ExportLength::Loops(_));
            if ui.selectable_label(loops, "Loops").clicked() && !loops {
                self.export_length = ExportLength::Loops(1);
            }
            if ui.selectable_label(!loops, "Seconds").clicked() && loops {
                self.export_length = ExportLength::Seconds(60.0);
            }
            match &mut self.export_length {
                ExportLength::Loops(n) => {
                    ui.add(egui::DragValue::new(n).range(1..=99));
                }
                ExportLength::Seconds(s) => {
                    ui.add(
                        egui::DragValue::new(s)
                            .range(1.0..=MAX_EXPORT_SECONDS)
                            .suffix(" s"),
                    );
                }
            }
        });
    }

    /// Export the song beside its file, at the length set in the File menu.
    pub(crate) fn export(&mut self, format: ExportFormat) {
        let length = self.export_length;
        let (name, result) = match format {
            ExportFormat::Wav => ("WAV", self.core.export_wav_to_default(length)),
            ExportFormat::Flac => ("FLAC", self.core.export_flac_to_default(length)),
            ExportFormat::Midi => ("MIDI", self.core.export_midi_to_default(length)),
        };
        self.status_message = Some(match result {
            Ok(path) => format!("Exported {name} ({length}): {}", path.display()),
            Err(e) => format!("{name} export failed: {e}"),
        });
    }

    fn reset_cursor_state(&mut self) {
        self.cursor_row = 0;
        self.cursor_channel = 0;
        self.cursor_sub = SubColumn::Note;
        self.edit_order = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RtrackApp;

    #[test]
    fn saving_writes_the_file_and_clears_the_dirty_flag() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");

        let mut app = RtrackApp::headless(2, 16);
        app.core.file_path = Some(path.clone());
        app.core.dirty = true;

        app.do_save();

        assert!(path.exists(), "the song file was not written");
        assert!(!app.core.dirty, "a saved song is not dirty");
        let msg = app.status_message.as_deref().expect("a status message");
        assert!(msg.starts_with("Saved:"), "{msg}");
    }

    #[test]
    fn saving_records_the_song_in_the_recent_files_list() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");

        let mut app = RtrackApp::headless(2, 16);
        // The test config directory is shared across this binary's tests, and
        // the app loads the recent list from it at construction -- so start
        // from a known state rather than whatever another test saved.
        app.recent_files.clear();
        app.core.file_path = Some(path.clone());
        app.do_save();

        let canonical = std::fs::canonicalize(&path).expect("canonicalize");
        assert_eq!(
            app.recent_files.first(),
            Some(&canonical),
            "the song just saved should be the most recent entry"
        );
    }

    #[test]
    fn saving_the_same_song_twice_does_not_list_it_twice() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");

        let mut app = RtrackApp::headless(2, 16);
        app.recent_files.clear();
        app.core.file_path = Some(path);
        app.do_save();
        app.do_save();

        assert_eq!(app.recent_files.len(), 1, "{:?}", app.recent_files);
    }

    #[test]
    fn a_save_that_cannot_be_written_reports_rather_than_panicking() {
        let dir = tempfile::tempdir().expect("temp dir");
        // A path whose parent does not exist.
        let path = dir.path().join("no_such_dir").join("song.rtrk");

        let mut app = RtrackApp::headless(2, 16);
        app.core.file_path = Some(path);
        app.core.dirty = true;

        app.do_save();

        let msg = app.status_message.as_deref().expect("a status message");
        assert!(msg.starts_with("Save failed:"), "{msg}");
        assert!(
            app.core.dirty,
            "a song that failed to save is still unsaved"
        );
    }

    #[test]
    fn resetting_the_cursor_puts_it_back_at_the_first_cell() {
        let mut app = RtrackApp::headless(4, 16);
        app.cursor_row = 11;
        app.cursor_channel = 3;
        app.cursor_sub = SubColumn::Fx2;
        app.edit_order = 2;

        app.reset_cursor_state();

        assert_eq!(app.cursor_row, 0);
        assert_eq!(app.cursor_channel, 0);
        assert_eq!(app.cursor_sub, SubColumn::Note);
        assert_eq!(app.edit_order, 0);
    }
}
