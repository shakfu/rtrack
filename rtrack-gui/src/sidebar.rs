use crate::app::RtrackApp;

impl RtrackApp {
    pub fn draw_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("order_list")
            .default_width(90.0)
            .show(ctx, |ui| {
                self.draw_order_list(ui);
                ui.separator();
                self.draw_channel_list(ui);
            });
    }

    fn draw_order_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Order");
        ui.add_space(4.0);

        let active_order = if self.core.playing {
            self.core.engine.order
        } else {
            self.edit_order
        };

        let order_len = self.core.song.order.len();

        egui::ScrollArea::vertical()
            .id_salt("order_scroll")
            .max_height(ui.available_height() * 0.5)
            .show(ui, |ui| {
                for i in 0..order_len {
                    let pat_idx = self.core.song.order[i];
                    let label = format!("{:02X}: P{:02X}", i, pat_idx);
                    let is_active = i == active_order;

                    let text = if is_active {
                        egui::RichText::new(label)
                            .strong()
                            .color(egui::Color32::from_rgb(255, 200, 60))
                    } else {
                        egui::RichText::new(label).monospace()
                    };

                    if ui
                        .selectable_label(is_active, text)
                        .clicked()
                    {
                        self.edit_order = i;
                        self.cursor_row = 0;
                    }
                }
            });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("+").on_hover_text("New empty pattern").clicked() {
                let new_pat_idx = self.core.song.patterns.len();
                let rows = self.core.song.patterns[0].rows;
                let channels = self.core.song.channels;
                self.core.song.patterns.push(rtrack_core::tracker::Pattern::new(rows, channels));
                self.core.song.order.push(new_pat_idx);
                self.core.song.sync_order_repeats();
            }
            if ui.add_enabled(self.core.song.order.len() > 1, egui::Button::new("-"))
                .on_hover_text("Remove last order entry")
                .clicked()
            {
                self.core.song.order.pop();
                self.core.song.sync_order_repeats();
                if self.edit_order >= self.core.song.order.len() {
                    self.edit_order = self.core.song.order.len() - 1;
                }
            }
            if ui.button("D").on_hover_text("Duplicate current pattern").clicked() {
                let src_pat_idx = self.core.song.order[self.edit_order];
                let cloned = self.core.song.patterns[src_pat_idx].clone();
                let new_pat_idx = self.core.song.patterns.len();
                self.core.song.patterns.push(cloned);
                self.core.song.order.insert(self.edit_order + 1, new_pat_idx);
                self.core.song.sync_order_repeats();
                self.edit_order += 1;
                self.cursor_row = 0;
                self.core.dirty = true;
            }
            if ui.button("^").on_hover_text("Insert order entry at current position").clicked() {
                let pat = self.core.song.order[self.edit_order];
                self.core.song.order.insert(self.edit_order + 1, pat);
                self.core.song.sync_order_repeats();
                self.edit_order += 1;
                self.cursor_row = 0;
                self.core.dirty = true;
            }
        });
    }

    fn draw_channel_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Channels");
        ui.add_space(4.0);

        let solo = self.core.solo_channel;

        // Collect display info first to avoid borrow issues
        let channel_info: Vec<(String, bool)> = self
            .core
            .channels
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let name = if ch.name.is_empty() {
                    format!("Ch{}", i + 1)
                } else {
                    ch.name.clone()
                };
                (name, ch.muted)
            })
            .collect();

        let mut mute_toggle: Option<usize> = None;
        let mut solo_toggle: Option<usize> = None;

        egui::ScrollArea::vertical()
            .id_salt("channel_scroll")
            .show(ui, |ui| {
                for (i, (name, muted)) in channel_info.iter().enumerate() {
                    let is_solo = solo == Some(i);
                    let dimmed = *muted && !is_solo;

                    ui.horizontal(|ui| {
                        let name_color = if dimmed {
                            egui::Color32::from_rgb(100, 100, 100)
                        } else if is_solo {
                            egui::Color32::from_rgb(100, 220, 255)
                        } else {
                            egui::Color32::from_rgb(200, 200, 200)
                        };

                        ui.label(
                            egui::RichText::new(name)
                                .monospace()
                                .color(name_color),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Solo button
                            let s_text = if is_solo {
                                egui::RichText::new("S")
                                    .strong()
                                    .color(egui::Color32::from_rgb(100, 220, 255))
                            } else {
                                egui::RichText::new("S")
                                    .color(egui::Color32::from_rgb(140, 140, 140))
                            };
                            if ui.small_button(s_text).clicked() {
                                solo_toggle = Some(i);
                            }

                            // Mute button
                            let m_text = if *muted {
                                egui::RichText::new("M")
                                    .strong()
                                    .color(egui::Color32::from_rgb(255, 80, 80))
                            } else {
                                egui::RichText::new("M")
                                    .color(egui::Color32::from_rgb(140, 140, 140))
                            };
                            if ui.small_button(m_text).clicked() {
                                mute_toggle = Some(i);
                            }
                        });
                    });
                }
            });

        // Apply toggles after iteration
        if let Some(ch) = mute_toggle {
            if let Some(msg) = self.core.toggle_channel_mute(ch) {
                self.status_message = Some(msg);
            }
        }
        if let Some(ch) = solo_toggle {
            let msg = self.core.toggle_solo(ch);
            self.status_message = Some(msg);
        }
    }
}
