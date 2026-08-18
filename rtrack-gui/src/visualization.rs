use std::sync::Arc;

use egui::{pos2, Color32, Painter, Rect, Stroke, Ui, Vec2};
use rustfft::{num_complex::Complex, FftPlanner};

use rtrack_core::audio::{AudioEngine, VoiceSnapshot};
use rtrack_core::sample::SampleBank;

/// FFT size for spectrum analysis (must be power of 2).
const FFT_SIZE: usize = 2048;

/// Number of spectrum bars to display.
const SPECTRUM_BARS: usize = 64;

/// Peak hold decay rate per frame (~60fps: decays over ~30 frames).
const PEAK_DECAY: f32 = 0.04;

/// Smoothing factor for spectrum bins (0=no smoothing, 1=frozen).
const SPECTRUM_SMOOTHING: f32 = 0.7;

/// Smoothing for meter levels.
const METER_SMOOTHING: f32 = 0.8;

/// Number of waveform bins for sample display.
const WAVEFORM_BINS: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VisTab {
    Spectrum,
    Samples,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SliceMode {
    Equal,
    Transient,
}

/// Action to apply slices (consumed by app).
pub struct SliceAction {
    pub slot: usize,
    pub mode: SliceMode,
    pub count: usize,
    pub sensitivity: f32,
}

/// State for the spectrum analyzer, level meters, and sample viewer.
pub struct VisualizationState {
    // Tab
    pub tab: VisTab,

    // Spectrum state
    sample_buf: Vec<f32>,
    planner: FftPlanner<f32>,
    spectrum: Vec<f32>,
    window: Vec<f32>,
    meter_l: f32,
    meter_r: f32,
    peak_hold_l: f32,
    peak_hold_r: f32,

    // Sample viewer state
    pub selected_sample_slot: Option<usize>,
    /// Set when a slot button is clicked (consumed by app to trigger preview).
    pub preview_slot: Option<usize>,
    cached_voice_snapshots: Vec<VoiceSnapshot>,

    // Slicing controls
    pub slice_mode: SliceMode,
    pub slice_count: usize,
    pub slice_sensitivity: f32,
    preview_slice_points: Vec<usize>,
    /// Set when parameters change (consumed by app to commit slices).
    pub pending_slice_action: Option<SliceAction>,
    /// The base slot slicing operates on (first slot of the sample, not the viewed slot).
    slice_source_slot: Option<usize>,
    // Change tracking for auto-apply (mode, count, sensitivity_bits -- NOT slot)
    last_applied: Option<(SliceMode, usize, u32)>,
}

impl VisualizationState {
    pub fn new() -> Self {
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / FFT_SIZE as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * t).cos())
            })
            .collect();
        Self {
            tab: VisTab::Spectrum,
            sample_buf: Vec::with_capacity(FFT_SIZE * 2),
            planner: FftPlanner::new(),
            spectrum: vec![-90.0; SPECTRUM_BARS],
            window,
            meter_l: 0.0,
            meter_r: 0.0,
            peak_hold_l: 0.0,
            peak_hold_r: 0.0,
            selected_sample_slot: None,
            preview_slot: None,
            cached_voice_snapshots: Vec::new(),
            slice_mode: SliceMode::Equal,
            slice_count: 8,
            slice_sensitivity: 0.5,
            preview_slice_points: Vec::new(),
            pending_slice_action: None,
            slice_source_slot: None,
            last_applied: None,
        }
    }

    /// Drain audio samples from the engine and update spectrum + meters + voice snapshots.
    pub fn update(&mut self, audio: &mut Option<AudioEngine>) {
        // Read peak levels from audio engine
        if let Some(ref audio) = audio {
            let (pl, pr) = audio.peak_levels();
            self.meter_l = self.meter_l * METER_SMOOTHING + pl * (1.0 - METER_SMOOTHING);
            self.meter_r = self.meter_r * METER_SMOOTHING + pr * (1.0 - METER_SMOOTHING);
            if pl > self.peak_hold_l {
                self.peak_hold_l = pl;
            } else {
                self.peak_hold_l = (self.peak_hold_l - PEAK_DECAY).max(0.0);
            }
            if pr > self.peak_hold_r {
                self.peak_hold_r = pr;
            } else {
                self.peak_hold_r = (self.peak_hold_r - PEAK_DECAY).max(0.0);
            }
        }

        // Drain visualization samples
        if let Some(ref mut audio) = audio {
            audio.read_vis_samples(&mut self.sample_buf);
        }

        // Compute FFT when we have enough
        if self.sample_buf.len() >= FFT_SIZE {
            self.compute_spectrum();
            let drain_to = self.sample_buf.len() - FFT_SIZE / 2;
            self.sample_buf.drain(..drain_to);
        }
        if self.sample_buf.len() > FFT_SIZE * 4 {
            let keep_from = self.sample_buf.len() - FFT_SIZE;
            self.sample_buf.drain(..keep_from);
        }

        // Update voice snapshots
        if let Some(ref audio) = audio {
            self.cached_voice_snapshots = audio.voice_snapshots();
        }
    }

    fn compute_spectrum(&mut self) {
        let fft = self.planner.plan_fft_forward(FFT_SIZE);
        let start = self.sample_buf.len() - FFT_SIZE;
        let mut buffer: Vec<Complex<f32>> = self.sample_buf[start..start + FFT_SIZE]
            .iter()
            .enumerate()
            .map(|(i, &s)| Complex::new(s * self.window[i], 0.0))
            .collect();

        fft.process(&mut buffer);

        let half = FFT_SIZE / 2;
        let magnitudes: Vec<f32> = buffer[..half]
            .iter()
            .map(|c| {
                let mag = c.norm() / FFT_SIZE as f32;
                (20.0 * mag.max(1e-10).log10()).max(-90.0)
            })
            .collect();

        let min_freq_bin = 1;
        let max_freq_bin = half;
        for bar in 0..SPECTRUM_BARS {
            let t0 = bar as f32 / SPECTRUM_BARS as f32;
            let t1 = (bar + 1) as f32 / SPECTRUM_BARS as f32;
            let lo = (min_freq_bin as f32 * (max_freq_bin as f32 / min_freq_bin as f32).powf(t0))
                as usize;
            let hi = ((min_freq_bin as f32 * (max_freq_bin as f32 / min_freq_bin as f32).powf(t1))
                as usize)
                .max(lo + 1)
                .min(half);
            let sum: f32 = magnitudes[lo..hi].iter().sum();
            let avg = sum / (hi - lo) as f32;
            self.spectrum[bar] =
                self.spectrum[bar] * SPECTRUM_SMOOTHING + avg * (1.0 - SPECTRUM_SMOOTHING);
        }
    }

    /// Draw the visualization panel with tab selector.
    pub fn draw(&mut self, ui: &mut Ui, sample_bank: &Arc<SampleBank>) {
        // Tab bar
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, VisTab::Spectrum, "Spectrum");
            ui.selectable_value(&mut self.tab, VisTab::Samples, "Samples");

            if self.tab == VisTab::Samples {
                ui.separator();
                // Sample slot selector: show loaded slots
                let loaded = sample_bank.loaded_slots();
                if loaded.is_empty() {
                    ui.label(
                        egui::RichText::new("No samples loaded")
                            .color(Color32::from_rgb(120, 120, 140)),
                    );
                } else {
                    for &slot in &loaded {
                        let name = sample_bank
                            .get(slot)
                            .map(|s| {
                                if s.name.is_empty() {
                                    format!("{:02X}", slot)
                                } else {
                                    s.name.clone()
                                }
                            })
                            .unwrap_or_default();
                        let selected = self.selected_sample_slot == Some(slot);
                        // Highlight slots that are currently playing
                        let is_playing = self
                            .cached_voice_snapshots
                            .iter()
                            .any(|v| v.sample_index == slot);
                        let label = if is_playing {
                            egui::RichText::new(&name).color(Color32::from_rgb(100, 255, 100))
                        } else {
                            egui::RichText::new(&name)
                        };
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected_sample_slot = Some(slot);
                            self.preview_slot = Some(slot);
                        }
                    }
                    // Auto-select first if none selected
                    if self.selected_sample_slot.is_none()
                        || !loaded.contains(&self.selected_sample_slot.unwrap_or(usize::MAX))
                    {
                        self.selected_sample_slot = Some(loaded[0]);
                    }
                }
            }
        });

        // Content area
        match self.tab {
            VisTab::Spectrum => self.draw_spectrum_tab(ui),
            VisTab::Samples => self.draw_samples_tab(ui, sample_bank),
        }
    }

    fn draw_spectrum_tab(&self, ui: &mut Ui) {
        let avail = ui.available_size();
        let meter_width = 20.0;
        let meter_gap = 6.0;
        let spectrum_width = avail.x - (meter_width * 2.0 + meter_gap * 3.0);

        ui.horizontal(|ui| {
            let spectrum_size = Vec2::new(spectrum_width.max(100.0), avail.y);
            let (response, painter) = ui.allocate_painter(spectrum_size, egui::Sense::hover());
            draw_spectrum_bars(&painter, response.rect, &self.spectrum);

            ui.add_space(meter_gap);

            let meter_size = Vec2::new(meter_width, avail.y);
            let (_, painter_l) = ui.allocate_painter(meter_size, egui::Sense::hover());
            draw_meter(
                &painter_l,
                painter_l.clip_rect(),
                self.meter_l,
                self.peak_hold_l,
                "L",
            );

            ui.add_space(2.0);

            let (_, painter_r) = ui.allocate_painter(meter_size, egui::Sense::hover());
            draw_meter(
                &painter_r,
                painter_r.clip_rect(),
                self.meter_r,
                self.peak_hold_r,
                "R",
            );
        });
    }

    fn draw_samples_tab(&mut self, ui: &mut Ui, sample_bank: &Arc<SampleBank>) {
        let slot = match self.selected_sample_slot {
            Some(s) => s,
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("No sample selected");
                });
                return;
            }
        };

        let sample = match sample_bank.get(slot) {
            Some(s) => s,
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("Sample slot empty");
                });
                return;
            }
        };

        if sample.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Empty sample");
            });
            return;
        }

        // Collect voice positions from all slots that share the same source file,
        // so sliced samples show all active slice playheads on one waveform.
        let source_path = sample.source_path.as_deref();
        let related_slots: Vec<usize> = if source_path.is_some() {
            sample_bank
                .loaded_slots()
                .into_iter()
                .filter(|&s| {
                    sample_bank
                        .get(s)
                        .and_then(|smp| smp.source_path.as_deref())
                        == source_path
                })
                .collect()
        } else {
            vec![slot]
        };

        let voice_positions: Vec<(f64, bool)> = self
            .cached_voice_snapshots
            .iter()
            .filter(|v| related_slots.contains(&v.sample_index))
            .map(|v| (v.position, v.sample_index == slot))
            .collect();

        // Collect committed slice boundaries from related slots
        let slice_boundaries: Vec<(usize, usize)> = if related_slots.len() > 1 {
            related_slots
                .iter()
                .filter_map(|&s| sample_bank.get(s).map(|smp| (smp.trim_start, smp.end())))
                .collect()
        } else {
            Vec::new()
        };

        // Slicing controls row
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.slice_mode, SliceMode::Equal, "Equal");
            ui.selectable_value(&mut self.slice_mode, SliceMode::Transient, "Transient");
            ui.separator();
            match self.slice_mode {
                SliceMode::Equal => {
                    ui.label("Slices:");
                    ui.add(egui::DragValue::new(&mut self.slice_count).range(2..=64));
                }
                SliceMode::Transient => {
                    ui.label("Sensitivity:");
                    ui.add(
                        egui::Slider::new(&mut self.slice_sensitivity, 0.01..=1.0).max_decimals(2),
                    );
                }
            }
            let num = self.preview_slice_points.len() + 1;
            ui.label(
                egui::RichText::new(format!("({} slices)", num))
                    .color(Color32::from_rgb(0, 220, 220)),
            );
        });

        // Determine the source slot for slicing (first related slot, stable across clicks)
        let source_slot = if let Some(ss) = self.slice_source_slot {
            if related_slots.contains(&ss) {
                ss
            } else {
                // Source slot no longer related (different sample), reset
                let first = *related_slots.first().unwrap_or(&slot);
                self.slice_source_slot = Some(first);
                self.last_applied = None;
                first
            }
        } else {
            let first = *related_slots.first().unwrap_or(&slot);
            self.slice_source_slot = Some(first);
            first
        };

        // Use the source slot's sample for slicing (always has full data)
        let source_sample = sample_bank.get(source_slot).unwrap_or(sample);
        let full_len = source_sample.len();

        // Compute preview slice positions over full data range
        self.preview_slice_points.clear();
        if full_len > 0 {
            match self.slice_mode {
                SliceMode::Equal => {
                    for i in 1..self.slice_count {
                        self.preview_slice_points
                            .push((i * full_len) / self.slice_count);
                    }
                }
                SliceMode::Transient => {
                    let pts = rtrack_core::sample::detect_transients_range(
                        source_sample,
                        self.slice_sensitivity,
                        0,
                        full_len,
                    );
                    // Skip the first point (always 0)
                    for &p in pts.iter().skip(1) {
                        self.preview_slice_points.push(p);
                    }
                }
            }
        }

        // Auto-apply: emit action when slice PARAMETERS change (not when view slot changes)
        let current_key = (
            self.slice_mode,
            self.slice_count,
            self.slice_sensitivity.to_bits(),
        );
        if self.last_applied != Some(current_key) {
            self.last_applied = Some(current_key);
            self.pending_slice_action = Some(SliceAction {
                slot: source_slot,
                mode: self.slice_mode,
                count: self.slice_count,
                sensitivity: self.slice_sensitivity,
            });
        }

        // Waveform
        let avail = ui.available_size();
        let (response, painter) =
            ui.allocate_painter(Vec2::new(avail.x, avail.y), egui::Sense::hover());
        let rect = response.rect;

        draw_sample_waveform(
            &painter,
            rect,
            sample,
            &voice_positions,
            &slice_boundaries,
            &self.preview_slice_points,
        );
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

