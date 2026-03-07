use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use crate::app::App;
use super::theme::Theme;

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
#[allow(dead_code)]
const GAPS: u16 = 3;          // spaces between sub-columns
#[allow(dead_code)]
const CHANNEL_WIDTH: u16 = NOTE_WIDTH + INST_WIDTH + VOL_WIDTH + FX_WIDTH + GAPS;
const SEPARATOR_WIDTH: u16 = 3; // " | "
pub const MAX_CHANNEL_NAME: usize = 5;

#[allow(dead_code)]
pub fn channel_total_width(num_channels: usize) -> u16 {
    ROW_NUM_WIDTH + SEPARATOR_WIDTH
        + (CHANNEL_WIDTH * num_channels as u16)
        + (SEPARATOR_WIDTH * (num_channels.saturating_sub(1)) as u16)
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let buf = f.buffer_mut();

    let pattern_idx = app.song.order[app.current_order_position()];
    let pattern = &app.song.patterns[pattern_idx];

    // Draw column headers on the first row
    let header_y = area.y;
    let header_style = Style::default().fg(theme.row_bar).add_modifier(Modifier::BOLD);
    let dim_header = Style::default().fg(theme.row_beat);
    write_str(buf, area.x, header_y, "   ", dim_header);
    write_str(buf, area.x + ROW_NUM_WIDTH, header_y, " | ", Style::default().fg(theme.separator));
    {
        let mut hx = area.x + ROW_NUM_WIDTH + SEPARATOR_WIDTH;
        let visible = app.visible_channels();
        let first_visible = visible.start;
        for ch in visible.clone() {
            if ch > first_visible {
                write_str(buf, hx, header_y, " | ", Style::default().fg(theme.separator));
                hx += SEPARATOR_WIDTH;
            }
            let col_start = hx;
            let ch_name = app.channel_names.get(ch).filter(|n| !n.is_empty());
            let ch_type = app.channel_types.get(ch).copied().unwrap_or(crate::app::ChannelType::Midi);
            let type_label = ch_type.label();
            let type_style = Style::default().fg(match ch_type {
                crate::app::ChannelType::Midi => theme.header_bpm,
                crate::app::ChannelType::Synth => theme.header_octave,
                crate::app::ChannelType::Sample => theme.effect_set,
            }).add_modifier(Modifier::BOLD);

            // Layout: <name> <pad> [TYP] M   (right-justified type + indicator)
            // Indicator: last char (pos 12), type: pos 6-10, space at 11
            let w = CHANNEL_WIDTH as usize; // 13
            let name_str: String = if let Some(name) = ch_name {
                name.chars().take(MAX_CHANNEL_NAME).collect()
            } else {
                format!("{}", ch + 1)
            };

            // Mute/solo indicator char
            let indicator = if app.solo_channel == Some(ch) {
                'S'
            } else if app.muted_channels.get(ch).copied().unwrap_or(false) {
                'M'
            } else {
                ' '
            };

            // Right portion: "[MID] M" = 7 chars (type_label + space + indicator)
            let right = format!("{} {}", type_label, indicator);
            let right_len = right.len(); // 7
            let left_space = w.saturating_sub(right_len);
            let padded_name = format!("{:<width$}", name_str, width = left_space);
            let header_full: String = format!("{}{}", padded_name, right)
                .chars().take(w).collect();

            // Write full string in dim style as background
            write_str(buf, hx, header_y, &header_full, dim_header);
            // Overlay name in bold
            write_str(buf, hx, header_y, &name_str, header_style);
            // Overlay type label in color (right-justified)
            let type_x = hx + left_space as u16;
            write_str(buf, type_x, header_y, type_label, type_style);
            // Overlay indicator in color
            if indicator == 'S' {
                write_str(buf, hx + (w - 1) as u16, header_y, "S",
                    Style::default().fg(theme.mode_insert).add_modifier(Modifier::BOLD));
            } else if indicator == 'M' {
                write_str(buf, hx + (w - 1) as u16, header_y, "M",
                    Style::default().fg(theme.muted_dim).add_modifier(Modifier::BOLD));
            }

            hx = col_start + CHANNEL_WIDTH;
        }
    }

    // Pattern data starts one row below the header
    let data_area_y = area.y + 1;
    let data_area_height = area.height.saturating_sub(1) as usize;

    let visible_rows = data_area_height;
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

        let y = data_area_y + screen_y as u16;
        if y >= data_area_y + data_area_height as u16 {
            break;
        }

        let is_cursor_row = !app.is_playing() && row_idx == app.cursor_row;
        let is_playback_row = app.is_playing() && row_idx == app.playback_row;
        let beat_interval = app.song.highlight_beat.max(1);
        let bar_interval = app.song.highlight_bar.max(1);
        let is_beat = row_idx % beat_interval == 0;
        let is_bar = row_idx % bar_interval == 0;

        // Row number
        let row_num_style = if is_bar {
            Style::default().fg(theme.row_bar).add_modifier(Modifier::BOLD)
        } else if is_beat {
            Style::default().fg(theme.row_beat)
        } else {
            Style::default().fg(theme.row_normal)
        };

        let row_str = format!("{:02X}", row_idx);
        write_str(buf, area.x, y, &row_str, row_num_style);
        write_str(buf, area.x + ROW_NUM_WIDTH, y, " | ", Style::default().fg(theme.separator));

        let mut x = area.x + ROW_NUM_WIDTH + SEPARATOR_WIDTH;

        let visible = app.visible_channels();
        let first_visible = visible.start;
        for ch in visible.clone() {
            if ch > first_visible {
                write_str(buf, x, y, " | ", Style::default().fg(theme.separator));
                x += SEPARATOR_WIDTH;
            }

            let cell = pattern.get(row_idx, ch);

            // Check if this cell is inside a block selection
            let in_block = app.block_bounds().map_or(false, |(r0, r1, c0, c1)| {
                row_idx >= r0 && row_idx <= r1 && ch >= c0 && ch <= c1
            });

            // Determine styles for each sub-column
            let base_bg = if is_playback_row {
                theme.playback_row_bg
            } else if in_block {
                theme.cursor_row_bg
            } else if is_cursor_row {
                theme.cursor_row_bg
            } else {
                ratatui::style::Color::Reset
            };

            let is_muted = !app.is_channel_audible(ch);

            let note_fg = if is_muted { theme.muted_dim } else if cell.note.is_some() { theme.note_set } else { theme.note_empty };
            let inst_fg = if is_muted { theme.muted_dim } else if cell.instrument.is_some() { theme.instrument_set } else { theme.instrument_empty };
            let vol_fg = if is_muted { theme.muted_dim } else if cell.volume.is_some() { theme.volume_set } else { theme.volume_empty };
            let fx_fg = if is_muted { theme.muted_dim } else if cell.effect.is_some() || cell.effect_value.is_some() {
                theme.effect_set
            } else {
                theme.effect_empty
            };

            // Highlight the cursor sub-column
            let cursor_on = |sub: SubColumn| -> Style {
                if is_cursor_row && ch == app.cursor_channel && app.cursor_sub == sub && !app.is_playing() {
                    Style::default().bg(theme.cursor_bg).add_modifier(Modifier::BOLD)
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
        // 1 channel: 3 (row) + 3 (sep) + 13 (channel) = 19
        assert_eq!(channel_total_width(1), 19);
        // 4 channels: 3 + 3 + 4*13 + 3*3 = 67
        assert_eq!(channel_total_width(4), 67);
    }
}
