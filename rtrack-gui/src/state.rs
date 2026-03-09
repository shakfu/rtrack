use egui::Color32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn toggle(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }
}

/// Grid color palette, derived from the active theme.
#[derive(Clone, Copy)]
pub struct GridColors {
    pub bg_normal: Color32,
    pub bg_cursor_row: Color32,
    pub bg_playback_row: Color32,
    pub bg_cursor_cell: Color32,
    pub bg_beat: Color32,
    pub bg_bar: Color32,
    pub bg_block: Color32,

    pub fg_row_num: Color32,
    pub fg_row_bar: Color32,
    pub fg_note_set: Color32,
    pub fg_note_empty: Color32,
    pub fg_inst_set: Color32,
    pub fg_inst_empty: Color32,
    pub fg_vol_set: Color32,
    pub fg_vol_empty: Color32,
    pub fg_fx_set: Color32,
    pub fg_fx_empty: Color32,
    pub fg_separator: Color32,
    pub fg_muted: Color32,
    pub fg_header: Color32,
}

impl GridColors {
    pub fn dark() -> Self {
        Self {
            bg_normal: Color32::from_rgb(24, 24, 32),
            bg_cursor_row: Color32::from_rgb(40, 40, 60),
            bg_playback_row: Color32::from_rgb(60, 40, 30),
            bg_cursor_cell: Color32::from_rgb(60, 60, 100),
            bg_beat: Color32::from_rgb(28, 28, 38),
            bg_bar: Color32::from_rgb(32, 32, 44),
            bg_block: Color32::from_rgba_premultiplied(80, 120, 200, 60),

            fg_row_num: Color32::from_rgb(100, 100, 120),
            fg_row_bar: Color32::from_rgb(160, 160, 200),
            fg_note_set: Color32::from_rgb(180, 220, 255),
            fg_note_empty: Color32::from_rgb(60, 60, 80),
            fg_inst_set: Color32::from_rgb(255, 200, 100),
            fg_inst_empty: Color32::from_rgb(60, 60, 80),
            fg_vol_set: Color32::from_rgb(100, 255, 100),
            fg_vol_empty: Color32::from_rgb(60, 60, 80),
            fg_fx_set: Color32::from_rgb(255, 150, 150),
            fg_fx_empty: Color32::from_rgb(60, 60, 80),
            fg_separator: Color32::from_rgb(50, 50, 70),
            fg_muted: Color32::from_rgb(50, 50, 60),
            fg_header: Color32::from_rgb(140, 140, 180),
        }
    }

    pub fn light() -> Self {
        Self {
            bg_normal: Color32::from_rgb(245, 245, 248),
            bg_cursor_row: Color32::from_rgb(210, 220, 240),
            bg_playback_row: Color32::from_rgb(240, 215, 200),
            bg_cursor_cell: Color32::from_rgb(180, 195, 230),
            bg_beat: Color32::from_rgb(235, 235, 242),
            bg_bar: Color32::from_rgb(225, 225, 235),
            bg_block: Color32::from_rgba_premultiplied(80, 120, 200, 40),

            fg_row_num: Color32::from_rgb(140, 140, 160),
            fg_row_bar: Color32::from_rgb(60, 60, 100),
            fg_note_set: Color32::from_rgb(30, 80, 160),
            fg_note_empty: Color32::from_rgb(190, 190, 200),
            fg_inst_set: Color32::from_rgb(180, 120, 20),
            fg_inst_empty: Color32::from_rgb(190, 190, 200),
            fg_vol_set: Color32::from_rgb(30, 140, 30),
            fg_vol_empty: Color32::from_rgb(190, 190, 200),
            fg_fx_set: Color32::from_rgb(180, 50, 50),
            fg_fx_empty: Color32::from_rgb(190, 190, 200),
            fg_separator: Color32::from_rgb(180, 180, 200),
            fg_muted: Color32::from_rgb(200, 200, 210),
            fg_header: Color32::from_rgb(80, 80, 120),
        }
    }

    pub fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        }
    }
}

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
            SubColumn::Note => SubColumn::Instrument,
            SubColumn::Instrument => SubColumn::Volume,
            SubColumn::Volume => SubColumn::Effect,
            SubColumn::Effect => SubColumn::Note,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SubColumn::Note => SubColumn::Effect,
            SubColumn::Instrument => SubColumn::Note,
            SubColumn::Volume => SubColumn::Instrument,
            SubColumn::Effect => SubColumn::Volume,
        }
    }
}
