//! Deterministic tracker playback engine.
//!
//! `TrackerEngine` advances a `Song` tick-by-tick and emits `TrackerEvent`s.
//! Consumers (TUI, offline renderer, MIDI export, tests) translate events
//! to their domain (MIDI messages, synth calls, file writes, assertions).

use crate::constants::*;
use crate::tracker::{Note, Song};

// ---------------------------------------------------------------------------
// Channel state
// ---------------------------------------------------------------------------

/// Per-channel state for continuous effects (arpeggio, portamento, vibrato, volume slide).
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub note: Option<u8>,
    pub volume: u8,
    pub pitch_offset: f64,
    pub porta_target: Option<u8>,
    pub vibrato_phase: f64,
    pub effect: Option<u8>,
    pub effect_param: u8,
    pub delayed_note: Option<(u8, u8, bool)>,
    pub delay_tick: u8,
    pub active_instrument: Option<u8>,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            note: None,
            volume: 100,
            pitch_offset: 0.0,
            porta_target: None,
            vibrato_phase: 0.0,
            effect: None,
            effect_param: 0,
            delayed_note: None,
            delay_tick: 0,
            active_instrument: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events emitted by the engine. Consumers translate these to their domain.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackerEvent {
    /// A new row has started.
    RowAdvanced {
        order: usize,
        row: usize,
        pattern: usize,
    },
    /// Note on.
    NoteOn {
        channel: usize,
        midi_note: u8,
        velocity: u8,
        instrument: Option<u8>,
    },
    /// Note off on a channel.
    NoteOff { channel: usize },
    /// Pitch bend change (semitone offset from base note).
    PitchBend {
        channel: usize,
        semitone_offset: f64,
    },
    /// Volume change on a channel (from volume slide).
    VolumeChange { channel: usize, volume: u8 },
    /// MIDI CC (Cxx effect).
    MidiCC {
        channel: usize,
        controller: u8,
        value: u8,
    },
    /// Program change (Exx effect).
    ProgramChange { channel: usize, program: u8 },
    /// Speed changed (Fxx < 0x20).
    SpeedChanged { speed: u8 },
    /// Tempo changed (Fxx >= 0x20, or tempo automation).
    TempoChanged { bpm: f64 },
    /// Playback generation incremented (wrapped past end of order list).
    GenerationAdvanced { generation: u32 },
}

// ---------------------------------------------------------------------------
// Per-channel info provided by the host
// ---------------------------------------------------------------------------

/// Per-channel configuration that the engine needs from the host.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    /// Whether this channel is audible (not muted/soloed-out).
    /// Inaudible channels still update state, but no events are emitted.
    pub audible: bool,
    /// Volume scale factor (0.0..1.0) applied to note velocity.
    pub volume_scale: f32,
    /// Default instrument for this channel (Synth track fallback).
    pub default_instrument: Option<u8>,
    /// Whether this channel is a Synth-type channel (for default instrument fallback).
    pub is_synth: bool,
}

