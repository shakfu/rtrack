use std::sync::mpsc;

use anyhow::{Context, Result};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, MidiOutputPort};

#[cfg(unix)]
use midir::os::unix::VirtualOutput;
#[cfg(unix)]
use midir::os::unix::VirtualInput;

const MIDI_NOTE_ON: u8 = 0x90;
const MIDI_NOTE_OFF: u8 = 0x80;
const MIDI_CC: u8 = 0xB0;
const MIDI_PROGRAM_CHANGE: u8 = 0xC0;
const MIDI_PITCH_BEND: u8 = 0xE0;
const MIDI_CLOCK: u8 = 0xF8;
const MIDI_START: u8 = 0xFA;
const MIDI_STOP: u8 = 0xFC;

pub struct MidiEngine {
    connection: Option<MidiOutputConnection>,
    /// Track which notes are currently sounding per channel so we can send note-off
    active_notes: [Option<u8>; 16],
    /// Name of the currently connected port
    pub port_name: Option<String>,
    /// Whether to send MIDI clock messages
    pub clock_enabled: bool,
}

impl MidiEngine {
    pub fn new() -> Self {
        Self {
            connection: None,
            active_notes: [None; 16],
            port_name: None,
            clock_enabled: false,
        }
    }

    pub fn disconnect(&mut self) {
        let _ = self.all_notes_off();
        self.connection = None;
        self.port_name = None;
    }

    /// List available MIDI output ports
    pub fn list_ports() -> Result<Vec<String>> {
        let output = MidiOutput::new("rtrack-list")
            .context("Failed to create MIDI output for listing")?;
        let ports = output.ports();
        let names: Vec<String> = ports
            .iter()
            .map(|p| output.port_name(p).unwrap_or_else(|_| "Unknown".to_string()))
            .collect();
        Ok(names)
    }

    /// Connect to a MIDI output port by index
    pub fn connect(&mut self, port_index: usize) -> Result<()> {
        self.disconnect();
        let output = MidiOutput::new("rtrack")
            .context("Failed to create MIDI output")?;
        let ports = output.ports();
        let port: &MidiOutputPort = ports
            .get(port_index)
            .context("Invalid MIDI port index")?;
        let name = output
            .port_name(port)
            .unwrap_or_else(|_| "Unknown".to_string());

        let conn = output
            .connect(port, "rtrack-out")
            .map_err(|e| anyhow::anyhow!("Failed to connect to MIDI port '{}': {}", name, e))?;

        self.port_name = Some(name);
        self.connection = Some(conn);
        Ok(())
    }

