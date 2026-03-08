pub mod pattern_editor;
pub mod sample_editor;
pub mod synth_editor;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
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
        Mode::TrackConfig => draw_track_config(f, app, &theme),
        Mode::FileBrowser => draw_file_browser(f, app, &theme),
        Mode::RecentFiles => draw_recent_files(f, app, &theme),
        _ => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let song = &app.song;
    let pattern_idx = app.current_order_position();
    let pattern_num = song.order.get(pattern_idx).copied().unwrap_or(0);

    // Draw the border first
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.header_border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Two-column layout: left has all info, right is for status symbols (Link, etc.)
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),       // left: song info + transport + position
            Constraint::Length(8),      // right: status symbols (right-justified)
        ])
        .split(inner);

    // -- Left: song info + bpm + transport + position + edit state --
    let title_display = if app.dirty {
        format!("{}*", song.title)
    } else {
        song.title.clone()
    };

    let mut left_spans = vec![
        Span::styled(" rtrack", Style::default().fg(theme.header_title).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(title_display, Style::default().fg(theme.status_text)),
        Span::raw("  "),
        Span::styled(
            format!("{}bpm", song.bpm),
            Style::default().fg(theme.header_bpm),
        ),
        Span::styled(
            if app.is_playing() { " \u{25B6}" } else { " \u{25A0}" },
            if app.is_playing() {
                Style::default().fg(theme.header_title)
            } else {
                Style::default()
            },
        ),
        Span::styled(
            " \u{25CF}",
            if app.recording {
                Style::default().fg(theme.mode_insert).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_hint)
            },
        ),
    ];
    let mins = (app.timing.playback_elapsed / 60.0) as u32;
    let secs = (app.timing.playback_elapsed % 60.0) as u32;
    let centis = ((app.timing.playback_elapsed.fract()) * 100.0) as u32;
    left_spans.push(Span::styled(
        format!(" {:02}:{:02}:{:02}", mins, secs, centis),
        Style::default().fg(theme.header_bpm),
    ));
    left_spans.extend([
        Span::raw("  "),
        Span::styled(
            format!("P:{:02X}/{:02X}",
                pattern_num, song.current_pattern_count() - 1),
            Style::default().fg(theme.header_position),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Oct:{}", app.current_octave),
            Style::default().fg(theme.header_octave),
        ),
        Span::styled(
            format!(" Stp:{}", app.edit_step),
            Style::default().fg(theme.header_octave),
        ),
        Span::styled(
            format!(" Ch:{}/{}",
                app.cursor_channel + 1,
                app.song.channels),
            Style::default().fg(theme.header_octave),
        ),
    ]);
    f.render_widget(Paragraph::new(Line::from(left_spans)), columns[0]);

    // -- Right: status symbols (right-justified) --
    let mut right_spans: Vec<Span> = Vec::new();
    if app.link.is_enabled() {
        let peers = app.link.num_peers();
        let style = if peers > 0 {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.status_hint)
        };
        right_spans.push(Span::styled(format!("Link:{}", peers), style));
    }
    if !right_spans.is_empty() {
        let right = Paragraph::new(Line::from(right_spans))
            .alignment(ratatui::layout::Alignment::Right);
        f.render_widget(right, columns[1]);
    }
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
        Mode::TrackConfig => Span::styled(" TRACK ", Style::default().fg(theme.header_octave).add_modifier(Modifier::BOLD)),
        Mode::PatternMatrix => Span::styled(" MATRIX ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::FileBrowser => Span::styled(" FILES ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
        Mode::RecentFiles => Span::styled(" RECENT ", Style::default().fg(theme.mode_port_select).add_modifier(Modifier::BOLD)),
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
        Line::from(vec![Span::styled("  Tab/S-Tab    ", key_style), Span::styled("Next / prev track (wraps)", text_style)]),
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
        Line::from(vec![Span::styled("  Ctrl+R       ", key_style), Span::styled("Toggle recording (punch-in MIDI notes during playback)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+L/R     ", key_style), Span::styled("Next / prev order position", text_style)]),
        Line::from(vec![Span::styled("  F9-F12       ", key_style), Span::styled("Mute/unmute ch (current page)", text_style)]),
        Line::from(vec![Span::styled("  ( / )        ", key_style), Span::styled("Edit step down / up", text_style)]),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  q            ", key_style), Span::styled("Quit", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+N       ", key_style), Span::styled("New pattern (append to order)", text_style)]),
        Line::from(vec![Span::styled("  Ctrl+D       ", key_style), Span::styled("Clone current pattern", text_style)]),
        Line::from(vec![Span::styled("  Enter        ", key_style), Span::styled("Track config (name, type, effects)", text_style)]),
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
        Line::from(vec![Span::styled("  :fx :effects ", key_style), Span::styled("Track config (name, type, effects)", text_style)]),
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
    let scroll = app.dialogs.help_scroll;
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
    let popup_area = centered_rect(42, 12, area);

    f.render_widget(Clear, popup_area);

    let fields = [
        (SettingsField::Title, "Title"),
        (SettingsField::Bpm, "BPM"),
        (SettingsField::Speed, "Speed"),
        (SettingsField::Channels, "Channels"),
        (SettingsField::Rows, "Rows"),
        (SettingsField::HighlightBeat, "Beat Hilight"),
        (SettingsField::HighlightBar, "Bar Hilight"),
        (SettingsField::Swing, "Swing"),
    ];

    let lines: Vec<Line> = fields.iter().map(|(field, label)| {
        let is_active = *field == app.dialogs.settings_field;
        let display_val = if is_active {
            format!("{}_", app.dialogs.settings_edit_buf)
        } else {
            match field {
                SettingsField::Title => app.song.title.clone(),
                SettingsField::Bpm => app.song.bpm.to_string(),
                SettingsField::Speed => app.song.speed.to_string(),
                SettingsField::Channels => app.song.channels.to_string(),
                SettingsField::Rows => app.song.rows_per_pattern.to_string(),
                SettingsField::HighlightBeat => app.song.highlight_beat.to_string(),
                SettingsField::HighlightBar => app.song.highlight_bar.to_string(),
                SettingsField::Swing => format!("{}%", app.song.swing),
            }
        };
        let label_style = Style::default().fg(theme.settings_label);
        let val_style = if is_active {
            Style::default().fg(theme.settings_active).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.settings_value)
        };
        Line::from(vec![
            Span::styled(format!("  {:14}", label), label_style),
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
    let start = if app.dialogs.instrument_cursor > visible / 2 {
        app.dialogs.instrument_cursor - visible / 2
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
            let style = if i == app.dialogs.instrument_cursor {
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

fn draw_track_config(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let ch = app.cursor_channel;
    let ch_type = app.channels.get(ch).map(|c| c.channel_type).unwrap_or(crate::app::ChannelType::Midi);
    let is_synth = ch_type == crate::app::ChannelType::Synth;
    let has_fx = ch_type != crate::app::ChannelType::Midi;

    let is_sample = ch_type == crate::app::ChannelType::Sample;
    let popup_h = match ch_type {
        crate::app::ChannelType::Midi => 5,
        crate::app::ChannelType::Synth => 18,
        crate::app::ChannelType::Sample => 18,
    };
    let popup_area = centered_rect(60, popup_h, area);
    f.render_widget(Clear, popup_area);

    let params = app.channels.get(ch)
        .map(|c| c.effects_params.clone())
        .unwrap_or_default();
    let field = app.ch_fx_field;
    // Effects fields start at offset 3 for Synth and Sample, 2 for Midi
    let fx_off: usize = if is_synth || is_sample { 3 } else { 2 };

    let active = Style::default().fg(theme.settings_active).add_modifier(Modifier::BOLD);
    let normal = Style::default().fg(theme.popup_text);
    let on_style = Style::default().fg(theme.mode_insert).add_modifier(Modifier::BOLD);
    let off_style = Style::default().fg(theme.muted_dim);
    let dim = Style::default().fg(theme.status_hint);

    let style_for = |f_idx: usize| -> Style {
        if field == f_idx { active } else { normal }
    };

    let on_off = |enabled: bool, f_idx: usize| -> Span {
        if enabled {
            Span::styled("[x]", if field == f_idx { active } else { on_style })
        } else {
            Span::styled("[ ]", if field == f_idx { active } else { off_style })
        }
    };

    // Build a lookup for CC mappings on this channel
    let cc_label = |fx_rel: usize| -> Span<'static> {
        if let Some(param) = crate::app::LearnableParam::from_fx_field(fx_rel) {
            for m in &app.midi_cc_mappings {
                if m.channel == ch && m.param == param {
                    return Span::styled(format!(" CC{}", m.cc), Style::default().fg(Color::Yellow));
                }
            }
        }
        Span::raw("")
    };

    let name_display = if field == 0 {
        format!("{}_", app.rename_buf)
    } else {
        app.rename_buf.clone()
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  Name: ", style_for(0)),
            Span::styled(name_display, style_for(0)),
        ]),
        Line::from(vec![
            Span::styled("  Type: ", style_for(1)),
            Span::styled(ch_type.label().to_string(), style_for(1)),
            Span::styled("  (Left/Right)", dim),
        ]),
    ];

    if is_synth {
        let inst_val = app.channels.get(ch).and_then(|c| c.default_instrument);
        let inst_display = match inst_val {
            Some(i) => {
                let inst = &app.instruments[i as usize];
                let label = if !inst.name.is_empty() {
                    inst.name.clone()
                } else if let Some(ref sp) = inst.synth_params {
                    crate::audio::synth::Patch::from_program(sp.waveform).name().to_string()
                } else {
                    // No synth_params configured yet -- show default patch name
                    crate::audio::synth::Patch::from_program(i).name().to_string()
                };
                format!("{:02X} {}", i, label)
            }
            None => "-- (none)".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled("  Inst: ", style_for(2)),
            Span::styled(inst_display, style_for(2)),
        ]));
    }

    if is_sample {
        let selected_slot = app.channels.get(ch).and_then(|c| c.default_instrument);
        let sample_display = selected_slot
            .and_then(|s| app.sample_bank.get(s as usize))
            .map(|s| format!("{:02X} {} ({:.1}s)", selected_slot.unwrap(), s.name, s.duration()))
            .unwrap_or_else(|| "(none)".to_string());
        let loaded = app.sample_bank.loaded_slots();
        let hint_text = if loaded.is_empty() {
            "  (Enter:browse)"
        } else {
            "  (L/R:select  Enter:browse)"
        };
        lines.push(Line::from(vec![
            Span::styled("  Smpl: ", style_for(2)),
            Span::styled(sample_display, style_for(2)),
            Span::styled(hint_text, dim),
        ]));
    }

    if has_fx {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  -- Effects --", dim)));
        lines.push(Line::from(vec![
            Span::styled("  Filter:     ", style_for(fx_off)),
            on_off(params.filter_enabled, fx_off),
            Span::styled("  Cutoff: ", style_for(fx_off + 1)),
            Span::styled(format!("{:.0}", params.filter_cutoff), style_for(fx_off + 1)),
            cc_label(1),
            Span::styled("  Res: ", style_for(fx_off + 2)),
            Span::styled(format!("{:.2}", params.filter_resonance), style_for(fx_off + 2)),
            cc_label(2),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Distortion: ", style_for(fx_off + 3)),
            on_off(params.distortion_enabled, fx_off + 3),
            Span::styled("  Drive: ", style_for(fx_off + 4)),
            Span::styled(format!("{:.1}", params.distortion_drive), style_for(fx_off + 4)),
            cc_label(4),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Chorus:     ", style_for(fx_off + 5)),
            on_off(params.chorus_enabled, fx_off + 5),
            Span::styled("  Rate: ", style_for(fx_off + 6)),
            Span::styled(format!("{:.1}", params.chorus_rate), style_for(fx_off + 6)),
            cc_label(6),
            Span::styled("  Depth: ", style_for(fx_off + 7)),
            Span::styled(format!("{:.1}", params.chorus_depth), style_for(fx_off + 7)),
            cc_label(7),
            Span::styled("  Mix: ", style_for(fx_off + 8)),
            Span::styled(format!("{:.2}", params.chorus_mix), style_for(fx_off + 8)),
            cc_label(8),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Delay:      ", style_for(fx_off + 9)),
            on_off(params.delay_enabled, fx_off + 9),
            Span::styled("  Time: ", style_for(fx_off + 10)),
            Span::styled(format!("{:.0}ms", params.delay_time), style_for(fx_off + 10)),
            cc_label(10),
            Span::styled("  Fdbk: ", style_for(fx_off + 11)),
            Span::styled(format!("{:.2}", params.delay_feedback), style_for(fx_off + 11)),
            cc_label(11),
            Span::styled("  Mix: ", style_for(fx_off + 12)),
            Span::styled(format!("{:.2}", params.delay_mix), style_for(fx_off + 12)),
            cc_label(12),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Reverb:     ", style_for(fx_off + 13)),
            on_off(params.reverb_enabled, fx_off + 13),
            Span::styled("  Size: ", style_for(fx_off + 14)),
            Span::styled(format!("{:.2}", params.reverb_size), style_for(fx_off + 14)),
            cc_label(14),
            Span::styled("  Damp: ", style_for(fx_off + 15)),
            Span::styled(format!("{:.2}", params.reverb_damp), style_for(fx_off + 15)),
            cc_label(15),
            Span::styled("  Mix: ", style_for(fx_off + 16)),
            Span::styled(format!("{:.2}", params.reverb_mix), style_for(fx_off + 16)),
            cc_label(16),
        ]));
    }

    let hint = if has_fx {
        " Tab:next  L/R:adjust  L:learn  U:unlearn  Esc:close "
    } else {
        " Tab:next  Left/Right:cycle  Enter/Esc:close "
    };

    let widget = Paragraph::new(lines).block(
        Block::default()
            .title(format!(" Track {} ", ch + 1))
            .title_bottom(hint)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border)),
    );
    f.render_widget(widget, popup_area);
}

