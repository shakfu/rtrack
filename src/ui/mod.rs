pub mod pattern_editor;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{App, Mode};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),  // pattern editor
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    pattern_editor::draw(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);

    match app.mode {
        Mode::MidiPortSelect => draw_port_selector(f, app),
        Mode::Help => draw_help(f),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let song = &app.song;
    let pattern_idx = app.current_order_position();
    let pattern_num = song.order.get(pattern_idx).copied().unwrap_or(0);

    let header_text = vec![Line::from(vec![
        Span::styled(" rtrack ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" | "),
        Span::styled(&song.title, Style::default().fg(Color::White)),
        Span::raw(" | BPM: "),
        Span::styled(format!("{}", song.bpm), Style::default().fg(Color::Yellow)),
        Span::raw(" SPD: "),
        Span::styled(format!("{}", song.speed), Style::default().fg(Color::Yellow)),
        Span::raw(" | Pat: "),
        Span::styled(
            format!("{:02X}/{:02X}", pattern_num, song.current_pattern_count() - 1),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" Ord: "),
        Span::styled(
            format!("{:02X}/{:02X}", pattern_idx, song.order.len().saturating_sub(1)),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" Oct: "),
        Span::styled(
            format!("{}", app.current_octave),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw(if app.is_playing() { " [PLAYING]" } else { " [STOPPED]" }),
    ])];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(header, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let midi_status = if app.midi_connected() {
        Span::styled(
            format!(" MIDI:{} ", app.midi_port_display_name()),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled(" MIDI:-- ", Style::default().fg(Color::DarkGray))
    };

    let mode_span = match app.mode {
        Mode::Normal => Span::styled(" NORMAL ", Style::default().fg(Color::Blue)),
        Mode::Insert => Span::styled(" INSERT ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Mode::MidiPortSelect => Span::styled(" MIDI SELECT ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Mode::Help => Span::styled(" HELP ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    };

    let help = Span::styled(
        " [F1]Help [Space]Play/Stop [Esc]Mode [Tab/S-Tab]Track [F2]MIDI [+/-]Oct [q]Quit ",
        Style::default().fg(Color::DarkGray),
    );

    let status = Paragraph::new(Line::from(vec![mode_span, midi_status, help]));
    f.render_widget(status, area);
}

fn draw_help(f: &mut Frame) {
    let help_lines = vec![
        Line::from(Span::styled("  Global Keys", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  F1           ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle help"),
        ]),
        Line::from(vec![
            Span::styled("  F2           ", Style::default().fg(Color::Yellow)),
            Span::raw("MIDI port selector"),
        ]),
        Line::from(vec![
            Span::styled("  Space        ", Style::default().fg(Color::Yellow)),
            Span::raw("Play / Stop"),
        ]),
        Line::from(vec![
            Span::styled("  Esc          ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle Normal / Insert mode"),
        ]),
        Line::from(vec![
            Span::styled("  Tab/S-Tab    ", Style::default().fg(Color::Yellow)),
            Span::raw("Next / previous track"),
        ]),
        Line::from(vec![
            Span::styled("  Arrows       ", Style::default().fg(Color::Yellow)),
            Span::raw("Move cursor"),
        ]),
        Line::from(vec![
            Span::styled("  PgUp/PgDn    ", Style::default().fg(Color::Yellow)),
            Span::raw("Move cursor 16 rows"),
        ]),
        Line::from(vec![
            Span::styled("  Home/End     ", Style::default().fg(Color::Yellow)),
            Span::raw("Jump to first / last row"),
        ]),
        Line::from(vec![
            Span::styled("  + / -        ", Style::default().fg(Color::Yellow)),
            Span::raw("Octave up / down"),
        ]),
        Line::from(vec![
            Span::styled("  [ / ]        ", Style::default().fg(Color::Yellow)),
            Span::raw("BPM down / up"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q            ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Insert Mode", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  z s x d c .. ", Style::default().fg(Color::Yellow)),
            Span::raw("Notes C C# D D# E .. (lower octave)"),
        ]),
        Line::from(vec![
            Span::styled("  q 2 w 3 e .. ", Style::default().fg(Color::Yellow)),
            Span::raw("Notes C C# D D# E .. (upper octave)"),
        ]),
        Line::from(vec![
            Span::styled("  0-9 / a-f    ", Style::default().fg(Color::Yellow)),
            Span::raw("Hex entry (inst/vol/fx columns)"),
        ]),
        Line::from(vec![
            Span::styled("  Del/Bksp     ", Style::default().fg(Color::Yellow)),
            Span::raw("Clear sub-column at cursor"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+1       ", Style::default().fg(Color::Yellow)),
            Span::raw("Enter note-off (===)"),
        ]),
        Line::from(vec![
            Span::styled("  Esc          ", Style::default().fg(Color::Yellow)),
            Span::raw("Return to Normal mode"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Press Esc or F1 to close", Style::default().fg(Color::DarkGray))),
    ];

    let area = f.area();
    let popup_height = (help_lines.len() as u16 + 2).min(area.height.saturating_sub(4));
    let popup_width = 52u16.min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let help = Paragraph::new(help_lines).block(
        Block::default()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(help, popup_area);
}

fn draw_port_selector(f: &mut Frame, app: &App) {
    let area = f.area();

    // Center a popup
    let popup_width = 50u16.min(area.width.saturating_sub(4));
    let popup_height = (app.midi_port_list.len() as u16 + 2).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = app
        .midi_port_list
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == app.midi_port_cursor {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if app.midi.port_name.as_deref() == Some(name) {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!(" {} ", name)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" MIDI Output Port [Enter=Select, Esc=Cancel] ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(list, popup_area);
}
