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
}

const CHAR_WIDTH: f32 = 8.4;
const ROW_HEIGHT: f32 = 18.0;
const NOTE_CHARS: usize = 3;
const INST_CHARS: usize = 2;
const VOL_CHARS: usize = 2;
const FX_CHARS: usize = 3;
const GAP_CHARS: usize = 1;
const ROW_NUM_CHARS: usize = 3;
const SEPARATOR_CHARS: usize = 3;

fn channel_width_chars() -> usize {
    NOTE_CHARS + GAP_CHARS + INST_CHARS + GAP_CHARS + VOL_CHARS + GAP_CHARS + FX_CHARS
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
    pub colors: GridColors,
}

pub fn draw_grid(ui: &mut Ui, pattern: &Pattern, params: &GridParams) -> Vec<GridAction> {
    let font = FontId::monospace(13.0);
    let available = ui.available_size();
    let (response, painter) =
        ui.allocate_painter(available, egui::Sense::click());
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
            + ch_offset as f32
                * (channel_width_chars() + SEPARATOR_CHARS) as f32
                * CHAR_WIDTH;

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
        let is_playback_row =
            params.playing && row_idx == params.playback_row;
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

        // Channels
        for ch_idx in first_ch..last_ch {
            let ch_offset = ch_idx - first_ch;
            let base_x = rect.left()
                + (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH
                + ch_offset as f32
                    * (channel_width_chars() + SEPARATOR_CHARS) as f32
                    * CHAR_WIDTH;

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

            let note_fg = if is_muted { c.fg_muted } else if cell.note.is_some() { c.fg_note_set } else { c.fg_note_empty };
            let inst_fg = if is_muted { c.fg_muted } else if cell.instrument.is_some() { c.fg_inst_set } else { c.fg_inst_empty };
            let vol_fg = if is_muted { c.fg_muted } else if cell.volume.is_some() { c.fg_vol_set } else { c.fg_vol_empty };
            let fx_fg = if is_muted { c.fg_muted } else if cell.effect.is_some() || cell.effect_value.is_some() { c.fg_fx_set } else { c.fg_fx_empty };

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
            painter.text(Pos2::new(x, y + 2.0), egui::Align2::LEFT_TOP, &note_text, font.clone(), note_fg);
            x += (NOTE_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Instrument
            draw_sub(x, INST_CHARS, SubColumn::Instrument, &painter);
            painter.text(Pos2::new(x, y + 2.0), egui::Align2::LEFT_TOP, &inst_text, font.clone(), inst_fg);
            x += (INST_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Volume
            draw_sub(x, VOL_CHARS, SubColumn::Volume, &painter);
            painter.text(Pos2::new(x, y + 2.0), egui::Align2::LEFT_TOP, &vol_text, font.clone(), vol_fg);
            x += (VOL_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

            // Effect
            draw_sub(x, FX_CHARS, SubColumn::Effect, &painter);
            painter.text(Pos2::new(x, y + 2.0), egui::Align2::LEFT_TOP, &fx_text, font.clone(), fx_fg);
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

    // Click to set cursor
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let click_y = pos.y;
            let click_x = pos.x;

            // Determine row
            if click_y >= data_top {
                let screen_row = ((click_y - data_top) / ROW_HEIGHT) as usize;
                let row_idx = start_row + screen_row;
                if row_idx < pattern.rows {
                    // Determine channel and sub-column
                    let channels_start_x =
                        rect.left() + (ROW_NUM_CHARS + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;
                    let channel_total_width =
                        (channel_width_chars() + SEPARATOR_CHARS) as f32 * CHAR_WIDTH;

                    let rel_x = click_x - channels_start_x;
                    if rel_x >= 0.0 {
                        let ch_offset = (rel_x / channel_total_width) as usize;
                        let ch_idx = first_ch + ch_offset;

                        if ch_idx < last_ch {
                            // Determine sub-column within channel
                            let within_ch = rel_x - ch_offset as f32 * channel_total_width;
                            let note_end = NOTE_CHARS as f32 * CHAR_WIDTH;
                            let inst_start = (NOTE_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;
                            let inst_end = inst_start + INST_CHARS as f32 * CHAR_WIDTH;
                            let vol_start =
                                inst_start + (INST_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;
                            let vol_end = vol_start + VOL_CHARS as f32 * CHAR_WIDTH;
                            let fx_start =
                                vol_start + (VOL_CHARS + GAP_CHARS) as f32 * CHAR_WIDTH;

                            let sub = if within_ch < note_end {
                                SubColumn::Note
                            } else if within_ch >= inst_start && within_ch < inst_end {
                                SubColumn::Instrument
                            } else if within_ch >= vol_start && within_ch < vol_end {
                                SubColumn::Volume
                            } else if within_ch >= fx_start {
                                SubColumn::Effect
                            } else {
                                // In a gap region, snap to nearest sub-column
                                if within_ch < inst_start {
                                    SubColumn::Note
                                } else if within_ch < vol_start {
                                    SubColumn::Instrument
                                } else {
                                    SubColumn::Volume
                                }
                            };

                            actions.push(GridAction::SetCursor {
                                row: row_idx,
                                channel: ch_idx,
                                sub,
                            });
                        }
                    }
                }
            }
        }
    }

    actions
}
