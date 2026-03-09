use ratatui::style::Color;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Theme {
    // Header
    pub header_title: Color,
    pub header_bpm: Color,
    pub header_position: Color,
    pub header_octave: Color,
    pub header_border: Color,

    // Pattern editor - row numbers
    pub row_bar: Color,
    pub row_beat: Color,
    pub row_normal: Color,

    // Pattern editor - cell data
    pub note_set: Color,
    pub note_empty: Color,
    pub instrument_set: Color,
    pub instrument_empty: Color,
    pub volume_set: Color,
    pub volume_empty: Color,
    pub effect_set: Color,
    pub effect_empty: Color,
    pub muted_dim: Color,

    // Pattern editor - cursor/highlight
    pub cursor_bg: Color,
    pub cursor_row_bg: Color,
    pub playback_row_bg: Color,
    pub separator: Color,

    // Order list sidebar
    pub order_current: Color,
    pub order_normal: Color,
    pub order_border: Color,

    // Status bar
    pub mode_normal: Color,
    pub mode_insert: Color,
    pub mode_port_select: Color,
    pub mode_help: Color,
    pub midi_connected: Color,
    pub midi_disconnected: Color,
    pub status_text: Color,
    pub status_hint: Color,

    // Link
    pub link_active: Color,
    pub link_inactive: Color,

    // Popups
    pub popup_border: Color,
    pub popup_title: Color,
    pub popup_highlight_fg: Color,
    pub popup_highlight_bg: Color,
    pub popup_text: Color,
    pub popup_key: Color,

    // Song settings dialog
    pub settings_label: Color,
    pub settings_value: Color,
    pub settings_active: Color,
}

#[allow(dead_code)]
impl Theme {
    pub fn dark() -> Self {
        Self {
            header_title: Color::Cyan,
            header_bpm: Color::Yellow,
            header_position: Color::Green,
            header_octave: Color::Magenta,
            header_border: Color::DarkGray,

            row_bar: Color::White,
            row_beat: Color::Yellow,
            row_normal: Color::DarkGray,

            note_set: Color::White,
            note_empty: Color::Rgb(60, 60, 60),
            instrument_set: Color::Yellow,
            instrument_empty: Color::Rgb(60, 60, 60),
            volume_set: Color::Green,
            volume_empty: Color::Rgb(60, 60, 60),
            effect_set: Color::Cyan,
            effect_empty: Color::Rgb(60, 60, 60),
            muted_dim: Color::Rgb(40, 40, 40),

            cursor_bg: Color::Rgb(80, 80, 160),
            cursor_row_bg: Color::Rgb(30, 30, 50),
            playback_row_bg: Color::DarkGray,
            separator: Color::DarkGray,

            order_current: Color::Cyan,
            order_normal: Color::DarkGray,
            order_border: Color::DarkGray,

            mode_normal: Color::Blue,
            mode_insert: Color::Red,
            mode_port_select: Color::Yellow,
            mode_help: Color::Cyan,
            midi_connected: Color::Green,
            midi_disconnected: Color::DarkGray,
            status_text: Color::White,
            status_hint: Color::DarkGray,

            link_active: Color::Rgb(255, 100, 0),
            link_inactive: Color::DarkGray,

            popup_border: Color::Cyan,
            popup_title: Color::Cyan,
            popup_highlight_fg: Color::Black,
            popup_highlight_bg: Color::Cyan,
            popup_text: Color::White,
            popup_key: Color::Yellow,

            settings_label: Color::DarkGray,
            settings_value: Color::White,
            settings_active: Color::Cyan,
        }
    }

