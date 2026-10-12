//! Deterministic tracker playback engine.
//!
//! `TrackerEngine` advances a `Song` tick-by-tick and emits `TrackerEvent`s.
//! Consumers (TUI, offline renderer, MIDI export, tests) translate events
//! to their domain (MIDI messages, synth calls, file writes, assertions).

use crate::constants::*;
use crate::tracker::{Fx, FxCmd, Note, Song, FX_COLUMNS};

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
    /// Vibrato phase per effect column, and one for the table's effect, so
    /// vibratos run independently.
    pub vibrato_phase: [f64; FX_COLUMNS + 1],
    /// The row's effect in each column, after `RND` is resolved.
    pub fx: [Option<Fx>; FX_COLUMNS],
    /// Note, velocity, note-off flag and sample offset of a `DEL` note.
    pub delayed_note: Option<(u8, u8, bool, u8)>,
    pub delay_tick: u8,
    pub active_instrument: Option<u8>,
    /// The table playing for the sounding note.
    pub table: Option<TableState>,
    /// The table a `DEL` note starts when it sounds.
    pub delayed_table: Option<u8>,
    /// Last continuous effect written in each column, with the parameter as
    /// written. `RND` varies that parameter rather than the last randomized
    /// one, so repeated `RND` rows deviate from the written value instead of
    /// random-walking away from it.
    pub last_continuous: [Option<Fx>; FX_COLUMNS],
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            note: None,
            volume: 100,
            pitch_offset: 0.0,
            porta_target: None,
            vibrato_phase: [0.0; FX_COLUMNS + 1],
            fx: [None; FX_COLUMNS],
            delayed_note: None,
            delay_tick: 0,
            active_instrument: None,
            table: None,
            delayed_table: None,
            last_continuous: [None; FX_COLUMNS],
        }
    }
}

/// Where a channel is in its table (`docs/dev/g7-tables.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableState {
    pub number: u8,
    pub row: usize,
    /// Ticks the current row has played.
    pub tick: u8,
    /// Past the last row of a table that does not loop: the last row holds.
    pub held: bool,
    /// The pitch offset last applied, so a return to 0 is bent back.
    pub transpose: i8,
}

/// What one tick of a table contributes.
struct TableStep {
    transpose: i8,
    previous_transpose: i8,
    volume: Option<u8>,
    fx: Option<Fx>,
    /// The row starts on this tick, so its volume and `Cnn`/`PRG` apply.
    row_started: bool,
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
        /// `9xx`: start a sample `xx`/256 of the way into its played span.
        /// 0 for every other note, and ignored by synths and MIDI.
        sample_offset: u8,
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
    /// Instrument for a note on this channel that names none.
    pub default_instrument: Option<u8>,
}

