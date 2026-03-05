use anyhow::{Context, Result};
use midir::{MidiOutput, MidiOutputConnection, MidiOutputPort};

#[cfg(unix)]
use midir::os::unix::VirtualOutput;

const MIDI_NOTE_ON: u8 = 0x90;
const MIDI_NOTE_OFF: u8 = 0x80;

pub struct MidiEngine {
    connection: Option<MidiOutputConnection>,
    /// Track which notes are currently sounding per channel so we can send note-off
    active_notes: [Option<u8>; 16],
    /// Name of the currently connected port
    pub port_name: Option<String>,
}

impl MidiEngine {
    pub fn new() -> Self {
        Self {
            connection: None,
            active_notes: [None; 16],
            port_name: None,
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
    fn test_list_ports() {
        // Should not panic even if no MIDI devices are present
        let result = MidiEngine::list_ports();
        assert!(result.is_ok());
    }
}
