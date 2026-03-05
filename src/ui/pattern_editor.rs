use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    Frame,
};

use crate::app::App;

/// Which sub-column within a channel the cursor is on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubColumn {
    Note,
    Instrument,
    Volume,
    Effect,
}

impl SubColumn {
    pub fn next(self) -> Self {
        match self {
            Self::Note => Self::Instrument,
            Self::Instrument => Self::Volume,
            Self::Volume => Self::Effect,
            Self::Effect => Self::Note,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Note => Self::Effect,
            Self::Instrument => Self::Note,
            Self::Volume => Self::Instrument,
            Self::Effect => Self::Volume,
        }
    }
}

// Layout constants
const ROW_NUM_WIDTH: u16 = 3; // "00 "
const NOTE_WIDTH: u16 = 3;    // "C-4"
const INST_WIDTH: u16 = 2;    // "01"
const VOL_WIDTH: u16 = 2;     // "80"
const FX_WIDTH: u16 = 3;      // "000"
const GAPS: u16 = 4;          // spaces between sub-columns
const CHANNEL_WIDTH: u16 = NOTE_WIDTH + INST_WIDTH + VOL_WIDTH + FX_WIDTH + GAPS;
const SEPARATOR_WIDTH: u16 = 3; // " | "

pub fn channel_total_width(num_channels: usize) -> u16 {
    ROW_NUM_WIDTH + SEPARATOR_WIDTH
        + (CHANNEL_WIDTH * num_channels as u16)
        + (SEPARATOR_WIDTH * (num_channels.saturating_sub(1)) as u16)
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let buf = f.buffer_mut();

    let pattern_idx = app.song.order[app.current_order_position()];
    let pattern = &app.song.patterns[pattern_idx];

    let visible_rows = area.height as usize;
    let center_offset = visible_rows / 2;

    // Determine which rows to display, centered on cursor/playback position
    let focus_row = if app.is_playing() {
        app.playback_row
    } else {
        app.cursor_row
    };

    let start_row = if focus_row >= center_offset {
        focus_row - center_offset
    } else {
        0
    };

    for screen_y in 0..visible_rows {
        let row_idx = start_row + screen_y;
        if row_idx >= pattern.rows {
            break;
        }

        let y = area.y + screen_y as u16;
        if y >= area.y + area.height {
            break;
        }

        let is_cursor_row = !app.is_playing() && row_idx == app.cursor_row;
        let is_playback_row = app.is_playing() && row_idx == app.playback_row;
        let is_beat = row_idx % 4 == 0;
        let is_bar = row_idx % 16 == 0;

        // Row number
        let row_num_style = if is_bar {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else if is_beat {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let row_str = format!("{:02X}", row_idx);
        write_str(buf, area.x, y, &row_str, row_num_style);
        write_str(buf, area.x + ROW_NUM_WIDTH, y, " | ", Style::default().fg(Color::DarkGray));

        let mut x = area.x + ROW_NUM_WIDTH + SEPARATOR_WIDTH;

        for ch in 0..pattern.channels {
            if ch > 0 {
                write_str(buf, x, y, " | ", Style::default().fg(Color::DarkGray));
                x += SEPARATOR_WIDTH;
            }

            let cell = pattern.get(row_idx, ch);

            // Determine styles for each sub-column
            let base_bg = if is_playback_row {
                Color::DarkGray
            } else if is_cursor_row {
                Color::Rgb(30, 30, 50)
            } else {
                Color::Reset
            };

            let note_fg = if cell.note.is_some() { Color::White } else { Color::Rgb(60, 60, 60) };
            let inst_fg = if cell.instrument.is_some() { Color::Yellow } else { Color::Rgb(60, 60, 60) };
            let vol_fg = if cell.volume.is_some() { Color::Green } else { Color::Rgb(60, 60, 60) };
            let fx_fg = if cell.effect.is_some() || cell.effect_value.is_some() {
                Color::Cyan
            } else {
                Color::Rgb(60, 60, 60)
            };

            // Highlight the cursor sub-column
            let cursor_on = |sub: SubColumn| -> Style {
                if is_cursor_row && ch == app.cursor_channel && app.cursor_sub == sub && !app.is_playing() {
                    Style::default().bg(Color::Rgb(80, 80, 160)).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(base_bg)
                }
            };

            // Note
            let note_str = cell.display_note();
            write_str(buf, x, y, &note_str, cursor_on(SubColumn::Note).fg(note_fg));
            x += NOTE_WIDTH;

            write_str(buf, x, y, " ", Style::default().bg(base_bg));
            x += 1;

            // Instrument
            let inst_str = cell.display_instrument();
            write_str(buf, x, y, &inst_str, cursor_on(SubColumn::Instrument).fg(inst_fg));
            x += INST_WIDTH;

            write_str(buf, x, y, " ", Style::default().bg(base_bg));
            x += 1;

            // Volume
            let vol_str = cell.display_volume();
            write_str(buf, x, y, &vol_str, cursor_on(SubColumn::Volume).fg(vol_fg));
            x += VOL_WIDTH;

            write_str(buf, x, y, " ", Style::default().bg(base_bg));
            x += 1;

            // Effect
            let fx_str = cell.display_effect();
            write_str(buf, x, y, &fx_str, cursor_on(SubColumn::Effect).fg(fx_fg));
            x += FX_WIDTH;
        }
    }
}

fn write_str(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    let area = buf.area;
    for (i, ch) in s.chars().enumerate() {
        let cx = x + i as u16;
        if cx < area.x + area.width && y < area.y + area.height {
            buf[(cx, y)].set_char(ch).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_column_navigation() {
        assert_eq!(SubColumn::Note.next(), SubColumn::Instrument);
        assert_eq!(SubColumn::Instrument.next(), SubColumn::Volume);
        assert_eq!(SubColumn::Volume.next(), SubColumn::Effect);
        assert_eq!(SubColumn::Effect.next(), SubColumn::Note);

        assert_eq!(SubColumn::Note.prev(), SubColumn::Effect);
        assert_eq!(SubColumn::Effect.prev(), SubColumn::Volume);
    }

    #[test]
    fn test_channel_total_width() {
        // 1 channel: 3 (row) + 3 (sep) + 14 (channel) = 20
        assert_eq!(channel_total_width(1), 20);
        // 4 channels: 3 + 3 + 4*14 + 3*3 = 71
        assert_eq!(channel_total_width(4), 71);
    }
}
