use std::time::Instant;

use crate::constants::*;
use crate::engine::TrackerEvent;
use crate::midi::MidiInputEvent;
use crate::tracker::{Note, NoteValue};

use super::{App, ChannelType, ClockMode, Mode};

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

    pub fn play_from_start(&mut self) {
        self.edit_order = 0;
        self.cursor_row = 0;
        self.play();
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.engine.reset(&self.song, self.edit_order, self.cursor_row);
        self.sync_engine_channel_info();
        self.last_tick = Some(Instant::now());
        self.tick_accumulator = 0.0;
        self.clock_tick_accumulator = 0.0;
        self.playback_elapsed = 0.0;
        // Capture initial Link beat position for beat-timeline mode
        if self.link.is_enabled() {
            self.last_link_beat = self.link.beat_at_time_now();
            self.link.request_play();
        }
        let _ = self.midi.send_start();
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.last_tick = None;
        // Reset pitch bends to center before killing notes
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

    /// Sync tempo from Link peers if changed externally
    pub fn sync_link(&mut self) {
        if !self.link.is_enabled() {
            return;
        }

        if let Some(new_tempo) = self.link.poll_tempo_change() {
            let new_bpm = new_tempo.round() as u16;
            if new_bpm != self.song.bpm && new_bpm >= 32 && new_bpm <= 300 {
                self.song.bpm = new_bpm;
                self.engine.bpm = new_bpm;
            }
        }
    }

    /// Push channel audibility/volume info from App's channel configs into the engine.
    fn sync_engine_channel_info(&mut self) {
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

    pub fn tick_playback(&mut self) {
        if !self.playing {
            return;
        }

        // In external clock mode, timing comes from MIDI clock messages, not internal timer
        if self.clock_mode == ClockMode::ExternalMidi {
            return;
        }

        // Link beat-timeline mode: drive ticks from Link's beat position
        if self.link.is_enabled() {
            let beat = self.link.beat_at_time_now();
            // 24 ticks per beat (standard tracker convention)
            let link_ticks = beat * MIDI_CLOCKS_PER_BEAT;
            let last_ticks = self.last_link_beat * MIDI_CLOCKS_PER_BEAT;
            let delta_ticks = link_ticks - last_ticks;
            if delta_ticks > 0.0 {
                let spt = self.engine.seconds_per_tick(&self.song);
                let ticks_per_second = 1.0 / spt;
                // Convert Link tick delta to our tracker ticks
                let tracker_ticks = delta_ticks / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT / 60.0) * ticks_per_second;
                self.tick_accumulator += tracker_ticks * spt;
                self.playback_elapsed += delta_ticks / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT / 60.0);
            }
            self.last_link_beat = beat;

            let spt = self.engine.seconds_per_tick(&self.song);
            while self.tick_accumulator >= spt {
                self.tick_accumulator -= spt;
                self.process_tick();
            }
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_tick {
            let elapsed = now.duration_since(last).as_secs_f64();
            self.tick_accumulator += elapsed;
            self.playback_elapsed += elapsed;

            // Send MIDI clock: 24 ppqn (pulses per quarter note)
            if self.midi.clock_enabled {
                self.clock_tick_accumulator += elapsed;
                let clock_interval = 60.0 / (self.engine.bpm as f64 * MIDI_CLOCKS_PER_BEAT);
                while self.clock_tick_accumulator >= clock_interval {
                    self.clock_tick_accumulator -= clock_interval;
                    let _ = self.midi.send_clock();
                }
            }

            let spt = self.engine.seconds_per_tick(&self.song);
            while self.tick_accumulator >= spt {
                self.tick_accumulator -= spt;
                self.process_tick();
            }
        }
        self.last_tick = Some(now);
    }

    /// Process a single sub-tick by delegating to the engine and dispatching events.
    pub(crate) fn process_tick(&mut self) {
        self.engine.process_tick(&self.song);
        let events = self.engine.drain_events();
        self.dispatch_engine_events(events);
        // Update follow-playback cursor from engine position (post-advance)
        if self.follow_playback && self.engine.tick == 1 {
            // tick was just incremented from 0 to 1, meaning we just processed row advance
            self.cursor_row = self.engine.row;
            self.edit_order = self.engine.order;
        }
    }

    /// Translate engine events to MIDI/audio output.
    fn dispatch_engine_events(&mut self, events: Vec<TrackerEvent>) {
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

    /// Toggle between internal and external clock modes
    pub fn toggle_clock_mode(&mut self) {
        self.clock_mode = match self.clock_mode {
            ClockMode::Internal => {
                self.ext_clock_count = 0;
                ClockMode::ExternalMidi
            }
            ClockMode::ExternalMidi => ClockMode::Internal,
        };
    }

    /// Handle an incoming MIDI clock tick (0xF8, 24 ppqn).
    /// Maps MIDI clock ticks to tracker sub-ticks: every (24/speed) clock ticks = 1 tracker tick.
    pub(crate) fn handle_external_clock(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi || !self.playing {
            return;
        }

        self.ext_clock_count += 1;

        // 24 MIDI clock ticks = 1 beat. speed tracker ticks = 1 row.
        // clocks_per_tracker_tick = 24 / speed
        let clocks_per_tick = (24u32 / self.engine.speed as u32).max(1);
        if self.ext_clock_count >= clocks_per_tick {
            self.ext_clock_count = 0;
            self.process_tick();

            // Update elapsed time based on BPM (estimated)
            let spt = self.engine.seconds_per_tick(&self.song);
            self.playback_elapsed += spt;
        }
    }

    /// Handle external MIDI Start message (0xFA)
    pub(crate) fn handle_external_start(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi {
            return;
        }
        self.playing = true;
        self.engine.reset(&self.song, 0, 0);
        self.sync_engine_channel_info();
        self.ext_clock_count = 0;
        self.playback_elapsed = 0.0;
    }

    /// Handle external MIDI Stop message (0xFC)
    pub(crate) fn handle_external_stop(&mut self) {
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

    /// Handle external MIDI Continue message (0xFB)
    pub(crate) fn handle_external_continue(&mut self) {
        if self.clock_mode != ClockMode::ExternalMidi {
            return;
        }
        self.playing = true;
        self.ext_clock_count = 0;
    }

    // -- MIDI input handling --

    /// Process incoming MIDI note events from external controllers
    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    pub(crate) fn handle_midi_input(&mut self, event: MidiInputEvent) {
        match event {
            MidiInputEvent::NoteOn { channel: _, note, velocity } => {
                // Only enter notes in Insert mode when not playing
                if self.mode != Mode::Insert || self.playing {
                    let midi_ch = self.midi_channel_for(self.cursor_channel);
                    self.preview_note(midi_ch, note, velocity);
                    return;
                }

                let octave = note / SEMITONES_PER_OCTAVE;
                let note_index = note % SEMITONES_PER_OCTAVE;
                if let Some(note_val) = NoteValue::from_index(note_index) {
                    let tracker_note = Note::On { value: note_val, octave };
                    self.push_undo();
                    let midi_ch = self.midi_channel_for(self.cursor_channel);
                    self.preview_note(midi_ch, note, velocity);

                    let pattern_idx = self.song.order[self.current_order_position()];
                    let cell = self.song.patterns[pattern_idx].get_mut(self.cursor_row, self.cursor_channel);
                    cell.note = Some(tracker_note);
                    cell.volume = Some(velocity);
                    self.move_cursor_down(self.edit_step);
                }
            }
            MidiInputEvent::NoteOff { channel: _, note } => {
                // Forward note-off to MIDI output (thru)
                let midi_ch = self.midi_channel_for(self.cursor_channel);
                self.send_note_off(midi_ch, note);
            }
            MidiInputEvent::CC { channel: _, controller, value } => {
                // Forward CC to output (MIDI thru)
                let midi_ch = self.midi_channel_for(self.cursor_channel);
                self.send_cc(midi_ch, controller, value);
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                // Forward pitch bend to output (MIDI thru)
                let midi_ch = self.midi_channel_for(self.cursor_channel);
                self.send_pitch_bend(midi_ch, value);
            }
            MidiInputEvent::ProgramChange { channel: _, program } => {
                // Forward program change to output (MIDI thru)
                let midi_ch = self.midi_channel_for(self.cursor_channel);
                self.send_program_change(midi_ch, program);
            }
            MidiInputEvent::Clock => {
                self.handle_external_clock();
            }
            MidiInputEvent::Start => {
                self.handle_external_start();
            }
            MidiInputEvent::Stop => {
                self.handle_external_stop();
            }
            MidiInputEvent::Continue => {
                self.handle_external_continue();
            }
        }
    }
}