impl Default for ChannelInfo {
    fn default() -> Self {
        Self {
            audible: true,
            volume_scale: 1.0,
            default_instrument: None,
            is_synth: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TrackerEngine {
    // -- Position --
    pub row: usize,
    pub order: usize,
    pub generation: u32,
    pub tick: u8,

    // -- Timing (local copies, never mutates Song) --
    pub speed: u8,
    pub bpm: f64,

    // -- Per-order repeat tracking --
    pub repeat_count: u8,

    // -- Per-channel effect state --
    pub channel_states: Vec<ChannelState>,

    // -- Per-channel host info --
    channel_info: Vec<ChannelInfo>,

    // -- Event buffer --
    events: Vec<TrackerEvent>,

    // -- Whether to wrap at end of order list --
    pub wrap_at_end: bool,

    // -- Set when the song finishes (order exhausted, no wrap) --
    pub finished: bool,
}

impl TrackerEngine {
    /// Create a new engine.
    pub fn new(song: &Song, wrap_at_end: bool) -> Self {
        let mut engine = Self {
            row: 0,
            order: 0,
            generation: 0,
            tick: 0,
            speed: song.speed,
            bpm: song.bpm as f64,
            repeat_count: 0,
            channel_states: vec![ChannelState::default(); song.channels],
            channel_info: vec![ChannelInfo::default(); song.channels],
            events: Vec::with_capacity(64),
            wrap_at_end,
            finished: false,
        };
        engine.skip_zero_repeats_forward(song);
        engine
    }

    /// Reset engine to the given starting position.
    pub fn reset(&mut self, song: &Song, start_order: usize, start_row: usize) {
        self.row = start_row;
        self.order = start_order;
        self.generation = 0;
        self.tick = 0;
        self.speed = song.speed;
        self.bpm = song.bpm as f64;
        self.repeat_count = 0;
        self.finished = false;
        self.channel_states = vec![ChannelState::default(); song.channels];
        self.events.clear();
        self.skip_zero_repeats_forward(song);
    }

    /// Update the channel info slice (call when mute/solo/volume changes).
    pub fn set_channel_info(&mut self, info: Vec<ChannelInfo>) {
        self.channel_info = info;
    }

    /// Update a single channel's info.
    pub fn update_channel_info(&mut self, ch: usize, info: ChannelInfo) {
        if ch >= self.channel_info.len() {
            self.channel_info.resize_with(ch + 1, ChannelInfo::default);
        }
        self.channel_info[ch] = info;
    }

    /// Process one sub-tick. Returns the events emitted during this tick.
    pub fn process_tick(&mut self, song: &Song) -> &[TrackerEvent] {
        self.events.clear();

        if self.finished {
            return &self.events;
        }

        if self.tick == 0 {
            self.advance_row(song);
        } else {
            self.process_effects(song);
        }

        self.tick += 1;
        if self.tick >= self.speed {
            self.tick = 0;
        }

        &self.events
    }

    /// Drain all events (alternative to the slice return).
    pub fn drain_events(&mut self) -> Vec<TrackerEvent> {
        std::mem::take(&mut self.events)
    }

    /// Get current seconds_per_tick (incorporating swing).
    pub fn seconds_per_tick(&self, song: &Song) -> f64 {
        let base_tps = (self.bpm * MIDI_CLOCKS_PER_BEAT) / 60.0;
        let base_spt = 1.0 / base_tps;
        if song.swing == 50 {
            base_spt
        } else {
            let swing_f = song.swing as f64;
            if self.row.is_multiple_of(2) {
                base_spt * swing_f / 50.0
            } else {
                base_spt * (100.0 - swing_f) / 50.0
            }
        }
    }

    // -- Private implementation --

    fn emit(&mut self, event: TrackerEvent) {
        self.events.push(event);
    }

    fn channel_audible(&self, ch: usize) -> bool {
        self.channel_info.get(ch).is_none_or(|ci| ci.audible)
    }

    fn channel_volume_scale(&self, ch: usize) -> f32 {
        self.channel_info.get(ch).map_or(1.0, |ci| ci.volume_scale)
    }

    fn resolve_instrument(&self, ch: usize, cell_instrument: Option<u8>) -> Option<u8> {
        cell_instrument.or_else(|| {
            let info = self.channel_info.get(ch)?;
            if info.is_synth {
                info.default_instrument
            } else {
                None
            }
        })
    }

    fn order_repeat_at(&self, song: &Song, idx: usize) -> u8 {
        song.order_repeats.get(idx).copied().unwrap_or(1)
    }

    fn advance_order_position(&mut self, song: &Song) {
        let repeat = self.order_repeat_at(song, self.order);
        self.repeat_count += 1;
        if repeat > 1 && self.repeat_count < repeat {
            return;
        }
        self.repeat_count = 0;
        self.order += 1;
        let arr_len = song.arrangement_len();
        if self.order >= arr_len {
            if self.wrap_at_end {
                self.order = 0;
                self.generation += 1;
                self.emit(TrackerEvent::GenerationAdvanced {
                    generation: self.generation,
                });
            } else {
                self.finished = true;
                return;
            }
        }
        self.skip_zero_repeats_forward(song);
    }

    fn skip_zero_repeats_forward(&mut self, song: &Song) {
        let len = song.arrangement_len();
        if len == 0 {
            self.finished = true;
            return;
        }
        let start = self.order;
        for _ in 0..len {
            if self.order_repeat_at(song, self.order) > 0 {
                return;
            }
            self.order += 1;
            if self.order >= len {
                if self.wrap_at_end {
                    self.order = 0;
                    self.generation += 1;
                    self.emit(TrackerEvent::GenerationAdvanced {
                        generation: self.generation,
                    });
                } else {
                    self.finished = true;
                    return;
                }
            }
            if self.order == start {
                return;
            }
        }
    }

    /// Tick 0: process the new row -- trigger notes, set up effect state, advance row pointer.
    fn advance_row(&mut self, song: &Song) {
        let arr_len = song.arrangement_len();
        if self.order >= arr_len {
            self.finished = true;
            return;
        }
        let channels = song.channels;

        // Ensure channel_states has enough entries
        while self.channel_states.len() < channels {
            self.channel_states.push(ChannelState::default());
        }

        // Emit RowAdvanced
        self.emit(TrackerEvent::RowAdvanced {
            order: self.order,
            row: self.row,
            pattern: self.order,
        });

        // Collect cell data via chain/phrase model, applying chain transpose
        type CellTuple = (Option<Note>, Option<u8>, Option<u8>, Option<u8>, Option<u8>);
        let cells: Vec<CellTuple> = (0..channels)
            .map(|ch| {
                let cell = song.cell_at(self.order, self.row, ch);
                let transpose = song.chain_transpose_at(self.order, ch);
                let note = if transpose != 0 {
                    cell.note.map(|n| n.transposed(transpose))
                } else {
                    cell.note
                };
                let inst = self.resolve_instrument(ch, cell.instrument);
                (note, cell.volume, cell.effect, cell.effect_value, inst)
            })
            .collect();

        // Scan for pattern-level effects
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
                        self.speed = val;
                        self.emit(TrackerEvent::SpeedChanged { speed: val });
                    } else if val >= 0x20 {
                        self.bpm = val as f64;
                        self.emit(TrackerEvent::TempoChanged { bpm: val as f64 });
                    }
                }
                _ => {}
            }
        }

