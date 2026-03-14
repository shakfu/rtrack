use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use rtrack_core::audio::{AudioEngine, VoiceSnapshot};

const METER_SMOOTHING: f32 = 0.8;
const PEAK_DECAY: f32 = 0.02;
const SPECTRUM_BARS: usize = 32;
const SPECTRUM_SMOOTHING: f32 = 0.7;
const FFT_SIZE: usize = 1024;

/// Which view the bottom panel displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomPanel {
    #[default]
    HorizontalMeter,
    /// Borderless single-line L/R meters side by side.
    CompactMeter,
    /// Half-block: L on upper half, R on lower half of each character cell.
    MiniMeter,
    VerticalMeterSpectrum,
    SampleView,
    /// Separator line with a small mini meter in the right corner.
    SeparatorMini,
    /// Mini meter embedded in the status bar (no panel row).
    StatusBarMeter,
    /// Same as StatusBarMeter but with a separator line above.
    StatusBarMeterSep,
    /// Just a separator line, no meters.
    SeparatorOnly,
    /// No visualization panel at all.
    None,
}

impl BottomPanel {
    pub fn cycle(self) -> Self {
        match self {
            Self::HorizontalMeter => Self::CompactMeter,
            Self::CompactMeter => Self::MiniMeter,
            Self::MiniMeter => Self::VerticalMeterSpectrum,
            Self::VerticalMeterSpectrum => Self::SampleView,
            Self::SampleView => Self::SeparatorMini,
            Self::SeparatorMini => Self::StatusBarMeter,
            Self::StatusBarMeter => Self::StatusBarMeterSep,
            Self::StatusBarMeterSep => Self::SeparatorOnly,
            Self::SeparatorOnly => Self::None,
            Self::None => Self::HorizontalMeter,
        }
    }

    /// Height in terminal rows for this panel variant.
    pub fn height(self) -> u16 {
        match self {
            Self::HorizontalMeter => 2,
            Self::CompactMeter => 1,
            Self::MiniMeter => 1,
            Self::VerticalMeterSpectrum => 6,
            Self::SampleView => 7,
            Self::SeparatorMini => 1,
            Self::StatusBarMeter => 0,
            Self::StatusBarMeterSep => 1,
            Self::SeparatorOnly => 1,
            Self::None => 0,
        }
    }
}

/// Persistent visualization state, updated each frame from the audio engine.
pub struct VisualizationState {
    // Level meters
    pub meter_l: f32,
    pub meter_r: f32,
    pub peak_hold_l: f32,
    pub peak_hold_r: f32,

    // Spectrum
    pub spectrum: Vec<f32>,
    sample_buf: Vec<f32>,

    // Voice snapshots
    pub voice_snapshots: Vec<VoiceSnapshot>,

    // Panel mode
    pub panel: BottomPanel,
}

impl VisualizationState {
    pub fn new() -> Self {
        Self {
            meter_l: 0.0,
            meter_r: 0.0,
            peak_hold_l: 0.0,
            peak_hold_r: 0.0,
            spectrum: vec![0.0; SPECTRUM_BARS],
            sample_buf: Vec::with_capacity(FFT_SIZE * 2),
            voice_snapshots: Vec::new(),
            panel: BottomPanel::default(),
        }
    }

    /// Call each frame to drain audio data and update meters/spectrum.
    pub fn update(&mut self, audio: &mut Option<AudioEngine>) {
        let Some(audio) = audio.as_mut() else {
            return;
        };

        // Peak levels
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

        // Drain visualization samples
        audio.read_vis_samples(&mut self.sample_buf);

        // Compute spectrum when we have enough samples
        if self.sample_buf.len() >= FFT_SIZE {
            self.compute_spectrum();
            // Keep a sliding window
            let drain = self.sample_buf.len() - FFT_SIZE / 4;
            self.sample_buf.drain(..drain);
        }

        // Voice snapshots
        self.voice_snapshots = audio.voice_snapshots();
    }

