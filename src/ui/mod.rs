pub mod pattern_editor;
pub mod sample_editor;
pub mod synth_editor;
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
    let area = f.area();
    if area.width < 40 || area.height < 10 {
        let msg = format!("Terminal too small ({}x{}, need 40x10)", area.width, area.height);
        let text = Paragraph::new(msg)
            .style(Style::default().fg(ratatui::style::Color::Red));
        f.render_widget(text, area);
        return;
    }

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

    if app.mode == Mode::PatternMatrix {
        // Full-screen pattern matrix replaces order sidebar + pattern editor
        draw_pattern_matrix(f, app, chunks[1], &theme);
    } else {
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
    }

    draw_status_bar(f, app, chunks[2], &theme);

    match app.mode {
        Mode::MidiPortSelect => draw_port_selector(f, app, &theme),
        Mode::Help => draw_help(f, app, &theme),
        Mode::SongSettings => draw_song_settings(f, app, &theme),
        Mode::InstrumentList => draw_instrument_list(f, app, &theme),
        Mode::SampleEditor => sample_editor::draw_sample_editor(f, app),
        Mode::SynthEditor => synth_editor::draw_synth_editor(f, app),
        Mode::QuitConfirm => draw_quit_confirm(f, app, &theme),
        Mode::ChannelRename => draw_channel_rename(f, app, &theme),
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
        Span::raw(if app.dirty { " [*]" } else { "" }),
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
        if app.is_playing() {
            let mins = (app.playback_elapsed / 60.0) as u32;
            let secs = (app.playback_elapsed % 60.0) as u32;
            Span::styled(
                format!(" {}:{:02}", mins, secs),
                Style::default().fg(theme.header_bpm),
            )
        } else {
            Span::raw("")
        },
        Span::raw(if app.follow_playback { " FLW" } else { "" }),
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
    if app.mode == Mode::Command {
        let text = format!(":{}", app.command_buf);
        let cmd_line = Paragraph::new(text)
            .style(Style::default().fg(theme.popup_highlight_fg));
        f.render_widget(cmd_line, area);
        return;
    }

    let midi_status = if app.midi.send_error_count > 0 {
        Span::styled(
            format!(" MIDI:ERR({}) ", app.midi.send_error_count),
            Style::default().fg(theme.mode_insert),
        )
    } else if app.midi_connected() {
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
        Mode::SynthEditor => Span::styled(" SYNTH EDIT ", Style::default().fg(theme.header_octave).add_modifier(Modifier::BOLD)),
        Mode::QuitConfirm => Span::styled(" QUIT? ", Style::default().fg(theme.mode_insert).add_modifier(Modifier::BOLD)),
        Mode::ChannelRename => Span::styled(" RENAME ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::PatternMatrix => Span::styled(" MATRIX ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::Command => Span::from(""), // handled above, never reached
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
            " [:]Cmd [F1]Help [Space]Play/Stop [Esc]Mode [Tab]Page [q]Quit ",
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
        Line::from(vec![Span::styled("  Ctrl+C/X/V   ", key_style), Span::styled("Copy / Cut / Paste row (or block)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+B       ", key_style), Span::styled("Toggle block selection", text_style)]),
        Line::from(vec![Span::styled("  Shift+Up/Dn  ", key_style), Span::styled("Transpose note(s) up/down semitone", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+I       ", key_style), Span::styled("Interpolate block (volume/effect ramp)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+F       ", key_style), Span::styled("Toggle follow mode (cursor follows playback)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+R       ", key_style), Span::styled("Rename current channel", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+L/R     ", key_style), Span::styled("Next / prev order position", text_style)]),
        Line::from(vec![Span::styled("  F9-F12       ", key_style), Span::styled("Mute/unmute ch (current page)", text_style)]),
        Line::from(vec![Span::styled("  ( / )        ", key_style), Span::styled("Edit step down / up", text_style)]),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  q            ", key_style), Span::styled("Quit", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+N       ", key_style), Span::styled("New pattern (append to order)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+D       ", key_style), Span::styled("Clone current pattern", text_style)]),
        Line::from(vec![Span::styled("  :            ", key_style), Span::styled("Command mode (vim-style)", text_style)]),
        Line::from(vec![Span::styled("  Ins / Bksp   ", key_style), Span::styled("Insert / delete row in pattern", text_style)]),
        Line::from(vec![Span::styled("  F6           ", key_style), Span::styled("Song settings dialog", text_style)]),
        Line::from(vec![Span::styled("  F7           ", key_style), Span::styled("Instrument list", text_style)]),
        Line::from(vec![Span::styled("  F8           ", key_style), Span::styled("Cycle color theme", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+E       ", key_style), Span::styled("Export to MIDI file (.mid)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+W       ", key_style), Span::styled("Export to WAV file", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+L       ", key_style), Span::styled("Export to FLAC file", text_style)]),
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
        Line::from(Span::styled("--- Commands (: in Normal) ---", section_style)),
        Line::from(vec![Span::styled("  :p :pattern  ", key_style), Span::styled("Pattern matrix (full screen)", text_style)]),
        Line::from(vec![Span::styled("  :w :write    ", key_style), Span::styled("Save file", text_style)]),
        Line::from(vec![Span::styled("  :q :quit     ", key_style), Span::styled("Quit (prompts if unsaved)", text_style)]),
        Line::from(vec![Span::styled("  :q!          ", key_style), Span::styled("Force quit without saving", text_style)]),
        Line::from(vec![Span::styled("  :wq          ", key_style), Span::styled("Save and quit", text_style)]),
        Line::from(vec![Span::styled("  :h :help     ", key_style), Span::styled("Help screen", text_style)]),
        Line::from(vec![Span::styled("  :set         ", key_style), Span::styled("Song settings", text_style)]),
        Line::from(vec![Span::styled("  :inst        ", key_style), Span::styled("Instrument list", text_style)]),
        Line::from(vec![Span::styled("  :midi        ", key_style), Span::styled("MIDI port selector", text_style)]),
        Line::from(vec![Span::styled("  :wav :flac   ", key_style), Span::styled("Export audio", text_style)]),
        Line::from(vec![Span::styled("  :link        ", key_style), Span::styled("Toggle Ableton Link", text_style)]),
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
            .title(" Song Settings ")
            .title_bottom(" Tab:next  Enter/Esc:close ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(widget, popup_area);
}

