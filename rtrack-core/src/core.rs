//! Headless tracker core: owns song, engine, audio, MIDI, and channel state.
//! Both TUI and GUI frontends wrap this core with their own input/UI logic.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::AudioEngine;
use crate::constants::*;
use crate::engine::{TrackerEngine, TrackerEvent};
use crate::link::LinkEngine;
use crate::midi::{MidiEngine, MidiInputEngine};
use crate::sample::playback::NewNoteAction;
use crate::sample::{Sample, SampleBank};
use crate::tracker::{
    InstrumentDef, InstrumentEntry, Note, NoteValue, SampleRef, SampleRefEntry, Song, SongFile,
};

use crate::error::{Error, Result};
use crate::types::{
    autosave_path_for, default_channel_configs, make_relative, resolve_relative, ChannelConfig,
    ChannelType, ClockMode, Instrument, LearnableParam, MidiCcMapping, PlaybackTiming,
    ScheduledPosition, AUTOSAVE_INTERVAL_SECS,
};

/// What a load produced, beyond the song itself.
///
/// A load can succeed while still having something to say -- samples that
/// could not be found, structural damage that was repaired, a file from a
/// newer version. The caller decides how much of that to show and how to
/// word it.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    /// The file that was loaded.
    pub path: PathBuf,
    /// Structural problems `Song::repair` corrected, one description each.
    pub repairs: Vec<String>,
    /// Samples referenced by the song that could not be loaded, as
    /// (name, reason) pairs.
    pub missing_samples: Vec<(String, String)>,
    /// The file declares a format version this build does not know.
    pub from_newer_version: bool,
}

impl LoadReport {
    /// True if the load was completely clean.
    pub fn is_clean(&self) -> bool {
        self.repairs.is_empty() && self.missing_samples.is_empty() && !self.from_newer_version
    }
}

/// A note sounding because the user touched the keyboard, not because the
/// sequencer played it.
#[derive(Debug, Clone, Copy)]
pub struct PreviewNote {
    /// MIDI channel the note was sent on.
    pub channel: u8,
    pub note: u8,
    pub started: Instant,
    /// Whether the note has to be stopped explicitly.
    ///
    /// A one-shot sample ends by itself, so cutting it off after a fixed
    /// timeout would truncate it -- an amen-break slice runs to ~340ms,
    /// well past the timeout. Sustaining sources (the synth, external MIDI,
    /// looping samples) do need stopping.
    pub needs_note_off: bool,
}

/// Headless tracker core. Owns all non-UI state: song data, playback engine,
/// audio/MIDI I/O, channel configuration, instruments, and samples.
/// The sample side of the editor, captured for undo.
///
/// Holds the whole bank and instrument table rather than a diff: slicing
/// rewrites a run of slots and renames their instruments, so there is little
/// to be saved by recording which, and a snapshot cannot drift out of step
/// with what it describes.
#[derive(Clone)]
pub struct SampleSnapshot {
    bank: Arc<SampleBank>,
    instruments: Vec<Instrument>,
}

pub struct TrackerCore {
    // -- Song & engine --
    pub song: Song,
    pub engine: TrackerEngine,

    // -- I/O --
    pub midi: MidiEngine,
    pub midi_input: MidiInputEngine,
    pub link: LinkEngine,
    pub audio: Option<AudioEngine>,
    pub sample_bank: Arc<SampleBank>,

    // -- Playback --
    pub playing: bool,
    pub recording: bool,
    pub timing: PlaybackTiming,
    pub clock_mode: ClockMode,

    // -- Channel / instrument config --
    pub channels: Vec<ChannelConfig>,
    pub instruments: Vec<Instrument>,
    pub send_bus_params: Vec<crate::audio::effects::SendBusParams>,
    pub solo_channel: Option<usize>,

    // -- File state --
    pub file_path: Option<PathBuf>,
    pub dirty: bool,

    // -- MIDI learn --
    pub midi_cc_mappings: Vec<MidiCcMapping>,
    pub midi_learn_pending: Option<(usize, LearnableParam)>,

    // -- Preview --
    pub preview_note: Option<PreviewNote>,

    /// Audio frame the tick currently being dispatched should sound at.
    /// `None` for anything triggered outside the sequencer (note preview,
    /// live MIDI input), which is applied as soon as the audio thread
    /// sees it.
    scheduled_frame: Option<u64>,
}

impl TrackerCore {
    /// Create a new headless tracker core with default state (4 channels, 64 rows).
    pub fn new() -> Self {
        Self::with_song_size(4, 64)
    }

    /// Create a new headless tracker core with the given number of channels and rows per pattern.
    pub fn with_song_size(channels: usize, rows: usize) -> Self {
        let mut midi = MidiEngine::new();
        if midi.create_virtual_port().is_err() {
            let _ = midi.connect_first_available();
        }

        let mut midi_input = MidiInputEngine::new();
        let _ = midi_input.create_virtual_port();

        let song = Song::new(channels, rows);
        let link = LinkEngine::new(song.bpm as f64);
        let engine = TrackerEngine::new(&song, true);

        Self {
            song,
            engine,
            midi,
            midi_input,
            link,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            playing: false,
            recording: false,
            timing: PlaybackTiming::new(),
            clock_mode: ClockMode::Internal,
            channels: default_channel_configs(channels),
            instruments: (0..MAX_INSTRUMENTS)
                .map(|_| Instrument::default())
                .collect(),
            send_bus_params: (0..crate::audio::effects::MAX_SEND_BUSES)
                .map(|_| crate::audio::effects::SendBusParams::default())
                .collect(),
            solo_channel: None,
            file_path: None,
            dirty: false,
            midi_cc_mappings: Vec::new(),
            midi_learn_pending: None,
            preview_note: None,
            scheduled_frame: None,
        }
    }
}

impl Default for TrackerCore {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for `TrackerCore` that allows skipping hardware initialization
/// (MIDI ports, Ableton Link) for offline, test, or headless use.
pub struct TrackerCoreBuilder {
    channels: usize,
    rows: usize,
    headless: bool,
    midi: Option<MidiEngine>,
    midi_input: Option<MidiInputEngine>,
    link: Option<LinkEngine>,
}

impl TrackerCoreBuilder {
    pub fn new() -> Self {
        Self {
            channels: 4,
            rows: 64,
            headless: false,
            midi: None,
            midi_input: None,
            link: None,
        }
    }

    /// Set the number of channels and rows per pattern.
    pub fn song_size(mut self, channels: usize, rows: usize) -> Self {
        self.channels = channels;
        self.rows = rows;
        self
    }

    /// Skip all hardware initialization (MIDI ports, Ableton Link).
    /// Creates disconnected engines using their Default impls.
    pub fn headless(mut self) -> Self {
        self.headless = true;
        self
    }

    /// Inject a pre-configured MIDI output engine.
    pub fn midi(mut self, midi: MidiEngine) -> Self {
        self.midi = Some(midi);
        self
    }

    /// Inject a pre-configured MIDI input engine.
    pub fn midi_input(mut self, midi_input: MidiInputEngine) -> Self {
        self.midi_input = Some(midi_input);
        self
    }

    /// Inject a pre-configured Link engine.
    pub fn link(mut self, link: LinkEngine) -> Self {
        self.link = Some(link);
        self
    }

    pub fn build(self) -> TrackerCore {
        let song = Song::new(self.channels, self.rows);

        let midi = self.midi.unwrap_or_else(|| {
            if self.headless {
                MidiEngine::default()
            } else {
                let mut m = MidiEngine::new();
                if m.create_virtual_port().is_err() {
                    let _ = m.connect_first_available();
                }
                m
            }
        });

        let midi_input = self.midi_input.unwrap_or_else(|| {
            if self.headless {
                MidiInputEngine::default()
            } else {
                let mut m = MidiInputEngine::new();
                let _ = m.create_virtual_port();
                m
            }
        });

        let link = self.link.unwrap_or_else(|| {
            if self.headless {
                // LinkEngine has no Default -- still construct it, but it won't
                // connect to peers unless enable() is called.
                LinkEngine::new(song.bpm as f64)
            } else {
                LinkEngine::new(song.bpm as f64)
            }
        });

        let engine = TrackerEngine::new(&song, true);

        TrackerCore {
            song,
            engine,
            midi,
            midi_input,
            link,
            audio: None,
            sample_bank: Arc::new(SampleBank::new()),
            playing: false,
            recording: false,
            timing: PlaybackTiming::new(),
            clock_mode: ClockMode::Internal,
            channels: default_channel_configs(self.channels),
            instruments: (0..MAX_INSTRUMENTS)
                .map(|_| Instrument::default())
                .collect(),
            send_bus_params: (0..crate::audio::effects::MAX_SEND_BUSES)
                .map(|_| crate::audio::effects::SendBusParams::default())
                .collect(),
            solo_channel: None,
            file_path: None,
            dirty: false,
            midi_cc_mappings: Vec::new(),
            midi_learn_pending: None,
            preview_note: None,
            scheduled_frame: None,
        }
    }
}

impl Default for TrackerCoreBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackerCore {
    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Returns true if playback is currently running.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Returns true if a MIDI output port is connected.
    pub fn midi_connected(&self) -> bool {
        self.midi.is_connected()
    }