    /// Create a virtual MIDI output port that other applications can connect to.
    /// The port will appear as "RTRACK_MIDI" in DAWs and other MIDI software.
    /// Only supported on macOS and Linux.
    #[cfg(unix)]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        self.disconnect();
        let output = MidiOutput::new("rtrack")
            .context("Failed to create MIDI output")?;
        let conn = output
            .create_virtual("RTRACK_MIDI")
            .map_err(|e| anyhow::anyhow!("Failed to create virtual MIDI port: {}", e))?;
        self.port_name = Some("RTRACK_MIDI (virtual)".to_string());
        self.connection = Some(conn);
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        anyhow::bail!("Virtual MIDI ports are not supported on this platform")
    }

    /// Try to connect to the first available port, or do nothing if none available
    pub fn connect_first_available(&mut self) -> Result<bool> {
        self.disconnect();
        let output = MidiOutput::new("rtrack")
            .context("Failed to create MIDI output")?;
        let ports = output.ports();
        if ports.is_empty() {
            return Ok(false);
        }
        let port = &ports[0];
        let name = output
            .port_name(port)
            .unwrap_or_else(|_| "Unknown".to_string());
        let conn = output
            .connect(port, "rtrack-out")
            .map_err(|e| anyhow::anyhow!("Failed to connect: {}", e))?;
        self.port_name = Some(name);
        self.connection = Some(conn);
        Ok(true)
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Send a MIDI note-on message
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        let ch = channel & 0x0F;
        // Turn off any previously active note on this channel
        if let Some(prev_note) = self.active_notes[ch as usize] {
            self.send(&[MIDI_NOTE_OFF | ch, prev_note, 0])?;
        }
        self.active_notes[ch as usize] = Some(note);
        self.send(&[MIDI_NOTE_ON | ch, note, velocity])
    }

    /// Send a MIDI note-off message
    #[allow(dead_code)]
    pub fn note_off(&mut self, channel: u8, note: u8) -> Result<()> {
        let ch = channel & 0x0F;
        self.active_notes[ch as usize] = None;
        self.send(&[MIDI_NOTE_OFF | ch, note, 0])
    }

    /// Send note-off for the active note on a channel
    pub fn channel_note_off(&mut self, channel: u8) -> Result<()> {
        let ch = (channel & 0x0F) as usize;
        if let Some(note) = self.active_notes[ch] {
            self.active_notes[ch] = None;
            self.send(&[MIDI_NOTE_OFF | channel & 0x0F, note, 0])?;
        }
        Ok(())
    }

    /// Send a MIDI Control Change message
    pub fn send_cc(&mut self, channel: u8, controller: u8, value: u8) -> Result<()> {
        let ch = channel & 0x0F;
        self.send(&[MIDI_CC | ch, controller & 0x7F, value & 0x7F])
    }

    /// Send a MIDI Program Change message
    pub fn program_change(&mut self, channel: u8, program: u8) -> Result<()> {
        let ch = channel & 0x0F;
        self.send(&[MIDI_PROGRAM_CHANGE | ch, program & 0x7F])
    }

    /// Send a MIDI Pitch Bend message. Value is 14-bit: 0x2000 = center (no bend).
    pub fn pitch_bend(&mut self, channel: u8, value: u16) -> Result<()> {
        let ch = channel & 0x0F;
        let lsb = (value & 0x7F) as u8;
        let msb = ((value >> 7) & 0x7F) as u8;
        self.send(&[MIDI_PITCH_BEND | ch, lsb, msb])
    }

    /// Send MIDI clock tick (0xF8) - should be sent 24 times per beat
    pub fn send_clock(&mut self) -> Result<()> {
        if self.clock_enabled {
            self.send(&[MIDI_CLOCK])
        } else {
            Ok(())
        }
    }

    /// Send MIDI Start message (0xFA)
    pub fn send_start(&mut self) -> Result<()> {
        if self.clock_enabled {
            self.send(&[MIDI_START])
        } else {
            Ok(())
        }
    }

    /// Send MIDI Stop message (0xFC)
    pub fn send_stop(&mut self) -> Result<()> {
        if self.clock_enabled {
            self.send(&[MIDI_STOP])
        } else {
            Ok(())
        }
    }

    /// Kill all active notes (panic)
    pub fn all_notes_off(&mut self) -> Result<()> {
        for ch in 0..16u8 {
            if let Some(note) = self.active_notes[ch as usize] {
                self.send(&[MIDI_NOTE_OFF | ch, note, 0])?;
                self.active_notes[ch as usize] = None;
            }
        }
        Ok(())
    }

    fn send(&mut self, message: &[u8]) -> Result<()> {
        if let Some(conn) = &mut self.connection {
            conn.send(message)
                .map_err(|e| anyhow::anyhow!("MIDI send error: {}", e))?;
        }
        Ok(())
    }
}

/// Represents a MIDI note-on message received from an external controller
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct MidiInputEvent {
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
}

/// Handles MIDI input from external controllers
pub struct MidiInputEngine {
    _connection: Option<MidiInputConnection<()>>,
    receiver: mpsc::Receiver<MidiInputEvent>,
    pub port_name: Option<String>,
}

impl MidiInputEngine {
    pub fn new() -> Self {
        let (_tx, receiver) = mpsc::channel();
        Self {
            _connection: None,
            receiver,
            port_name: None,
        }
    }

    /// List available MIDI input ports
    #[allow(dead_code)]
    pub fn list_ports() -> Result<Vec<String>> {
        let input = MidiInput::new("rtrack-input-list")
            .context("Failed to create MIDI input for listing")?;
        let ports = input.ports();
        let names: Vec<String> = ports
            .iter()
            .map(|p| input.port_name(p).unwrap_or_else(|_| "Unknown".to_string()))
            .collect();
        Ok(names)
    }

    /// Create a virtual MIDI input port for receiving notes
    #[cfg(unix)]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        self.disconnect();
        let (tx, rx) = mpsc::channel();
        self.receiver = rx;

        let input = MidiInput::new("rtrack-input")
            .context("Failed to create MIDI input")?;
        let conn = input
            .create_virtual("RTRACK_MIDI_IN", move |_stamp, message, _| {
                if message.len() >= 3 {
                    let status = message[0] & 0xF0;
                    if status == MIDI_NOTE_ON && message[2] > 0 {
                        let _ = tx.send(MidiInputEvent {
                            channel: message[0] & 0x0F,
                            note: message[1],
                            velocity: message[2],
                        });
                    }
                }
            }, ())
            .map_err(|e| anyhow::anyhow!("Failed to create virtual MIDI input: {}", e))?;