fn draw_instrument_list(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_area = centered_rect(popup_width, 20, area);

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
            // Show indicators for what's configured
            let mut tags = String::new();
            if let Some(ref sp) = inst.synth_params {
                use crate::audio::synth::Patch;
                let patch_name = Patch::from_program(sp.waveform).name();
                tags.push_str(&format!(" [{}]", patch_name));
            }
            if inst.sample_index.is_some() { tags.push_str(" [SMP]"); }
            if inst.midi_program.is_some() {
                tags.push_str(&format!(" [PRG:{:02X}]", inst.midi_program.unwrap()));
            }
            let text = format!(" {:02X} {}{}", i, name, tags);
            let has_data = inst.synth_params.is_some()
                || inst.sample_index.is_some()
                || inst.midi_program.is_some()
                || !inst.name.is_empty();
            let style = if i == app.instrument_cursor {
                Style::default().fg(theme.popup_highlight_fg).bg(theme.popup_highlight_bg).add_modifier(Modifier::BOLD)
            } else if has_data {
                Style::default().fg(theme.popup_text)
            } else {
                Style::default().fg(theme.status_hint)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Instruments ")
            .title_bottom(" Esc:close  Type:name  Enter:sample  Tab:synth ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(list, popup_area);
}

fn draw_quit_confirm(f: &mut Frame, _app: &App, theme: &Theme) {
    let area = f.area();
    let popup_area = centered_rect(44, 5, area);
    f.render_widget(Clear, popup_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Unsaved changes! Quit without saving?",
            Style::default().fg(theme.popup_text),
        )),
        Line::from(Span::styled(
            "  [Y] Quit  [S] Save & Quit  [Any] Cancel",
            Style::default().fg(theme.popup_key),
        )),
    ];

    let widget = Paragraph::new(lines).block(
        Block::default()
            .title(" Quit Confirmation ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(widget, popup_area);
}

fn draw_channel_rename(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_area = centered_rect(40, 5, area);
    f.render_widget(Clear, popup_area);

    let ch = app.cursor_channel;
    let ch_type = app.channel_types.get(ch).copied().unwrap_or(crate::app::ChannelType::Midi);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("  Name: "),
                Style::default().fg(theme.popup_text),
            ),
            Span::styled(
                format!("{}_", app.rename_buf),
                Style::default().fg(theme.settings_active).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  Type: "),
                Style::default().fg(theme.popup_text),
            ),
            Span::styled(
                format!("{}", ch_type.label()),
                Style::default().fg(theme.settings_active).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  (Tab to cycle)",
                Style::default().fg(theme.status_hint),
            ),
        ]),
    ];

    let widget = Paragraph::new(lines).block(
        Block::default()
            .title(format!(" Channel {} ", ch + 1))
            .title_bottom(" Enter/Esc:confirm  Tab:type ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(widget, popup_area);
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
            .title(" MIDI Output Port ")
            .title_bottom(" Enter:select  Esc:cancel ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );

    f.render_widget(list, popup_area);
}

fn draw_pattern_matrix(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    use ratatui::style::Color;

    let num_channels = app.song.channels;
    let ch_col_w: u16 = 5;
    let label_w: u16 = 15; // " 0: [00] x01 |"

    let block = Block::default()
        .title(" Pattern Matrix ")
        .title_bottom(" Enter:jump Ins:dup Del:rm +/-:pat [/]:rep ^N:new ^D:clone ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border));
    f.render_widget(block, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let buf = f.buffer_mut();
    let header_style = Style::default().fg(theme.header_title).add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(theme.popup_text);

    // Header row: "Pos Pat  Rep | Ch1  Ch2  ..."
    let header_y = inner.y;
    let header_label = "Pos Pat  Rep |";
    write_str(buf, inner.x, header_y, inner, header_label, header_style);
    for ch in 0..num_channels {
        let col_x = inner.x + label_w + ch as u16 * ch_col_w;
        let name = app.channel_names.get(ch).filter(|n| !n.is_empty());
        let label = match name {
            Some(n) => format!("{:>4} ", &n[..n.len().min(4)]),
            None => format!(" Ch{} ", ch + 1),
        };
        write_str(buf, col_x, header_y, inner, &label, header_style);
    }

    // Separator line
    let sep_y = inner.y + 1;
    if sep_y < inner.y + inner.height {
        for x in inner.x..inner.x + inner.width {
            buf[(x, sep_y)].set_char('-');
            buf[(x, sep_y)].set_style(dim_style);
        }
    }

    // Compute visible range with centered scrolling
    let data_height = (inner.height.saturating_sub(2)) as usize;
    let order_len = app.song.order.len();
    let half = data_height / 2;
    let scroll_offset = if order_len <= data_height {
        0
    } else if app.matrix_cursor <= half {
        0
    } else if app.matrix_cursor + half >= order_len {
        order_len.saturating_sub(data_height)
    } else {
        app.matrix_cursor - half
    };

    // Precompute: for each pattern, which channels have data?
    let pattern_channel_has_data: Vec<Vec<bool>> = app.song.patterns.iter().map(|pat| {
        let mut channel_data = vec![false; pat.channels];
        for row in &pat.data {
            for (ch, cell) in row.iter().enumerate() {
                if !cell.is_empty() {
                    channel_data[ch] = true;
                }
            }
        }
        channel_data
    }).collect();

    // Draw order rows
    for vis_row in 0..data_height {
        let ord_idx = scroll_offset + vis_row;
        if ord_idx >= order_len {
            break;
        }
        let y = inner.y + 2 + vis_row as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let pat_idx = app.song.order[ord_idx];
        let is_cursor = ord_idx == app.matrix_cursor;
        let is_playing = app.playing && ord_idx == app.playback_order;

        // Row label: " 0: [00] x01 |"
        let repeat = app.song.order_repeats.get(ord_idx).copied().unwrap_or(1);
        let rep_str = if repeat == 0 { " -- ".to_string() } else { format!(" x{:<2}", repeat) };
        let label = format!("{:>2}: [{:02X}]{} |", ord_idx, pat_idx, rep_str);
        let label_style = if is_cursor {
            Style::default().fg(theme.popup_highlight_fg).bg(theme.popup_highlight_bg).add_modifier(Modifier::BOLD)
        } else if is_playing {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            dim_style
        };
        write_str(buf, inner.x, y, inner, &label, label_style);

        // Channel cells: filled block vs dots
        let has_data = pattern_channel_has_data.get(pat_idx);
        for ch in 0..num_channels {
            let col_x = inner.x + label_w + ch as u16 * ch_col_w;
            let ch_has_data = has_data.and_then(|d| d.get(ch)).copied().unwrap_or(false);
            let (cell_text, cell_style) = if is_cursor {
                let text = if ch_has_data { "#### " } else { "  .  " };
                let style = Style::default()
                    .fg(theme.popup_highlight_fg)
                    .bg(theme.popup_highlight_bg)
                    .add_modifier(if ch_has_data { Modifier::BOLD } else { Modifier::empty() });
                (text, style)
            } else if ch_has_data {
                ("#### ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                ("  .  ", Style::default().fg(theme.muted_dim))
            };
            write_str(buf, col_x, y, inner, cell_text, cell_style);
        }
    }
}

/// Write a string into the buffer, clipping to `bounds`.
fn write_str(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, bounds: Rect, s: &str, style: Style) {
    for (i, c) in s.chars().enumerate() {
        let cx = x + i as u16;
        if cx < bounds.x + bounds.width && y < bounds.y + bounds.height {
            buf[(cx, y)].set_char(c);
            buf[(cx, y)].set_style(style);
        }
    }
}
