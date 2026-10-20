use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{App, Mode, SampleField, SettingsField, SubColumn};
use rtrack_core::constants::*;
use rtrack_core::theory::ScaleSetting;
use rtrack_core::tracker::{Fx, Note, Song};
use rtrack_core::{InstrumentKind, MidiOutput};

impl App {
    // -- Song settings dialog --

    pub(crate) fn open_song_settings(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::SongSettings;
        self.settings_select_field(SettingsField::Title);
    }

    fn close_song_settings(&mut self) {
        self.mode = Mode::Normal;
        // A later visit's change is a step of its own.
        self.break_edit_coalescing();
    }

    /// The song's current value for a settings field, as the edit buffer holds
    /// it. Seeds the buffer on selection and decides whether a field changed.
    fn settings_field_value(&self, field: SettingsField) -> String {
        match field {
            SettingsField::Title => self.core.song.title.clone(),
            SettingsField::Bpm => self.core.song.bpm.to_string(),
            SettingsField::Speed => self.core.song.speed.to_string(),
            SettingsField::Channels => self.core.song.channels.to_string(),
            SettingsField::Rows => self.core.song.rows_per_pattern.to_string(),
            SettingsField::HighlightBeat => self.core.song.highlight_beat.to_string(),
            SettingsField::HighlightBar => self.core.song.highlight_bar.to_string(),
            SettingsField::Groove => self.core.song.groove_text(),
            SettingsField::Scale => match self.core.song.scale {
                Some(sc) => sc.display(),
                None => "off".to_string(),
            },
        }
    }

    fn settings_select_field(&mut self, field: SettingsField) {
        self.dialogs.settings_field = field;
        self.dialogs.settings_buf_untouched = true;
        self.dialogs.settings_edit_buf = self.settings_field_value(field);
    }

