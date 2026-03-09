use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        self.draw_song_settings(ctx);
    }

    fn draw_song_settings(&mut self, ctx: &egui::Context) {
        if !self.show_song_settings {
            return;
        }

        let mut open = true;
        egui::Window::new("Song Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                egui::Grid::new("song_settings_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        // Title
                        ui.label("Title:");
                        ui.text_edit_singleline(&mut self.core.song.title);
                        ui.end_row();

                        // BPM
                        ui.label("BPM:");
                        let prev_bpm = self.core.song.bpm;
                        ui.add(egui::DragValue::new(&mut self.core.song.bpm).range(20..=999));
                        if self.core.song.bpm != prev_bpm {
                            self.core.link.set_tempo(self.core.song.bpm as f64);
                        }
                        ui.end_row();

                        // Speed
                        ui.label("Speed:");
                        ui.add(egui::DragValue::new(&mut self.core.song.speed).range(1..=31));
                        ui.end_row();

                        // Highlight Beat
                        ui.label("Beat highlight:");
                        ui.add(egui::DragValue::new(&mut self.core.song.highlight_beat).range(1..=64));
                        ui.end_row();

                        // Highlight Bar
                        ui.label("Bar highlight:");
                        ui.add(egui::DragValue::new(&mut self.core.song.highlight_bar).range(1..=256));
                        ui.end_row();

                        // Swing
                        ui.label("Swing:");
                        ui.add(egui::DragValue::new(&mut self.core.song.swing).range(0..=100).suffix("%"));
                        ui.end_row();

                        ui.separator();
                        ui.separator();
                        ui.end_row();

                        // Channels (read-only)
                        ui.label("Channels:");
                        ui.label(format!("{}", self.core.song.channels));
                        ui.end_row();

                        // Pattern count (read-only)
                        ui.label("Patterns:");
                        ui.label(format!("{}", self.core.song.patterns.len()));
                        ui.end_row();

                        // Order length (read-only)
                        ui.label("Order length:");
                        ui.label(format!("{}", self.core.song.order.len()));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        self.show_song_settings = false;
                    }
                });
            });

        if !open {
            self.show_song_settings = false;
        }
    }
}