fn draw_spectrum_bars(painter: &Painter, rect: Rect, spectrum: &[f32]) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(15, 15, 20));

    let bar_count = spectrum.len();
    if bar_count == 0 {
        return;
    }

    let bar_width = rect.width() / bar_count as f32;
    let db_min = -72.0f32;
    let db_max = 0.0f32;
    let db_range = db_max - db_min;

    for (i, &db) in spectrum.iter().enumerate() {
        let normalized = ((db - db_min) / db_range).clamp(0.0, 1.0);
        if normalized < 0.005 {
            continue;
        }

        let x = rect.left() + i as f32 * bar_width;
        let bar_height = normalized * rect.height();
        let bar_top = rect.bottom() - bar_height;
        let color = level_color(normalized);

        let bar_rect = Rect::from_min_max(
            pos2(x + 0.5, bar_top),
            pos2(x + bar_width - 0.5, rect.bottom()),
        );
        painter.rect_filled(bar_rect, 0.0, color);
    }

    for &db_line in &[-6.0, -12.0, -24.0, -48.0] {
        let norm = ((db_line - db_min) / db_range).clamp(0.0, 1.0);
        let y = rect.bottom() - norm * rect.height();
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(0.5_f32, Color32::from_rgba_premultiplied(80, 80, 100, 60)),
        );
    }
}

