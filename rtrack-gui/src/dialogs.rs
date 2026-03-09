use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        self.draw_song_settings(ctx);
        self.draw_quit_confirm(ctx);
        self.draw_track_config(ctx);
        self.draw_instrument_list(ctx);
        self.draw_synth_editor(ctx);
        self.draw_sample_editor(ctx);
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

    fn draw_quit_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_quit_confirm {
            return;
        }
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. What would you like to do?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save & Quit").clicked() {
                        self.do_save();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Quit without saving").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_quit_confirm = false;
                    }
                });
            });
    }

    fn draw_track_config(&mut self, ctx: &egui::Context) {
        let ch_idx = match self.show_track_config {
            Some(idx) => idx,
            None => return,
        };

        let mut open = true;
        egui::Window::new(format!("Track {} Config", ch_idx + 1))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(350.0)
            .show(ctx, |ui| {
                if ch_idx >= self.core.channels.len() {
                    ui.label("Invalid channel");
                    return;
                }

                egui::Grid::new("track_config_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        // Name
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.core.channels[ch_idx].name);
                        ui.end_row();

                        // Muted
                        ui.label("Muted:");
                        ui.checkbox(&mut self.core.channels[ch_idx].muted, "");
                        ui.end_row();

                        // Type
                        ui.label("Type:");
                        let current_type = self.core.channels[ch_idx].channel_type;
                        ui.horizontal(|ui| {
                            if ui.button("<").clicked() {
                                self.core.channels[ch_idx].channel_type = current_type.prev();
                            }
                            ui.label(current_type.label());
                            if ui.button(">").clicked() {
                                self.core.channels[ch_idx].channel_type = current_type.next();
                            }
                        });
                        ui.end_row();

                        // MIDI Channel
                        ui.label("MIDI Ch:");
                        let mut midi_ch = self.core.channels[ch_idx].midi_channel as i32;
                        if ui.add(egui::DragValue::new(&mut midi_ch).range(0..=15)).changed() {
                            self.core.channels[ch_idx].midi_channel = midi_ch as u8;
                        }
                        ui.end_row();

                        // Default Instrument
                        ui.label("Instrument:");
                        let mut inst = self.core.channels[ch_idx].default_instrument.unwrap_or(0) as i32;
                        if ui.add(egui::DragValue::new(&mut inst).range(0..=255)).changed() {
                            self.core.channels[ch_idx].default_instrument = Some(inst as u8);
                        }
                        ui.end_row();

                        // Volume
                        ui.label("Volume:");
                        ui.add(egui::Slider::new(&mut self.core.channels[ch_idx].volume, 0.0..=1.0));
                        ui.end_row();

                        // Pan
                        ui.label("Pan:");
                        ui.add(egui::Slider::new(&mut self.core.channels[ch_idx].pan, -1.0..=1.0));
                        ui.end_row();
                    });

                // Per-channel effects section (only for Synth/Sample types)
                let ch_type = self.core.channels[ch_idx].channel_type;
                if ch_type != rtrack_core::ChannelType::Midi {
                    ui.separator();
                    ui.heading("Effects");

                    egui::Grid::new("track_effects_grid")
                        .num_columns(2)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            let fx = &mut self.core.channels[ch_idx].effects_params;

                            // Filter
                            ui.checkbox(&mut fx.filter_enabled, "Filter");
                            ui.end_row();
                            if fx.filter_enabled {
                                ui.label("  Cutoff:");
                                ui.add(egui::Slider::new(&mut fx.filter_cutoff, 20.0..=20000.0).logarithmic(true).suffix(" Hz"));
                                ui.end_row();
                                ui.label("  Resonance:");
                                ui.add(egui::Slider::new(&mut fx.filter_resonance, 0.0..=1.0));
                                ui.end_row();
                            }

                            // Distortion
                            ui.checkbox(&mut fx.distortion_enabled, "Distortion");
                            ui.end_row();
                            if fx.distortion_enabled {
                                ui.label("  Drive:");
                                ui.add(egui::Slider::new(&mut fx.distortion_drive, 1.0..=20.0));
                                ui.end_row();
                            }

                            // Chorus
                            ui.checkbox(&mut fx.chorus_enabled, "Chorus");
                            ui.end_row();
                            if fx.chorus_enabled {
                                ui.label("  Rate:");
                                ui.add(egui::Slider::new(&mut fx.chorus_rate, 0.1..=10.0).suffix(" Hz"));
                                ui.end_row();
                                ui.label("  Depth:");
                                ui.add(egui::Slider::new(&mut fx.chorus_depth, 0.5..=20.0));
                                ui.end_row();
                                ui.label("  Mix:");
                                ui.add(egui::Slider::new(&mut fx.chorus_mix, 0.0..=1.0));
                                ui.end_row();
                            }

                            // Delay
                            ui.checkbox(&mut fx.delay_enabled, "Delay");
                            ui.end_row();
                            if fx.delay_enabled {
                                ui.label("  Time:");
                                ui.add(egui::Slider::new(&mut fx.delay_time, 1.0..=2000.0).suffix(" ms"));
                                ui.end_row();
                                ui.label("  Feedback:");
                                ui.add(egui::Slider::new(&mut fx.delay_feedback, 0.0..=0.95));
                                ui.end_row();
                                ui.label("  Mix:");
                                ui.add(egui::Slider::new(&mut fx.delay_mix, 0.0..=1.0));
                                ui.end_row();
                            }

                            // Reverb
                            ui.checkbox(&mut fx.reverb_enabled, "Reverb");
                            ui.end_row();
                            if fx.reverb_enabled {
                                ui.label("  Size:");
                                ui.add(egui::Slider::new(&mut fx.reverb_size, 0.0..=1.0));
                                ui.end_row();
                                ui.label("  Damp:");
                                ui.add(egui::Slider::new(&mut fx.reverb_damp, 0.0..=1.0));
                                ui.end_row();
                                ui.label("  Mix:");
                                ui.add(egui::Slider::new(&mut fx.reverb_mix, 0.0..=1.0));
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    self.show_track_config = None;
                }
            });

        if !open {
            self.show_track_config = None;
        }
    }
}
