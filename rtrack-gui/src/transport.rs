use egui::{Color32, DragValue, RichText, Ui};

use crate::app::RtrackApp;
use crate::state::Mode;

const PLAY_COLOR: Color32 = Color32::from_rgb(100, 255, 100);
const STOP_COLOR: Color32 = Color32::from_rgb(200, 200, 200);
const RECORD_COLOR: Color32 = Color32::from_rgb(255, 80, 80);
const MODE_NORMAL_COLOR: Color32 = Color32::from_rgb(100, 180, 255);
const MODE_INSERT_COLOR: Color32 = Color32::from_rgb(255, 100, 100);

impl RtrackApp {
    pub fn draw_transport(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // Title
            let title = if self.core.song.title.is_empty() {
                "untitled"
            } else {
                &self.core.song.title
            };
            let dirty = if self.core.dirty { " [*]" } else { "" };
            ui.label(
                RichText::new(format!("{}{}", title, dirty))
                    .strong()
                    .size(14.0),
            );

            ui.separator();

            // Play/Stop button -- larger and more prominent
            let play_btn = egui::Button::new(
                RichText::new(if self.core.playing {
                    "[ Stop ]"
                } else {
                    "[ Play ]"
                })
                .size(15.0)
                .strong()
                .color(if self.core.playing {
                    STOP_COLOR
                } else {
                    PLAY_COLOR
                }),
            );
            if ui.add(play_btn).clicked() {
                self.core
                    .toggle_playback(self.edit_order, self.cursor_row);
            }

            // Record toggle
            let rec_text = if self.core.recording {
                RichText::new("REC").color(RECORD_COLOR).strong()
            } else {
                RichText::new("REC").color(Color32::GRAY)
            };
            if ui.button(rec_text).clicked() {
                self.core.recording = !self.core.recording;
            }

            ui.separator();

            // BPM -- interactive drag value
            let prev_bpm = self.core.song.bpm;
            ui.add(
                DragValue::new(&mut self.core.song.bpm)
                    .range(20..=999)
                    .suffix(" BPM"),
            );
            if self.core.song.bpm != prev_bpm {
                self.core.link.set_tempo(self.core.song.bpm as f64);
            }

            // Speed -- interactive drag value
            ui.add(
                DragValue::new(&mut self.core.song.speed)
                    .range(1..=31)
                    .prefix("Spd:"),
            );

            ui.separator();

            // Position
            ui.label(
                RichText::new(format!(
                    "Ord:{:02X} Row:{:02X}",
                    if self.core.playing {
                        self.core.engine.order
                    } else {
                        self.edit_order
                    },
                    if self.core.playing {
                        self.core.engine.row
                    } else {
                        self.cursor_row
                    }
                ))
                .monospace()
                .size(13.0),
            );

            // Pattern info
            let order_pos = if self.core.playing {
                self.core.engine.order
            } else {
                self.edit_order
            };
            if order_pos < self.core.song.order.len() {
                let pat_idx = self.core.song.order[order_pos];
                let row_count = if pat_idx < self.core.song.patterns.len() {
                    self.core.song.patterns[pat_idx].rows
                } else {
                    0
                };
                ui.label(
                    RichText::new(format!("Pat:{:02X} Len:{}", pat_idx, row_count))
                        .monospace()
                        .size(13.0)
                        .color(Color32::from_rgb(150, 150, 170)),
                );
            }

            ui.separator();

            // Octave -- interactive drag value
            let mut oct = self.current_octave as i32;
            ui.add(DragValue::new(&mut oct).range(0..=9).prefix("Oct:"));
            self.current_octave = oct as u8;

            // Edit step -- interactive drag value
            let mut step = self.edit_step as i32;
            ui.add(DragValue::new(&mut step).range(0..=16).prefix("Step:"));
            self.edit_step = step as usize;

            // Follow mode toggle
            ui.checkbox(&mut self.follow_playback, "Follow");

            ui.separator();

            // Mode
            let (mode_text, mode_color) = match self.mode {
                Mode::Normal => ("NORMAL", MODE_NORMAL_COLOR),
                Mode::Insert => ("INSERT", MODE_INSERT_COLOR),
            };
            ui.label(
                RichText::new(mode_text)
                    .monospace()
                    .strong()
                    .size(13.0)
                    .color(mode_color),
            );

            // Audio status
            if self.core.has_sf2() {
                ui.label(
                    RichText::new("SF2")
                        .monospace()
                        .size(12.0)
                        .color(Color32::from_rgb(180, 180, 100)),
                );
            } else if self.core.has_audio() {
                ui.label(
                    RichText::new("SYNTH")
                        .monospace()
                        .size(12.0)
                        .color(Color32::from_rgb(180, 180, 100)),
                );
            }
        });
    }
}