fn draw_meter(painter: &Painter, rect: Rect, level: f32, peak: f32, label: &str) {
    painter.rect_filled(rect, 2.0, Color32::from_rgb(20, 20, 25));

    let label_height = 14.0;
    let meter_rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - label_height));

    let level_norm = level.sqrt().clamp(0.0, 1.0);
    let segments = 32;
    let seg_height = meter_rect.height() / segments as f32;
    let x_left = meter_rect.left() + 1.0;
    let x_right = meter_rect.right() - 1.0;

    for s in 0..segments {
        let norm = (s as f32 + 0.5) / segments as f32;
        if norm > level_norm {
            break;
        }
        let seg_bottom = meter_rect.bottom() - s as f32 * seg_height;
        let seg_top = seg_bottom - seg_height;
        let color = level_color(norm);
        painter.rect_filled(
            Rect::from_min_max(pos2(x_left, seg_top), pos2(x_right, seg_bottom)),
            0.0,
            color,
        );
    }

    let peak_norm = peak.sqrt().clamp(0.0, 1.0);
    if peak_norm > 0.01 {
        let peak_y = meter_rect.bottom() - peak_norm * meter_rect.height();
        painter.line_segment(
            [
                pos2(meter_rect.left() + 1.0, peak_y),
                pos2(meter_rect.right() - 1.0, peak_y),
            ],
            Stroke::new(1.5_f32, Color32::WHITE),
        );
    }

    painter.text(
        pos2(rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::monospace(10.0),
        Color32::from_rgb(150, 150, 170),
    );
}

