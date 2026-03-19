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
    /// Mini meter embedded in status bar + separator line above.
    #[default]
    StatusBarMeterSep,
    /// Vertical L/R meters + spectrum analyzer.
    VerticalMeterSpectrum,
    /// Sample waveform display.
    SampleView,
}

impl BottomPanel {
    pub fn cycle(self) -> Self {
        match self {
            Self::StatusBarMeterSep => Self::VerticalMeterSpectrum,
            Self::VerticalMeterSpectrum => Self::SampleView,
            Self::SampleView => Self::StatusBarMeterSep,
        }
    }

    /// Height in terminal rows for this panel variant.
    pub fn height(self) -> u16 {
        match self {
            Self::StatusBarMeterSep => 1,
            Self::VerticalMeterSpectrum => 6,
            Self::SampleView => 7,
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

impl Default for VisualizationState {
    fn default() -> Self {
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
            let f_center =
                min_freq * (max_freq / min_freq).powf((bar as f32 + 0.5) / SPECTRUM_BARS as f32);

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
    let bar_chars = [
        ' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
        '\u{2588}',
    ];

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
pub fn build_statusbar_meter_spans(
    vis: &VisualizationState,
    avail_width: usize,
) -> Vec<Span<'static>> {
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

        spans.push(Span::styled(ch.to_string(), Style::default().fg(fg).bg(bg)));
    }

    spans
}