    /// Returns the display name of the connected MIDI port, or "--" if disconnected.
    pub fn midi_port_display_name(&self) -> &str {
        self.midi.port_name.as_deref().unwrap_or("--")
    }

    /// Returns true if the audio engine is initialized.
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Returns true if a SoundFont (.sf2) is loaded in the audio engine.
    pub fn has_sf2(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.has_sf2())
    }

    /// Counts of audio commands dropped or applied ahead of their frame, and
    /// of callbacks too large for the render buffers. `None` when there is no
    /// audio engine. All three are silent under load by necessity, so this is
    /// the only way to tell a scheduling problem from a timing illusion.
    pub fn audio_stats(&self) -> Option<crate::audio::AudioStats> {
        self.audio.as_ref().map(|a| a.stats())
    }

    /// Returns true if per-channel audio effects are enabled.
    pub fn audio_effects_enabled(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.effects_enabled())
    }

    /// Toggle per-channel audio effects on/off. Returns the new state.
    #[allow(dead_code)]
    pub fn toggle_audio_effects(&mut self) -> bool {
        self.audio.as_mut().is_some_and(|a| a.toggle_effects())
    }

    /// Map a tracker channel index to its MIDI channel (0-15).
    pub fn midi_channel_for(&self, tracker_channel: usize) -> u8 {
        let ch = self
            .channels
            .get(tracker_channel)
            .map(|c| c.midi_channel)
            .unwrap_or(tracker_channel as u8);
        ch & 0x0F
    }

    /// Returns true if the channel should produce sound (respects mute and solo state).
    pub fn is_channel_audible(&self, channel: usize) -> bool {
        if let Some(solo) = self.solo_channel {
            return channel == solo;
        }
        self.channels.get(channel).is_none_or(|c| !c.muted)
    }

    /// Push every channel's effects parameters to the audio engine. Used
    /// after loading a song, when all channel state changes at once.
    pub fn push_all_channel_effects(&mut self) {
        if let Some(ref mut audio) = self.audio {
            for (ch, cfg) in self.channels.iter().enumerate() {
                if ch >= crate::audio::channel_effects::MAX_EFFECT_CHANNELS {
                    break;
                }
                audio.set_channel_effects(ch as u8, &cfg.effects_params);
            }
        }
    }

    /// Push every send bus's parameters to the audio engine.
    pub fn push_all_send_bus_params(&mut self) {
        if let Some(ref mut audio) = self.audio {
            for (bus, params) in self.send_bus_params.iter().enumerate() {
                audio.set_send_bus_params(bus as u8, params);
            }
        }
    }

    /// Collect per-channel effects parameters for all channels (used by export).
    pub fn channel_effects_params_slice(
        &self,
    ) -> Vec<crate::audio::channel_effects::ChannelEffectsParams> {
        self.channels
            .iter()
            .map(|c| c.effects_params.clone())
            .collect()
    }

    /// Compute the pitch bend value per semitone for a channel, based on its active instrument's
    /// pitch bend range setting.
    pub fn channel_pitch_bend_per_semitone(&self, ch: usize) -> f64 {
        let range = self
            .engine
            .channel_states
            .get(ch)
            .and_then(|cs| cs.active_instrument)
            .and_then(|idx| self.instruments.get(idx as usize))
            .and_then(|inst| inst.pitch_bend_range)
            .unwrap_or(DEFAULT_PITCH_BEND_RANGE);
        (PITCH_BEND_CENTER as f64) / range
    }

    /// Returns true if the instrument at the given index has a sample loaded.
    #[allow(dead_code)]
    pub fn instrument_has_sample(&self, inst: usize) -> bool {
        self.instruments
            .get(inst)
            .and_then(|i| i.sample_index)
            .and_then(|idx| self.sample_bank.get(idx))
            .is_some()
    }

    // -----------------------------------------------------------------------
    // Playback
    // -----------------------------------------------------------------------

    /// Enable or disable Ableton Link synchronization.
    pub fn toggle_link(&mut self) {
        if self.link.is_enabled() {
            self.link.disable();
        } else {
            self.link.enable();
        }
    }

    /// Toggle playback: start from the given position if stopped, or stop if playing.
    pub fn toggle_playback(&mut self, start_order: usize, start_row: usize) {
        if self.playing {
            self.stop();
        } else {
            self.play(start_order, start_row);
        }
    }

    /// Start playback from the given order and row position.
    pub fn play(&mut self, start_order: usize, start_row: usize) {
        self.playing = true;
        self.engine.reset(&self.song, start_order, start_row);
        self.sync_engine_channel_info();
        self.timing.reset();
        self.timing.last_tick = Some(Instant::now());
        // Anchor the scheduler to the device clock so the first tick sounds
        // at the next callback rather than being treated as overdue.
        if let Some(ref audio) = self.audio {
            let now_frame = audio.frame_clock();
            self.timing.next_tick_frame = now_frame;
            self.timing.last_clock_frame = now_frame;
        }
        if self.link.is_enabled() {
            self.timing.last_link_beat = self.link.beat_at_time_now();
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    /// Stop playback, reset pitch bends, and send all-notes-off.
    pub fn stop(&mut self) {
        self.playing = false;
        self.timing.last_tick = None;
        // Anything already queued ahead of the audio clock must not sound
        // after the transport has stopped.
        self.timing.scheduled_positions.clear();
        self.scheduled_frame = None;
        for ch in 0..self.engine.channel_states.len() {
            let midi_ch = self.midi_channel_for(ch);
            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
        }
        self.send_all_notes_off();
        let _ = self.midi.send_stop();

        if self.link.is_enabled() {
            self.link.request_stop();
        }
    }

    /// Poll Ableton Link for tempo changes and apply them to the song.
    pub fn sync_link(&mut self) {
        if !self.link.is_enabled() {
            return;
        }

        if let Some(new_tempo) = self.link.poll_tempo_change() {
            let new_bpm = new_tempo.round() as u16;
            if new_bpm != self.song.bpm && (32..=300).contains(&new_bpm) {
                self.song.bpm = new_bpm;
                self.engine.bpm = new_tempo;
            }
        }
    }

    /// Push current channel mute/solo/volume state into the engine.
    pub fn sync_engine_channel_info(&mut self) {
        let infos: Vec<crate::engine::ChannelInfo> = self
            .channels
            .iter()
            .enumerate()
            .map(|(i, ch)| crate::engine::ChannelInfo {
                audible: self.is_channel_audible(i),
                volume_scale: ch.volume,
                default_instrument: ch.default_instrument,
                is_synth: ch.channel_type == ChannelType::Synth,
            })
            .collect();
        self.engine.set_channel_info(infos);
    }

    /// Drive the playback tick accumulator. Returns true if at least one tick was processed.
    pub fn tick_playback(&mut self) -> bool {
        if !self.playing {
            return false;
        }

        if self.clock_mode == ClockMode::ExternalMidi {
            return false;
        }

        // Link beat-timeline mode
        if self.link.is_enabled() {
            return self.tick_playback_link();
        }

        // Audio-clock mode: schedule ticks against the device's frame counter
        // and stamp each event with the frame it should sound at. Playback
        // timing then depends on the audio device, not on how often the UI
        // thread happens to call this.
        if self.audio.is_some() {
            return self.tick_playback_audio_clock();
        }

        // Wall-clock fallback: no audio device, so there is no frame counter
        // to schedule against. Used for MIDI-only playback and headless runs.
        self.tick_playback_wall_clock()
    }

    /// Frames between two ticks at the given tick length and sample rate.
    ///
    /// Always at least one frame: a zero advance would spin the scheduler
    /// loop forever at absurd tempos.
    pub fn frames_per_tick(seconds_per_tick: f64, sample_rate: f64) -> u64 {
        (seconds_per_tick * sample_rate).round().max(1.0) as u64
    }

    /// Drive playback from the audio device's frame clock.
    fn tick_playback_audio_clock(&mut self) -> bool {
        let (now_frame, sample_rate) = match self.audio {
            Some(ref audio) => (audio.frame_clock(), audio.sample_rate()),
            None => return false,
        };
        if sample_rate <= 0.0 {
            return false;
        }

        // Elapsed time since the last call, measured in frames actually
        // consumed by the device.
        let elapsed_frames = now_frame.saturating_sub(self.timing.last_clock_frame);
        self.timing.last_clock_frame = now_frame;
        let elapsed = elapsed_frames as f64 / sample_rate;
        self.timing.playback_elapsed += elapsed;
        self.emit_midi_clock(elapsed);

        // Never schedule into the past: if the device ran on while we were
        // stalled, catch up to now rather than firing a burst of late events.
        if self.timing.next_tick_frame < now_frame {
            self.timing.next_tick_frame = now_frame;
        }

        let horizon = now_frame + (SCHEDULER_LOOKAHEAD_SECS * sample_rate) as u64;
        let mut ticked = false;
        while self.timing.next_tick_frame <= horizon {
            let frame = self.timing.next_tick_frame;
            self.scheduled_frame = Some(frame);
            let is_row_start = self.engine.tick == 0;
            self.process_tick();
            self.scheduled_frame = None;

            if is_row_start {
                // `sounding_*` names the row just triggered; `order`/`row`
                // have already moved on to the next one.
                self.record_scheduled_position(
                    frame,
                    self.engine.sounding_order,
                    self.engine.sounding_row,
                );
            }

            // Read the tick length after processing: a speed or tempo effect
            // on this tick applies from here on.
            let spt = self.engine.seconds_per_tick(&self.song);
            let advance = Self::frames_per_tick(spt, sample_rate);
            self.timing.next_tick_frame = frame + advance;
            ticked = true;
        }

        self.expire_scheduled_positions(now_frame);
        ticked
    }

    /// Drive playback from wall-clock deltas. Used when no audio device is
    /// available, where there is nothing to be sample-accurate against.
    fn tick_playback_wall_clock(&mut self) -> bool {
        let mut ticked = false;
        let now = Instant::now();
        if let Some(last) = self.timing.last_tick {
            let elapsed = now.duration_since(last).as_secs_f64();
            self.timing.tick_accumulator += elapsed;
            self.timing.playback_elapsed += elapsed;
            self.emit_midi_clock(elapsed);

            // Recompute the tick length each iteration: a speed or tempo
            // change mid-loop must take effect on the following tick.
            while self.timing.tick_accumulator >= self.engine.seconds_per_tick(&self.song) {
                self.timing.tick_accumulator -= self.engine.seconds_per_tick(&self.song);
                self.process_tick();
                ticked = true;
            }
        }
        self.timing.last_tick = Some(now);
        ticked
    }

    /// Drive playback from the Ableton Link beat timeline.
    fn tick_playback_link(&mut self) -> bool {
        let mut ticked = false;
        let beat = self.link.beat_at_time_now();
        let link_ticks = beat * MIDI_CLOCKS_PER_BEAT;
        let last_ticks = self.timing.last_link_beat * MIDI_CLOCKS_PER_BEAT;
        let delta_ticks = link_ticks - last_ticks;
        if delta_ticks > 0.0 {
            let spt = self.engine.seconds_per_tick(&self.song);
            let ticks_per_second = 1.0 / spt;
            let tracker_ticks =
                delta_ticks / (self.engine.bpm * MIDI_CLOCKS_PER_BEAT / 60.0) * ticks_per_second;
            self.timing.tick_accumulator += tracker_ticks * spt;
            self.timing.playback_elapsed +=
                delta_ticks / (self.engine.bpm * MIDI_CLOCKS_PER_BEAT / 60.0);
        }
        self.timing.last_link_beat = beat;

        while self.timing.tick_accumulator >= self.engine.seconds_per_tick(&self.song) {
            self.timing.tick_accumulator -= self.engine.seconds_per_tick(&self.song);
            self.process_tick();
            ticked = true;
        }
        ticked
    }

    /// Emit outgoing MIDI clock pulses for a span of elapsed seconds.
    fn emit_midi_clock(&mut self, elapsed: f64) {
        if !self.midi.clock_enabled {
            return;
        }
        self.timing.clock_tick_accumulator += elapsed;
        let clock_interval = 60.0 / (self.engine.bpm * MIDI_CLOCKS_PER_BEAT);
        if clock_interval <= 0.0 {
            return;
        }
        while self.timing.clock_tick_accumulator >= clock_interval {
            self.timing.clock_tick_accumulator -= clock_interval;
            let _ = self.midi.send_clock();
        }
    }

    /// Remember where the song will be when `frame` becomes audible.
    fn record_scheduled_position(&mut self, frame: u64, order: usize, row: usize) {
        self.timing
            .scheduled_positions
            .push_back(ScheduledPosition { frame, order, row });
        while self.timing.scheduled_positions.len() > MAX_SCHEDULED_POSITIONS {
            self.timing.scheduled_positions.pop_front();
        }
    }

    /// Drop positions the listener has already passed, keeping the most
    /// recent one so `playback_position` always has an answer.
    fn expire_scheduled_positions(&mut self, now_frame: u64) {
        while self.timing.scheduled_positions.len() > 1
            && self.timing.scheduled_positions[1].frame <= now_frame
        {
            self.timing.scheduled_positions.pop_front();
        }
    }

    /// The song position the listener is currently hearing.
    ///
    /// With lookahead scheduling the engine runs ahead of the audio output,
    /// so `engine.order` / `engine.row` describe notes that have been queued
    /// but are not audible yet. UI that follows playback should use this.
    pub fn playback_position(&self) -> (usize, usize) {
        if let Some(ref audio) = self.audio {
            let now_frame = audio.frame_clock();
            let audible = self
                .timing
                .scheduled_positions
                .iter()
                .rev()
                .find(|p| p.frame <= now_frame);
            if let Some(p) = audible {
                return (p.order, p.row);
            }
            if let Some(p) = self.timing.scheduled_positions.front() {
                // Nothing audible yet: report the first queued position
                // rather than a row that has not been reached.
                return (p.order, p.row);
            }
        }
        (self.engine.sounding_order, self.engine.sounding_row)
    }

    /// Process a single sub-tick: drive engine and dispatch events.
    pub fn process_tick(&mut self) {
        self.engine.process_tick(&self.song);
        let events = self.engine.drain_events();
        self.dispatch_engine_events(events);
    }

    /// Translate engine events to MIDI/audio output.
    pub fn dispatch_engine_events(&mut self, events: Vec<TrackerEvent>) {
        for event in events {
            match event {
                TrackerEvent::NoteOn {
                    channel,
                    midi_note,
                    velocity,
                    instrument,
                } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_note_on_with_instrument(midi_ch, midi_note, velocity, instrument);
                }
                TrackerEvent::NoteOff { channel } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_channel_note_off(midi_ch);
                }
                TrackerEvent::PitchBend {
                    channel,
                    semitone_offset,
                } => {
                    let midi_ch = self.midi_channel_for(channel);
                    let pb_per_semi = self.channel_pitch_bend_per_semitone(channel);
                    let bend = (semitone_offset * pb_per_semi) as i32;
                    let value =
                        (PITCH_BEND_CENTER as i32 + bend).clamp(0, PITCH_BEND_MAX as i32) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                TrackerEvent::VolumeChange { channel, volume } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_cc(midi_ch, 7, volume);
                }
                TrackerEvent::MidiCC {
                    channel,
                    controller,
                    value,
                } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_cc(midi_ch, controller, value);
                }
                TrackerEvent::ProgramChange { channel, program } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_program_change(midi_ch, program);
                }
                TrackerEvent::SpeedChanged { speed } => {
                    self.song.speed = speed;
                }
                TrackerEvent::TempoChanged { bpm } => {
                    self.song.bpm = bpm.round() as u16;
                    if self.link.is_enabled() {
                        self.link.set_tempo(bpm);
                    }
                }
                TrackerEvent::RowAdvanced { .. } | TrackerEvent::GenerationAdvanced { .. } => {}
            }
        }
    }

    // -- External MIDI clock sync --

    /// Switch between internal and external MIDI clock mode.
    pub fn toggle_clock_mode(&mut self) {
        self.clock_mode = match self.clock_mode {
            ClockMode::Internal => {
                self.timing.ext_clock_count = 0;
                ClockMode::ExternalMidi
            }
            ClockMode::ExternalMidi => ClockMode::Internal,
        };
    }

    /// Process an incoming external MIDI clock tick (advances playback when in external mode).
    pub fn handle_external_clock(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi || !self.playing {
            return;
        }

        self.timing.ext_clock_count += 1;

        let clocks_per_tick = (24u32 / self.engine.speed as u32).max(1);
        if self.timing.ext_clock_count >= clocks_per_tick {
            self.timing.ext_clock_count = 0;
            self.process_tick();

            let spt = self.engine.seconds_per_tick(&self.song);
            self.timing.playback_elapsed += spt;
        }
    }

    /// Handle an external MIDI Start message (reset and begin playback).
    pub fn handle_external_start(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi {
            return;
        }
        self.playing = true;
        self.engine.reset(&self.song, 0, 0);
        self.sync_engine_channel_info();
        self.timing.ext_clock_count = 0;
        self.timing.playback_elapsed = 0.0;
    }

    /// Handle an external MIDI Stop message (halt playback and silence notes).
    pub fn handle_external_stop(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi {
            return;
        }
        self.playing = false;
        for ch in 0..self.engine.channel_states.len() {
            let midi_ch = self.midi_channel_for(ch);
            self.send_pitch_bend(midi_ch, PITCH_BEND_CENTER);
        }
        self.send_all_notes_off();
    }

    /// Handle an external MIDI Continue message (resume playback from current position).
    pub fn handle_external_continue(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi {
            return;
        }
        self.playing = true;
        self.timing.ext_clock_count = 0;
    }

    // -----------------------------------------------------------------------
    // Sound output helpers (dispatch to MIDI + optional audio engine)
    // -----------------------------------------------------------------------

    /// Send a note-on to both MIDI output and the audio engine.
    pub fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let at = self.scheduled_frame;
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref mut audio) = self.audio {
            match at {
                Some(frame) => audio.note_on_at(frame, channel, note, velocity),
                None => audio.note_on(channel, note, velocity),
            }
        }
    }

    /// Send a note-on routed through the instrument's sound source (sample, synth params,
    /// preset patch, or default synth).
    pub fn send_note_on_with_instrument(
        &mut self,
        channel: u8,
        note: u8,
        velocity: u8,
        instrument: Option<u8>,
    ) {
        let inst_idx = instrument.unwrap_or(0) as usize;
        let inst = self.instruments.get(inst_idx);
        // Sequencer-driven notes carry the frame they should sound at;
        // anything else (preview, live MIDI) sounds immediately.
        let at = self.scheduled_frame;

        // Route 1: sample engine
        if let Some(sid) = inst.and_then(|i| i.sample_index) {
            if self.sample_bank.get(sid).is_some() {
                let _ = self.midi.note_on(channel, note, velocity);
                if let Some(ref mut audio) = self.audio {
                    match at {
                        // A note from a pattern row takes the channel over,
                        // the way a tracker channel has always worked.
                        Some(frame) => audio.sample_note_on_at(
                            frame,
                            sid,
                            note,
                            velocity,
                            channel,
                            NewNoteAction::Cut,
                        ),
                        // Previews and live MIDI are not pattern rows, so
                        // they stack and a chord stays a chord.
                        None => audio.sample_note_on(
                            sid,
                            note,
                            velocity,
                            channel,
                            NewNoteAction::Continue,
                        ),
                    }
                }
                return;
            }
        }

        // Route 2: custom synth params
        if let Some(params) = inst.and_then(|i| i.synth_params.as_ref()) {
            let params = params.clone();
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                match at {
                    Some(frame) => {
                        audio.note_on_with_params_at(frame, channel, note, velocity, &params)
                    }
                    None => audio.note_on_with_params(channel, note, velocity, &params),
                }
            }
            return;
        }

        // Route 3: instrument number maps to preset patch
        if instrument.is_some() {
            let params = crate::audio::synth::SynthParams::from_patch(inst_idx as u8);
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                match at {
                    Some(frame) => {
                        audio.note_on_with_params_at(frame, channel, note, velocity, &params)
                    }
                    None => audio.note_on_with_params(channel, note, velocity, &params),
                }
            }
            return;
        }

        // Route 4: default synth
        self.send_note_on(channel, note, velocity);
    }

    /// Send note-off for all active notes on a MIDI channel.
    pub fn send_channel_note_off(&mut self, channel: u8) {
        let at = self.scheduled_frame;
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref mut audio) = self.audio {
            match at {
                Some(frame) => {
                    audio.note_off_all_channel_at(frame, channel);
                    audio.sample_note_off_channel_at(frame, channel);
                }
                None => {
                    audio.note_off_all_channel(channel);
                    audio.sample_note_off_channel(channel);
                }
            }
        }
    }

    /// Send note-off for a specific note on a MIDI channel.
    pub fn send_note_off(&mut self, channel: u8, note: u8) {
        let _ = self.midi.note_off(channel, note);
        if let Some(ref mut audio) = self.audio {
            audio.note_off(channel, note);
            audio.sample_note_off(channel, note);
        }
    }

    /// Play a short preview note (auto-expires after a timeout).
    pub fn preview_note(&mut self, channel: u8, note: u8, velocity: u8) {
        self.preview_note_with_instrument(channel, note, velocity, None);
    }

    /// Play a short preview note routed through a specific instrument.
    pub fn preview_note_with_instrument(
        &mut self,
        channel: u8,
        note: u8,
        velocity: u8,
        instrument: Option<u8>,
    ) {
        if let Some(previous) = self.preview_note.take() {
            self.send_channel_note_off(previous.channel);
        }
        let needs_note_off = !self.instrument_is_one_shot_sample(instrument);
        if instrument.is_some() {
            self.send_note_on_with_instrument(channel, note, velocity, instrument);
        } else {
            self.send_note_on(channel, note, velocity);
        }
        self.preview_note = Some(PreviewNote {
            channel,
            note,
            started: Instant::now(),
            needs_note_off,
        });
    }

    /// Silence the preview note if its timeout has elapsed.
    pub fn expire_preview_note(&mut self) {
        if let Some(preview) = self.preview_note {
            let elapsed = preview.started.elapsed();
            if elapsed > std::time::Duration::from_millis(PREVIEW_NOTE_TIMEOUT_MS) {
                if preview.needs_note_off {
                    self.send_channel_note_off(preview.channel);
                    self.preview_note = None;
                } else if elapsed > std::time::Duration::from_millis(PREVIEW_ONE_SHOT_MAX_MS) {
                    // Stop tracking it; the voice has long since ended on its
                    // own, and holding the record would suppress the note-off
                    // for whatever is previewed next.
                    self.preview_note = None;
                }
            }
        }
        // Both frontends call this every UI frame, which makes it the natural
        // place to free whatever the audio thread has handed back.
        if let Some(ref mut audio) = self.audio {
            audio.reclaim_garbage();
        }
    }

    /// Send all-notes-off to MIDI and the audio engine (panic/silence).
    pub fn send_all_notes_off(&mut self) {
        let _ = self.midi.all_notes_off();
        if let Some(ref mut audio) = self.audio {
            audio.note_off_all();
            audio.sample_note_off_all();
        }
    }

    /// Send a MIDI Control Change message to both MIDI output and the audio engine.
    pub fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        let _ = self.midi.send_cc(channel, controller, value);
        if let Some(ref mut audio) = self.audio {
            audio.send_cc(channel, controller, value);
        }
    }

    /// Send a MIDI Program Change message to both MIDI output and the audio engine.
    pub fn send_program_change(&mut self, channel: u8, program: u8) {
        let _ = self.midi.program_change(channel, program);
        if let Some(ref mut audio) = self.audio {
            audio.program_change(channel, program);
        }
    }

    /// Send a MIDI Pitch Bend message to both MIDI output and the audio engine.
    pub fn send_pitch_bend(&mut self, channel: u8, value: u16) {
        let _ = self.midi.pitch_bend(channel, value);
        if let Some(ref mut audio) = self.audio {
            audio.pitch_bend(channel, value);
        }
    }

    // -----------------------------------------------------------------------
    // Note recording (used by frontends for step/punch-in recording)
    // -----------------------------------------------------------------------

    /// Record a note into the pattern at the given position (punch-in recording).
    /// Auto-fills instrument from the channel's default if the channel is Synth or Sample type.
    /// Returns true if the note was recorded.
    /// The instrument a note entered at this cell should sound with.
    ///
    /// Tracker instrument columns are sticky: a note with a blank instrument
    /// plays whatever the column last named. rtrack's engine does not infer
    /// that at playback time, so note entry resolves it and writes it into
    /// the cell, which keeps what you hear while editing identical to what
    /// you hear on playback.
    ///
    /// Resolution order:
    /// 1. The track's default instrument, for Synth and Sample tracks --
    ///    this is the "currently selected instrument" behaviour.
    /// 2. Whatever the cell already names, so re-entering a note over an
    ///    existing one keeps its sound.
    /// 3. The nearest instrument above it in the same channel column.
    ///
    /// Step 3 is what makes a sliced sample usable: the slices are separate
    /// instruments, the track is often a plain Midi-typed one, and without it
    /// every newly entered note fell through to the built-in synth.
    pub fn resolve_edit_instrument(&self, order: usize, row: usize, channel: usize) -> Option<u8> {
        if let Some(inst) = self.track_default_instrument(channel) {
            return Some(inst);
        }
        if let Some(inst) = self.song.cell_at(order, row, channel).instrument {
            return Some(inst);
        }
        (0..row)
            .rev()
            .find_map(|r| self.song.cell_at(order, r, channel).instrument)
    }

    /// True if this instrument plays a sample that ends by itself.
    fn instrument_is_one_shot_sample(&self, instrument: Option<u8>) -> bool {
        let Some(idx) = instrument else {
            return false;
        };
        let Some(sample_index) = self
            .instruments
            .get(idx as usize)
            .and_then(|i| i.sample_index)
        else {
            return false;
        };
        match self.sample_bank.get(sample_index) {
            Some(sample) => !sample.loop_enabled,
            None => false,
        }
    }

    /// The track's default instrument, for tracks that auto-fill one.
    fn track_default_instrument(&self, channel: usize) -> Option<u8> {
        let cfg = self.channels.get(channel)?;
        match cfg.channel_type {
            ChannelType::Synth | ChannelType::Sample => cfg.default_instrument,
            ChannelType::Midi => None,
        }
    }

    /// Preview a note as it will sound at a particular cell.
    ///
    /// Use this rather than [`TrackerCore::preview_note`] for anything driven
    /// by the edit cursor: it routes through the same instrument the note
    /// will use on playback, so a sample track previews its sample instead of
    /// falling back to the built-in synth.
    pub fn preview_note_for_cell(
        &mut self,
        order: usize,
        row: usize,
        channel: usize,
        note: u8,
        velocity: u8,
    ) {
        let instrument = self.resolve_edit_instrument(order, row, channel);
        let midi_ch = self.midi_channel_for(channel);
        self.preview_note_with_instrument(midi_ch, note, velocity, instrument);
    }

    pub fn record_note_at(
        &mut self,
        order: usize,
        row: usize,
        channel: usize,
        note: u8,
        velocity: u8,
    ) -> bool {
        let note_index = note % SEMITONES_PER_OCTAVE;
        let octave = note / SEMITONES_PER_OCTAVE;
        let note_val = match NoteValue::from_index(note_index) {
            Some(v) => v,
            None => return false,
        };
        if order >= self.song.order_len() {
            return false;
        }
        let tracker_note = Note::On {
            value: note_val,
            octave,
        };
        // Resolve the instrument the same way manual note entry does, so a
        // recorded note sounds like the ones around it.
        let auto_inst = self.resolve_edit_instrument(order, row, channel);
        let mut cell = *self.song.cell_at(order, row, channel);
        cell.note = Some(tracker_note);
        cell.volume = Some(velocity);
        if let Some(inst) = auto_inst {
            cell.instrument = Some(inst);
        }
        self.song.set_cell(order, row, channel, cell);
        self.dirty = true;
        true
    }

    /// Record a note-off at the given position (punch-in recording).
    pub fn record_note_off_at(&mut self, order: usize, row: usize, channel: usize) {
        if order >= self.song.order_len() {
            return;
        }
        let mut cell = *self.song.cell_at(order, row, channel);
        cell.note = Some(Note::Off);
        self.song.set_cell(order, row, channel, cell);
        self.dirty = true;
    }

    /// Handle incoming MIDI CC: apply learned mappings or forward as thru.
    /// Returns Some(message) if a MIDI learn binding was made.
    pub fn handle_midi_cc(
        &mut self,
        controller: u8,
        value: u8,
        thru_channel: u8,
    ) -> Option<String> {
        // MIDI learn: if waiting for a CC, bind it
        if let Some((ch, param)) = self.midi_learn_pending.take() {
            self.midi_cc_mappings
                .retain(|m| m.cc != controller && !(m.channel == ch && m.param == param));
            self.midi_cc_mappings.push(MidiCcMapping {
                cc: controller,
                channel: ch,
                param,
            });
            return Some(format!(
                "Mapped CC{} -> {} (ch {})",
                controller,
                param.name(),
                ch + 1
            ));
        }

        // Apply any learned CC mappings
        let mut applied = false;
        for mapping in &self.midi_cc_mappings {
            if mapping.cc == controller {
                let ch = mapping.channel;
                if ch < self.channels.len() {
                    mapping
                        .param
                        .apply(&mut self.channels[ch].effects_params, value);
                    if let Some(ref mut audio) = self.audio {
                        audio.set_channel_effects(ch as u8, &self.channels[ch].effects_params);
                    }
                    applied = true;
                }
            }
        }

        if !applied {
            self.send_cc(thru_channel, controller, value);
        }
        None
    }

    // -----------------------------------------------------------------------
    // Channel management
    // -----------------------------------------------------------------------

    /// Toggle mute on a channel. Returns a status message.
    /// Toggle mute on a channel, clearing any solo.
    ///
    /// Returns the channel's new muted state, or `None` if the index is out
    /// of range. Wording for the status bar is the frontend's business.
    pub fn toggle_channel_mute(&mut self, channel: usize) -> Option<bool> {
        let ch_cfg = self.channels.get_mut(channel)?;
        self.solo_channel = None;
        ch_cfg.muted = !ch_cfg.muted;
        let muted = ch_cfg.muted;
        if muted {
            let midi_ch = self.midi_channel_for(channel);
            self.send_channel_note_off(midi_ch);
        }
        Some(muted)
    }

    /// Toggle solo on a channel. Returns the soloed channel, or `None` if
    /// solo was turned off.
    pub fn toggle_solo(&mut self, channel: usize) -> Option<usize> {
        if self.solo_channel == Some(channel) {
            self.solo_channel = None;
            return None;
        }
        self.solo_channel = Some(channel);
        for ch in 0..self.channels.len() {
            if ch != channel {
                let midi_ch = self.midi_channel_for(ch);
                self.send_channel_note_off(midi_ch);
            }
        }
        Some(channel)
    }

    /// Toggle outgoing MIDI clock transmission. Returns whether it is now on.
    pub fn toggle_midi_clock(&mut self) -> bool {
        self.midi.clock_enabled = !self.midi.clock_enabled;
        self.midi.clock_enabled
    }

    /// Map aftertouch pressure to filter cutoff and push to audio engine.
    pub fn apply_aftertouch_to_filter(&mut self, ch: usize, pressure: u8) {
        if ch >= self.channels.len() {
            return;
        }
        let params = &mut self.channels[ch].effects_params;
        if !params.filter_enabled {
            return;
        }
        let t = pressure as f32 / 127.0;
        params.filter_cutoff = 20.0 * (1000.0_f32).powf(t);
        if let Some(ref mut audio) = self.audio {
            audio.set_channel_effects(ch as u8, params);
        }
    }

    // -----------------------------------------------------------------------
    // Sample loading
    // -----------------------------------------------------------------------

    /// Load a sample file into a bank slot. Returns Ok(name) or Err(message).
    pub fn load_sample(&mut self, slot: usize, path: &std::path::Path) -> Result<String> {
        let mut bank = (*self.sample_bank).clone();
        match bank.load(slot, path) {
            Ok(()) => {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sample")
                    .to_string();
                if slot < self.instruments.len() {
                    self.instruments[slot].sample_index = Some(slot);
                    if self.instruments[slot].name.is_empty() {
                        self.instruments[slot].name = name.clone();
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }
                Ok(name)
            }
            Err(e) => Err(Error::Sample { slot, source: e }),
        }
    }

    /// Load samples from a directory named `<slot>-<name>.wav`.
    /// Returns how many slots were filled.
    pub fn load_sample_directory(&mut self, dir: &std::path::Path) -> Result<usize> {
        let mut bank = (*self.sample_bank).clone();
        match bank.load_directory(dir) {
            Ok(meta) => {
                let mut loaded = 0;
                for (i, sample) in bank.samples.iter().enumerate() {
                    if let Some(s) = sample {
                        loaded += 1;
                        if i < self.instruments.len() {
                            self.instruments[i].sample_index = Some(i);
                            if self.instruments[i].name.is_empty() {
                                self.instruments[i].name = s.name.clone();
                            }
                        }
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }
                if let Some(bpm) = meta.bpm {
                    self.song.bpm = bpm;
                    if self.link.is_enabled() {
                        self.link.set_tempo(bpm as f64);
                    }
                }
                Ok(loaded)
            }
            Err(e) => Err(Error::file(dir, e)),
        }
    }

    /// Load a sample into a slot, also making it that track's default
    /// instrument if the track does not have one yet.
    pub fn load_sample_into_slot(&mut self, slot: usize, path: &std::path::Path) -> Result<()> {
        let mut bank = (*self.sample_bank).clone();
        match bank.load(slot, path) {
            Ok(()) => {
                if slot < self.instruments.len() {
                    self.instruments[slot].sample_index = Some(slot);
                    if self.instruments[slot].name.is_empty() {
                        self.instruments[slot].name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("sample")
                            .to_string();
                    }
                }
                if let Some(ch) = self.channels.get_mut(slot) {
                    if ch.default_instrument.is_none() {
                        ch.default_instrument = Some(slot as u8);
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }
                Ok(())
            }
            Err(e) => Err(Error::Sample { slot, source: e }),
        }
    }

    /// Slice a sample. Returns Ok(count) or Err(message).
    /// The sample bank and instrument table as they stand, for undo.
    ///
    /// Slicing rewrites consecutive slots, so an editor needs to be able to
    /// put back what was there. Cheap despite the size: slots are
    /// `Arc<Sample>`, so no audio is copied.
    pub fn snapshot_samples(&self) -> SampleSnapshot {
        SampleSnapshot {
            bank: Arc::clone(&self.sample_bank),
            instruments: self.instruments.clone(),
        }
    }

    /// Put back a snapshot taken by [`TrackerCore::snapshot_samples`].
    pub fn restore_samples(&mut self, snapshot: SampleSnapshot) {
        self.sample_bank = snapshot.bank;
        self.instruments = snapshot.instruments;
        if let Some(ref mut audio) = self.audio {
            audio.set_sample_bank(Arc::clone(&self.sample_bank));
        }
        self.dirty = true;
    }

    /// Slots in `slot..end` holding something this slicing did not produce.
    ///
    /// Re-slicing its own output is the operation working, not destruction:
    /// a kit re-cut from 8 pieces to 16 has to be free to replace the 8. Only
    /// material from elsewhere -- another sample, a synth patch, a name
    /// somebody typed -- is worth stopping for.
    pub fn unrelated_slots(&self, slot: usize, end: usize) -> Vec<usize> {
        let source = self
            .sample_bank
            .get(slot)
            .and_then(|s| s.source_path.clone());

        (slot..end.min(MAX_INSTRUMENTS))
            .filter(|&i| {
                if i == slot {
                    return false; // the sample being sliced
                }
                if let Some(sample) = self.sample_bank.get(i) {
                    // Another slice of the same file is our own output.
                    return sample.source_path != source || source.is_none();
                }
                match self.instruments.get(i) {
                    Some(inst) => {
                        inst.synth_params.is_some()
                            || inst.midi_program.is_some()
                            || !inst.name.is_empty()
                    }
                    None => false,
                }
            })
            .collect()
    }

    /// Slice a slot into `count` pieces (or at detected transients).
    ///
    /// `range` decides what gets divided: [`SliceRange::Source`] re-derives
    /// from the whole sample, so running this again at a different count
    /// replaces the previous slicing rather than eating into it, while
    /// [`SliceRange::Span`] subdivides the slot's own span.
    ///
    /// Slices land in consecutive slots from `slot`, overwriting what is
    /// there. `overwrite` decides whether that is allowed to happen to
    /// instruments this slicing did not itself produce; with
    /// [`SliceOverwrite::Refuse`] such a request fails with
    /// [`Error::SlotsOccupied`] and nothing is written.
    pub fn slice_sample(
        &mut self,
        slot: usize,
        count: usize,
        sensitivity: f32,
        use_transients: bool,
        range: crate::sample::SliceRange,
        overwrite: crate::sample::SliceOverwrite,
    ) -> Result<usize> {
        let Some(sample) = self.sample_bank.get(slot) else {
            return Err(Error::NoSampleInSlot { slot });
        };

        let slices = if use_transients {
            let (start, end) = sample.slice_bounds(range);
            let points = crate::sample::detect_transients_range(sample, sensitivity, start, end);
            crate::sample::slice_at_points(sample, &points, range)
        } else {
            crate::sample::slice_equal(sample, count, range)
        };

        if slices.is_empty() {
            return Err(Error::SampleTooShort { slot });
        }

        let slice_count = slices.len();
        let end_slot = slot + slice_count;
        if end_slot > MAX_INSTRUMENTS {
            return Err(Error::NotEnoughSlots {
                needed: slice_count,
                from_slot: slot,
            });
        }

        if overwrite == crate::sample::SliceOverwrite::Refuse {
            let occupied = self.unrelated_slots(slot, end_slot);
            if let Some(&first) = occupied.first() {
                return Err(Error::SlotsOccupied {
                    first,
                    count: occupied.len(),
                });
            }
        }

        // Collect names before consuming slices
        let slice_names: Vec<String> = slices.iter().map(|s| s.name.clone()).collect();

        let mut bank = (*self.sample_bank).clone();
        for (i, s) in slices.into_iter().enumerate() {
            bank.samples[slot + i] = Some(Arc::new(s));
        }
        self.sample_bank = Arc::new(bank);
        if let Some(ref mut audio) = self.audio {
            audio.set_sample_bank(Arc::clone(&self.sample_bank));
        }

        for (i, name) in slice_names.iter().enumerate() {
            let inst_slot = slot + i;
            if inst_slot < self.instruments.len() {
                self.instruments[inst_slot].sample_index = Some(inst_slot);
                // Name every slot slicing writes into, rather than only the
                // empty ones. Slicing replaces the audio in these slots, so
                // a name describing what used to be there is wrong -- the
                // first slot in particular kept the whole sample's name and
                // ended up as "amen" sitting alongside "amen_S01".
                self.instruments[inst_slot].name = name.clone();
            }
        }

        Ok(slice_count)
    }

    // -----------------------------------------------------------------------
    // Export
    // -----------------------------------------------------------------------

    /// Collect instrument definitions for the offline renderer.
    pub fn export_instruments(&self) -> Vec<crate::sample::export::ExportInstrument> {
        self.instruments
            .iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect()
    }

    /// Return the audio engine's sample rate, or 44100 if no audio engine is running.
    pub fn export_sample_rate(&self) -> u32 {
        self.audio
            .as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100)
    }

    /// Export the song to a WAV file at the given path.
    #[allow(dead_code)]
    pub fn export_wav(&self, path: &std::path::Path) -> Result<()> {
        crate::sample::export::render_to_wav(
            path,
            &self.song,
            &self.sample_bank,
            &self.export_instruments(),
            &self.channel_effects_params_slice(),
            &self.send_bus_params,
            self.export_sample_rate(),
        )
        .map_err(|source| Error::Export { source })
    }

    /// Export the song to a WAV file alongside the song file (or in the current directory).
    pub fn export_wav_to_default(&self) -> Result<PathBuf> {
        let path = self
            .file_path
            .as_ref()
            .map(|p| p.with_extension("wav"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.wav", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        crate::sample::export::render_to_wav(
            &path,
            &self.song,
            &self.sample_bank,
            &instruments,
            &self.channel_effects_params_slice(),
            &self.send_bus_params,
            sample_rate,
        )
        .map(|()| path)
        .map_err(|source| Error::Export { source })
    }

    /// Export the song to a FLAC file alongside the song file (or in the current directory).
    pub fn export_flac_to_default(&self) -> Result<PathBuf> {
        let path = self
            .file_path
            .as_ref()
            .map(|p| p.with_extension("flac"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.flac", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        crate::sample::export::render_to_flac(
            &path,
            &self.song,
            &self.sample_bank,
            &instruments,
            &self.channel_effects_params_slice(),
            &self.send_bus_params,
            sample_rate,
        )
        .map(|()| path)
        .map_err(|source| Error::Export { source })
    }

    /// Export the song to a standard MIDI file alongside the song file.
    pub fn export_midi_to_default(&self) -> Result<PathBuf> {
        let path = self
            .file_path
            .as_ref()
            .map(|p| p.with_extension("mid"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.mid", name))
            });
        crate::midi_file::export_midi(&self.song, &path).map_err(|e| Error::file(&path, e))?;
        Ok(path)
    }

    // -----------------------------------------------------------------------
    // File I/O
    // -----------------------------------------------------------------------

    /// Build a SongFile with instrument definitions and sample references.
    pub fn build_song_file(&self, save_path: &std::path::Path) -> SongFile {
        let save_dir = save_path.parent().unwrap_or(std::path::Path::new("."));

        let instruments: Vec<InstrumentEntry> = self
            .instruments
            .iter()
            .enumerate()
            .filter(|(_, inst)| {
                !inst.name.is_empty()
                    || inst.sample_index.is_some()
                    || inst.midi_program.is_some()
                    || inst.synth_params.is_some()
            })
            .map(|(slot, inst)| InstrumentEntry {
                slot,
                def: InstrumentDef {
                    name: inst.name.clone(),
                    midi_program: inst.midi_program,
                    sample_index: inst.sample_index,
                    synth_params: inst.synth_params.clone(),
                    pitch_bend_range: inst.pitch_bend_range,
                },
            })
            .collect();

        let sample_refs: Vec<SampleRefEntry> = self
            .sample_bank
            .samples
            .iter()
            .enumerate()
            .filter_map(|(slot, opt)| {
                opt.as_ref().map(|sample| {
                    let rel_path = sample
                        .source_path
                        .as_ref()
                        .map(|p| {
                            let abs = std::path::Path::new(p);
                            make_relative(save_dir, abs)
                        })
                        .unwrap_or_default();

                    SampleRefEntry {
                        slot,
                        sample_ref: SampleRef {
                            name: sample.name.clone(),
                            path: rel_path,
                            base_note: sample.base_note,
                            trim_start: sample.trim_start,
                            trim_end: sample.trim_end,
                            loop_enabled: sample.loop_enabled,
                            loop_start: sample.loop_start,
                            loop_end: sample.loop_end,
                        },
                    }
                })
            })
            .collect();

        SongFile {
            version: crate::tracker::FORMAT_VERSION,
            song: self.song.clone(),
            instruments,
            sample_refs,
            channels_config: self.channels.clone(),
            send_buses: self.send_bus_params.clone(),
            midi_cc_mappings: self.midi_cc_mappings.clone(),
        }
    }

    /// Save the song. Returns Ok(message) or Err(message).
    pub fn save(&mut self) -> Result<PathBuf> {
        let path = self.file_path.clone().unwrap_or_else(|| {
            let name = self.song.title.replace(' ', "_").to_lowercase();
            PathBuf::from(format!("{}.rtrk", name))
        });
        let song_file = self.build_song_file(&path);
        song_file.save(&path).map_err(|e| Error::file(&path, e))?;
        self.file_path = Some(path.clone());
        self.dirty = false;
        let _ = std::fs::remove_file(autosave_path_for(&path));
        Ok(path)
    }

    /// Auto-save to a temporary file if the song is dirty and enough time has
    /// elapsed. Doing nothing is success; only a failed write is an error.
    pub fn auto_save(&mut self, last_autosave: &mut Instant) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if last_autosave.elapsed().as_secs() < AUTOSAVE_INTERVAL_SECS {
            return Ok(());
        }
        *last_autosave = Instant::now();
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.rtrk", name))
            }
        };
        let autosave = autosave_path_for(&path);
        let song_file = self.build_song_file(&path);
        song_file
            .save(&autosave)
            .map_err(|e| Error::file(&autosave, e))
    }

    /// Remove the auto-save temp file.
    pub fn cleanup_autosave(&self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.rtrk", name))
            }
        };
        let autosave = autosave_path_for(&path);
        let _ = std::fs::remove_file(autosave);
    }

    /// Load a song file. Restores core state (song, instruments, samples).
    /// The caller is responsible for resetting UI state (cursor, history, etc.).
    pub fn load_file(&mut self, path: &std::path::Path) -> Result<LoadReport> {
        match SongFile::load(path) {
            Ok(song_file) => {
                let from_newer = song_file.is_from_newer_version();
                let mut song = song_file.song;
                let repairs = song.repair();
                self.solo_channel = None;
                self.song = song;

                // Restore mixer state. Files written before channel state was
                // persisted carry no entries, so fall back to defaults and
                // pad any channels the file did not describe.
                self.channels = default_channel_configs(self.song.channels);
                for (i, cfg) in song_file.channels_config.into_iter().enumerate() {
                    if i < self.channels.len() {
                        self.channels[i] = cfg;
                    }
                }
                for (i, bus) in song_file.send_buses.into_iter().enumerate() {
                    if i < self.send_bus_params.len() {
                        self.send_bus_params[i] = bus;
                    }
                }
                self.midi_cc_mappings = song_file.midi_cc_mappings;
                self.push_all_channel_effects();
                self.push_all_send_bus_params();

                // Restore instruments
                self.instruments = (0..MAX_INSTRUMENTS)
                    .map(|_| Instrument::default())
                    .collect();
                for entry in &song_file.instruments {
                    if entry.slot < self.instruments.len() {
                        self.instruments[entry.slot].name = entry.def.name.clone();
                        self.instruments[entry.slot].midi_program = entry.def.midi_program;
                        self.instruments[entry.slot].sample_index = entry.def.sample_index;
                        self.instruments[entry.slot].synth_params = entry.def.synth_params.clone();
                        self.instruments[entry.slot].pitch_bend_range = entry.def.pitch_bend_range;
                    }
                }

                // Reload samples from file references
                let load_dir = path.parent().unwrap_or(std::path::Path::new("."));
                let mut bank = SampleBank::new();
                let mut sample_errors: Vec<(String, String)> = Vec::new();
                // Slices of one source are stored as one path repeated with
                // different spans, so decoding per entry would read the file
                // once per slice and leave each slot holding its own copy of
                // the audio -- the memory the shared buffer exists to avoid.
                // Each distinct path is decoded once and the buffer shared.
                let mut decoded: std::collections::HashMap<std::path::PathBuf, Arc<Sample>> =
                    std::collections::HashMap::new();
                for entry in &song_file.sample_refs {
                    if entry.slot >= bank.samples.len() {
                        continue;
                    }
                    // A path the song cannot legitimately name is reported
                    // like a missing file rather than resolved to something
                    // else: loading the wrong sample silently is worse than
                    // loading none.
                    let sample_path = match resolve_relative(load_dir, &entry.sample_ref.path) {
                        Ok(p) => p,
                        Err(e) => {
                            sample_errors.push((entry.sample_ref.name.clone(), e.to_string()));
                            continue;
                        }
                    };
                    let source = match decoded.get(&sample_path) {
                        Some(s) => Ok(Arc::clone(s)),
                        None => {
                            let mut scratch = SampleBank::new();
                            scratch.load(0, &sample_path).map(|()| {
                                let loaded = scratch.samples[0].take().expect("load filled slot 0");
                                decoded.insert(sample_path.clone(), Arc::clone(&loaded));
                                loaded
                            })
                        }
                    };
                    match source {
                        Ok(source) => {
                            let mut sample = (*source).clone();
                            sample.name = entry.sample_ref.name.clone();
                            sample.base_note = entry.sample_ref.base_note;
                            // The file on disk may have been replaced since
                            // the song was saved. A span past the end of a
                            // shorter file would play nothing at all, with
                            // no indication why, so clamp it to the audio
                            // that is actually there.
                            let frames = sample.data.len();
                            sample.trim_start = entry.sample_ref.trim_start.min(frames);
                            sample.trim_end = entry.sample_ref.trim_end.min(frames);
                            sample.loop_enabled = entry.sample_ref.loop_enabled;
                            sample.loop_start = entry.sample_ref.loop_start.min(frames);
                            sample.loop_end = entry.sample_ref.loop_end.min(frames);
                            bank.samples[entry.slot] = Some(Arc::new(sample));
                        }
                        Err(e) => {
                            sample_errors.push((entry.sample_ref.name.clone(), e.to_string()));
                        }
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }

                self.file_path = Some(path.to_path_buf());
                self.dirty = false;
                Ok(LoadReport {
                    path: path.to_path_buf(),
                    repairs,
                    missing_samples: sample_errors,
                    from_newer_version: from_newer,
                })
            }
            Err(e) => Err(Error::file(path, e)),
        }
    }

    /// Import a MIDI file. Returns Ok(song) or Err(message).
    pub fn import_midi_file(&mut self, path: &std::path::Path) -> Result<LoadReport> {
        match crate::midi_file::import_midi(path) {
            Ok(song) => {
                self.channels = default_channel_configs(song.channels);
                self.solo_channel = None;
                self.song = song;
                Ok(LoadReport {
                    path: path.to_path_buf(),
                    ..LoadReport::default()
                })
            }
            Err(e) => Err(Error::file(path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_headless() {
        let core = TrackerCoreBuilder::new()
            .song_size(2, 32)
            .headless()
            .build();
        assert_eq!(core.song.patterns[0].rows, 32);
        assert_eq!(core.channels.len(), 2);
        assert!(!core.midi.is_connected());
        assert!(!core.playing);
    }

    #[test]
    fn test_builder_default_matches_new() {
        let from_new = TrackerCore::new();
        let from_builder = TrackerCoreBuilder::new().build();
        assert_eq!(
            from_new.song.patterns.len(),
            from_builder.song.patterns.len()
        );
        assert_eq!(from_new.channels.len(), from_builder.channels.len());
        assert_eq!(from_new.instruments.len(), from_builder.instruments.len());
    }

    #[test]
    fn test_builder_custom_song_size() {
        let core = TrackerCoreBuilder::new()
            .song_size(8, 128)
            .headless()
            .build();
        assert_eq!(core.song.patterns[0].rows, 128);
        assert_eq!(core.channels.len(), 8);
    }

    #[test]
    fn test_builder_injected_midi() {
        let midi = MidiEngine::default();
        let core = TrackerCoreBuilder::new().headless().midi(midi).build();
        assert!(!core.midi.is_connected());
    }

    // -- Scheduler --

    #[test]
    fn frames_per_tick_matches_the_tick_rate() {
        // 120 BPM, 24 ticks per beat -> 48 ticks/sec -> 1000 frames at 48kHz.
        let spt = 1.0 / 48.0;
        assert_eq!(TrackerCore::frames_per_tick(spt, 48_000.0), 1000);
        assert_eq!(TrackerCore::frames_per_tick(spt, 44_100.0), 919); // 918.75 rounded
    }

    #[test]
    fn frames_per_tick_never_returns_zero() {
        // An absurd tempo must not produce a zero advance, which would spin
        // the scheduler loop forever.
        assert_eq!(TrackerCore::frames_per_tick(0.0, 48_000.0), 1);
        assert_eq!(TrackerCore::frames_per_tick(1e-9, 48_000.0), 1);
    }

    #[test]
    fn ticks_accumulate_to_the_right_rate_over_a_bar() {
        // Four beats at 120 BPM is two seconds; at 24 ticks per beat that is
        // 96 ticks, and the frame advances must sum to two seconds of audio.
        let spt = 1.0 / 48.0;
        let sr = 48_000.0;
        let total: u64 = (0..96).map(|_| TrackerCore::frames_per_tick(spt, sr)).sum();
        assert_eq!(total, 96_000, "96 ticks at 48kHz should span 2 seconds");
    }

    #[test]
    fn playback_position_falls_back_to_the_engine_without_audio() {
        // A headless core has no frame clock, so nothing is scheduled ahead
        // and the engine position is the audible position.
        let mut core = TrackerCoreBuilder::new()
            .song_size(2, 16)
            .headless()
            .build();
        core.engine.sounding_order = 0;
        core.engine.sounding_row = 7;
        assert_eq!(core.playback_position(), (0, 7));
    }

    #[test]
    fn wall_clock_playback_still_advances_without_an_audio_device() {
        // MIDI-only and headless runs keep the original timing path.
        let mut core = TrackerCoreBuilder::new()
            .song_size(1, 16)
            .headless()
            .build();
        core.play(0, 0);
        assert!(core.is_playing());
        // Two calls: the first only records a baseline timestamp.
        core.tick_playback();
        std::thread::sleep(std::time::Duration::from_millis(30));
        core.tick_playback();
        assert!(
            core.timing.playback_elapsed > 0.0,
            "wall-clock path did not advance"
        );
    }

    #[test]
    fn stopping_discards_positions_queued_ahead_of_the_clock() {
        let mut core = TrackerCoreBuilder::new()
            .song_size(1, 16)
            .headless()
            .build();
        core.timing
            .scheduled_positions
            .push_back(ScheduledPosition {
                frame: 1000,
                order: 0,
                row: 3,
            });
        core.play(0, 0);
        core.stop();
        assert!(
            core.timing.scheduled_positions.is_empty(),
            "queued positions must not outlive the transport"
        );
    }

    #[test]
    fn scheduled_positions_are_capped() {
        let mut core = TrackerCoreBuilder::new()
            .song_size(1, 16)
            .headless()
            .build();
        for i in 0..(MAX_SCHEDULED_POSITIONS + 50) {
            core.record_scheduled_position(i as u64, 0, i);
        }
        assert_eq!(
            core.timing.scheduled_positions.len(),
            MAX_SCHEDULED_POSITIONS
        );
        // The newest entries are the ones kept.
        assert_eq!(
            core.timing.scheduled_positions.back().map(|p| p.row),
            Some(MAX_SCHEDULED_POSITIONS + 49)
        );
    }

    #[test]
    fn expiring_positions_keeps_the_most_recent_audible_one() {
        let mut core = TrackerCoreBuilder::new()
            .song_size(1, 16)
            .headless()
            .build();
        for (frame, row) in [(0u64, 0usize), (100, 1), (200, 2), (300, 3)] {
            core.record_scheduled_position(frame, 0, row);
        }
        // The listener is at frame 250: rows 0 and 1 are history, row 2 is
        // what is being heard, row 3 is still queued.
        core.expire_scheduled_positions(250);
        assert_eq!(
            core.timing.scheduled_positions.front().map(|p| p.row),
            Some(2)
        );
        assert_eq!(core.timing.scheduled_positions.len(), 2);
    }

    // -- Instrument resolution when editing --

    fn sample_core() -> TrackerCore {
        let mut core = TrackerCoreBuilder::new()
            .song_size(2, 16)
            .headless()
            .build();
        // Eight slices of one sample, as `slice_sample` produces.
        for slot in 0..8 {
            core.instruments[slot].sample_index = Some(slot);
        }
        core
    }

    fn note_cell(instrument: Option<u8>) -> crate::tracker::Cell {
        crate::tracker::Cell {
            note: Some(Note::On {
                value: NoteValue::C,
                octave: 5,
            }),
            instrument,
            ..crate::tracker::Cell::default()
        }
    }

    #[test]
    fn editing_inherits_the_instrument_from_the_column_above() {
        // The case that made sliced samples unusable: the track is Midi-typed
        // (which is what every song saved before channel state was persisted
        // loads as), so there is no track default, and an empty cell resolved
        // to nothing and fell through to the built-in synth.
        let mut core = sample_core();
        core.song.set_cell(0, 0, 0, note_cell(Some(3)));
        assert_eq!(core.resolve_edit_instrument(0, 4, 0), Some(3));
    }

    #[test]
    fn the_nearest_instrument_above_wins() {
        let mut core = sample_core();
        core.song.set_cell(0, 0, 0, note_cell(Some(1)));
        core.song.set_cell(0, 4, 0, note_cell(Some(6)));
        assert_eq!(core.resolve_edit_instrument(0, 8, 0), Some(6));
        assert_eq!(core.resolve_edit_instrument(0, 2, 0), Some(1));
    }

    #[test]
    fn a_cells_own_instrument_is_kept_when_re_entering_a_note() {
        let mut core = sample_core();
        core.song.set_cell(0, 0, 0, note_cell(Some(1)));
        core.song.set_cell(0, 4, 0, note_cell(Some(6)));
        // Editing the note at row 4 must not silently retune it to row 0's.
        assert_eq!(core.resolve_edit_instrument(0, 4, 0), Some(6));
    }

    #[test]
    fn the_track_default_takes_priority_where_one_is_set() {
        let mut core = sample_core();
        core.channels[0].channel_type = ChannelType::Sample;
        core.channels[0].default_instrument = Some(2);
        core.song.set_cell(0, 0, 0, note_cell(Some(7)));
        assert_eq!(
            core.resolve_edit_instrument(0, 4, 0),
            Some(2),
            "an explicitly selected instrument is what the user is placing"
        );
    }

    #[test]
    fn midi_tracks_do_not_pick_up_a_track_default() {
        let mut core = sample_core();
        core.channels[0].channel_type = ChannelType::Midi;
        core.channels[0].default_instrument = Some(2);
        assert_eq!(core.resolve_edit_instrument(0, 0, 0), None);
    }

    #[test]
    fn resolution_is_per_channel() {
        let mut core = sample_core();
        core.song.set_cell(0, 0, 0, note_cell(Some(3)));
        assert_eq!(core.resolve_edit_instrument(0, 4, 0), Some(3));
        assert_eq!(
            core.resolve_edit_instrument(0, 4, 1),
            None,
            "channel 1 has nothing above it"
        );
    }

    #[test]
    fn resolution_does_not_reach_across_order_positions() {
        let mut core = sample_core();
        core.song.order = vec![0, 1];
        core.song.sync_order_repeats();
        core.song.patterns.push(crate::tracker::Pattern::new(16, 2));
        core.song.set_cell(0, 0, 0, note_cell(Some(3)));
        assert_eq!(
            core.resolve_edit_instrument(1, 4, 0),
            None,
            "a later pattern should not inherit from an earlier one"
        );
    }

    #[test]
    fn an_empty_column_resolves_to_nothing() {
        let core = sample_core();
        assert_eq!(core.resolve_edit_instrument(0, 8, 0), None);
    }

    #[test]
    fn out_of_range_coordinates_resolve_to_nothing() {
        let core = sample_core();
        assert_eq!(core.resolve_edit_instrument(99, 0, 0), None);
        assert_eq!(core.resolve_edit_instrument(0, 0, 99), None);
    }

    #[test]
    fn recorded_notes_inherit_the_instrument_from_the_column_above() {
        // MIDI step and punch-in recording route through the same resolution.
        let mut core = sample_core();
        core.song.set_cell(0, 0, 0, note_cell(Some(3)));
        assert!(core.record_note_at(0, 4, 0, 60, 100));
        assert_eq!(core.song.cell_at(0, 4, 0).instrument, Some(3));
    }

    #[test]
    fn recorded_notes_still_honour_a_track_default() {
        let mut core = sample_core();
        core.channels[0].channel_type = ChannelType::Synth;
        core.channels[0].default_instrument = Some(5);
        assert!(core.record_note_at(0, 0, 0, 60, 100));
        assert_eq!(core.song.cell_at(0, 0, 0).instrument, Some(5));
    }
}