/// Draw a sample waveform with trim/loop markers and playback positions.
/// `voice_positions` contains (position, is_selected_slot) tuples.
/// `slice_boundaries` contains (trim_start, trim_end) for all related slots (committed dividers).
/// `preview_slices` contains frame positions for live preview slice markers.
fn draw_sample_waveform(
    painter: &Painter,
    rect: Rect,
    sample: &rtrack_core::sample::Sample,
    voice_positions: &[(f64, bool)],
    slice_boundaries: &[(usize, usize)],
    preview_slices: &[usize],
) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(15, 15, 20));

    let total_len = sample.len();
    if total_len == 0 {
        return;
    }

    let w = rect.width();
    let mid_y = rect.center().y;
    let half_h = rect.height() / 2.0 - 2.0;

    // Downsample to screen bins
    let num_bins = (w as usize).min(WAVEFORM_BINS);
    let bin_size = total_len as f32 / num_bins as f32;

    // Draw waveform
    let trim_start = sample.trim_start;
    let trim_end = sample.end();

    for i in 0..num_bins {
        let start_frame = (i as f32 * bin_size) as usize;
        let end_frame = (((i + 1) as f32 * bin_size) as usize).min(total_len);

        let mut peak = 0.0f32;
        for frame_idx in start_frame..end_frame {
            let frame = sample.frame_at(frame_idx);
            let mono = (frame[0].abs() + frame[1].abs()) * 0.5;
            if mono > peak {
                peak = mono;
            }
        }

        let x = rect.left() + (i as f32 / num_bins as f32) * w;
        let h = peak * half_h;

        // Dim regions outside trim range
        let frame_mid = (start_frame + end_frame) / 2;
        let in_trim = frame_mid >= trim_start && frame_mid < trim_end;
        let in_loop = sample.loop_enabled
            && frame_mid >= sample.effective_loop_start()
            && frame_mid < sample.effective_loop_end();

        let color = if in_loop {
            Color32::from_rgb(80, 220, 120)
        } else if in_trim {
            Color32::from_rgb(80, 180, 255)
        } else {
            Color32::from_rgb(50, 70, 90)
        };

        if h > 0.5 {
            painter.line_segment(
                [pos2(x, mid_y - h), pos2(x, mid_y + h)],
                Stroke::new(1.0_f32, color),
            );
        }
    }

    // Center line
    painter.line_segment(
        [pos2(rect.left(), mid_y), pos2(rect.right(), mid_y)],
        Stroke::new(0.5_f32, Color32::from_rgba_premultiplied(80, 80, 100, 80)),
    );

    // Slice boundary dividers (from related slots sharing the same source file)
    if !slice_boundaries.is_empty() {
        let slice_color = Color32::from_rgb(180, 160, 60);
        let mut boundaries: Vec<usize> = Vec::new();
        for &(start, end) in slice_boundaries {
            if start > 0 {
                boundaries.push(start);
            }
            if end < total_len {
                boundaries.push(end);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        for &b in &boundaries {
            let x = rect.left() + (b as f32 / total_len as f32) * w;
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0_f32, slice_color),
            );
        }
    }

    // Trim markers for the selected slot (brighter than slice dividers)
    let marker_color_trim = Color32::from_rgb(255, 200, 60);
    if trim_start > 0 {
        let x = rect.left() + (trim_start as f32 / total_len as f32) * w;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5_f32, marker_color_trim),
        );
    }
    if sample.trim_end > 0 && sample.trim_end < total_len {
        let x = rect.left() + (sample.trim_end as f32 / total_len as f32) * w;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5_f32, marker_color_trim),
        );
    }

    // Loop markers
    if sample.loop_enabled {
        let loop_color = Color32::from_rgb(100, 255, 100);
        let ls = rect.left() + (sample.effective_loop_start() as f32 / total_len as f32) * w;
        let le = rect.left() + (sample.effective_loop_end() as f32 / total_len as f32) * w;
        painter.line_segment(
            [pos2(ls, rect.top()), pos2(ls, rect.bottom())],
            Stroke::new(1.0_f32, loop_color),
        );
        painter.line_segment(
            [pos2(le, rect.top()), pos2(le, rect.bottom())],
            Stroke::new(1.0_f32, loop_color),
        );
    }

    // Preview slice markers (live, from slicing controls)
    if !preview_slices.is_empty() {
        let preview_color = Color32::from_rgb(0, 220, 220);
        for &p in preview_slices {
            let x = rect.left() + (p as f32 / total_len as f32) * w;
            if x >= rect.left() && x <= rect.right() {
                painter.line_segment(
                    [pos2(x, rect.top()), pos2(x, rect.bottom())],
                    Stroke::new(1.0_f32, preview_color),
                );
            }
        }
    }

    // Playback position indicators
    let playhead_selected = Color32::from_rgb(255, 80, 80);
    let playhead_related = Color32::from_rgb(255, 200, 80);
    for &(pos, is_selected) in voice_positions {
        let x = rect.left() + (pos as f32 / total_len as f32) * w;
        if x >= rect.left() && x <= rect.right() {
            let color = if is_selected {
                playhead_selected
            } else {
                playhead_related
            };
            // Vertical playhead line
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(2.0_f32, color),
            );
            // Small triangle at top
            let tri_size = 4.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(x, rect.top()),
                    pos2(x - tri_size, rect.top() - tri_size),
                    pos2(x + tri_size, rect.top() - tri_size),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }

    // Sample info text
    let info = format!(
        "{} | {:.2}s | {} Hz",
        sample.name,
        sample.duration(),
        sample.sample_rate as u32,
    );
    painter.text(
        pos2(rect.left() + 4.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        info,
        egui::FontId::monospace(10.0),
        Color32::from_rgb(160, 160, 180),
    );

    // Voice count
    if !voice_positions.is_empty() {
        let voice_text = format!("{} playing", voice_positions.len());
        painter.text(
            pos2(rect.right() - 4.0, rect.top() + 2.0),
            egui::Align2::RIGHT_TOP,
            voice_text,
            egui::FontId::monospace(10.0),
            playhead_selected,
        );
    }
}

fn level_color(normalized: f32) -> Color32 {
    if normalized < 0.6 {
        let t = normalized / 0.6;
        Color32::from_rgb(
            (40.0 + t * 215.0) as u8,
            (200.0 + t * 55.0) as u8,
            (60.0 * (1.0 - t)) as u8,
        )
    } else {
        let t = (normalized - 0.6) / 0.4;
        Color32::from_rgb(255, (255.0 * (1.0 - t * 0.8)) as u8, 0)
    }
}
