use rtrack_core::theory::{Scale, ScaleSetting};
use rtrack_core::tracker::NoteValue;

use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        self.draw_song_settings(ctx);
        self.draw_quit_confirm(ctx);
        self.draw_discard_confirm(ctx);
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
                        let mut title = self.core.song.title.clone();
                        ui.text_edit_singleline(&mut title);
                        if title != self.core.song.title {
                            let before = self.core.song.clone();
                            self.core.song.title = title;
                            self.record_settings_edit("song.title", before);
                        }
                        ui.end_row();

                        // BPM
                        ui.label("BPM:");
                        let mut bpm = self.core.song.bpm;
                        ui.add(egui::DragValue::new(&mut bpm).range(20..=999));
                        if bpm != self.core.song.bpm {
                            let before = self.core.song.clone();
                            self.core.set_bpm(bpm);
                            self.record_settings_edit("song.bpm", before);
                        }
                        ui.end_row();

                        // Speed
                        ui.label("Speed:");
                        let mut speed = self.core.song.speed;
                        ui.add(egui::DragValue::new(&mut speed).range(1..=31));
                        if speed != self.core.song.speed {
                            let before = self.core.song.clone();
                            self.core.set_speed(speed);
                            self.record_settings_edit("song.speed", before);
                        }
                        ui.end_row();

                        // Highlight Beat
                        ui.label("Beat highlight:");
                        let mut highlight_beat = self.core.song.highlight_beat;
                        ui.add(egui::DragValue::new(&mut highlight_beat).range(1..=64));
                        if highlight_beat != self.core.song.highlight_beat {
                            let before = self.core.song.clone();
                            self.core.song.highlight_beat = highlight_beat;
                            self.record_settings_edit("song.highlight_beat", before);
                        }
                        ui.end_row();

                        // Highlight Bar
                        ui.label("Bar highlight:");
                        let mut highlight_bar = self.core.song.highlight_bar;
                        ui.add(egui::DragValue::new(&mut highlight_bar).range(1..=256));
                        if highlight_bar != self.core.song.highlight_bar {
                            let before = self.core.song.clone();
                            self.core.song.highlight_bar = highlight_bar;
                            self.record_settings_edit("song.highlight_bar", before);
                        }
                        ui.end_row();

                        // Groove: ticks per row, cycling by row
                        ui.label("Groove:");
                        let text = self
                            .groove_text
                            .get_or_insert_with(|| self.core.song.groove_text());
                        let response = ui
                            .add(egui::TextEdit::singleline(text).desired_width(120.0))
                            .on_hover_text(
                                "Ticks per row, cycling: e.g. 7 5 for swing, off for none",
                            );
                        if response.lost_focus() {
                            let typed = self.groove_text.take().unwrap_or_default();
                            match rtrack_core::tracker::Song::parse_groove(&typed) {
                                Ok(groove) if groove != self.core.song.groove => {
                                    let before = self.core.song.clone();
                                    self.core.song.groove = groove;
                                    self.record_settings_edit("song.groove", before);
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    self.status_message = Some(format!("Groove unchanged: {e}"))
                                }
                            }
                        } else if !response.has_focus() {
                            self.groove_text = None;
                        }
                        ui.end_row();

                        // Scale for keyboard note entry
                        ui.label("Scale:");
                        ui.horizontal(|ui| {
                            let current = self.core.song.scale;
                            let mut chosen = current;

                            let root = current.map(|s| s.root).unwrap_or(NoteValue::C);
                            let scale = current.map(|s| s.scale).unwrap_or(Scale::Major);

                            egui::ComboBox::from_id_salt("scale_root")
                                .width(50.0)
                                .selected_text(root.name())
                                .show_ui(ui, |ui| {
                                    for i in 0..12u8 {
                                        let v = NoteValue::from_index(i).unwrap();
                                        if ui.selectable_label(v == root, v.name()).clicked() {
                                            chosen = Some(ScaleSetting::new(v, scale));
                                        }
                                    }
                                });

                            egui::ComboBox::from_id_salt("scale_name")
                                .width(150.0)
                                .selected_text(match current {
                                    Some(s) => s.scale.name(),
                                    None => "off",
                                })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(current.is_none(), "off").clicked() {
                                        chosen = None;
                                    }
                                    for s in Scale::ALL {
                                        let on = current.map(|c| c.scale) == Some(s);
                                        if ui.selectable_label(on, s.name()).clicked() {
                                            chosen = Some(ScaleSetting::new(root, s));
                                        }
                                    }
                                });

                            if chosen != current {
                                let before = self.core.song.clone();
                                self.core.song.scale = chosen;
                                self.record_settings_edit("song.scale", before);
                            }
                        });
                        ui.end_row();

                        // Rows per pattern (current pattern)
                        ui.label("Rows:");
                        let order_pos = if self.core.playing {
                            self.core.playback_position().0
                        } else {
                            self.edit_order
                        };
                        let mut rows = self
                            .core
                            .song
                            .pattern_at(order_pos)
                            .map(|p| p.rows)
                            .unwrap_or(self.core.song.rows_per_pattern);
                        let prev_rows = rows;
                        ui.add(egui::DragValue::new(&mut rows).range(1..=256));
                        if rows != prev_rows {
                            let before = self.core.song.clone();
                            if let Some(pattern) = self.core.song.pattern_at_mut(order_pos) {
                                pattern.resize_rows(rows);
                                self.record_settings_edit("song.rows", before);
                            }
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
                            let before = self.core.song.clone();
                            self.core.set_channel_count(ch_count);
                            if self.cursor_channel >= ch_count {
                                self.cursor_channel = ch_count.saturating_sub(1);
                            }
                            self.record_settings_edit("song.channels", before);
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
                        // Two visits to the same control with the dialog
                        // closed in between are two things the user did.
                        self.break_edit_coalescing();
                    }
                });
            });

        if !open {
            self.show_song_settings = false;
            self.break_edit_coalescing();
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
                    if ui.button("Save & Quit").clicked() && self.do_save() {
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

    /// Ask before New or Open replaces a song with unsaved changes.
    fn draw_discard_confirm(&mut self, ctx: &egui::Context) {
        let Some(change) = self.pending_song_change.clone() else {
            return;
        };
        let verb = match change {
            crate::menu::PendingSongChange::New => "New Song",
            crate::menu::PendingSongChange::Open(_) => "Open",
        };
        egui::Window::new("Unsaved Changes")
            .id(egui::Id::new("discard_confirm"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. What would you like to do?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // A failed save keeps the prompt; the status bar says why.
                    if ui.button(format!("Save & {verb}")).clicked() && self.do_save() {
                        self.pending_song_change = None;
                        self.apply_song_change(change.clone());
                    }
                    if ui.button(format!("{verb} without saving")).clicked() {
                        self.pending_song_change = None;
                        self.apply_song_change(change.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_song_change = None;
                    }
                });
            });
    }

    /// Alias this channel of the edited pattern to another pattern's copy.
    fn draw_alias_controls(&mut self, ui: &mut egui::Ui, ch: usize) {
        use rtrack_core::tracker::ColumnAlias;
        let Some(&pattern) = self.core.song.order.get(self.edit_order) else {
            return;
        };
        let current = self.core.song.patterns[pattern].alias(ch);
        let last = self.core.song.patterns.len().saturating_sub(1);
        let mut source = current.map_or(if pattern == 0 { 1 } else { 0 }, |a| a.source);
        let mut transpose = current.map_or(0, |a| a.transpose);
        let mut on = current.is_some();
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut on, format!("Play another pattern here (pattern {pattern:02X})"))
                .on_hover_text("This channel plays the same channel of the chosen pattern. Edits here change that pattern.")
                .changed();
        });
        if on {
            ui.horizontal(|ui| {
                ui.label("Pattern");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut source)
                            .range(0..=last)
                            .hexadecimal(2, false, true),
                    )
                    .changed();
                ui.label("Transpose");
                changed |= ui
                    .add(egui::DragValue::new(&mut transpose).range(-48..=48))
                    .changed();
            });
        }
        if !changed {
            return;
        }
        let before = self.core.song.clone();
        let result = if on {
            if current.is_some() {
                // Re-point without clearing anything: drop the old alias first.
                self.core.song.patterns[pattern].aliases[ch] = None;
            }
            self.core
                .song
                .set_alias(pattern, ch, ColumnAlias { source, transpose })
        } else {
            self.core.song.unalias(pattern, ch);
            Ok(())
        };
        match result {
            Ok(()) => self.record_settings_edit("alias", before),
            Err(e) => {
                self.core.song = before;
                self.status_message = Some(format!("Alias refused: {e}"));
            }
        }
    }

    /// The channel dialog, with its edits recorded for undo: channel
    /// settings are stored outside the song, so the song snapshot does not
    /// cover them.
    fn draw_track_config(&mut self, ctx: &egui::Context) {
        let channel = self.show_track_config;
        let before = channel.and_then(|ch| self.core.channels.get(ch).cloned());
        self.draw_track_config_body(ctx);
        if let (Some(ch), Some(before)) = (channel, before) {
            self.record_channel_edit(ch, before);
        }
    }

    /// Record a change to channel `ch`'s settings, which were `before`, as an
    /// undo step. Successive changes to one part merge, so a slider drag is
    /// one step. Mute is a performance action and is not recorded.
    pub(crate) fn record_channel_edit(&mut self, ch: usize, before: rtrack_core::ChannelConfig) {
        let Some(after) = self.core.channels.get(ch) else {
            return;
        };
        if !after.differs_for_undo(&before) {
            return;
        }
        let source = rtrack_core::editor::EditSource::new(after.changed_part(&before), ch);
        let edit = rtrack_core::editor::Edit::Channel(Box::new(rtrack_core::editor::ChannelEdit {
            index: ch,
            before,
            after: after.clone(),
        }));
        self.history.push_from(source, edit);
        self.core.dirty = true;
    }

    fn draw_track_config_body(&mut self, ctx: &egui::Context) {
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

                        // Default instrument, for notes with an empty
                        // instrument column.
                        ui.label("Instrument:");
                        ui.horizontal(|ui| {
                            let default = self.core.channels[ch_idx].default_instrument;
                            let mut on = default.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                self.core.channels[ch_idx].default_instrument =
                                    on.then_some(default.unwrap_or(0));
                            }
                            if let Some(inst) = default {
                                let mut v = inst as i32;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut v)
                                            .range(0..=255)
                                            .hexadecimal(2, false, true),
                                    )
                                    .changed()
                                {
                                    self.core.channels[ch_idx].default_instrument = Some(v as u8);
                                }
                                let i = &self.core.instruments[inst as usize];
                                ui.label(format!(
                                    "[{}] {}",
                                    i.kind.tag(),
                                    if i.name.is_empty() {
                                        i.describe(inst as usize)
                                    } else {
                                        i.name.clone()
                                    }
                                ));
                            }
                        });
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

                // Column alias in the pattern being edited
                ui.separator();
                self.draw_alias_controls(ui, ch_idx);

                // Per-channel effects section
                {
                    ui.separator();
                    ui.heading("Effects");
                    ui.add_space(4.0);

                    let fx = &mut self.core.channels[ch_idx].effects_params;

                    // Filter
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.filter_enabled, "Filter");
                        if fx.filter_enabled {
                            ui.indent("filter_params", |ui| {
                                egui::Grid::new("filter_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Cutoff:");
                                        ui.add(
                                            egui::Slider::new(
                                                &mut fx.filter_cutoff,
                                                20.0..=20000.0,
                                            )
                                            .logarithmic(true)
                                            .suffix(" Hz"),
                                        );
                                        ui.end_row();
                                        ui.label("Resonance:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.filter_resonance,
                                            0.0..=1.0,
                                        ));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Distortion
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.distortion_enabled, "Distortion");
                        if fx.distortion_enabled {
                            ui.indent("dist_params", |ui| {
                                egui::Grid::new("dist_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Drive:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.distortion_drive,
                                            1.0..=20.0,
                                        ));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Chorus
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.chorus_enabled, "Chorus");
                        if fx.chorus_enabled {
                            ui.indent("chorus_params", |ui| {
                                egui::Grid::new("chorus_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Rate:");
                                        ui.add(
                                            egui::Slider::new(&mut fx.chorus_rate, 0.1..=10.0)
                                                .suffix(" Hz"),
                                        );
                                        ui.end_row();
                                        ui.label("Depth:");
                                        ui.add(egui::Slider::new(&mut fx.chorus_depth, 0.5..=20.0));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.chorus_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Delay
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.delay_enabled, "Delay");
                        if fx.delay_enabled {
                            ui.indent("delay_params", |ui| {
                                egui::Grid::new("delay_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Time:");
                                        ui.add(
                                            egui::Slider::new(&mut fx.delay_time, 1.0..=2000.0)
                                                .suffix(" ms"),
                                        );
                                        ui.end_row();
                                        ui.label("Feedback:");
                                        ui.add(egui::Slider::new(
                                            &mut fx.delay_feedback,
                                            0.0..=0.95,
                                        ));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.delay_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
                        }
                    });
                    ui.add_space(2.0);

                    // Reverb
                    ui.group(|ui| {
                        ui.checkbox(&mut fx.reverb_enabled, "Reverb");
                        if fx.reverb_enabled {
                            ui.indent("reverb_params", |ui| {
                                egui::Grid::new("reverb_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Size:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_size, 0.0..=1.0));
                                        ui.end_row();
                                        ui.label("Damp:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_damp, 0.0..=1.0));
                                        ui.end_row();
                                        ui.label("Mix:");
                                        ui.add(egui::Slider::new(&mut fx.reverb_mix, 0.0..=1.0));
                                        ui.end_row();
                                    });
                            });
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
                            ("0-9 a-f", "Hex digit (instrument/volume, effect parameter)"),
                            ("Letters", "Effect command name, e.g. VI for VIB"),
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
                                "Each channel has 5 sub-columns: Note | Instrument | Volume | FX1 | FX2\n\
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