    pub fn light() -> Self {
        Self {
            header_title: Color::Blue,
            header_bpm: Color::DarkGray,
            header_position: Color::DarkGray,
            header_octave: Color::Magenta,
            header_border: Color::Gray,

            row_bar: Color::Black,
            row_beat: Color::DarkGray,
            row_normal: Color::Gray,

            note_set: Color::Black,
            note_empty: Color::Rgb(200, 200, 200),
            instrument_set: Color::DarkGray,
            instrument_empty: Color::Rgb(200, 200, 200),
            volume_set: Color::DarkGray,
            volume_empty: Color::Rgb(200, 200, 200),
            effect_set: Color::Blue,
            effect_empty: Color::Rgb(200, 200, 200),
            muted_dim: Color::Rgb(220, 220, 220),

            cursor_bg: Color::Rgb(180, 180, 240),
            cursor_row_bg: Color::Rgb(230, 230, 245),
            playback_row_bg: Color::Rgb(220, 220, 220),
            separator: Color::Gray,

            order_current: Color::Blue,
            order_normal: Color::Gray,
            order_border: Color::Gray,

            mode_normal: Color::Blue,
            mode_insert: Color::Red,
            mode_port_select: Color::DarkGray,
            mode_help: Color::Blue,
            midi_connected: Color::Green,
            midi_disconnected: Color::Gray,
            status_text: Color::Black,
            status_hint: Color::Gray,

            link_active: Color::Rgb(200, 80, 0),
            link_inactive: Color::Gray,

            popup_border: Color::Blue,
            popup_title: Color::Blue,
            popup_highlight_fg: Color::White,
            popup_highlight_bg: Color::Blue,
            popup_text: Color::Black,
            popup_key: Color::DarkGray,

            settings_label: Color::Gray,
            settings_value: Color::Black,
            settings_active: Color::Blue,
        }
    }

    pub fn monokai() -> Self {
        Self {
            header_title: Color::Rgb(102, 217, 239),
            header_bpm: Color::Rgb(230, 219, 116),
            header_position: Color::Rgb(166, 226, 46),
            header_octave: Color::Rgb(174, 129, 255),
            header_border: Color::Rgb(117, 113, 94),

            row_bar: Color::Rgb(248, 248, 242),
            row_beat: Color::Rgb(230, 219, 116),
            row_normal: Color::Rgb(117, 113, 94),

            note_set: Color::Rgb(248, 248, 242),
            note_empty: Color::Rgb(70, 68, 60),
            instrument_set: Color::Rgb(230, 219, 116),
            instrument_empty: Color::Rgb(70, 68, 60),
            volume_set: Color::Rgb(166, 226, 46),
            volume_empty: Color::Rgb(70, 68, 60),
            effect_set: Color::Rgb(102, 217, 239),
            effect_empty: Color::Rgb(70, 68, 60),
            muted_dim: Color::Rgb(50, 48, 42),

            cursor_bg: Color::Rgb(80, 78, 120),
            cursor_row_bg: Color::Rgb(50, 48, 55),
            playback_row_bg: Color::Rgb(60, 58, 50),
            separator: Color::Rgb(117, 113, 94),

            order_current: Color::Rgb(102, 217, 239),
            order_normal: Color::Rgb(117, 113, 94),
            order_border: Color::Rgb(117, 113, 94),

            mode_normal: Color::Rgb(102, 217, 239),
            mode_insert: Color::Rgb(249, 38, 114),
            mode_port_select: Color::Rgb(230, 219, 116),
            mode_help: Color::Rgb(102, 217, 239),
            midi_connected: Color::Rgb(166, 226, 46),
            midi_disconnected: Color::Rgb(117, 113, 94),
            status_text: Color::Rgb(248, 248, 242),
            status_hint: Color::Rgb(117, 113, 94),

            link_active: Color::Rgb(253, 151, 31),
            link_inactive: Color::Rgb(117, 113, 94),

            popup_border: Color::Rgb(102, 217, 239),
            popup_title: Color::Rgb(102, 217, 239),
            popup_highlight_fg: Color::Rgb(39, 40, 34),
            popup_highlight_bg: Color::Rgb(102, 217, 239),
            popup_text: Color::Rgb(248, 248, 242),
            popup_key: Color::Rgb(230, 219, 116),

            settings_label: Color::Rgb(117, 113, 94),
            settings_value: Color::Rgb(248, 248, 242),
            settings_active: Color::Rgb(102, 217, 239),
        }
    }
}

pub const THEME_NAMES: &[&str] = &["dark", "light", "monokai"];

#[allow(dead_code)]
pub fn theme_by_name(name: &str) -> Theme {
    match name {
        "light" => Theme::light(),
        "monokai" => Theme::monokai(),
        _ => Theme::dark(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_by_name() {
        let _dark = theme_by_name("dark");
        let _light = theme_by_name("light");
        let _monokai = theme_by_name("monokai");
        let _fallback = theme_by_name("nonexistent");
    }

    #[test]
    fn test_theme_names_consistent() {
        for name in THEME_NAMES {
            let _ = theme_by_name(name);
        }
    }
}
