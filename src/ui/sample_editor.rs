use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::constants::SEMITONES_PER_OCTAVE;

use crate::app::{App, SampleField};

/// Draw the sample editor popup
pub fn draw_sample_editor(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 30, f.area());
    f.render_widget(Clear, area);

    let slot = app.sample_editor_slot;
    let title = format!(" Sample Editor - Slot {:02X} ", slot);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let sample = app.sample_bank.get(slot);

    let mut lines = Vec::new();

    if let Some(sample) = sample {
        // Sample info
        lines.push(Line::from(vec![
            Span::styled("  Name: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&sample.name, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} Hz", sample.sample_rate as u32),
                Style::default().fg(Color::White),
            ),
            Span::styled("  Frames: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", sample.len()),
                Style::default().fg(Color::White),
            ),
            Span::styled("  Duration: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.2}s", sample.duration()),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(""));

        // Editable fields
        let fields: Vec<(SampleField, &str, String)> = vec![
            (SampleField::BaseNote, "Base Note", format!("{} (MIDI {})", note_name(sample.base_note), sample.base_note)),
            (SampleField::TrimStart, "Trim Start", format!("{}", sample.trim_start)),
            (SampleField::TrimEnd, "Trim End", format!("{}", if sample.trim_end == 0 { sample.data.len() } else { sample.trim_end })),
            (SampleField::LoopEnabled, "Loop", format!("{}", if sample.loop_enabled { "ON" } else { "OFF" })),
            (SampleField::LoopStart, "Loop Start", format!("{}", sample.loop_start)),
            (SampleField::LoopEnd, "Loop End", format!("{}", if sample.loop_end == 0 { sample.end() } else { sample.loop_end })),
        ];

        for (field, label, value) in &fields {
            let is_active = *field == app.sample_editor_field;
            let marker = if is_active { "> " } else { "  " };
            let label_style = if is_active {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let value_style = if is_active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, label_style),
                Span::styled(format!("{:<12}", label), label_style),
                Span::styled(value.clone(), value_style),
            ]));
        }

        // Slice section
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  -- Slice --",
            Style::default().fg(Color::DarkGray),
        )));

        let slice_fields: Vec<(SampleField, &str, String)> = vec![
            (SampleField::SliceCount, "Slices", format!("{}", app.sample_slice_count)),
            (SampleField::SliceSensitivity, "Sensitivity", format!("{:.0}%", app.sample_slice_sensitivity * 100.0)),
            (SampleField::SliceEqual, "[Equal]", "Enter to slice".to_string()),
            (SampleField::SliceTransient, "[Transient]", "Enter to slice".to_string()),
        ];

        for (field, label, value) in &slice_fields {
            let is_active = *field == app.sample_editor_field;
            let marker = if is_active { "> " } else { "  " };
            let label_style = if is_active {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Magenta)
            };
            let value_style = if is_active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, label_style),
                Span::styled(format!("{:<14}", label), label_style),
                Span::styled(value.clone(), value_style),
            ]));
        }

        lines.push(Line::from(""));

        // Waveform preview (simple text-based)
        let waveform_width = (inner.width as usize).saturating_sub(4);
        if waveform_width > 10 {
            lines.push(Line::from(Span::styled(
                "  Waveform:",
                Style::default().fg(Color::DarkGray),
            )));
            let preview = render_waveform(sample, waveform_width, 4);
            for row in preview {
                lines.push(Line::from(Span::styled(
                    format!("  {}", row),
                    Style::default().fg(Color::Green),
                )));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Tab: next field  Up/Down: +/-1  Left/Right: +/-10  Esc: close",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No sample loaded in this slot.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Load a sample with: rtrack --load-sample <slot> <file.wav>",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Esc: close",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// Render a text-based waveform preview
fn render_waveform(sample: &crate::sample::Sample, width: usize, height: usize) -> Vec<String> {
    let len = sample.end().saturating_sub(sample.trim_start);
    if len == 0 {
        return vec!["(empty)".to_string()];
    }

    // Downsample: for each column, find the peak value
    let mut peaks = Vec::with_capacity(width);
    for col in 0..width {
        let start = sample.trim_start + (col * len) / width;
        let end = sample.trim_start + ((col + 1) * len) / width;
        let mut peak: f32 = 0.0;
        for i in start..end.min(sample.data.len()) {
            let frame = sample.data[i];
            let mono = (frame[0] + frame[1]) * 0.5;
            peak = peak.max(mono.abs());
        }
        peaks.push(peak);
    }

    // Normalize
    let max_peak = peaks.iter().cloned().fold(0.0f32, f32::max);
    if max_peak > 0.0 {
        for p in &mut peaks {
            *p /= max_peak;
        }
    }

    // Render using block characters
    let blocks = [' ', '.', '-', '=', '#'];
    let mut rows = vec![String::with_capacity(width); height];
    for col in 0..width {
        let level = (peaks[col] * height as f32) as usize;
        for row in 0..height {
            let from_bottom = height - 1 - row;
            if from_bottom < level {
                let intensity = ((peaks[col] * (blocks.len() - 1) as f32) as usize)
                    .min(blocks.len() - 1);
                rows[row].push(blocks[intensity]);
            } else {
                rows[row].push(' ');
            }
        }
    }
    rows
}

fn note_name(note: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = note / SEMITONES_PER_OCTAVE;
    let name = names[(note % SEMITONES_PER_OCTAVE) as usize];
    format!("{}{}", name, octave)
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}
