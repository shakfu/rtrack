use egui::{FontId, Pos2, Rect, Ui};
use rtrack_core::tracker::Pattern;

use crate::state::{GridColors, Mode, SubColumn};

#[derive(Debug)]
pub enum GridAction {
    SetCursor {
        row: usize,
        channel: usize,
        sub: SubColumn,
    },
    Scroll {
        rows: i32,
    },
    DragStart {
        row: usize,
        channel: usize,
    },
    DragUpdate {
        row: usize,
        channel: usize,
    },
}

const CHAR_WIDTH: f32 = 8.4;
const ROW_HEIGHT: f32 = 18.0;
const NOTE_CHARS: usize = 3;
const INST_CHARS: usize = 2;
const VOL_CHARS: usize = 2;
const FX_CHARS: usize = 3;
const GAP_CHARS: usize = 2;
const ROW_NUM_CHARS: usize = 3;
const SEPARATOR_CHARS: usize = 3;

fn channel_width_chars() -> usize {
    NOTE_CHARS + GAP_CHARS + INST_CHARS + GAP_CHARS + VOL_CHARS + GAP_CHARS + FX_CHARS
}

/// How many channels fit in the given pixel width.
pub fn max_visible_channels(available_width: f32) -> usize {
    let row_num_px = (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;
    let ch_px = (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;
    let count = ((available_width - row_num_px) / ch_px).floor() as usize;
    count.max(1)
}

#[allow(dead_code)]
pub struct GridParams {
    pub cursor_row: usize,
    pub cursor_channel: usize,
    pub cursor_sub: SubColumn,
    pub mode: Mode,
    pub playing: bool,
    pub playback_row: usize,
    pub playback_order: usize,
    pub edit_order: usize,
    pub highlight_beat: usize,
    pub highlight_bar: usize,
    pub first_visible_channel: usize,
    pub visible_channel_count: usize,
    pub muted_channels: Vec<bool>,
    pub solo_channel: Option<usize>,
    pub channel_names: Vec<String>,
    pub block_start: Option<(usize, usize)>,
    pub block_end: Option<(usize, usize)>,
    pub colors: GridColors,
}

/// Where the grid was laid out, as far as hit-testing needs to know.
///
/// Pulled out of `draw_grid` so the pointer-to-cell mapping is a function of
/// numbers rather than of a live `Ui`. It is pure arithmetic over layout
/// constants, it decides what every click in the editor does, and it silently
/// stops matching the drawing if one of those constants moves -- which is a
/// combination that wants a test more than most of the file does.
#[derive(Debug, Clone, Copy)]
pub struct GridGeometry {
    /// Screen y of the first data row, below the header.
    pub data_top: f32,
    /// Screen x of the grid's left edge, before the row-number gutter.
    pub left: f32,
    /// Pattern row drawn at the top of the view.
    pub start_row: usize,
    /// Rows in the pattern being drawn.
    pub rows: usize,
    /// First and one-past-last channel currently drawn.
    pub first_channel: usize,
    pub last_channel: usize,
}

/// Map a pointer position to the cell under it.
///
/// `None` for anything outside the data area: the header, the row-number
/// gutter, past the last row, or past the last drawn channel.
pub fn hit_test(pos: Pos2, geom: &GridGeometry) -> Option<(usize, usize, SubColumn)> {
    if pos.y < geom.data_top {
        return None;
    }
    let screen_row = ((pos.y - geom.data_top) / ROW_HEIGHT) as usize;
    let row_idx = geom.start_row + screen_row;
    if row_idx >= geom.rows {
        return None;
    }

    let channels_start_x = geom.left + (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;
    let channel_total_width = (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;

    let rel_x = pos.x - channels_start_x;
    if rel_x < 0.0 {
        return None;
    }
    let ch_offset = (rel_x / channel_total_width) as usize;
    let ch_idx = geom.first_channel + ch_offset;
    if ch_idx >= geom.last_channel {
        return None;
    }

    let within_ch = rel_x - ch_offset as f32 * channel_total_width;
    let note_end = NOTE_CHARS as f32 * CHAR_WIDTH;
    let inst_start = (NOTE_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;
    let inst_end = inst_start + INST_CHARS as f32 * CHAR_WIDTH;
    let vol_start = inst_start + (INST_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;
    let vol_end = vol_start + VOL_CHARS as f32 * CHAR_WIDTH;
    let fx_start = vol_start + (VOL_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

    // The gaps between fields, and the separator past the effect column,
    // resolve to the field on their left rather than to nothing: a click a
    // pixel wide of a column should still land somewhere.
    let sub = if within_ch < note_end {
        SubColumn::Note
    } else if within_ch >= inst_start && within_ch < inst_end {
        SubColumn::Instrument
    } else if within_ch >= vol_start && within_ch < vol_end {
        SubColumn::Volume
    } else if within_ch >= fx_start {
        SubColumn::Effect
    } else if within_ch < inst_start {
        SubColumn::Note
    } else if within_ch < vol_start {
        SubColumn::Instrument
    } else {
        SubColumn::Volume
    };

    Some((row_idx, ch_idx, sub))
}

pub fn draw_grid(ui: &mut Ui, pattern: &Pattern, params: &GridParams) -> Vec<GridAction> {
    let font = FontId::monospace(13.0);
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = response.rect;
    let c = &params.colors;

    // Fill background
    painter.rect_filled(rect, 0.0, c.bg_normal);

    // Calculate layout
    let first_ch = params.first_visible_channel;
    let last_ch = (first_ch + params.visible_channel_count).min(pattern.channels);
    let header_height = ROW_HEIGHT;

    // Draw channel headers
    for ch_idx in first_ch..last_ch {
        let ch_offset = ch_idx - first_ch;
        let x = rect.left()
            + (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH
            + ch_offset as f32 * (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;

        let default_name = format!("Ch{}", ch_idx + 1);
        let name = params
            .channel_names
            .get(ch_idx)
            .filter(|n| !n.is_empty())
            .map(|n| n.as_str())
            .unwrap_or(&default_name);
        let header = format!("{:^width$}", name, width = channel_width_chars());
        painter.text(
            Pos2::new(x, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            &header,
            font.clone(),
            c.fg_header,
        );
    }

    // Pattern rows
    let data_top = rect.top() + header_height;
    let visible_rows = ((rect.height() - header_height) / ROW_HEIGHT) as usize;
    let center = visible_rows / 2;

    let focus_row = if params.playing {
        params.playback_row
    } else {
        params.cursor_row
    };
    let start_row = focus_row.saturating_sub(center);

    let beat = params.highlight_beat.max(1);
    let bar = params.highlight_bar.max(1);

    for screen_y in 0..visible_rows {
        let row_idx = start_row + screen_y;
        if row_idx >= pattern.rows {
            break;
        }

        let y = data_top + screen_y as f32 * ROW_HEIGHT;
        let is_cursor_row = !params.playing && row_idx == params.cursor_row;
        let is_playback_row = params.playing && row_idx == params.playback_row;
        let is_beat = row_idx % beat == 0;
        let is_bar = row_idx % bar == 0;

        // Row background
        let row_bg = if is_playback_row {
            c.bg_playback_row
        } else if is_cursor_row {
            c.bg_cursor_row
        } else if is_bar {
            c.bg_bar
        } else if is_beat {
            c.bg_beat
        } else {
            c.bg_normal
        };

        let row_rect = Rect::from_min_size(
            Pos2::new(rect.left(), y),
            egui::vec2(rect.width(), ROW_HEIGHT),
        );
        painter.rect_filled(row_rect, 0.0, row_bg);

        // Row number
        let row_text = format!("{:02X}", row_idx);
        let row_fg = if is_bar { c.fg_row_bar } else { c.fg_row_num };
        painter.text(
            Pos2::new(rect.left() + CHAR_WIDTH * 0.5, y + 2.0),
            egui::Align2::LEFT_TOP,
            &row_text,
            font.clone(),
            row_fg,
        );

        // Separator after row number
        let sep_x = rect.left() + ROW_NUM_CHARS as f32 * CHAR_WIDTH;
        painter.text(
            Pos2::new(sep_x, y + 2.0),
            egui::Align2::LEFT_TOP,
            " | ",
            font.clone(),
            c.fg_separator,
        );

        // Block selection range
        let block_range = match (params.block_start, params.block_end) {
            (Some((r1, c1)), Some((r2, c2))) => {
                let min_r = r1.min(r2);
                let max_r = r1.max(r2);
                let min_c = c1.min(c2);
                let max_c = c1.max(c2);
                Some((min_r, max_r, min_c, max_c))
            }
            _ => None,
        };

        // Channels
        for ch_idx in first_ch..last_ch {
            let ch_offset = ch_idx - first_ch;
            let base_x = rect.left()
                + (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH
                + ch_offset as f32 * (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;

            // Block highlight overlay for this cell
            if let Some((min_r, max_r, min_c, max_c)) = block_range {
                if row_idx >= min_r && row_idx <= max_r && ch_idx >= min_c && ch_idx <= max_c {
                    let block_rect = Rect::from_min_size(
                        Pos2::new(base_x, y),
                        egui::vec2(channel_width_chars() as f32 * CHAR_WIDTH, ROW_HEIGHT),
                    );
                    painter.rect_filled(block_rect, 0.0, c.bg_block);
                }
            }

            // Channel separator (except first)
            if ch_offset > 0 {
                let sx = base_x - SEPARATOR_CHARS as f32 * CHAR_WIDTH;
                painter.text(
                    Pos2::new(sx, y + 2.0),
                    egui::Align2::LEFT_TOP,
                    " | ",
                    font.clone(),
                    c.fg_separator,
                );
            }

            let cell = pattern.get(row_idx, ch_idx);
            let is_muted = if let Some(solo) = params.solo_channel {
                ch_idx != solo
            } else {
                *params.muted_channels.get(ch_idx).unwrap_or(&false)
            };

            // Sub-column texts and colors
            let note_text = cell.display_note();
            let inst_text = cell.display_instrument();
            let vol_text = cell.display_volume();
            let fx_text = cell.display_effect();

            let note_fg = if is_muted {
                c.fg_muted
            } else if cell.note.is_some() {
                c.fg_note_set
            } else {
                c.fg_note_empty
            };
            let inst_fg = if is_muted {
                c.fg_muted
            } else if cell.instrument.is_some() {
                c.fg_inst_set
            } else {
                c.fg_inst_empty
            };
            let vol_fg = if is_muted {
                c.fg_muted
            } else if cell.volume.is_some() {
                c.fg_vol_set
            } else {
                c.fg_vol_empty
            };
            let fx_fg = if is_muted {
                c.fg_muted
            } else if cell.effect.is_some() || cell.effect_value.is_some() {
                c.fg_fx_set
            } else {
                c.fg_fx_empty
            };

            // Positions
            let mut x = base_x;

            // Cursor highlight
            let draw_sub = |px: f32, chars: usize, sub: SubColumn, p: &egui::Painter| {
                if is_cursor_row
                    && ch_idx == params.cursor_channel
                    && params.cursor_sub == sub
                    && !params.playing
                {
                    let highlight = Rect::from_min_size(
                        Pos2::new(px - 1.0, y),
                        egui::vec2(chars as f32 * CHAR_WIDTH + 2.0, ROW_HEIGHT),
                    );
                    p.rect_filled(highlight, 0.0, c.bg_cursor_cell);
                }
            };

            // Note
            draw_sub(x, NOTE_CHARS, SubColumn::Note, &painter);
            painter.text(
                Pos2::new(x, y + 2.0),
                egui::Align2::LEFT_TOP,
                &note_text,
                font.clone(),
                note_fg,
            );
            x += (NOTE_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Instrument
            draw_sub(x, INST_CHARS, SubColumn::Instrument, &painter);
            painter.text(
                Pos2::new(x, y + 2.0),
                egui::Align2::LEFT_TOP,
                &inst_text,
                font.clone(),
                inst_fg,
            );
            x += (INST_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Volume
            draw_sub(x, VOL_CHARS, SubColumn::Volume, &painter);
            painter.text(
                Pos2::new(x, y + 2.0),
                egui::Align2::LEFT_TOP,
                &vol_text,
                font.clone(),
                vol_fg,
            );
            x += (VOL_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Effect
            draw_sub(x, FX_CHARS, SubColumn::Effect, &painter);
            painter.text(
                Pos2::new(x, y + 2.0),
                egui::Align2::LEFT_TOP,
                &fx_text,
                font.clone(),
                fx_fg,
            );
        }
    }

    // Handle mouse interactions
    let mut actions = Vec::new();

    // Scroll wheel
    let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll_y.abs() > 1.0 {
        let rows = -(scroll_y / ROW_HEIGHT).round() as i32;
        if rows != 0 {
            actions.push(GridAction::Scroll { rows });
        }
    }

    // Helper: convert pointer position to (row, channel, sub-column)
    let geom = GridGeometry {
        data_top,
        left: rect.left(),
        start_row,
        rows: pattern.rows,
        first_channel: first_ch,
        last_channel: last_ch,
    };
    let hit_test = |pos: Pos2| hit_test(pos, &geom);

    // Click to set cursor (non-drag click)
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some((row, channel, sub)) = hit_test(pos) {
                actions.push(GridAction::SetCursor { row, channel, sub });
            }
        }
    }

    // Drag to select block
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some((row, channel, _sub)) = hit_test(pos) {
                actions.push(GridAction::DragStart { row, channel });
            }
        }
    }
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some((row, channel, _sub)) = hit_test(pos) {
                actions.push(GridAction::DragUpdate { row, channel });
            }
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid drawn at the origin: header 40px tall, showing rows 0..64 of a
    /// 64-row pattern and channels 0..4.
    fn geom() -> GridGeometry {
        GridGeometry {
            data_top: 40.0,
            left: 0.0,
            start_row: 0,
            rows: 64,
            first_channel: 0,
            last_channel: 4,
        }
    }

    /// Screen x of the start of `channel`'s note column.
    fn channel_x(channel: usize) -> f32 {
        (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH
            + channel as f32 * (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH
    }

    fn row_y(row: usize) -> f32 {
        40.0 + row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0
    }

    #[test]
    fn a_click_in_the_header_hits_nothing() {
        assert!(hit_test(Pos2::new(100.0, 0.0), &geom()).is_none());
        assert!(hit_test(Pos2::new(100.0, 39.9), &geom()).is_none());
    }

    #[test]
    fn a_click_in_the_row_number_gutter_hits_nothing() {
        // The gutter is the row number plus the separator after it; the
        // channels do not start until past both.
        assert!(hit_test(Pos2::new(0.0, row_y(3)), &geom()).is_none());
        assert!(hit_test(Pos2::new(channel_x(0) - 1.0, row_y(3)), &geom()).is_none());
    }

    #[test]
    fn a_click_past_the_last_row_hits_nothing() {
        assert!(hit_test(Pos2::new(channel_x(0), row_y(64)), &geom()).is_none());
    }

    #[test]
    fn a_click_past_the_last_visible_channel_hits_nothing() {
        assert!(hit_test(Pos2::new(channel_x(4), row_y(0)), &geom()).is_none());
    }

    #[test]
    fn each_row_maps_to_itself() {
        for row in [0usize, 1, 17, 63] {
            let hit = hit_test(Pos2::new(channel_x(0) + 1.0, row_y(row)), &geom());
            assert_eq!(hit.map(|(r, _, _)| r), Some(row), "row {row}");
        }
    }

    #[test]
    fn each_channel_maps_to_itself() {
        for ch in 0..4 {
            let hit = hit_test(Pos2::new(channel_x(ch) + 1.0, row_y(0)), &geom());
            assert_eq!(hit.map(|(_, c, _)| c), Some(ch), "channel {ch}");
        }
    }

    /// Horizontal scrolling offsets which pattern channel a screen column is.
    #[test]
    fn a_scrolled_view_maps_screen_columns_to_the_channels_drawn() {
        let scrolled = GridGeometry {
            first_channel: 4,
            last_channel: 8,
            ..geom()
        };
        // The leftmost drawn column is channel 4, not channel 0.
        assert_eq!(
            hit_test(Pos2::new(channel_x(0) + 1.0, row_y(0)), &scrolled).map(|(_, c, _)| c),
            Some(4)
        );
        assert_eq!(
            hit_test(Pos2::new(channel_x(3) + 1.0, row_y(0)), &scrolled).map(|(_, c, _)| c),
            Some(7)
        );
        assert!(hit_test(Pos2::new(channel_x(4), row_y(0)), &scrolled).is_none());
    }

    /// Vertical scrolling does the same for rows, and the end of the pattern
    /// is still the end.
    #[test]
    fn a_scrolled_view_maps_screen_rows_to_the_rows_drawn() {
        let scrolled = GridGeometry {
            start_row: 32,
            ..geom()
        };
        assert_eq!(
            hit_test(Pos2::new(channel_x(0) + 1.0, row_y(0)), &scrolled).map(|(r, _, _)| r),
            Some(32)
        );
        assert_eq!(
            hit_test(Pos2::new(channel_x(0) + 1.0, row_y(31)), &scrolled).map(|(r, _, _)| r),
            Some(63)
        );
        assert!(hit_test(Pos2::new(channel_x(0) + 1.0, row_y(32)), &scrolled).is_none());
    }

    /// The four fields of a cell, each hit at its own midpoint. This is the
    /// mapping that decides whether typing a hex digit edits the volume or
    /// the effect, so it is worth pinning field by field.
    #[test]
    fn each_sub_column_is_reachable_at_its_own_midpoint() {
        let base = channel_x(1);
        let mid = |start_chars: usize, width_chars: usize| {
            base + (start_chars as f32 + width_chars as f32 / 2.0) * CHAR_WIDTH
        };

        let note = mid(0, NOTE_CHARS);
        let inst = mid(NOTE_CHARS + GAP_CHARS, INST_CHARS);
        let vol = mid(NOTE_CHARS + GAP_CHARS + INST_CHARS + GAP_CHARS, VOL_CHARS);
        let fx = mid(
            NOTE_CHARS + GAP_CHARS + INST_CHARS + GAP_CHARS + VOL_CHARS + GAP_CHARS,
            FX_CHARS,
        );

        for (x, expected) in [
            (note, SubColumn::Note),
            (inst, SubColumn::Instrument),
            (vol, SubColumn::Volume),
            (fx, SubColumn::Effect),
        ] {
            let hit = hit_test(Pos2::new(x, row_y(2)), &geom());
            assert_eq!(hit, Some((2, 1, expected)), "at x={x}");
        }
    }

    /// A click in the gap between two fields belongs to the field on its
    /// left. Recorded because it is a choice, not an accident: the
    /// alternative -- returning `None` -- would make a one-pixel miss do
    /// nothing at all.
    #[test]
    fn a_click_in_the_gap_between_fields_takes_the_field_to_its_left() {
        let base = channel_x(0);
        let gap_after_note = base + (NOTE_CHARS as f32 + 0.5) * CHAR_WIDTH;
        assert_eq!(
            hit_test(Pos2::new(gap_after_note, row_y(0)), &geom()),
            Some((0, 0, SubColumn::Note))
        );

        let gap_after_inst =
            base + ((NOTE_CHARS + GAP_CHARS + INST_CHARS) as f32 + 0.5) * CHAR_WIDTH;
        assert_eq!(
            hit_test(Pos2::new(gap_after_inst, row_y(0)), &geom()),
            Some((0, 0, SubColumn::Instrument))
        );
    }

    /// `max_visible_channels` is the other half of the same geometry: what it
    /// returns has to be a width `hit_test` agrees is inside the grid.
    #[test]
    fn the_visible_channel_count_agrees_with_the_hit_test() {
        for width in [400.0f32, 800.0, 1200.0, 1920.0] {
            let count = max_visible_channels(width);
            let g = GridGeometry {
                first_channel: 0,
                last_channel: count,
                ..geom()
            };
            // The last channel it claims fits must be hittable...
            assert!(
                hit_test(Pos2::new(channel_x(count - 1) + 1.0, row_y(0)), &g).is_some(),
                "channel {} reported visible at width {width} but is not hittable",
                count - 1
            );
            // ...and its right edge must be within the width offered.
            let used = channel_x(count - 1)
                + (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;
            assert!(
                used <= width || count == 1,
                "{count} channels need {used}px but only {width}px was offered"
            );
        }
    }

    #[test]
    fn a_degenerate_width_still_reports_one_channel() {
        // Narrower than the gutter: the count is clamped rather than zero or
        // a wrapped subtraction.
        assert_eq!(max_visible_channels(0.0), 1);
        assert_eq!(max_visible_channels(-100.0), 1);
    }
}
