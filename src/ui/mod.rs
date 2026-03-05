pub mod pattern_editor;
pub mod theme;

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
            Constraint::Min(10),  // main area (order sidebar + pattern editor)
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);

    // Split main area: order sidebar + pattern editor
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(7), // order list sidebar
            Constraint::Min(20),  // pattern editor
        ])
        .split(chunks[1]);

    draw_order_sidebar(f, app, main_chunks[0]);
    pattern_editor::draw(f, app, main_chunks[1]);
    draw_status_bar(f, app, chunks[2]);

    match app.mode {
        Mode::MidiPortSelect => draw_port_selector(f, app),
        Mode::Help => draw_help(f),
        Mode::SongSettings => draw_song_settings(f, app),
        Mode::InstrumentList => draw_instrument_list(f, app),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let song = &app.song;
    let pattern_idx = app.current_order_position();
    let pattern_num = song.order.get(pattern_idx).copied().unwrap_or(0);

    let link_span = if app.link.is_enabled() {
        Span::styled(
            format!(" L:{}", app.link.num_peers()),
            Style::default().fg(Color::Rgb(255, 100, 0)),
        )
    } else {
        Span::styled(" L:--", Style::default().fg(Color::DarkGray))
    };

    let header_text = vec![Line::from(vec![
        Span::styled(" rtrack", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(&song.title, Style::default().fg(Color::White)),
        Span::styled(
            format!(" {}bpm s{}", song.bpm, song.speed),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!(" P:{:02X}/{:02X} O:{:02X}/{:02X}",
                pattern_num, song.current_pattern_count() - 1,
                pattern_idx, song.order.len().saturating_sub(1)),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!(" Oct:{}", app.current_octave),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(
            format!(" Stp:{}", app.edit_step),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw(if app.is_playing() { " PLAY" } else { " STOP" }),
        link_span,
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
        Mode::SongSettings => Span::styled(" SETTINGS ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Mode::InstrumentList => Span::styled(" INSTRUMENTS ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    };

    let audio_span = if app.has_audio() {
        Span::styled(" SF2 ", Style::default().fg(Color::Magenta))
    } else {
        Span::from("")
    };

    let msg_span = if let Some(ref msg) = app.status_message {
        Span::styled(format!(" {} ", msg), Style::default().fg(Color::White))
    } else {
        Span::styled(
            " [F1]Help [Space]Play/Stop [Esc]Mode [Tab/S-Tab]Track [F2]MIDI [F3]Link [+/-]Oct [q]Quit ",
            Style::default().fg(Color::DarkGray),
        )
    };

    let status = Paragraph::new(Line::from(vec![mode_span, midi_status, audio_span, msg_span]));
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
            Span::styled("  F3           ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle Ableton Link"),
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
        Line::from(vec![
            Span::styled("  Ctrl+S       ", Style::default().fg(Color::Yellow)),
            Span::raw("Save"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+Z / Y   ", Style::default().fg(Color::Yellow)),
            Span::raw("Undo / Redo"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+C/X/V   ", Style::default().fg(Color::Yellow)),
            Span::raw("Copy / Cut / Paste row"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+L/R     ", Style::default().fg(Color::Yellow)),
            Span::raw("Next / prev order position"),
        ]),
        Line::from(vec![
            Span::styled("  F9-F12       ", Style::default().fg(Color::Yellow)),
            Span::raw("Mute/unmute ch 1-4"),
        ]),
        Line::from(vec![
            Span::styled("  ( / )        ", Style::default().fg(Color::Yellow)),
            Span::raw("Edit step down / up"),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q            ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+N       ", Style::default().fg(Color::Yellow)),
            Span::raw("New pattern (append to order)"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+D       ", Style::default().fg(Color::Yellow)),
            Span::raw("Clone current pattern"),
        ]),
        Line::from(vec![
            Span::styled("  F4 / F5      ", Style::default().fg(Color::Yellow)),
            Span::raw("Insert / remove order entry"),
        ]),
        Line::from(vec![
            Span::styled("  Ins / Bksp   ", Style::default().fg(Color::Yellow)),
            Span::raw("Insert / delete row in pattern"),
        ]),
        Line::from(vec![
            Span::styled("  F6           ", Style::default().fg(Color::Yellow)),
            Span::raw("Song settings dialog"),
        ]),
        Line::from(vec![
            Span::styled("  F7           ", Style::default().fg(Color::Yellow)),
            Span::raw("Instrument list"),
        ]),
        Line::from(vec![
            Span::styled("  F8           ", Style::default().fg(Color::Yellow)),
            Span::raw("Cycle color theme"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+E       ", Style::default().fg(Color::Yellow)),
            Span::raw("Export to MIDI file (.mid)"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+M       ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle MIDI clock output"),
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

fn draw_order_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let current_pos = app.current_order_position();
    let visible = area.height.saturating_sub(2) as usize;

    // Center on current position
    let start = if current_pos > visible / 2 {
        current_pos - visible / 2
    } else {
        0
    };

    let mut lines = Vec::new();
    for i in start..app.song.order.len().min(start + visible) {
        let pat = app.song.order[i];
        let text = format!("{:02X}:{:02X}", i, pat);
        let style = if i == current_pos {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    let block = Block::default()
        .title(" Ord ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn draw_song_settings(f: &mut Frame, app: &App) {
    use crate::app::SettingsField;

    let area = f.area();
    let popup_width = 42u16.min(area.width.saturating_sub(4));
    let popup_height = 9u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let fields = [
        (SettingsField::Title, "Title", &app.song.title),
        (SettingsField::Bpm, "BPM", &app.song.bpm.to_string()),
        (SettingsField::Speed, "Speed", &app.song.speed.to_string()),
        (SettingsField::Channels, "Channels", &app.song.channels.to_string()),
        (SettingsField::Rows, "Rows", &app.song.rows_per_pattern.to_string()),
    ];

    let lines: Vec<Line> = fields.iter().map(|(field, label, _value)| {
        let is_active = *field == app.settings_field;
        let display_val = if is_active {
            format!("{}_", app.settings_edit_buf)
        } else {
            match field {
                SettingsField::Title => app.song.title.clone(),
                SettingsField::Bpm => app.song.bpm.to_string(),
                SettingsField::Speed => app.song.speed.to_string(),
                SettingsField::Channels => app.song.channels.to_string(),
                SettingsField::Rows => app.song.rows_per_pattern.to_string(),
            }
        };
        let label_style = Style::default().fg(Color::DarkGray);
        let val_style = if is_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        Line::from(vec![
            Span::styled(format!("  {:10}", label), label_style),
            Span::styled(display_val, val_style),
        ])
    }).collect();

    let widget = Paragraph::new(lines).block(
        Block::default()
            .title(" Song Settings [Tab=next, Enter/Esc=close] ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(widget, popup_area);
}

fn draw_instrument_list(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = 40u16.min(area.width.saturating_sub(4));
    let popup_height = 20u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let visible = (popup_height - 2) as usize;
    let start = if app.instrument_cursor > visible / 2 {
        app.instrument_cursor - visible / 2
    } else {
        0
    };

    let items: Vec<ListItem> = (start..256.min(start + visible))
        .map(|i| {
            let inst = &app.instruments[i];
            let name = if inst.name.is_empty() {
                "---".to_string()
            } else {
                inst.name.clone()
            };
            let text = format!(" {:02X} {}", i, name);
            let style = if i == app.instrument_cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if !inst.name.is_empty() {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Instruments [Esc=close, type=name] ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list, popup_area);
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
