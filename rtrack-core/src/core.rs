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
use crate::sample::SampleBank;
use crate::tracker::{Note, NoteValue, Song, SongFile, InstrumentDef, InstrumentEntry, SampleRef, SampleRefEntry};

use crate::types::{
    ChannelConfig, ChannelType, ClockMode, Instrument, LearnableParam,
    MidiCcMapping, PlaybackTiming,
    autosave_path_for, default_channel_configs, make_relative, resolve_relative,
    AUTOSAVE_INTERVAL_SECS,
};

/// Headless tracker core. Owns all non-UI state: song data, playback engine,
/// audio/MIDI I/O, channel configuration, instruments, and samples.
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
    pub preview_note: Option<(u8, u8, Instant)>,
}

impl TrackerCore {
    /// Create a new headless tracker core with default state.
    pub fn new() -> Self {
        let mut midi = MidiEngine::new();
        if midi.create_virtual_port().is_err() {
            let _ = midi.connect_first_available();
        }

        let mut midi_input = MidiInputEngine::new();
        let _ = midi_input.create_virtual_port();

        let song = Song::new(4, 64);
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
            channels: default_channel_configs(4),
            instruments: (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect(),
            send_bus_params: (0..crate::audio::effects::MAX_SEND_BUSES)
                .map(|_| crate::audio::effects::SendBusParams::default())
                .collect(),
            solo_channel: None,
            file_path: None,
            dirty: false,
            midi_cc_mappings: Vec::new(),
            midi_learn_pending: None,
            preview_note: None,
        }
    }
}

impl Default for TrackerCore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackerCore {
    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn midi_connected(&self) -> bool {
        self.midi.is_connected()
    }

    pub fn midi_port_display_name(&self) -> &str {
        self.midi.port_name.as_deref().unwrap_or("--")
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    pub fn has_sf2(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.has_sf2())
    }