    fn compute_spectrum(&mut self) {
        // Fast band-energy spectrum: use Goertzel algorithm (O(N) per bin, no full DFT).
        let n = FFT_SIZE.min(self.sample_buf.len());
        let samples = &self.sample_buf[self.sample_buf.len() - n..];

        let sample_rate = 48000.0_f32;
        let min_freq = 60.0_f32;
        let max_freq = 16000.0_f32.min(sample_rate / 2.0);

        for bar in 0..SPECTRUM_BARS {
            // Log-spaced center frequency for this bar
            let f_center = min_freq
                * (max_freq / min_freq).powf((bar as f32 + 0.5) / SPECTRUM_BARS as f32);

            // Goertzel algorithm: O(N) per frequency bin
            let k = (f_center / sample_rate * n as f32).round();
            let w = std::f32::consts::TAU * k / n as f32;
            let coeff = 2.0 * w.cos();

            let mut s0: f32 = 0.0;
            let mut s1: f32 = 0.0;
            let mut s2: f32;

            for &sample in samples {
                s2 = s1;
                s1 = s0;
                s0 = sample + coeff * s1 - s2;
            }

            let mag = (s0 * s0 + s1 * s1 - coeff * s0 * s1).max(0.0).sqrt();
            let normalized_mag = mag / (n as f32 * 0.5);

            // dB scale
            let db = (20.0 * (normalized_mag + 1e-10).log10()).max(-60.0);
            let normalized = ((db + 60.0) / 60.0).clamp(0.0, 1.0);

            // Exponential smoothing
            self.spectrum[bar] =
                self.spectrum[bar] * SPECTRUM_SMOOTHING + normalized * (1.0 - SPECTRUM_SMOOTHING);
        }
    }
}

/// Draw the visualization panel (level meters + spectrum bars) in the given area.
pub fn draw_visualization(f: &mut Frame, vis: &VisualizationState, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 1 || inner.width < 20 {
        return;
    }

    // Layout: [L meter][R meter] [spectrum bars]
    let meter_width = 3; // "L " + bar character + space
    let spectrum_start = meter_width * 2 + 1;
    let spectrum_width = (inner.width as usize).saturating_sub(spectrum_start + 1);

    let height = inner.height as usize;

    // Build lines
    let mut lines: Vec<Line> = Vec::with_capacity(height);

    // Block characters for vertical bars (from empty to full)
    let bar_chars = [' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];

    // Convert meter levels from linear amplitude to dB-normalized (0.0-1.0)
    // so that typical audio levels are visible on the meter
    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);
    let peak_l_db = amp_to_db_normalized(vis.peak_hold_l);
    let peak_r_db = amp_to_db_normalized(vis.peak_hold_r);

    for row in 0..height {
        let mut spans = Vec::new();
        let from_top = row as f32 / height as f32;
        let threshold = 1.0 - from_top;

        // L meter
        let l_char = if meter_l_db >= threshold {
            '\u{2588}'
        } else if peak_l_db >= threshold && peak_l_db < threshold + (1.0 / height as f32) {
            '\u{2584}'
        } else {
            ' '
        };
        let l_color = meter_color(threshold);
        spans.push(Span::styled("L", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            l_char.to_string(),
            Style::default().fg(l_color),
        ));
        spans.push(Span::raw(" "));

        // R meter
        let r_char = if meter_r_db >= threshold {
            '\u{2588}'
        } else if peak_r_db >= threshold && peak_r_db < threshold + (1.0 / height as f32) {
            '\u{2584}'
        } else {
            ' '
        };
        let r_color = meter_color(threshold);
        spans.push(Span::styled("R", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            r_char.to_string(),
            Style::default().fg(r_color),
        ));
        spans.push(Span::raw(" "));

        // Spectrum bars
        let mut spec_str = String::with_capacity(spectrum_width);
        let bars_to_show = spectrum_width.min(SPECTRUM_BARS);
        for bar_idx in 0..bars_to_show {
            // Map bar_idx to spectrum index
            let spec_idx = bar_idx * SPECTRUM_BARS / bars_to_show;
            let level = vis.spectrum[spec_idx.min(SPECTRUM_BARS - 1)];

            if level >= threshold {
                // Full block
                spec_str.push('\u{2588}');
            } else if level >= threshold - (1.0 / height as f32) && level > 0.01 {
                // Partial block
                let frac = (level - (threshold - 1.0 / height as f32)) * height as f32;
                let idx = (frac * (bar_chars.len() - 1) as f32) as usize;
                spec_str.push(bar_chars[idx.min(bar_chars.len() - 1)]);
            } else {
                spec_str.push(' ');
            }
        }
        // Pad remaining width
        while spec_str.len() < spectrum_width {
            spec_str.push(' ');
        }

        let spec_color = spectrum_color(threshold);
        spans.push(Span::styled(spec_str, Style::default().fg(spec_color)));

        lines.push(Line::from(spans));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// Draw a compact horizontal L/R meter (single line inside a top-border block).
pub fn draw_horizontal_meter(f: &mut Frame, vis: &VisualizationState, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 1 || inner.width < 14 {
        return;
    }

    // Layout: "L [bar] R [bar]" -- split width evenly between L and R
    let gap = 1; // space between L and R sections
    let label_width = 2; // "L " or "R "
    let total_labels = label_width * 2 + gap; // "L " + "R " + gap
    let total_bar = (inner.width as usize).saturating_sub(total_labels);
    let bar_l_width = total_bar / 2;
    let bar_r_width = total_bar - bar_l_width;

    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);
    let peak_l_db = amp_to_db_normalized(vis.peak_hold_l);
    let peak_r_db = amp_to_db_normalized(vis.peak_hold_r);

    let mut spans = Vec::new();

    for (label, level, peak, bar_width) in [
        ("L ", meter_l_db, peak_l_db, bar_l_width),
        ("R ", meter_r_db, peak_r_db, bar_r_width),
    ] {
        let filled = (level * bar_width as f32) as usize;
        let peak_col = (peak * bar_width as f32) as usize;

        spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));

        let mut bar = String::with_capacity(bar_width);
        for col in 0..bar_width {
            if col == peak_col && peak_col > filled {
                bar.push('\u{2502}');
            } else if col < filled {
                bar.push('\u{2588}');
            } else {
                bar.push(' ');
            }
        }

        // Split bar into colored segments: green | yellow | red
        let green_end = (0.7 * bar_width as f32) as usize;
        let yellow_end = (0.85 * bar_width as f32) as usize;

        let green_part: String = bar.chars().take(green_end).collect();
        let yellow_part: String = bar.chars().skip(green_end).take(yellow_end - green_end).collect();
        let red_part: String = bar.chars().skip(yellow_end).collect();

        spans.push(Span::styled(green_part, Style::default().fg(Color::Green)));
        spans.push(Span::styled(yellow_part, Style::default().fg(Color::Yellow)));
        spans.push(Span::styled(red_part, Style::default().fg(Color::Red)));

        // Add gap between L and R (not after R)
        if label == "L " {
            spans.push(Span::raw(" "));
        }
    }

    let line = Line::from(spans);
    let para = Paragraph::new(vec![line]);
    f.render_widget(para, inner);
}

