use std::time::Instant;

use crate::midi::MidiInputEvent;
use crate::tracker::{Note, NoteValue};

use super::{
    App, ChannelState, Mode,
    EFFECT_ARPEGGIO, EFFECT_MIDI_CC, EFFECT_NOTE_DELAY, EFFECT_PATTERN_BREAK,
    EFFECT_PORTA_DOWN, EFFECT_PORTA_UP, EFFECT_POSITION_JUMP, EFFECT_PROGRAM_CHANGE,
    EFFECT_SET_SPEED, EFFECT_TONE_PORTA, EFFECT_VIBRATO, EFFECT_VOLUME_SLIDE,
    PITCH_BEND_CENTER, PITCH_BEND_PER_SEMITONE,
};

impl App {
    // -- Playback --

    pub fn toggle_link(&mut self) {
        if self.link.is_enabled() {
            self.link.disable();
        } else {
            self.link.enable();
        }
    }

    pub fn toggle_playback(&mut self) {
        if self.playing {
            self.stop();
        } else {
            self.play();
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.playback_row = self.cursor_row;
        self.playback_order = 0;
        self.playback_generation = 0;
        self.playback_tick = 0;
        self.last_tick = Some(Instant::now());
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;
        // Reset channel states and ensure we have enough for all channels
        let ch_count = self.song.channels;
        self.channel_states = vec![ChannelState::default(); ch_count];

        if self.link.is_enabled() {
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.last_tick = None;
        // Reset pitch bends to center before killing notes
        for ch in 0..self.channel_states.len() {
            let midi_ch = self.midi_channel_for(ch);
            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
        }
        self.send_all_notes_off();
        let _ = self.midi.send_stop();

        if self.link.is_enabled() {
            self.link.request_stop();
        }
    }

    /// Sync tempo from Link peers if changed externally
    pub fn sync_link(&mut self) {
        if !self.link.is_enabled() {
            return;
        }

        if let Some(new_tempo) = self.link.poll_tempo_change() {
            let new_bpm = new_tempo.round() as u16;
            if new_bpm != self.song.bpm && new_bpm >= 32 && new_bpm <= 300 {
                self.song.bpm = new_bpm;
            }
        }
    }

    pub fn tick_playback(&mut self) {
        if !self.playing {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_tick {
            let elapsed = now.duration_since(last).as_secs_f64();
            self.tick_accumulator += elapsed;

            // Send MIDI clock: 24 ppqn (pulses per quarter note)
            if self.midi.clock_enabled {
                self.clock_tick_accumulator += elapsed;
                let clock_interval = 60.0 / (self.song.bpm as f64 * 24.0);
                while self.clock_tick_accumulator >= clock_interval {
                    self.clock_tick_accumulator -= clock_interval;
                    let _ = self.midi.send_clock();
                }
            }

            let spt = self.song.seconds_per_tick();
            while self.tick_accumulator >= spt {
                self.tick_accumulator -= spt;
                self.process_tick();
            }
        }
        self.last_tick = Some(now);
    }

    /// Process a single sub-tick. Tick 0 = new row (notes + row effects). Ticks 1+ = continuous effects.
    pub(crate) fn process_tick(&mut self) {
        if self.playback_tick == 0 {
            self.advance_playback();
            if self.follow_playback {
                self.cursor_row = self.playback_row;
                self.edit_order = self.playback_order;
            }
        } else {
            self.process_effects_tick();
        }
        self.playback_tick += 1;
        if self.playback_tick >= self.song.speed {
            self.playback_tick = 0;
        }
    }

    /// Tick 0: process the new row -- trigger notes, set up channel effect state, advance row pointer.
    pub(crate) fn advance_playback(&mut self) {
        let pattern_idx = self.song.order[self.playback_order];
        let pattern_rows = self.song.patterns[pattern_idx].rows;
        let channels = self.song.patterns[pattern_idx].channels;

        // Ensure channel_states has enough entries
        while self.channel_states.len() < channels {
            self.channel_states.push(ChannelState::default());
        }

        // Collect cell data we need before mutating self
        let cells: Vec<(Option<Note>, Option<u8>, Option<u8>, Option<u8>, Option<u8>)> = (0..channels)
            .map(|ch| {
                let cell = self.song.patterns[pattern_idx].get(self.playback_row, ch);
                (cell.note, cell.volume, cell.effect, cell.effect_value, cell.instrument)
            })
            .collect();

        // Scan for pattern-level effects (first one wins)
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        for &(_, _, effect, effect_value, _) in &cells {
            match effect {
                Some(EFFECT_POSITION_JUMP) => {
                    jump_order = Some(effect_value.unwrap_or(0) as usize);
                }
                Some(EFFECT_PATTERN_BREAK) => {
                    break_row = Some(effect_value.unwrap_or(0) as usize);
                }
                Some(EFFECT_SET_SPEED) => {
                    let val = effect_value.unwrap_or(0);
                    if val > 0 && val < 0x20 {
                        self.song.speed = val;
                    } else if val >= 0x20 {
                        self.song.bpm = val as u16;
                    }
                }
                _ => {}
            }
        }

        // Play the current row and set up per-channel effect state
        for (ch, (note, volume, effect, effect_value, instrument)) in cells.into_iter().enumerate() {
            let midi_ch = self.midi_channel_for(ch);
            let param = effect_value.unwrap_or(0);

            // For tone portamento (3xx), a new note sets the target instead of triggering
            let is_tone_porta = effect == Some(EFFECT_TONE_PORTA);

            if !self.is_channel_audible(ch) {
                // Still update channel state for muted channels so effects resume correctly
                if let Some(Note::On { .. }) = note {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else {
                            self.channel_states[ch].note = Some(midi_note);
                        }
                    }
                }
                self.channel_states[ch].effect = effect;
                self.channel_states[ch].effect_param = param;
                continue;
            }

            // Clear any previous delayed note
            self.channel_states[ch].delayed_note = None;

            // Note delay (6xx): defer note trigger to tick xx
            let is_note_delay = effect == Some(EFFECT_NOTE_DELAY) && param > 0;

            // Process notes
            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            // Tone portamento: set target, don't retrigger
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else if is_note_delay {
                            // Defer note trigger to the specified tick
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            self.channel_states[ch].delayed_note = Some((midi_note, vel, false));
                            self.channel_states[ch].delay_tick = param;
                        } else {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            // Reset pitch bend on new note
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                            self.send_note_on_with_instrument(midi_ch, midi_note, vel, instrument);
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = vel;
                        }
                    }
                }
                Some(Note::Off) => {
                    if is_note_delay {
                        // Defer note-off to the specified tick
                        self.channel_states[ch].delayed_note = Some((0, 0, true));
                        self.channel_states[ch].delay_tick = param;
                    } else {
                        self.send_channel_note_off(midi_ch);
                        self.channel_states[ch].note = None;
                        self.channel_states[ch].pitch_offset = 0.0;
                        self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                    }
                }
                None => {
                    // No note: update volume if specified
                    if let Some(vol) = volume {
                        self.channel_states[ch].volume = vol;
                    }
                }
            }

            // Store effect state for subsequent ticks
            self.channel_states[ch].effect = effect;
            self.channel_states[ch].effect_param = param;

            // Process immediate (tick 0) effects
            match effect {
                Some(EFFECT_MIDI_CC) => {
                    let controller = instrument.unwrap_or(0);
                    self.send_cc(midi_ch, controller, param);
                }
                Some(EFFECT_PROGRAM_CHANGE) => {
                    self.send_program_change(midi_ch, param);
                }
                _ => {}
            }
        }