    pub(crate) fn settings_apply_field(&mut self) {
        // Tabbing past a field the user did not change must not push an undo
        // entry or mark the song dirty.
        if self.dialogs.settings_edit_buf == self.settings_field_value(self.dialogs.settings_field)
        {
            return;
        }
        match self.dialogs.settings_field {
            SettingsField::Title => {
                if !self.dialogs.settings_edit_buf.is_empty() {
                    self.core.begin_edit();
                    self.core.song.title = self.dialogs.settings_edit_buf.clone();
                }
            }
            SettingsField::Bpm => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u16>() {
                    let v = v.clamp(32, 300);
                    self.core.begin_edit();
                    self.core.song.bpm = v;
                    if self.core.link.is_enabled() {
                        self.core.link.set_tempo(v as f64);
                    }
                }
            }
            SettingsField::Speed => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u8>() {
                    let v = v.clamp(1, 31);
                    self.core.begin_edit();
                    self.core.song.speed = v;
                }
            }
            SettingsField::Channels => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, MAX_CHANNELS);
                    if v != self.core.song.channels {
                        self.core.set_channel_count(v);
                        self.clamp_cursor_to_song();
                    }
                }
            }
            SettingsField::Rows => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    if v != self.core.song.rows_per_pattern {
                        self.core.begin_edit();
                        self.core.song.rows_per_pattern = v;
                    }
                }
            }
            SettingsField::HighlightBeat => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 64);
                    self.core.begin_edit();
                    self.core.song.highlight_beat = v;
                }
            }
            SettingsField::HighlightBar => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    self.core.begin_edit();
                    self.core.song.highlight_bar = v;
                }
            }
            SettingsField::Groove => match Song::parse_groove(&self.dialogs.settings_edit_buf) {
                Ok(groove) if groove != self.core.song.groove => {
                    self.core.begin_edit();
                    self.core.song.groove = groove;
                }
                Ok(_) => {}
                Err(e) => self.status_message = Some(e),
            },
            SettingsField::Scale => match ScaleSetting::parse(&self.dialogs.settings_edit_buf) {
                Ok(parsed) if parsed != self.core.song.scale => {
                    self.core.begin_edit();
                    self.core.song.scale = parsed;
                }
                Ok(_) => {}
                Err(_) => {
                    self.status_message = Some(format!(
                        "Unknown scale: {} (try \"C minor\" or \"off\")",
                        self.dialogs.settings_edit_buf
                    ));
                }
            },
        }
    }

    fn handle_song_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(6) => {
                self.settings_apply_field();
                self.close_song_settings();
            }
            KeyCode::Tab | KeyCode::Down => {
                self.settings_apply_field();
                let next = self.dialogs.settings_field.next();
                self.settings_select_field(next);
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.settings_apply_field();
                let prev = self.dialogs.settings_field.prev();
                self.settings_select_field(prev);
            }
            KeyCode::Enter => {
                self.settings_apply_field();
                self.close_song_settings();
            }
            KeyCode::Char(c) => {
                // The buffer is seeded with the field's current value, so the
                // first character typed replaces it: entering "C minor" over a
                // seeded "off" should not produce "offC minor".
                if self.dialogs.settings_buf_untouched {
                    self.dialogs.settings_edit_buf.clear();
                }
                self.dialogs.settings_buf_untouched = false;
                self.dialogs.settings_edit_buf.push(c);
            }
            KeyCode::Backspace => {
                // Backspace edits the seeded value rather than clearing it.
                self.dialogs.settings_buf_untouched = false;
                self.dialogs.settings_edit_buf.pop();
            }
            _ => {}
        }
    }

    // -- Instrument list --

    pub(crate) fn open_instrument_list(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::InstrumentList;
    }

    fn close_instrument_list(&mut self) {
        self.mode = Mode::Normal;
    }

    // -- Tables --

    /// Open the table editor on table `number`, or on the table of the
    /// instrument under the cursor: the cell's, else the track's default.
    pub(crate) fn open_table_editor(&mut self, number: Option<u8>) {
        let pos = self.sheet();
        let number = number
            .or_else(|| {
                self.core
                    .resolve_edit_instrument(pos, self.cursor_row, self.cursor_channel)
            })
            .unwrap_or(0);
        self.dialogs.table_number = number;
        self.dialogs.table_row = 0;
        self.dialogs.table_field = super::TableField::Transpose;
        self.prev_mode = Mode::Normal;
        self.mode = Mode::TableEditor;
    }

    /// Rows in the open table; a table not created yet shows one.
    fn table_rows(&self) -> usize {
        self.core
            .song
            .tables
            .get(&self.dialogs.table_number)
            .map_or(1, |t| t.rows.len())
    }

    /// Edit the open table, creating it on first edit, as one undo step.
    fn edit_table(&mut self, f: impl FnOnce(&mut rtrack_core::tracker::Table, usize)) {
        self.core.begin_edit();
        let row = self.dialogs.table_row;
        let table = self.core.song.table_mut(self.dialogs.table_number);
        f(table, row);
        let rows = table.rows.len();
        self.dialogs.table_row = row.min(rows - 1);
    }

    fn handle_table_editor_key(&mut self, key: KeyEvent) {
        use super::TableField;
        use rtrack_core::tracker::{Cell, Table, MAX_TABLE_TICKS};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let field = self.dialogs.table_field;
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(':') => {
                self.command_buf.clear();
                self.prev_mode = Mode::TableEditor;
                self.mode = Mode::Command;
            }
            KeyCode::Left | KeyCode::Right if ctrl => {
                let delta: isize = if key.code == KeyCode::Left { -1 } else { 1 };
                let rows = (self.table_rows() as isize + delta).max(1) as usize;
                self.edit_table(|t, _| t.resize(rows));
            }
            KeyCode::Up => self.dialogs.table_row = self.dialogs.table_row.saturating_sub(1),
            KeyCode::Down if self.dialogs.table_row + 1 < self.table_rows() => {
                self.dialogs.table_row += 1;
            }
            KeyCode::Left => {
                self.dialogs.table_field = match field {
                    TableField::Transpose => TableField::Effect,
                    TableField::Volume => TableField::Transpose,
                    TableField::Effect => TableField::Volume,
                }
            }
            KeyCode::Right => {
                self.dialogs.table_field = match field {
                    TableField::Transpose => TableField::Volume,
                    TableField::Volume => TableField::Effect,
                    TableField::Effect => TableField::Transpose,
                }
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let up = key.code == KeyCode::Char(']');
                self.edit_table(|t, _| {
                    t.ticks_per_row = if up {
                        (t.ticks_per_row + 1).min(MAX_TABLE_TICKS)
                    } else {
                        t.ticks_per_row.saturating_sub(1).max(1)
                    };
                });
            }
            KeyCode::Char('l') => self.edit_table(|t, row| {
                t.loop_from = if t.loop_from == Some(row as u8) {
                    None
                } else {
                    Some(row as u8)
                };
            }),
            KeyCode::Char('+') | KeyCode::Char('-') => {
                let d: i16 = if key.code == KeyCode::Char('+') {
                    1
                } else {
                    -1
                };
                self.edit_table(|t, row| {
                    let r = &mut t.rows[row];
                    match field {
                        TableField::Transpose => {
                            let max = rtrack_core::tracker::MAX_LINK_TRANSPOSE as i16;
                            r.transpose = (r.transpose as i16 + d).clamp(-max, max) as i8;
                        }
                        TableField::Volume => {
                            let v = r
                                .volume
                                .map_or(0x40, |v| (v as i16 + d).clamp(0, 127) as u8);
                            r.volume = Some(v);
                        }
                        TableField::Effect => {
                            if let Some(fx) = r.fx.as_mut() {
                                fx.param = (fx.param as i16 + d).clamp(0, 255) as u8;
                            }
                        }
                    }
                });
            }
            KeyCode::Char('=') if field == TableField::Effect => {
                let text = self
                    .core
                    .song
                    .tables
                    .get(&self.dialogs.table_number)
                    .and_then(|t| t.rows.get(self.dialogs.table_row))
                    .and_then(|r| r.fx)
                    .map(|fx| fx.to_string())
                    .unwrap_or_default();
                self.command_buf = format!("tfx {text}");
                self.prev_mode = Mode::TableEditor;
                self.mode = Mode::Command;
            }
            KeyCode::Char(c) if c.is_ascii_hexdigit() && field != TableField::Transpose => {
                let digit = c.to_digit(16).unwrap() as u8;
                // Effect digits go through the pattern's entry rules, then
                // through the table's command check.
                let current = self
                    .core
                    .song
                    .tables
                    .get(&self.dialogs.table_number)
                    .and_then(|t| t.rows.get(self.dialogs.table_row).copied())
                    .unwrap_or_default();
                if field == TableField::Effect {
                    let mut cell = Cell::default();
                    cell.fx[0] = current.fx;
                    cell.enter_effect_digit(0, digit);
                    if let Some(fx) = cell.fx[0].filter(|fx| !Table::allows(fx.cmd)) {
                        self.status_message =
                            Some(format!("{} cannot play in a table", fx.cmd.name()));
                        return;
                    }
                    self.edit_table(|t, row| t.rows[row].fx = cell.fx[0]);
                } else {
                    let v = ((current.volume.unwrap_or(0) << 4) | digit) & 0x7F;
                    self.edit_table(|t, row| t.rows[row].volume = Some(v));
                }
            }
            KeyCode::Delete | KeyCode::Backspace => self.edit_table(|t, row| {
                let r = &mut t.rows[row];
                match field {
                    TableField::Transpose => r.transpose = 0,
                    TableField::Volume => r.volume = None,
                    TableField::Effect => r.fx = None,
                }
            }),
            _ => {}
        }
    }

    // -- Phrases --

    pub(crate) fn open_phrase_list(&mut self) {
        let count = self.core.song.phrases.len();
        if let Some(view) = self.phrase_view {
            self.dialogs.phrase_cursor = view.phrase;
        }
        self.dialogs.phrase_cursor = self.dialogs.phrase_cursor.min(count.saturating_sub(1));
        self.prev_mode = Mode::Normal;
        self.mode = Mode::PhraseList;
    }

    fn handle_phrase_list_key(&mut self, key: KeyEvent) {
        let count = self.core.song.phrases.len();
        let cursor = self.dialogs.phrase_cursor;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Left | KeyCode::Right if ctrl && cursor < count => {
                let len = self.core.song.phrases[cursor].rows.len();
                let len = if key.code == KeyCode::Left {
                    len.saturating_sub(1)
                } else {
                    len + 1
                };
                self.core.begin_edit();
                self.core.song.resize_phrase(cursor, len);
                self.status_message = Some(format!(
                    "{} rows",
                    self.core.song.phrases[cursor].rows.len()
                ));
            }
            KeyCode::Up => self.dialogs.phrase_cursor = cursor.saturating_sub(1),
            KeyCode::Down if cursor + 1 < count => self.dialogs.phrase_cursor = cursor + 1,
            // With the phrase view open there is no pattern column to link,
            // so Enter shows the phrase instead.
            KeyCode::Enter if cursor < count && self.phrase_view.is_some() => {
                self.mode = Mode::Normal;
                self.open_phrase_view(Some(cursor));
            }
            KeyCode::Enter if cursor < count => {
                self.mode = Mode::Normal;
                self.link_cursor_column(Some(cursor));
            }
            KeyCode::Char(c) if cursor < count && !ctrl => {
                self.core.begin_edit();
                self.core.song.phrases[cursor].name.push(c);
            }
            KeyCode::Backspace if cursor < count => {
                self.core.begin_edit();
                self.core.song.phrases[cursor].name.pop();
            }
            _ => {}
        }
    }

    /// Show a phrase in the grid in place of the pattern: `phrase`, or the
    /// one the cursor's track plays.
    pub(crate) fn open_phrase_view(&mut self, phrase: Option<usize>) {
        use rtrack_core::editor::PhraseView;
        let pattern_row = self.pattern_row();
        let opened = match phrase {
            Some(i) if i < self.core.song.phrases.len() => Some((
                PhraseView {
                    phrase: i,
                    pattern_row,
                },
                0,
            )),
            Some(_) => None,
            None => PhraseView::open(
                &self.core.song,
                self.current_order_position(),
                pattern_row,
                self.cursor_channel,
            ),
        };
        match opened {
            Some((view, row)) => {
                self.phrase_view = Some(view);
                self.cursor_row = row;
                self.block_anchor = None;
            }
            None if phrase.is_some() => self.status_message = Some("No such phrase".to_string()),
            None => {
                self.status_message =
                    Some("No phrases yet: :phrase links the track to a new one".to_string())
            }
        }
    }

    /// Return the grid to the pattern, at the row it showed before.
    pub(crate) fn close_phrase_view(&mut self) {
        if let Some(view) = self.phrase_view.take() {
            self.block_anchor = None;
            self.cursor_row = view.pattern_row.min(self.current_pattern_rows() - 1);
        }
    }

    fn toggle_phrase_view(&mut self) {
        if self.phrase_view.is_some() {
            self.close_phrase_view();
        } else {
            self.open_phrase_view(None);
        }
    }

    /// Show the next or previous phrase.
    fn step_phrase_view(&mut self, forward: bool) {
        let Some(mut view) = self.phrase_view else {
            return;
        };
        if view.step(&self.core.song, forward) {
            self.phrase_view = Some(view);
            self.block_anchor = None;
            self.cursor_row = self.cursor_row.min(self.current_pattern_rows() - 1);
        }
    }

    /// Add an empty phrase as long as the pattern, or a copy of the phrase
    /// on view, and show it.
    fn add_phrase_in_view(&mut self, duplicate: bool) {
        let Some(view) = self.phrase_view else {
            return;
        };
        self.core.begin_edit();
        let added = if duplicate {
            self.core.song.duplicate_phrase(view.phrase)
        } else {
            let rows = self.core.song.rows_at(self.current_order_position());
            Some(self.core.song.add_phrase(rows))
        };
        if let Some(i) = added {
            self.open_phrase_view(Some(i));
        }
    }

    /// Link the cursor's column to phrase `phrase`, or to a new phrase made
    /// from its cells. A column already linked keeps its timing.
    fn link_cursor_column(&mut self, phrase: Option<usize>) {
        use rtrack_core::tracker::PhraseTiming;
        let pos = self.current_order_position();
        let ch = self.cursor_channel;
        let timing = self
            .core
            .song
            .pattern_at(pos)
            .and_then(|p| p.link(ch))
            .map_or(PhraseTiming::Aligned, |l| l.timing);
        self.core.begin_edit();
        let linked = match phrase {
            Some(i) => self.core.song.link_column(pos, ch, i, timing).then_some(i),
            None => self.core.song.link_new_phrase(pos, ch, timing),
        };
        self.status_message = Some(match linked {
            Some(i) => format!("Track {} plays phrase {i:02X}", ch + 1),
            None => {
                self.core.cancel_edit();
                "No such phrase".to_string()
            }
        });
    }

    fn unlink_cursor_column(&mut self) {
        let pos = self.current_order_position();
        self.core.begin_edit();
        self.status_message = Some(if self.core.song.unlink_column(pos, self.cursor_channel) {
            "Unlinked; the phrase rows are now the track's own cells".to_string()
        } else {
            self.core.cancel_edit();
            "Track is not linked to a phrase".to_string()
        });
    }

    /// Set how far the cursor column's linked phrase is transposed at
    /// playback.
    fn set_cursor_link_transpose(&mut self, steps: i8) {
        let pos = self.current_order_position();
        let ch = self.cursor_channel;
        if self
            .core
            .song
            .pattern_at(pos)
            .and_then(|p| p.link(ch))
            .is_none()
        {
            self.status_message = Some("Track is not linked to a phrase".to_string());
            return;
        }
        self.core.begin_edit();
        self.core.song.set_link_transpose(pos, ch, steps);
        let set = self
            .core
            .song
            .pattern_at(pos)
            .and_then(|p| p.link(ch))
            .map_or(0, |l| l.transpose);
        let unit = if self.core.song.scale.is_some() {
            "scale degree(s)"
        } else {
            "semitone(s)"
        };
        self.status_message = Some(format!("Phrase plays transposed {set:+} {unit}"));
    }

    /// Switch the cursor column's link between aligned and running.
    fn toggle_cursor_link_timing(&mut self) {
        use rtrack_core::tracker::PhraseTiming;
        let pos = self.current_order_position();
        let ch = self.cursor_channel;
        let Some(link) = self.core.song.pattern_at(pos).and_then(|p| p.link(ch)) else {
            self.status_message = Some("Track is not linked to a phrase".to_string());
            return;
        };
        let timing = match link.timing {
            PhraseTiming::Aligned => PhraseTiming::Running,
            PhraseTiming::Running => PhraseTiming::Aligned,
        };
        self.core.begin_edit();
        self.core.song.set_link_timing(pos, ch, timing);
        self.status_message = Some(match timing {
            PhraseTiming::Aligned => "Phrase restarts with the pattern".to_string(),
            PhraseTiming::Running => "Phrase runs across patterns".to_string(),
        });
    }

    fn handle_instrument_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(7) => self.close_instrument_list(),
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_midi_instrument(MidiEdit::ToggleOutput);
            }
            KeyCode::Left | KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let step = if key.code == KeyCode::Left { -1 } else { 1 };
                self.edit_midi_instrument(MidiEdit::Program(step));
            }
            KeyCode::Up | KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let step = if key.code == KeyCode::Down { -1 } else { 1 };
                self.edit_midi_instrument(MidiEdit::Channel(step));
            }
            KeyCode::Up => {
                if self.dialogs.instrument_cursor > 0 {
                    self.dialogs.instrument_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.dialogs.instrument_cursor < MAX_INSTRUMENTS - 1 {
                    self.dialogs.instrument_cursor += 1;
                }
            }
            KeyCode::PageUp => {
                self.dialogs.instrument_cursor = self.dialogs.instrument_cursor.saturating_sub(16);
            }
            KeyCode::PageDown => {
                self.dialogs.instrument_cursor =
                    (self.dialogs.instrument_cursor + 16).min(MAX_INSTRUMENTS - 1);
            }
            KeyCode::Enter => {
                // Open sample editor for current instrument
                self.open_sample_editor();
            }
            KeyCode::Tab => {
                // Open synth editor for current instrument
                self.open_synth_editor();
            }
            KeyCode::Char(c) => {
                self.core.instruments[self.dialogs.instrument_cursor]
                    .name
                    .push(c);
                self.core.dirty = true;
            }
            KeyCode::Backspace => {
                self.core.instruments[self.dialogs.instrument_cursor]
                    .name
                    .pop();
                self.core.dirty = true;
            }
            _ => {}
        }
    }

    /// Change a MIDI setting of the instrument under the cursor.
    fn edit_midi_instrument(&mut self, edit: MidiEdit) {
        let slot = self.dialogs.instrument_cursor;
        if !matches!(
            self.core.instruments[slot].kind,
            InstrumentKind::Midi { .. }
        ) {
            self.status_message = Some(format!("Instrument {slot:02X} is not a MIDI instrument"));
            return;
        }
        self.core.begin_edit_with_samples();
        let InstrumentKind::Midi {
            program,
            channel,
            output,
        } = &mut self.core.instruments[slot].kind
        else {
            return;
        };
        self.status_message = Some(match edit {
            MidiEdit::ToggleOutput => {
                *output = if output.is_builtin() {
                    MidiOutput::External { port: None }
                } else {
                    MidiOutput::Builtin
                };
                if output.is_builtin() {
                    "Plays built in".to_string()
                } else {
                    "Plays on the selected MIDI port".to_string()
                }
            }
            MidiEdit::Program(step) => {
                *program = (*program as i32 + step).rem_euclid(128) as u8;
                format!("Program {program}")
            }
            // `None` (the track's channel), then channels 1-16, wrapping.
            MidiEdit::Channel(step) => {
                let next = (channel.map_or(0, |c| c as i32 + 1) + step).rem_euclid(17);
                *channel = (next > 0).then(|| (next - 1) as u8);
                match channel {
                    Some(c) => format!("MIDI channel {}", *c + 1),
                    None => "MIDI channel: the track's".to_string(),
                }
            }
        });
    }

    fn handle_sample_editor_key(&mut self, key: KeyEvent) {
        let slot = self.dialogs.sample_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.core.dirty = true;
            }
            KeyCode::Tab => {
                self.dialogs.sample_editor_field = self.dialogs.sample_editor_field.next();
                self.dialogs.sample_slice_overwrite_armed = false;
                self.break_edit_coalescing();
            }
            KeyCode::BackTab => {
                self.dialogs.sample_editor_field = self.dialogs.sample_editor_field.prev();
                self.dialogs.sample_slice_overwrite_armed = false;
                self.break_edit_coalescing();
            }
            KeyCode::Enter => {
                // Execute slice actions on Enter
                match self.dialogs.sample_editor_field {
                    SampleField::SliceEqual => {
                        let msg = match self.slice_sample(false) {
                            Ok(n) => format!("Sliced into {} equal segments", n),
                            Err(e) => self.describe_slice_error(&e),
                        };
                        self.status_message = Some(msg);
                    }
                    SampleField::SliceTransient => {
                        let msg = match self.slice_sample(true) {
                            Ok(n) => format!("Sliced at {} transients", n),
                            Err(e) => self.describe_slice_error(&e),
                        };
                        self.status_message = Some(msg);
                    }
                    _ => {}
                }
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Right | KeyCode::Left => {
                let delta = match key.code {
                    KeyCode::Up => 1,
                    KeyCode::Down => -1,
                    KeyCode::Right => 10,
                    _ => -10,
                };
                // Shift moves a trim edge on this slice only.
                let coupling = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    rtrack_core::sample::Coupling::Alone
                } else {
                    rtrack_core::sample::Coupling::Shared
                };
                self.adjust_sample_field(slot, delta, coupling);
            }
            KeyCode::Char('z') => self.snap_sample_loop(slot),
            _ => {}
        }
    }

    /// Change instrument `slot`'s drive or filter, as one undoable step per
    /// run of key presses. Up/Down step finely, Left/Right by 10; cutoff
    /// moves in semitones.
    fn adjust_sample_fx(&mut self, slot: usize, field: SampleField, delta: i64) {
        use rtrack_core::audio::synth::FilterType;
        use rtrack_core::{InstrumentKind, VoiceFilter};
        let before = self.core.snapshot_samples();
        let Some(InstrumentKind::Sample { fx, .. }) =
            self.core.instruments.get_mut(slot).map(|i| &mut i.kind)
        else {
            self.status_message = Some(format!("Instrument {slot:02X} does not play a sample"));
            return;
        };
        let before_fx = *fx;
        match (field, &mut fx.filter) {
            (SampleField::Drive, _) => {
                fx.drive =
                    (fx.drive + delta as f32 * 0.1).clamp(1.0, rtrack_core::SampleFx::MAX_DRIVE);
            }
            (SampleField::Filter, filter) => {
                let order = [
                    None,
                    Some(FilterType::LowPass),
                    Some(FilterType::HighPass),
                    Some(FilterType::BandPass),
                ];
                let at = order
                    .iter()
                    .position(|k| *k == filter.map(|f| f.kind))
                    .unwrap_or(0);
                let next =
                    order[(at as i64 + delta.signum()).rem_euclid(order.len() as i64) as usize];
                *filter = next.map(|kind| VoiceFilter {
                    kind,
                    ..filter.unwrap_or_default()
                });
            }
            (SampleField::Cutoff, Some(f)) => {
                f.cutoff_hz = (f.cutoff_hz * 2f32.powf(delta as f32 / 12.0)).clamp(20.0, 20000.0);
            }
            (SampleField::Resonance, Some(f)) => {
                f.resonance = (f.resonance + delta as f32 * 0.01).clamp(0.0, 0.95);
            }
            _ => {
                self.status_message = Some("Filter is off".to_string());
                return;
            }
        }
        if *fx != before_fx {
            self.record_sample_field_edit(
                rtrack_core::editor::EditSource::new(field.undo_control(), slot),
                before,
            );
        }
    }

    /// Move the slot's loop points to the nearest rising zero crossings, as
    /// one undoable step.
    fn snap_sample_loop(&mut self, slot: usize) {
        let before = self.core.snapshot_samples();
        let mut bank = (*self.core.sample_bank).clone();
        let Some(sample) = bank
            .samples
            .get_mut(slot)
            .and_then(|s| s.as_mut())
            .map(Arc::make_mut)
        else {
            self.status_message = Some("No sample loaded in this slot".to_string());
            return;
        };
        if !sample.snap_loop_to_zero_crossings() {
            self.status_message =
                Some("Loop unchanged: already on zero crossings, or none within 20 ms".to_string());
            return;
        }
        let (start, end) = (sample.loop_start, sample.loop_end);
        self.core.sample_bank = Arc::new(bank);
        if let Some(ref mut audio) = self.core.audio {
            audio.set_sample_bank(Arc::clone(&self.core.sample_bank));
        }
        self.record_sample_field_edit(
            rtrack_core::editor::EditSource::new("sample.loop_snap", slot),
            before,
        );
        self.status_message = Some(format!("Loop snapped to zero crossings: {start}..{end}"));
    }

    fn adjust_sample_field(
        &mut self,
        slot: usize,
        delta: i64,
        coupling: rtrack_core::sample::Coupling,
    ) {
        // Handle slice parameter fields (no sample mutation needed)
        match self.dialogs.sample_editor_field {
            field @ (SampleField::Drive
            | SampleField::Filter
            | SampleField::Cutoff
            | SampleField::Resonance) => {
                self.adjust_sample_fx(slot, field, delta);
                return;
            }
            SampleField::SliceCount => {
                self.dialogs.sample_slice_count =
                    (self.dialogs.sample_slice_count as i64 + delta).clamp(2, 64) as usize;
                return;
            }
            SampleField::SliceSensitivity => {
                let step = delta as f32 * 0.05;
                self.dialogs.sample_slice_sensitivity =
                    (self.dialogs.sample_slice_sensitivity + step).clamp(0.0, 1.0);
                return;
            }
            SampleField::SliceRange => {
                use rtrack_core::sample::SliceRange;
                self.dialogs.sample_slice_range = match self.dialogs.sample_slice_range {
                    SliceRange::Source => SliceRange::Span,
                    SliceRange::Span => SliceRange::Source,
                };
                return;
            }
            SampleField::SliceEqual => {
                let msg = match self.slice_sample(false) {
                    Ok(n) => format!("Sliced into {} equal segments", n),
                    Err(e) => self.describe_slice_error(&e),
                };
                self.status_message = Some(msg);
                return;
            }
            SampleField::SliceTransient => {
                let msg = match self.slice_sample(true) {
                    Ok(n) => format!("Sliced at {} transients", n),
                    Err(e) => self.describe_slice_error(&e),
                };
                self.status_message = Some(msg);
                return;
            }
            _ => {}
        }

        // Taken before the mutation so it is the "before" side of the step.
        // Cheap: the bank is `Arc<Sample>` per slot, so nothing is copied.
        let before = self.core.snapshot_samples();
        let field = self.dialogs.sample_editor_field;

        // Trim and loop edges go through the same rule as a GUI drag, so a
        // shared slice edge moves both slices and loop points stay in the span.
        use rtrack_core::sample::Boundary;
        let boundary = match field {
            SampleField::TrimStart => Some(Boundary::TrimStart),
            SampleField::TrimEnd => Some(Boundary::TrimEnd),
            SampleField::LoopStart => Some(Boundary::LoopStart),
            SampleField::LoopEnd => Some(Boundary::LoopEnd),
            _ => None,
        };
        if let Some(boundary) = boundary {
            let mut bank = (*self.core.sample_bank).clone();
            let Some(current) = bank.get(slot).map(|s| s.boundary(boundary)) else {
                self.status_message = Some("No sample loaded in this slot".to_string());
                return;
            };
            let target = (current as i64 + delta * 100).max(0) as usize;
            bank.move_boundary(slot, boundary, target, coupling);
            self.core.sample_bank = Arc::new(bank);
            if let Some(ref mut audio) = self.core.audio {
                audio.set_sample_bank(Arc::clone(&self.core.sample_bank));
            }
            self.record_sample_field_edit(
                rtrack_core::editor::EditSource::new(field.undo_control(), slot),
                before,
            );
            return;
        }
        let mut bank = (*self.core.sample_bank).clone();
        if let Some(sample) = bank
            .samples
            .get_mut(slot)
            .and_then(|s| s.as_mut())
            .map(std::sync::Arc::make_mut)
        {
            match self.dialogs.sample_editor_field {
                SampleField::BaseNote => {
                    sample.base_note =
                        (sample.base_note as i64 + delta).clamp(0, MIDI_MAX_NOTE as i64) as u8;
                }
                SampleField::LoopEnabled => {
                    sample.loop_enabled = !sample.loop_enabled;
                }
                SampleField::TrimStart
                | SampleField::TrimEnd
                | SampleField::LoopStart
                | SampleField::LoopEnd => unreachable!(),
                SampleField::SliceCount
                | SampleField::SliceSensitivity
                | SampleField::SliceRange
                | SampleField::SliceEqual
                | SampleField::SliceTransient
                | SampleField::Drive
                | SampleField::Filter
                | SampleField::Cutoff
                | SampleField::Resonance => unreachable!(),
            }
            self.core.sample_bank = Arc::new(bank);
            if let Some(ref mut audio) = self.core.audio {
                audio.set_sample_bank(Arc::clone(&self.core.sample_bank));
            }
            // Held arrow keys on one field are one edit; the source ties the
            // run together and the slot keeps two samples' fields apart.
            self.record_sample_field_edit(
                rtrack_core::editor::EditSource::new(field.undo_control(), slot),
                before,
            );
        } else {
            self.status_message = Some("No sample loaded in this slot".to_string());
        }
    }

    // -- Synth editor --

    fn handle_synth_editor_key(&mut self, key: KeyEvent) {
        let slot = self.dialogs.synth_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.core.dirty = true;
                self.status_message =
                    Some(format!("Synth params saved for instrument {:02X}", slot));
            }
            KeyCode::Tab => {
                self.dialogs.synth_editor_field = self.dialogs.synth_editor_field.next();
            }
            KeyCode::BackTab => {
                self.dialogs.synth_editor_field = self.dialogs.synth_editor_field.prev();
            }
            KeyCode::Up => {
                self.adjust_synth_field(slot, 1);
            }
            KeyCode::Down => {
                self.adjust_synth_field(slot, -1);
            }
            KeyCode::Right => {
                self.adjust_synth_field(slot, 10);
            }
            KeyCode::Left => {
                self.adjust_synth_field(slot, -10);
            }
            KeyCode::Delete => {
                // Clear custom synth params (revert to the slot's preset)
                self.core.instruments[slot].kind = rtrack_core::InstrumentKind::default_for(slot);
                self.core.dirty = true;
                self.status_message = Some("Synth params cleared (using preset)".to_string());
                self.mode = self.prev_mode;
            }
            _ => {}
        }
    }

    fn adjust_synth_field(&mut self, slot: usize, delta: i32) {
        use crate::app::SynthField;
        use rtrack_core::audio::synth::Patch;

        if let Some(params) = self.core.instruments[slot].synth_params_mut() {
            match self.dialogs.synth_editor_field {
                SynthField::Waveform => {
                    let max = Patch::count() as i32;
                    params.waveform = ((params.waveform as i32 + delta).rem_euclid(max)) as u8;
                }
                SynthField::Attack => {
                    params.attack = (params.attack + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::Decay => {
                    params.decay = (params.decay + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::Sustain => {
                    params.sustain = (params.sustain + delta as f32 * 0.01).clamp(0.0, 1.0);
                }
                SynthField::Release => {
                    params.release = (params.release + delta as f32 * 0.001).clamp(0.0, 5.0);
                }
                SynthField::Drive => {
                    params.drive = (params.drive + delta as f32 * 0.1)
                        .clamp(1.0, rtrack_core::SampleFx::MAX_DRIVE);
                }
                SynthField::FilterCutoff => {
                    params.filter_cutoff =
                        (params.filter_cutoff + delta as f32 * 0.1).clamp(0.1, 40.0);
                }
                SynthField::FilterResonance => {
                    params.filter_resonance =
                        (params.filter_resonance + delta as f32 * 0.01).clamp(0.0, 0.95);
                }
                SynthField::FilterEnv => {
                    params.filter_env = (params.filter_env + delta as f32 * 0.1).clamp(0.0, 8.0);
                }
                SynthField::Detune => {
                    params.detune = (params.detune + delta as f32 * 0.1).clamp(0.0, 50.0);
                }
                SynthField::FilterType => {
                    use rtrack_core::audio::synth::FilterType;
                    params.filter_type = match (params.filter_type, delta > 0) {
                        (FilterType::LowPass, true) => FilterType::HighPass,
                        (FilterType::HighPass, true) => FilterType::BandPass,
                        (FilterType::BandPass, true) => FilterType::LowPass,
                        (FilterType::LowPass, false) => FilterType::BandPass,
                        (FilterType::HighPass, false) => FilterType::LowPass,
                        (FilterType::BandPass, false) => FilterType::HighPass,
                    };
                }
                SynthField::SubOsc => {
                    params.sub_osc = (params.sub_osc + delta as f32 * 0.01).clamp(0.0, 1.0);
                }
                SynthField::FmRatio => {
                    params.fm_ratio = (params.fm_ratio + delta as f32 * 0.1).clamp(0.0, 16.0);
                }
                SynthField::FmIndex => {
                    params.fm_index = (params.fm_index + delta as f32 * 0.1).clamp(0.0, 10.0);
                }
                SynthField::PulseWidth => {
                    params.pulse_width =
                        (params.pulse_width + delta as f32 * 0.01).clamp(0.05, 0.95);
                }
            }
        }
    }

    /// Answer the unsaved-changes prompt, for quitting or for opening the
    /// song in `pending_open`.
    fn handle_quit_confirm_key(&mut self, key: KeyEvent) {
        let proceed = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => true,
            // A failed save leaves the prompt open; its status says why.
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if !self.save() {
                    return;
                }
                true
            }
            _ => false,
        };
        self.mode = self.prev_mode;
        let open = self.pending_open.take();
        match (proceed, open) {
            (true, Some(path)) => self.load_file_discarding(path),
            (true, None) => self.should_quit = true,
            (false, Some(_)) => self.status_message = Some("Open cancelled".to_string()),
            (false, None) => self.status_message = Some("Quit cancelled".to_string()),
        }
    }

    // -- Channel rename --

    fn open_track_config(&mut self) {
        let ch = self.cursor_channel;
        self.rename_buf = self
            .core
            .channels
            .get(ch)
            .map(|c| &c.name)
            .cloned()
            .unwrap_or_default();
        self.ch_fx_field = 0;
        self.prev_mode = self.mode;
        self.mode = Mode::TrackConfig;
    }

    // Track config fields:
    // Midi:   0=Name, 1=Type (2 fields)
    // Synth:  0=Name, 1=Type, 2=Instrument, 3..16=Effects (17 fields)
    // Sample: 0=Name, 1=Type, 2..15=Effects (16 fields)
    //
    // Effects (relative to fx_off):
    //  +0=Filter, +1=Cutoff, +2=Resonance,
    //  +3=Distortion, +4=Drive,
    //  +5=Chorus, +6=Rate, +7=Depth, +8=Mix,
    //  +9=Delay, +10=Time, +11=Feedback, +12=Mix,
    //  +13=Reverb, +14=Size, +15=Damp, +16=Mix  (but wait, that's not right)
    // 3 + 2 + 4 + 4 + 4 = 17 effect fields total
    const EFFECT_FIELDS: usize = 17;

    /// Fields: Name, Instrument, then the effects.
    const TRACK_CONFIG_FX_OFFSET: usize = 2;

    fn track_config_num_fields(&self) -> usize {
        Self::TRACK_CONFIG_FX_OFFSET + Self::EFFECT_FIELDS
    }

    fn track_config_fx_offset(&self) -> usize {
        Self::TRACK_CONFIG_FX_OFFSET
    }

    fn handle_track_config_key(&mut self, key: KeyEvent) {
        let ch = self.cursor_channel;
        // Ensure channel config exists
        while ch >= self.core.channels.len() {
            let idx = self.core.channels.len();
            self.core
                .channels
                .push(super::ChannelConfig::new(idx as u8));
        }
        let num_fields = self.track_config_num_fields();
        match key.code {
            KeyCode::Esc => {
                // Save name
                self.core.channels[ch].name = self.rename_buf.clone();
                // Send effects to audio engine
                if let Some(ref mut audio) = self.core.audio {
                    audio.set_channel_effects(ch as u8, &self.core.channels[ch].effects_params);
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Tab | KeyCode::Down => {
                self.ch_fx_field = (self.ch_fx_field + 1) % num_fields;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.ch_fx_field = (self.ch_fx_field + num_fields - 1) % num_fields;
            }
            KeyCode::Left => {
                self.adjust_track_config_field(ch, -1);
            }
            KeyCode::Right => {
                self.adjust_track_config_field(ch, 1);
            }
            KeyCode::Enter => {
                // On the Instrument field, load a sample into the track's slot
                if self.ch_fx_field == 1 {
                    self.open_file_browser(
                        super::FileBrowserAction::LoadSample(ch),
                        vec!["wav".to_string(), "aif".to_string(), "aiff".to_string()],
                    );
                    return;
                }
                // Save name
                self.core.channels[ch].name = self.rename_buf.clone();
                // Send effects to audio engine
                if let Some(ref mut audio) = self.core.audio {
                    audio.set_channel_effects(ch as u8, &self.core.channels[ch].effects_params);
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Char(c) => {
                // Only type into name field
                if self.ch_fx_field == 0 {
                    if self.rename_buf.len() < MAX_CHANNEL_NAME {
                        self.rename_buf.push(c);
                    }
                } else if c == 'l' || c == 'L' {
                    // MIDI learn: bind next CC to the current parameter
                    let fx_off = self.track_config_fx_offset();
                    if self.ch_fx_field >= fx_off {
                        let rel = self.ch_fx_field - fx_off;
                        if let Some(param) = super::LearnableParam::from_fx_field(rel) {
                            self.core.midi_learn_pending = Some((ch, param));
                            self.status_message = Some(format!(
                                "Move a CC to map -> {} (ch {})",
                                param.name(),
                                ch + 1
                            ));
                        }
                    }
                } else if c == 'u' || c == 'U' {
                    // Unmap: remove mapping for the current parameter
                    let fx_off = self.track_config_fx_offset();
                    if self.ch_fx_field >= fx_off {
                        let rel = self.ch_fx_field - fx_off;
                        if let Some(param) = super::LearnableParam::from_fx_field(rel) {
                            let before = self.core.midi_cc_mappings.len();
                            self.core
                                .midi_cc_mappings
                                .retain(|m| !(m.channel == ch && m.param == param));
                            if self.core.midi_cc_mappings.len() < before {
                                self.status_message =
                                    Some(format!("Unmapped {} (ch {})", param.name(), ch + 1));
                            }
                        }
                    }
                }
            }
            // Backspace only edits the name field; other fields are adjusted,
            // not typed into.
            KeyCode::Backspace if self.ch_fx_field == 0 => {
                self.rename_buf.pop();
            }
            _ => {}
        }
    }

    fn adjust_track_config_field(&mut self, ch: usize, dir: i32) {
        let fx_off = self.track_config_fx_offset();
        match self.ch_fx_field {
            0 => {} // Name: typed, not adjusted
            1 => {
                // Instrument: none, then 00-FF
                let current = self.core.channels[ch].default_instrument;
                let next = match (current, dir > 0) {
                    (None, true) => Some(0),
                    (None, false) | (Some(0), false) => None,
                    (Some(v), true) => Some(v.saturating_add(1)),
                    (Some(v), false) => Some(v - 1),
                };
                self.core.channels[ch].default_instrument = next;
                self.core.dirty = true;
                // Preview the selected instrument
                if let Some(inst) = next {
                    let note = self.current_octave * 12 + 12; // C at current octave
                    self.core
                        .preview_note_with_instrument(ch as u8, note, 100, Some(inst));
                }
            }
            f if f == fx_off => {
                self.core.channels[ch].effects_params.filter_enabled =
                    !self.core.channels[ch].effects_params.filter_enabled
            }
            f if f == fx_off + 1 => {
                let step = if dir > 0 { 100.0 } else { -100.0 };
                self.core.channels[ch].effects_params.filter_cutoff =
                    (self.core.channels[ch].effects_params.filter_cutoff + step)
                        .clamp(20.0, 20000.0);
            }
            f if f == fx_off + 2 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.filter_resonance =
                    (self.core.channels[ch].effects_params.filter_resonance + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 3 => {
                self.core.channels[ch].effects_params.distortion_enabled =
                    !self.core.channels[ch].effects_params.distortion_enabled
            }
            f if f == fx_off + 4 => {
                let step = if dir > 0 { 0.5 } else { -0.5 };
                self.core.channels[ch].effects_params.distortion_drive =
                    (self.core.channels[ch].effects_params.distortion_drive + step)
                        .clamp(1.0, 20.0);
            }
            f if f == fx_off + 5 => {
                self.core.channels[ch].effects_params.chorus_enabled =
                    !self.core.channels[ch].effects_params.chorus_enabled
            }
            f if f == fx_off + 6 => {
                let step = if dir > 0 { 0.1 } else { -0.1 };
                self.core.channels[ch].effects_params.chorus_rate =
                    (self.core.channels[ch].effects_params.chorus_rate + step).clamp(0.1, 10.0);
            }
            f if f == fx_off + 7 => {
                let step = if dir > 0 { 0.5 } else { -0.5 };
                self.core.channels[ch].effects_params.chorus_depth =
                    (self.core.channels[ch].effects_params.chorus_depth + step).clamp(0.5, 20.0);
            }
            f if f == fx_off + 8 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.chorus_mix =
                    (self.core.channels[ch].effects_params.chorus_mix + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 9 => {
                self.core.channels[ch].effects_params.delay_enabled =
                    !self.core.channels[ch].effects_params.delay_enabled
            }
            f if f == fx_off + 10 => {
                let step = if dir > 0 { 10.0 } else { -10.0 };
                self.core.channels[ch].effects_params.delay_time =
                    (self.core.channels[ch].effects_params.delay_time + step).clamp(1.0, 2000.0);
            }
            f if f == fx_off + 11 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.delay_feedback =
                    (self.core.channels[ch].effects_params.delay_feedback + step).clamp(0.0, 0.95);
            }
            f if f == fx_off + 12 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.delay_mix =
                    (self.core.channels[ch].effects_params.delay_mix + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 13 => {
                self.core.channels[ch].effects_params.reverb_enabled =
                    !self.core.channels[ch].effects_params.reverb_enabled
            }
            f if f == fx_off + 14 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.reverb_size =
                    (self.core.channels[ch].effects_params.reverb_size + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 15 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.reverb_damp =
                    (self.core.channels[ch].effects_params.reverb_damp + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 16 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.core.channels[ch].effects_params.reverb_mix =
                    (self.core.channels[ch].effects_params.reverb_mix + step).clamp(0.0, 1.0);
            }
            _ => {}
        }
        // Live update effects
        if self.ch_fx_field >= fx_off {
            if let Some(ref mut audio) = self.core.audio {
                audio.set_channel_effects(ch as u8, &self.core.channels[ch].effects_params);
            }
        }
    }

    // -- Pattern matrix --

    fn open_pattern_matrix(&mut self) {
        self.matrix_cursor = self.current_order_position();
        self.prev_mode = self.mode;
        self.mode = Mode::PatternMatrix;
    }

    fn handle_pattern_matrix_key(&mut self, key: KeyEvent) {
        let order_len = self.core.song.order.len();

        // Ctrl combos
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => {
                    // New empty pattern, insert after cursor
                    if self.refuse_at_pattern_limit() {
                        return;
                    }
                    self.core.begin_edit();
                    let idx = self.core.song.add_pattern();
                    self.core.song.order.insert(self.matrix_cursor + 1, idx);
                    self.core
                        .song
                        .order_repeats
                        .insert(self.matrix_cursor + 1, 1);
                    self.matrix_cursor += 1;
                    self.core.dirty = true;
                    self.status_message = Some(format!("New pattern {:02X}", idx));
                    return;
                }
                KeyCode::Char('d') => {
                    // Clone current pattern, insert after cursor
                    if self.refuse_at_pattern_limit() {
                        return;
                    }
                    self.core.begin_edit();
                    let src_idx = self.core.song.order[self.matrix_cursor];
                    let cloned = self.core.song.patterns[src_idx].clone();
                    let new_idx = self.core.song.patterns.len();
                    self.core.song.patterns.push(cloned);
                    self.core.song.order.insert(self.matrix_cursor + 1, new_idx);
                    self.core
                        .song
                        .order_repeats
                        .insert(self.matrix_cursor + 1, 1);
                    self.matrix_cursor += 1;
                    self.core.dirty = true;
                    self.status_message =
                        Some(format!("Cloned {:02X} -> {:02X}", src_idx, new_idx));
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.matrix_cursor > 0 {
                    self.matrix_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.matrix_cursor + 1 < order_len {
                    self.matrix_cursor += 1;
                }
            }
            KeyCode::Home => {
                self.matrix_cursor = 0;
            }
            KeyCode::End => {
                if order_len > 0 {
                    self.matrix_cursor = order_len - 1;
                }
            }
            KeyCode::PageUp => {
                self.matrix_cursor = self.matrix_cursor.saturating_sub(8);
            }
            KeyCode::PageDown => {
                self.matrix_cursor = (self.matrix_cursor + 8).min(order_len.saturating_sub(1));
            }
            KeyCode::Enter => {
                // Jump to the selected order position and close
                self.edit_order = self.matrix_cursor;
                if !self.core.playing {
                    self.core.engine.order = self.matrix_cursor;
                }
                self.cursor_row = 0;
                self.mode = Mode::Normal;
            }
            // Insert duplicate order entry after cursor
            KeyCode::Insert => {
                self.core.begin_edit();
                let pat = self.core.song.order[self.matrix_cursor];
                self.core.song.order.insert(self.matrix_cursor + 1, pat);
                self.core
                    .song
                    .order_repeats
                    .insert(self.matrix_cursor + 1, 1);
                self.matrix_cursor += 1;
                self.core.dirty = true;
            }
            // Delete order entry at cursor
            KeyCode::Delete | KeyCode::Backspace => {
                if self.core.song.order.len() > 1 {
                    self.core.begin_edit();
                    self.core.song.order.remove(self.matrix_cursor);
                    self.core.song.order_repeats.remove(self.matrix_cursor);
                    if self.matrix_cursor >= self.core.song.order.len() {
                        self.matrix_cursor = self.core.song.order.len() - 1;
                    }
                    self.core.dirty = true;
                    self.status_message = Some("Removed order entry".to_string());
                } else {
                    self.status_message = Some("Cannot remove last entry".to_string());
                }
            }
            // Increase repeat count
            KeyCode::Char(']') => {
                self.core.begin_edit();
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur < 99 {
                    self.core.song.order_repeats[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            // Decrease repeat count
            KeyCode::Char('[') => {
                self.core.begin_edit();
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur > 0 {
                    self.core.song.order_repeats[self.matrix_cursor] = cur - 1;
                    self.core.dirty = true;
                }
            }
            // Change which pattern this order entry points to
            KeyCode::Right | KeyCode::Char('+') => {
                let max_pat = self.core.song.patterns.len() - 1;
                let cur = self.core.song.order[self.matrix_cursor];
                if cur < max_pat {
                    self.core.begin_edit();
                    self.core.song.order[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            KeyCode::Left | KeyCode::Char('-') => {
                let cur = self.core.song.order[self.matrix_cursor];
                if cur > 0 {
                    self.core.begin_edit();
                    self.core.song.order[self.matrix_cursor] = cur - 1;
                    self.core.dirty = true;
                }
            }
            _ => {}
        }
    }

    // -- File browser --

    fn handle_file_browser_key(&mut self, key: KeyEvent) {
        let num_entries = self.dialogs.file_browser.entries.len();
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.dialogs.file_browser.cursor > 0 {
                    self.dialogs.file_browser.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.dialogs.file_browser.cursor + 1 < num_entries {
                    self.dialogs.file_browser.cursor += 1;
                }
            }
            KeyCode::PageUp => {
                self.dialogs.file_browser.cursor =
                    self.dialogs.file_browser.cursor.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.dialogs.file_browser.cursor =
                    (self.dialogs.file_browser.cursor + 10).min(num_entries.saturating_sub(1));
            }
            KeyCode::Home => {
                self.dialogs.file_browser.cursor = 0;
            }
            KeyCode::End => {
                if num_entries > 0 {
                    self.dialogs.file_browser.cursor = num_entries - 1;
                }
            }
            KeyCode::Backspace => {
                // Go up one directory
                if let Some(parent) = self.dialogs.file_browser.dir.parent() {
                    self.dialogs.file_browser.dir = parent.to_path_buf();
                    self.dialogs.file_browser.refresh();
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = self
                    .dialogs
                    .file_browser
                    .entries
                    .get(self.dialogs.file_browser.cursor)
                    .cloned()
                {
                    let path = self.dialogs.file_browser.dir.join(&entry.name);
                    if entry.is_dir {
                        self.dialogs.file_browser.dir = path;
                        self.dialogs.file_browser.refresh();
                    } else {
                        // File selected -- perform the action
                        self.mode = self.prev_mode;
                        self.on_file_selected(path);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_recent_files_key(&mut self, key: KeyEvent) {
        let count = self.recent_files.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.dialogs.recent_cursor > 0 {
                    self.dialogs.recent_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 && self.dialogs.recent_cursor + 1 < count {
                    self.dialogs.recent_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(path) = self.recent_files.get(self.dialogs.recent_cursor).cloned() {
                    self.mode = Mode::Normal;
                    self.load_file(path);
                }
            }
            _ => {}
        }
    }

    // -- Command mode --

    fn handle_command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.command_buf.clear();
            }
            KeyCode::Enter => {
                let cmd = self.command_buf.trim().to_string();
                self.command_buf.clear();
                self.execute_command(&cmd);
            }
            KeyCode::Backspace => {
                if self.command_buf.pop().is_none() {
                    // Empty buffer, cancel command mode
                    self.mode = self.prev_mode;
                }
            }
            KeyCode::Char(c) => {
                self.command_buf.push(c);
            }
            _ => {}
        }
    }

    fn execute_command(&mut self, cmd: &str) {
        // `:w <path>` saves as; `:w! <path>` may replace another file.
        if let Some((name, path)) = cmd.split_once(' ') {
            if matches!(name, "w" | "write" | "w!") && !path.trim().is_empty() {
                self.mode = Mode::Normal;
                self.save_as(PathBuf::from(path.trim()), name == "w!");
                return;
            }
        }
        match cmd {
            "p" | "pattern" => {
                self.open_pattern_matrix();
            }
            "q" | "quit" => {
                if self.core.dirty {
                    self.prev_mode = Mode::Normal;
                    self.mode = Mode::QuitConfirm;
                } else {
                    self.should_quit = true;
                }
            }
            "q!" => {
                self.should_quit = true;
            }
            "w" | "write" => {
                self.mode = Mode::Normal;
                self.save();
            }
            "wq" => {
                self.mode = Mode::Normal;
                self.should_quit = self.save();
            }
            "h" | "help" => {
                self.open_help();
            }
            "set" | "settings" => {
                self.open_song_settings();
            }
            "inst" | "instruments" => {
                self.open_instrument_list();
            }
            "midi" => {
                self.open_port_selector();
            }
            "link" => {
                self.mode = Mode::Normal;
                self.core.toggle_link();
            }
            "ew" | "wav" => {
                self.mode = Mode::Normal;
                self.export_wav_file();
            }
            "ef" | "flac" => {
                self.mode = Mode::Normal;
                self.export_flac_file();
            }
            "em" | "exportmidi" => {
                self.mode = Mode::Normal;
                self.export_midi();
            }
            "fx" | "effects" => {
                self.open_track_config();
            }
            "fx2" => {
                self.mode = self.prev_mode;
                self.show_second_fx = !self.show_second_fx;
                self.clamp_cursor_to_fx_columns();
                self.status_message = Some(if self.show_second_fx {
                    "Second effect column shown".to_string()
                } else {
                    "Second effect column shown only where used".to_string()
                });
            }
            "e" => {
                self.mode = self.prev_mode;
                self.set_effect_at_cursor("");
            }
            "phrases" => self.open_phrase_list(),
            "pedit" => {
                self.mode = Mode::Normal;
                self.open_phrase_view(None);
            }
            _ if cmd.starts_with("pedit ") => {
                self.mode = Mode::Normal;
                let arg = cmd["pedit ".len()..].trim();
                match usize::from_str_radix(arg, 16) {
                    Ok(i) => self.open_phrase_view(Some(i)),
                    Err(_) => self.status_message = Some(format!("Not a phrase number: {arg}")),
                }
            }
            "table" => self.open_table_editor(None),
            _ if cmd.starts_with("table ") => {
                let arg = cmd["table ".len()..].trim();
                match u8::from_str_radix(arg, 16) {
                    Ok(n) => self.open_table_editor(Some(n)),
                    Err(_) => {
                        self.mode = self.prev_mode;
                        self.status_message = Some(format!("Not a table number: {arg}"));
                    }
                }
            }
            "tdel" => {
                self.mode = Mode::Normal;
                let n = self.dialogs.table_number;
                self.core.begin_edit();
                self.status_message = Some(if self.core.song.tables.remove(&n).is_some() {
                    format!("Deleted table {n:02X}")
                } else {
                    self.core.cancel_edit();
                    format!("No table {n:02X}")
                });
            }
            _ if cmd.starts_with("tname ") => {
                self.mode = self.prev_mode;
                let name = cmd["tname ".len()..].trim().to_string();
                self.edit_table(|t, _| t.name = name);
            }
            "tfx" => {
                self.mode = self.prev_mode;
                self.edit_table(|t, row| t.rows[row].fx = None);
            }
            _ if cmd.starts_with("tfx ") => {
                self.mode = self.prev_mode;
                let text = cmd["tfx ".len()..].to_string();
                let current = self
                    .core
                    .song
                    .tables
                    .get(&self.dialogs.table_number)
                    .cloned()
                    .unwrap_or_else(|| rtrack_core::tracker::Table::new(1));
                let mut probe = current;
                match probe.set_row_effect(self.dialogs.table_row, &text) {
                    Ok(()) => {
                        let fx = probe.rows[self.dialogs.table_row].fx;
                        self.edit_table(|t, row| t.rows[row].fx = fx);
                    }
                    Err(e) => self.status_message = Some(e),
                }
            }
            "phrase" => {
                self.mode = self.prev_mode;
                self.link_cursor_column(None);
            }
            "unlink" => {
                self.mode = self.prev_mode;
                self.unlink_cursor_column();
            }
            "run" => {
                self.mode = self.prev_mode;
                self.toggle_cursor_link_timing();
            }
            _ if cmd.starts_with("tr ") => {
                self.mode = self.prev_mode;
                let arg = cmd["tr ".len()..].trim();
                match arg.trim_start_matches('+').parse::<i8>() {
                    Ok(steps) => self.set_cursor_link_transpose(steps),
                    Err(_) => self.status_message = Some(format!("Not a step count: {arg}")),
                }
            }
            _ if cmd.starts_with("phrase ") => {
                self.mode = self.prev_mode;
                let arg = cmd["phrase ".len()..].trim();
                match usize::from_str_radix(arg, 16) {
                    Ok(i) => self.link_cursor_column(Some(i)),
                    Err(_) => self.status_message = Some(format!("Not a phrase number: {arg}")),
                }
            }
            _ if cmd.starts_with("e ") => {
                self.mode = self.prev_mode;
                self.set_effect_at_cursor(&cmd[2..]);
            }
            "load" => {
                self.mode = Mode::Normal;
                self.open_sample_browser();
            }
            "open" => {
                self.mode = Mode::Normal;
                self.open_song_browser();
            }
            "recent" => {
                self.open_recent_files();
            }
            "audio" | "astat" => {
                self.mode = Mode::Normal;
                self.status_message = Some(match self.core.audio_stats() {
                    Some(stats) if stats.all_clear() => format!(
                        "Audio OK: nothing dropped or rescheduled ({} oversized callbacks)",
                        stats.oversized_callbacks
                    ),
                    Some(stats) => format!(
                        "Audio: {} commands dropped, {} applied early, {} oversized callbacks",
                        stats.commands_dropped,
                        stats.commands_applied_early,
                        stats.oversized_callbacks
                    ),
                    None => "No audio engine".to_string(),
                });
            }
            _ => {
                self.mode = self.prev_mode;
                self.status_message = Some(format!("Unknown command: {}", cmd));
            }
        }
    }

    // -- Note transpose --

    /// Transpose selected notes by the given number of semitones.
    /// If block selection is active, transpose the block; otherwise transpose at cursor.
    fn transpose_notes(&mut self, steps: i8) {
        // Copied out before the pattern is borrowed mutably.
        let scale = self.core.song.scale;
        self.core.begin_edit();
        let pos = self.sheet();

        if let Some((anchor_row, anchor_ch)) = self.block_anchor {
            let (r0, r1) = if anchor_row <= self.cursor_row {
                (anchor_row, self.cursor_row)
            } else {
                (self.cursor_row, anchor_row)
            };
            let (c0, c1) = if anchor_ch <= self.cursor_channel {
                (anchor_ch, self.cursor_channel)
            } else {
                (self.cursor_channel, anchor_ch)
            };
            for at in self.core.song.addresses(pos, r0..=r1, c0..=c1) {
                if let Some(cell) = self.core.song.cell_mut(at) {
                    cell.transpose_note(steps, scale);
                }
            }
            let unit = if scale.is_some() {
                "scale degree(s)"
            } else {
                "semitone(s)"
            };
            self.status_message = Some(format!("Transposed block by {} {}", steps, unit));
        } else {
            if let Some(cell) =
                self.core
                    .song
                    .cell_at_mut(pos, self.cursor_row, self.cursor_channel)
            {
                cell.transpose_note(steps, scale);
            }
        }
    }

    // -- Mouse handling --

    pub fn handle_mouse(&mut self, event: MouseEvent, pattern_area_y: u16, pattern_area_x: u16) {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_mouse_click(event.column, event.row, pattern_area_y, pattern_area_x);
            }
            MouseEventKind::ScrollUp => {
                self.move_cursor_up(3);
            }
            MouseEventKind::ScrollDown => {
                self.move_cursor_down(3);
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16, area_y: u16, area_x: u16) {
        // Check if click is in the pattern editor area
        if y < area_y {
            return;
        }
        let screen_y = (y - area_y) as usize;

        // Calculate which row was clicked
        let rows = self.current_pattern_rows();
        let visible_rows = 40usize; // approximate
        let center_offset = visible_rows / 2;
        let focus_row = self.cursor_row;
        let start_row = focus_row.saturating_sub(center_offset);
        let clicked_row = start_row + screen_y;
        if clicked_row < rows {
            self.cursor_row = clicked_row;
        }

        // Calculate which channel/sub-column was clicked, using the
        // renderer's layout so the two cannot disagree.
        use crate::tui::pattern_editor::{channel_width, ROW_NUM_WIDTH, SEPARATOR_WIDTH};
        let fx_columns = self.fx_columns();

        if x < area_x + ROW_NUM_WIDTH + SEPARATOR_WIDTH {
            return;
        }
        let col_x = x - area_x - ROW_NUM_WIDTH - SEPARATOR_WIDTH;

        // Each channel is followed by a separator
        let stride = channel_width(fx_columns) + SEPARATOR_WIDTH;
        let ch = (col_x / stride) as usize;
        let within = col_x % stride;

        // Map visible channel index to actual channel (offset by page)
        let visible = self.visible_channels();
        let actual_ch = visible.start + ch;
        if visible.contains(&actual_ch) {
            self.cursor_channel = actual_ch;
            // note=0..3, gap=3..5, inst=5..7, gap=7..9, vol=9..11, gap=11..13,
            // fx=13..18, gap=18..20, fx2=20..25
            if within < 5 {
                self.cursor_sub = SubColumn::Note;
            } else if within < 9 {
                self.cursor_sub = SubColumn::Instrument;
            } else if within < 13 {
                self.cursor_sub = SubColumn::Volume;
            } else if within < 20 || fx_columns < 2 {
                self.cursor_sub = SubColumn::Effect;
            } else {
                self.cursor_sub = SubColumn::Effect2;
            }
        }
    }

    // -- Input handling --

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status_message = None;
        self.dispatch_key(key);
        self.clamp_cursor_to_fx_columns();
        // One keypress is one undo step. Whatever the handlers snapshotted is
        // paired with the song as it now stands; a key that edited nothing
        // leaves nothing pending and this costs nothing.
        self.core.commit_edit();
    }

    fn dispatch_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            Mode::MidiPortSelect => self.handle_port_select_key(key),
            Mode::Help => self.handle_help_key(key),
            Mode::SongSettings => self.handle_song_settings_key(key),
            Mode::InstrumentList => self.handle_instrument_list_key(key),
            Mode::PhraseList => self.handle_phrase_list_key(key),
            Mode::TableEditor => self.handle_table_editor_key(key),
            Mode::SampleEditor => self.handle_sample_editor_key(key),
            Mode::SynthEditor => self.handle_synth_editor_key(key),
            Mode::QuitConfirm => self.handle_quit_confirm_key(key),
            Mode::TrackConfig => self.handle_track_config_key(key),
            Mode::PatternMatrix => self.handle_pattern_matrix_key(key),
            Mode::Command => self.handle_command_key(key),
            Mode::FileBrowser => self.handle_file_browser_key(key),
            Mode::RecentFiles => self.handle_recent_files_key(key),
        }
    }

    fn handle_common_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => {
                    self.save();
                    return true;
                }
                KeyCode::Char('o') => {
                    self.open_song_browser();
                    return true;
                }
                KeyCode::Char('z') => {
                    self.undo();
                    return true;
                }
                KeyCode::Char('y') => {
                    self.redo();
                    return true;
                }
                KeyCode::Char('b') => {
                    self.toggle_block_select();
                    return true;
                }
                KeyCode::Char('f') => {
                    self.toggle_follow();
                    return true;
                }
                KeyCode::Char('i') => {
                    self.interpolate_block();
                    return true;
                }
                KeyCode::Char('c') => {
                    if self.block_anchor.is_some() {
                        self.copy_block();
                    } else {
                        self.copy_row();
                    }
                    return true;
                }
                KeyCode::Char('v') => {
                    if self.clipboard.has_block() {
                        self.paste_block();
                    } else {
                        self.paste_row();
                    }
                    return true;
                }
                KeyCode::Char('x') => {
                    if self.block_anchor.is_some() {
                        self.cut_block();
                    } else {
                        self.cut_row();
                    }
                    return true;
                }
                KeyCode::Right if self.phrase_view.is_some() => {
                    self.step_phrase_view(true);
                    return true;
                }
                KeyCode::Left if self.phrase_view.is_some() => {
                    self.step_phrase_view(false);
                    return true;
                }
                KeyCode::Right => {
                    self.next_order_position();
                    return true;
                }
                KeyCode::Left => {
                    self.prev_order_position();
                    return true;
                }
                KeyCode::Char('e') => {
                    self.export_midi();
                    return true;
                }
                KeyCode::Char('w') => {
                    self.export_wav_file();
                    return true;
                }
                KeyCode::Char('l') => {
                    self.export_flac_file();
                    return true;
                }
                KeyCode::Char('m') => {
                    self.toggle_midi_clock();
                    return true;
                }
                KeyCode::Char('r') => {
                    self.toggle_recording();
                    return true;
                }
                // Ctrl+F9-F12: solo channels on current page
                KeyCode::F(9) => {
                    let ch = self.track_page * CHANNELS_PER_PAGE;
                    self.toggle_solo(ch);
                    return true;
                }
                KeyCode::F(10) => {
                    let ch = self.track_page * CHANNELS_PER_PAGE + 1;
                    self.toggle_solo(ch);
                    return true;
                }
                KeyCode::F(11) => {
                    let ch = self.track_page * CHANNELS_PER_PAGE + 2;
                    self.toggle_solo(ch);
                    return true;
                }
                KeyCode::F(12) => {
                    let ch = self.track_page * CHANNELS_PER_PAGE + 3;
                    self.toggle_solo(ch);
                    return true;
                }
                _ => {}
            }
        }
        // F9-F12: mute channels on current page
        match key.code {
            KeyCode::F(9) => {
                let ch = self.track_page * CHANNELS_PER_PAGE;
                self.toggle_channel_mute(ch);
                return true;
            }
            KeyCode::F(10) => {
                let ch = self.track_page * CHANNELS_PER_PAGE + 1;
                self.toggle_channel_mute(ch);
                return true;
            }
            KeyCode::F(11) => {
                let ch = self.track_page * CHANNELS_PER_PAGE + 2;
                self.toggle_channel_mute(ch);
                return true;
            }
            KeyCode::F(12) => {
                let ch = self.track_page * CHANNELS_PER_PAGE + 3;
                self.toggle_channel_mute(ch);
                return true;
            }
            _ => {}
        }
        false
    }

    /// Handle keys shared between Normal and Insert modes: navigation, playback,
    /// track paging, function keys, octave, BPM, edit step.
    fn handle_shared_key(&mut self, key: KeyEvent) -> bool {
        // Shift+Up/Down: note transpose
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => {
                    self.transpose_notes(1);
                    return true;
                }
                KeyCode::Down => {
                    self.transpose_notes(-1);
                    return true;
                }
                _ => {}
            }
        }

        // Ctrl+Space: play from start (order 0, row 0)
        if key.code == KeyCode::Char(' ') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.core.playing {
                self.core.stop();
            } else {
                self.play_from_start();
            }
            return true;
        }

        match key.code {
            // Playback
            KeyCode::Char(' ') => {
                self.toggle_playback();
                return true;
            }

            // Function keys shared by both modes
            KeyCode::F(1) => {
                self.open_help();
                return true;
            }
            KeyCode::F(2) => {
                self.open_port_selector();
                return true;
            }
            KeyCode::F(3) => {
                self.core.toggle_link();
                return true;
            }
            KeyCode::F(4) => {
                self.vis.panel = self.vis.panel.cycle();
                return true;
            }
            KeyCode::F(5) => {
                self.toggle_phrase_view();
                return true;
            }
            KeyCode::F(6) => {
                self.open_song_settings();
                return true;
            }
            KeyCode::F(7) => {
                self.open_instrument_list();
                return true;
            }
            KeyCode::F(8) => {
                self.cycle_theme();
                return true;
            }

            // Track cycling
            KeyCode::Tab => {
                self.cycle_track(1);
                return true;
            }
            KeyCode::BackTab => {
                self.cycle_track(-1);
                return true;
            }

            // Navigation
            KeyCode::Up => {
                self.move_cursor_up(1);
                return true;
            }
            KeyCode::Down => {
                self.move_cursor_down(1);
                return true;
            }
            KeyCode::Left => {
                self.move_cursor_left();
                return true;
            }
            KeyCode::Right => {
                self.move_cursor_right();
                return true;
            }
            KeyCode::PageUp => {
                self.move_cursor_up(16);
                return true;
            }
            KeyCode::PageDown => {
                self.move_cursor_down(16);
                return true;
            }
            KeyCode::Home => {
                self.cursor_row = 0;
                return true;
            }
            KeyCode::End => {
                self.cursor_row = self.current_pattern_rows() - 1;
                return true;
            }

            // Octave up/down
            KeyCode::Char('+') => {
                if self.current_octave < 9 {
                    self.current_octave += 1;
                }
                return true;
            }
            KeyCode::Char('-') => {
                if self.current_octave > 0 {
                    self.current_octave -= 1;
                }
                return true;
            }

            _ => {}
        }
        false
    }

    /// Cycle to next/previous track, wrapping around. Updates track page automatically.
    fn cycle_track(&mut self, dir: i32) {
        let n = self.core.song.channels;
        // The phrase view shows one track's column.
        if n == 0 || self.phrase_view.is_some() {
            return;
        }
        self.cursor_channel = if dir > 0 {
            (self.cursor_channel + 1) % n
        } else {
            (self.cursor_channel + n - 1) % n
        };
        self.cursor_sub = SubColumn::Note;
        self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) {
            return;
        }
        if self.handle_shared_key(key) {
            return;
        }

        // Ctrl combos specific to Normal mode
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') if self.phrase_view.is_some() => {
                    self.add_phrase_in_view(false);
                    return;
                }
                KeyCode::Char('d') if self.phrase_view.is_some() => {
                    self.add_phrase_in_view(true);
                    return;
                }
                KeyCode::Char('n') => {
                    self.add_new_pattern_to_order();
                    return;
                }
                KeyCode::Char('d') => {
                    self.clone_current_pattern();
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('q') => {
                if self.core.dirty {
                    self.prev_mode = self.mode;
                    self.mode = Mode::QuitConfirm;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Esc => self.mode = Mode::Insert,

            // BPM
            KeyCode::Char(']') => self.change_bpm(1),
            KeyCode::Char('[') => self.change_bpm(-1),

            // Edit step
            KeyCode::Char(')') => self.change_edit_step(1),
            KeyCode::Char('(') => self.change_edit_step(-1),

            // Command mode
            KeyCode::Char(':') => {
                self.command_buf.clear();
                self.prev_mode = self.mode;
                self.mode = Mode::Command;
            }

            // Track config (name, type, effects)
            KeyCode::Enter => self.open_track_config(),

            // Row insert/delete
            KeyCode::Insert => self.insert_row_at_cursor(),
            KeyCode::Backspace => self.delete_row_at_cursor(),

            _ => {}
        }
    }

    /// Keep the cursor off the second effect column while it is hidden.
    pub(crate) fn clamp_cursor_to_fx_columns(&mut self) {
        if self.cursor_sub == SubColumn::Effect2 && self.fx_columns() < 2 {
            self.cursor_sub = SubColumn::Effect;
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) {
            return;
        }
        if self.handle_shared_key(key) {
            return;
        }

        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,

            // Delete current cell content
            KeyCode::Delete | KeyCode::Backspace => {
                self.delete_at_cursor();
            }

            // Note off (= key in Insert mode on Note sub-column)
            KeyCode::Char('=') if self.cursor_sub == SubColumn::Note => {
                self.enter_note_off();
            }

            // Edit an effect by name
            KeyCode::Char('=') if self.cursor_sub.fx_column().is_some() => {
                self.open_effect_prompt();
            }

            // Piano keyboard note entry
            KeyCode::Char(c) => match self.cursor_sub {
                SubColumn::Note => self.try_enter_note(c),
                sub => self.try_enter_hex_digit(c, sub),
            },

            _ => {}
        }
    }

    fn open_help(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::Help;
        self.dialogs.help_scroll = 0;
    }

    fn close_help(&mut self) {
        self.mode = Mode::Normal;
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') => self.close_help(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.dialogs.help_scroll = self.dialogs.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.dialogs.help_scroll += 1;
            }
            KeyCode::PageUp => {
                self.dialogs.help_scroll = self.dialogs.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.dialogs.help_scroll += 10;
            }
            KeyCode::Home => {
                self.dialogs.help_scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_port_select_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(2) => self.close_port_selector(),
            KeyCode::Up => {
                if self.dialogs.midi_port_cursor > 0 {
                    self.dialogs.midi_port_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.dialogs.midi_port_cursor + 1 < self.dialogs.midi_port_list.len() {
                    self.dialogs.midi_port_cursor += 1;
                }
            }
            KeyCode::Enter => self.select_midi_port(),
            _ => {}
        }
    }

    fn try_enter_note(&mut self, c: char) {
        let Some((value, octave)) =
            rtrack_core::keymap::piano_key_at_octave(c, self.current_octave)
        else {
            return;
        };
        let note = self.core.song.snap_entry(Note::On { value, octave });

        self.core.begin_edit();

        let order = self.sheet();
        let ch = self.cursor_channel;
        // One resolution for both the preview and the written cell, so what
        // you hear while typing is what plays back.
        let instrument = self
            .core
            .resolve_edit_instrument(order, self.cursor_row, ch);

        if let Some(midi_note) = note.to_midi_note() {
            self.core.preview_note_for_cell(
                order,
                self.cursor_row,
                ch,
                midi_note,
                MIDI_DEFAULT_VELOCITY,
            );
        }

        let Some(cell) = self.core.song.cell_at_mut(order, self.cursor_row, ch) else {
            return;
        };
        cell.note = Some(note);
        if let Some(inst) = instrument {
            cell.instrument = Some(inst);
        }

        // Advance cursor
        self.move_cursor_down(self.edit_step);
    }

    fn try_enter_hex_digit(&mut self, c: char, sub: SubColumn) {
        let digit = match c {
            '0'..='9' => c as u8 - b'0',
            'a'..='f' => c as u8 - b'a' + 10,
            _ => return,
        };

        self.core.begin_edit();

        let Some(cell) = self.cursor_cell_mut() else {
            return;
        };

        match sub {
            SubColumn::Instrument => {
                let current = cell.instrument.unwrap_or(0);
                // Shift left and add new digit (2 hex digits max)
                cell.instrument = Some((current << 4) | digit);
            }
            SubColumn::Volume => {
                let current = cell.volume.unwrap_or(0);
                cell.volume = Some((current << 4) | digit);
            }
            SubColumn::Effect => cell.enter_effect_digit(0, digit),
            SubColumn::Effect2 => cell.enter_effect_digit(1, digit),
            SubColumn::Note => {} // handled separately
        }
    }

    fn enter_note_off(&mut self) {
        self.core.begin_edit();
        let Some(cell) = self.cursor_cell_mut() else {
            return;
        };
        cell.note = Some(Note::Off);
        self.move_cursor_down(self.edit_step);
    }

    fn delete_at_cursor(&mut self) {
        self.core.begin_edit();
        let sub = self.cursor_sub;
        let Some(cell) = self.cursor_cell_mut() else {
            return;
        };
        match sub {
            SubColumn::Note => cell.note = None,
            SubColumn::Instrument => cell.instrument = None,
            SubColumn::Volume => cell.volume = None,
            SubColumn::Effect => cell.fx[0] = None,
            SubColumn::Effect2 => cell.fx[1] = None,
        }
    }

    /// The cell under the cursor, wherever it is stored.
    fn cursor_cell_mut(&mut self) -> Option<&mut rtrack_core::tracker::Cell> {
        let pos = self.sheet();
        self.core
            .song
            .cell_at_mut(pos, self.cursor_row, self.cursor_channel)
    }

    /// Open the command line to edit the effect under the cursor by name,
    /// prefilled with its current text.
    fn open_effect_prompt(&mut self) {
        let Some(col) = self.cursor_sub.fx_column() else {
            return;
        };
        let cell = self
            .core
            .song
            .cell_at(self.sheet(), self.cursor_row, self.cursor_channel);
        let text = cell.fx[col].map(|fx| fx.to_string()).unwrap_or_default();
        self.command_buf = format!("e {text}");
        self.prev_mode = self.mode;
        self.mode = Mode::Command;
    }

    /// Set the effect under the cursor from its text form; `:e` with no
    /// text clears it.
    fn set_effect_at_cursor(&mut self, text: &str) {
        let Some(col) = self.cursor_sub.fx_column() else {
            self.status_message = Some("Move the cursor to an effect column first".to_string());
            return;
        };
        let pos = self.sheet();
        let mut cell = *self
            .core
            .song
            .cell_at(pos, self.cursor_row, self.cursor_channel);
        match cell.set_effect_text(col, text) {
            Ok(()) => {
                self.core.begin_edit();
                self.core
                    .song
                    .set_cell(pos, self.cursor_row, self.cursor_channel, cell);
            }
            Err(e) => self.status_message = Some(e),
        }
    }

    // -- Cursor movement --

    pub(crate) fn move_cursor_up(&mut self, amount: usize) {
        if self.cursor_row >= amount {
            self.cursor_row -= amount;
        } else {
            self.cursor_row = 0;
        }
    }

    pub(crate) fn move_cursor_down(&mut self, amount: usize) {
        let max = self.current_pattern_rows() - 1;
        self.cursor_row = (self.cursor_row + amount).min(max);
    }

    /// Rows on the sheet the grid shows: the current pattern or the phrase.
    pub(crate) fn current_pattern_rows(&self) -> usize {
        self.core.song.rows_at(self.sheet())
    }

    fn change_bpm(&mut self, delta: i16) {
        let new_bpm = (self.core.song.bpm as i16 + delta).clamp(32, 300) as u16;
        self.core.song.bpm = new_bpm;
        if self.core.link.is_enabled() {
            self.core.link.set_tempo(new_bpm as f64);
        }
    }

    fn change_edit_step(&mut self, delta: i16) {
        let new_step = (self.edit_step as i16 + delta).clamp(0, 16) as usize;
        self.edit_step = new_step;
        self.status_message = Some(format!("Edit step: {}", self.edit_step));
    }

    pub(crate) fn insert_row_at_cursor(&mut self) {
        self.core.begin_edit();
        if let Some(view) = self.phrase_view {
            self.core
                .song
                .insert_phrase_row(view.phrase, self.cursor_row);
            self.status_message = Some(format!("Inserted phrase row at {:02X}", self.cursor_row));
            return;
        }
        let pattern_idx = self.core.song.order[self.current_order_position()];
        self.core.song.patterns[pattern_idx].insert_row(self.cursor_row);
        self.status_message = Some(format!(
            "Inserted row at {:02X}{}",
            self.cursor_row,
            self.linked_row_op_note()
        ));
    }

    pub(crate) fn delete_row_at_cursor(&mut self) {
        self.core.begin_edit();
        if let Some(view) = self.phrase_view {
            self.core
                .song
                .delete_phrase_row(view.phrase, self.cursor_row);
            self.status_message = Some(format!("Deleted phrase row at {:02X}", self.cursor_row));
            return;
        }
        let pattern_idx = self.core.song.order[self.current_order_position()];
        self.core.song.patterns[pattern_idx].delete_row(self.cursor_row);
        self.status_message = Some(format!(
            "Deleted row at {:02X}{}",
            self.cursor_row,
            self.linked_row_op_note()
        ));
    }

    /// Row insert and delete act on the pattern's own cells; a linked
    /// track's phrase is unchanged, which the status line says.
    fn linked_row_op_note(&self) -> &'static str {
        let pos = self.current_order_position();
        let linked = self
            .core
            .song
            .pattern_at(pos)
            .is_some_and(|p| p.links.iter().any(Option::is_some));
        if linked {
            " (linked phrases unchanged)"
        } else {
            ""
        }
    }

    fn toggle_follow(&mut self) {
        self.follow_playback = !self.follow_playback;
        let state = if self.follow_playback { "on" } else { "off" };
        self.status_message = Some(format!("Follow mode {}", state));
    }

    pub(crate) fn move_cursor_left(&mut self) {
        if self.cursor_sub == SubColumn::Note {
            if self.cursor_channel > 0 && self.phrase_view.is_none() {
                self.cursor_channel -= 1;
                self.cursor_sub = SubColumn::Note.prev(self.fx_columns());
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.prev(self.fx_columns());
        }
    }

    pub(crate) fn move_cursor_right(&mut self) {
        if self.cursor_sub.next(self.fx_columns()) == SubColumn::Note {
            if self.cursor_channel + 1 < self.core.song.channels && self.phrase_view.is_none() {
                self.cursor_channel += 1;
                self.cursor_sub = SubColumn::Note;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.next(self.fx_columns());
        }
    }

    // -- Block selection --

    /// Toggle block selection anchor at the current cursor position
    pub fn toggle_block_select(&mut self) {
        if self.block_anchor.is_some() {
            self.block_anchor = None;
            self.status_message = Some("Block selection cleared".to_string());
        } else {
            self.block_anchor = Some((self.cursor_row, self.cursor_channel));
            self.status_message = Some("Block selection started".to_string());
        }
    }

    /// Get the block selection bounds: (row_start, row_end, ch_start, ch_end) inclusive
    pub fn block_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        self.block_anchor.map(|(anchor_row, anchor_ch)| {
            let (r0, r1) = if anchor_row <= self.cursor_row {
                (anchor_row, self.cursor_row)
            } else {
                (self.cursor_row, anchor_row)
            };
            let (c0, c1) = if anchor_ch <= self.cursor_channel {
                (anchor_ch, self.cursor_channel)
            } else {
                (self.cursor_channel, anchor_ch)
            };
            (r0, r1, c0, c1)
        })
    }

    /// Copy the block selection to clipboard (2D grid)
    fn copy_block(&mut self) {
        if let Some((r0, r1, c0, c1)) = self.block_bounds() {
            let pos = self.sheet();
            let song = &self.core.song;
            let block: Vec<Vec<_>> = (r0..=r1)
                .map(|r| (c0..=c1).map(|c| *song.cell_at(pos, r, c)).collect())
                .collect();
            self.clipboard.set_block(block);
            let rows = r1 - r0 + 1;
            let cols = c1 - c0 + 1;
            self.status_message = Some(format!("Copied block {}x{}", rows, cols));
        }
    }

    /// Cut block selection (copy + clear)
    fn cut_block(&mut self) {
        self.copy_block();
        if let Some((r0, r1, c0, c1)) = self.block_bounds() {
            self.core.begin_edit();
            let pos = self.sheet();
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.core
                        .song
                        .set_cell(pos, r, c, rtrack_core::tracker::Cell::default());
                }
            }
            self.block_anchor = None;
            self.status_message = Some("Cut block".to_string());
        }
    }

    /// Interpolate volume and effect values across a block selection.
    /// Uses the first and last row values as endpoints and fills intermediate rows linearly.
    fn interpolate_block(&mut self) {
        let bounds = match self.block_bounds() {
            Some(b) => b,
            None => {
                self.status_message = Some("No block selected (Ctrl+B first)".to_string());
                return;
            }
        };
        let (r0, r1, c0, c1) = bounds;
        if r1 <= r0 {
            self.status_message = Some("Need at least 2 rows to interpolate".to_string());
            return;
        }

        self.core.begin_edit();
        let pos = self.sheet();
        let song = &mut self.core.song;
        let steps = (r1 - r0) as f64;

        for c in c0..=c1 {
            let first = *song.cell_at(pos, r0, c);
            let last = *song.cell_at(pos, r1, c);

            // Interpolate volume
            if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                for r in r0..=r1 {
                    let t = (r - r0) as f64 / steps;
                    let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                    if let Some(cell) = song.cell_at_mut(pos, r, c) {
                        cell.volume = Some(v.round() as u8);
                    }
                }
            }

            // Interpolate the effect parameter when both endpoints have the same command
            if let (Some(f0), Some(f1)) = (first.fx[0], last.fx[0]) {
                if f0.cmd == f1.cmd {
                    for r in r0..=r1 {
                        let t = (r - r0) as f64 / steps;
                        let p = f0.param as f64 + (f1.param as f64 - f0.param as f64) * t;
                        if let Some(cell) = song.cell_at_mut(pos, r, c) {
                            cell.fx[0] = Some(Fx::new(f0.cmd, p.round() as u8));
                        }
                    }
                }
            }
        }

        self.status_message = Some("Interpolated block".to_string());
    }

    /// Paste block clipboard at cursor position
    fn paste_block(&mut self) {
        if let Some(block) = self.clipboard.block().cloned() {
            self.core.begin_edit();
            let pos = self.sheet();
            for (ri, row) in block.iter().enumerate() {
                for (ci, cell) in row.iter().enumerate() {
                    // Cells past the pattern's edge do not resolve and are skipped.
                    self.core.song.set_cell(
                        pos,
                        self.cursor_row + ri,
                        self.cursor_channel + ci,
                        *cell,
                    );
                }
            }
            self.status_message = Some("Pasted block".to_string());
        }
    }
}

/// A change [`App::edit_midi_instrument`] makes.
enum MidiEdit {
    ToggleOutput,
    Program(i32),
    Channel(i32),
}
