use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        self.draw_song_settings(ctx);
        self.draw_quit_confirm(ctx);
        self.draw_track_config(ctx);
        self.draw_help(ctx);
        self.draw_midi_ports(ctx);
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
                        ui.add(
                            egui::DragValue::new(&mut self.core.song.highlight_beat).range(1..=64),
                        );
                        ui.end_row();

                        // Highlight Bar
                        ui.label("Bar highlight:");
                        ui.add(
                            egui::DragValue::new(&mut self.core.song.highlight_bar).range(1..=256),
                        );
                        ui.end_row();

                        // Swing
                        ui.label("Swing:");
                        ui.add(
                            egui::DragValue::new(&mut self.core.song.swing)
                                .range(0..=100)
                                .suffix("%"),
                        );
                        ui.end_row();

                        // Rows per pattern (current pattern)
                        ui.label("Rows:");
                        let order_pos = if self.core.playing {
                            self.core.engine.order
                        } else {
                            self.edit_order
                        };
                        let pat_idx = self.core.song.order[order_pos];
                        let mut rows = self.core.song.patterns[pat_idx].rows;
                        let prev_rows = rows;
                        ui.add(egui::DragValue::new(&mut rows).range(1..=256));
                        if rows != prev_rows {
                            self.core.song.patterns[pat_idx].resize_rows(rows);
                            self.core.dirty = true;
                        }
                        ui.end_row();

                        ui.separator();
                        ui.separator();
                        ui.end_row();

                        // Channels
                        ui.label("Channels:");
                        let mut ch_count = self.core.song.channels;
                        let prev_ch_count = ch_count;
                        ui.add(
                            egui::DragValue::new(&mut ch_count)
                                .range(1..=rtrack_core::constants::MAX_CHANNELS),
                        );
                        if ch_count != prev_ch_count {
                            self.core.song.channels = ch_count;
                            for pat in &mut self.core.song.patterns {
                                for row in &mut pat.data {
                                    row.resize(ch_count, rtrack_core::tracker::Cell::default());
                                }
                                pat.channels = ch_count;
                            }
                            while self.core.channels.len() < ch_count {
                                let idx = self.core.channels.len();
                                self.core
                                    .channels
                                    .push(rtrack_core::ChannelConfig::new(idx as u8));
                            }
                            self.core.channels.truncate(ch_count);
                            if self.cursor_channel >= ch_count {
                                self.cursor_channel = ch_count.saturating_sub(1);
                            }
                            self.core.dirty = true;
                        }
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
                        self.core.dirty = false;
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
                        if ui
                            .add(egui::DragValue::new(&mut midi_ch).range(0..=15))
                            .changed()
                        {
                            self.core.channels[ch_idx].midi_channel = midi_ch as u8;
                        }
                        ui.end_row();

                        // Default Instrument
                        ui.label("Instrument:");
                        let mut inst =
                            self.core.channels[ch_idx].default_instrument.unwrap_or(0) as i32;
                        if ui
                            .add(egui::DragValue::new(&mut inst).range(0..=255))
                            .changed()
                        {
                            self.core.channels[ch_idx].default_instrument = Some(inst as u8);
                        }
                        ui.end_row();

                        // Volume
                        ui.label("Volume:");
                        ui.add(egui::Slider::new(
                            &mut self.core.channels[ch_idx].volume,
                            0.0..=1.0,
                        ));
                        ui.end_row();

                        // Pan
                        ui.label("Pan:");
                        ui.add(egui::Slider::new(
                            &mut self.core.channels[ch_idx].pan,
                            -1.0..=1.0,
                        ));
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
                                ui.add(
                                    egui::Slider::new(&mut fx.filter_cutoff, 20.0..=20000.0)
                                        .logarithmic(true)
                                        .suffix(" Hz"),
                                );
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
                                ui.add(
                                    egui::Slider::new(&mut fx.chorus_rate, 0.1..=10.0)
                                        .suffix(" Hz"),
                                );
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
                                ui.add(
                                    egui::Slider::new(&mut fx.delay_time, 1.0..=2000.0)
                                        .suffix(" ms"),
                                );
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

                    // MIDI Learn section
                    ui.separator();
                    ui.heading("MIDI Learn");

                    // Show learning status
                    if let Some((learn_ch, learn_param)) = self.core.midi_learn_pending {
                        if learn_ch == ch_idx {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Waiting for CC... ({})",
                                    learn_param.name()
                                ))
                                .color(egui::Color32::from_rgb(255, 200, 60)),
                            );
                            if ui.button("Cancel").clicked() {
                                self.core.midi_learn_pending = None;
                            }
                        }
                    }

                    // Learn buttons for each parameter
                    use rtrack_core::LearnableParam;
                    let params = [
                        LearnableParam::FilterCutoff,
                        LearnableParam::FilterResonance,
                        LearnableParam::DistortionDrive,
                        LearnableParam::ChorusRate,
                        LearnableParam::ChorusDepth,
                        LearnableParam::ChorusMix,
                        LearnableParam::DelayTime,
                        LearnableParam::DelayFeedback,
                        LearnableParam::DelayMix,
                        LearnableParam::ReverbSize,
                        LearnableParam::ReverbDamp,
                        LearnableParam::ReverbMix,
                    ];

                    egui::Grid::new("midi_learn_grid")
                        .num_columns(3)
                        .spacing([8.0, 3.0])
                        .show(ui, |ui| {
                            for param in &params {
                                let mapping = self
                                    .core
                                    .midi_cc_mappings
                                    .iter()
                                    .find(|m| m.channel == ch_idx && m.param == *param);

                                ui.label(param.name());
                                if let Some(m) = mapping {
                                    ui.label(format!("CC{}", m.cc));
                                    if ui.small_button("Unlearn").clicked() {
                                        let p = *param;
                                        self.core
                                            .midi_cc_mappings
                                            .retain(|m| !(m.channel == ch_idx && m.param == p));
                                        self.status_message = Some(format!(
                                            "Unlearned {} (ch {})",
                                            p.name(),
                                            ch_idx + 1
                                        ));
                                    }
                                } else {
                                    ui.label("---");
                                    if ui.small_button("Learn").clicked() {
                                        self.core.midi_learn_pending = Some((ch_idx, *param));
                                    }
                                }
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

    fn draw_help(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }

        let mut open = true;
        egui::Window::new("Help")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let dim = egui::Color32::from_rgb(140, 140, 160);

                        ui.heading("General");
                        help_grid(ui, "help_general", &[
                            ("Space", "Play / Stop"),
                            ("Ctrl+Space", "Play from start"),
                            ("i", "Enter Insert mode"),
                            ("Escape", "Return to Normal mode / Close dialog"),
                            ("F1", "Toggle this help"),
                            ("F7", "Instrument editor"),
                            ("Ctrl+P", "Pattern matrix"),
                            ("Ctrl+S", "Save"),
                            ("Ctrl+Z", "Undo"),
                            ("Ctrl+Shift+Z", "Redo"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Navigation");
                        help_grid(ui, "help_nav", &[
                            ("Up / Down", "Move cursor"),
                            ("Left / Right", "Move sub-column"),
                            ("Tab / Shift+Tab", "Next / previous channel"),
                            ("Page Up / Down", "Move 16 rows"),
                            ("Home / End", "First / last row"),
                            ("Ctrl+Left / Right", "Previous / next order position"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Editing (Insert Mode)");
                        help_grid(ui, "help_edit", &[
                            ("z s x d c v g b h n j m", "Notes C C# D D# E F F# G G# A A# B (lower octave)"),
                            ("q 2 w 3 e r 5 t 6 y 7 u", "Notes C C# D D# E F F# G G# A A# B (upper octave)"),
                            ("= (equals)", "Note off"),
                            ("0-9 a-f", "Hex digit (instrument/volume/effect columns)"),
                            ("Delete", "Clear cell"),
                            ("Insert", "Insert row"),
                            ("Backspace", "Delete row"),
                            ("+  / -", "Octave up / down"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Block Operations");
                        help_grid(ui, "help_block", &[
                            ("Ctrl+B", "Start/toggle block selection"),
                            ("Ctrl+C", "Copy (cell or block)"),
                            ("Ctrl+X", "Cut (cell or block)"),
                            ("Ctrl+V", "Paste (cell or block)"),
                            ("Ctrl+I", "Interpolate (volume/effect values in block)"),
                            ("Shift+Up / Down", "Transpose note up/down (cell or block)"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Pattern / Song");
                        help_grid(ui, "help_pattern", &[
                            ("Ctrl+N", "New pattern (insert after current)"),
                            ("Ctrl+D", "Clone pattern (insert after current)"),
                            ("Ctrl+F", "Toggle follow mode"),
                            ("Ctrl+R", "Toggle recording"),
                            ("Enter", "Open track config (Normal mode)"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Export");
                        help_grid(ui, "help_export", &[
                            ("Ctrl+E", "Export MIDI"),
                            ("Ctrl+W", "Export WAV"),
                            ("Ctrl+L", "Export FLAC"),
                            ("Ctrl+M", "Toggle MIDI clock output"),
                        ], dim);

                        ui.add_space(8.0);
                        ui.heading("Sub-columns");
                        ui.label(
                            egui::RichText::new(
                                "Each channel has 4 sub-columns: Note | Instrument | Volume | Effect\n\
                                 Use Left/Right arrows to move between them."
                            ).color(dim),
                        );
                    });

                ui.add_space(4.0);
                if ui.button("Close").clicked() {
                    self.show_help = false;
                }
            });

        if !open {
            self.show_help = false;
        }
    }

    fn draw_midi_ports(&mut self, ctx: &egui::Context) {
        if !self.show_midi_ports {
            return;
        }

        let mut open = true;
        egui::Window::new("MIDI Ports")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .show(ctx, |ui| {
                // Output ports
                ui.heading("Output");
                if self.midi_port_list.is_empty() {
                    ui.label("No MIDI output ports found.");
                } else {
                    let current = self.core.midi_port_display_name().to_string();
                    for (i, port_name) in self.midi_port_list.iter().enumerate() {
                        let selected = *port_name == current;
                        let label = if selected {
                            format!("> {}", port_name)
                        } else {
                            port_name.clone()
                        };
                        if ui.selectable_label(selected, &label).clicked() {
                            match self.core.midi.connect(i) {
                                Ok(()) => {
                                    self.status_message =
                                        Some(format!("Output: connected to {}", port_name));
                                }
                                Err(e) => {
                                    self.status_message =
                                        Some(format!("MIDI connect error: {}", e));
                                }
                            }
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Virtual Out").clicked() {
                        match self.core.midi.create_virtual_port() {
                            Ok(()) => {
                                self.status_message =
                                    Some("Created virtual MIDI output port".to_string());
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Virtual port error: {}", e));
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // Input ports
                ui.heading("Input");
                let input_port_name = self.core.midi_input.port_name.clone().unwrap_or_default();
                if self.midi_input_port_list.is_empty() {
                    ui.label("No MIDI input ports found.");
                } else {
                    for (i, port_name) in self.midi_input_port_list.iter().enumerate() {
                        let selected = *port_name == input_port_name;
                        let label = if selected {
                            format!("> {}", port_name)
                        } else {
                            port_name.clone()
                        };
                        if ui.selectable_label(selected, &label).clicked() {
                            match self.core.midi_input.connect(i) {
                                Ok(()) => {
                                    self.status_message =
                                        Some(format!("Input: connected to {}", port_name));
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("MIDI input error: {}", e));
                                }
                            }
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Virtual In").clicked() {
                        match self.core.midi_input.create_virtual_port() {
                            Ok(()) => {
                                self.status_message =
                                    Some("Created virtual MIDI input port".to_string());
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Virtual input error: {}", e));
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // Clock mode
                ui.heading("Clock");
                {
                    use rtrack_core::ClockMode;
                    let is_external = self.core.clock_mode == ClockMode::ExternalMidi;
                    ui.horizontal(|ui| {
                        ui.label("Source:");
                        if ui.selectable_label(!is_external, "Internal").clicked() && is_external {
                            self.core.toggle_clock_mode();
                            self.status_message = Some("Clock: Internal".to_string());
                        }
                        if ui.selectable_label(is_external, "External MIDI").clicked()
                            && !is_external
                        {
                            self.core.toggle_clock_mode();
                            self.status_message = Some("Clock: External MIDI".to_string());
                        }
                    });

                    // MIDI clock output toggle
                    let mut clock_out = self.core.midi.clock_enabled;
                    if ui.checkbox(&mut clock_out, "Send MIDI clock").changed() {
                        let msg = self.core.toggle_midi_clock();
                        self.status_message = Some(msg);
                    }
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.midi_port_list =
                            rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                        self.midi_input_port_list =
                            rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                    }
                    if ui.button("Close").clicked() {
                        self.show_midi_ports = false;
                    }
                });
            });

        if !open {
            self.show_midi_ports = false;
        }
    }

    pub fn draw_load_sf2(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SoundFont", &["sf2", "SF2"])
            .pick_file()
        {
            match rtrack_core::audio::AudioEngine::new(Some(&path)) {
                Ok(engine) => {
                    self.core.audio = Some(engine);
                    self.status_message = Some(format!(
                        "Loaded SF2: {}",
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                    ));
                }
                Err(e) => {
                    self.status_message = Some(format!("Failed to load SF2: {}", e));
                }
            }
        }
    }
}

fn help_grid(ui: &mut egui::Ui, id: &str, items: &[(&str, &str)], dim: egui::Color32) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 3.0])
        .show(ui, |ui| {
            for (key, desc) in items {
                ui.label(egui::RichText::new(*key).monospace().strong());
                ui.label(egui::RichText::new(*desc).color(dim));
                ui.end_row();
            }
        });
}