/// Convert linear amplitude to a 0.0-1.0 dB-normalized value (60 dB range).
fn amp_to_db_normalized(amp: f32) -> f32 {
    let db = (20.0 * (amp + 1e-10).log10()).max(-60.0);
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn meter_color(threshold: f32) -> Color {
    if threshold > 0.85 {
        Color::Red
    } else if threshold > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn spectrum_color(threshold: f32) -> Color {
    if threshold > 0.85 {
        Color::Red
    } else if threshold > 0.6 {
        Color::Yellow
    } else if threshold > 0.3 {
        Color::Cyan
    } else {
        Color::Blue
    }
}

/// Just a horizontal line separator (height 1).
pub fn draw_separator(f: &mut Frame, area: Rect) {
    if area.height < 1 {
        return;
    }
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(block, area);
}

/// Build mini meter spans for embedding in the status bar.
/// Returns spans that can be appended to a status bar Line.
pub fn build_statusbar_meter_spans(vis: &VisualizationState, avail_width: usize) -> Vec<Span<'static>> {
    let bar_width = avail_width.saturating_sub(4).min(24); // "LR " + meter chars
    if bar_width == 0 {
        return Vec::new();
    }

    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);
    let filled_l = (meter_l_db * bar_width as f32) as usize;
    let filled_r = (meter_r_db * bar_width as f32) as usize;

    let mut spans = Vec::new();
    spans.push(Span::styled(" LR ", Style::default().fg(Color::DarkGray)));

    for col in 0..bar_width {
        let frac = col as f32 / bar_width as f32;
        let zone_color = if frac > 0.85 {
            Color::Red
        } else if frac > 0.7 {
            Color::Yellow
        } else {
            Color::Green
        };

        let l_on = col < filled_l;
        let r_on = col < filled_r;

        let (ch, fg, bg) = match (l_on, r_on) {
            (true, true) => ('\u{2580}', zone_color, zone_color),
            (true, false) => ('\u{2580}', zone_color, Color::Black),
            (false, true) => ('\u{2584}', zone_color, Color::Reset),
            (false, false) => (' ', Color::Reset, Color::Reset),
        };

        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(fg).bg(bg),
        ));
    }

    spans
}

/// Separator line with a small half-block mini meter in the right corner (height 1).
pub fn draw_separator_mini(f: &mut Frame, vis: &VisualizationState, area: Rect) {
    if area.height < 1 || area.width < 10 {
        return;
    }

    let meter_width: u16 = 20.min(area.width / 3); // meter takes up to 1/3 of width
    let sep_width = area.width.saturating_sub(meter_width);

    // Draw separator on the left portion using horizontal line chars
    let sep_str: String = "\u{2500}".repeat(sep_width as usize);
    let sep_span = Span::styled(sep_str, Style::default().fg(Color::DarkGray));

    // Build mini meter spans for the right portion
    let bar_width = (meter_width as usize).saturating_sub(3); // "LR " label
    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);
    let filled_l = (meter_l_db * bar_width as f32) as usize;
    let filled_r = (meter_r_db * bar_width as f32) as usize;

    let mut spans = vec![sep_span];
    spans.push(Span::styled("LR", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" "));

    for col in 0..bar_width {
        let frac = col as f32 / bar_width as f32;
        let zone_color = if frac > 0.85 {
            Color::Red
        } else if frac > 0.7 {
            Color::Yellow
        } else {
            Color::Green
        };

        let l_on = col < filled_l;
        let r_on = col < filled_r;

        let (ch, fg, bg) = match (l_on, r_on) {
            (true, true) => ('\u{2580}', zone_color, zone_color),
            (true, false) => ('\u{2580}', zone_color, Color::Black),
            (false, true) => ('\u{2584}', zone_color, Color::Reset),
            (false, false) => (' ', Color::Reset, Color::Reset),
        };

        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(fg).bg(bg),
        ));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(vec![line]);
    f.render_widget(para, area);
}