impl Default for ChannelInfo {
    fn default() -> Self {
        Self {
            audible: true,
            volume_scale: 1.0,
            default_instrument: None,
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
    /// The row whose notes are currently sounding.
    ///
    /// `row`/`order` are a *write* pointer: `advance_row` emits a row and
    /// then moves them on to the next one, so for ticks 1..speed-1 they
    /// already name the following row. Anything that describes what is being
    /// played -- tick length under swing, the UI playback cursor -- needs
    /// this instead.
    pub sounding_row: usize,
    /// Order position whose notes are currently sounding. See `sounding_row`.
    pub sounding_order: usize,
    pub generation: u32,
    pub tick: u8,

    // -- Timing (local copies, never mutates Song) --
    pub speed: u8,
    pub bpm: f64,

    // -- Per-order repeat tracking --
    pub repeat_count: u8,
    /// Rows played since the song started, along the path playback took.
    /// Running phrases play row `rows_played % len`; jumps and the wrap at
    /// the end keep counting, so their phase never re-aligns by itself.
    pub rows_played: u64,

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

    // -- PRNG for 7xx and 8xx --
    //
    // Seeded to a fixed value on reset, so a song plays the same way every
    // time and an offline render matches what was heard in the editor.
    rng: u64,
}

/// Fixed PRNG seed. See `TrackerEngine::rng`.
const RNG_SEED: u64 = 0x2545_F491_4F6C_DD1D;

/// Effects whose parameter `8xx` may vary: the ones read on every tick, where
/// a changed parameter alters the sound rather than the song structure. A
/// randomized position jump or pattern break would scramble playback, and a
/// randomized program change would pick an unrelated instrument.
fn is_continuous(effect: FxCmd) -> bool {
    matches!(
        effect,
        FxCmd::Arp | FxCmd::PitchUp | FxCmd::PitchDown | FxCmd::Vibrato | FxCmd::VolSlide
    )
}

impl TrackerEngine {
    /// xorshift64*. Small, seedable, and good enough to pick notes with.
    fn next_rand(&mut self) -> u64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// True with probability `chance`/255. 0 never fires, 255 always does.
    fn roll(&mut self, chance: u8) -> bool {
        (self.next_rand() % 255) < chance as u64
    }

    /// A value in `-range..=range`.
    fn rand_offset(&mut self, range: u8) -> i32 {
        let span = 2 * range as u64 + 1;
        (self.next_rand() % span) as i32 - range as i32
    }

    /// Resolve `RND` to the last continuous effect written in the same
    /// column, with its parameter varied by up to `xx` in either direction.
    /// Any other effect passes through. `RND` in a column that has run no
    /// continuous effect does nothing.
    fn resolve_randomize(&mut self, ch: usize, col: usize, fx: Option<Fx>) -> Option<Fx> {
        let Some(Fx {
            cmd: FxCmd::Random,
            param: range,
        }) = fx
        else {
            return fx;
        };
        let last = self.channel_states[ch].last_continuous[col]?;
        let varied = (last.param as i32 + self.rand_offset(range)).clamp(0, 255) as u8;
        Some(Fx::new(last.cmd, varied))
    }

    /// Create a new engine.
    pub fn new(song: &Song, wrap_at_end: bool) -> Self {
        let mut engine = Self {
            row: 0,
            order: 0,
            sounding_row: 0,
            sounding_order: 0,
            generation: 0,
            tick: 0,
            speed: song.speed,
            bpm: song.bpm as f64,
            repeat_count: 0,
            rows_played: 0,
            channel_states: vec![ChannelState::default(); song.channels],
            channel_info: vec![ChannelInfo::default(); song.channels],
            events: Vec::with_capacity(64),
            wrap_at_end,
            finished: false,
            rng: RNG_SEED,
        };
        engine.skip_zero_repeats_forward(song);
        engine
    }

    /// Reset engine to the given starting position.
    pub fn reset(&mut self, song: &Song, start_order: usize, start_row: usize) {
        self.row = start_row;
        self.order = start_order;
        self.sounding_row = start_row;
        self.sounding_order = start_order;
        self.generation = 0;
        self.tick = 0;
        self.speed = song.speed;
        self.bpm = song.bpm as f64;
        self.repeat_count = 0;
        self.finished = false;
        self.rng = RNG_SEED;
        self.channel_states = vec![ChannelState::default(); song.channels];
        self.events.clear();
        self.skip_zero_repeats_forward(song);
        self.rows_played = song.first_pass_row(self.order) + self.row as u64;
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

    /// Seconds the current tick lasts, with the song's groove applied.
    pub fn seconds_per_tick(&self, song: &Song) -> f64 {
        let base_tps = (self.bpm * MIDI_CLOCKS_PER_BEAT) / 60.0;
        song.grooved(1.0 / base_tps, self.sounding_row)
    }

    /// How much the groove stretches the current tick. The groove belongs
    /// to the row being played, not to the write pointer, which has
    /// already moved on to the next row.
    pub fn groove_factor(&self, song: &Song) -> f64 {
        song.groove_factor(self.sounding_row)
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
        cell_instrument.or_else(|| self.channel_info.get(ch)?.default_instrument)
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
        let order_len = song.order_len();
        if self.order >= order_len {
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
        let len = song.order_len();
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
        let order_len = song.order_len();
        if self.order >= order_len {
            self.finished = true;
            return;
        }
        let channels = song.channels;

        // Ensure channel_states has enough entries
        while self.channel_states.len() < channels {
            self.channel_states.push(ChannelState::default());
        }

        self.sounding_row = self.row;
        self.sounding_order = self.order;

        // Emit RowAdvanced
        self.emit(TrackerEvent::RowAdvanced {
            order: self.order,
            row: self.row,
            pattern: self.order,
        });

        // Collect this row's cells across all channels
        type CellTuple = (
            Option<Note>,
            Option<u8>,
            [Option<Fx>; FX_COLUMNS],
            Option<u8>,
        );
        let played = self.rows_played;
        self.rows_played += 1;
        let cells: Vec<CellTuple> = (0..channels)
            .map(|ch| {
                let cell = &song.playback_cell(self.order, self.row, ch, played);
                let inst = self.resolve_instrument(ch, cell.instrument);
                (cell.note, cell.volume, cell.fx, inst)
            })
            .collect();

        // Scan for pattern-level effects. Columns are read left to right, so
        // the right-hand one wins when both set the same thing.
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        for (_, _, fx, _) in &cells {
            for Fx { cmd, param } in fx.iter().flatten().copied() {
                match cmd {
                    FxCmd::Jump => jump_order = Some(param as usize),
                    FxCmd::Break => break_row = Some(param as usize),
                    FxCmd::Ticks if param > 0 => {
                        self.speed = param;
                        self.emit(TrackerEvent::SpeedChanged { speed: param });
                    }
                    FxCmd::Tempo if param > 0 => {
                        self.bpm = param as f64;
                        self.emit(TrackerEvent::TempoChanged { bpm: param as f64 });
                    }
                    _ => {}
                }
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
        for (ch, (note, volume, written, instrument)) in cells.into_iter().enumerate() {
            // `RND` stands in for the column's last continuous effect, so the
            // rest of the row treats it as if that effect had been written.
            let mut fx = written;
            for (col, slot) in fx.iter_mut().enumerate() {
                *slot = self.resolve_randomize(ch, col, *slot);
            }
            let find = |cmd: FxCmd| fx.iter().flatten().rev().find(|f| f.cmd == cmd);

            // `CHA`: the note plays only if every roll succeeds. The rolls
            // happen on every channel, muted or not, so muting one does not
            // shift the sequence the others draw from.
            let mut suppressed = false;
            for f in fx.iter().flatten() {
                if f.cmd == FxCmd::Chance && !self.roll(f.param) {
                    suppressed = true;
                }
            }
            let (note, volume) = if suppressed {
                (None, None)
            } else {
                (note, volume)
            };

            for col in 0..FX_COLUMNS {
                let was_random = matches!(
                    written[col],
                    Some(Fx {
                        cmd: FxCmd::Random,
                        ..
                    })
                );
                if let Some(f) = fx[col].filter(|f| !was_random && is_continuous(f.cmd)) {
                    self.channel_states[ch].last_continuous[col] = Some(f);
                }
            }

            let is_tone_porta = find(FxCmd::PitchSlide).is_some();
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
                self.channel_states[ch].fx = fx;
                continue;
            }

            // Clear previous delayed note
            self.channel_states[ch].delayed_note = None;

            let delay = find(FxCmd::Delay).map_or(0, |f| f.param);
            let is_note_delay = delay > 0;
            let sample_offset = find(FxCmd::Offset).map_or(0, |f| f.param);
            // `TBLxx`, or the table numbered as the note's instrument.
            let tbl = find(FxCmd::Table).map(|f| f.param);
            let table = tbl.or(instrument);

            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else if is_note_delay {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            self.channel_states[ch].delayed_note =
                                Some((midi_note, scaled_vel, false, sample_offset));
                            self.channel_states[ch].delay_tick = delay;
                            self.channel_states[ch].delayed_table = table;
                        } else {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            // Reset pitch bend on new note
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = [0.0; FX_COLUMNS + 1];
                            self.emit(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                            self.emit(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: scaled_vel,
                                instrument,
                                sample_offset,
                            });
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].active_instrument = instrument;
                            self.channel_states[ch].volume = vel; // store unscaled
                            self.start_table(ch, table, song);
                        }
                    }
                }
                Some(Note::Off) => {
                    if is_note_delay {
                        self.channel_states[ch].delayed_note = Some((0, 0, true, 0));
                        self.channel_states[ch].delay_tick = delay;
                    } else {
                        self.emit(TrackerEvent::NoteOff { channel: ch });
                        self.channel_states[ch].note = None;
                        self.channel_states[ch].table = None;
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
                    // `TBL` with no note restarts on the sounding one.
                    if tbl.is_some() && self.channel_states[ch].note.is_some() {
                        self.start_table(ch, tbl, song);
                    }
                }
            }

            // Store effect state
            self.channel_states[ch].fx = fx;

            // Tick-0 immediate effects, from every column
            for Fx { cmd, param } in fx.iter().flatten().copied() {
                match cmd {
                    FxCmd::Cc(controller) => self.emit(TrackerEvent::MidiCC {
                        channel: ch,
                        controller,
                        value: param,
                    }),
                    FxCmd::Program => self.emit(TrackerEvent::ProgramChange {
                        channel: ch,
                        program: param,
                    }),
                    _ => {}
                }
            }

            // The table's row on this tick. Continuous effects wait for
            // tick 1, as in the effect columns; its pitch applies now, so a
            // note does not sound one tick at the wrong pitch.
            if let Some(step) = self.advance_table(ch, song) {
                self.apply_table_row_start(ch, &step);
                if step.transpose != 0 || step.previous_transpose != 0 {
                    let offset = self.channel_states[ch].pitch_offset + step.transpose as f64;
                    self.emit(TrackerEvent::PitchBend {
                        channel: ch,
                        semitone_offset: offset,
                    });
                }
            }
        }

        // Position jump (Bxx)
        if let Some(target_order) = jump_order {
            let target = target_order.min(order_len.saturating_sub(1));
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
                let target_rows = song.rows_at(self.order);
                self.row = break_row.unwrap_or(0).min(target_rows - 1);
            }
            return;
        }

        // Pattern break (Dxx)
        if let Some(target_row) = break_row {
            self.advance_order_position(song);
            if !self.finished {
                let target_rows = song.rows_at(self.order);
                self.row = target_row.min(target_rows - 1);
            }
            return;
        }

        // Normal advance
        self.row += 1;
        let phrase_rows = song.rows_at(self.order);
        if self.row >= phrase_rows {
            self.row = 0;
            self.advance_order_position(song);
        }
    }

    /// Ticks 1..speed-1: process continuous effects.
    ///
    /// Each column keeps its own effect. Slides from both columns move the
    /// one pitch offset, arpeggio and vibrato add to it, and volume slides
    /// add, so a channel emits at most one pitch bend and one volume change
    /// per tick.
    fn process_effects(&mut self, song: &Song) {
        let channels = self.channel_states.len();
        for ch in 0..channels {
            if !self.channel_audible(ch) {
                continue;
            }

            let fx = self.channel_states[ch].fx;

            // Note delay
            let delayed = fx.iter().flatten().any(|f| f.cmd == FxCmd::Delay);
            if delayed {
                if let Some((midi_note, vel, is_off, offset)) = self.channel_states[ch].delayed_note
                {
                    if self.tick == self.channel_states[ch].delay_tick {
                        if is_off {
                            self.events.push(TrackerEvent::NoteOff { channel: ch });
                            self.channel_states[ch].note = None;
                            self.channel_states[ch].table = None;
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                        } else {
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = [0.0; FX_COLUMNS + 1];
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                            self.events.push(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: vel,
                                instrument: self.channel_states[ch].active_instrument,
                                sample_offset: offset,
                            });
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = vel;
                            let table = self.channel_states[ch].delayed_table.take();
                            self.start_table(ch, table, song);
                        }
                        self.channel_states[ch].delayed_note = None;
                    }
                }
            }

            let base_note = match self.channel_states[ch].note {
                Some(n) => n,
                None => continue,
            };

            let mut bend = false;
            let mut modulation = 0.0;
            let mut volume_changed = false;
            // The table's effect runs as a third column.
            let step = self.advance_table(ch, song);
            let mut columns = [None; FX_COLUMNS + 1];
            columns[..FX_COLUMNS].copy_from_slice(&fx);
            if let Some(step) = &step {
                self.apply_table_row_start(ch, step);
                columns[FX_COLUMNS] = step.fx;
                modulation += step.transpose as f64;
                bend |= step.transpose != 0 || step.previous_transpose != 0;
            }
            for (col, f) in columns.iter().enumerate() {
                let Some(Fx { cmd, param }) = *f else {
                    continue;
                };
                let state = &mut self.channel_states[ch];
                match cmd {
                    FxCmd::Arp if param != 0 => {
                        let x = (param >> 4) as f64;
                        let y = (param & 0x0F) as f64;
                        modulation += match self.tick % 3 {
                            0 => 0.0,
                            1 => x,
                            _ => y,
                        };
                        bend = true;
                    }
                    FxCmd::PitchUp => {
                        state.pitch_offset += param as f64 / 16.0;
                        bend = true;
                    }
                    FxCmd::PitchDown => {
                        state.pitch_offset -= param as f64 / 16.0;
                        bend = true;
                    }
                    FxCmd::PitchSlide => {
                        if let Some(target) = state.porta_target {
                            let current = base_note as f64 + state.pitch_offset;
                            let target_f = target as f64;
                            let speed = param as f64 / 16.0;
                            if current < target_f {
                                state.pitch_offset += speed.min(target_f - current);
                            } else if current > target_f {
                                state.pitch_offset -= speed.min(current - target_f);
                            }
                            bend = true;
                        }
                    }
                    FxCmd::Vibrato => {
                        let speed_v = (param >> 4) as f64;
                        let depth = (param & 0x0F) as f64;
                        let phase = &mut state.vibrato_phase[col];
                        *phase += speed_v / 64.0;
                        if *phase >= 1.0 {
                            *phase -= 1.0;
                        }
                        modulation += (*phase * std::f64::consts::TAU).sin() * depth / 16.0;
                        bend = true;
                    }
                    FxCmd::VolSlide => {
                        let delta = (param >> 4) as i16 - (param & 0x0F) as i16;
                        state.volume =
                            (state.volume as i16 + delta).clamp(0, MIDI_MAX_VALUE as i16) as u8;
                        volume_changed = true;
                    }
                    _ => {}
                }
            }
            if bend {
                self.events.push(TrackerEvent::PitchBend {
                    channel: ch,
                    semitone_offset: self.channel_states[ch].pitch_offset + modulation,
                });
            }
            if volume_changed {
                self.events.push(TrackerEvent::VolumeChange {
                    channel: ch,
                    volume: self.channel_states[ch].volume,
                });
            }
        }
    }