        // Process position jump (Bxx)
        if let Some(target_order) = jump_order {
            let target = target_order.min(self.song.order.len() - 1);
            if target <= self.playback_order {
                self.playback_generation += 1;
            }
            self.playback_order = target;
            let target_pattern = self.song.order[self.playback_order];
            let target_rows = self.song.patterns[target_pattern].rows;
            self.playback_row = break_row.unwrap_or(0).min(target_rows - 1);
            return;
        }

        // Process pattern break (Dxx)
        if let Some(target_row) = break_row {
            self.playback_order += 1;
            if self.playback_order >= self.song.order.len() {
                self.playback_order = 0;
                self.playback_generation += 1;
            }
            let target_pattern = self.song.order[self.playback_order];
            let target_rows = self.song.patterns[target_pattern].rows;
            self.playback_row = target_row.min(target_rows - 1);
            return;
        }

        // Normal advance
        self.playback_row += 1;
        if self.playback_row >= pattern_rows {
            self.playback_row = 0;
            self.playback_order += 1;
            if self.playback_order >= self.song.order.len() {
                self.playback_order = 0;
                self.playback_generation += 1;
            }
        }
    }

    /// Ticks 1..speed-1: process continuous effects (arpeggio, portamento, vibrato, volume slide).
    pub(crate) fn process_effects_tick(&mut self) {
        let channels = self.channel_states.len();
        for ch in 0..channels {
            if !self.is_channel_audible(ch) {
                continue;
            }
            let midi_ch = self.midi_channel_for(ch);

            // Process note delay before other effects (note may not exist yet)
            if self.channel_states[ch].effect == Some(EFFECT_NOTE_DELAY) {
                if let Some((midi_note, vel, is_off)) = self.channel_states[ch].delayed_note {
                    if self.playback_tick == self.channel_states[ch].delay_tick {
                        if is_off {
                            self.send_channel_note_off(midi_ch);
                            self.channel_states[ch].note = None;
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                        } else {
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
                            self.send_note_on(midi_ch, midi_note, vel);
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = vel;
                        }
                        self.channel_states[ch].delayed_note = None;
                    }
                }
                continue;
            }

            let effect = self.channel_states[ch].effect;
            let param = self.channel_states[ch].effect_param;
            let base_note = match self.channel_states[ch].note {
                Some(n) => n,
                None => continue,
            };

            match effect {
                Some(EFFECT_ARPEGGIO) if param != 0 => {
                    let x = (param >> 4) as u8;
                    let y = (param & 0x0F) as u8;
                    // Cycle through base, base+x, base+y on ticks 1, 2, 3...
                    let phase = self.playback_tick % 3;
                    let offset = match phase {
                        0 => 0.0,
                        1 => x as f64,
                        _ => y as f64,
                    };
                    let bend = (offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_PORTA_UP) => {
                    // Slide pitch up by param units per tick (param in 16ths of a semitone)
                    self.channel_states[ch].pitch_offset += param as f64 / 16.0;
                    let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_PORTA_DOWN) => {
                    self.channel_states[ch].pitch_offset -= param as f64 / 16.0;
                    let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_TONE_PORTA) => {
                    if let Some(target) = self.channel_states[ch].porta_target {
                        let current = base_note as f64 + self.channel_states[ch].pitch_offset;
                        let target_f = target as f64;
                        let speed = param as f64 / 16.0;
                        if current < target_f {
                            self.channel_states[ch].pitch_offset += speed.min(target_f - current);
                        } else if current > target_f {
                            self.channel_states[ch].pitch_offset -= speed.min(current - target_f);
                        }
                        let bend = (self.channel_states[ch].pitch_offset * PITCH_BEND_PER_SEMITONE) as i32;
                        let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                        self.send_pitch_bend(midi_ch, value);
                    }
                }
                Some(EFFECT_VIBRATO) => {
                    let speed = (param >> 4) as f64;
                    let depth = (param & 0x0F) as f64;
                    self.channel_states[ch].vibrato_phase += speed / 64.0;
                    if self.channel_states[ch].vibrato_phase >= 1.0 {
                        self.channel_states[ch].vibrato_phase -= 1.0;
                    }
                    let sine = (self.channel_states[ch].vibrato_phase * std::f64::consts::TAU).sin();
                    let offset = sine * depth / 16.0; // depth in 16ths of a semitone
                    let total = self.channel_states[ch].pitch_offset + offset;
                    let bend = (total * PITCH_BEND_PER_SEMITONE) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, 0x3FFF) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                Some(EFFECT_VOLUME_SLIDE) => {
                    let up = (param >> 4) as i16;
                    let down = (param & 0x0F) as i16;
                    let delta = up - down;
                    let new_vol = (self.channel_states[ch].volume as i16 + delta).clamp(0, 127) as u8;
                    self.channel_states[ch].volume = new_vol;
                    // Send volume as CC 7
                    self.send_cc(midi_ch, 7, new_vol);
                }
                _ => {}
            }
        }
    }

    // -- MIDI input handling --

    /// Process incoming MIDI note events from external controllers
    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    pub(crate) fn handle_midi_input(&mut self, event: MidiInputEvent) {
        // Only enter notes in Insert mode when not playing
        if self.mode != Mode::Insert || self.playing {
            // Still preview the note
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.preview_note(midi_ch, event.note, event.velocity);
            return;
        }

        // Convert MIDI note number to Note
        let octave = event.note / 12;
        let note_index = event.note % 12;
        if let Some(note_val) = NoteValue::from_index(note_index) {
            let note = Note::On {
                value: note_val,
                octave,
            };

            self.push_undo();

            // Preview the note
            let midi_ch = self.midi_channel_for(self.cursor_channel);
            self.preview_note(midi_ch, event.note, event.velocity);

            // Write to pattern
            let pattern_idx = self.song.order[self.current_order_position()];
            let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
            cell.note = Some(note);
            cell.volume = Some(event.velocity);

            // Advance cursor
            self.move_cursor_down(self.edit_step);
        }
    }
}