/// Borderless single-line L/R meter (height 1, no border).
/// Same layout as HorizontalMeter but without the top border.
pub fn draw_compact_meter(f: &mut Frame, vis: &VisualizationState, area: Rect) {
    if area.height < 1 || area.width < 14 {
        return;
    }

    let width = area.width as usize;
    let gap = 1;
    let label_width = 2; // "L " or "R "
    let total_labels = label_width * 2 + gap;
    let total_bar = width.saturating_sub(total_labels);
    let bar_l_width = total_bar / 2;
    let bar_r_width = total_bar - bar_l_width;

    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);
    let peak_l_db = amp_to_db_normalized(vis.peak_hold_l);
    let peak_r_db = amp_to_db_normalized(vis.peak_hold_r);

    let mut spans = Vec::new();

    for (label, level, peak, bar_width) in [
        ("L ", meter_l_db, peak_l_db, bar_l_width),
        ("R ", meter_r_db, peak_r_db, bar_r_width),
    ] {
        let filled = (level * bar_width as f32) as usize;
        let peak_col = (peak * bar_width as f32) as usize;

        spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));

        let mut bar = String::with_capacity(bar_width);
        for col in 0..bar_width {
            if col == peak_col && peak_col > filled {
                bar.push('\u{2502}');
            } else if col < filled {
                bar.push('\u{2588}');
            } else {
                bar.push(' ');
            }
        }

        let green_end = (0.7 * bar_width as f32) as usize;
        let yellow_end = (0.85 * bar_width as f32) as usize;

        let green_part: String = bar.chars().take(green_end).collect();
        let yellow_part: String = bar.chars().skip(green_end).take(yellow_end - green_end).collect();
        let red_part: String = bar.chars().skip(yellow_end).collect();

        spans.push(Span::styled(green_part, Style::default().fg(Color::Green)));
        spans.push(Span::styled(yellow_part, Style::default().fg(Color::Yellow)));
        spans.push(Span::styled(red_part, Style::default().fg(Color::Red)));

        if label == "L " {
            spans.push(Span::raw(" "));
        }
    }

    let line = Line::from(spans);
    let para = Paragraph::new(vec![line]);
    f.render_widget(para, area);
}

/// Half-block stereo meter: L in upper half, R in lower half of each cell (height 1).
/// Uses upper-half block (\u{2580}) with fg=L color, bg=R color.
pub fn draw_mini_meter(f: &mut Frame, vis: &VisualizationState, area: Rect) {
    if area.height < 1 || area.width < 8 {
        return;
    }

    let bar_width = (area.width as usize).saturating_sub(4); // "LR " prefix + trailing

    let meter_l_db = amp_to_db_normalized(vis.meter_l);
    let meter_r_db = amp_to_db_normalized(vis.meter_r);

    let filled_l = (meter_l_db * bar_width as f32) as usize;
    let filled_r = (meter_r_db * bar_width as f32) as usize;

    let mut spans = Vec::new();
    spans.push(Span::styled("LR ", Style::default().fg(Color::DarkGray)));

    // Each column: upper half = L, lower half = R
    // \u{2580} = upper half block (fg color on top, bg color on bottom)
    for col in 0..bar_width {
        let frac = col as f32 / bar_width as f32;
        let zone_color = if frac > 0.85 {
            Color::Red
        } else if frac > 0.7 {
            Color::Yellow
        } else {
            Color::Green
        };
        let l_on = col < filled_l;
        let r_on = col < filled_r;

        let (ch, fg, bg) = match (l_on, r_on) {
            (true, true) => ('\u{2580}', zone_color, zone_color),
            (true, false) => ('\u{2580}', zone_color, Color::Black),
            (false, true) => ('\u{2584}', zone_color, Color::Reset),
            (false, false) => (' ', Color::Reset, Color::Reset),
        };

        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(fg).bg(bg),
        ));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(vec![line]);
    f.render_widget(para, area);
}
