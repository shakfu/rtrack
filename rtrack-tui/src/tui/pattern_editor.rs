use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    Frame,
};

use super::theme::Theme;
use crate::app::App;
pub use crate::app::SubColumn;

// Layout constants
pub(crate) const ROW_NUM_WIDTH: u16 = 3; // "00 "
const NOTE_WIDTH: u16 = 3; // "C-4"
const INST_WIDTH: u16 = 2; // "01"
const VOL_WIDTH: u16 = 2; // "80"
const FX_WIDTH: u16 = 5; // "PVB44"
#[allow(dead_code)]
const GAPS: u16 = 6; // spaces between sub-columns (2 per gap)
/// Width of a track showing one effect column.
pub(crate) const CHANNEL_WIDTH: u16 = NOTE_WIDTH + INST_WIDTH + VOL_WIDTH + FX_WIDTH + GAPS;
pub(crate) const SEPARATOR_WIDTH: u16 = 3; // " | "
pub use rtrack_core::constants::MAX_CHANNEL_NAME;

/// Width of a track showing `fx_columns` effect columns.
pub(crate) fn channel_width(fx_columns: usize) -> u16 {
    CHANNEL_WIDTH + (fx_columns.saturating_sub(1) as u16) * (2 + FX_WIDTH)
}

#[allow(dead_code)]
pub fn channel_total_width(num_channels: usize, fx_columns: usize) -> u16 {
    ROW_NUM_WIDTH
        + SEPARATOR_WIDTH
        + (channel_width(fx_columns) * num_channels as u16)
        + (SEPARATOR_WIDTH * (num_channels.saturating_sub(1)) as u16)
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let buf = f.buffer_mut();

    let pattern = match app.core.song.pattern_at(app.current_order_position()) {
        Some(p) => p,
        // A song with no resolvable pattern at the cursor has nothing to
        // draw. Bail out rather than panicking inside the draw loop.
        None => return,
    };

    // Draw column headers on the first row
    let header_y = area.y;
    let header_style = Style::default()
        .fg(theme.row_bar)
        .add_modifier(Modifier::BOLD);
    let dim_header = Style::default().fg(theme.row_beat);
    write_str(buf, area.x, header_y, "   ", dim_header);
    write_str(
        buf,
        area.x + ROW_NUM_WIDTH,
        header_y,
        " | ",
        Style::default().fg(theme.separator),
    );
    {
        let mut hx = area.x + ROW_NUM_WIDTH + SEPARATOR_WIDTH;
        let visible = app.visible_channels();
        let first_visible = visible.start;
        for ch in visible.clone() {
            if ch > first_visible {
                write_str(
                    buf,
                    hx,
                    header_y,
                    " | ",
                    Style::default().fg(theme.separator),
                );
                hx += SEPARATOR_WIDTH;
            }
            let col_start = hx;
            let ch_name = app
                .core
                .channels
                .get(ch)
                .map(|c| &c.name)
                .filter(|n| !n.is_empty());
            // A linked track shows its phrase: P for aligned, R for running.
            let link = app
                .core
                .song
                .pattern_at(app.current_order_position())
                .and_then(|p| p.link(ch));
            let link_label = link.map(|l| {
                let kind = match l.timing {
                    rtrack_core::tracker::PhraseTiming::Aligned => 'P',
                    rtrack_core::tracker::PhraseTiming::Running => 'R',
                };
                format!("[{kind}{:02X}]", l.phrase.min(0xFF))
            });
            let type_label: &str = link_label
                .as_deref()
                .unwrap_or_else(|| app.core.track_label(ch));
            let type_style = Style::default()
                .fg(match type_label {
                    _ if link.is_some() => theme.mode_insert,
                    "[SMP]" => theme.effect_set,
                    "[SYN]" => theme.header_octave,
                    "[---]" => theme.muted_dim,
                    _ => theme.header_bpm,
                })
                .add_modifier(Modifier::BOLD);

            // Layout: <name> <pad> [TYP] M   (right-justified type + indicator)
            let w = channel_width(app.fx_columns()) as usize;
            let name_str: String = if let Some(name) = ch_name {
                name.chars().take(MAX_CHANNEL_NAME).collect()
            } else {
                format!("{}", ch + 1)
            };

            // Mute/solo indicator char
            let indicator = if app.core.solo_channel == Some(ch) {
                'S'
            } else if app.core.channels.get(ch).is_some_and(|c| c.muted) {
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
                .chars()
                .take(w)
                .collect();

            // Write full string in dim style as background
            write_str(buf, hx, header_y, &header_full, dim_header);
            // Overlay name in bold
            write_str(buf, hx, header_y, &name_str, header_style);
            // Overlay type label in color (right-justified)
            let type_x = hx + left_space as u16;
            write_str(buf, type_x, header_y, type_label, type_style);
            // Overlay indicator in color
            if indicator == 'S' {
                write_str(
                    buf,
                    hx + (w - 1) as u16,
                    header_y,
                    "S",
                    Style::default()
                        .fg(theme.mode_insert)
                        .add_modifier(Modifier::BOLD),
                );
            } else if indicator == 'M' {
                write_str(
                    buf,
                    hx + (w - 1) as u16,
                    header_y,
                    "M",
                    Style::default()
                        .fg(theme.muted_dim)
                        .add_modifier(Modifier::BOLD),
                );
            }

            hx = col_start + channel_width(app.fx_columns());
        }
    }

    // Pattern data starts one row below the header
    let data_area_y = area.y + 1;
    let data_area_height = area.height.saturating_sub(1) as usize;

    let visible_rows = data_area_height;
    let center_offset = visible_rows / 2;

    // Determine which rows to display, centered on cursor/playback position
    let focus_row = if app.core.is_playing() {
        app.playback_row()
    } else {
        app.cursor_row
    };

    let start_row = focus_row.saturating_sub(center_offset);

    for screen_y in 0..visible_rows {
        let row_idx = start_row + screen_y;
        if row_idx >= pattern.rows {
            break;
        }

        let y = data_area_y + screen_y as u16;
        if y >= data_area_y + data_area_height as u16 {
            break;
        }

        let is_cursor_row = !app.core.is_playing() && row_idx == app.cursor_row;
        let is_playback_row = app.core.is_playing() && row_idx == app.playback_row();
        let beat_interval = app.core.song.highlight_beat.max(1);
        let bar_interval = app.core.song.highlight_bar.max(1);
        let is_beat = row_idx % beat_interval == 0;
        let is_bar = row_idx % bar_interval == 0;

        // Row number
        let row_num_style = if is_bar {
            Style::default()
                .fg(theme.row_bar)
                .add_modifier(Modifier::BOLD)
        } else if is_beat {
            Style::default().fg(theme.row_beat)
        } else {
            Style::default().fg(theme.row_normal)
        };

        let row_str = format!("{:02X}", row_idx);
        write_str(buf, area.x, y, &row_str, row_num_style);
        write_str(
            buf,
            area.x + ROW_NUM_WIDTH,
            y,
            " | ",
            Style::default().fg(theme.separator),
        );

        let fx_columns = app.fx_columns();
        let mut x = area.x + ROW_NUM_WIDTH + SEPARATOR_WIDTH;

        let visible = app.visible_channels();
        let first_visible = visible.start;
        for ch in visible.clone() {
            if ch > first_visible {
                write_str(buf, x, y, " | ", Style::default().fg(theme.separator));
                x += SEPARATOR_WIDTH;
            }

            let cell = app
                .core
                .song
                .cell_at(app.current_order_position(), row_idx, ch);

            // Check if this cell is inside a block selection
            let in_block = app.block_bounds().is_some_and(|(r0, r1, c0, c1)| {
                row_idx >= r0 && row_idx <= r1 && ch >= c0 && ch <= c1
            });

            // Determine styles for each sub-column
            let base_bg = if is_playback_row {
                theme.playback_row_bg
            } else if in_block || is_cursor_row {
                theme.cursor_row_bg
            } else {
                ratatui::style::Color::Reset
            };

            let is_muted = !app.core.is_channel_audible(ch);

            let note_fg = if is_muted {
                theme.muted_dim
            } else if cell.note.is_some() {
                theme.note_set
            } else {
                theme.note_empty
            };
            let inst_fg = if is_muted {
                theme.muted_dim
            } else if cell.instrument.is_some() {
                theme.instrument_set
            } else {
                theme.instrument_empty
            };
            let vol_fg = if is_muted {
                theme.muted_dim
            } else if cell.volume.is_some() {
                theme.volume_set
            } else {
                theme.volume_empty
            };
            let fx_fg = |col: usize| {
                if is_muted {
                    theme.muted_dim
                } else if cell.fx[col].is_some() {
                    theme.effect_set
                } else {
                    theme.effect_empty
                }
            };

            // Highlight the cursor sub-column
            let cursor_on = |sub: SubColumn| -> Style {
                if is_cursor_row
                    && ch == app.cursor_channel
                    && app.cursor_sub == sub
                    && !app.core.is_playing()
                {
                    Style::default()
                        .bg(theme.cursor_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(base_bg)
                }
            };

            // Note
            let note_str = cell.display_note();
            write_str(buf, x, y, &note_str, cursor_on(SubColumn::Note).fg(note_fg));
            x += NOTE_WIDTH;

            write_str(buf, x, y, "  ", Style::default().bg(base_bg));
            x += 2;

            // Instrument
            let inst_str = cell.display_instrument();
            write_str(
                buf,
                x,
                y,
                &inst_str,
                cursor_on(SubColumn::Instrument).fg(inst_fg),
            );
            x += INST_WIDTH;

            write_str(buf, x, y, "  ", Style::default().bg(base_bg));
            x += 2;

            // Volume
            let vol_str = cell.display_volume();
            write_str(buf, x, y, &vol_str, cursor_on(SubColumn::Volume).fg(vol_fg));
            x += VOL_WIDTH;

            write_str(buf, x, y, "  ", Style::default().bg(base_bg));
            x += 2;

            // Effects
            for (col, sub) in [SubColumn::Effect, SubColumn::Effect2]
                .into_iter()
                .enumerate()
                .take(fx_columns)
            {
                if col > 0 {
                    write_str(buf, x, y, "  ", Style::default().bg(base_bg));
                    x += 2;
                }
                let fx_str = cell.display_effect(col);
                write_str(buf, x, y, &fx_str, cursor_on(sub).fg(fx_fg(col)));
                x += FX_WIDTH;
            }
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
        assert_eq!(SubColumn::Note.next(1), SubColumn::Instrument);
        assert_eq!(SubColumn::Instrument.next(1), SubColumn::Volume);
        assert_eq!(SubColumn::Volume.next(1), SubColumn::Effect);
        assert_eq!(SubColumn::Effect.next(1), SubColumn::Note);

        assert_eq!(SubColumn::Note.prev(1), SubColumn::Effect);
        assert_eq!(SubColumn::Effect.prev(1), SubColumn::Volume);
    }

    #[test]
    fn test_channel_total_width() {
        // 1 channel: 3 (row) + 3 (sep) + 18 (channel) = 24
        assert_eq!(channel_total_width(1, 1), 24);
        // 4 channels: 3 + 3 + 4*18 + 3*3 = 87
        assert_eq!(channel_total_width(4, 1), 87);
        // Two effect columns add a gap and a field: 18 + 7 = 25 per track
        assert_eq!(channel_total_width(4, 2), 115);
    }
}
