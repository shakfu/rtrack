use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_pattern_matrix(&mut self, ui: &mut egui::Ui) {
        let order_len = self.core.song.order.len();
        let channels = self.core.song.channels;

        ui.horizontal(|ui| {
            ui.heading("Pattern Matrix");
            ui.separator();
            ui.label(
                egui::RichText::new(
                    "Up/Dn:Navigate  Enter:Jump  Ins:Dup  Del:Remove  Left/Right:Pattern  Ctrl+N:New  Ctrl+D:Clone  Esc:Close"
                )
                .color(egui::Color32::from_rgb(120, 120, 140))
                .small(),
            );
        });

        ui.add_space(4.0);

        // Precompute which channels have data per pattern
        let pattern_channel_data: Vec<Vec<bool>> = self
            .core
            .song
            .patterns
            .iter()
            .map(|pat| {
                (0..channels)
                    .map(|ch| (0..pat.rows).any(|r| !pat.get(r, ch).is_empty()))
                    .collect()
            })
            .collect();

        // Channel names for header
        let ch_names: Vec<String> = (0..channels)
            .map(|i| {
                let name = &self.core.channels[i].name;
                if name.is_empty() {
                    format!("Ch{}", i + 1)
                } else {
                    name.chars()
                        .take(rtrack_core::constants::MAX_CHANNEL_NAME)
                        .collect()
                }
            })
            .collect();

        let playback_order = if self.core.playing {
            Some(self.core.playback_position().0)
        } else {
            None
        };

        egui::ScrollArea::vertical()
            .id_salt("pattern_matrix_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Header
                ui.horizontal(|ui| {
                    let header = format!("{:<12}", "Pos Pat Rep");
                    ui.label(
                        egui::RichText::new(header)
                            .monospace()
                            .strong()
                            .color(egui::Color32::from_rgb(180, 180, 200)),
                    );
                    ui.label(
                        egui::RichText::new("|")
                            .monospace()
                            .color(egui::Color32::from_rgb(80, 80, 100)),
                    );
                    for name in &ch_names {
                        ui.label(
                            egui::RichText::new(format!("{:^6}", name))
                                .monospace()
                                .strong()
                                .color(egui::Color32::from_rgb(140, 140, 180)),
                        );
                    }
                });

                ui.separator();

                // Rows
                for ord_idx in 0..order_len {
                    let pat_idx = self.core.song.order.get(ord_idx).copied().unwrap_or(0);
                    let repeat = self
                        .core
                        .song
                        .order_repeats
                        .get(ord_idx)
                        .copied()
                        .unwrap_or(1);

                    let is_cursor = ord_idx == self.matrix_cursor;
                    let is_playing = playback_order == Some(ord_idx);
                    let is_edit = ord_idx == self.edit_order;

                    let rep_str = if repeat == 0 {
                        " -- ".to_string()
                    } else {
                        format!(" x{:<2}", repeat)
                    };
                    let label = format!("{:>2}: [{:02X}]{}", ord_idx, pat_idx, rep_str);

                    let row_color = if is_cursor {
                        egui::Color32::from_rgb(220, 220, 255)
                    } else if is_playing {
                        egui::Color32::from_rgb(255, 200, 60)
                    } else if is_edit {
                        egui::Color32::from_rgb(180, 200, 220)
                    } else {
                        egui::Color32::from_rgb(160, 160, 180)
                    };

                    let bg = if is_cursor {
                        Some(egui::Color32::from_rgb(50, 50, 80))
                    } else {
                        None
                    };

                    ui.horizontal(|ui| {
                        if let Some(bg_color) = bg {
                            let rect = ui.available_rect_before_wrap();
                            ui.painter().rect_filled(rect, 0.0, bg_color);
                        }

                        let resp = ui.selectable_label(
                            is_cursor,
                            egui::RichText::new(&label)
                                .monospace()
                                .strong()
                                .color(row_color),
                        );

                        if resp.clicked() {
                            self.matrix_cursor = ord_idx;
                        }
                        if resp.double_clicked() {
                            self.edit_order = ord_idx;
                            self.cursor_row = 0;
                            self.show_pattern_matrix = false;
                        }

                        ui.label(
                            egui::RichText::new("|")
                                .monospace()
                                .color(egui::Color32::from_rgb(80, 80, 100)),
                        );

                        // Channel data indicators
                        if let Some(ch_data) = pattern_channel_data.get(pat_idx) {
                            for (ch_idx, has_data) in ch_data.iter().enumerate() {
                                let _ = ch_idx;
                                let (text, color) = if *has_data {
                                    ("####", egui::Color32::from_rgb(80, 200, 80))
                                } else {
                                    ("  .  ", egui::Color32::from_rgb(60, 60, 80))
                                };
                                ui.label(
                                    egui::RichText::new(format!("{:^6}", text))
                                        .monospace()
                                        .color(color),
                                );
                            }
                        }
                    });
                }
            });

        // Buttons at bottom
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Ctrl+N: New Pattern").clicked() {
                let idx = self.core.song.add_pattern();
                self.core.song.order.insert(self.matrix_cursor + 1, idx);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
                self.status_message = Some(format!("New pattern {:02X}", idx));
            }
            if ui.button("Ctrl+D: Clone").clicked() {
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
            if ui
                .add_enabled(
                    self.core.song.order.len() > 1,
                    egui::Button::new("Del: Remove"),
                )
                .clicked()
            {
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
            if ui.button("Ins: Duplicate Entry").clicked() {
                let pat = self.core.song.order[self.matrix_cursor];
                self.core.song.order.insert(self.matrix_cursor + 1, pat);
                self.core.song.sync_order_repeats();
                self.matrix_cursor += 1;
                self.core.dirty = true;
            }
            ui.separator();
            if ui.button("Close (Esc)").clicked() {
                self.show_pattern_matrix = false;
            }
        });
    }
}
