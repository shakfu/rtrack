use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::constants::*;
use crate::tracker::Note;
use crate::tracker::NoteValue;
use crate::ui::pattern_editor::SubColumn;

use super::{App, ChannelType, Mode, SampleField, SettingsField};

impl App {
    // -- Song settings dialog --

    pub(crate) fn open_song_settings(&mut self) {
        self.prev_mode = self.mode;
        self.mode = Mode::SongSettings;
        self.dialogs.settings_field = SettingsField::Title;
        self.dialogs.settings_edit_buf = self.song.title.clone();
    }

    fn close_song_settings(&mut self) {
        self.mode = Mode::Normal;
    }

    fn settings_select_field(&mut self, field: SettingsField) {
        self.dialogs.settings_field = field;
        self.dialogs.settings_edit_buf = match field {
            SettingsField::Title => self.song.title.clone(),
            SettingsField::Bpm => self.song.bpm.to_string(),
            SettingsField::Speed => self.song.speed.to_string(),
            SettingsField::Channels => self.song.channels.to_string(),
            SettingsField::Rows => self.song.rows_per_pattern.to_string(),
            SettingsField::HighlightBeat => self.song.highlight_beat.to_string(),
            SettingsField::HighlightBar => self.song.highlight_bar.to_string(),
            SettingsField::Swing => self.song.swing.to_string(),
        };
    }