    pub fn audio_effects_enabled(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.effects_enabled())
    }

    #[allow(dead_code)]
    pub fn toggle_audio_effects(&mut self) -> bool {
        self.audio.as_mut().is_some_and(|a| a.toggle_effects())
    }

    pub fn midi_channel_for(&self, tracker_channel: usize) -> u8 {
        let ch = self.channels.get(tracker_channel)
            .map(|c| c.midi_channel)
            .unwrap_or(tracker_channel as u8);
        ch & 0x0F
    }

    pub fn is_channel_audible(&self, channel: usize) -> bool {
        if let Some(solo) = self.solo_channel {
            return channel == solo;
        }
        self.channels.get(channel).is_none_or(|c| !c.muted)
    }

    pub fn channel_effects_params_slice(&self) -> Vec<crate::audio::channel_effects::ChannelEffectsParams> {
        self.channels.iter().map(|c| c.effects_params.clone()).collect()
    }

    pub fn channel_pitch_bend_per_semitone(&self, ch: usize) -> f64 {
        let range = self.engine.channel_states.get(ch)
            .and_then(|cs| cs.active_instrument)
            .and_then(|idx| self.instruments.get(idx as usize))
            .and_then(|inst| inst.pitch_bend_range)
            .unwrap_or(DEFAULT_PITCH_BEND_RANGE);
        (PITCH_BEND_CENTER as f64) / range
    }

    #[allow(dead_code)]
    pub fn instrument_has_sample(&self, inst: usize) -> bool {
        self.instruments.get(inst)
            .and_then(|i| i.sample_index)
            .and_then(|idx| self.sample_bank.get(idx))
            .is_some()
    }

    // -----------------------------------------------------------------------
    // Playback
    // -----------------------------------------------------------------------

    pub fn toggle_link(&mut self) {
        if self.link.is_enabled() {
            self.link.disable();
        } else {
            self.link.enable();
        }
    }

    pub fn toggle_playback(&mut self, start_order: usize, start_row: usize) {
        if self.playing {
            self.stop();
        } else {
            self.play(start_order, start_row);
        }
    }

    pub fn play(&mut self, start_order: usize, start_row: usize) {
        self.playing = true;
        self.engine.reset(&self.song, start_order, start_row);
        self.sync_engine_channel_info();
        self.timing.reset();
        self.timing.last_tick = Some(Instant::now());
        if self.link.is_enabled() {
            self.timing.last_link_beat = self.link.beat_at_time_now();
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.timing.last_tick = None;
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

    pub fn sync_link(&mut self) {
        if !self.link.is_enabled() {
            return;
        }

        if let Some(new_tempo) = self.link.poll_tempo_change() {
            let new_bpm = new_tempo.round() as u16;
            if new_bpm != self.song.bpm && (32..=300).contains(&new_bpm) {
                self.song.bpm = new_bpm;
                self.engine.bpm = new_bpm;
            }
        }
    }

    pub fn sync_engine_channel_info(&mut self) {
        let infos: Vec<crate::engine::ChannelInfo> = self.channels.iter().enumerate().map(|(i, ch)| {
            crate::engine::ChannelInfo {
                audible: self.is_channel_audible(i),
                volume_scale: ch.volume,
                default_instrument: ch.default_instrument,
                is_synth: ch.channel_type == ChannelType::Synth,
            }
        }).collect();
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

        let mut ticked = false;

        // Link beat-timeline mode
        if self.link.is_enabled() {
            let beat = self.link.beat_at_time_now();
            let link_ticks = beat * MIDI_CLOCKS_PER_BEAT;
            let last_ticks = self.timing.last_link_beat * MIDI_CLOCKS_PER_BEAT;
            let delta_ticks = link_ticks - last_ticks;
            if delta_ticks > 0.0 {
                let spt = self.engine.seconds_per_tick(&self.song);
                let ticks_per_second = 1.0 / spt;
                let tracker_ticks = delta_ticks / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT / 60.0) * ticks_per_second;
                self.timing.tick_accumulator += tracker_ticks * spt;
                self.timing.playback_elapsed += delta_ticks / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT / 60.0);
            }
            self.timing.last_link_beat = beat;

            let spt = self.engine.seconds_per_tick(&self.song);
            while self.timing.tick_accumulator >= spt {
                self.timing.tick_accumulator -= spt;
                self.process_tick();
                ticked = true;
            }
            return ticked;
        }

        let now = Instant::now();
        if let Some(last) = self.timing.last_tick {
            let elapsed = now.duration_since(last).as_secs_f64();
            self.timing.tick_accumulator += elapsed;
            self.timing.playback_elapsed += elapsed;

            if self.midi.clock_enabled {
                self.timing.clock_tick_accumulator += elapsed;
                let clock_interval = 60.0 / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT);
                while self.timing.clock_tick_accumulator >= clock_interval {
                    self.timing.clock_tick_accumulator -= clock_interval;
                    let _ = self.midi.send_clock();
                }
            }

            let spt = self.engine.seconds_per_tick(&self.song);
            while self.timing.tick_accumulator >= spt {
                self.timing.tick_accumulator -= spt;
                self.process_tick();
                ticked = true;
            }
        }
        self.timing.last_tick = Some(now);
        ticked
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
                TrackerEvent::NoteOn { channel, midi_note, velocity, instrument } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_note_on_with_instrument(midi_ch, midi_note, velocity, instrument);
                }
                TrackerEvent::NoteOff { channel } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_channel_note_off(midi_ch);
                }
                TrackerEvent::PitchBend { channel, semitone_offset } => {
                    let midi_ch = self.midi_channel_for(channel);
                    let pb_per_semi = self.channel_pitch_bend_per_semitone(channel);
                    let bend = (semitone_offset * pb_per_semi) as i32;
                    let value = (PITCH_BEND_CENTER as i32 + bend).clamp(0, PITCH_BEND_MAX as i32) as u16;
                    self.send_pitch_bend(midi_ch, value);
                }
                TrackerEvent::VolumeChange { channel, volume } => {
                    let midi_ch = self.midi_channel_for(channel);
                    self.send_cc(midi_ch, 7, volume);
                }
                TrackerEvent::MidiCC { channel, controller, value } => {
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
                    self.song.bpm = bpm;
                    if self.link.is_enabled() {
                        self.link.set_tempo(bpm as f64);
                    }
                }
                TrackerEvent::RowAdvanced { .. } | TrackerEvent::GenerationAdvanced { .. } => {}
            }
        }
    }

    // -- External MIDI clock sync --

    pub fn toggle_clock_mode(&mut self) {
        self.clock_mode = match self.clock_mode {
            ClockMode::Internal => {
                self.timing.ext_clock_count = 0;
                ClockMode::ExternalMidi
            }
            ClockMode::ExternalMidi => ClockMode::Internal,
        };
    }

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

    pub fn send_note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        let _ = self.midi.note_on(channel, note, velocity);
        if let Some(ref mut audio) = self.audio {
            audio.note_on(channel, note, velocity);
        }
    }

    pub fn send_note_on_with_instrument(&mut self, channel: u8, note: u8, velocity: u8, instrument: Option<u8>) {
        let inst_idx = instrument.unwrap_or(0) as usize;
        let inst = self.instruments.get(inst_idx);

        // Route 1: sample engine
        if let Some(sid) = inst.and_then(|i| i.sample_index) {
            if self.sample_bank.get(sid).is_some() {
                let _ = self.midi.note_on(channel, note, velocity);
                if let Some(ref mut audio) = self.audio {
                    audio.sample_note_on(sid, note, velocity, channel);
                }
                return;
            }
        }

        // Route 2: custom synth params
        if let Some(params) = inst.and_then(|i| i.synth_params.as_ref()) {
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                audio.note_on_with_params(channel, note, velocity, params);
            }
            return;
        }

        // Route 3: instrument number maps to preset patch
        if instrument.is_some() {
            let params = crate::audio::synth::SynthParams::from_patch(inst_idx as u8);
            let _ = self.midi.note_on(channel, note, velocity);
            if let Some(ref mut audio) = self.audio {
                audio.note_on_with_params(channel, note, velocity, &params);
            }
            return;
        }

        // Route 4: default synth
        self.send_note_on(channel, note, velocity);
    }

    pub fn send_channel_note_off(&mut self, channel: u8) {
        let _ = self.midi.channel_note_off(channel);
        if let Some(ref mut audio) = self.audio {
            audio.note_off_all_channel(channel);
            audio.sample_note_off_channel(channel);
        }
    }

    pub fn send_note_off(&mut self, channel: u8, note: u8) {
        let _ = self.midi.note_off(channel, note);
        if let Some(ref mut audio) = self.audio {
            audio.note_off(channel, note);
            audio.sample_note_off(channel, note);
        }
    }

    pub fn preview_note(&mut self, channel: u8, note: u8, velocity: u8) {
        self.preview_note_with_instrument(channel, note, velocity, None);
    }

    pub fn preview_note_with_instrument(&mut self, channel: u8, note: u8, velocity: u8, instrument: Option<u8>) {
        if let Some((prev_ch, _prev_note, _)) = self.preview_note.take() {
            self.send_channel_note_off(prev_ch);
        }
        if instrument.is_some() {
            self.send_note_on_with_instrument(channel, note, velocity, instrument);
        } else {
            self.send_note_on(channel, note, velocity);
        }
        self.preview_note = Some((channel, note, Instant::now()));
    }

    pub fn expire_preview_note(&mut self) {
        if let Some((ch, _note, started)) = self.preview_note {
            if started.elapsed() > std::time::Duration::from_millis(PREVIEW_NOTE_TIMEOUT_MS) {
                self.send_channel_note_off(ch);
                self.preview_note = None;
            }
        }
    }

    pub fn send_all_notes_off(&mut self) {
        let _ = self.midi.all_notes_off();
        if let Some(ref mut audio) = self.audio {
            audio.note_off_all();
            audio.sample_note_off_all();
        }
    }

    pub fn send_cc(&mut self, channel: u8, controller: u8, value: u8) {
        let _ = self.midi.send_cc(channel, controller, value);
        if let Some(ref mut audio) = self.audio {
            audio.send_cc(channel, controller, value);
        }
    }

    pub fn send_program_change(&mut self, channel: u8, program: u8) {
        let _ = self.midi.program_change(channel, program);
        if let Some(ref mut audio) = self.audio {
            audio.program_change(channel, program);
        }
    }

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
    pub fn record_note_at(&mut self, order: usize, row: usize, channel: usize, note: u8, velocity: u8) -> bool {
        let note_index = note % SEMITONES_PER_OCTAVE;
        let octave = note / SEMITONES_PER_OCTAVE;
        let note_val = match NoteValue::from_index(note_index) {
            Some(v) => v,
            None => return false,
        };
        if order >= self.song.order.len() {
            return false;
        }
        let pattern_idx = self.song.order[order];
        if pattern_idx >= self.song.patterns.len() {
            return false;
        }
        let tracker_note = Note::On { value: note_val, octave };
        let cell = self.song.patterns[pattern_idx].get_mut(row, channel);
        cell.note = Some(tracker_note);
        cell.volume = Some(velocity);
        // Auto-fill instrument from track default
        let ch_type = self.channels.get(channel).map(|c| c.channel_type);
        if ch_type == Some(ChannelType::Synth) || ch_type == Some(ChannelType::Sample) {
            if let Some(inst) = self.channels.get(channel).and_then(|c| c.default_instrument) {
                cell.instrument = Some(inst);
            }
        }
        self.dirty = true;
        true
    }

    /// Record a note-off at the given position (punch-in recording).
    pub fn record_note_off_at(&mut self, order: usize, row: usize, channel: usize) {
        if order >= self.song.order.len() {
            return;
        }
        let pattern_idx = self.song.order[order];
        if pattern_idx >= self.song.patterns.len() {
            return;
        }
        let cell = self.song.patterns[pattern_idx].get_mut(row, channel);
        cell.note = Some(Note::Off);
        self.dirty = true;
    }

    /// Handle incoming MIDI CC: apply learned mappings or forward as thru.
    /// Returns Some(message) if a MIDI learn binding was made.
    pub fn handle_midi_cc(&mut self, controller: u8, value: u8, thru_channel: u8) -> Option<String> {
        // MIDI learn: if waiting for a CC, bind it
        if let Some((ch, param)) = self.midi_learn_pending.take() {
            self.midi_cc_mappings.retain(|m| m.cc != controller && !(m.channel == ch && m.param == param));
            self.midi_cc_mappings.push(MidiCcMapping { cc: controller, channel: ch, param });
            return Some(format!("Mapped CC{} -> {} (ch {})", controller, param.name(), ch + 1));
        }

        // Apply any learned CC mappings
        let mut applied = false;
        for mapping in &self.midi_cc_mappings {
            if mapping.cc == controller {
                let ch = mapping.channel;
                if ch < self.channels.len() {
                    mapping.param.apply(&mut self.channels[ch].effects_params, value);
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
    pub fn toggle_channel_mute(&mut self, channel: usize) -> Option<String> {
        if let Some(ch_cfg) = self.channels.get_mut(channel) {
            self.solo_channel = None;
            ch_cfg.muted = !ch_cfg.muted;
            let muted = ch_cfg.muted;
            let state = if muted { "muted" } else { "unmuted" };
            if muted {
                let midi_ch = self.midi_channel_for(channel);
                self.send_channel_note_off(midi_ch);
            }
            Some(format!("Ch {} {}", channel + 1, state))
        } else {
            None
        }
    }

    /// Toggle solo on a channel. Returns a status message.
    pub fn toggle_solo(&mut self, channel: usize) -> String {
        if self.solo_channel == Some(channel) {
            self.solo_channel = None;
            "Solo off".to_string()
        } else {
            self.solo_channel = Some(channel);
            for ch in 0..self.channels.len() {
                if ch != channel {
                    let midi_ch = self.midi_channel_for(ch);
                    self.send_channel_note_off(midi_ch);
                }
            }
            format!("Solo ch {}", channel + 1)
        }
    }

    pub fn toggle_midi_clock(&mut self) -> String {
        self.midi.clock_enabled = !self.midi.clock_enabled;
        let state = if self.midi.clock_enabled { "on" } else { "off" };
        format!("MIDI clock {}", state)
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
    pub fn load_sample(&mut self, slot: usize, path: &std::path::Path) -> Result<String, String> {
        let mut bank = (*self.sample_bank).clone();
        match bank.load(slot, path) {
            Ok(()) => {
                let name = path.file_stem()
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
            Err(e) => Err(format!("Sample load error: {}", e)),
        }
    }

    /// Load samples from a directory. Returns Ok(message) or Err(message).
    pub fn load_sample_directory(&mut self, dir: &std::path::Path) -> Result<String, String> {
        let mut bank = (*self.sample_bank).clone();
        match bank.load_directory(dir) {
            Ok(meta) => {
                for (i, sample) in bank.samples.iter().enumerate() {
                    if let Some(s) = sample {
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
                Ok(format!("Loaded samples from: {}", dir.display()))
            }
            Err(e) => Err(format!("Sample dir error: {}", e)),
        }
    }

    /// Load a sample via file browser into a slot. Returns status message.
    pub fn load_sample_into_slot(&mut self, slot: usize, path: &std::path::Path) -> String {
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
                format!("Loaded sample into slot {:02X}", slot)
            }
            Err(e) => format!("Failed to load: {}", e),
        }
    }

    /// Slice a sample. Returns Ok(count) or Err(message).
    pub fn slice_sample(&mut self, slot: usize, count: usize, sensitivity: f32, use_transients: bool) -> Result<usize, String> {
        let sample = match self.sample_bank.get(slot) {
            Some(s) => s.clone(),
            None => return Err("No sample loaded in this slot".to_string()),
        };

        let slices = if use_transients {
            let points = crate::sample::detect_transients(&sample, sensitivity);
            crate::sample::slice_at_points(&sample, &points)
        } else {
            crate::sample::slice_equal(&sample, count)
        };

        if slices.is_empty() {
            return Err("Sample too short to slice".to_string());
        }

        let end_slot = slot + slices.len();
        if end_slot > 256 {
            return Err(format!("Not enough sample slots (need {} from slot {:02X})", slices.len(), slot));
        }

        let mut bank = (*self.sample_bank).clone();
        for (i, s) in slices.iter().enumerate() {
            bank.samples[slot + i] = Some(s.clone());
        }
        let slice_count = slices.len();
        self.sample_bank = Arc::new(bank);
        if let Some(ref mut audio) = self.audio {
            audio.set_sample_bank(Arc::clone(&self.sample_bank));
        }

        for (i, slice) in slices.iter().enumerate().take(slice_count) {
            let inst_slot = slot + i;
            if inst_slot < self.instruments.len() {
                self.instruments[inst_slot].sample_index = Some(inst_slot);
                if self.instruments[inst_slot].name.is_empty() {
                    self.instruments[inst_slot].name = slice.name.clone();
                }
            }
        }

        Ok(slice_count)
    }

    // -----------------------------------------------------------------------
    // Export
    // -----------------------------------------------------------------------

    pub fn export_instruments(&self) -> Vec<crate::sample::export::ExportInstrument> {
        self.instruments.iter()
            .map(|i| crate::sample::export::ExportInstrument {
                sample_index: i.sample_index,
                midi_program: i.midi_program.unwrap_or(0),
                synth_params: i.synth_params.clone(),
            })
            .collect()
    }

    pub fn export_sample_rate(&self) -> u32 {
        self.audio.as_ref()
            .map(|a| a.sample_rate() as u32)
            .unwrap_or(44100)
    }

    #[allow(dead_code)]
    pub fn export_wav(&self, path: &std::path::Path) -> Result<(), String> {
        crate::sample::export::render_to_wav(
            path, &self.song, &self.sample_bank, &self.export_instruments(),
            &self.channel_effects_params_slice(), &self.send_bus_params,
            self.export_sample_rate(),
        ).map_err(|e| format!("{}", e))
    }

    pub fn export_wav_to_default(&self) -> Result<String, String> {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("wav"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.wav", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        crate::sample::export::render_to_wav(
            &path, &self.song, &self.sample_bank, &instruments,
            &self.channel_effects_params_slice(), &self.send_bus_params, sample_rate,
        ).map(|()| format!("Exported WAV: {}", path.display()))
         .map_err(|e| format!("WAV export failed: {}", e))
    }

    pub fn export_flac_to_default(&self) -> Result<String, String> {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("flac"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.flac", name))
            });
        let instruments = self.export_instruments();
        let sample_rate = self.export_sample_rate();
        crate::sample::export::render_to_flac(
            &path, &self.song, &self.sample_bank, &instruments,
            &self.channel_effects_params_slice(), &self.send_bus_params, sample_rate,
        ).map(|()| format!("Exported FLAC: {}", path.display()))
         .map_err(|e| format!("FLAC export failed: {}", e))
    }

    pub fn export_midi_to_default(&self) -> Result<String, String> {
        let path = self.file_path.as_ref()
            .map(|p| p.with_extension("mid"))
            .unwrap_or_else(|| {
                let name = self.song.title.replace(' ', "_").to_lowercase();
                PathBuf::from(format!("{}.mid", name))
            });
        crate::midi_file::export_midi(&self.song, &path)
            .map(|()| format!("Exported: {}", path.display()))
            .map_err(|e| format!("Export failed: {}", e))
    }

    // -----------------------------------------------------------------------
    // File I/O
    // -----------------------------------------------------------------------

    /// Build a SongFile with instrument definitions and sample references.
    pub fn build_song_file(&self, save_path: &std::path::Path) -> SongFile {
        let save_dir = save_path.parent().unwrap_or(std::path::Path::new("."));

        let instruments: Vec<InstrumentEntry> = self.instruments.iter().enumerate()
            .filter(|(_, inst)| !inst.name.is_empty() || inst.sample_index.is_some() || inst.midi_program.is_some() || inst.synth_params.is_some())
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

        let sample_refs: Vec<SampleRefEntry> = self.sample_bank.samples.iter().enumerate()
            .filter_map(|(slot, opt)| {
                opt.as_ref().map(|sample| {
                    let rel_path = sample.source_path.as_ref().map(|p| {
                        let abs = std::path::Path::new(p);
                        make_relative(save_dir, abs)
                    }).unwrap_or_default();

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
            song: self.song.clone(),
            instruments,
            sample_refs,
        }
    }

    /// Save the song. Returns Ok(message) or Err(message).
    pub fn save(&mut self) -> Result<String, String> {
        let path = self.file_path.clone().unwrap_or_else(|| {
            let name = self.song.title.replace(' ', "_").to_lowercase();
            PathBuf::from(format!("{}.rtrk", name))
        });
        let song_file = self.build_song_file(&path);
        match song_file.save(&path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
                self.dirty = false;
                let _ = std::fs::remove_file(autosave_path_for(&path));
                Ok(format!("Saved: {}", path.display()))
            }
            Err(e) => Err(format!("Save failed: {}", e)),
        }
    }

    /// Auto-save to a temporary file if dirty and enough time has elapsed.
    /// Returns Ok(()) if saved, Err(msg) on failure, or Ok(()) if skipped.
    pub fn auto_save(&mut self, last_autosave: &mut Instant) -> Option<String> {
        if !self.dirty {
            return None;
        }
        if last_autosave.elapsed().as_secs() < AUTOSAVE_INTERVAL_SECS {
            return None;
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
        if let Err(e) = song_file.save(&autosave) {
            return Some(format!("Auto-save failed: {}", e));
        }
        None
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
    pub fn load_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        match SongFile::load(path) {
            Ok(song_file) => {
                let song = song_file.song;
                self.channels = default_channel_configs(song.channels);
                self.solo_channel = None;
                self.song = song;
                self.song.sync_order_repeats();

                // Restore instruments
                self.instruments = (0..MAX_INSTRUMENTS).map(|_| Instrument::default()).collect();
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
                let mut bank = (*self.sample_bank).clone();
                let mut sample_errors = Vec::new();
                for entry in &song_file.sample_refs {
                    if entry.slot >= bank.samples.len() {
                        continue;
                    }
                    let sample_path = resolve_relative(load_dir, &entry.sample_ref.path);
                    match bank.load(entry.slot, &sample_path) {
                        Ok(()) => {
                            if let Some(ref mut sample) = bank.samples[entry.slot] {
                                sample.base_note = entry.sample_ref.base_note;
                                sample.trim_start = entry.sample_ref.trim_start;
                                sample.trim_end = entry.sample_ref.trim_end;
                                sample.loop_enabled = entry.sample_ref.loop_enabled;
                                sample.loop_start = entry.sample_ref.loop_start;
                                sample.loop_end = entry.sample_ref.loop_end;
                            }
                        }
                        Err(e) => {
                            sample_errors.push(format!("{}: {}", entry.sample_ref.name, e));
                        }
                    }
                }
                self.sample_bank = Arc::new(bank);
                if let Some(ref mut audio) = self.audio {
                    audio.set_sample_bank(Arc::clone(&self.sample_bank));
                }

                self.file_path = Some(path.to_path_buf());
                self.dirty = false;
                if sample_errors.is_empty() {
                    Ok(format!("Loaded: {}", path.display()))
                } else {
                    Ok(format!(
                        "Loaded (missing samples: {}): {}",
                        sample_errors.join(", "),
                        path.display()
                    ))
                }
            }
            Err(e) => Err(format!("Load failed: {}", e)),
        }
    }

    /// Import a MIDI file. Returns Ok(song) or Err(message).
    pub fn import_midi_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        match crate::midi_file::import_midi(path) {
            Ok(song) => {
                self.channels = default_channel_configs(song.channels);
                self.solo_channel = None;
                self.song = song;
                Ok(format!("Imported: {}", path.display()))
            }
            Err(e) => Err(format!("Import failed: {}", e)),
        }
    }
}

