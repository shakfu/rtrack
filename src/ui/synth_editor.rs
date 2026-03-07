use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, SynthField};
use crate::audio::synth::{FilterType, Patch};

/// Draw the synth editor popup
pub fn draw_synth_editor(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 24, f.area());
    f.render_widget(Clear, area);

    let slot = app.synth_editor_slot;
    let inst_name = if app.instruments[slot].name.is_empty() {
        "---"
    } else {
        &app.instruments[slot].name
    };
    let title = format!(" Synth Editor - {:02X} {} ", slot, inst_name);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();

    if let Some(ref params) = app.instruments[slot].synth_params {
        let patch = Patch::from_program(params.waveform);

        let filter_type_str = match params.filter_type {
            FilterType::LowPass => "LP",
            FilterType::HighPass => "HP",
            FilterType::BandPass => "BP",
        };
        let fields: Vec<(SynthField, &str, String)> = vec![
            (SynthField::Waveform, "Waveform", format!("{} ({})", params.waveform, patch.name())),
            (SynthField::Attack, "Attack", format!("{:.3}s", params.attack)),
            (SynthField::Decay, "Decay", format!("{:.3}s", params.decay)),
            (SynthField::Sustain, "Sustain", format!("{:.2}", params.sustain)),
            (SynthField::Release, "Release", format!("{:.3}s", params.release)),
            (SynthField::FilterType, "Filter Type", filter_type_str.to_string()),
            (SynthField::FilterCutoff, "Filter Cut", format!("{:.1}x", params.filter_cutoff)),
            (SynthField::FilterResonance, "Filter Res", format!("{:.2}", params.filter_resonance)),
            (SynthField::FilterEnv, "Filter Env", format!("{:.1} oct", params.filter_env)),
            (SynthField::Detune, "Detune", format!("{:.1} cents", params.detune)),
            (SynthField::SubOsc, "Sub Osc", format!("{:.2}", params.sub_osc)),
            (SynthField::FmRatio, "FM Ratio", format!("{:.1}", params.fm_ratio)),
            (SynthField::FmIndex, "FM Index", format!("{:.1}", params.fm_index)),
            (SynthField::PulseWidth, "Pulse Width", format!("{:.2}", params.pulse_width)),
        ];

        lines.push(Line::from(""));
        for (field, label, value) in &fields {
            let is_active = *field == app.synth_editor_field;
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
                Span::styled(format!("{:<14}", label), label_style),
                Span::styled(value.clone(), value_style),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Tab: next  Up/Down: +/-1  Left/Right: +/-10  Del: clear  Esc: close",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No synth params configured.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Esc: close",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
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
