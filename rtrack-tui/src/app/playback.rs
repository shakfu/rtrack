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
        self.core.tick_playback();
        // Follow the position the listener is hearing, not the position the
        // sequencer has run ahead to.
        if self.follow_playback {
            let (order, row) = self.core.playback_position();
            self.cursor_row = row;
            self.edit_order = order;
        }
    }

    // -- MIDI input handling --

    /// Process incoming MIDI note events from external controllers
    /// Drain MIDI input, committing any undo step the notes produced.
    ///
    /// Step recording writes into the pattern, so this is an editing entry
    /// point like `handle_key` and needs the same commit.
    pub fn poll_midi_input(&mut self) {
        self.poll_midi_input_inner();
        self.commit_undo();
    }

    fn poll_midi_input_inner(&mut self) {
        while let Some(event) = self.core.midi_input.poll() {
            self.handle_midi_input(event);
        }
    }

    pub(crate) fn handle_midi_input(&mut self, event: MidiInputEvent) {
        match event {
            MidiInputEvent::NoteOn {
                channel: _,
                note,
                velocity,
            } => {
                let ch = self.cursor_channel;

                // Punch-in recording: playing + recording + Insert mode
                if self.core.playing && self.core.recording && self.mode == Mode::Insert {
                    let order = self.core.engine.order;
                    let row = self.core.engine.row;
                    self.core.record_note_at(order, row, ch, note, velocity);
                    self.core
                        .preview_note_for_cell(order, row, ch, note, velocity);
                    return;
                }

                // Step recording: Insert mode + stopped
                if self.mode == Mode::Insert && !self.core.playing {
                    self.push_undo();
                    let order = self.current_order_position();
                    let row = self.cursor_row;
                    self.core
                        .preview_note_for_cell(order, row, ch, note, velocity);
                    if self.core.record_note_at(order, row, ch, note, velocity) {
                        self.move_cursor_down(self.edit_step);
                    }
                    return;
                }

                // All other modes: preview only, still through the
                // instrument the cursor's cell would use.
                let order = self.current_order_position();
                self.core
                    .preview_note_for_cell(order, self.cursor_row, ch, note, velocity);
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
            MidiInputEvent::CC {
                channel: _,
                controller,
                value,
            } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                if let Some(msg) = self.core.handle_midi_cc(controller, value, midi_ch) {
                    self.status_message = Some(msg);
                }
            }
            MidiInputEvent::PitchBend { channel: _, value } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_pitch_bend(midi_ch, value);
            }
            MidiInputEvent::ProgramChange {
                channel: _,
                program,
            } => {
                let midi_ch = self.core.midi_channel_for(self.cursor_channel);
                self.core.send_program_change(midi_ch, program);
            }
            MidiInputEvent::ChannelPressure {
                channel: _,
                pressure,
            } => {
                self.core
                    .apply_aftertouch_to_filter(self.cursor_channel, pressure);
            }
            MidiInputEvent::PolyPressure {
                channel: _,
                note: _,
                pressure,
            } => {
                self.core
                    .apply_aftertouch_to_filter(self.cursor_channel, pressure);
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
