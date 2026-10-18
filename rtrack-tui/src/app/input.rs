use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{App, Mode, SampleField, SettingsField, SubColumn};
use rtrack_core::constants::*;
use rtrack_core::engine::ExportLength;
use rtrack_core::theory::ScaleSetting;
use rtrack_core::tracker::{Note, Song};

impl App {
    // -- Song settings dialog --

    pub(crate) fn open_song_settings(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::SongSettings;
        self.settings_select_field(SettingsField::Title);
    }

    fn close_song_settings(&mut self) {
        self.mode = Mode::Normal;
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
                    self.push_undo();
                    self.core.song.title = self.dialogs.settings_edit_buf.clone();
                }
            }
            SettingsField::Bpm => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u16>() {
                    let v = v.clamp(32, 300);
                    self.push_undo();
                    self.core.set_bpm(v);
                }
            }
            SettingsField::Speed => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u8>() {
                    let v = v.clamp(1, 31);
                    self.push_undo();
                    self.core.set_speed(v);
                }
            }
            SettingsField::Channels => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, MAX_CHANNELS);
                    if v != self.core.song.channels {
                        self.push_undo();
                        self.core.set_channel_count(v);
                        if self.cursor_channel >= v {
                            self.cursor_channel = v - 1;
                        }
                    }
                }
            }
            SettingsField::Rows => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    if v != self.core.song.rows_per_pattern {
                        self.push_undo();
                        self.core.song.rows_per_pattern = v;
                    }
                }
            }
            SettingsField::HighlightBeat => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 64);
                    self.push_undo();
                    self.core.song.highlight_beat = v;
                }
            }
            SettingsField::HighlightBar => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    self.push_undo();
                    self.core.song.highlight_bar = v;
                }
            }
            SettingsField::Groove => match Song::parse_groove(&self.dialogs.settings_edit_buf) {
                Ok(groove) => {
                    self.push_undo();
                    self.core.song.groove = groove;
                }
                Err(e) => self.status_message = Some(format!("Groove unchanged: {e}")),
            },
            SettingsField::Scale => match ScaleSetting::parse(&self.dialogs.settings_edit_buf) {
                Ok(parsed) if parsed != self.core.song.scale => {
                    self.push_undo();
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

    fn handle_instrument_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(7) => self.close_instrument_list(),
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
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_table_editor();
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.dialogs.mod_field = 0;
                self.prev_mode = self.mode;
                self.mode = Mode::Modulation;
            }
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.dialogs.inst_fx_field = 0;
                self.prev_mode = self.mode;
                self.mode = Mode::InstrumentFx;
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let inst = &mut self.core.instruments[self.dialogs.instrument_cursor];
                inst.fixed_pitch = !inst.fixed_pitch;
                self.core.dirty = true;
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let slot = self.dialogs.instrument_cursor;
                let inst = &mut self.core.instruments[slot];
                inst.kind = inst.kind.cycled(slot, true);
                self.core.dirty = true;
            }
            KeyCode::Left | KeyCode::Right => {
                // MIDI instruments: channel, or program with Ctrl.
                let slot = self.dialogs.instrument_cursor;
                let up = key.code == KeyCode::Right;
                if let super::InstrumentKind::Midi(ref mut out) = self.core.instruments[slot].kind {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        out.program = match (out.program, up) {
                            (None, true) => Some(0),
                            (None, false) => None,
                            (Some(0), false) => None,
                            (Some(p), true) => Some((p + 1).min(MIDI_MAX_VALUE)),
                            (Some(p), false) => Some(p - 1),
                        };
                    } else {
                        out.channel = if up {
                            (out.channel + 1).min(15)
                        } else {
                            out.channel.saturating_sub(1)
                        };
                    }
                    self.core.dirty = true;
                }
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
                | SampleField::SliceTransient => unreachable!(),
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
                // Back to the slot's preset patch.
                self.core.instruments[slot].kind = super::InstrumentKind::Preset;
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

        if let super::InstrumentKind::Synth { ref mut params } = self.core.instruments[slot].kind {
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

    fn handle_table_editor_key(&mut self, key: KeyEvent) {
        use rtrack_core::tracker::{FxKey, TABLE_ROWS};
        let t = self.dialogs.table_index;
        if t >= self.core.song.tables.len() {
            self.mode = self.prev_mode;
            return;
        }
        let row = self.dialogs.table_row;
        let field = self.dialogs.table_field;
        // Edit a copy, and record an undo step only if it changed.
        let mut table = self.core.song.tables[t].clone();
        let r = &mut table.rows[row];
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                return;
            }
            KeyCode::Up => self.dialogs.table_row = row.saturating_sub(1),
            KeyCode::Down => self.dialogs.table_row = (row + 1).min(TABLE_ROWS - 1),
            KeyCode::Left => self.dialogs.table_field = field.saturating_sub(1),
            KeyCode::Right | KeyCode::Tab => self.dialogs.table_field = (field + 1).min(2),
            KeyCode::Delete | KeyCode::Backspace => match field {
                0 => r.transpose = None,
                1 => r.volume = None,
                _ => r.fx = None,
            },
            KeyCode::Char('[') => table.length = table.length.saturating_sub(1).max(1),
            KeyCode::Char(']') => table.length = (table.length + 1).min(TABLE_ROWS as u8),
            KeyCode::Char('<') => table.tic = table.tic.saturating_sub(1).max(1),
            KeyCode::Char('>') => table.tic = table.tic.saturating_add(1),
            KeyCode::Char(c @ ('+' | '=' | '-')) if field == 0 => {
                let step = if c == '-' { -1 } else { 1 };
                r.transpose = Some(r.transpose.unwrap_or(0).saturating_add(step).clamp(-48, 48));
            }
            KeyCode::Char(c) if field == 1 => {
                if let Some(d) = c.to_digit(16) {
                    r.volume = Some((r.volume.unwrap_or(0) << 4) | d as u8);
                }
            }
            KeyCode::Char(c) if field == 2 => {
                let pos = (t, row, 0, 0);
                if self.dialogs.table_typing.at(pos).key(&mut r.fx, c) == FxKey::Rejected {
                    self.status_message = Some(format!("No effect command starts that way: {c}"));
                }
            }
            _ => {}
        }
        if self.dialogs.table_row != row || self.dialogs.table_field != field {
            self.dialogs.table_typing.reset();
        }
        if table != self.core.song.tables[t] {
            if let Some(fx) = table.rows[row].fx {
                if !rtrack_core::tracker::allowed_in_table(fx.cmd) {
                    self.status_message =
                        Some(format!("{} does nothing in a table", fx.cmd.name()));
                }
            }
            self.push_undo();
            self.core.song.tables[t] = table;
            self.core.dirty = true;
        }
    }

    /// The instrument's own filter and drive: 0 filter on, 1 cutoff,
    /// 2 resonance, 3 drive on, 4 drive amount.
    fn handle_instrument_fx_key(&mut self, key: KeyEvent) {
        let field = self.dialogs.inst_fx_field;
        let fx = &mut self.core.instruments[self.dialogs.instrument_cursor].fx;
        let dir = match key.code {
            KeyCode::Right => 1.0,
            KeyCode::Left => -1.0,
            _ => 0.0,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.mode = self.prev_mode,
            KeyCode::Up => self.dialogs.inst_fx_field = field.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.dialogs.inst_fx_field = (field + 1).min(4),
            KeyCode::Left | KeyCode::Right => {
                match field {
                    0 => fx.filter_enabled = !fx.filter_enabled,
                    // Cutoff moves by a semitone's ratio, so the range is even.
                    1 => {
                        fx.filter_cutoff =
                            (fx.filter_cutoff * 2f32.powf(dir / 12.0)).clamp(20.0, 20000.0)
                    }
                    2 => fx.filter_resonance = (fx.filter_resonance + 0.05 * dir).clamp(0.0, 1.0),
                    3 => fx.distortion_enabled = !fx.distortion_enabled,
                    _ => fx.distortion_drive = (fx.distortion_drive + 0.5 * dir).clamp(1.0, 20.0),
                }
                self.core.dirty = true;
            }
            _ => {}
        }
    }

    /// Modulation slots. Each slot has `MOD_FIELDS` fields: source, dest,
    /// amount, then the source's own: attack, hold, decay for an envelope;
    /// shape, period for an LFO.
    fn handle_modulation_key(&mut self, key: KeyEvent) {
        use rtrack_core::modulation::{LfoShape, ModDest, ModSlot, ModSource, MOD_SLOTS};
        let last = MOD_SLOTS * super::MOD_FIELDS - 1;
        let at = self.dialogs.mod_field;
        let step: i32 = match key.code {
            KeyCode::Right => 1,
            KeyCode::Left => -1,
            KeyCode::PageUp => 16,
            KeyCode::PageDown => -16,
            _ => 0,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.mode = self.prev_mode;
                return;
            }
            KeyCode::Up => self.dialogs.mod_field = at.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.dialogs.mod_field = (at + 1).min(last),
            _ => {}
        }
        if step == 0 {
            return;
        }
        let (slot, field) = (at / super::MOD_FIELDS, at % super::MOD_FIELDS);
        let mods = &mut self.core.instruments[self.dialogs.instrument_cursor].mods;
        let cycle = |i: usize, n: usize| (i as i32 + step.signum()).rem_euclid(n as i32) as usize;
        let nudge = |v: u8, lo: u8| (v as i32 + step).clamp(lo as i32, 255) as u8;
        if field == 0 {
            // Off, envelope, LFO
            let now = match mods[slot].map(|m| m.source) {
                None => 0,
                Some(ModSource::Envelope { .. }) => 1,
                Some(ModSource::Lfo { .. }) => 2,
            };
            mods[slot] = match cycle(now, 3) {
                0 => None,
                1 => Some(ModSlot {
                    source: ModSource::Envelope {
                        attack: 0,
                        hold: 6,
                        decay: 24,
                    },
                    ..mods[slot].unwrap_or_default()
                }),
                _ => Some(ModSlot {
                    source: ModSource::Lfo {
                        shape: LfoShape::Sine,
                        period: 24,
                    },
                    ..mods[slot].unwrap_or_default()
                }),
            };
        } else if let Some(m) = mods[slot].as_mut() {
            match (field, &mut m.source) {
                (1, _) => {
                    let i = ModDest::ALL.iter().position(|d| *d == m.dest).unwrap_or(0);
                    m.dest = ModDest::ALL[cycle(i, ModDest::ALL.len())];
                }
                (2, _) => m.amount = (m.amount as i32 + step).clamp(-128, 127) as i8,
                (3, ModSource::Envelope { attack, .. }) => *attack = nudge(*attack, 0),
                (4, ModSource::Envelope { hold, .. }) => *hold = nudge(*hold, 0),
                (5, ModSource::Envelope { decay, .. }) => *decay = nudge(*decay, 0),
                (3, ModSource::Lfo { shape, .. }) => {
                    let i = LfoShape::ALL.iter().position(|s| s == shape).unwrap_or(0);
                    *shape = LfoShape::ALL[cycle(i, LfoShape::ALL.len())];
                }
                (4, ModSource::Lfo { period, .. }) => *period = nudge(*period, 1),
                _ => {}
            }
        }
        self.core.dirty = true;
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

    // Track config fields: 0=Name, 1=Instrument (default), 2.. effects.
    //
    // Effects (relative to fx_off):
    //  +0=Filter, +1=Cutoff, +2=Resonance,
    //  +3=Distortion, +4=Drive,
    //  +5=Chorus, +6=Rate, +7=Depth, +8=Mix,
    //  +9=Delay, +10=Time, +11=Feedback, +12=Mix,
    //  +13=Reverb, +14=Size, +15=Damp, +16=Mix
    const EFFECT_FIELDS: usize = 17;

    /// Field index where the effects fields start.
    pub(crate) const TRACK_FX_OFFSET: usize = 2;

    fn track_config_num_fields(&self) -> usize {
        Self::TRACK_FX_OFFSET + Self::EFFECT_FIELDS
    }

    fn track_config_fx_offset(&self) -> usize {
        Self::TRACK_FX_OFFSET
    }

    fn handle_track_config_key(&mut self, key: KeyEvent) {
        let ch = self.cursor_channel;
        // Ensure channel config exists
        if ch >= self.core.channels.len() {
            self.core
                .channels
                .resize_with(ch + 1, super::ChannelConfig::default);
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
                // Default instrument: none, then 00..FF.
                let next = match (self.core.channels[ch].default_instrument, dir > 0) {
                    (None, true) => Some(0),
                    (None, false) => None,
                    (Some(0), false) => None,
                    (Some(v), true) => Some(v.saturating_add(1)),
                    (Some(v), false) => Some(v - 1),
                };
                self.core.channels[ch].default_instrument = next;
                if let Some(inst) = next {
                    let note = self.current_octave * 12 + 12; // C at current octave
                    self.core
                        .preview_note_with_instrument(ch, note, 100, Some(inst));
                }
                self.core.dirty = true;
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
                    self.push_undo();
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
                    self.push_undo();
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
                self.push_undo();
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
                    self.push_undo();
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
                self.push_undo();
                self.core.song.sync_order_repeats();
                let cur = self.core.song.order_repeats[self.matrix_cursor];
                if cur < 99 {
                    self.core.song.order_repeats[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            // Decrease repeat count
            KeyCode::Char('[') => {
                self.push_undo();
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
                    self.push_undo();
                    self.core.song.order[self.matrix_cursor] = cur + 1;
                    self.core.dirty = true;
                }
            }
            KeyCode::Left | KeyCode::Char('-') => {
                let cur = self.core.song.order[self.matrix_cursor];
                if cur > 0 {
                    self.push_undo();
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
        if let Some(args) = cmd.strip_prefix("alias ") {
            self.mode = Mode::Normal;
            self.alias_column(args);
            return;
        }
        // `:w <path>` saves as; `:w! <path>` may replace another file.
        if let Some((name, path)) = cmd.split_once(' ') {
            if matches!(name, "w" | "write" | "w!") && !path.trim().is_empty() {
                self.mode = Mode::Normal;
                self.save_as(PathBuf::from(path.trim()), name == "w!");
                return;
            }
        }
        // Exports take an optional length: `:wav 3` (loops) or `:wav 90s`.
        let (name, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
        if matches!(name, "ew" | "wav" | "ef" | "flac" | "em" | "exportmidi") {
            self.mode = Mode::Normal;
            let length = match arg.trim() {
                "" => ExportLength::default(),
                arg => match arg.parse() {
                    Ok(length) => length,
                    Err(e) => {
                        self.status_message = Some(e);
                        return;
                    }
                },
            };
            match name {
                "ew" | "wav" => self.export_wav_file(length),
                "ef" | "flac" => self.export_flac_file(length),
                _ => self.export_midi(length),
            }
            return;
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
            "fx" | "effects" => {
                self.open_track_config();
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
            "unalias" => {
                self.mode = Mode::Normal;
                let pattern = self.core.song.order[self.current_order_position()];
                if self.core.song.patterns[pattern]
                    .alias(self.cursor_channel)
                    .is_some()
                {
                    self.push_undo();
                    self.core.song.unalias(pattern, self.cursor_channel);
                    self.status_message = Some("Column now holds its own copy".to_string());
                } else {
                    self.status_message = Some("This column is not an alias".to_string());
                }
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
        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];

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
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.core
                        .song
                        .edit_cell(pattern_idx, r, c, |cell| cell.transpose_note(steps, scale));
                }
            }
            let unit = if scale.is_some() {
                "scale degree(s)"
            } else {
                "semitone(s)"
            };
            self.status_message = Some(format!("Transposed block by {} {}", steps, unit));
        } else {
            let (row, ch) = (self.cursor_row, self.cursor_channel);
            self.core.song.edit_cell(pattern_idx, row, ch, |cell| {
                cell.transpose_note(steps, scale)
            });
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
        let pattern_idx = self.core.song.order[self.current_order_position()];
        let pattern = &self.core.song.patterns[pattern_idx];
        let visible_rows = 40usize; // approximate
        let center_offset = visible_rows / 2;
        let focus_row = self.cursor_row;
        let start_row = focus_row.saturating_sub(center_offset);
        let clicked_row = start_row + screen_y;
        if clicked_row < pattern.rows {
            self.cursor_row = clicked_row;
        }

        // Calculate which channel/sub-column was clicked
        use crate::tui::pattern_editor::{
            sub_column_at, CHANNEL_WIDTH, ROW_NUM_WIDTH, SEPARATOR_WIDTH,
        };
        if x < area_x + ROW_NUM_WIDTH + SEPARATOR_WIDTH {
            return;
        }
        let col_x = x - area_x - ROW_NUM_WIDTH - SEPARATOR_WIDTH;

        // Each channel is followed by a separator
        let stride = CHANNEL_WIDTH + SEPARATOR_WIDTH;
        let ch = (col_x / stride) as usize;
        let within = col_x % stride;

        // Map visible channel index to actual channel (offset by page)
        let actual_ch = self.track_page * CHANNELS_PER_PAGE + ch;
        if actual_ch < pattern.channels && actual_ch < self.core.song.channels {
            self.cursor_channel = actual_ch;
            self.cursor_sub = sub_column_at(within.min(CHANNEL_WIDTH - 1));
        }
    }

    // -- Input handling --

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status_message = None;
        self.dispatch_key(key);
        // One keypress is one undo step. Whatever the handlers snapshotted is
        // paired with the song as it now stands; a key that edited nothing
        // leaves nothing pending and this costs nothing.
        self.commit_undo();
    }

    fn dispatch_key(&mut self, key: KeyEvent) {
        // Instruments are stored outside the song, so the song snapshot does
        // not cover them: compare the one an instrument editor works on.
        let edited = self.edited_instrument_slot();
        let before = edited.map(|slot| self.core.instruments[slot].clone());
        // Channel settings likewise, in the track config dialog.
        let channel = (self.mode == Mode::TrackConfig).then_some(self.cursor_channel);
        let channel_before = channel.and_then(|ch| self.core.channels.get(ch).cloned());
        self.dispatch_mode_key(key);
        if let (Some(slot), Some(before)) = (edited, before) {
            self.record_instrument_edit(slot, before);
        }
        if let (Some(ch), Some(before)) = (channel, channel_before) {
            self.record_channel_edit(ch, before);
        }
    }

    /// The instrument the current mode edits, if it is an instrument editor.
    fn edited_instrument_slot(&self) -> Option<usize> {
        match self.mode {
            Mode::InstrumentList | Mode::InstrumentFx | Mode::Modulation => {
                Some(self.dialogs.instrument_cursor)
            }
            Mode::SynthEditor => Some(self.dialogs.synth_editor_slot),
            _ => None,
        }
    }

    fn dispatch_mode_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            Mode::MidiPortSelect => self.handle_port_select_key(key),
            Mode::Help => self.handle_help_key(key),
            Mode::SongSettings => self.handle_song_settings_key(key),
            Mode::InstrumentList => self.handle_instrument_list_key(key),
            Mode::SampleEditor => self.handle_sample_editor_key(key),
            Mode::SynthEditor => self.handle_synth_editor_key(key),
            Mode::TableEditor => self.handle_table_editor_key(key),
            Mode::InstrumentFx => self.handle_instrument_fx_key(key),
            Mode::Modulation => self.handle_modulation_key(key),
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
                KeyCode::Right => {
                    self.next_order_position();
                    return true;
                }
                KeyCode::Left => {
                    self.prev_order_position();
                    return true;
                }
                KeyCode::Char('e') => {
                    self.export_midi(ExportLength::default());
                    return true;
                }
                KeyCode::Char('w') => {
                    self.export_wav_file(ExportLength::default());
                    return true;
                }
                KeyCode::Char('l') => {
                    self.export_flac_file(ExportLength::default());
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
        if n == 0 {
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

            // Undo, as in vi. Normal mode only: in Insert mode `u` is a note.
            KeyCode::Char('u') => self.undo(),

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

            // Piano keyboard note entry
            KeyCode::Char(c) => match self.cursor_sub {
                SubColumn::Note => self.try_enter_note(c),
                sub => self.type_into_field(c, sub),
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

        self.push_undo();

        let order = self.current_order_position();
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

        let Some(&pattern_idx) = self.core.song.order.get(order) else {
            return;
        };
        self.core
            .song
            .edit_cell(pattern_idx, self.cursor_row, ch, |cell| {
                cell.note = Some(note);
                if let Some(inst) = instrument {
                    cell.instrument = Some(inst);
                }
            });

        // Advance cursor
        self.move_cursor_down(self.edit_step);
    }

    /// `:alias P [T]`: play pattern P's copy of the cursor's channel here,
    /// transposed T semitones. P is hex, as the pattern matrix shows it.
    fn alias_column(&mut self, args: &str) {
        let mut parts = args.split_whitespace();
        let source = parts.next().and_then(|p| usize::from_str_radix(p, 16).ok());
        let transpose = match parts.next() {
            None => Some(0),
            Some(t) => t.trim_start_matches('+').parse::<i8>().ok(),
        };
        let (Some(source), Some(transpose)) = (source, transpose) else {
            self.status_message = Some("Usage: :alias <pattern hex> [transpose]".to_string());
            return;
        };
        let pattern = self.core.song.order[self.current_order_position()];
        let before = self.core.song.clone();
        let alias = rtrack_core::tracker::ColumnAlias { source, transpose };
        match self
            .core
            .song
            .set_alias(pattern, self.cursor_channel, alias)
        {
            Ok(()) => {
                // The snapshot was taken before the change, as push_undo would.
                self.pending_undo = Some(super::PendingUndo {
                    song: before,
                    samples: None,
                    channel: None,
                });
                self.history.clear_redo();
                self.status_message = Some(format!(
                    "Channel {} plays pattern {:02X}{}",
                    self.cursor_channel + 1,
                    source,
                    if transpose != 0 {
                        format!(" {transpose:+}")
                    } else {
                        String::new()
                    }
                ));
            }
            Err(e) => self.status_message = Some(format!("Alias refused: {e}")),
        }
    }

    /// Type into the instrument, volume or an effect slot at the cursor.
    fn type_into_field(&mut self, c: char, sub: SubColumn) {
        let order = self.current_order_position();
        let pos = (
            order,
            self.cursor_row,
            self.cursor_channel,
            sub.fx_slot().unwrap_or(usize::MAX),
        );
        let mut cell = self
            .core
            .song
            .cell_at(order, self.cursor_row, self.cursor_channel);
        let key = rtrack_core::editor::type_into(&mut cell, sub, c, self.fx_typing.at(pos));
        if key == rtrack_core::tracker::FxKey::Changed {
            self.push_undo();
            self.core
                .song
                .set_cell(order, self.cursor_row, self.cursor_channel, cell);
        }
    }

    fn enter_note_off(&mut self) {
        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];
        let (row, ch) = (self.cursor_row, self.cursor_channel);
        self.core
            .song
            .edit_cell(pattern_idx, row, ch, |cell| cell.note = Some(Note::Off));
        self.move_cursor_down(self.edit_step);
    }

    fn delete_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];
        let (row, ch, sub) = (self.cursor_row, self.cursor_channel, self.cursor_sub);
        self.core.song.edit_cell(pattern_idx, row, ch, |cell| {
            rtrack_core::editor::clear_field(cell, sub)
        });
        self.fx_typing.reset();
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

    /// Get the number of rows in the current pattern (per-pattern length)
    fn current_pattern_rows(&self) -> usize {
        let pattern_idx = self.core.song.order[self.current_order_position()];
        self.core.song.patterns[pattern_idx].rows
    }

    fn change_bpm(&mut self, delta: i16) {
        let new_bpm = (self.core.song.bpm as i16 + delta).clamp(32, 300) as u16;
        if new_bpm != self.core.song.bpm {
            self.push_undo();
            self.core.set_bpm(new_bpm);
        }
    }

    fn change_edit_step(&mut self, delta: i16) {
        let new_step = (self.edit_step as i16 + delta).clamp(0, 16) as usize;
        self.edit_step = new_step;
        self.status_message = Some(format!("Edit step: {}", self.edit_step));
    }

    pub(crate) fn insert_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];
        self.core.song.patterns[pattern_idx].insert_row(self.cursor_row);
        self.status_message = Some(format!("Inserted row at {:02X}", self.cursor_row));
    }

    pub(crate) fn delete_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];
        self.core.song.patterns[pattern_idx].delete_row(self.cursor_row);
        self.status_message = Some(format!("Deleted row at {:02X}", self.cursor_row));
    }

    fn toggle_follow(&mut self) {
        self.follow_playback = !self.follow_playback;
        let state = if self.follow_playback { "on" } else { "off" };
        self.status_message = Some(format!("Follow mode {}", state));
    }

    pub(crate) fn move_cursor_left(&mut self) {
        if self.cursor_sub == SubColumn::Note {
            if self.cursor_channel > 0 {
                self.cursor_channel -= 1;
                self.cursor_sub = SubColumn::LAST;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.prev();
        }
    }

    pub(crate) fn move_cursor_right(&mut self) {
        if self.cursor_sub == SubColumn::LAST {
            if self.cursor_channel < self.core.song.channels - 1 {
                self.cursor_channel += 1;
                self.cursor_sub = SubColumn::Note;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.next();
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
            let pattern_idx = self.core.song.order[self.current_order_position()];
            let song = &self.core.song;
            let block: Vec<Vec<rtrack_core::tracker::Cell>> = (r0..=r1)
                .map(|r| (c0..=c1).map(|c| song.cell(pattern_idx, r, c)).collect())
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
            self.push_undo();
            let pattern_idx = self.core.song.order[self.current_order_position()];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.core
                        .song
                        .edit_cell(pattern_idx, r, c, |cell| *cell = Default::default());
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

        self.push_undo();
        let pattern_idx = self.core.song.order[self.current_order_position()];
        let song = &mut self.core.song;
        let steps = (r1 - r0) as f64;

        for c in c0..=c1 {
            let first = song.cell(pattern_idx, r0, c);
            let last = song.cell(pattern_idx, r1, c);
            for r in r0..=r1 {
                let t = (r - r0) as f64 / steps;
                song.edit_cell(pattern_idx, r, c, |cell| {
                    // Volume, and each effect slot whose endpoints hold the
                    // same command
                    if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                        let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                        cell.volume = Some(v.round() as u8);
                    }
                    for slot in 0..rtrack_core::tracker::FX_COLUMNS {
                        if let Some(fx) =
                            rtrack_core::editor::interpolate_fx(&first, &last, slot, t)
                        {
                            cell.fx[slot] = Some(fx);
                        }
                    }
                });
            }
        }

        self.status_message = Some("Interpolated block".to_string());
    }

    /// Paste block clipboard at cursor position
    fn paste_block(&mut self) {
        if let Some(block) = self.clipboard.block().cloned() {
            self.push_undo();
            let pattern_idx = self.core.song.order[self.current_order_position()];
            for (ri, row) in block.iter().enumerate() {
                for (ci, cell) in row.iter().enumerate() {
                    let (r, c) = (self.cursor_row + ri, self.cursor_channel + ci);
                    self.core
                        .song
                        .edit_cell(pattern_idx, r, c, |dst| *dst = *cell);
                }
            }
            self.status_message = Some("Pasted block".to_string());
        }
    }
}
