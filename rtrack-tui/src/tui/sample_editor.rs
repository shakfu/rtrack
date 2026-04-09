use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use rtrack_core::constants::SEMITONES_PER_OCTAVE;
use rtrack_core::sample::{self, Sample};

use crate::app::{App, SampleField};

/// Draw the sample waveform in the bottom panel (non-modal).
/// Shows the sample from the current channel's instrument.
pub fn draw_sample_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 1 || inner.width < 20 {
        return;
    }

    // Find sample slot from current channel's instrument
    let slot = current_sample_slot(app);
    let sample = slot.and_then(|s| app.core.sample_bank.get(s));

    let Some(sample) = sample else {
        let msg = if slot.is_some() {
            "  (no sample loaded)"
        } else {
            "  (no sample assigned to current instrument)"
        };
        let para = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(para, inner);
        return;
    };

    let slot = slot.unwrap();
    let waveform_width = (inner.width as usize).saturating_sub(2);

    // Voice positions for this sample
    let voice_positions: Vec<f64> = app
        .vis
        .voice_snapshots
        .iter()
        .filter(|v| v.sample_index == slot && v.active)
        .map(|v| v.position)
        .collect();

    let height = inner.height as usize;

    // Label line takes 1 row
    let waveform_height = height.saturating_sub(1).max(1);

    let preview = render_waveform_colored(
        sample,
        waveform_width,
        waveform_height,
        &voice_positions,
        &[],
        &[],
    );

    let mut lines = Vec::with_capacity(height);

    // Info line
    let name = if sample.name.len() > 20 {
        &sample.name[..20]
    } else {
        &sample.name
    };
    lines.push(Line::from(vec![
        Span::styled(format!(" {:02X}", slot), Style::default().fg(Color::Yellow)),
        Span::styled(format!(" {} ", name), Style::default().fg(Color::White)),
        Span::styled(
            format!("{:.1}s ", sample.duration()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            note_name(sample.base_note),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    for line in preview.into_iter().take(waveform_height) {
        lines.push(line);
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// Get the sample slot for the current channel's instrument, if any.
fn current_sample_slot(app: &App) -> Option<usize> {
    let ch = app.cursor_channel;
    let instruments = &app.core.instruments;
    if ch < instruments.len() {
        instruments[ch].sample_index
    } else {
        None
    }
}

/// Draw the sample editor popup
pub fn draw_sample_editor(f: &mut Frame, app: &App) {
    let area = centered_rect(65, 24, f.area());
    f.render_widget(Clear, area);

    let slot = app.dialogs.sample_editor_slot;
    let title = format!(" Sample Editor - Slot {:02X} ", slot);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let sample = app.core.sample_bank.get(slot);

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
            (
                SampleField::BaseNote,
                "Base Note",
                format!(
                    "{} (MIDI {})",
                    note_name(sample.base_note),
                    sample.base_note
                ),
            ),
            (
                SampleField::TrimStart,
                "Trim Start",
                format!("{}", sample.trim_start),
            ),
            (
                SampleField::TrimEnd,
                "Trim End",
                format!(
                    "{}",
                    if sample.trim_end == 0 {
                        sample.data.len()
                    } else {
                        sample.trim_end
                    }
                ),
            ),
            (
                SampleField::LoopEnabled,
                "Loop",
                (if sample.loop_enabled { "ON" } else { "OFF" }).to_string(),
            ),
            (
                SampleField::LoopStart,
                "Loop Start",
                format!("{}", sample.loop_start),
            ),
            (
                SampleField::LoopEnd,
                "Loop End",
                format!(
                    "{}",
                    if sample.loop_end == 0 {
                        sample.end()
                    } else {
                        sample.loop_end
                    }
                ),
            ),
        ];

        for (field, label, value) in &fields {
            let is_active = *field == app.dialogs.sample_editor_field;
            let marker = if is_active { "> " } else { "  " };
            let label_style = if is_active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let value_style = if is_active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
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
            (
                SampleField::SliceCount,
                "Slices",
                format!("{}", app.dialogs.sample_slice_count),
            ),
            (
                SampleField::SliceSensitivity,
                "Sensitivity",
                format!("{:.0}%", app.dialogs.sample_slice_sensitivity * 100.0),
            ),
            (
                SampleField::SliceEqual,
                "[Equal]",
                "Enter to slice".to_string(),
            ),
            (
                SampleField::SliceTransient,
                "[Transient]",
                "Enter to slice".to_string(),
            ),
        ];

        for (field, label, value) in &slice_fields {
            let is_active = *field == app.dialogs.sample_editor_field;
            let marker = if is_active { "> " } else { "  " };
            let label_style = if is_active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Magenta)
            };
            let value_style = if is_active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
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

        // Waveform preview with voice positions and slice boundaries
        let waveform_width = (inner.width as usize).saturating_sub(4);
        if waveform_width > 10 {
            lines.push(Line::from(Span::styled(
                "  Waveform:",
                Style::default().fg(Color::DarkGray),
            )));

            // Collect voice positions for this sample slot
            let voice_positions: Vec<f64> = app
                .vis
                .voice_snapshots
                .iter()
                .filter(|v| v.sample_index == slot && v.active)
                .map(|v| v.position)
                .collect();

            // Compute slice boundary preview based on current field focus
            let slice_preview = compute_slice_preview(
                sample,
                &app.dialogs.sample_editor_field,
                app.dialogs.sample_slice_count,
                app.dialogs.sample_slice_sensitivity,
            );

            // Find related slot boundaries (other samples sharing same source path)
            let related_boundaries = find_related_boundaries(app, slot);

            let preview = render_waveform_colored(
                sample,
                waveform_width,
                4,
                &voice_positions,
                &slice_preview,
                &related_boundaries,
            );
            for line in preview {
                lines.push(line);
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

/// Compute slice boundary positions (in frames) based on current editor settings.
fn compute_slice_preview(
    sample: &Sample,
    field: &SampleField,
    count: usize,
    sensitivity: f32,
) -> Vec<usize> {
    // Only show preview when user is focused on slice-related fields
    match field {
        SampleField::SliceCount | SampleField::SliceEqual => {
            if count <= 1 {
                return Vec::new();
            }
            let total = sample.data.len();
            (1..count).map(|i| i * total / count).collect()
        }
        SampleField::SliceSensitivity | SampleField::SliceTransient => {
            sample::detect_transients(sample, sensitivity)
        }
        _ => Vec::new(),
    }
}

/// Find trim boundaries of related samples (same source file) for visual markers.
fn find_related_boundaries(app: &App, slot: usize) -> Vec<usize> {
    let bank = &app.core.sample_bank;
    let source = match bank.get(slot).and_then(|s| s.source_path.as_ref()) {
        Some(s) => s.clone(),
        None => return Vec::new(),
    };
    if source.is_empty() {
        return Vec::new();
    }

    let total_len = match bank.get(slot) {
        Some(s) => s.data.len(),
        None => return Vec::new(),
    };

    let mut boundaries = Vec::new();
    for i in 0..bank.samples.len() {
        if i == slot {
            continue;
        }
        if let Some(other) = bank.get(i) {
            if other.source_path.as_deref() == Some(source.as_str())
                && other.data.len() == total_len
            {
                if other.trim_start > 0 {
                    boundaries.push(other.trim_start);
                }
                if other.trim_end > 0 && other.trim_end < total_len {
                    boundaries.push(other.trim_end);
                }
            }
        }
    }
    boundaries.sort();
    boundaries.dedup();
    boundaries
}

/// Render a colored waveform with voice playheads, slice previews, and related boundaries.
fn render_waveform_colored(
    sample: &Sample,
    width: usize,
    height: usize,
    voice_positions: &[f64],
    slice_preview: &[usize],
    related_boundaries: &[usize],
) -> Vec<Line<'static>> {
    let total_len = sample.data.len();
    if total_len == 0 {
        return vec![Line::from(Span::styled(
            "  (empty)",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    // Compute per-column peaks over the full sample data
    let mut peaks = Vec::with_capacity(width);
    for col in 0..width {
        let start = col * total_len / width;
        let end = ((col + 1) * total_len / width).min(total_len);
        let mut peak: f32 = 0.0;
        for i in start..end {
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

    // Determine column markers
    let trim_start_col = sample.trim_start * width / total_len;
    let trim_end_col = if sample.trim_end == 0 || sample.trim_end >= total_len {
        width
    } else {
        sample.trim_end * width / total_len
    };

    let loop_start_col = if sample.loop_enabled {
        Some(sample.loop_start * width / total_len)
    } else {
        None
    };
    let loop_end_col = if sample.loop_enabled {
        let le = if sample.loop_end == 0 {
            sample.end()
        } else {
            sample.loop_end
        };
        Some(le * width / total_len)
    } else {
        None
    };

    // Voice playhead columns
    let voice_cols: Vec<usize> = voice_positions
        .iter()
        .map(|&pos| ((pos as usize) * width / total_len).min(width.saturating_sub(1)))
        .collect();

    // Slice preview columns
    let slice_cols: Vec<usize> = slice_preview
        .iter()
        .map(|&pos| (pos * width / total_len).min(width.saturating_sub(1)))
        .collect();

    // Related boundary columns
    let boundary_cols: Vec<usize> = related_boundaries
        .iter()
        .map(|&pos| (pos * width / total_len).min(width.saturating_sub(1)))
        .collect();

    // Render rows
    let blocks = [' ', '.', '-', '=', '#'];
    let mut result = Vec::with_capacity(height);

    for row in 0..height {
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::raw("  ".to_string()));

        for (col, &peak) in peaks.iter().enumerate().take(width) {
            let from_bottom = height - 1 - row;
            let level = (peak * height as f32) as usize;

            // Determine character
            let ch = if voice_cols.contains(&col) {
                // Voice playhead: always visible
                '|'
            } else if slice_cols.contains(&col) {
                // Slice preview marker
                ':'
            } else if boundary_cols.contains(&col) {
                // Related slice boundary
                '|'
            } else if Some(col) == loop_start_col || Some(col) == loop_end_col {
                // Loop markers
                '|'
            } else if from_bottom < level {
                let intensity = ((peak * (blocks.len() - 1) as f32) as usize).min(blocks.len() - 1);
                blocks[intensity]
            } else {
                ' '
            };

            // Determine color
            let color = if voice_cols.contains(&col) {
                Color::Red
            } else if slice_cols.contains(&col) {
                Color::Cyan
            } else if boundary_cols.contains(&col) {
                Color::Yellow
            } else if Some(col) == loop_start_col || Some(col) == loop_end_col {
                Color::LightGreen
            } else if col < trim_start_col || col >= trim_end_col {
                // Outside trim range: dimmed
                Color::DarkGray
            } else {
                Color::Green
            };

            spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }

        result.push(Line::from(spans));
    }

    // Legend line
    let mut legend_spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
    if !voice_cols.is_empty() {
        legend_spans.push(Span::styled("|", Style::default().fg(Color::Red)));
        legend_spans.push(Span::styled("play ", Style::default().fg(Color::DarkGray)));
    }
    if !slice_cols.is_empty() {
        legend_spans.push(Span::styled(":", Style::default().fg(Color::Cyan)));
        legend_spans.push(Span::styled("slice ", Style::default().fg(Color::DarkGray)));
    }
    if !boundary_cols.is_empty() {
        legend_spans.push(Span::styled("|", Style::default().fg(Color::Yellow)));
        legend_spans.push(Span::styled(
            "boundary ",
            Style::default().fg(Color::DarkGray),
        ));
    }
    if loop_start_col.is_some() {
        legend_spans.push(Span::styled("|", Style::default().fg(Color::LightGreen)));
        legend_spans.push(Span::styled("loop ", Style::default().fg(Color::DarkGray)));
    }
    if legend_spans.len() > 1 {
        result.push(Line::from(legend_spans));
    }

    result
}

fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
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
