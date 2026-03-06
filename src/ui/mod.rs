pub mod pattern_editor;
pub mod sample_editor;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{App, Mode};
use theme::Theme;

/// Center a popup of given size within the available area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

pub fn draw(f: &mut Frame, app: &App) {
    let theme = app.theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),  // main area (order sidebar + pattern editor)
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0], &theme);

    // Split main area: order sidebar + pattern editor
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(7), // order list sidebar
            Constraint::Min(20),  // pattern editor
        ])
        .split(chunks[1]);

    draw_order_sidebar(f, app, main_chunks[0], &theme);
    pattern_editor::draw(f, app, main_chunks[1], &theme);
    draw_status_bar(f, app, chunks[2], &theme);

    match app.mode {
        Mode::MidiPortSelect => draw_port_selector(f, app, &theme),
        Mode::Help => draw_help(f, app, &theme),
        Mode::SongSettings => draw_song_settings(f, app, &theme),
        Mode::InstrumentList => draw_instrument_list(f, app, &theme),
        Mode::SampleEditor => sample_editor::draw_sample_editor(f, app),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let song = &app.song;
    let pattern_idx = app.current_order_position();
    let pattern_num = song.order.get(pattern_idx).copied().unwrap_or(0);

    let link_span = if app.link.is_enabled() {
        Span::styled(
            format!(" L:{}", app.link.num_peers()),
            Style::default().fg(theme.link_active),
        )
    } else {
        Span::styled(" L:--", Style::default().fg(theme.link_inactive))
    };

    let header_text = vec![Line::from(vec![
        Span::styled(" rtrack", Style::default().fg(theme.header_title).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(&song.title, Style::default().fg(theme.status_text)),
        Span::styled(
            format!(" {}bpm s{}", song.bpm, song.speed),
            Style::default().fg(theme.header_bpm),
        ),
        Span::styled(
            format!(" P:{:02X}/{:02X} O:{:02X}/{:02X}",
                pattern_num, song.current_pattern_count() - 1,
                pattern_idx, song.order.len().saturating_sub(1)),
            Style::default().fg(theme.header_position),
        ),
        Span::styled(
            format!(" Oct:{}", app.current_octave),
            Style::default().fg(theme.header_octave),
        ),
        Span::styled(
            format!(" Stp:{}", app.edit_step),
            Style::default().fg(theme.header_octave),
        ),
        Span::styled(
            format!(" Ch:{}/{} Pg:{}",
                app.cursor_channel + 1,
                app.song.channels,
                app.track_page + 1),
            Style::default().fg(theme.header_octave),
        ),
        Span::raw(if app.is_playing() { " PLAY" } else { " STOP" }),
        link_span,
    ])];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.header_border)),
    );
    f.render_widget(header, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let midi_status = if app.midi_connected() {
        Span::styled(
            format!(" MIDI:{} ", app.midi_port_display_name()),
            Style::default().fg(theme.midi_connected),
        )
    } else {
        Span::styled(" MIDI:-- ", Style::default().fg(theme.midi_disconnected))
    };

    let mode_span = match app.mode {
        Mode::Normal => Span::styled(" NORMAL ", Style::default().fg(theme.mode_normal)),
        Mode::Insert => Span::styled(" INSERT ", Style::default().fg(theme.mode_insert).add_modifier(Modifier::BOLD)),
        Mode::MidiPortSelect => Span::styled(" MIDI SELECT ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::Help => Span::styled(" HELP ", Style::default().fg(theme.mode_help).add_modifier(Modifier::BOLD)),
        Mode::SongSettings => Span::styled(" SETTINGS ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::InstrumentList => Span::styled(" INSTRUMENTS ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::SampleEditor => Span::styled(" SAMPLE EDIT ", Style::default().fg(theme.header_octave).add_modifier(Modifier::BOLD)),
    };

    let audio_span = if app.has_sf2() {
        Span::styled(" SF2 ", Style::default().fg(theme.header_octave))
    } else if app.has_audio() {
        Span::styled(" SYNTH ", Style::default().fg(theme.header_octave))
    } else {
        Span::from("")
    };

    let fx_span = if app.audio_effects_enabled() {
        Span::styled(" FX ", Style::default().fg(theme.effect_set))
    } else {
        Span::from("")
    };

    let msg_span = if let Some(ref msg) = app.status_message {
        Span::styled(format!(" {} ", msg), Style::default().fg(theme.status_text))
    } else {
        Span::styled(
            " [F1]Help [Space]Play/Stop [Esc]Mode [Tab]Page [Ctrl+1-8]Track [F2]MIDI [+/-]Oct [q]Quit ",
            Style::default().fg(theme.status_hint),
        )
    };

    let status = Paragraph::new(Line::from(vec![mode_span, midi_status, audio_span, fx_span, msg_span]));
    f.render_widget(status, area);
}

fn build_help_lines(theme: &Theme) -> Vec<Line<'static>> {
    let key_style = Style::default().fg(theme.popup_key);
    let text_style = Style::default().fg(theme.popup_text);
    let section_style = Style::default().fg(theme.popup_title).add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.status_hint);

    vec![
        Line::from(Span::styled("  Global Keys", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  F1           ", key_style), Span::styled("Toggle help", text_style)]),
        Line::from(vec![Span::styled("  F2           ", key_style), Span::styled("MIDI port selector", text_style)]),
        Line::from(vec![Span::styled("  F3           ", key_style), Span::styled("Toggle Ableton Link", text_style)]),
        Line::from(vec![Span::styled("  Space        ", key_style), Span::styled("Play / Stop", text_style)]),
        Line::from(vec![Span::styled("  Esc          ", key_style), Span::styled("Toggle Normal / Insert mode", text_style)]),
        Line::from(vec![Span::styled("  Tab/S-Tab    ", key_style), Span::styled("Next / prev track page", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+1..8    ", key_style), Span::styled("Select track 1-8 directly", text_style)]),
        Line::from(vec![Span::styled("  Arrows       ", key_style), Span::styled("Move cursor", text_style)]),
        Line::from(vec![Span::styled("  PgUp/PgDn    ", key_style), Span::styled("Move cursor 16 rows", text_style)]),
        Line::from(vec![Span::styled("  Home/End     ", key_style), Span::styled("Jump to first / last row", text_style)]),
        Line::from(vec![Span::styled("  + / -        ", key_style), Span::styled("Octave up / down", text_style)]),
        Line::from(vec![Span::styled("  [ / ]        ", key_style), Span::styled("BPM down / up", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+S       ", key_style), Span::styled("Save", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+Z / Y   ", key_style), Span::styled("Undo / Redo", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+C/X/V   ", key_style), Span::styled("Copy / Cut / Paste row", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+L/R     ", key_style), Span::styled("Next / prev order position", text_style)]),
        Line::from(vec![Span::styled("  F9-F12       ", key_style), Span::styled("Mute/unmute ch (current page)", text_style)]),
        Line::from(vec![Span::styled("  ( / )        ", key_style), Span::styled("Edit step down / up", text_style)]),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  q            ", key_style), Span::styled("Quit", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+N       ", key_style), Span::styled("New pattern (append to order)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+D       ", key_style), Span::styled("Clone current pattern", text_style)]),
        Line::from(vec![Span::styled("  F4 / F5      ", key_style), Span::styled("Insert / remove order entry", text_style)]),
        Line::from(vec![Span::styled("  Ins / Bksp   ", key_style), Span::styled("Insert / delete row in pattern", text_style)]),
        Line::from(vec![Span::styled("  F6           ", key_style), Span::styled("Song settings dialog", text_style)]),
        Line::from(vec![Span::styled("  F7           ", key_style), Span::styled("Instrument list", text_style)]),
        Line::from(vec![Span::styled("  F8           ", key_style), Span::styled("Cycle color theme", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+E       ", key_style), Span::styled("Export to MIDI file (.mid)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+W       ", key_style), Span::styled("Export to WAV file", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+M       ", key_style), Span::styled("Toggle MIDI clock output", text_style)]),
        Line::from(""),
        Line::from(Span::styled("  Insert Mode", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  z s x d c .. ", key_style), Span::styled("Notes C C# D D# E .. (lower octave)", text_style)]),
        Line::from(vec![Span::styled("  q 2 w 3 e .. ", key_style), Span::styled("Notes C C# D D# E .. (upper octave)", text_style)]),
        Line::from(vec![Span::styled("  0-9 / a-f    ", key_style), Span::styled("Hex entry (inst/vol/fx columns)", text_style)]),
        Line::from(vec![Span::styled("  Del/Bksp     ", key_style), Span::styled("Clear sub-column at cursor", text_style)]),
        Line::from(vec![Span::styled("  =            ", key_style), Span::styled("Enter note-off (===)", text_style)]),
        Line::from(vec![Span::styled("  Esc          ", key_style), Span::styled("Return to Normal mode", text_style)]),
        Line::from(""),
        Line::from(Span::styled("  [Up/Down] scroll | [Esc/F1] close", dim_style)),
    ]
}

fn draw_help(f: &mut Frame, app: &App, theme: &Theme) {
    let all_lines = build_help_lines(theme);
    let total_lines = all_lines.len();

    let area = f.area();
    let popup_width = 54u16;
    // Show as many lines as fit, with borders
    let max_content_height = area.height.saturating_sub(4);
    let content_height = (total_lines as u16).min(max_content_height);
    let popup_height = content_height + 2; // +2 for borders
    let popup_area = centered_rect(popup_width, popup_height, area);

    f.render_widget(Clear, popup_area);

    // Apply scroll offset
    let scroll = app.help_scroll;
    let max_scroll = total_lines.saturating_sub(content_height as usize);
    let offset = scroll.min(max_scroll);
    let visible_lines: Vec<Line> = all_lines.into_iter().skip(offset).take(content_height as usize).collect();

    let scroll_indicator = if max_scroll > 0 {
        format!(" Help [{}/{}] ", offset + 1, max_scroll + 1)
    } else {
        " Help ".to_string()
    };

    let help = Paragraph::new(visible_lines).block(
        Block::default()
            .title(scroll_indicator)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );

    f.render_widget(help, popup_area);
}

fn draw_order_sidebar(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
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
            Style::default().fg(theme.order_current).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.order_normal)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    let block = Block::default()
        .title(" Ord ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.order_border));

    let widget = Paragraph::new(lines).block(block);
    f.render_widget(widget, area);
}

fn draw_song_settings(f: &mut Frame, app: &App, theme: &Theme) {
    use crate::app::SettingsField;

    let area = f.area();
    let popup_area = centered_rect(42, 9, area);

    f.render_widget(Clear, popup_area);

    let fields = [
        (SettingsField::Title, "Title"),
        (SettingsField::Bpm, "BPM"),
        (SettingsField::Speed, "Speed"),
        (SettingsField::Channels, "Channels"),
        (SettingsField::Rows, "Rows"),
    ];

    let lines: Vec<Line> = fields.iter().map(|(field, label)| {
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
        let label_style = Style::default().fg(theme.settings_label);
        let val_style = if is_active {
            Style::default().fg(theme.settings_active).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.settings_value)
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
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(widget, popup_area);
}

fn draw_instrument_list(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_area = centered_rect(40, 20, area);

    f.render_widget(Clear, popup_area);

    let visible = (popup_area.height.saturating_sub(2)) as usize;
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
                Style::default().fg(theme.popup_highlight_fg).bg(theme.popup_highlight_bg).add_modifier(Modifier::BOLD)
            } else if !inst.name.is_empty() {
                Style::default().fg(theme.popup_text)
            } else {
                Style::default().fg(theme.status_hint)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Instruments [Esc=close, type=name] ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(list, popup_area);
}

fn draw_port_selector(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_height = (app.midi_port_list.len() as u16 + 2).min(area.height.saturating_sub(4));
    let popup_area = centered_rect(50, popup_height, area);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = app
        .midi_port_list
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == app.midi_port_cursor {
                Style::default()
                    .fg(theme.popup_highlight_fg)
                    .bg(theme.popup_highlight_bg)
                    .add_modifier(Modifier::BOLD)
            } else if app.midi.port_name.as_deref() == Some(name) {
                Style::default().fg(theme.midi_connected)
            } else {
                Style::default().fg(theme.popup_text)
            };
            ListItem::new(format!(" {} ", name)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" MIDI Output Port [Enter=Select, Esc=Cancel] ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );

    f.render_widget(list, popup_area);
}
