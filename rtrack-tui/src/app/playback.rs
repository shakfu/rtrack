use rtrack_core::midi::MidiInputEvent;

use super::{App, Mode};

impl App {
    // -- Playback --

    pub fn toggle_playback(&mut self) {
        self.core.toggle_playback(self.edit_order, self.cursor_row);
    }

    pub fn toggle_recording(&mut self) {
        self.core.recording = !self.core.recording;
        if self.core.recording {
            self.status_message = Some("Recording armed".to_string());
        } else {
            self.status_message = Some("Recording off".to_string());
        }
    }

    pub fn play_from_start(&mut self) {
        self.edit_order = 0;
        self.cursor_row = 0;
        self.core.play(0, 0);
    }

    pub fn play(&mut self) {
        self.core.play(self.edit_order, self.cursor_row);
    }

    pub fn tick_playback(&mut self) {
        if self.core.tick_playback() {
            // Update follow-playback cursor from engine position (post-advance)
            if self.follow_playback && self.core.engine.tick == 1 {
                self.cursor_row = self.core.engine.row;
                self.edit_order = self.core.engine.order;
            }
        }
    }

    // -- MIDI input handling --

    /// Process incoming MIDI note events from external controllers
    pub fn poll_midi_input(&mut self) {
        while let Some(event) = self.core.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    pub(crate) fn handle_midi_input(&mut self, event: MidiInputEvent) {
        match event {
            MidiInputEvent::NoteOn { channel: _, note, velocity } => {
                let ch = self.cursor_channel;
                let midi_ch = self.core.midi_channel_for(ch);

                // Punch-in recording: playing + recording + Insert mode
                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    let order = self.core.engine.order;
                    let row = self.core.engine.row;
                    self.core.record_note_at(order, row, ch, note, velocity);
                    self.core.preview_note(midi_ch, note, velocity);
                    return;
                }

                // Step recording: Insert mode + stopped
                if self.mode == Mode::Insert && !self.core.playing {
                    self.push_undo();
                    self.core.preview_note(midi_ch, note, velocity);
                    let order = self.current_order_position();
                    let row = self.cursor_row;
                    if self.core.record_note_at(order, row, ch, note, velocity) {
                        self.move_cursor_down(self.edit_step);
                    }
                    return;
                }

                // All other modes: preview only
                self.core.preview_note(midi_ch, note, velocity);
            }
            MidiInputEvent::NoteOff { channel: _, note } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_note_off(midi_ch, note);

                // Punch-in: record note-off at engine position
                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    self.core.record_note_off_at(
                        self.core.engine.order,
                        self.core.engine.row,
                        self.cursor_channel,
                    );
                }
            }
            MidiInputEvent::CC { channel: _, controller, value } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                if let Some(msg) = self.core.handle_midi_cc(controller, value, midi_ch) {
                    self.status_message = Some(msg);
                }
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_pitch_bend(midi_ch, value);
            }
            MidiInputEvent::ProgramChange { channel: _, program } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_program_change(midi_ch, program);
            }
            MidiInputEvent::ChannelPressure { channel: _, pressure } => {
                self.core.apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::PolyPressure { channel: _, note: _, pressure } => {
                self.core.apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::Clock => {
                self.core.handle_external_clock();
            }
            MidiInputEvent::Start => {
                self.core.handle_external_start();
            }
            MidiInputEvent::Stop => {
                self.core.handle_external_stop();
            }
            MidiInputEvent::Continue => {
                self.core.handle_external_continue();
            }
        }
    }
}