    /// Start table `number` from row 0 on channel `ch`, or stop the table
    /// if there is no such table.
    fn start_table(&mut self, ch: usize, number: Option<u8>, song: &Song) {
        self.channel_states[ch].table =
            number
                .filter(|n| song.tables.contains_key(n))
                .map(|number| TableState {
                    number,
                    row: 0,
                    tick: 0,
                    held: false,
                    transpose: 0,
                });
    }

    /// The current table row of channel `ch`, then move on one tick: to the
    /// next row when this one has had its ticks, looping or holding at the
    /// end.
    fn advance_table(&mut self, ch: usize, song: &Song) -> Option<TableStep> {
        let state = self.channel_states[ch].table?;
        let Some(table) = song.tables.get(&state.number) else {
            self.channel_states[ch].table = None;
            return None;
        };
        let row = table.rows[state.row.min(table.rows.len() - 1)];
        let step = TableStep {
            transpose: row.transpose,
            previous_transpose: state.transpose,
            volume: row.volume,
            fx: row.fx,
            row_started: state.tick == 0 && !state.held,
        };

        let mut next = state;
        next.transpose = row.transpose;
        if next.held {
            self.channel_states[ch].table = Some(next);
            return Some(step);
        }
        next.tick += 1;
        if next.tick >= table.ticks_per_row.max(1) {
            next.tick = 0;
            if next.row + 1 < table.rows.len() {
                next.row += 1;
            } else if let Some(from) = table.loop_from {
                next.row = from as usize;
            } else {
                next.held = true;
            }
        }
        self.channel_states[ch].table = Some(next);
        Some(step)
    }