        // Check tempo automation
        if let Some(bpm) = song.tempo_at(self.order, self.row) {
            if bpm >= 1.0 {
                self.bpm = bpm;
                self.emit(TrackerEvent::TempoChanged { bpm });
            }
        }

        // Process notes and tick-0 effects
        for (ch, (note, volume, effect, effect_value, instrument)) in cells.into_iter().enumerate()
        {
            let param = effect_value.unwrap_or(0);
            let is_tone_porta = effect == Some(EFFECT_TONE_PORTA);
            let audible = self.channel_audible(ch);

            if !audible {
                // Still update channel state for muted channels
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

            // Clear previous delayed note
            self.channel_states[ch].delayed_note = None;

            let is_note_delay = effect == Some(EFFECT_NOTE_DELAY) && param > 0;

            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else if is_note_delay {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            self.channel_states[ch].delayed_note =
                                Some((midi_note, scaled_vel, false));
                            self.channel_states[ch].delay_tick = param;
                        } else {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            // Reset pitch bend on new note
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.emit(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                            self.emit(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: scaled_vel,
                                instrument,
                            });
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].active_instrument = instrument;
                            self.channel_states[ch].volume = vel; // store unscaled
                        }
                    }
                }
                Some(Note::Off) => {
                    if is_note_delay {
                        self.channel_states[ch].delayed_note = Some((0, 0, true));
                        self.channel_states[ch].delay_tick = param;
                    } else {
                        self.emit(TrackerEvent::NoteOff { channel: ch });
                        self.channel_states[ch].note = None;
                        self.channel_states[ch].pitch_offset = 0.0;
                        self.emit(TrackerEvent::PitchBend {
                            channel: ch,
                            semitone_offset: 0.0,
                        });
                    }
                }
                None => {
                    if let Some(vol) = volume {
                        self.channel_states[ch].volume = vol;
                    }
                }
            }

            // Store effect state
            self.channel_states[ch].effect = effect;
            self.channel_states[ch].effect_param = param;

            // Tick-0 immediate effects
            match effect {
                Some(EFFECT_MIDI_CC) => {
                    let controller = instrument.unwrap_or(0);
                    self.emit(TrackerEvent::MidiCC {
                        channel: ch,
                        controller,
                        value: param,
                    });
                }
                Some(EFFECT_PROGRAM_CHANGE) => {
                    self.emit(TrackerEvent::ProgramChange {
                        channel: ch,
                        program: param,
                    });
                }
                _ => {}
            }
        }

        // Position jump (Bxx)
        if let Some(target_order) = jump_order {
            let target = target_order.min(arr_len - 1);
            if target <= self.order {
                self.generation += 1;
                self.emit(TrackerEvent::GenerationAdvanced {
                    generation: self.generation,
                });
            }
            self.order = target;
            self.repeat_count = 0;
            self.skip_zero_repeats_forward(song);
            if !self.finished {
                let target_rows = song.phrase_rows_at(self.order);
                self.row = break_row.unwrap_or(0).min(target_rows - 1);
            }
            return;
        }

        // Pattern break (Dxx)
        if let Some(target_row) = break_row {
            self.advance_order_position(song);
            if !self.finished {
                let target_rows = song.phrase_rows_at(self.order);
                self.row = target_row.min(target_rows - 1);
            }
            return;
        }

        // Normal advance
        self.row += 1;
        let phrase_rows = song.phrase_rows_at(self.order);
        if self.row >= phrase_rows {
            self.row = 0;
            self.advance_order_position(song);
        }
    }

    /// Ticks 1..speed-1: process continuous effects.
    fn process_effects(&mut self, _song: &Song) {
        let channels = self.channel_states.len();
        for ch in 0..channels {
            if !self.channel_audible(ch) {
                continue;
            }

            // Note delay
            if self.channel_states[ch].effect == Some(EFFECT_NOTE_DELAY) {
                if let Some((midi_note, vel, is_off)) = self.channel_states[ch].delayed_note {
                    if self.tick == self.channel_states[ch].delay_tick {
                        if is_off {
                            self.events.push(TrackerEvent::NoteOff { channel: ch });
                            self.channel_states[ch].note = None;
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                        } else {
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                            self.events.push(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: vel,
                                instrument: self.channel_states[ch].active_instrument,
                            });
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
            let _ = base_note; // used by some effects below

            match effect {
                Some(EFFECT_ARPEGGIO) if param != 0 => {
                    let x = (param >> 4) as f64;
                    let y = (param & 0x0F) as f64;
                    let phase = self.tick % 3;
                    let offset = match phase {
                        0 => 0.0,
                        1 => x,
                        _ => y,
                    };
                    self.events.push(TrackerEvent::PitchBend {
                        channel: ch,
                        semitone_offset: offset,
                    });
                }
                Some(EFFECT_PORTA_UP) => {
                    self.channel_states[ch].pitch_offset += param as f64 / 16.0;
                    self.events.push(TrackerEvent::PitchBend {
                        channel: ch,
                        semitone_offset: self.channel_states[ch].pitch_offset,
                    });
                }
                Some(EFFECT_PORTA_DOWN) => {
                    self.channel_states[ch].pitch_offset -= param as f64 / 16.0;
                    self.events.push(TrackerEvent::PitchBend {
                        channel: ch,
                        semitone_offset: self.channel_states[ch].pitch_offset,
                    });
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
                        self.events.push(TrackerEvent::PitchBend {
                            channel: ch,
                            semitone_offset: self.channel_states[ch].pitch_offset,
                        });
                    }
                }
                Some(EFFECT_VIBRATO) => {
                    let speed_v = (param >> 4) as f64;
                    let depth = (param & 0x0F) as f64;
                    self.channel_states[ch].vibrato_phase += speed_v / 64.0;
                    if self.channel_states[ch].vibrato_phase >= 1.0 {
                        self.channel_states[ch].vibrato_phase -= 1.0;
                    }
                    let sine =
                        (self.channel_states[ch].vibrato_phase * std::f64::consts::TAU).sin();
                    let offset = sine * depth / 16.0;
                    let total = self.channel_states[ch].pitch_offset + offset;
                    self.events.push(TrackerEvent::PitchBend {
                        channel: ch,
                        semitone_offset: total,
                    });
                }
                Some(EFFECT_VOLUME_SLIDE) => {
                    let up = (param >> 4) as i16;
                    let down = (param & 0x0F) as i16;
                    let delta = up - down;
                    let new_vol = (self.channel_states[ch].volume as i16 + delta)
                        .clamp(0, MIDI_MAX_VALUE as i16) as u8;
                    self.channel_states[ch].volume = new_vol;
                    self.events.push(TrackerEvent::VolumeChange {
                        channel: ch,
                        volume: new_vol,
                    });
                }
                _ => {}
            }
        }
    }

    fn scale_velocity(&self, ch: usize, vel: u8) -> u8 {
        let scale = self.channel_volume_scale(ch);
        (vel as f32 * scale)
            .round()
            .clamp(0.0, MIDI_MAX_VALUE as f32) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{Cell, Note, NoteValue, Song};

    fn make_song() -> Song {
        Song::new(2, 16)
    }

    #[test]
    fn test_engine_new() {
        let song = make_song();
        let engine = TrackerEngine::new(&song, true);
        assert_eq!(engine.row, 0);
        assert_eq!(engine.order, 0);
        assert_eq!(engine.generation, 0);
        assert_eq!(engine.tick, 0);
        assert_eq!(engine.speed, song.speed);
        assert_eq!(engine.bpm, song.bpm as f64);
        assert!(!engine.finished);
    }

    #[test]
    fn test_engine_advance_emits_row() {
        let song = make_song();
        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.iter().any(|e| matches!(
            e,
            TrackerEvent::RowAdvanced {
                row: 0,
                order: 0,
                ..
            }
        )));
    }

    #[test]
    fn test_engine_note_on_off() {
        let mut song = make_song();
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                ..Cell::default()
            },
        );
        song.set_cell(0,
            1,
            0,
            Cell {
                note: Some(Note::Off),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);

        // Tick through row 0
        let events = engine.process_tick(&song).to_vec();
        assert!(events.iter().any(|e| matches!(
            e,
            TrackerEvent::NoteOn {
                channel: 0,
                midi_note: 48,
                velocity: 100,
                ..
            }
        )));

        // Advance through remaining ticks of row 0
        for _ in 1..song.speed {
            engine.process_tick(&song);
        }

        // Row 1: note off
        let events = engine.process_tick(&song).to_vec();
        assert!(events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOff { channel: 0 })));
    }

    #[test]
    fn test_engine_portamento_up() {
        let mut song = Song::new(1, 4);
        song.speed = 3;
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                effect: Some(EFFECT_PORTA_UP),
                effect_value: Some(0x10),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        // Tick 0
        engine.process_tick(&song);
        // Tick 1: should emit pitch bend
        let events = engine.process_tick(&song).to_vec();
        let pb = events
            .iter()
            .find(|e| matches!(e, TrackerEvent::PitchBend { .. }));
        assert!(pb.is_some());
        if let Some(TrackerEvent::PitchBend {
            semitone_offset, ..
        }) = pb
        {
            assert!(*semitone_offset > 0.0);
        }
    }

    #[test]
    fn test_engine_volume_slide() {
        let mut song = Song::new(1, 4);
        song.speed = 3;
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                effect: Some(EFFECT_VOLUME_SLIDE),
                effect_value: Some(0x0F), // down by 15
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song); // tick 0
        let events = engine.process_tick(&song).to_vec(); // tick 1
        let vc = events
            .iter()
            .find(|e| matches!(e, TrackerEvent::VolumeChange { .. }));
        assert!(vc.is_some());
        if let Some(TrackerEvent::VolumeChange { volume, .. }) = vc {
            assert_eq!(*volume, 85); // 100 - 15
        }
    }

    #[test]
    fn test_engine_set_speed_tempo() {
        let mut song = Song::new(1, 4);
        song.set_cell(0,
            0,
            0,
            Cell {
                effect: Some(EFFECT_SET_SPEED),
                effect_value: Some(3), // set speed to 3
                ..Cell::default()
            },
        );
        song.set_cell(0,
            1,
            0,
            Cell {
                effect: Some(EFFECT_SET_SPEED),
                effect_value: Some(0x80), // set BPM to 128
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        assert!(events
            .iter()
            .any(|e| matches!(e, TrackerEvent::SpeedChanged { speed: 3 })));
        assert_eq!(engine.speed, 3);

        // Advance through remaining ticks
        for _ in 1..3 {
            engine.process_tick(&song);
        }
        let events = engine.process_tick(&song).to_vec();
        assert!(events
            .iter()
            .any(|e| matches!(e, TrackerEvent::TempoChanged { bpm } if *bpm == 128.0)));
        assert_eq!(engine.bpm, 128.0);
    }

    #[test]
    fn test_engine_position_jump() {
        let mut song = Song::new(1, 4);
        song.order = vec![0, 0, 0];
        song.rebuild_phrases_from_patterns();
        song.set_cell(0,
            0,
            0,
            Cell {
                effect: Some(EFFECT_POSITION_JUMP),
                effect_value: Some(2),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        assert_eq!(engine.order, 2);
        assert_eq!(engine.row, 0);
    }

    #[test]
    fn test_engine_pattern_break() {
        let mut song = Song::new(1, 16);
        song.order = vec![0, 0];
        song.rebuild_phrases_from_patterns();
        song.set_cell(0,
            0,
            0,
            Cell {
                effect: Some(EFFECT_PATTERN_BREAK),
                effect_value: Some(8),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        assert_eq!(engine.order, 1);
        assert_eq!(engine.row, 8);
    }

    #[test]
    fn test_engine_generation_wrap() {
        let mut song = Song::new(1, 2);
        song.order = vec![0];
        let mut engine = TrackerEngine::new(&song, true);

        // Process all ticks of row 0 and row 1, then should wrap
        for _ in 0..(2 * song.speed as usize) {
            engine.process_tick(&song);
        }
        assert_eq!(engine.generation, 1);
    }

    #[test]
    fn test_engine_no_wrap_finishes() {
        let mut song = Song::new(1, 2);
        song.order = vec![0];
        let mut engine = TrackerEngine::new(&song, false);

        for _ in 0..(2 * song.speed as usize) {
            engine.process_tick(&song);
        }
        assert!(engine.finished);
    }

    #[test]
    fn test_engine_muted_channel_no_events() {
        let mut song = make_song();
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        engine.update_channel_info(
            0,
            ChannelInfo {
                audible: false,
                ..Default::default()
            },
        );
        let events = engine.process_tick(&song).to_vec();
        // Should not emit NoteOn for muted channel
        assert!(!events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { channel: 0, .. })));
        // But state should still be updated
        assert_eq!(engine.channel_states[0].note, Some(48));
    }

    #[test]
    fn test_engine_program_change() {
        let mut song = make_song();
        song.set_cell(0,
            0,
            0,
            Cell {
                effect: Some(EFFECT_PROGRAM_CHANGE),
                effect_value: Some(5),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.iter().any(|e| matches!(
            e,
            TrackerEvent::ProgramChange {
                channel: 0,
                program: 5
            }
        )));
    }

    #[test]
    fn test_engine_midi_cc() {
        let mut song = make_song();
        song.set_cell(0,
            0,
            0,
            Cell {
                effect: Some(EFFECT_MIDI_CC),
                effect_value: Some(64),
                instrument: Some(7),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.iter().any(|e| matches!(
            e,
            TrackerEvent::MidiCC {
                channel: 0,
                controller: 7,
                value: 64
            }
        )));
    }

    #[test]
    fn test_engine_note_delay() {
        let mut song = Song::new(1, 4);
        song.speed = 4;
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                effect: Some(EFFECT_NOTE_DELAY),
                effect_value: Some(2),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        // Tick 0: no note on
        let events = engine.process_tick(&song).to_vec();
        assert!(!events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { .. })));
        // Tick 1: no note on
        let events = engine.process_tick(&song).to_vec();
        assert!(!events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { .. })));
        // Tick 2: note on
        let events = engine.process_tick(&song).to_vec();
        assert!(events.iter().any(|e| matches!(
            e,
            TrackerEvent::NoteOn {
                channel: 0,
                midi_note: 48,
                ..
            }
        )));
    }

    #[test]
    fn test_engine_arpeggio() {
        let mut song = Song::new(1, 4);
        song.speed = 6;
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                effect: Some(EFFECT_ARPEGGIO),
                effect_value: Some(0x37), // x=3, y=7
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song); // tick 0
                                    // Tick 1: phase 1 -> offset = 3
        let events = engine.process_tick(&song).to_vec();
        let pb = events.iter().find_map(|e| {
            if let TrackerEvent::PitchBend {
                semitone_offset, ..
            } = e
            {
                Some(*semitone_offset)
            } else {
                None
            }
        });
        assert_eq!(pb, Some(3.0));
        // Tick 2: phase 2 -> offset = 7
        let events = engine.process_tick(&song).to_vec();
        let pb = events.iter().find_map(|e| {
            if let TrackerEvent::PitchBend {
                semitone_offset, ..
            } = e
            {
                Some(*semitone_offset)
            } else {
                None
            }
        });
        assert_eq!(pb, Some(7.0));
    }

    #[test]
    fn test_engine_seconds_per_tick_swing() {
        let mut song = make_song();
        song.swing = 75; // heavy swing

        let engine_even = TrackerEngine {
            row: 0,
            order: 0,
            generation: 0,
            tick: 0,
            speed: song.speed,
            bpm: song.bpm as f64,
            repeat_count: 0,
            channel_states: vec![],
            channel_info: vec![],
            events: vec![],
            wrap_at_end: true,
            finished: false,
        };
        let engine_odd = TrackerEngine {
            row: 1,
            ..engine_even.clone()
        };

        let spt_even = engine_even.seconds_per_tick(&song);
        let spt_odd = engine_odd.seconds_per_tick(&song);
        assert!(
            spt_even > spt_odd,
            "Even row should be longer with swing > 50"
        );
        // Total should be conserved
        let base_spt = 1.0 / ((song.bpm as f64 * MIDI_CLOCKS_PER_BEAT) / 60.0);
        assert!((spt_even + spt_odd - 2.0 * base_spt).abs() < 1e-10);
    }

    #[test]
    fn test_engine_order_repeats() {
        let mut song = Song::new(1, 2);
        song.order = vec![0, 0];
        song.order_repeats = vec![2, 1]; // first entry plays twice
        song.rebuild_phrases_from_patterns();

        let mut engine = TrackerEngine::new(&song, false);
        // Pattern has 2 rows, speed 6. Each full pattern = 2 * 6 = 12 ticks.
        // First order entry repeats twice = 24 ticks. Second entry plays once = 12 ticks.
        for _ in 0..24 {
            engine.process_tick(&song);
        }
        assert_eq!(engine.order, 1);
        assert!(!engine.finished);

        for _ in 0..12 {
            engine.process_tick(&song);
        }
        assert!(engine.finished);
    }

    #[test]
    fn test_engine_chain_transpose() {
        let mut song = Song::new(1, 4);
        song.set_cell(0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                ..Cell::default()
            },
        );

        // Set transpose +7 semitones on channel 0's chain
        let chain_idx = song.arrangement[0][0].unwrap();
        song.chains[chain_idx].entries[0].transpose = 7;

        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();

        // Should emit NoteOn with C4+7 = G4 = MIDI 55
        let note_on = events.iter().find(|e| matches!(e, TrackerEvent::NoteOn { .. }));
        assert!(note_on.is_some(), "Expected NoteOn event");
        match note_on.unwrap() {
            TrackerEvent::NoteOn { midi_note, .. } => {
                assert_eq!(*midi_note, 55, "C4 + 7 semitones = G4 = MIDI 55");
            }
            _ => unreachable!(),
        }
    }
}