    pub(crate) fn settings_apply_field(&mut self) {
        match self.dialogs.settings_field {
            SettingsField::Title => {
                if !self.dialogs.settings_edit_buf.is_empty() {
                    self.push_undo();
                    self.song.title = self.dialogs.settings_edit_buf.clone();
                }
            }
            SettingsField::Bpm => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u16>() {
                    let v = v.clamp(32, 300);
                    self.push_undo();
                    self.song.bpm = v;
                    if self.link.is_enabled() {
                        self.link.set_tempo(v as f64);
                    }
                }
            }
            SettingsField::Speed => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u8>() {
                    let v = v.clamp(1, 31);
                    self.push_undo();
                    self.song.speed = v;
                }
            }
            SettingsField::Channels => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, MAX_CHANNELS);
                    if v != self.song.channels {
                        self.push_undo();
                        self.song.channels = v;
                        // Resize all patterns
                        for pat in &mut self.song.patterns {
                            for row in &mut pat.data {
                                row.resize(v, crate::tracker::Cell::default());
                            }
                            pat.channels = v;
                        }
                        // Grow or shrink channel configs to match new channel count
                        while self.channels.len() < v {
                            let idx = self.channels.len();
                            self.channels.push(super::ChannelConfig::new(idx as u8));
                        }
                        self.channels.truncate(v);
                        if self.cursor_channel >= v {
                            self.cursor_channel = v - 1;
                        }
                    }
                }
            }
            SettingsField::Rows => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    if v != self.song.rows_per_pattern {
                        self.push_undo();
                        self.song.rows_per_pattern = v;
                    }
                }
            }
            SettingsField::HighlightBeat => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 64);
                    self.push_undo();
                    self.song.highlight_beat = v;
                }
            }
            SettingsField::HighlightBar => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<usize>() {
                    let v = v.clamp(1, 256);
                    self.push_undo();
                    self.song.highlight_bar = v;
                }
            }
            SettingsField::Swing => {
                if let Ok(v) = self.dialogs.settings_edit_buf.parse::<u8>() {
                    let v = v.clamp(0, 100);
                    self.push_undo();
                    self.song.swing = v;
                }
            }
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
                self.dialogs.settings_edit_buf.push(c);
            }
            KeyCode::Backspace => {
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
                self.dialogs.instrument_cursor = (self.dialogs.instrument_cursor + 16).min(MAX_INSTRUMENTS - 1);
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
                self.instruments[self.dialogs.instrument_cursor].name.push(c);
                self.dirty = true;
            }
            KeyCode::Backspace => {
                self.instruments[self.dialogs.instrument_cursor].name.pop();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn handle_sample_editor_key(&mut self, key: KeyEvent) {
        let slot = self.dialogs.sample_editor_slot;
        match key.code {
            KeyCode::Esc => {
                self.mode = self.prev_mode;
                self.dirty = true;
            }
            KeyCode::Tab => {
                self.dialogs.sample_editor_field = self.dialogs.sample_editor_field.next();
            }
            KeyCode::BackTab => {
                self.dialogs.sample_editor_field = self.dialogs.sample_editor_field.prev();
            }
            KeyCode::Enter => {
                // Execute slice actions on Enter
                match self.dialogs.sample_editor_field {
                    SampleField::SliceEqual => {
                        match self.slice_sample(false) {
                            Ok(n) => self.status_message = Some(format!("Sliced into {} equal segments", n)),
                            Err(e) => self.status_message = Some(e),
                        }
                    }
                    SampleField::SliceTransient => {
                        match self.slice_sample(true) {
                            Ok(n) => self.status_message = Some(format!("Sliced at {} transients", n)),
                            Err(e) => self.status_message = Some(e),
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Up => {
                self.adjust_sample_field(slot, 1);
            }
            KeyCode::Down => {
                self.adjust_sample_field(slot, -1);
            }
            KeyCode::Right => {
                self.adjust_sample_field(slot, 10);
            }
            KeyCode::Left => {
                self.adjust_sample_field(slot, -10);
            }
            _ => {}
        }
    }

    fn adjust_sample_field(&mut self, slot: usize, delta: i64) {
        // Handle slice parameter fields (no sample mutation needed)
        match self.dialogs.sample_editor_field {
            SampleField::SliceCount => {
                self.dialogs.sample_slice_count = (self.dialogs.sample_slice_count as i64 + delta)
                    .clamp(2, 64) as usize;
                return;
            }
            SampleField::SliceSensitivity => {
                let step = delta as f32 * 0.05;
                self.dialogs.sample_slice_sensitivity = (self.dialogs.sample_slice_sensitivity + step)
                    .clamp(0.0, 1.0);
                return;
            }
            SampleField::SliceEqual => {
                match self.slice_sample(false) {
                    Ok(n) => self.status_message = Some(format!("Sliced into {} equal segments", n)),
                    Err(e) => self.status_message = Some(e),
                }
                return;
            }
            SampleField::SliceTransient => {
                match self.slice_sample(true) {
                    Ok(n) => self.status_message = Some(format!("Sliced at {} transients", n)),
                    Err(e) => self.status_message = Some(e),
                }
                return;
            }
            _ => {}
        }

        let mut bank = (*self.sample_bank).clone();
        if let Some(ref mut sample) = bank.samples.get_mut(slot).and_then(|s| s.as_mut()) {
            match self.dialogs.sample_editor_field {
                SampleField::BaseNote => {
                    sample.base_note = (sample.base_note as i64 + delta).clamp(0, MIDI_MAX_NOTE as i64) as u8;
                }
                SampleField::TrimStart => {
                    sample.trim_start = (sample.trim_start as i64 + delta * 100)
                        .clamp(0, sample.data.len() as i64 - 1) as usize;
                }
                SampleField::TrimEnd => {
                    let max = sample.data.len();
                    sample.trim_end = if sample.trim_end == 0 {
                        (max as i64 + delta * 100).clamp(0, max as i64) as usize
                    } else {
                        (sample.trim_end as i64 + delta * 100).clamp(0, max as i64) as usize
                    };
                }
                SampleField::LoopEnabled => {
                    sample.loop_enabled = !sample.loop_enabled;
                }
                SampleField::LoopStart => {
                    let max = sample.effective_loop_end();
                    sample.loop_start = (sample.loop_start as i64 + delta * 100)
                        .clamp(0, max as i64) as usize;
                }
                SampleField::LoopEnd => {
                    let max = sample.end();
                    sample.loop_end = if sample.loop_end == 0 {
                        (max as i64 + delta * 100).clamp(0, max as i64) as usize
                    } else {
                        (sample.loop_end as i64 + delta * 100).clamp(0, max as i64) as usize
                    };
                }
                SampleField::SliceCount | SampleField::SliceSensitivity
                | SampleField::SliceEqual | SampleField::SliceTransient => unreachable!(),
            }
            self.sample_bank = Arc::new(bank);
            if let Some(ref mut audio) = self.audio {
                audio.set_sample_bank(Arc::clone(&self.sample_bank));
            }
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
                self.dirty = true;
                self.status_message = Some(format!("Synth params saved for instrument {:02X}", slot));
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
                // Clear custom synth params (revert to channel default)
                self.instruments[slot].synth_params = None;
                self.dirty = true;
                self.status_message = Some("Synth params cleared (using channel default)".to_string());
                self.mode = self.prev_mode;
            }
            _ => {}
        }
    }

    fn adjust_synth_field(&mut self, slot: usize, delta: i32) {
        use crate::app::SynthField;
        use crate::audio::synth::Patch;

        if let Some(ref mut params) = self.instruments[slot].synth_params {
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
                    params.filter_cutoff = (params.filter_cutoff + delta as f32 * 0.1).clamp(0.1, 40.0);
                }
                SynthField::FilterResonance => {
                    params.filter_resonance = (params.filter_resonance + delta as f32 * 0.01).clamp(0.0, 0.95);
                }
                SynthField::FilterEnv => {
                    params.filter_env = (params.filter_env + delta as f32 * 0.1).clamp(0.0, 8.0);
                }
                SynthField::Detune => {
                    params.detune = (params.detune + delta as f32 * 0.1).clamp(0.0, 50.0);
                }
                SynthField::FilterType => {
                    use crate::audio::synth::FilterType;
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
                    params.pulse_width = (params.pulse_width + delta as f32 * 0.01).clamp(0.05, 0.95);
                }
            }
        }
    }

    fn handle_quit_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.should_quit = true;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save();
                self.should_quit = true;
            }
            _ => {
                // Any other key cancels
                self.mode = self.prev_mode;
                self.status_message = Some("Quit cancelled".to_string());
            }
        }
    }

    // -- Channel rename --

    fn open_track_config(&mut self) {
        let ch = self.cursor_channel;
        self.rename_buf = self.channels.get(ch).map(|c| &c.name).cloned().unwrap_or_default();
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

    fn track_config_num_fields(&self) -> usize {
        let ch = self.cursor_channel;
        let ch_type = self.channels.get(ch).map(|c| c.channel_type).unwrap_or(ChannelType::Midi);
        match ch_type {
            ChannelType::Midi => 2,
            ChannelType::Synth => 3 + Self::EFFECT_FIELDS, // Name, Type, Instrument + effects
            ChannelType::Sample => 3 + Self::EFFECT_FIELDS, // Name, Type, Load + effects
        }
    }

    /// Returns the field index offset where effects fields start (varies by track type).
    fn track_config_fx_offset(&self) -> usize {
        let ch = self.cursor_channel;
        let ch_type = self.channels.get(ch).map(|c| c.channel_type).unwrap_or(ChannelType::Midi);
        match ch_type {
            ChannelType::Synth | ChannelType::Sample => 3, // after Name, Type, Instrument/Load
            ChannelType::Midi => 2, // after Name, Type
        }
    }

    fn handle_track_config_key(&mut self, key: KeyEvent) {
        let ch = self.cursor_channel;
        // Ensure channel config exists
        while ch >= self.channels.len() {
            let idx = self.channels.len();
            self.channels.push(super::ChannelConfig::new(idx as u8));
        }
        let num_fields = self.track_config_num_fields();
        match key.code {
            KeyCode::Esc => {
                // Save name
                self.channels[ch].name = self.rename_buf.clone();
                // Send effects to audio engine
                if let Some(ref mut audio) = self.audio {
                    audio.set_channel_effects(ch as u8, &self.channels[ch].effects_params);
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
                let ch_type = self.channels.get(ch).map(|c| c.channel_type).unwrap_or(ChannelType::Midi);
                // On the Load field for Sample tracks, open file browser
                if self.ch_fx_field == 2 && ch_type == ChannelType::Sample {
                    self.open_file_browser(
                        super::FileBrowserAction::LoadSample(ch),
                        vec!["wav".to_string(), "aif".to_string(), "aiff".to_string()],
                    );
                    return;
                }
                // Save name
                self.channels[ch].name = self.rename_buf.clone();
                // Send effects to audio engine
                if let Some(ref mut audio) = self.audio {
                    audio.set_channel_effects(ch as u8, &self.channels[ch].effects_params);
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Char(c) => {
                // Only type into name field
                if self.ch_fx_field == 0 {
                    if self.rename_buf.len() < crate::ui::pattern_editor::MAX_CHANNEL_NAME {
                        self.rename_buf.push(c);
                    }
                }
            }
            KeyCode::Backspace => {
                if self.ch_fx_field == 0 {
                    self.rename_buf.pop();
                }
            }
            _ => {}
        }
    }

    fn adjust_track_config_field(&mut self, ch: usize, dir: i32) {
        let fx_off = self.track_config_fx_offset();
        match self.ch_fx_field {
            0 => {} // Name: typed, not adjusted
            1 => {
                // Type: cycle
                if ch < self.channels.len() {
                    self.channels[ch].channel_type = if dir > 0 {
                        self.channels[ch].channel_type.next()
                    } else {
                        self.channels[ch].channel_type.prev()
                    };
                    // Clamp field index if switching (field count changes)
                    let new_num = self.track_config_num_fields();
                    if self.ch_fx_field >= new_num {
                        self.ch_fx_field = new_num - 1;
                    }
                }
            }
            2 => {
                let ch_type = self.channels.get(ch).map(|c| c.channel_type).unwrap_or(ChannelType::Midi);
                match ch_type {
                    ChannelType::Synth => {
                        // Instrument field
                        match self.channels[ch].default_instrument {
                            None => self.channels[ch].default_instrument = Some(0),
                            Some(v) => {
                                let next = (v as i32 + dir).clamp(0, 255) as u8;
                                self.channels[ch].default_instrument = Some(next);
                            }
                        }
                    }
                    ChannelType::Sample => {
                        // Load sample -- open file browser
                        self.open_file_browser(
                            super::FileBrowserAction::LoadSample(ch),
                            vec!["wav".to_string(), "aif".to_string(), "aiff".to_string()],
                        );
                    }
                    _ => {}
                }
            }
            f if f == fx_off => self.channels[ch].effects_params.filter_enabled = !self.channels[ch].effects_params.filter_enabled,
            f if f == fx_off + 1 => {
                let step = if dir > 0 { 100.0 } else { -100.0 };
                self.channels[ch].effects_params.filter_cutoff = (self.channels[ch].effects_params.filter_cutoff + step).clamp(20.0, 20000.0);
            }
            f if f == fx_off + 2 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.filter_resonance = (self.channels[ch].effects_params.filter_resonance + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 3 => self.channels[ch].effects_params.distortion_enabled = !self.channels[ch].effects_params.distortion_enabled,
            f if f == fx_off + 4 => {
                let step = if dir > 0 { 0.5 } else { -0.5 };
                self.channels[ch].effects_params.distortion_drive = (self.channels[ch].effects_params.distortion_drive + step).clamp(1.0, 20.0);
            }
            f if f == fx_off + 5 => self.channels[ch].effects_params.chorus_enabled = !self.channels[ch].effects_params.chorus_enabled,
            f if f == fx_off + 6 => {
                let step = if dir > 0 { 0.1 } else { -0.1 };
                self.channels[ch].effects_params.chorus_rate = (self.channels[ch].effects_params.chorus_rate + step).clamp(0.1, 10.0);
            }
            f if f == fx_off + 7 => {
                let step = if dir > 0 { 0.5 } else { -0.5 };
                self.channels[ch].effects_params.chorus_depth = (self.channels[ch].effects_params.chorus_depth + step).clamp(0.5, 20.0);
            }
            f if f == fx_off + 8 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.chorus_mix = (self.channels[ch].effects_params.chorus_mix + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 9 => self.channels[ch].effects_params.delay_enabled = !self.channels[ch].effects_params.delay_enabled,
            f if f == fx_off + 10 => {
                let step = if dir > 0 { 10.0 } else { -10.0 };
                self.channels[ch].effects_params.delay_time = (self.channels[ch].effects_params.delay_time + step).clamp(1.0, 2000.0);
            }
            f if f == fx_off + 11 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.delay_feedback = (self.channels[ch].effects_params.delay_feedback + step).clamp(0.0, 0.95);
            }
            f if f == fx_off + 12 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.delay_mix = (self.channels[ch].effects_params.delay_mix + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 13 => self.channels[ch].effects_params.reverb_enabled = !self.channels[ch].effects_params.reverb_enabled,
            f if f == fx_off + 14 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.reverb_size = (self.channels[ch].effects_params.reverb_size + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 15 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.reverb_damp = (self.channels[ch].effects_params.reverb_damp + step).clamp(0.0, 1.0);
            }
            f if f == fx_off + 16 => {
                let step = if dir > 0 { 0.05 } else { -0.05 };
                self.channels[ch].effects_params.reverb_mix = (self.channels[ch].effects_params.reverb_mix + step).clamp(0.0, 1.0);
            }
            _ => {}
        }
        // Live update effects
        if self.ch_fx_field >= fx_off {
            if let Some(ref mut audio) = self.audio {
                audio.set_channel_effects(ch as u8, &self.channels[ch].effects_params);
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
        let order_len = self.song.order.len();

        // Ctrl combos
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => {
                    // New empty pattern, insert after cursor
                    self.push_undo();
                    let idx = self.song.add_pattern();
                    self.song.order.insert(self.matrix_cursor + 1, idx);
                    self.song.order_repeats.insert(self.matrix_cursor + 1, 1);
                    self.matrix_cursor += 1;
                    self.dirty = true;
                    self.status_message = Some(format!("New pattern {:02X}", idx));
                    return;
                }
                KeyCode::Char('d') => {
                    // Clone current pattern, insert after cursor
                    self.push_undo();
                    let src_idx = self.song.order[self.matrix_cursor];
                    let cloned = self.song.patterns[src_idx].clone();
                    let new_idx = self.song.patterns.len();
                    self.song.patterns.push(cloned);
                    self.song.order.insert(self.matrix_cursor + 1, new_idx);
                    self.song.order_repeats.insert(self.matrix_cursor + 1, 1);
                    self.matrix_cursor += 1;
                    self.dirty = true;
                    self.status_message = Some(format!("Cloned {:02X} -> {:02X}", src_idx, new_idx));
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
                if !self.playing {
                    self.engine.order = self.matrix_cursor;
                }
                self.cursor_row = 0;
                self.mode = Mode::Normal;
            }
            // Insert duplicate order entry after cursor
            KeyCode::Insert => {
                self.push_undo();
                let pat = self.song.order[self.matrix_cursor];
                self.song.order.insert(self.matrix_cursor + 1, pat);
                self.song.order_repeats.insert(self.matrix_cursor + 1, 1);
                self.matrix_cursor += 1;
                self.dirty = true;
            }
            // Delete order entry at cursor
            KeyCode::Delete | KeyCode::Backspace => {
                if self.song.order.len() > 1 {
                    self.push_undo();
                    self.song.order.remove(self.matrix_cursor);
                    self.song.order_repeats.remove(self.matrix_cursor);
                    if self.matrix_cursor >= self.song.order.len() {
                        self.matrix_cursor = self.song.order.len() - 1;
                    }
                    self.dirty = true;
                    self.status_message = Some("Removed order entry".to_string());
                } else {
                    self.status_message = Some("Cannot remove last entry".to_string());
                }
            }
            // Increase repeat count
            KeyCode::Char(']') => {
                self.push_undo();
                self.song.sync_order_repeats();
                let cur = self.song.order_repeats[self.matrix_cursor];
                if cur < 99 {
                    self.song.order_repeats[self.matrix_cursor] = cur + 1;
                    self.dirty = true;
                }
            }
            // Decrease repeat count
            KeyCode::Char('[') => {
                self.push_undo();
                self.song.sync_order_repeats();
                let cur = self.song.order_repeats[self.matrix_cursor];
                if cur > 0 {
                    self.song.order_repeats[self.matrix_cursor] = cur - 1;
                    self.dirty = true;
                }
            }
            // Change which pattern this order entry points to
            KeyCode::Right | KeyCode::Char('+') => {
                let max_pat = self.song.patterns.len() - 1;
                let cur = self.song.order[self.matrix_cursor];
                if cur < max_pat {
                    self.push_undo();
                    self.song.order[self.matrix_cursor] = cur + 1;
                    self.dirty = true;
                }
            }
            KeyCode::Left | KeyCode::Char('-') => {
                let cur = self.song.order[self.matrix_cursor];
                if cur > 0 {
                    self.push_undo();
                    self.song.order[self.matrix_cursor] = cur - 1;
                    self.dirty = true;
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
                self.dialogs.file_browser.cursor = self.dialogs.file_browser.cursor.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.dialogs.file_browser.cursor = (self.dialogs.file_browser.cursor + 10).min(num_entries.saturating_sub(1));
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
                if let Some(entry) = self.dialogs.file_browser.entries.get(self.dialogs.file_browser.cursor).cloned() {
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
        match cmd {
            "p" | "pattern" => {
                self.open_pattern_matrix();
            }
            "q" | "quit" => {
                if self.dirty {
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
                self.save();
                self.should_quit = true;
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
                self.toggle_link();
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
            "load" => {
                self.mode = Mode::Normal;
                self.open_file_browser(
                    super::FileBrowserAction::LoadSample(self.cursor_channel),
                    vec!["wav".to_string(), "aif".to_string(), "aiff".to_string()],
                );
            }
            "open" => {
                self.mode = Mode::Normal;
                self.open_file_browser(
                    super::FileBrowserAction::OpenSong,
                    vec!["rtrk".to_string()],
                );
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
    fn transpose_notes(&mut self, semitones: i8) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];

        if let Some((anchor_row, anchor_ch)) = self.history.block_anchor {
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
            let pattern = &mut self.song.patterns[pattern_idx];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    transpose_cell_note(pattern.get_mut(r, c), semitones);
                }
            }
            self.status_message = Some(format!("Transposed block by {} semitone(s)", semitones));
        } else {
            let pattern = &mut self.song.patterns[pattern_idx];
            transpose_cell_note(pattern.get_mut(self.cursor_row, self.cursor_channel), semitones);
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
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &self.song.patterns[pattern_idx];
        let visible_rows = 40usize; // approximate
        let center_offset = visible_rows / 2;
        let focus_row = self.cursor_row;
        let start_row = if focus_row >= center_offset {
            focus_row - center_offset
        } else {
            0
        };
        let clicked_row = start_row + screen_y;
        if clicked_row < pattern.rows {
            self.cursor_row = clicked_row;
        }

        // Calculate which channel/sub-column was clicked
        let row_num_width: u16 = 3;
        let sep_width: u16 = 3;
        let channel_width: u16 = 14;

        if x < area_x + row_num_width + sep_width {
            return;
        }
        let col_x = x - area_x - row_num_width - sep_width;

        // Each channel is channel_width + separator_width (except first)
        let stride = channel_width + sep_width;
        let ch = (col_x / stride) as usize;
        let within = col_x % stride;

        // Map visible channel index to actual channel (offset by page)
        let actual_ch = self.track_page * CHANNELS_PER_PAGE + ch as usize;
        if actual_ch < pattern.channels && actual_ch < self.song.channels {
            self.cursor_channel = actual_ch;
            // note=0..3, gap=3, inst=4..6, gap=6, vol=7..9, gap=9, fx=10..13
            if within < 3 {
                self.cursor_sub = SubColumn::Note;
            } else if within < 6 {
                self.cursor_sub = SubColumn::Instrument;
            } else if within < 9 {
                self.cursor_sub = SubColumn::Volume;
            } else if within < 14 {
                self.cursor_sub = SubColumn::Effect;
            }
        }
    }

    // -- Input handling --

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status_message = None;
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            Mode::MidiPortSelect => self.handle_port_select_key(key),
            Mode::Help => self.handle_help_key(key),
            Mode::SongSettings => self.handle_song_settings_key(key),
            Mode::InstrumentList => self.handle_instrument_list_key(key),
            Mode::SampleEditor => self.handle_sample_editor_key(key),
            Mode::SynthEditor => self.handle_synth_editor_key(key),
            Mode::QuitConfirm => self.handle_quit_confirm_key(key),
            Mode::TrackConfig => self.handle_track_config_key(key),
            Mode::PatternMatrix => self.handle_pattern_matrix_key(key),
            Mode::Command => self.handle_command_key(key),
            Mode::FileBrowser => self.handle_file_browser_key(key),
        }
    }

    fn handle_common_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => { self.save(); return true; }
                KeyCode::Char('z') => { self.undo(); return true; }
                KeyCode::Char('y') => { self.redo(); return true; }
                KeyCode::Char('b') => { self.toggle_block_select(); return true; }
                KeyCode::Char('f') => { self.toggle_follow(); return true; }
                KeyCode::Char('i') => { self.interpolate_block(); return true; }
                KeyCode::Char('c') => {
                    if self.history.block_anchor.is_some() {
                        self.copy_block();
                    } else {
                        self.copy_row();
                    }
                    return true;
                }
                KeyCode::Char('v') => {
                    if self.history.block_clipboard.is_some() {
                        self.paste_block();
                    } else {
                        self.paste_row();
                    }
                    return true;
                }
                KeyCode::Char('x') => {
                    if self.history.block_anchor.is_some() {
                        self.cut_block();
                    } else {
                        self.cut_row();
                    }
                    return true;
                }
                KeyCode::Right => { self.next_order_position(); return true; }
                KeyCode::Left => { self.prev_order_position(); return true; }
                KeyCode::Char('e') => { self.export_midi(); return true; }
                KeyCode::Char('w') => { self.export_wav_file(); return true; }
                KeyCode::Char('l') => { self.export_flac_file(); return true; }
                KeyCode::Char('m') => { self.toggle_midi_clock(); return true; }
                // Ctrl+F9-F12: solo channels on current page
                KeyCode::F(9) => { let ch = self.track_page * CHANNELS_PER_PAGE; self.toggle_solo(ch); return true; }
                KeyCode::F(10) => { let ch = self.track_page * CHANNELS_PER_PAGE + 1; self.toggle_solo(ch); return true; }
                KeyCode::F(11) => { let ch = self.track_page * CHANNELS_PER_PAGE + 2; self.toggle_solo(ch); return true; }
                KeyCode::F(12) => { let ch = self.track_page * CHANNELS_PER_PAGE + 3; self.toggle_solo(ch); return true; }
                _ => {}
            }
        }
        // F9-F12: mute channels on current page
        match key.code {
            KeyCode::F(9) => { let ch = self.track_page * CHANNELS_PER_PAGE; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(10) => { let ch = self.track_page * CHANNELS_PER_PAGE + 1; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(11) => { let ch = self.track_page * CHANNELS_PER_PAGE + 2; self.toggle_channel_mute(ch); return true; }
            KeyCode::F(12) => { let ch = self.track_page * CHANNELS_PER_PAGE + 3; self.toggle_channel_mute(ch); return true; }
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
                KeyCode::Up => { self.transpose_notes(1); return true; }
                KeyCode::Down => { self.transpose_notes(-1); return true; }
                _ => {}
            }
        }

        // Ctrl+Space: play from start (order 0, row 0)
        if key.code == KeyCode::Char(' ') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.playing { self.stop(); } else { self.play_from_start(); }
            return true;
        }

        match key.code {
            // Playback
            KeyCode::Char(' ') => { self.toggle_playback(); return true; }

            // Function keys shared by both modes
            KeyCode::F(1) => { self.open_help(); return true; }
            KeyCode::F(2) => { self.open_port_selector(); return true; }
            KeyCode::F(3) => { self.toggle_link(); return true; }
            KeyCode::F(6) => { self.open_song_settings(); return true; }
            KeyCode::F(7) => { self.open_instrument_list(); return true; }
            KeyCode::F(8) => { self.cycle_theme(); return true; }

            // Track cycling
            KeyCode::Tab => { self.cycle_track(1); return true; }
            KeyCode::BackTab => { self.cycle_track(-1); return true; }

            // Navigation
            KeyCode::Up => { self.move_cursor_up(1); return true; }
            KeyCode::Down => { self.move_cursor_down(1); return true; }
            KeyCode::Left => { self.move_cursor_left(); return true; }
            KeyCode::Right => { self.move_cursor_right(); return true; }
            KeyCode::PageUp => { self.move_cursor_up(16); return true; }
            KeyCode::PageDown => { self.move_cursor_down(16); return true; }
            KeyCode::Home => { self.cursor_row = 0; return true; }
            KeyCode::End => { self.cursor_row = self.current_pattern_rows() - 1; return true; }

            // Octave up/down
            KeyCode::Char('+') => {
                if self.current_octave < 9 { self.current_octave += 1; }
                return true;
            }
            KeyCode::Char('-') => {
                if self.current_octave > 0 { self.current_octave -= 1; }
                return true;
            }

            _ => {}
        }
        false
    }

    /// Cycle to next/previous track, wrapping around. Updates track page automatically.
    fn cycle_track(&mut self, dir: i32) {
        let n = self.song.channels;
        if n == 0 { return; }
        self.cursor_channel = if dir > 0 {
            (self.cursor_channel + 1) % n
        } else {
            (self.cursor_channel + n - 1) % n
        };
        self.cursor_sub = SubColumn::Note;
        self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) { return; }
        if self.handle_shared_key(key) { return; }

        // Ctrl combos specific to Normal mode
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => { self.add_new_pattern_to_order(); return; }
                KeyCode::Char('d') => { self.clone_current_pattern(); return; }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('q') => {
                if self.dirty {
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

    fn handle_insert_key(&mut self, key: KeyEvent) {
        if self.handle_common_key(key) { return; }
        if self.handle_shared_key(key) { return; }

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
            KeyCode::Char(c) => {
                match self.cursor_sub {
                    SubColumn::Note => self.try_enter_note(c),
                    SubColumn::Instrument => self.try_enter_hex_digit(c, SubColumn::Instrument),
                    SubColumn::Volume => self.try_enter_hex_digit(c, SubColumn::Volume),
                    SubColumn::Effect => self.try_enter_hex_digit(c, SubColumn::Effect),
                }
            }

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
        // Piano keyboard layout (lowercase):
        // z=C, s=C#, x=D, d=D#, c=E, v=F, g=F#, b=G, h=G#, n=A, j=A#, m=B
        // Upper octave:
        // q=C, 2=C#, w=D, 3=D#, e=E, r=F, 5=F#, t=G, 6=G#, y=A, 7=A#, u=B
        let (note_val, octave_offset) = match c {
            'z' => (NoteValue::C, 0),
            's' => (NoteValue::Cs, 0),
            'x' => (NoteValue::D, 0),
            'd' => (NoteValue::Ds, 0),
            'c' => (NoteValue::E, 0),
            'v' => (NoteValue::F, 0),
            'g' => (NoteValue::Fs, 0),
            'b' => (NoteValue::G, 0),
            'h' => (NoteValue::Gs, 0),
            'n' => (NoteValue::A, 0),
            'j' => (NoteValue::As, 0),
            'm' => (NoteValue::B, 0),
            'q' => (NoteValue::C, 1),
            '2' => (NoteValue::Cs, 1),
            'w' => (NoteValue::D, 1),
            '3' => (NoteValue::Ds, 1),
            'e' => (NoteValue::E, 1),
            'r' => (NoteValue::F, 1),
            '5' => (NoteValue::Fs, 1),
            't' => (NoteValue::G, 1),
            '6' => (NoteValue::Gs, 1),
            'y' => (NoteValue::A, 1),
            '7' => (NoteValue::As, 1),
            'u' => (NoteValue::B, 1),
            _ => return,
        };

        let octave = self.current_octave + octave_offset;
        if octave > 9 {
            return;
        }

        let note = Note::On {
            value: note_val,
            octave,
        };

        self.push_undo();

        // Determine instrument: use track default if set (Synth/Sample tracks), else cell's existing value
        let pattern_idx = self.song.order[self.current_order_position()];
        let ch = self.cursor_channel;
        let ch_type = self.channels.get(ch).map(|c| c.channel_type);
        let track_inst = if ch_type == Some(ChannelType::Synth) || ch_type == Some(ChannelType::Sample) {
            self.channels.get(ch).and_then(|c| c.default_instrument)
        } else {
            None
        };
        let current_inst = track_inst.or(self.song.patterns[pattern_idx].get(self.cursor_row, ch).instrument);

        // Preview the note
        if let Some(midi_note) = note.to_midi_note() {
            let midi_ch = self.midi_channel_for(ch);
            self.preview_note_with_instrument(midi_ch, midi_note, MIDI_DEFAULT_VELOCITY, current_inst);
        }

        // Write to pattern
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, ch);
        cell.note = Some(note);
        // Auto-fill instrument from track default
        if let Some(inst) = track_inst {
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

        self.push_undo();

        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);

        match sub {
            SubColumn::Instrument => {
                let current = cell.instrument.unwrap_or(0);
                // Shift left and add new digit (2 hex digits max)
                cell.instrument = Some(((current << 4) | digit) & 0xFF);
            }
            SubColumn::Volume => {
                let current = cell.volume.unwrap_or(0);
                cell.volume = Some(((current << 4) | digit) & 0xFF);
            }
            SubColumn::Effect => {
                // Effect is 1 hex digit for command + 2 hex digits for value
                // Simple approach: rotate through effect then effect_value
                if cell.effect.is_none() {
                    cell.effect = Some(digit);
                } else {
                    let current_val = cell.effect_value.unwrap_or(0);
                    cell.effect_value = Some(((current_val << 4) | digit) & 0xFF);
                }
            }
            SubColumn::Note => {} // handled separately
        }
    }

    fn enter_note_off(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        cell.note = Some(Note::Off);
        self.move_cursor_down(self.edit_step);
    }

    fn delete_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
        match self.cursor_sub {
            SubColumn::Note => cell.note = None,
            SubColumn::Instrument => cell.instrument = None,
            SubColumn::Volume => cell.volume = None,
            SubColumn::Effect => {
                cell.effect = None;
                cell.effect_value = None;
            }
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

    /// Get the number of rows in the current pattern (per-pattern length)
    fn current_pattern_rows(&self) -> usize {
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].rows
    }

    fn change_bpm(&mut self, delta: i16) {
        let new_bpm = (self.song.bpm as i16 + delta).clamp(32, 300) as u16;
        self.song.bpm = new_bpm;
        if self.link.is_enabled() {
            self.link.set_tempo(new_bpm as f64);
        }
    }

    fn change_edit_step(&mut self, delta: i16) {
        let new_step = (self.edit_step as i16 + delta).clamp(0, 16) as usize;
        self.edit_step = new_step;
        self.status_message = Some(format!("Edit step: {}", self.edit_step));
    }

    pub(crate) fn insert_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].insert_row(self.cursor_row);
        self.status_message = Some(format!("Inserted row at {:02X}", self.cursor_row));
    }

    pub(crate) fn delete_row_at_cursor(&mut self) {
        self.push_undo();
        let pattern_idx = self.song.order[self.current_order_position()];
        self.song.patterns[pattern_idx].delete_row(self.cursor_row);
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
                self.cursor_sub = SubColumn::Effect;
                self.track_page = self.cursor_channel / CHANNELS_PER_PAGE;
            }
        } else {
            self.cursor_sub = self.cursor_sub.prev();
        }
    }

    pub(crate) fn move_cursor_right(&mut self) {
        if self.cursor_sub == SubColumn::Effect {
            if self.cursor_channel < self.song.channels - 1 {
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
        if self.history.block_anchor.is_some() {
            self.history.block_anchor = None;
            self.status_message = Some("Block selection cleared".to_string());
        } else {
            self.history.block_anchor = Some((self.cursor_row, self.cursor_channel));
            self.status_message = Some("Block selection started".to_string());
        }
    }

    /// Get the block selection bounds: (row_start, row_end, ch_start, ch_end) inclusive
    pub fn block_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        self.history.block_anchor.map(|(anchor_row, anchor_ch)| {
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
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &self.song.patterns[pattern_idx];
            let mut block = Vec::new();
            for r in r0..=r1 {
                let mut row = Vec::new();
                for c in c0..=c1 {
                    row.push(*pattern.get(r, c));
                }
                block.push(row);
            }
            self.history.block_clipboard = Some(block);
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
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &mut self.song.patterns[pattern_idx];
            for r in r0..=r1 {
                for c in c0..=c1 {
                    pattern.set_cell(r, c, crate::tracker::Cell::default());
                }
            }
            self.history.block_anchor = None;
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
        let pattern_idx = self.song.order[self.current_order_position()];
        let pattern = &mut self.song.patterns[pattern_idx];
        let steps = (r1 - r0) as f64;

        for c in c0..=c1 {
            let first = *pattern.get(r0, c);
            let last = *pattern.get(r1, c);

            // Interpolate volume
            if let (Some(v0), Some(v1)) = (first.volume, last.volume) {
                for r in r0..=r1 {
                    let t = (r - r0) as f64 / steps;
                    let v = v0 as f64 + (v1 as f64 - v0 as f64) * t;
                    pattern.get_mut(r, c).volume = Some(v.round() as u8);
                }
            }

            // Interpolate effect_value (when both endpoints have the same effect command)
            if first.effect == last.effect && first.effect.is_some() {
                if let (Some(ev0), Some(ev1)) = (first.effect_value, last.effect_value) {
                    for r in r0..=r1 {
                        let t = (r - r0) as f64 / steps;
                        let ev = ev0 as f64 + (ev1 as f64 - ev0 as f64) * t;
                        pattern.get_mut(r, c).effect = first.effect;
                        pattern.get_mut(r, c).effect_value = Some(ev.round() as u8);
                    }
                }
            }
        }

        self.status_message = Some("Interpolated block".to_string());
    }

    /// Paste block clipboard at cursor position
    fn paste_block(&mut self) {
        if let Some(ref block) = self.history.block_clipboard.clone() {
            self.push_undo();
            let pattern_idx = self.song.order[self.current_order_position()];
            let pattern = &mut self.song.patterns[pattern_idx];
            for (ri, row) in block.iter().enumerate() {
                let r = self.cursor_row + ri;
                if r >= pattern.rows {
                    break;
                }
                for (ci, cell) in row.iter().enumerate() {
                    let c = self.cursor_channel + ci;
                    if c < pattern.channels {
                        pattern.set_cell(r, c, *cell);
                    }
                }
            }
            self.status_message = Some("Pasted block".to_string());
        }
    }
}

/// Transpose a single cell's note by the given number of semitones, clamping to valid MIDI range.
fn transpose_cell_note(cell: &mut crate::tracker::Cell, semitones: i8) {
    if let Some(Note::On { ref value, ref octave }) = cell.note {
        let semi = SEMITONES_PER_OCTAVE as i16;
        let midi = (*octave as i16) * semi + value.to_index() as i16 + semitones as i16;
        if midi >= 0 && midi <= MIDI_MAX_NOTE as i16 {
            let new_octave = (midi / semi) as u8;
            let new_note_idx = (midi % semi) as u8;
            if let Some(nv) = NoteValue::from_index(new_note_idx) {
                cell.note = Some(Note::On { value: nv, octave: new_octave });
            }
        }
    }
}