        self.port_name = Some("RTRACK_MIDI_IN (virtual)".to_string());
        self._connection = Some(conn);
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        anyhow::bail!("Virtual MIDI input ports are not supported on this platform")
    }

    /// Connect to a MIDI input port by index
    #[allow(dead_code)]
    pub fn connect(&mut self, port_index: usize) -> Result<()> {
        self.disconnect();
        let (tx, rx) = mpsc::channel();
        self.receiver = rx;

        let input = MidiInput::new("rtrack-input")
            .context("Failed to create MIDI input")?;
        let ports = input.ports();
        let port = ports
            .get(port_index)
            .context("Invalid MIDI input port index")?
            .clone();
        let name = input
            .port_name(&port)
            .unwrap_or_else(|_| "Unknown".to_string());

        let conn = input
            .connect(&port, "rtrack-in", move |_stamp, message, _| {
                if message.len() >= 3 {
                    let status = message[0] & 0xF0;
                    if status == MIDI_NOTE_ON && message[2] > 0 {
                        let _ = tx.send(MidiInputEvent {
                            channel: message[0] & 0x0F,
                            note: message[1],
                            velocity: message[2],
                        });
                    }
                }
            }, ())
            .map_err(|e| anyhow::anyhow!("Failed to connect to MIDI input '{}': {}", name, e))?;

        self.port_name = Some(name);
        self._connection = Some(conn);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self._connection = None;
        self.port_name = None;
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self._connection.is_some()
    }

    /// Poll for incoming MIDI note events (non-blocking)
    pub fn poll(&self) -> Option<MidiInputEvent> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for MidiEngine {
    fn drop(&mut self) {
        let _ = self.all_notes_off();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_engine_new() {
        let engine = MidiEngine::new();
        assert!(!engine.is_connected());
        assert!(engine.active_notes.iter().all(|n| n.is_none()));
    }

    #[test]
    fn test_note_on_off_without_connection() {
        // When not connected, note_on/off should succeed silently
        let mut engine = MidiEngine::new();
        assert!(engine.note_on(0, 60, 100).is_ok());
        assert!(engine.note_off(0, 60).is_ok());
        assert!(engine.all_notes_off().is_ok());
    }

    #[test]
    fn test_active_note_tracking() {
        let mut engine = MidiEngine::new();
        engine.note_on(0, 60, 100).unwrap();
        assert_eq!(engine.active_notes[0], Some(60));

        engine.note_off(0, 60).unwrap();
        assert_eq!(engine.active_notes[0], None);
    }

    #[test]
    fn test_channel_note_off() {
        let mut engine = MidiEngine::new();
        engine.note_on(2, 64, 80).unwrap();
        assert_eq!(engine.active_notes[2], Some(64));

        engine.channel_note_off(2).unwrap();
        assert_eq!(engine.active_notes[2], None);

        // Calling again should be fine
        engine.channel_note_off(2).unwrap();
    }

    #[test]
    fn test_send_cc_without_connection() {
        let mut engine = MidiEngine::new();
        assert!(engine.send_cc(0, 7, 100).is_ok());
    }

    #[test]
    fn test_program_change_without_connection() {
        let mut engine = MidiEngine::new();
        assert!(engine.program_change(0, 42).is_ok());
    }

    #[test]
    fn test_midi_clock_messages() {
        let mut engine = MidiEngine::new();
        // Clock disabled by default
        assert!(!engine.clock_enabled);
        assert!(engine.send_clock().is_ok());
        assert!(engine.send_start().is_ok());
        assert!(engine.send_stop().is_ok());

        // Enable clock
        engine.clock_enabled = true;
        assert!(engine.send_clock().is_ok());
        assert!(engine.send_start().is_ok());
        assert!(engine.send_stop().is_ok());
    }

    #[test]
    fn test_midi_input_engine_new() {
        let engine = MidiInputEngine::new();
        assert!(!engine.is_connected());
        assert!(engine.poll().is_none());
    }

    #[test]
    fn test_list_ports() {
        // Should not panic even if no MIDI devices are present
        let result = MidiEngine::list_ports();
        assert!(result.is_ok());
    }
}
