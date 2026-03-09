use std::sync::Arc;

use rtrack_core::audio::synth::{FilterType, Patch, SynthParams};
use rtrack_core::constants::MAX_INSTRUMENTS;
use rtrack_core::sample::Sample;
use rtrack_core::Instrument;

use crate::app::RtrackApp;

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

fn note_name(midi_note: u8) -> String {
    let name = NOTE_NAMES[(midi_note % 12) as usize];
    let octave = midi_note / 12;
    format!("{}{}", name, octave)
}

/// Instrument type for the sidebar display and type selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstType {
    Empty,
    Synth,
    Sample,
    Midi,
}

impl InstType {
    fn label(self) -> &'static str {
        match self {
            InstType::Empty => "[---]",
            InstType::Synth => "[SYN]",
            InstType::Sample => "[SMP]",
            InstType::Midi => "[MID]",
        }
    }

    fn from_instrument(inst: &Instrument) -> Self {
        if inst.synth_params.is_some() {
            InstType::Synth
        } else if inst.sample_index.is_some() {
            InstType::Sample
        } else if inst.midi_program.is_some() {
            InstType::Midi
        } else {
            InstType::Empty
        }
    }
}

impl RtrackApp {
    /// Left sidebar: instrument list + action buttons.
    /// Called from app.rs inside an egui::SidePanel.
    pub fn draw_instrument_sidebar(&mut self, ui: &mut egui::Ui) {
        let num_instruments = self.core.instruments.len().min(MAX_INSTRUMENTS);

        ui.horizontal(|ui| {
            ui.heading("Instruments");
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.button("Close (Esc)").clicked() {
                        self.show_instrument_list = false;
                    }
                },
            );
        });

        ui.add_space(4.0);

        // New instrument buttons
        ui.horizontal(|ui| {
            if ui.button("+ Synth").clicked() {
                if let Some(slot) = self.find_empty_instrument_slot() {
                    self.core.instruments[slot].name =
                        format!("Synth {:02X}", slot);
                    self.core.instruments[slot].synth_params =
                        Some(SynthParams::from_patch(0));
                    self.selected_instrument = Some(slot);
                    self.core.dirty = true;
                }
            }
            if ui.button("+ Sample").clicked() {
                if let Some(slot) = self.find_empty_instrument_slot() {
                    self.core.instruments[slot].name =
                        format!("Sample {:02X}", slot);
                    self.core.instruments[slot].sample_index = Some(slot);
                    self.selected_instrument = Some(slot);
                    self.core.dirty = true;
                }
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // Scrollable instrument list
        egui::ScrollArea::vertical()
            .id_salt("inst_sidebar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..num_instruments {
                    let inst = &self.core.instruments[i];
                    let itype = InstType::from_instrument(inst);
                    let is_empty = itype == InstType::Empty;

                    if is_empty && i >= 16 {
                        continue;
                    }

                    let display_name = if inst.name.is_empty() {
                        "(empty)".to_string()
                    } else {
                        inst.name.clone()
                    };

                    let label =
                        format!("{:02X} {} {}", i, itype.label(), display_name);
                    let selected = self.selected_instrument == Some(i);

                    let text = if selected {
                        egui::RichText::new(label)
                            .monospace()
                            .strong()
                            .color(egui::Color32::from_rgb(255, 200, 60))
                    } else if is_empty {
                        egui::RichText::new(label)
                            .monospace()
                            .color(egui::Color32::from_rgb(100, 100, 120))
                    } else {
                        egui::RichText::new(label).monospace()
                    };

                    if ui.selectable_label(selected, text).clicked() {
                        self.selected_instrument = Some(i);
                    }
                }
            });

        // Bottom action buttons
        ui.separator();
        ui.horizontal(|ui| {
            let has_sel = self.selected_instrument.is_some();
            if ui
                .add_enabled(has_sel, egui::Button::new("Clear"))
                .on_hover_text("Reset instrument to empty")
                .clicked()
            {
                if let Some(idx) = self.selected_instrument {
                    self.core.instruments[idx] = Instrument::default();
                    self.core.dirty = true;
                }
            }
        });
    }

    /// Right panel: editor for selected instrument.
    /// Called from app.rs inside the CentralPanel.
    pub fn draw_instrument_panel_view(&mut self, ui: &mut egui::Ui) {
        match self.selected_instrument {
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("Select an instrument from the sidebar");
                });
            }
            Some(idx) if idx >= self.core.instruments.len() => {
                ui.label("Invalid instrument index");
            }
            Some(idx) => {
                self.draw_instrument_panel(ui, idx);
            }
        }
    }

    /// Right panel: common header + type selector + type-specific editor.
    fn draw_instrument_panel(&mut self, ui: &mut egui::Ui, idx: usize) {
        let itype = InstType::from_instrument(&self.core.instruments[idx]);

        // Header: name + type selector
        ui.horizontal(|ui| {
            ui.label(format!("Instrument {:02X}", idx));
            ui.separator();
            ui.label("Name:");
            if ui
                .text_edit_singleline(&mut self.core.instruments[idx].name)
                .changed()
            {
                self.core.dirty = true;
            }
        });

        ui.add_space(4.0);

        // Type selector
        ui.horizontal(|ui| {
            ui.label("Type:");

            let mut new_type = itype;
            ui.selectable_value(&mut new_type, InstType::Empty, "Empty");
            ui.selectable_value(&mut new_type, InstType::Synth, "Synth");
            ui.selectable_value(&mut new_type, InstType::Sample, "Sample");
            ui.selectable_value(&mut new_type, InstType::Midi, "MIDI");

            if new_type != itype {
                self.change_instrument_type(idx, new_type);
            }
        });

        // Pitch bend range (common to all non-empty types)
        if itype != InstType::Empty {
            ui.horizontal(|ui| {
                ui.label("Pitch Bend Range:");
                let mut pbr = self.core.instruments[idx]
                    .pitch_bend_range
                    .unwrap_or(2.0) as f32;
                if ui
                    .add(egui::Slider::new(&mut pbr, 0.0..=48.0).suffix(" st"))
                    .changed()
                {
                    self.core.instruments[idx].pitch_bend_range = Some(pbr as f64);
                    self.core.dirty = true;
                }
            });
        }

        ui.separator();

        // Type-specific editor
        egui::ScrollArea::vertical()
            .id_salt("inst_editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match itype {
                    InstType::Empty => {
                        ui.label("Choose a type above to configure this instrument.");
                    }
                    InstType::Synth => {
                        self.draw_synth_params(ui, idx);
                    }
                    InstType::Sample => {
                        self.draw_sample_params(ui, idx);
                    }
                    InstType::Midi => {
                        self.draw_midi_params(ui, idx);
                    }
                }
            });
    }

    /// Change instrument type, preserving name and pitch bend range.
    fn change_instrument_type(&mut self, idx: usize, new_type: InstType) {
        let inst = &mut self.core.instruments[idx];
        let name = inst.name.clone();
        let pbr = inst.pitch_bend_range;

        inst.synth_params = None;
        inst.sample_index = None;
        inst.midi_program = None;

        match new_type {
            InstType::Synth => {
                inst.synth_params = Some(SynthParams::from_patch(0));
            }
            InstType::Sample => {
                inst.sample_index = Some(idx);
            }
            InstType::Midi => {
                inst.midi_program = Some(0);
            }
            InstType::Empty => {}
        }

        inst.name = name;
        inst.pitch_bend_range = pbr;
        self.core.dirty = true;
    }

    // ------------------------------------------------------------------
    // Synth Editor Panel
    // ------------------------------------------------------------------

    fn draw_synth_params(&mut self, ui: &mut egui::Ui, idx: usize) {
        let params = match self.core.instruments[idx].synth_params.as_mut() {
            Some(p) => p,
            None => return, // type was just changed away from Synth
        };

        let mut changed = false;

        // Patch selector
        ui.horizontal(|ui| {
            ui.label("Patch:");
            let current_patch = Patch::from_program(params.waveform);
            egui::ComboBox::from_id_salt(format!("synth_patch_{}", idx))
                .selected_text(current_patch.name())
                .show_ui(ui, |ui| {
                    for prog in 0..Patch::count() {
                        let patch = Patch::from_program(prog);
                        if ui
                            .selectable_value(&mut params.waveform, prog, patch.name())
                            .clicked()
                        {
                            let preset = SynthParams::from_patch(prog);
                            params.attack = preset.attack;
                            params.decay = preset.decay;
                            params.sustain = preset.sustain;
                            params.release = preset.release;
                            params.filter_cutoff = preset.filter_cutoff;
                            params.filter_resonance = preset.filter_resonance;
                            params.filter_env = preset.filter_env;
                            params.detune = preset.detune;
                            params.filter_type = preset.filter_type;
                            params.sub_osc = preset.sub_osc;
                            params.fm_ratio = preset.fm_ratio;
                            params.fm_index = preset.fm_index;
                            params.pulse_width = preset.pulse_width;
                            changed = true;
                        }
                    }
                });
        });

        ui.add_space(8.0);

        // Envelope + Filter in a single grid (avoids ui.columns width assertion)
        ui.heading("Envelope");
        egui::Grid::new(format!("synth_adsr_{}", idx))
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Attack:");
                if ui.add(
                    egui::Slider::new(&mut params.attack, 0.001..=5.0)
                        .logarithmic(true)
                        .suffix(" s"),
                ).changed() { changed = true; }

                ui.label("Decay:");
                if ui.add(
                    egui::Slider::new(&mut params.decay, 0.001..=5.0)
                        .logarithmic(true)
                        .suffix(" s"),
                ).changed() { changed = true; }
                ui.end_row();

                ui.label("Sustain:");
                if ui.add(egui::Slider::new(&mut params.sustain, 0.0..=1.0)).changed() {
                    changed = true;
                }

                ui.label("Release:");
                if ui.add(
                    egui::Slider::new(&mut params.release, 0.001..=10.0)
                        .logarithmic(true)
                        .suffix(" s"),
                ).changed() { changed = true; }
                ui.end_row();
            });

        ui.add_space(8.0);

        ui.heading("Filter");
        egui::Grid::new(format!("synth_filter_{}", idx))
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Type:");
                ui.horizontal(|ui| {
                    if ui.selectable_value(&mut params.filter_type, FilterType::LowPass, "LP").changed() { changed = true; }
                    if ui.selectable_value(&mut params.filter_type, FilterType::HighPass, "HP").changed() { changed = true; }
                    if ui.selectable_value(&mut params.filter_type, FilterType::BandPass, "BP").changed() { changed = true; }
                });

                ui.label("Cutoff:");
                if ui.add(egui::Slider::new(&mut params.filter_cutoff, 0.0..=1.0).custom_formatter(
                    |v, _| {
                        let hz = 20.0 * (1000.0_f64).powf(v);
                        format!("{:.0} Hz", hz)
                    },
                )).changed() { changed = true; }
                ui.end_row();

                ui.label("Resonance:");
                if ui.add(egui::Slider::new(&mut params.filter_resonance, 0.0..=1.0)).changed() {
                    changed = true;
                }

                ui.label("Env Amt:");
                if ui.add(egui::Slider::new(&mut params.filter_env, -1.0..=1.0)).changed() {
                    changed = true;
                }
                ui.end_row();
            });

        ui.add_space(8.0);

        ui.heading("Oscillator");
        egui::Grid::new(format!("synth_osc_{}", idx))
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Detune:");
                if ui.add(egui::Slider::new(&mut params.detune, 0.0..=50.0).suffix(" cents")).changed() {
                    changed = true;
                }

                ui.label("Sub Osc:");
                if ui.add(egui::Slider::new(&mut params.sub_osc, 0.0..=1.0)).changed() {
                    changed = true;
                }
                ui.end_row();

                ui.label("Pulse Width:");
                if ui.add(egui::Slider::new(&mut params.pulse_width, 0.05..=0.95)).changed() {
                    changed = true;
                }

                ui.label(""); // spacer
                ui.label("");
                ui.end_row();
            });

        ui.add_space(8.0);

        ui.heading("FM Synthesis");
        egui::Grid::new(format!("synth_fm_{}", idx))
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("FM Ratio:");
                if ui.add(egui::Slider::new(&mut params.fm_ratio, 0.0..=16.0)).changed() {
                    changed = true;
                }

                ui.label("FM Index:");
                if ui.add(egui::Slider::new(&mut params.fm_index, 0.0..=10.0)).changed() {
                    changed = true;
                }
                ui.end_row();
            });

        if changed {
            self.core.dirty = true;
        }
    }

    // ------------------------------------------------------------------
    // Sample Editor Panel
    // ------------------------------------------------------------------

    fn draw_sample_params(&mut self, ui: &mut egui::Ui, idx: usize) {
        ui.horizontal(|ui| {
            if ui.button("Load Sample...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "aif", "aiff"])
                    .pick_file()
                {
                    let slot = self.core.instruments[idx].sample_index.unwrap_or(idx);

                    let mut bank = (*self.core.sample_bank).clone();
                    match bank.load(slot, &path) {
                        Ok(()) => {
                            self.core.sample_bank = Arc::new(bank);
                            self.core.instruments[idx].sample_index = Some(slot);
                            if let Some(ref mut audio) = self.core.audio {
                                audio.set_sample_bank(self.core.sample_bank.clone());
                            }
                            self.core.dirty = true;
                            self.status_message =
                                Some(format!("Loaded sample into slot {:02X}", slot));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Failed to load sample: {}", e));
                        }
                    }
                }
            }
            if ui.button("Load Directory...").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    match self.core.load_sample_directory(&dir) {
                        Ok(msg) => {
                            self.status_message = Some(msg);
                        }
                        Err(msg) => {
                            self.status_message = Some(msg);
                        }
                    }
                }
            }
        });

        ui.separator();

        let sample_slot = self.core.instruments[idx].sample_index;
        if let Some(slot) = sample_slot {
            // Extract sample data before entering closures to avoid borrow conflicts
            let waveform_peaks: Vec<f32> = self.core.sample_bank.get(slot).map(|s| {
                downsample_peaks(&s.data, 512)
            }).unwrap_or_default();
            let sample_info = self.core.sample_bank.get(slot).map(|s| {
                (
                    s.source_path.clone(),
                    s.sample_rate,
                    s.len(),
                    s.duration(),
                    s.base_note,
                    s.loop_enabled,
                    s.loop_start,
                    s.loop_end,
                    s.trim_start,
                    s.trim_end,
                )
            });
            if let Some((source_path, sample_rate, sample_len, duration, init_base_note, init_loop_enabled, init_loop_start, init_loop_end, init_trim_start, init_trim_end)) = sample_info {
                ui.heading("Sample Info");
                egui::Grid::new(format!("sample_info_{}", idx))
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("File:");
                        ui.label(source_path.as_deref().unwrap_or("(none)"));

                        ui.label("Rate:");
                        ui.label(format!("{} Hz", sample_rate as u32));
                        ui.end_row();

                        ui.label("Length:");
                        ui.label(format!(
                            "{} ({:.2}s)",
                            sample_len,
                            duration
                        ));
                        ui.label("");
                        ui.label("");
                        ui.end_row();
                    });

                // Waveform preview
                if !waveform_peaks.is_empty() {
                    ui.add_space(4.0);
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 80.0),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 20, 30));

                    let mid_y = rect.center().y;
                    let half_h = rect.height() / 2.0;
                    let w = rect.width();
                    let n = waveform_peaks.len() as f32;

                    for (i, &peak) in waveform_peaks.iter().enumerate() {
                        let x = rect.left() + (i as f32 / n) * w;
                        let h = peak * half_h;
                        painter.line_segment(
                            [egui::pos2(x, mid_y - h), egui::pos2(x, mid_y + h)],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 180, 255)),
                        );
                    }

                    // Draw trim markers
                    if init_trim_start > 0 {
                        let trim_x = rect.left() + (init_trim_start as f32 / sample_len as f32) * w;
                        painter.line_segment(
                            [egui::pos2(trim_x, rect.top()), egui::pos2(trim_x, rect.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 200, 60)),
                        );
                    }
                    if init_trim_end > 0 && init_trim_end < sample_len {
                        let trim_x = rect.left() + (init_trim_end as f32 / sample_len as f32) * w;
                        painter.line_segment(
                            [egui::pos2(trim_x, rect.top()), egui::pos2(trim_x, rect.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 200, 60)),
                        );
                    }

                    // Draw loop markers
                    if init_loop_enabled {
                        let ls = rect.left() + (init_loop_start as f32 / sample_len as f32) * w;
                        let le = rect.left() + (init_loop_end as f32 / sample_len as f32) * w;
                        painter.line_segment(
                            [egui::pos2(ls, rect.top()), egui::pos2(ls, rect.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 255, 100)),
                        );
                        painter.line_segment(
                            [egui::pos2(le, rect.top()), egui::pos2(le, rect.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 255, 100)),
                        );
                    }
                }

                ui.add_space(8.0);

                ui.heading("Parameters");

                let mut base_note = init_base_note;
                let mut loop_enabled = init_loop_enabled;
                let mut loop_start = init_loop_start;
                let mut loop_end_val = init_loop_end;
                let mut trim_start = init_trim_start;
                let mut trim_end_val = init_trim_end;
                let mut changed = false;

                egui::Grid::new(format!("sample_params_{}", idx))
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Base Note:");
                        ui.horizontal(|ui| {
                            let mut bn = base_note as i32;
                            if ui
                                .add(egui::DragValue::new(&mut bn).range(0..=127))
                                .changed()
                            {
                                base_note = bn as u8;
                                changed = true;
                            }
                            ui.label(note_name(base_note));
                        });

                        ui.label("Loop:");
                        if ui.checkbox(&mut loop_enabled, "").changed() {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Trim Start:");
                        if ui
                            .add(
                                egui::DragValue::new(&mut trim_start)
                                    .range(0..=sample_len.saturating_sub(1)),
                            )
                            .changed()
                        {
                            changed = true;
                        }

                        ui.label("Trim End:");
                        if ui
                            .add(egui::DragValue::new(&mut trim_end_val).range(0..=sample_len))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        if loop_enabled {
                            ui.label("Loop Start:");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut loop_start)
                                        .range(0..=sample_len.saturating_sub(1)),
                                )
                                .changed()
                            {
                                changed = true;
                            }

                            ui.label("Loop End:");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut loop_end_val)
                                        .range(0..=sample_len),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();
                        }
                    });

                if changed {
                    let mut bank_clone = (*self.core.sample_bank).clone();
                    if let Some(s) = bank_clone.samples[slot].as_mut() {
                        s.base_note = base_note;
                        s.loop_enabled = loop_enabled;
                        s.loop_start = loop_start;
                        s.loop_end = loop_end_val;
                        s.trim_start = trim_start;
                        s.trim_end = trim_end_val;
                    }
                    self.core.sample_bank = Arc::new(bank_clone);
                    if let Some(ref mut audio) = self.core.audio {
                        audio.set_sample_bank(self.core.sample_bank.clone());
                    }
                    self.core.dirty = true;
                }

                // Slicing section
                ui.separator();
                ui.heading("Slicing");
                ui.horizontal(|ui| {
                    ui.label("Slices:");
                    ui.add(egui::DragValue::new(&mut self.slice_count).range(2..=64));
                    if ui.button("Slice Equal").clicked() {
                        self.do_equal_slice(idx, slot);
                    }
                    ui.separator();
                    ui.label("Sensitivity:");
                    ui.add(egui::Slider::new(&mut self.slice_sensitivity, 0.0..=1.0));
                    if ui.button("Slice Transients").clicked() {
                        self.do_transient_slice(idx, slot);
                    }
                });
            } else {
                ui.label("No sample loaded. Click 'Load Sample' above.");
            }
        } else {
            ui.label("No sample slot assigned. Click 'Load Sample' to load one.");
        }
    }

    // ------------------------------------------------------------------
    // MIDI Program Editor Panel
    // ------------------------------------------------------------------

    fn draw_midi_params(&mut self, ui: &mut egui::Ui, idx: usize) {
        ui.heading("MIDI Program");
        let mut prog = self.core.instruments[idx].midi_program.unwrap_or(0) as i32;
        if ui
            .add(egui::DragValue::new(&mut prog).range(0..=127).prefix("Program: "))
            .changed()
        {
            self.core.instruments[idx].midi_program = Some(prog as u8);
            self.core.dirty = true;
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn find_empty_instrument_slot(&self) -> Option<usize> {
        let max = self.core.instruments.len().min(MAX_INSTRUMENTS);
        for i in 0..max {
            let inst = &self.core.instruments[i];
            if inst.name.is_empty()
                && inst.synth_params.is_none()
                && inst.sample_index.is_none()
                && inst.midi_program.is_none()
            {
                return Some(i);
            }
        }
        None
    }

    fn do_equal_slice(&mut self, inst_idx: usize, slot: usize) {
        let num_slices = self.slice_count;
        let slices = {
            let sample = match self.core.sample_bank.get(slot) {
                Some(s) => s,
                None => return,
            };
            rtrack_core::sample::slice_equal(sample, num_slices)
        };

        if slices.is_empty() {
            self.status_message = Some("No slices produced".to_string());
            return;
        }

        self.insert_slices(inst_idx, slot, slices);
    }

    fn do_transient_slice(&mut self, inst_idx: usize, slot: usize) {
        let sensitivity = self.slice_sensitivity;
        let (points, sample_clone) = {
            let sample = match self.core.sample_bank.get(slot) {
                Some(s) => s,
                None => return,
            };
            let pts = rtrack_core::sample::detect_transients(sample, sensitivity);
            (pts, sample.clone())
        };

        if points.is_empty() {
            self.status_message = Some("No transients detected".to_string());
            return;
        }

        let slices = rtrack_core::sample::slice_at_points(&sample_clone, &points);
        if slices.is_empty() {
            self.status_message = Some("No slices produced".to_string());
            return;
        }

        self.insert_slices(inst_idx, slot, slices);
    }

    fn insert_slices(&mut self, _inst_idx: usize, start_slot: usize, slices: Vec<Sample>) {
        let mut bank = (*self.core.sample_bank).clone();
        let num = slices.len();
        let max_slot = bank.samples.len();

        for (i, slice) in slices.into_iter().enumerate() {
            let target_slot = start_slot + i;
            if target_slot >= max_slot {
                break;
            }
            let slice_name = slice.name.clone();
            bank.samples[target_slot] = Some(slice);

            if target_slot < self.core.instruments.len() {
                let inst = &mut self.core.instruments[target_slot];
                if inst.name.is_empty() || target_slot != start_slot {
                    inst.name = slice_name;
                    inst.sample_index = Some(target_slot);
                }
            }
        }

        self.core.sample_bank = Arc::new(bank);
        if let Some(ref mut audio) = self.core.audio {
            audio.set_sample_bank(self.core.sample_bank.clone());
        }
        self.core.dirty = true;
        self.status_message = Some(format!(
            "Created {} slices starting at slot {:02X}",
            num, start_slot
        ));
    }
}

/// Downsample stereo audio data to peak values for waveform display.
fn downsample_peaks(data: &[[f32; 2]], num_bins: usize) -> Vec<f32> {
    if data.is_empty() || num_bins == 0 {
        return Vec::new();
    }
    let bin_size = (data.len() as f32 / num_bins as f32).max(1.0);
    let mut peaks = Vec::with_capacity(num_bins);
    for i in 0..num_bins {
        let start = (i as f32 * bin_size) as usize;
        let end = ((i + 1) as f32 * bin_size) as usize;
        let end = end.min(data.len());
        let mut max_val: f32 = 0.0;
        for frame in &data[start..end] {
            let mono = (frame[0].abs() + frame[1].abs()) * 0.5;
            if mono > max_val {
                max_val = mono;
            }
        }
        peaks.push(max_val);
    }
    peaks
}