fn draw_file_browser(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_h = 20.min(area.height.saturating_sub(4));
    let popup_area = centered_rect(70, popup_h, area);
    f.render_widget(Clear, popup_area);

    let dir_display = app.dialogs.file_browser.dir.to_string_lossy().to_string();
    let title = format!(" {} ", dir_display);

    let block = Block::default()
        .title(title)
        .title_bottom(" Up/Down:nav  Enter:open  Backspace:parent  Esc:cancel ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let visible_rows = inner.height as usize;
    let num_entries = app.dialogs.file_browser.entries.len();
    let cursor = app.dialogs.file_browser.cursor;

    // Calculate scroll offset to keep cursor visible
    let scroll = if cursor >= visible_rows {
        cursor - visible_rows + 1
    } else {
        0
    };

    let mut lines = Vec::new();

    if num_entries == 0 {
        lines.push(Line::from(Span::styled(
            "  (empty directory)",
            Style::default().fg(theme.status_hint),
        )));
    } else {
        for (i, entry) in app.dialogs.file_browser.entries.iter().enumerate().skip(scroll).take(visible_rows) {
            let is_selected = i == cursor;
            let marker = if is_selected { "> " } else { "  " };

            let (icon, name_style) = if entry.is_dir {
                ("/", if is_selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                })
            } else {
                ("", if is_selected {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.popup_text)
                })
            };

            let marker_style = if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_hint)
            };

            lines.push(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(format!("{}{}", entry.name, icon), name_style),
            ]));
        }
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn draw_recent_files(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let count = app.recent_files.len();
    let popup_h = (count as u16 + 2).max(4).min(area.height.saturating_sub(4));
    let popup_area = centered_rect(60, popup_h, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Recent Files ")
        .title_bottom(" Enter:open  Esc:cancel ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines = Vec::new();
    if count == 0 {
        lines.push(Line::from(Span::styled(
            "  (no recent files)",
            Style::default().fg(theme.status_hint),
        )));
    } else {
        for (i, path) in app.recent_files.iter().enumerate() {
            let is_selected = i == app.dialogs.recent_cursor;
            let display = path.file_name()
                .and_then(|f| f.to_str())
                .unwrap_or_else(|| path.to_str().unwrap_or("?"));
            let dir_display = path.parent()
                .and_then(|p| p.to_str())
                .unwrap_or("");
            let marker = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.popup_text)
            };
            let dim = Style::default().fg(theme.status_hint);
            let marker_style = if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.status_hint)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(display.to_string(), style),
                Span::styled(format!("  {}", dir_display), dim),
            ]));
        }
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn draw_port_selector(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let popup_height = (app.dialogs.midi_port_list.len() as u16 + 2).min(area.height.saturating_sub(4));
    let popup_area = centered_rect(50, popup_height, area);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = app
        .dialogs.midi_port_list
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == app.dialogs.midi_port_cursor {
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
        let name = app.channels.get(ch).map(|c| &c.name).filter(|n| !n.is_empty());
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
        let is_playing = app.playing && ord_idx == app.engine.order;

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
