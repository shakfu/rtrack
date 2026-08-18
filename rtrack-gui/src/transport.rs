use egui::{Color32, DragValue, RichText, Ui};

use crate::app::RtrackApp;
use crate::state::Mode;

const PLAY_COLOR: Color32 = Color32::from_rgb(100, 255, 100);
const STOP_COLOR: Color32 = Color32::from_rgb(200, 200, 200);
const RECORD_COLOR: Color32 = Color32::from_rgb(255, 80, 80);
const MODE_NORMAL_COLOR: Color32 = Color32::from_rgb(100, 180, 255);
const MODE_INSERT_COLOR: Color32 = Color32::from_rgb(255, 100, 100);
const LINK_COLOR: Color32 = Color32::from_rgb(100, 220, 160);
const LINK_DIM_COLOR: Color32 = Color32::from_rgb(80, 140, 100);
const MIDI_CONNECTED_COLOR: Color32 = Color32::from_rgb(100, 200, 255);
const MIDI_DIM_COLOR: Color32 = Color32::from_rgb(80, 80, 100);

impl RtrackApp {
    pub fn draw_transport(&mut self, ui: &mut Ui) {
        // Row 1: Transport -- title, play/stop, record, BPM, speed, time
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

            // Play/Stop button
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
                self.core.toggle_playback(self.edit_order, self.cursor_row);
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

            // BPM
            let prev_bpm = self.core.song.bpm;
            ui.add(
                DragValue::new(&mut self.core.song.bpm)
                    .range(20..=999)
                    .suffix(" BPM"),
            );
            if self.core.song.bpm != prev_bpm {
                self.core.link.set_tempo(self.core.song.bpm as f64);
            }

            // Speed
            ui.add(
                DragValue::new(&mut self.core.song.speed)
                    .range(1..=31)
                    .prefix("Spd:"),
            );

            ui.separator();

            // Playback time
            let elapsed = self.core.timing.playback_elapsed;
            let mins = (elapsed / 60.0) as u32;
            let secs = (elapsed % 60.0) as u32;
            let time_color = if self.core.playing {
                Color32::from_rgb(200, 200, 200)
            } else {
                Color32::from_rgb(100, 100, 120)
            };
            ui.label(
                RichText::new(format!("{}:{:02}", mins, secs))
                    .monospace()
                    .size(13.0)
                    .color(time_color),
            );
        });

        // Row 2: Edit state -- position, pattern, octave, step, follow, mode, status indicators
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // Position
            ui.label(
                RichText::new(format!(
                    "Ord:{:02X} Row:{:02X}",
                    if self.core.playing {
                        self.core.playback_position().0
                    } else {
                        self.edit_order
                    },
                    if self.core.playing {
                        self.core.playback_position().1
                    } else {
                        self.cursor_row
                    }
                ))
                .monospace()
                .size(13.0),
            );

            // Pattern info
            let order_pos = if self.core.playing {
                self.core.playback_position().0
            } else {
                self.edit_order
            };
            if order_pos < self.core.song.order.len() {
                let pat_idx = self.core.song.order.get(order_pos).copied().unwrap_or(0);
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

            // Octave
            let mut oct = self.current_octave as i32;
            ui.add(DragValue::new(&mut oct).range(0..=9).prefix("Oct:"));
            self.current_octave = oct as u8;

            // Edit step
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

            ui.separator();

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

            // MIDI status indicator (clickable -- opens MIDI ports dialog)
            {
                let midi_out = self.core.midi.is_connected();
                let midi_in = self.core.midi_input.is_connected();
                let (midi_text, midi_color) = if midi_out && midi_in {
                    ("MIDI:I/O", MIDI_CONNECTED_COLOR)
                } else if midi_out {
                    ("MIDI:OUT", MIDI_CONNECTED_COLOR)
                } else if midi_in {
                    ("MIDI:IN", MIDI_CONNECTED_COLOR)
                } else {
                    ("MIDI", MIDI_DIM_COLOR)
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(midi_text)
                                .monospace()
                                .size(12.0)
                                .color(midi_color),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.show_midi_ports = !self.show_midi_ports;
                    if self.show_midi_ports {
                        self.midi_port_list =
                            rtrack_core::midi::MidiEngine::list_ports().unwrap_or_default();
                        self.midi_input_port_list =
                            rtrack_core::midi::MidiInputEngine::list_ports().unwrap_or_default();
                    }
                }
            }

            // Clock mode indicator
            {
                use rtrack_core::ClockMode;
                let (clk_text, clk_color) = match self.core.clock_mode {
                    ClockMode::Internal => ("INT", MIDI_DIM_COLOR),
                    ClockMode::ExternalMidi => ("EXT", MIDI_CONNECTED_COLOR),
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(clk_text)
                                .monospace()
                                .size(12.0)
                                .color(clk_color),
                        )
                        .frame(false),
                    )
                    .on_hover_text(match self.core.clock_mode {
                        ClockMode::Internal => "Clock: Internal (click to switch to External MIDI)",
                        ClockMode::ExternalMidi => {
                            "Clock: External MIDI (click to switch to Internal)"
                        }
                    })
                    .clicked()
                {
                    self.core.toggle_clock_mode();
                    let mode_name = match self.core.clock_mode {
                        ClockMode::Internal => "Internal",
                        ClockMode::ExternalMidi => "External MIDI",
                    };
                    self.status_message = Some(format!("Clock: {}", mode_name));
                }
            }

            // Link status
            if self.core.link.is_enabled() {
                let peers = self.core.link.num_peers();
                let (link_text, color) = if peers > 0 {
                    (format!("LINK:{}", peers), LINK_COLOR)
                } else {
                    ("LINK".to_string(), LINK_DIM_COLOR)
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(link_text).monospace().size(12.0).color(color),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.core.toggle_link();
                    self.status_message = Some("Link disabled".to_string());
                }
            } else if ui
                .add(
                    egui::Button::new(
                        RichText::new("LINK")
                            .monospace()
                            .size(12.0)
                            .color(Color32::from_rgb(80, 80, 100)),
                    )
                    .frame(false),
                )
                .clicked()
            {
                self.core.toggle_link();
                self.status_message = Some("Link enabled".to_string());
            }
        });
    }
}