    /// What a table row does once, as it starts: set the volume, and send
    /// its `Cnn` or `PRG`.
    fn apply_table_row_start(&mut self, ch: usize, step: &TableStep) {
        if !step.row_started {
            return;
        }
        if let Some(volume) = step.volume {
            self.channel_states[ch].volume = volume;
            self.emit(TrackerEvent::VolumeChange {
                channel: ch,
                volume,
            });
        }
        match step.fx {
            Some(Fx {
                cmd: FxCmd::Cc(controller),
                param,
            }) => self.emit(TrackerEvent::MidiCC {
                channel: ch,
                controller,
                value: param,
            }),
            Some(Fx {
                cmd: FxCmd::Program,
                param,
            }) => self.emit(TrackerEvent::ProgramChange {
                channel: ch,
                program: param,
            }),
            _ => {}
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
    use crate::tracker::{Cell, Fx, Note, NoteValue, Song};

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
        song.set_cell(
            0,
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
        song.set_cell(
            0,
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                fx: [Some(Fx::new(FxCmd::PitchUp, 0x10)), None],
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                fx: [Some(Fx::new(FxCmd::VolSlide, 0x0F)), None], // down by 15
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Ticks, 3)), None], // set speed to 3
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Tempo, 0x80)), None], // set BPM to 128
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Jump, 2)), None],
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Break, 8)), None],
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
        song.set_cell(
            0,
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Program, 5)), None],
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

    /// A song of `rows` rows on one channel, every row a C-4 with `7xx`.
    fn probability_song(rows: usize, chance: u8) -> Song {
        let mut song = Song::new(1, rows);
        song.speed = 1;
        for row in 0..rows {
            song.set_cell(
                0,
                row,
                0,
                Cell {
                    note: Some(Note::On {
                        value: NoteValue::C,
                        octave: 4,
                    }),
                    volume: Some(100),
                    fx: [Some(Fx::new(FxCmd::Chance, chance)), None],
                    ..Cell::default()
                },
            );
        }
        song
    }

    fn count_note_ons(song: &Song, rows: usize) -> usize {
        let mut engine = TrackerEngine::new(song, false);
        (0..rows)
            .map(|_| {
                engine
                    .process_tick(song)
                    .iter()
                    .filter(|e| matches!(e, TrackerEvent::NoteOn { .. }))
                    .count()
            })
            .sum()
    }

    #[test]
    fn probability_ff_always_plays_and_00_never_does() {
        let rows = 64;
        assert_eq!(count_note_ons(&probability_song(rows, 0xFF), rows), rows);
        assert_eq!(count_note_ons(&probability_song(rows, 0x00), rows), 0);
    }

    #[test]
    fn probability_80_plays_about_half_the_notes() {
        let rows = 256;
        let played = count_note_ons(&probability_song(rows, 0x80), rows);
        // Binomial(256, 0.502): a 45-83 window is far outside any plausible
        // run, so this catches a broken roll without being flaky.
        assert!(
            (90..=166).contains(&played),
            "expected roughly half of {rows}, got {played}"
        );
    }

    #[test]
    fn probability_is_reproducible_across_playbacks() {
        let song = probability_song(64, 0x80);
        assert_eq!(count_note_ons(&song, 64), count_note_ons(&song, 64));
    }

    #[test]
    fn a_suppressed_note_leaves_the_channel_alone() {
        let mut song = Song::new(1, 2);
        song.speed = 1;
        song.set_cell(
            0,
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
        song.set_cell(
            0,
            1,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::E,
                    octave: 4,
                }),
                volume: Some(40),
                fx: [Some(Fx::new(FxCmd::Chance, 0)), None],
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        let sounding = engine.channel_states[0].note;
        let volume = engine.channel_states[0].volume;
        let events = engine.process_tick(&song).to_vec();

        assert!(
            !events.iter().any(|e| matches!(
                e,
                TrackerEvent::NoteOn { .. } | TrackerEvent::NoteOff { .. }
            )),
            "a suppressed note must not cut the one already sounding"
        );
        assert_eq!(engine.channel_states[0].note, sounding);
        assert_eq!(
            engine.channel_states[0].volume, volume,
            "the volume column of a suppressed row must not apply"
        );
    }

    #[test]
    fn randomize_varies_the_last_continuous_effect() {
        let mut song = Song::new(1, 2);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Vibrato, 0x44)), None],
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Random, 0x10)), None],
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        assert_eq!(
            engine.channel_states[0].last_continuous[0],
            Some(Fx::new(FxCmd::Vibrato, 0x44))
        );

        engine.process_tick(&song);
        let fx = engine.channel_states[0].fx[0].unwrap();
        assert_eq!(fx.cmd, FxCmd::Vibrato);
        let param = fx.param as i32;
        assert!(
            (0x44 - 0x10..=0x44 + 0x10).contains(&param),
            "0x{param:02X} is outside the +/-0x10 range"
        );
        assert_eq!(
            engine.channel_states[0].last_continuous[0],
            Some(Fx::new(FxCmd::Vibrato, 0x44)),
            "the written parameter stays the reference, so 8xx does not drift"
        );
    }

    #[test]
    fn randomize_without_a_previous_effect_does_nothing() {
        let mut song = Song::new(1, 1);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Random, 0x40)), None],
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        assert_eq!(engine.channel_states[0].fx[0], None);
    }

    #[test]
    fn randomize_ignores_structural_effects() {
        let mut song = Song::new(1, 2);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Program, 9)), None],
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Random, 0x40)), None],
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        let events = engine.process_tick(&song).to_vec();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, TrackerEvent::ProgramChange { .. })),
            "8xx must not re-fire a program change"
        );
    }

    #[test]
    fn test_engine_midi_cc() {
        let mut song = make_song();
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Cc(7), 64)), None],
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
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                fx: [Some(Fx::new(FxCmd::Delay, 2)), None],
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

    /// A one-channel song whose first row holds `cell`.
    fn one_row(speed: u8, cell: Cell) -> Song {
        let mut song = Song::new(1, 4);
        song.speed = speed;
        song.set_cell(0, 0, 0, cell);
        song
    }

    fn c4() -> Option<Note> {
        Some(Note::On {
            value: NoteValue::C,
            octave: 4,
        })
    }

    fn bends(events: &[TrackerEvent]) -> Vec<f64> {
        events
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::PitchBend {
                    semitone_offset, ..
                } => Some(*semitone_offset),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn flow_commands_are_read_from_the_second_column() {
        let song = one_row(
            1,
            Cell {
                fx: [None, Some(Fx::new(FxCmd::Break, 2))],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        assert_eq!(engine.row, 2);
    }

    #[test]
    fn the_right_hand_column_wins_a_flow_conflict() {
        let song = one_row(
            6,
            Cell {
                fx: [
                    Some(Fx::new(FxCmd::Ticks, 3)),
                    Some(Fx::new(FxCmd::Ticks, 5)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        assert_eq!(engine.speed, 5);
    }

    #[test]
    fn tick_zero_commands_fire_from_both_columns() {
        let song = one_row(
            1,
            Cell {
                fx: [
                    Some(Fx::new(FxCmd::Cc(7), 0x40)),
                    Some(Fx::new(FxCmd::Cc(10), 0x20)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        let ccs: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::MidiCC {
                    controller, value, ..
                } => Some((*controller, *value)),
                _ => None,
            })
            .collect();
        assert_eq!(ccs, vec![(7, 0x40), (10, 0x20)]);
    }

    #[test]
    fn a_delayed_note_keeps_its_sample_offset() {
        let song = one_row(
            4,
            Cell {
                note: c4(),
                fx: [
                    Some(Fx::new(FxCmd::Delay, 2)),
                    Some(Fx::new(FxCmd::Offset, 0x80)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        let mut events = Vec::new();
        for _ in 0..3 {
            events.extend(engine.process_tick(&song).iter().cloned());
        }
        assert_eq!(sample_offsets(&events), vec![0x80]);
    }

    #[test]
    fn pitch_slides_from_both_columns_add() {
        let song = one_row(
            3,
            Cell {
                note: c4(),
                fx: [
                    Some(Fx::new(FxCmd::PitchUp, 0x10)),
                    Some(Fx::new(FxCmd::PitchUp, 0x20)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        let events = engine.process_tick(&song).to_vec();
        // One bend per tick, carrying both slides: 1 + 2 semitones.
        assert_eq!(bends(&events), vec![3.0]);
    }

    #[test]
    fn arpeggio_adds_to_a_slide_in_the_other_column() {
        let song = one_row(
            3,
            Cell {
                note: c4(),
                fx: [
                    Some(Fx::new(FxCmd::Arp, 0x47)),
                    Some(Fx::new(FxCmd::PitchUp, 0x10)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        // Tick 1: arpeggio step x = 4, slide so far 1.
        assert_eq!(bends(engine.process_tick(&song)), vec![5.0]);
    }

    #[test]
    fn volume_slides_from_both_columns_add() {
        let song = one_row(
            2,
            Cell {
                note: c4(),
                volume: Some(0x40),
                fx: [
                    Some(Fx::new(FxCmd::VolSlide, 0x20)),
                    Some(Fx::new(FxCmd::VolSlide, 0x30)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        let events = engine.process_tick(&song).to_vec();
        let volumes: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::VolumeChange { volume, .. } => Some(*volume),
                _ => None,
            })
            .collect();
        assert_eq!(volumes, vec![0x40 + 2 + 3]);
    }

    #[test]
    fn randomize_varies_the_effect_in_its_own_column() {
        let mut song = Song::new(1, 2);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: [
                    Some(Fx::new(FxCmd::Vibrato, 0x44)),
                    Some(Fx::new(FxCmd::VolSlide, 0x10)),
                ],
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: [None, Some(Fx::new(FxCmd::Random, 0))],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        engine.process_tick(&song);
        assert_eq!(
            engine.channel_states[0].fx,
            [None, Some(Fx::new(FxCmd::VolSlide, 0x10))]
        );
    }

    #[test]
    fn chance_in_the_second_column_can_suppress_the_note() {
        let song = one_row(
            1,
            Cell {
                note: c4(),
                fx: [None, Some(Fx::new(FxCmd::Chance, 0))],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, true);
        let events = engine.process_tick(&song).to_vec();
        assert!(!events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { .. })));
    }

    /// A phrase of `len` rows whose row `i` holds MIDI note 60 + `i`.
    fn numbered_phrase(len: usize) -> crate::tracker::Phrase {
        crate::tracker::Phrase {
            name: String::new(),
            rows: (0..len)
                .map(|i| Cell {
                    note: Note::from_midi(60 + i as u8),
                    ..Cell::default()
                })
                .collect(),
        }
    }

    fn link_all(song: &mut Song, timing: crate::tracker::PhraseTiming) {
        for p in &mut song.patterns {
            p.links = vec![Some(crate::tracker::PhraseLink {
                phrase: 0,
                transpose: 0,
                timing,
            })];
        }
    }

    /// Note numbers, as phrase rows, of the first `rows` rows played.
    fn phrase_rows_played(song: &Song, engine: &mut TrackerEngine, rows: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..rows {
            for e in engine.process_tick(song) {
                if let TrackerEvent::NoteOn { midi_note, .. } = e {
                    out.push(midi_note - 60);
                }
            }
        }
        out
    }

    #[test]
    fn a_running_phrase_keeps_its_phase_across_patterns_and_repeats() {
        let mut song = Song::new(1, 4);
        song.speed = 1;
        song.add_order_entry();
        song.order_repeats = vec![2, 1];
        song.phrases.push(numbered_phrase(3));
        link_all(&mut song, crate::tracker::PhraseTiming::Running);
        let mut engine = TrackerEngine::new(&song, true);
        assert_eq!(
            phrase_rows_played(&song, &mut engine, 12),
            vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2]
        );
    }

    #[test]
    fn an_aligned_phrase_restarts_with_each_pattern() {
        let mut song = Song::new(1, 4);
        song.speed = 1;
        song.add_order_entry();
        song.phrases.push(numbered_phrase(3));
        link_all(&mut song, crate::tracker::PhraseTiming::Aligned);
        let mut engine = TrackerEngine::new(&song, true);
        assert_eq!(
            phrase_rows_played(&song, &mut engine, 8),
            vec![0, 1, 2, 0, 0, 1, 2, 0]
        );
    }

    #[test]
    fn a_jump_does_not_reset_a_running_phrase() {
        let mut song = Song::new(1, 4);
        song.speed = 1;
        song.phrases.push(numbered_phrase(3));
        link_all(&mut song, crate::tracker::PhraseTiming::Running);
        // Phrase row 1 jumps back to the start of the song. Resetting the
        // phase on the jump would replay rows 0 1 0 1; it keeps counting.
        song.phrases[0].rows[1].fx[0] = Some(Fx::new(FxCmd::Jump, 0));
        let mut engine = TrackerEngine::new(&song, true);
        assert_eq!(
            phrase_rows_played(&song, &mut engine, 6),
            vec![0, 1, 2, 0, 1, 2]
        );
    }

    #[test]
    fn playback_started_mid_song_takes_the_first_pass_phase() {
        let mut song = Song::new(1, 4);
        song.speed = 1;
        song.add_order_entry();
        song.phrases.push(numbered_phrase(3));
        link_all(&mut song, crate::tracker::PhraseTiming::Running);
        let mut engine = TrackerEngine::new(&song, true);
        engine.reset(&song, 1, 1);
        // Order 1 row 1 is row 5 of the song: 5 % 3 = 2.
        assert_eq!(phrase_rows_played(&song, &mut engine, 2), vec![2, 0]);
        // It matches what the grid shows there.
        assert_eq!(
            song.address(1, 1, 0),
            Some(crate::tracker::CellAddr::Phrase { phrase: 0, row: 2 })
        );
    }

    fn table(transposes: &[i8], ticks: u8, loop_from: Option<u8>) -> crate::tracker::Table {
        let mut t = crate::tracker::Table::new(transposes.len());
        for (row, &tr) in t.rows.iter_mut().zip(transposes) {
            row.transpose = tr;
        }
        t.ticks_per_row = ticks;
        t.loop_from = loop_from;
        t
    }

    /// A one-channel song: a C-4 on instrument `inst` with `fx`, at speed 6.
    fn table_song(inst: u8, fx: [Option<Fx>; FX_COLUMNS]) -> Song {
        let mut song = one_row(
            6,
            Cell {
                note: c4(),
                instrument: Some(inst),
                fx,
                ..Cell::default()
            },
        );
        song.tables.insert(1, table(&[0, 4, 7], 1, Some(0)));
        song
    }

    /// Pitch bends and other events per tick, for `ticks` ticks.
    fn run_ticks(song: &Song, ticks: usize) -> Vec<Vec<TrackerEvent>> {
        let mut engine = TrackerEngine::new(song, true);
        (0..ticks)
            .map(|_| engine.process_tick(song).to_vec())
            .collect()
    }

    #[test]
    fn an_arpeggio_table_bends_exactly_as_arp_does() {
        let with_table = table_song(1, [None, None]);
        let mut with_arp = table_song(2, [Some(Fx::new(FxCmd::Arp, 0x47)), None]);
        with_arp.tables.clear();
        let bends_of = |song: &Song| -> Vec<Vec<f64>> {
            run_ticks(song, 6).iter().map(|e| bends(e)).collect()
        };
        assert_eq!(bends_of(&with_table), bends_of(&with_arp));
        assert_eq!(
            bends_of(&with_table)[1..],
            [vec![4.0], vec![7.0], vec![0.0], vec![4.0], vec![7.0]]
        );
    }

    #[test]
    fn a_table_that_does_not_loop_holds_its_last_row() {
        let mut song = table_song(1, [None, None]);
        song.tables.insert(1, table(&[0, 12], 1, None));
        let ticks = run_ticks(&song, 5);
        assert_eq!(bends(&ticks[1]), vec![12.0]);
        assert_eq!(bends(&ticks[4]), vec![12.0], "the last row holds");
    }

    #[test]
    fn rows_last_their_ticks() {
        let mut song = table_song(1, [None, None]);
        song.tables.insert(1, table(&[0, 5], 2, None));
        let ticks = run_ticks(&song, 3);
        assert!(bends(&ticks[1]).is_empty(), "row 0 still plays on tick 1");
        assert_eq!(bends(&ticks[2]), vec![5.0]);
    }

    #[test]
    fn a_rows_volume_and_cc_apply_as_it_starts() {
        let mut song = table_song(1, [None, None]);
        let mut t = table(&[0, 0], 1, None);
        t.rows[0].volume = Some(0x20);
        t.rows[0].fx = Some(Fx::new(FxCmd::Cc(74), 0x10));
        song.tables.insert(1, t);
        let ticks = run_ticks(&song, 3);
        assert!(ticks[0].contains(&TrackerEvent::VolumeChange {
            channel: 0,
            volume: 0x20
        }));
        assert!(ticks[0].contains(&TrackerEvent::MidiCC {
            channel: 0,
            controller: 74,
            value: 0x10
        }));
        assert!(
            !ticks[2]
                .iter()
                .any(|e| matches!(e, TrackerEvent::MidiCC { .. })),
            "a held row does not resend"
        );
    }

    #[test]
    fn tbl_picks_another_table_and_an_instrument_without_one_plays_plainly() {
        let mut song = table_song(1, [Some(Fx::new(FxCmd::Table, 2)), None]);
        song.tables.insert(2, table(&[0, 9], 1, None));
        assert_eq!(bends(&run_ticks(&song, 2)[1]), vec![9.0]);

        let plain = table_song(3, [None, None]);
        assert!(run_ticks(&plain, 4)[1..]
            .iter()
            .all(|e| bends(e).is_empty()));
    }

    #[test]
    fn a_note_off_stops_the_table() {
        let mut song = table_song(1, [None, None]);
        song.set_cell(
            0,
            1,
            0,
            Cell {
                note: Some(Note::Off),
                ..Cell::default()
            },
        );
        let ticks = run_ticks(&song, 9);
        // After the note-off at tick 6, only its own reset bend.
        assert_eq!(bends(&ticks[6]), vec![0.0]);
        assert!(ticks[7..].iter().all(|e| bends(e).is_empty()));
    }

    #[test]
    fn a_delayed_note_starts_its_table_when_it_sounds() {
        let song = table_song(1, [Some(Fx::new(FxCmd::Delay, 2)), None]);
        let ticks = run_ticks(&song, 5);
        assert!(bends(&ticks[1]).is_empty(), "nothing sounds yet");
        // Tick 2: the note and table row 0; tick 3: row 1.
        assert_eq!(bends(&ticks[3]), vec![4.0]);
    }

    fn sample_offsets(events: &[TrackerEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::NoteOn { sample_offset, .. } => Some(*sample_offset),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_engine_sample_offset_is_carried_by_the_note_on() {
        let c5 = Some(Note::On {
            value: NoteValue::C,
            octave: 5,
        });
        let mut song = Song::new(1, 4);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: c5,
                fx: [Some(Fx::new(FxCmd::Offset, 0x80)), None],
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                note: c5,
                ..Cell::default()
            },
        );
        // 9xx with no note has nothing to start.
        song.set_cell(
            0,
            2,
            0,
            Cell {
                fx: [Some(Fx::new(FxCmd::Offset, 0x40)), None],
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, true);
        assert_eq!(sample_offsets(engine.process_tick(&song)), vec![0x80]);
        assert_eq!(sample_offsets(engine.process_tick(&song)), vec![0]);
        assert!(sample_offsets(engine.process_tick(&song)).is_empty());
    }

    #[test]
    fn test_engine_arpeggio() {
        let mut song = Song::new(1, 4);
        song.speed = 6;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                volume: Some(100),
                fx: [Some(Fx::new(FxCmd::Arp, 0x37)), None], // x=3, y=7
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
        song.groove = Song::swing_groove(75); // heavy swing

        let mut engine_even = TrackerEngine::new(&song, true);
        engine_even.sounding_row = 0;
        let mut engine_odd = TrackerEngine::new(&song, true);
        engine_odd.sounding_row = 1;

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
    fn test_swing_follows_the_sounding_row_not_the_write_pointer() {
        // `row` advances past the row being played as soon as its notes are
        // emitted, so timing that reads `row` would apply the next row's
        // swing to the current one.
        let mut song = make_song();
        song.groove = Song::swing_groove(75);

        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song); // triggers row 0, write pointer moves to 1
        assert_eq!(engine.sounding_row, 0);
        assert_eq!(engine.row, 1, "write pointer has moved on");

        let spt_row0 = engine.seconds_per_tick(&song);
        let base = 1.0 / ((song.bpm as f64 * MIDI_CLOCKS_PER_BEAT) / 60.0);
        assert!(
            spt_row0 > base,
            "row 0 is the long half of a swung pair, got {spt_row0} vs base {base}"
        );
    }

    #[test]
    fn test_engine_order_repeats() {
        let mut song = Song::new(1, 2);
        song.order = vec![0, 0];
        song.order_repeats = vec![2, 1]; // first entry plays twice

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
}
