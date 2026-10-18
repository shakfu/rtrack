//! Deterministic tracker playback engine.
//!
//! `TrackerEngine` advances a `Song` tick-by-tick and emits `TrackerEvent`s.
//! Consumers (TUI, offline renderer, MIDI export, tests) translate events
//! to their domain (MIDI messages, synth calls, file writes, assertions).

use crate::constants::*;
use crate::modulation::{ModOffsets, ModSlot, MOD_SLOTS};
use crate::tracker::{allowed_in_table, Cell, Fx, FxCmd, Note, Song, FX_COLUMNS};

// ---------------------------------------------------------------------------
// Export length
// ---------------------------------------------------------------------------

/// How much of a song an offline export renders.
///
/// Exports run the engine with wrapping on, so a song that loops with a
/// backward `JMP` ends a pass the same way as one that runs off the end of
/// the order list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportLength {
    /// Passes through the song.
    Loops(u32),
    /// Seconds of song, looping as needed. Ends on a row boundary.
    Seconds(f64),
}

impl Default for ExportLength {
    fn default() -> Self {
        ExportLength::Loops(1)
    }
}

impl std::str::FromStr for ExportLength {
    type Err = String;

    /// `3` is three loops; `90s` is ninety seconds.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let invalid = || format!("invalid length \"{s}\": use a loop count (2) or seconds (90s)");
        if let Some(secs) = s.strip_suffix('s') {
            let secs: f64 = secs.parse().map_err(|_| invalid())?;
            if !(secs > 0.0 && secs <= MAX_EXPORT_SECONDS) {
                return Err(format!(
                    "length must be between 0 and {MAX_EXPORT_SECONDS} seconds"
                ));
            }
            return Ok(ExportLength::Seconds(secs));
        }
        match s.parse::<u32>() {
            Ok(0) => Err("loop count must be at least 1".to_string()),
            Ok(n) => Ok(ExportLength::Loops(n)),
            Err(_) => Err(invalid()),
        }
    }
}

impl std::fmt::Display for ExportLength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportLength::Loops(1) => write!(f, "1 loop"),
            ExportLength::Loops(n) => write!(f, "{n} loops"),
            ExportLength::Seconds(s) => write!(f, "{s}s"),
        }
    }
}

/// Drives a [`TrackerEngine`] for an offline export and decides when to stop.
pub struct ExportClock {
    length: ExportLength,
    elapsed: f64,
}

impl ExportClock {
    pub fn new(length: ExportLength) -> Self {
        Self {
            length,
            elapsed: 0.0,
        }
    }

    /// Whether the export is complete before the engine's next tick.
    ///
    /// Errors once the render passes [`MAX_EXPORT_SECONDS`].
    pub fn done(&self, engine: &TrackerEngine) -> anyhow::Result<bool> {
        if engine.finished {
            return Ok(true);
        }
        // Stop only between rows, so the last row keeps its full length.
        if engine.tick == 0 {
            let reached = match self.length {
                ExportLength::Loops(n) => engine.generation >= n.max(1),
                // Tolerance for the rounding summed over many ticks.
                ExportLength::Seconds(s) => self.elapsed >= s - 1e-9,
            };
            if reached {
                return Ok(true);
            }
        }
        if self.elapsed > MAX_EXPORT_SECONDS {
            anyhow::bail!(
                "export passed the {} minute limit",
                MAX_EXPORT_SECONDS / 60.0
            );
        }
        Ok(false)
    }

    /// Account for one tick of `seconds`.
    pub fn advance(&mut self, seconds: f64) {
        self.elapsed += seconds;
    }
}

// ---------------------------------------------------------------------------
// Channel state
// ---------------------------------------------------------------------------

/// A note or note-off held back by `DLY` until its tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelayedNote {
    pub midi_note: u8,
    /// Velocity to play, after the channel's volume scale.
    pub velocity: u8,
    /// Channel volume to keep, before the scale, as an immediate note keeps.
    pub volume: u8,
    /// The row's instrument, which the note plays rather than the previous one.
    pub instrument: Option<u8>,
    pub off: bool,
}

/// Per-channel state for continuous effects (arpeggio, portamento, vibrato, volume slide).
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub note: Option<u8>,
    pub volume: u8,
    pub pitch_offset: f64,
    pub porta_target: Option<u8>,
    pub vibrato_phase: f64,
    /// The current row's effects, with `RND` already resolved. Continuous
    /// ones run on every tick until the next row.
    pub fx: [Option<Fx>; FX_COLUMNS],
    pub delayed_note: Option<DelayedNote>,
    pub delay_tick: u8,
    pub active_instrument: Option<u8>,
    /// Last continuous effect written on this channel, with the parameter as
    /// written. `RND` varies that parameter rather than the last randomized
    /// one, so repeated `RND` rows deviate from the written value instead of
    /// random-walking away from it.
    pub last_continuous: Option<Fx>,
    /// Controller number `CCV` sends on, set by `CCN`.
    pub cc_number: u8,
    /// The table running with the current note, if any.
    pub table: Option<TableRun>,
    /// Pitch the current tick's vibrato and arpeggio add, kept so a table
    /// step between effect ticks can re-send the whole bend.
    pub fx_bend: f64,
    /// Engine ticks since the note started, for modulation.
    pub mod_tick: u32,
    /// The modulation offsets last sent.
    pub mods: ModOffsets,
    /// Ticks left before `KIL` ends the note.
    pub cut_in: Option<u8>,
}

/// A table running on a channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableRun {
    /// Index into `Song::tables`.
    pub table: u8,
    pub row: usize,
    /// Ticks spent on the current row.
    pub tick: u8,
    /// Started this tick, so the first step is skipped.
    pub fresh: bool,
    /// Semitones from the last row that set a transpose.
    pub transpose: i8,
    /// Volume scale from the last row that set one; `None` until then.
    pub volume: Option<u8>,
    /// The current row's effect, if it runs on every tick.
    pub fx: Option<Fx>,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            note: None,
            volume: 100,
            pitch_offset: 0.0,
            porta_target: None,
            vibrato_phase: 0.0,
            fx: [None; FX_COLUMNS],
            delayed_note: None,
            delay_tick: 0,
            active_instrument: None,
            last_continuous: None,
            cc_number: 0,
            table: None,
            fx_bend: 0.0,
            mod_tick: 0,
            mods: ModOffsets::default(),
            cut_in: None,
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
        /// `9xx`: start a sample `xx`/256 of the way into its played span.
        /// 0 for every other note, and ignored by synths and MIDI.
        sample_offset: u8,
        /// `CHD`: further notes to start with this one, already snapped to
        /// the song's scale. They end with it.
        chord: [Option<u8>; 2],
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
    /// MIDI CC (`CCV`, on the channel's `CCN` number).
    MidiCC {
        channel: usize,
        controller: u8,
        value: u8,
    },
    /// Program change (`PRG`).
    ProgramChange { channel: usize, program: u8 },
    /// Modulation of the channel's instrument filter and drive, as offsets
    /// from the instrument's own settings: octaves of cutoff, units of drive.
    InstrumentFxMod {
        channel: usize,
        cutoff_octaves: f64,
        drive: f64,
    },
    /// Speed changed (`SPD`).
    SpeedChanged { speed: u8 },
    /// Tempo changed (`TPO`, or tempo automation).
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
    /// Instrument for notes whose instrument column is empty.
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

/// The velocity for each note of an `n`-note chord sounded by rtrack's own
/// audio: `velocity / sqrt(n)`, so the chord is about as loud as one note
/// and several chords do not drive the mix into clipping. MIDI devices mix
/// their own voices and get the full velocity.
pub fn chord_velocity(velocity: u8, notes: usize) -> u8 {
    if notes <= 1 {
        return velocity;
    }
    ((velocity as f64 / (notes as f64).sqrt()).round() as u8).max(1)
}

/// Per-instrument settings that change how the engine plays a note.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstrumentInfo {
    /// Table that runs with each note.
    pub table: Option<u8>,
    /// Ignore column-alias transposes and the song scale.
    pub fixed_pitch: bool,
    pub mods: [Option<ModSlot>; MOD_SLOTS],
}

impl From<&crate::types::Instrument> for InstrumentInfo {
    fn from(i: &crate::types::Instrument) -> Self {
        Self {
            table: i.table,
            fixed_pitch: i.fixed_pitch,
            mods: i.mods,
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
    /// Ticks per row by pattern row; empty means `speed`. `SPD` clears it.
    pub groove: Vec<u8>,
    /// Ticks the sounding row takes.
    pub row_ticks: u8,

    // -- Per-order repeat tracking --
    pub repeat_count: u8,

    // -- Per-channel effect state --
    pub channel_states: Vec<ChannelState>,

    // -- Per-channel host info --
    channel_info: Vec<ChannelInfo>,

    /// What the engine needs to know of each instrument, by slot, from
    /// the host.
    instruments: Vec<InstrumentInfo>,

    // -- Event buffer --
    events: Vec<TrackerEvent>,

    // -- Whether to wrap at end of order list --
    pub wrap_at_end: bool,

    // -- Set when the song finishes (order exhausted, no wrap) --
    pub finished: bool,

    // -- PRNG for CHA and RND --
    //
    // Seeded to a fixed value on reset, so a song plays the same way every
    // time and an offline render matches what was heard in the editor.
    rng: u64,
}

/// Fixed PRNG seed. See `TrackerEngine::rng`.
const RNG_SEED: u64 = 0x2545_F491_4F6C_DD1D;

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

    /// Resolve `RND` to the channel's last continuous effect with its
    /// parameter varied by up to `xx` in either direction. Any other effect
    /// passes through. `RND` on a channel that has run no continuous effect
    /// does nothing.
    fn resolve_randomize(&mut self, ch: usize, fx: Fx) -> Option<Fx> {
        if fx.cmd != FxCmd::Random {
            return Some(fx);
        }
        let last = self.channel_states[ch].last_continuous?;
        let varied = (last.param as i32 + self.rand_offset(fx.param)).clamp(0, 255) as u8;
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
            groove: song.groove.clone(),
            row_ticks: song.speed.max(1),
            repeat_count: 0,
            channel_states: vec![ChannelState::default(); song.channels],
            channel_info: vec![ChannelInfo::default(); song.channels],
            events: Vec::with_capacity(64),
            wrap_at_end,
            finished: false,
            rng: RNG_SEED,
            instruments: Vec::new(),
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
        self.groove = song.groove.clone();
        self.row_ticks = song.speed.max(1);
        self.repeat_count = 0;
        self.finished = false;
        self.rng = RNG_SEED;
        self.channel_states = vec![ChannelState::default(); song.channels];
        self.events.clear();
        self.skip_zero_repeats_forward(song);
    }

    /// Update the channel info slice (call when mute/solo/volume changes).
    pub fn set_channel_info(&mut self, info: Vec<ChannelInfo>) {
        self.channel_info = info;
    }

    /// Set what the engine knows of each instrument, indexed by slot.
    pub fn set_instruments(&mut self, instruments: Vec<InstrumentInfo>) {
        self.instruments = instruments;
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
        if !self.finished {
            self.step_cuts();
            self.step_tables(song);
            self.step_mods(song);
        }

        self.tick += 1;
        if self.tick >= self.row_ticks.max(1) {
            self.tick = 0;
        }

        &self.events
    }

    /// Drain all events (alternative to the slice return).
    pub fn drain_events(&mut self) -> Vec<TrackerEvent> {
        std::mem::take(&mut self.events)
    }

    /// Seconds per tick at the current tempo. A groove varies the ticks
    /// per row, not the length of a tick.
    pub fn seconds_per_tick(&self, _song: &Song) -> f64 {
        60.0 / (self.bpm * MIDI_CLOCKS_PER_BEAT)
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
        let cells: Vec<(Cell, Option<u8>)> = (0..channels)
            .map(|ch| {
                let (cell, transpose) = song.played_cell(self.order, self.row, ch);
                let inst = self.resolve_instrument(ch, cell.instrument);
                // An alias transposes the note, into the song's scale if it
                // has one, unless the instrument opts out.
                let cell = if self.fixed_pitch(inst) || transpose == 0 {
                    cell
                } else {
                    let mut cell = cell.transposed(transpose);
                    if let (Some(scale), Some(Note::On { .. })) = (song.scale, cell.note) {
                        cell.note = cell
                            .note
                            .and_then(|n| n.to_midi_note())
                            .and_then(|m| Note::from_midi(scale.snap(m)));
                    }
                    cell
                };
                (cell, inst)
            })
            .collect();

        // Scan for pattern-level effects
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        for fx in cells.iter().flat_map(|(cell, _)| cell.effects()) {
            match fx.cmd {
                FxCmd::Jump => jump_order = Some(fx.param as usize),
                FxCmd::Break => break_row = Some(fx.param as usize),
                FxCmd::Speed if fx.param > 0 => {
                    self.speed = fx.param;
                    self.groove.clear();
                    self.emit(TrackerEvent::SpeedChanged { speed: fx.param });
                }
                FxCmd::Tempo if fx.param > 0 => {
                    self.bpm = fx.param as f64;
                    self.emit(TrackerEvent::TempoChanged {
                        bpm: fx.param as f64,
                    });
                }
                _ => {}
            }
        }

        // The sounding row's length, after any SPD on it.
        self.row_ticks = match self.groove.len() {
            0 => self.speed.max(1),
            n => self.groove[self.sounding_row % n].max(1),
        };

        // Check tempo automation
        if let Some(bpm) = song.tempo_at(self.order, self.row) {
            if bpm >= 1.0 {
                self.bpm = bpm;
                self.emit(TrackerEvent::TempoChanged { bpm });
            }
        }

        // Process notes and tick-0 effects
        for (ch, (cell, instrument)) in cells.into_iter().enumerate() {
            // RND stands in for the channel's last continuous effect, so the
            // rest of the row treats it as if that effect had been written.
            let mut fx = [None; FX_COLUMNS];
            for (slot, written) in cell.fx.iter().enumerate() {
                let Some(written) = *written else { continue };
                fx[slot] = self.resolve_randomize(ch, written);
                if written.cmd.is_continuous() {
                    self.channel_states[ch].last_continuous = Some(written);
                }
            }
            let find = |cmd: FxCmd| fx.iter().flatten().find(|f| f.cmd == cmd).map(|f| f.param);

            // CHA: the note plays only if the roll succeeds. The roll happens
            // on every channel, muted or not, so muting one does not shift the
            // sequence the others draw from.
            let mut suppressed = false;
            for chance in fx.iter().flatten().filter(|f| f.cmd == FxCmd::Chance) {
                if !self.roll(chance.param) {
                    suppressed = true;
                }
            }
            let (note, volume) = if suppressed {
                (None, None)
            } else {
                (cell.note, cell.volume)
            };

            let is_tone_porta = find(FxCmd::Portamento).is_some();
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

            let delay = find(FxCmd::Delay).filter(|&d| d > 0);

            match note {
                Some(Note::On { .. }) => {
                    if let Some(midi_note) = note.unwrap().to_midi_note() {
                        if is_tone_porta {
                            self.channel_states[ch].porta_target = Some(midi_note);
                        } else if let Some(delay) = delay {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            self.channel_states[ch].delayed_note = Some(DelayedNote {
                                midi_note,
                                velocity: scaled_vel,
                                volume: vel,
                                instrument,
                                off: false,
                            });
                            self.channel_states[ch].delay_tick = delay;
                        } else {
                            let vel = volume.unwrap_or(self.channel_states[ch].volume);
                            let scaled_vel = self.scale_velocity(ch, vel);
                            // Reset pitch bend on new note
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.channel_states[ch].fx_bend = 0.0;
                            self.stop_mods(ch);
                            self.channel_states[ch].volume = vel; // store unscaled
                            let table =
                                find(FxCmd::Table).or_else(|| self.instrument_table(instrument));
                            self.start_table(ch, song, table);
                            self.channel_states[ch].active_instrument = instrument;
                            self.emit(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: self.table_transpose(ch, song, Some(midi_note)),
                            });
                            let chord = self.chord_tones(ch, song, midi_note, find(FxCmd::Chord));
                            self.emit(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: scaled_vel,
                                instrument,
                                sample_offset: find(FxCmd::Offset).unwrap_or(0),
                                chord,
                            });
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].active_instrument = instrument;
                            self.enter_table_row(ch, song);
                        }
                    }
                }
                Some(Note::Off) => {
                    if let Some(delay) = delay {
                        self.channel_states[ch].delayed_note = Some(DelayedNote {
                            midi_note: 0,
                            velocity: 0,
                            volume: 0,
                            instrument: None,
                            off: true,
                        });
                        self.channel_states[ch].delay_tick = delay;
                    } else {
                        self.emit(TrackerEvent::NoteOff { channel: ch });
                        self.channel_states[ch].note = None;
                        self.channel_states[ch].pitch_offset = 0.0;
                        self.stop_table(ch);
                        self.stop_mods(ch);
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
            self.channel_states[ch].fx = fx;

            // KIL counts from this row; a new note or note-off cancels an
            // earlier one's count.
            if let Some(ticks) = find(FxCmd::Kill) {
                self.channel_states[ch].cut_in = Some(ticks);
            } else if matches!(note, Some(Note::Off))
                || (matches!(note, Some(Note::On { .. })) && !is_tone_porta)
            {
                self.channel_states[ch].cut_in = None;
            }

            // TBL with no new note restarts the table on the sounding note.
            let no_new_note = !matches!(note, Some(Note::On { .. })) || is_tone_porta;
            if let (Some(t), true) = (find(FxCmd::Table), no_new_note) {
                if self.channel_states[ch].note.is_some() {
                    self.start_table(ch, song, Some(t));
                    self.emit_bend(ch, song);
                    self.enter_table_row(ch, song);
                }
            }

            // Tick-0 immediate effects, left to right, so `CCN` then `CCV`
            // in one cell sends on the new number.
            for f in fx.iter().flatten() {
                match f.cmd {
                    FxCmd::CcNumber => self.channel_states[ch].cc_number = f.param,
                    FxCmd::CcValue => {
                        let controller = self.channel_states[ch].cc_number;
                        self.emit(TrackerEvent::MidiCC {
                            channel: ch,
                            controller,
                            value: f.param,
                        });
                    }
                    FxCmd::Program => {
                        self.emit(TrackerEvent::ProgramChange {
                            channel: ch,
                            program: f.param,
                        });
                    }
                    _ => {}
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
    /// Pitch effects in both slots combine into one bend: slides and
    /// portamento move the base offset, and vibrato and arpeggio add to it.
    fn process_effects(&mut self, song: &Song) {
        let channels = self.channel_states.len();
        for ch in 0..channels {
            if !self.channel_audible(ch) {
                continue;
            }

            // Note delay
            let fx = self.channel_states[ch].fx;
            if fx.iter().flatten().any(|f| f.cmd == FxCmd::Delay) {
                if let Some(delayed) = self.channel_states[ch].delayed_note {
                    if self.tick == self.channel_states[ch].delay_tick {
                        let midi_note = delayed.midi_note;
                        if delayed.off {
                            self.events.push(TrackerEvent::NoteOff { channel: ch });
                            self.channel_states[ch].note = None;
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.stop_table(ch);
                            self.stop_mods(ch);
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: 0.0,
                            });
                        } else {
                            self.channel_states[ch].pitch_offset = 0.0;
                            self.channel_states[ch].vibrato_phase = 0.0;
                            self.channel_states[ch].fx_bend = 0.0;
                            self.stop_mods(ch);
                            let explicit = fx
                                .iter()
                                .flatten()
                                .find(|f| f.cmd == FxCmd::Table)
                                .map(|f| f.param);
                            let instrument = delayed.instrument;
                            self.channel_states[ch].active_instrument = instrument;
                            let table = explicit.or_else(|| self.instrument_table(instrument));
                            self.start_table(ch, song, table);
                            self.events.push(TrackerEvent::PitchBend {
                                channel: ch,
                                semitone_offset: self.table_transpose(ch, song, Some(midi_note)),
                            });
                            let chord = self.chord_tones(
                                ch,
                                song,
                                midi_note,
                                fx.iter()
                                    .flatten()
                                    .find(|f| f.cmd == FxCmd::Chord)
                                    .map(|f| f.param),
                            );
                            self.events.push(TrackerEvent::NoteOn {
                                channel: ch,
                                midi_note,
                                velocity: delayed.velocity,
                                instrument,
                                sample_offset: fx
                                    .iter()
                                    .flatten()
                                    .find(|f| f.cmd == FxCmd::Offset)
                                    .map_or(0, |f| f.param),
                                chord,
                            });
                            self.channel_states[ch].note = Some(midi_note);
                            self.channel_states[ch].volume = delayed.volume;
                            self.enter_table_row(ch, song);
                        }
                        self.channel_states[ch].delayed_note = None;
                    }
                }
                continue;
            }

            let Some(base_note) = self.channel_states[ch].note else {
                continue;
            };

            let mut bend: Option<f64> = None;
            let mut added = 0.0;
            let mut arp: Option<f64> = None;
            let table_fx = self.channel_states[ch].table.and_then(|t| t.fx);
            for f in fx.iter().flatten().chain(table_fx.iter()) {
                let param = f.param;
                let state = &mut self.channel_states[ch];
                match f.cmd {
                    FxCmd::Arp if param != 0 => {
                        let x = (param >> 4) as f64;
                        let y = (param & 0x0F) as f64;
                        arp = Some(match self.tick % 3 {
                            0 => 0.0,
                            1 => x,
                            _ => y,
                        });
                        bend.get_or_insert(0.0);
                    }
                    FxCmd::SlideUp => {
                        state.pitch_offset += param as f64 / 16.0;
                        bend.get_or_insert(0.0);
                    }
                    FxCmd::SlideDown => {
                        state.pitch_offset -= param as f64 / 16.0;
                        bend.get_or_insert(0.0);
                    }
                    FxCmd::Portamento => {
                        if let Some(target) = state.porta_target {
                            let current = base_note as f64 + state.pitch_offset;
                            let target_f = target as f64;
                            let speed = param as f64 / 16.0;
                            if current < target_f {
                                state.pitch_offset += speed.min(target_f - current);
                            } else if current > target_f {
                                state.pitch_offset -= speed.min(current - target_f);
                            }
                            bend.get_or_insert(0.0);
                        }
                    }
                    FxCmd::Vibrato => {
                        let speed_v = (param >> 4) as f64;
                        let depth = (param & 0x0F) as f64;
                        state.vibrato_phase += speed_v / 64.0;
                        if state.vibrato_phase >= 1.0 {
                            state.vibrato_phase -= 1.0;
                        }
                        let sine = (state.vibrato_phase * std::f64::consts::TAU).sin();
                        added += sine * depth / 16.0;
                        bend.get_or_insert(0.0);
                    }
                    FxCmd::VolumeSlide => {
                        let up = (param >> 4) as i16;
                        let down = (param & 0x0F) as i16;
                        let new_vol =
                            (state.volume as i16 + up - down).clamp(0, MIDI_MAX_VALUE as i16) as u8;
                        state.volume = new_vol;
                        self.events.push(TrackerEvent::VolumeChange {
                            channel: ch,
                            volume: new_vol,
                        });
                    }
                    _ => {}
                }
            }
            if let Some(offset) = arp {
                added += self.quantize(ch, song, Some(base_note), offset);
            }
            self.channel_states[ch].fx_bend = added;
            if bend.is_some() {
                self.emit_bend(ch, song);
            }
        }
    }

    // -- Tables --

    fn instrument_table(&self, instrument: Option<u8>) -> Option<u8> {
        self.instruments.get(instrument? as usize)?.table
    }

    fn fixed_pitch(&self, instrument: Option<u8>) -> bool {
        instrument
            .and_then(|i| self.instruments.get(i as usize))
            .is_some_and(|i| i.fixed_pitch)
    }

    /// Semitones the channel's table adds now to a note of pitch `base`,
    /// snapped to the song's scale.
    fn table_transpose(&self, ch: usize, song: &Song, base: Option<u8>) -> f64 {
        let t = self.channel_states[ch]
            .table
            .map_or(0.0, |t| t.transpose as f64);
        self.quantize(ch, song, base, t)
    }

    /// `offset` semitones above `base`, moved so the sum lands in the song's
    /// scale. Unchanged with no scale, no base note, or a fixed-pitch
    /// instrument (`docs/dev/design-gaps.md` G10).
    fn quantize(&self, ch: usize, song: &Song, base: Option<u8>, offset: f64) -> f64 {
        let (Some(scale), Some(base)) = (song.scale, base) else {
            return offset;
        };
        if offset == 0.0 || self.fixed_pitch(self.channel_states[ch].active_instrument) {
            return offset;
        }
        let target = (base as f64 + offset)
            .round()
            .clamp(0.0, MIDI_MAX_NOTE as f64) as u8;
        scale.snap(target) as f64 - base as f64
    }

    /// Send the channel's whole bend: slides, effects and table.
    fn emit_bend(&mut self, ch: usize, song: &Song) {
        if !self.channel_audible(ch) {
            return;
        }
        let state = &self.channel_states[ch];
        let table = self.table_transpose(ch, song, state.note);
        let semitone_offset = state.pitch_offset + state.fx_bend + table + state.mods.pitch;
        self.events.push(TrackerEvent::PitchBend {
            channel: ch,
            semitone_offset,
        });
    }

    /// Start `table` from its first row, replacing any table running here.
    /// A missing table index stops the table instead.
    fn start_table(&mut self, ch: usize, song: &Song, table: Option<u8>) {
        self.stop_table(ch);
        let Some(t) = table.filter(|&t| (t as usize) < song.tables.len()) else {
            return;
        };
        self.channel_states[ch].table = Some(TableRun {
            table: t,
            row: 0,
            tick: 0,
            fresh: true,
            transpose: 0,
            volume: None,
            fx: None,
        });
        // The first row's transpose sounds with the note itself.
        if let Some(tr) = song.tables[t as usize].rows[0].transpose {
            if let Some(run) = self.channel_states[ch].table.as_mut() {
                run.transpose = tr;
            }
        }
    }

    /// Stop the channel's table, putting back a volume it had scaled.
    fn stop_table(&mut self, ch: usize) {
        if let Some(run) = self.channel_states[ch].table.take() {
            if run.volume.is_some() && self.channel_audible(ch) {
                let volume = self.channel_volume(ch);
                self.events.push(TrackerEvent::VolumeChange {
                    channel: ch,
                    volume,
                });
            }
        }
    }

    /// The channel's volume after its table's scale and modulation.
    fn channel_volume(&self, ch: usize) -> u8 {
        let state = &self.channel_states[ch];
        let table = state.table.and_then(|t| t.volume).map_or(1.0, |v| {
            v.min(MIDI_MAX_VALUE) as f64 / MIDI_MAX_VALUE as f64
        });
        (state.volume as f64 * table + state.mods.volume)
            .round()
            .clamp(0.0, MIDI_MAX_VALUE as f64) as u8
    }

    // -- Chords --

    /// The notes `CHDxy` adds to `root`: x and y semitones up, each snapped
    /// to the song's scale. A 0 digit, or a note past the MIDI range, adds
    /// nothing.
    fn chord_tones(&self, ch: usize, song: &Song, root: u8, param: Option<u8>) -> [Option<u8>; 2] {
        let Some(p) = param else {
            return [None; 2];
        };
        [p >> 4, p & 0x0F].map(|interval| {
            if interval == 0 {
                return None;
            }
            let up = self.quantize(ch, song, Some(root), interval as f64);
            let note = (root as f64 + up).round();
            (note <= MIDI_MAX_NOTE as f64).then_some(note as u8)
        })
    }

    // -- Note cut --

    /// Count down each channel's `KIL`, and end its note when it runs out.
    fn step_cuts(&mut self) {
        for ch in 0..self.channel_states.len() {
            match self.channel_states[ch].cut_in {
                Some(0) => {
                    self.channel_states[ch].cut_in = None;
                    if self.channel_states[ch].note.is_none() {
                        continue;
                    }
                    self.channel_states[ch].note = None;
                    self.channel_states[ch].pitch_offset = 0.0;
                    self.channel_states[ch].fx_bend = 0.0;
                    self.stop_table(ch);
                    self.stop_mods(ch);
                    if self.channel_audible(ch) {
                        // As a written note-off does.
                        self.events.push(TrackerEvent::NoteOff { channel: ch });
                        self.events.push(TrackerEvent::PitchBend {
                            channel: ch,
                            semitone_offset: 0.0,
                        });
                    }
                }
                Some(n) => self.channel_states[ch].cut_in = Some(n - 1),
                None => {}
            }
        }
    }

    // -- Modulation --

    /// End the channel's modulation, putting back what it changed.
    fn stop_mods(&mut self, ch: usize) {
        let old = std::mem::take(&mut self.channel_states[ch].mods);
        self.channel_states[ch].mod_tick = 0;
        if !self.channel_audible(ch) {
            return;
        }
        if old.volume != 0.0 {
            let volume = self.channel_volume(ch);
            self.events.push(TrackerEvent::VolumeChange {
                channel: ch,
                volume,
            });
        }
        if old.cutoff != 0.0 || old.drive != 0.0 {
            self.events.push(TrackerEvent::InstrumentFxMod {
                channel: ch,
                cutoff_octaves: 0.0,
                drive: 0.0,
            });
        }
    }

    /// Run each sounding note's modulation slots for this tick.
    fn step_mods(&mut self, song: &Song) {
        for ch in 0..self.channel_states.len() {
            let state = &self.channel_states[ch];
            if state.note.is_none() {
                continue;
            }
            let Some(slots) = state
                .active_instrument
                .and_then(|i| self.instruments.get(i as usize))
                .map(|i| i.mods)
                .filter(|m| m.iter().any(Option::is_some))
            else {
                continue;
            };
            let now = ModOffsets::at(&slots, state.mod_tick);
            let old = state.mods;
            self.channel_states[ch].mod_tick += 1;
            if now == old {
                continue;
            }
            self.channel_states[ch].mods = now;
            if !self.channel_audible(ch) {
                continue;
            }
            if now.pitch != old.pitch {
                self.emit_bend(ch, song);
            }
            if now.volume != old.volume {
                let volume = self.channel_volume(ch);
                self.events.push(TrackerEvent::VolumeChange {
                    channel: ch,
                    volume,
                });
            }
            if now.cutoff != old.cutoff || now.drive != old.drive {
                self.events.push(TrackerEvent::InstrumentFxMod {
                    channel: ch,
                    cutoff_octaves: now.cutoff,
                    drive: now.drive,
                });
            }
        }
    }

    /// Apply the current table row: transpose, volume, and its effect.
    /// Returns true if the transpose changed.
    fn enter_table_row(&mut self, ch: usize, song: &Song) -> bool {
        let Some(run) = self.channel_states[ch].table else {
            return false;
        };
        // The song can be replaced under a running table (load, undo).
        let Some(row) = song.tables.get(run.table as usize).map(|t| t.rows[run.row]) else {
            self.channel_states[ch].table = None;
            return false;
        };
        let audible = self.channel_audible(ch);
        let state = &mut self.channel_states[ch];
        let before = run.transpose;
        let mut run = run;
        if let Some(tr) = row.transpose {
            run.transpose = tr;
        }
        let volume_changed = row.volume.is_some();
        if let Some(v) = row.volume {
            run.volume = Some(v);
        }
        run.fx = None;
        if let Some(f) = row.fx.filter(|f| allowed_in_table(f.cmd)) {
            match f.cmd {
                FxCmd::CcNumber => state.cc_number = f.param,
                FxCmd::Kill => state.cut_in = Some(f.param),
                FxCmd::CcValue if audible => {
                    self.events.push(TrackerEvent::MidiCC {
                        channel: ch,
                        controller: state.cc_number,
                        value: f.param,
                    });
                }
                FxCmd::Program if audible => {
                    self.events.push(TrackerEvent::ProgramChange {
                        channel: ch,
                        program: f.param,
                    });
                }
                _ => run.fx = Some(f),
            }
        }
        self.channel_states[ch].table = Some(run);
        if volume_changed && audible {
            let volume = self.channel_volume(ch);
            self.events.push(TrackerEvent::VolumeChange {
                channel: ch,
                volume,
            });
        }
        run.transpose != before
    }

    /// Move every running table on by one tick.
    fn step_tables(&mut self, song: &Song) {
        for ch in 0..self.channel_states.len() {
            let Some(mut run) = self.channel_states[ch].table else {
                continue;
            };
            if run.fresh {
                run.fresh = false;
                self.channel_states[ch].table = Some(run);
                continue;
            }
            let Some(table) = song.tables.get(run.table as usize) else {
                self.channel_states[ch].table = None;
                continue;
            };
            run.tick += 1;
            if run.tick < table.tic {
                self.channel_states[ch].table = Some(run);
                continue;
            }
            run.tick = 0;
            run.row = (run.row + 1) % table.length as usize;
            self.channel_states[ch].table = Some(run);
            if self.enter_table_row(ch, song) {
                self.emit_bend(ch, song);
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
    fn export_lengths_parse_as_loops_or_seconds() {
        assert_eq!("2".parse(), Ok(ExportLength::Loops(2)));
        assert_eq!(" 90s ".parse(), Ok(ExportLength::Seconds(90.0)));
        for bad in ["0", "0s", "-1", "s", "two", "1e9s", "nans"] {
            assert!(bad.parse::<ExportLength>().is_err(), "{bad} parsed");
        }
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
                fx: Fx::only(FxCmd::SlideUp, 0x10),
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
                fx: Fx::only(FxCmd::VolumeSlide, 0x0F),
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
                fx: Fx::only(FxCmd::Speed, 3),
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: Fx::only(FxCmd::Tempo, 0x80),
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
                fx: Fx::only(FxCmd::Jump, 2),
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
                fx: Fx::only(FxCmd::Break, 8),
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
                fx: Fx::only(FxCmd::Program, 5),
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
                    fx: Fx::only(FxCmd::Chance, chance),
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
                fx: Fx::only(FxCmd::Chance, 0),
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
                fx: Fx::only(FxCmd::Vibrato, 0x44),
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: Fx::only(FxCmd::Random, 0x10),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        assert_eq!(
            engine.channel_states[0].last_continuous,
            Some(Fx::new(FxCmd::Vibrato, 0x44))
        );

        engine.process_tick(&song);
        let fx = engine.channel_states[0].fx[0].expect("RND resolved to an effect");
        assert_eq!(fx.cmd, FxCmd::Vibrato);
        let param = fx.param as i32;
        assert!(
            (0x44 - 0x10..=0x44 + 0x10).contains(&param),
            "0x{param:02X} is outside the +/-0x10 range"
        );
        assert_eq!(
            engine.channel_states[0].last_continuous,
            Some(Fx::new(FxCmd::Vibrato, 0x44)),
            "the written parameter stays the reference, so RND does not drift"
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
                fx: Fx::only(FxCmd::Random, 0x40),
                ..Cell::default()
            },
        );

        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        assert_eq!(engine.channel_states[0].fx, [None; FX_COLUMNS]);
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
                fx: Fx::only(FxCmd::Program, 9),
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: Fx::only(FxCmd::Random, 0x40),
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
                fx: [
                    Some(Fx::new(FxCmd::CcNumber, 7)),
                    Some(Fx::new(FxCmd::CcValue, 64)),
                ],
                instrument: Some(3),
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
    fn the_cc_number_persists_on_the_channel() {
        let mut song = Song::new(1, 2);
        song.speed = 1;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                fx: Fx::only(FxCmd::CcNumber, 74),
                ..Cell::default()
            },
        );
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: Fx::only(FxCmd::CcValue, 9),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.contains(&TrackerEvent::MidiCC {
            channel: 0,
            controller: 74,
            value: 9
        }));
    }

    #[test]
    fn two_pitch_effects_combine_into_one_bend() {
        // A slide and a vibrato on one row: one bend per tick, carrying both.
        let mut song = Song::new(1, 2);
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
                fx: [
                    Some(Fx::new(FxCmd::SlideUp, 0x10)),
                    Some(Fx::new(FxCmd::Vibrato, 0x8F)),
                ],
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        let events = engine.process_tick(&song).to_vec();
        let bends: Vec<f64> = events
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::PitchBend {
                    semitone_offset, ..
                } => Some(*semitone_offset),
                _ => None,
            })
            .collect();
        assert_eq!(bends.len(), 1, "{events:?}");
        let vibrato = (8.0 / 64.0 * std::f64::consts::TAU).sin() * 15.0 / 16.0;
        assert!((bends[0] - (1.0 + vibrato)).abs() < 1e-9, "{}", bends[0]);
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
                fx: Fx::only(FxCmd::Delay, 2),
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
                fx: Fx::only(FxCmd::Offset, 0x80),
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
                fx: Fx::only(FxCmd::Offset, 0x40),
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
                fx: Fx::only(FxCmd::Arp, 0x37),
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

    /// Ticks each row lasted, for the first `rows` rows played.
    fn row_lengths(song: &Song, rows: usize) -> Vec<usize> {
        let mut engine = TrackerEngine::new(song, true);
        let mut lengths = vec![0; rows];
        for _ in 0..200 {
            let before = (engine.tick, engine.sounding_row);
            engine.process_tick(song);
            if before.0 == 0 && engine.sounding_row >= rows {
                break;
            }
            lengths[engine.sounding_row.min(rows - 1)] += 1;
        }
        lengths
    }

    #[test]
    fn a_groove_gives_rows_their_own_lengths() {
        // The old 75% swing at speed 6 is a 9/3 split of each 12-tick pair.
        let mut song = make_song();
        song.groove = vec![9, 3];
        assert_eq!(row_lengths(&song, 4), vec![9, 3, 9, 3]);
        // A tick keeps one length; the groove moves ticks between rows.
        let engine = TrackerEngine::new(&song, true);
        let base = 60.0 / (song.bpm as f64 * MIDI_CLOCKS_PER_BEAT);
        assert!((engine.seconds_per_tick(&song) - base).abs() < 1e-12);
    }

    #[test]
    fn the_groove_follows_the_sounding_row_not_the_write_pointer() {
        // `row` advances past the row being played as soon as its notes are
        // emitted, so timing that reads `row` would give row 0 row 1's length.
        let mut song = make_song();
        song.groove = vec![9, 3];
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        assert_eq!((engine.sounding_row, engine.row), (0, 1));
        assert_eq!(engine.row_ticks, 9);
    }

    #[test]
    fn spd_replaces_the_groove() {
        let mut song = make_song();
        song.groove = vec![9, 3];
        song.set_cell(
            0,
            1,
            0,
            Cell {
                fx: Fx::only(FxCmd::Speed, 2),
                ..Cell::default()
            },
        );
        assert_eq!(row_lengths(&song, 4), vec![9, 2, 2, 2]);
    }

    #[test]
    fn a_scale_snaps_an_alias_transpose_but_not_written_notes() {
        let mut song = Song::new(2, 2);
        song.scale = Some(crate::theory::ScaleSetting::new(
            NoteValue::C,
            crate::theory::Scale::Major,
        ));
        let src = song.add_pattern();
        // C-4 up 6 semitones is F#, off C major: it snaps down to F.
        song.patterns[src].data[0][0] = c4([None; FX_COLUMNS], None);
        song.set_alias(
            0,
            0,
            crate::tracker::ColumnAlias {
                source: src,
                transpose: 6,
            },
        )
        .unwrap();
        // A written F#-4 on the other channel plays as written.
        song.set_cell(
            0,
            0,
            1,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::Fs,
                    octave: 4,
                }),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        let notes: Vec<u8> = engine
            .process_tick(&song)
            .iter()
            .filter_map(|e| match e {
                TrackerEvent::NoteOn { midi_note, .. } => Some(*midi_note),
                _ => None,
            })
            .collect();
        assert_eq!(notes, vec![53, 54]);
    }

    #[test]
    fn a_scale_snaps_arpeggio_steps_unless_the_instrument_is_fixed() {
        let arp_steps = |fixed: bool| {
            let mut song = Song::new(1, 2);
            song.speed = 3;
            song.scale = Some(crate::theory::ScaleSetting::new(
                NoteValue::C,
                crate::theory::Scale::Major,
            ));
            // ARP 3/7 on C: E-flat is off C major and snaps down to D.
            song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Arp, 0x37), Some(1)));
            let mut engine = TrackerEngine::new(&song, false);
            let mut instruments = vec![InstrumentInfo::default(); 256];
            instruments[1].fixed_pitch = fixed;
            engine.set_instruments(instruments);
            engine.process_tick(&song);
            let mut steps = Vec::new();
            for _ in 0..2 {
                steps.extend(bends(engine.process_tick(&song)));
            }
            steps
        };
        assert_eq!(arp_steps(false), vec![2.0, 7.0]);
        assert_eq!(arp_steps(true), vec![3.0, 7.0]);
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

    // -- Tables --

    fn table_song(rows: &[(Option<i8>, Option<u8>, Option<Fx>)], length: u8) -> Song {
        let mut song = Song::new(1, 4);
        song.speed = 4;
        let mut table = crate::tracker::Table {
            length,
            ..Default::default()
        };
        for (i, &(transpose, volume, fx)) in rows.iter().enumerate() {
            table.rows[i] = crate::tracker::TableRow {
                transpose,
                volume,
                fx,
            };
        }
        song.tables.push(table);
        song
    }

    fn c4(fx: [Option<Fx>; FX_COLUMNS], instrument: Option<u8>) -> Cell {
        Cell {
            note: Some(Note::On {
                value: NoteValue::C,
                octave: 4,
            }),
            instrument,
            fx,
            ..Cell::default()
        }
    }

    /// (instrument, velocity) of every NoteOn over `ticks` ticks.
    fn note_ons(song: &Song, engine: &mut TrackerEngine, ticks: usize) -> Vec<(Option<u8>, u8)> {
        let mut out = Vec::new();
        for _ in 0..ticks {
            for e in engine.process_tick(song) {
                if let TrackerEvent::NoteOn {
                    instrument,
                    velocity,
                    ..
                } = e
                {
                    out.push((*instrument, *velocity));
                }
            }
        }
        out
    }

    #[test]
    fn a_delayed_note_plays_its_own_instrument() {
        let mut song = Song::new(1, 4);
        song.speed = 4;
        song.set_cell(0, 0, 0, c4([None; FX_COLUMNS], Some(1)));
        song.set_cell(0, 1, 0, c4(Fx::only(FxCmd::Delay, 1), Some(2)));
        let mut engine = TrackerEngine::new(&song, false);
        let ons = note_ons(&song, &mut engine, 8);
        assert_eq!(
            ons.iter().map(|o| o.0).collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
        assert_eq!(engine.channel_states[0].active_instrument, Some(2));
    }

    #[test]
    fn a_delayed_note_scales_its_volume_once() {
        let mut song = Song::new(1, 4);
        song.speed = 4;
        let mut first = c4(Fx::only(FxCmd::Delay, 1), None);
        first.volume = Some(100);
        song.set_cell(0, 0, 0, first);
        song.set_cell(0, 1, 0, c4([None; FX_COLUMNS], None));
        let mut engine = TrackerEngine::new(&song, false);
        engine.set_channel_info(vec![ChannelInfo {
            audible: true,
            volume_scale: 0.5,
            default_instrument: None,
        }]);
        let ons = note_ons(&song, &mut engine, 8);
        assert_eq!(ons.iter().map(|o| o.1).collect::<Vec<_>>(), vec![50, 50]);
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
    fn an_instruments_table_runs_with_each_note() {
        // Two rows, +0 then +12: an octave arpeggio, one row per tick.
        let mut song = table_song(&[(Some(0), None, None), (Some(12), None, None)], 2);
        song.set_cell(0, 0, 0, c4([None; FX_COLUMNS], Some(3)));
        let mut engine = TrackerEngine::new(&song, false);
        let mut instruments = vec![InstrumentInfo::default(); 256];
        instruments[3].table = Some(0);
        engine.set_instruments(instruments);

        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.extend(bends(engine.process_tick(&song)));
        }
        assert_eq!(seen, vec![0.0, 12.0, 0.0, 12.0]);
    }

    #[test]
    fn tbl_selects_a_table_for_the_rows_note() {
        let mut song = table_song(&[(Some(7), None, None)], 1);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Table, 0), None));
        let mut engine = TrackerEngine::new(&song, false);
        let events = engine.process_tick(&song).to_vec();
        assert_eq!(
            bends(&events),
            vec![7.0],
            "the first row sounds with the note"
        );
    }

    #[test]
    fn a_running_table_whose_song_lost_its_tables_stops_instead_of_panicking() {
        let mut song = table_song(&[(Some(0), None, None), (Some(5), None, None)], 2);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Table, 0), None));
        let mut engine = TrackerEngine::new(&song, true);
        engine.process_tick(&song);
        engine.process_tick(&song);
        assert!(engine.channel_states[0].table.is_some());

        song.tables.clear();
        for _ in 0..8 {
            engine.process_tick(&song);
        }
        assert!(engine.channel_states[0].table.is_none());
    }

    #[test]
    fn a_table_steps_at_its_own_tick_rate() {
        let mut song = table_song(&[(Some(0), None, None), (Some(5), None, None)], 2);
        song.tables[0].tic = 2;
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Table, 0), None));
        let mut engine = TrackerEngine::new(&song, false);
        let per_tick: Vec<Vec<f64>> = (0..4).map(|_| bends(engine.process_tick(&song))).collect();
        assert_eq!(per_tick, vec![vec![0.0], vec![], vec![5.0], vec![]]);
    }

    #[test]
    fn a_table_volume_scales_the_channel_and_note_off_restores_it() {
        let mut song = table_song(&[(None, Some(0x40), None)], 1);
        let mut cell = c4(Fx::only(FxCmd::Table, 0), None);
        cell.volume = Some(0x7E);
        song.set_cell(0, 0, 0, cell);
        song.set_cell(
            0,
            1,
            0,
            Cell {
                note: Some(Note::Off),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.contains(&TrackerEvent::VolumeChange {
            channel: 0,
            volume: 0x3F
        }));
        for _ in 0..3 {
            engine.process_tick(&song);
        }
        let events = engine.process_tick(&song).to_vec();
        assert!(events.contains(&TrackerEvent::VolumeChange {
            channel: 0,
            volume: 0x7E
        }));
        assert_eq!(engine.channel_states[0].table, None);
    }

    #[test]
    fn a_table_effect_runs_beside_the_rows_effects() {
        // The table slides while the pattern's vibrato wobbles; one bend.
        let mut song = table_song(&[(None, None, Some(Fx::new(FxCmd::SlideUp, 0x10)))], 1);
        song.set_cell(
            0,
            0,
            0,
            c4(
                [
                    Some(Fx::new(FxCmd::Table, 0)),
                    Some(Fx::new(FxCmd::Vibrato, 0x8F)),
                ],
                None,
            ),
        );
        let mut engine = TrackerEngine::new(&song, false);
        engine.process_tick(&song);
        let b = bends(engine.process_tick(&song));
        let vibrato = (8.0 / 64.0 * std::f64::consts::TAU).sin() * 15.0 / 16.0;
        assert_eq!(b.len(), 1, "{b:?}");
        assert!((b[0] - (1.0 + vibrato)).abs() < 1e-9, "{}", b[0]);
    }

    #[test]
    fn structural_effects_do_not_run_from_a_table() {
        let mut song = table_song(&[(None, None, Some(Fx::new(FxCmd::Jump, 0)))], 1);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Table, 0), None));
        let mut engine = TrackerEngine::new(&song, false);
        for _ in 0..4 {
            engine.process_tick(&song);
        }
        assert_eq!((engine.order, engine.row), (0, 1), "no jump happened");
    }

    #[test]
    fn a_missing_table_is_ignored() {
        let mut song = Song::new(1, 2);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Table, 9), None));
        let mut engine = TrackerEngine::new(&song, false);
        let events = engine.process_tick(&song).to_vec();
        assert!(events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { .. })));
        assert_eq!(engine.channel_states[0].table, None);
    }

    #[test]
    fn an_aliased_column_plays_its_source_transposed_and_looped() {
        let mut song = Song::new(1, 4);
        song.speed = 1;
        let src = song.add_pattern();
        song.patterns[src].resize_rows(2);
        song.patterns[src].data[0][0] = c4([None; FX_COLUMNS], None);
        song.set_alias(
            0,
            0,
            crate::tracker::ColumnAlias {
                source: src,
                transpose: 7,
            },
        )
        .unwrap();
        let mut engine = TrackerEngine::new(&song, false);
        let mut notes = Vec::new();
        for _ in 0..4 {
            for e in engine.process_tick(&song) {
                if let TrackerEvent::NoteOn { midi_note, .. } = e {
                    notes.push(*midi_note);
                }
            }
        }
        assert_eq!(notes, vec![55, 55], "C-4 + 7 on rows 0 and 2");
    }

    #[test]
    fn a_fixed_pitch_instrument_ignores_an_alias_transpose() {
        let mut song = Song::new(1, 2);
        let src = song.add_pattern();
        song.patterns[src].data[0][0] = c4([None; FX_COLUMNS], Some(1));
        song.set_alias(
            0,
            0,
            crate::tracker::ColumnAlias {
                source: src,
                transpose: 7,
            },
        )
        .unwrap();
        let mut engine = TrackerEngine::new(&song, false);
        let mut instruments = vec![InstrumentInfo::default(); 256];
        instruments[1].fixed_pitch = true;
        engine.set_instruments(instruments);
        let events = engine.process_tick(&song).to_vec();
        assert!(events
            .iter()
            .any(|e| matches!(e, TrackerEvent::NoteOn { midi_note: 48, .. })));
    }

    // -- Modulation --

    fn modulated(slots: [Option<ModSlot>; MOD_SLOTS]) -> (Song, TrackerEngine) {
        let mut song = Song::new(1, 2);
        song.speed = 4;
        song.set_cell(0, 0, 0, c4([None; FX_COLUMNS], Some(2)));
        song.set_cell(
            0,
            1,
            0,
            Cell {
                note: Some(Note::Off),
                ..Cell::default()
            },
        );
        let mut engine = TrackerEngine::new(&song, false);
        let mut instruments = vec![InstrumentInfo::default(); 256];
        instruments[2].mods = slots;
        engine.set_instruments(instruments);
        (song, engine)
    }

    #[test]
    fn a_pitch_lfo_bends_each_tick_from_the_notes_start() {
        use crate::modulation::{LfoShape, ModDest, ModSource};
        let (song, mut engine) = modulated([
            Some(ModSlot {
                source: ModSource::Lfo {
                    shape: LfoShape::Square,
                    period: 2,
                },
                dest: ModDest::Pitch,
                amount: 16,
            }),
            None,
        ]);
        let per_tick: Vec<Vec<f64>> = (0..3).map(|_| bends(engine.process_tick(&song))).collect();
        // Tick 0: the note's reset bend, then +2 semitones; then -2, +2.
        assert_eq!(per_tick, vec![vec![0.0, 2.0], vec![-2.0], vec![2.0]]);
    }

    #[test]
    fn a_volume_envelope_shapes_the_note_and_note_off_restores_it() {
        use crate::modulation::{ModDest, ModSource};
        let (song, mut engine) = modulated([
            Some(ModSlot {
                source: ModSource::Envelope {
                    attack: 0,
                    hold: 1,
                    decay: 2,
                },
                dest: ModDest::Volume,
                amount: -40,
            }),
            None,
        ]);
        let mut volumes = Vec::new();
        for _ in 0..5 {
            for e in engine.process_tick(&song) {
                if let TrackerEvent::VolumeChange { volume, .. } = e {
                    volumes.push(*volume);
                }
            }
        }
        // 100 - 40, held, then back up over the decay; note-off restores.
        assert_eq!(volumes, vec![60, 80, 100]);
    }

    #[test]
    fn a_cutoff_mod_reports_octaves_for_the_host() {
        use crate::modulation::{ModDest, ModSource};
        let (song, mut engine) = modulated([
            Some(ModSlot {
                source: ModSource::Envelope {
                    attack: 0,
                    hold: 9,
                    decay: 0,
                },
                dest: ModDest::Cutoff,
                amount: -32,
            }),
            None,
        ]);
        let events = engine.process_tick(&song).to_vec();
        assert!(events.contains(&TrackerEvent::InstrumentFxMod {
            channel: 0,
            cutoff_octaves: -2.0,
            drive: 0.0
        }));
        for _ in 0..3 {
            engine.process_tick(&song);
        }
        let events = engine.process_tick(&song).to_vec();
        assert!(
            events.contains(&TrackerEvent::InstrumentFxMod {
                channel: 0,
                cutoff_octaves: 0.0,
                drive: 0.0
            }),
            "note-off puts the filter back: {events:?}"
        );
    }

    // -- Note cut --

    /// The tick of each NoteOff in the first `ticks` ticks.
    fn note_off_ticks(song: &Song, ticks: usize) -> Vec<usize> {
        let mut engine = TrackerEngine::new(song, false);
        (0..ticks)
            .filter(|_| {
                engine
                    .process_tick(song)
                    .iter()
                    .any(|e| matches!(e, TrackerEvent::NoteOff { .. }))
            })
            .collect()
    }

    fn cut_song(rows: usize) -> Song {
        let mut song = Song::new(1, rows);
        song.speed = 6;
        song
    }

    #[test]
    fn kil_ends_the_note_after_its_ticks() {
        let mut song = cut_song(4);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Kill, 3), None));
        assert_eq!(note_off_ticks(&song, 24), vec![3]);
    }

    #[test]
    fn kil_counts_on_into_later_rows() {
        let mut song = cut_song(4);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Kill, 0x0C), None));
        assert_eq!(note_off_ticks(&song, 24), vec![12], "two rows at speed 6");
    }

    #[test]
    fn kil00_cuts_on_the_notes_own_tick() {
        let mut song = cut_song(4);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Kill, 0), None));
        let mut engine = TrackerEngine::new(&song, false);
        let events = engine.process_tick(&song).to_vec();
        let on = events
            .iter()
            .position(|e| matches!(e, TrackerEvent::NoteOn { .. }));
        let off = events
            .iter()
            .position(|e| matches!(e, TrackerEvent::NoteOff { .. }));
        assert!(on < off && off.is_some(), "{events:?}");
    }

    #[test]
    fn a_new_note_cancels_a_pending_cut() {
        let mut song = cut_song(4);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Kill, 0x0C), None));
        song.set_cell(0, 1, 0, c4([None; FX_COLUMNS], None));
        assert_eq!(note_off_ticks(&song, 24), Vec::<usize>::new());
    }

    // -- Chords --

    fn chord_of(song: &Song) -> [Option<u8>; 2] {
        let mut engine = TrackerEngine::new(song, false);
        engine
            .process_tick(song)
            .iter()
            .find_map(|e| match e {
                TrackerEvent::NoteOn { chord, .. } => Some(*chord),
                _ => None,
            })
            .expect("a note played")
    }

    #[test]
    fn chd_adds_notes_above_the_root() {
        let mut song = Song::new(1, 1);
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Chord, 0x37), None));
        assert_eq!(chord_of(&song), [Some(51), Some(55)], "C minor on C-4");
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Chord, 0x07), None));
        assert_eq!(chord_of(&song), [None, Some(55)], "a 0 digit adds nothing");
        song.set_cell(0, 0, 0, c4([None; FX_COLUMNS], None));
        assert_eq!(chord_of(&song), [None, None]);
    }

    #[test]
    fn chord_tones_snap_to_the_scale() {
        let mut song = Song::new(1, 1);
        song.scale = Some(crate::theory::ScaleSetting::new(
            NoteValue::C,
            crate::theory::Scale::Major,
        ));
        // +3 is E-flat, off C major: it snaps down to D.
        song.set_cell(0, 0, 0, c4(Fx::only(FxCmd::Chord, 0x37), None));
        assert_eq!(chord_of(&song), [Some(50), Some(55)]);
    }

    #[test]
    fn a_chord_shares_out_the_level_of_one_note() {
        assert_eq!(chord_velocity(100, 1), 100);
        assert_eq!(chord_velocity(100, 3), 58);
        assert_eq!(chord_velocity(1, 3), 1, "never silent");
    }
}
