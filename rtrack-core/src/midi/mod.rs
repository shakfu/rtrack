use std::sync::mpsc;

use anyhow::{Context, Result};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, MidiOutputPort};

#[cfg(unix)]
use midir::os::unix::VirtualInput;
#[cfg(unix)]
use midir::os::unix::VirtualOutput;

const MIDI_NOTE_ON: u8 = 0x90;
const MIDI_NOTE_OFF: u8 = 0x80;
const MIDI_CC: u8 = 0xB0;
const MIDI_PROGRAM_CHANGE: u8 = 0xC0;
const MIDI_CHANNEL_PRESSURE: u8 = 0xD0;
const MIDI_POLY_PRESSURE: u8 = 0xA0;
const MIDI_PITCH_BEND: u8 = 0xE0;
const MIDI_CLOCK: u8 = 0xF8;
const MIDI_START: u8 = 0xFA;
const MIDI_STOP: u8 = 0xFC;

/// The name rtrack's own virtual output port is listed under.
pub const VIRTUAL_PORT_NAME: &str = "RTRACK_MIDI (virtual)";

/// Where a MIDI message goes: the session port, or a port an instrument
/// names (`docs/dev/midi-ports.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortId {
    Session,
    Named(usize),
}

/// One output connection and what has been sent on it.
#[derive(Default)]
struct Port {
    connection: Option<MidiOutputConnection>,
    /// The notes sounding on each channel, so they can be stopped: one, or
    /// a chord's.
    active_notes: [Vec<u8>; 16],
    /// The program last sent on each channel.
    programs: [Option<u8>; 16],
    /// Every message sent, connected or not, for tests to inspect.
    #[cfg(test)]
    sent: Vec<Vec<u8>>,
}

impl Port {
    fn open(connection: MidiOutputConnection) -> Self {
        Self {
            connection: Some(connection),
            ..Self::default()
        }
    }
}

/// A port an instrument names, open or absent.
struct NamedPort {
    name: String,
    port: Port,
    /// Present for routing; in tests, without a connection.
    present: bool,
}

pub struct MidiEngine {
    session: Port,
    named: Vec<NamedPort>,
    /// Name of the currently connected port
    pub port_name: Option<String>,
    /// Whether to send MIDI clock messages
    pub clock_enabled: bool,
    /// Count of consecutive send failures (reset on success)
    pub send_error_count: u32,
    /// Last error message
    pub last_error: Option<String>,
    /// Whether the user chose the open port, rather than rtrack opening one
    /// at startup. `External` instruments send only to a chosen port.
    pub user_selected: bool,
}

impl MidiEngine {
    pub fn new() -> Self {
        Self {
            session: Port::default(),
            named: Vec::new(),
            port_name: None,
            clock_enabled: false,
            send_error_count: 0,
            last_error: None,
            user_selected: false,
        }
    }

    /// Messages sent on the session port, for tests.
    #[cfg(test)]
    pub(crate) fn sent(&self) -> &[Vec<u8>] {
        &self.session.sent
    }

    /// Messages sent on named port `name`, for tests.
    #[cfg(test)]
    pub(crate) fn sent_to(&self, name: &str) -> &[Vec<u8>] {
        self.named
            .iter()
            .find(|n| n.name == name)
            .map_or(&[], |n| &n.port.sent)
    }

    /// Register `name` as present without opening it, for tests.
    #[cfg(test)]
    pub(crate) fn add_test_port(&mut self, name: &str) {
        let id = self.named_port(name);
        if let PortId::Named(i) = id {
            self.named[i].present = true;
        }
    }
}

impl Default for MidiEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// The listed port `wanted` names: the same name, or else the same with a
/// trailing ALSA `" <client>:<port>"` removed from both, since ALSA numbers
/// a device when it appears and the numbers can change after a replug.
pub fn match_port_name(wanted: &str, listed: &[String]) -> Option<usize> {
    fn stripped(name: &str) -> &str {
        let Some((head, tail)) = name.rsplit_once(' ') else {
            return name;
        };
        let numbered = tail.split_once(':').is_some_and(|(c, p)| {
            !c.is_empty()
                && !p.is_empty()
                && c.bytes().all(|b| b.is_ascii_digit())
                && p.bytes().all(|b| b.is_ascii_digit())
        });
        if numbered {
            head
        } else {
            name
        }
    }
    listed.iter().position(|p| p == wanted).or_else(|| {
        let wanted = stripped(wanted);
        listed.iter().position(|p| stripped(p) == wanted)
    })
}

impl MidiEngine {
    pub fn disconnect(&mut self) {
        let _ = self.all_notes_off_on(PortId::Session);
        self.session = Port::default();
        self.port_name = None;
        self.user_selected = false;
    }

    /// Open the port the user last selected, or else rtrack's virtual port,
    /// or else the first port. Only the first counts as selected.
    pub fn open_at_startup(&mut self) {
        if let Some(name) = crate::config::load_midi_out_port() {
            if self.connect_by_name(&name).is_ok() {
                self.user_selected = true;
                return;
            }
        }
        if self.create_virtual_port().is_err() {
            let _ = self.connect_first_available();
        }
    }

    /// Connect to the output port listed as `name` (see
    /// [`match_port_name`]), or create the virtual port for
    /// [`VIRTUAL_PORT_NAME`].
    pub fn connect_by_name(&mut self, name: &str) -> Result<()> {
        if name == VIRTUAL_PORT_NAME {
            return self.create_virtual_port();
        }
        let index = match_port_name(name, &Self::list_ports()?)
            .with_context(|| format!("MIDI port '{name}' is not present"))?;
        self.connect(index)
    }

    /// List available MIDI output ports
    pub fn list_ports() -> Result<Vec<String>> {
        let output =
            MidiOutput::new("rtrack-list").context("Failed to create MIDI output for listing")?;
        let ports = output.ports();
        let names: Vec<String> = ports
            .iter()
            .map(|p| {
                output
                    .port_name(p)
                    .unwrap_or_else(|_| "Unknown".to_string())
            })
            .collect();
        Ok(names)
    }

    /// Open listed port `index`, returning its name and connection.
    fn open_port(port_index: usize) -> Result<(String, MidiOutputConnection)> {
        let output = MidiOutput::new("rtrack").context("Failed to create MIDI output")?;
        let ports = output.ports();
        let port: &MidiOutputPort = ports.get(port_index).context("Invalid MIDI port index")?;
        let name = output
            .port_name(port)
            .unwrap_or_else(|_| "Unknown".to_string());
        let conn = output
            .connect(port, "rtrack-out")
            .map_err(|e| anyhow::anyhow!("Failed to connect to MIDI port '{}': {}", name, e))?;
        Ok((name, conn))
    }

    /// Connect to a MIDI output port by index
    pub fn connect(&mut self, port_index: usize) -> Result<()> {
        self.disconnect();
        let (name, conn) = Self::open_port(port_index)?;
        self.port_name = Some(name);
        self.session = Port::open(conn);
        Ok(())
    }

    /// Create a virtual MIDI output port that other applications can connect to.
    /// The port will appear as "RTRACK_MIDI" in DAWs and other MIDI software.
    /// Only supported on macOS and Linux.
    #[cfg(unix)]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        self.disconnect();
        let output = MidiOutput::new("rtrack").context("Failed to create MIDI output")?;
        let conn = output
            .create_virtual("RTRACK_MIDI")
            .map_err(|e| anyhow::anyhow!("Failed to create virtual MIDI port: {}", e))?;
        self.port_name = Some(VIRTUAL_PORT_NAME.to_string());
        self.session = Port::open(conn);
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn create_virtual_port(&mut self) -> Result<()> {
        anyhow::bail!("Virtual MIDI ports are not supported on this platform")
    }

    /// Try to connect to the first available port, or do nothing if none available
    pub fn connect_first_available(&mut self) -> Result<bool> {
        self.disconnect();
        if Self::list_ports()?.is_empty() {
            return Ok(false);
        }
        let (name, conn) = Self::open_port(0)?;
        self.port_name = Some(name);
        self.session = Port::open(conn);
        Ok(true)
    }

    pub fn is_connected(&self) -> bool {
        self.session.connection.is_some()
    }

    // -- Named ports --

    /// The id of the port an instrument names, opening it on first use.
    /// An absent port keeps its id and is retried by
    /// [`Self::retry_named_ports`].
    pub fn named_port(&mut self, name: &str) -> PortId {
        if let Some(i) = self.named.iter().position(|n| n.name == name) {
            return PortId::Named(i);
        }
        self.named.push(NamedPort {
            name: name.to_string(),
            port: Port::default(),
            present: false,
        });
        let i = self.named.len() - 1;
        self.try_open_named(i, None);
        PortId::Named(i)
    }

    /// Whether a message to `id` reaches a port: the session port when the
    /// user selected it, a named port when it is open.
    pub fn is_present(&self, id: PortId) -> bool {
        match id {
            PortId::Session => self.user_selected,
            PortId::Named(i) => self.named.get(i).is_some_and(|n| n.present),
        }
    }

    /// The name a port id stands for; the session port's is its open port.
    pub fn name_of(&self, id: PortId) -> Option<&str> {
        match id {
            PortId::Session => self.port_name.as_deref(),
            PortId::Named(i) => self.named.get(i).map(|n| n.name.as_str()),
        }
    }

    /// Try again to open every named port that is absent.
    pub fn retry_named_ports(&mut self) {
        if self.named.iter().all(|n| n.present) {
            return;
        }
        let listed = Self::list_ports().ok();
        for i in 0..self.named.len() {
            if !self.named[i].present {
                self.try_open_named(i, listed.as_deref());
            }
        }
    }

    fn try_open_named(&mut self, i: usize, listed: Option<&[String]>) {
        let name = &self.named[i].name;
        // rtrack's own virtual port is the session's; see the design doc.
        if name == VIRTUAL_PORT_NAME {
            return;
        }
        let owned;
        let listed = match listed {
            Some(l) => l,
            None => match Self::list_ports() {
                Ok(l) => {
                    owned = l;
                    &owned
                }
                Err(_) => return,
            },
        };
        let Some(index) = match_port_name(name, listed) else {
            return;
        };
        if let Ok((_, conn)) = Self::open_port(index) {
            self.named[i].port = Port::open(conn);
            self.named[i].present = true;
        }
    }

    fn port_mut(&mut self, id: PortId) -> Option<&mut Port> {
        match id {
            PortId::Session => Some(&mut self.session),
            PortId::Named(i) => self.named.get_mut(i).map(|n| &mut n.port),
        }
    }

    // -- Sending --

    /// Send a MIDI note-on message
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.note_on_to(PortId::Session, channel, note, velocity)
    }

    pub fn note_on_to(&mut self, port: PortId, channel: u8, note: u8, velocity: u8) -> Result<()> {
        // Turn off any previously active notes on this channel
        self.channel_note_off_to(port, channel)?;
        self.add_note_to(port, channel, note, velocity)
    }

    /// Start a note beside the channel's sounding ones: a chord note.
    pub fn add_note_to(&mut self, port: PortId, channel: u8, note: u8, velocity: u8) -> Result<()> {
        let ch = channel & 0x0F;
        if let Some(p) = self.port_mut(port) {
            p.active_notes[ch as usize].push(note);
        }
        self.send(port, &[MIDI_NOTE_ON | ch, note, velocity])
    }

    /// Send a MIDI note-off message
    #[allow(dead_code)]
    pub fn note_off(&mut self, channel: u8, note: u8) -> Result<()> {
        self.note_off_to(PortId::Session, channel, note)
    }

    pub fn note_off_to(&mut self, port: PortId, channel: u8, note: u8) -> Result<()> {
        let ch = channel & 0x0F;
        if let Some(p) = self.port_mut(port) {
            p.active_notes[ch as usize].retain(|&n| n != note);
        }
        self.send(port, &[MIDI_NOTE_OFF | ch, note, 0])
    }

    /// Send note-off for the active note on a channel
    pub fn channel_note_off(&mut self, channel: u8) -> Result<()> {
        self.channel_note_off_to(PortId::Session, channel)
    }

    pub fn channel_note_off_to(&mut self, port: PortId, channel: u8) -> Result<()> {
        let ch = channel & 0x0F;
        let notes = self
            .port_mut(port)
            .map(|p| std::mem::take(&mut p.active_notes[ch as usize]))
            .unwrap_or_default();
        for note in notes {
            self.send(port, &[MIDI_NOTE_OFF | ch, note, 0])?;
        }
        Ok(())
    }

    /// Send a MIDI Control Change message
    pub fn send_cc(&mut self, channel: u8, controller: u8, value: u8) -> Result<()> {
        self.send_cc_to(PortId::Session, channel, controller, value)
    }

    pub fn send_cc_to(
        &mut self,
        port: PortId,
        channel: u8,
        controller: u8,
        value: u8,
    ) -> Result<()> {
        let ch = channel & 0x0F;
        self.send(port, &[MIDI_CC | ch, controller & 0x7F, value & 0x7F])
    }

    /// Send a MIDI Program Change message
    pub fn program_change(&mut self, channel: u8, program: u8) -> Result<()> {
        self.program_change_to(PortId::Session, channel, program)
    }

    pub fn program_change_to(&mut self, port: PortId, channel: u8, program: u8) -> Result<()> {
        let ch = channel & 0x0F;
        let program = program & 0x7F;
        if let Some(p) = self.port_mut(port) {
            p.programs[ch as usize] = Some(program);
        }
        self.send(port, &[MIDI_PROGRAM_CHANGE | ch, program])
    }

    /// Send a program change only if the channel's last one differs.
    pub fn program_if_changed(&mut self, port: PortId, channel: u8, program: u8) -> Result<()> {
        let last = self
            .port_mut(port)
            .and_then(|p| p.programs[(channel & 0x0F) as usize]);
        if last == Some(program & 0x7F) {
            return Ok(());
        }
        self.program_change_to(port, channel, program)
    }

    /// Send a MIDI Pitch Bend message. Value is 14-bit: 0x2000 = center (no bend).
    pub fn pitch_bend(&mut self, channel: u8, value: u16) -> Result<()> {
        self.pitch_bend_to(PortId::Session, channel, value)
    }

    pub fn pitch_bend_to(&mut self, port: PortId, channel: u8, value: u16) -> Result<()> {
        let ch = channel & 0x0F;
        let lsb = (value & 0x7F) as u8;
        let msb = ((value >> 7) & 0x7F) as u8;
        self.send(port, &[MIDI_PITCH_BEND | ch, lsb, msb])
    }

    /// Send a transport message to every open port, if clock is on.
    fn send_transport(&mut self, message: u8) -> Result<()> {
        if !self.clock_enabled {
            return Ok(());
        }
        let mut result = self.send(PortId::Session, &[message]);
        for i in 0..self.named.len() {
            if self.named[i].present {
                result = result.and(self.send(PortId::Named(i), &[message]));
            }
        }
        result
    }

    /// Send MIDI clock tick (0xF8) - should be sent 24 times per beat
    pub fn send_clock(&mut self) -> Result<()> {
        self.send_transport(MIDI_CLOCK)
    }

    /// Send MIDI Start message (0xFA)
    pub fn send_start(&mut self) -> Result<()> {
        self.send_transport(MIDI_START)
    }

    /// Send MIDI Stop message (0xFC)
    pub fn send_stop(&mut self) -> Result<()> {
        self.send_transport(MIDI_STOP)
    }

    /// Kill all active notes (panic), on every port.
    pub fn all_notes_off(&mut self) -> Result<()> {
        let mut result = self.all_notes_off_on(PortId::Session);
        for i in 0..self.named.len() {
            result = result.and(self.all_notes_off_on(PortId::Named(i)));
        }
        result
    }

    fn all_notes_off_on(&mut self, port: PortId) -> Result<()> {
        for ch in 0..16u8 {
            self.channel_note_off_to(port, ch)?;
        }
        Ok(())
    }

    fn send(&mut self, port: PortId, message: &[u8]) -> Result<()> {
        let Some(p) = self.port_mut(port) else {
            return Ok(());
        };
        #[cfg(test)]
        p.sent.push(message.to_vec());
        let Some(conn) = &mut p.connection else {
            return Ok(());
        };
        match conn.send(message) {
            Ok(()) => {
                self.send_error_count = 0;
                Ok(())
            }
            Err(e) => {
                self.send_error_count += 1;
                self.last_error = Some(format!("MIDI: {}", e));
                Err(anyhow::anyhow!("MIDI send error: {}", e))
            }
        }
    }
}

/// Represents a MIDI message received from an external controller.
#[derive(Debug, Clone, Copy)]
pub enum MidiInputEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    CC {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    /// Channel pressure (mono aftertouch): 0-127
    ChannelPressure {
        channel: u8,
        pressure: u8,
    },
    /// Polyphonic key pressure (poly aftertouch): per-note 0-127
    PolyPressure {
        channel: u8,
        note: u8,
        pressure: u8,
    },
    Clock,
    Start,
    Stop,
    Continue,
}

/// Handles MIDI input from external controllers
pub struct MidiInputEngine {
    _connection: Option<MidiInputConnection<()>>,
    receiver: mpsc::Receiver<MidiInputEvent>,
    pub port_name: Option<String>,
}

impl Default for MidiInputEngine {
    fn default() -> Self {
        Self::new()
    }
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

        let input = MidiInput::new("rtrack-input").context("Failed to create MIDI input")?;
        let conn = input
            .create_virtual(
                "RTRACK_MIDI_IN",
                move |_stamp, message, _| {
                    parse_midi_input(message, &tx);
                },
                (),
            )
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

        let input = MidiInput::new("rtrack-input").context("Failed to create MIDI input")?;
        let ports = input.ports();
        let port = ports
            .get(port_index)
            .context("Invalid MIDI input port index")?
            .clone();
        let name = input
            .port_name(&port)
            .unwrap_or_else(|_| "Unknown".to_string());

        let conn = input
            .connect(
                &port,
                "rtrack-in",
                move |_stamp, message, _| {
                    parse_midi_input(message, &tx);
                },
                (),
            )
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

/// Parse a raw MIDI message and send the corresponding event through the channel.
fn parse_midi_input(message: &[u8], tx: &mpsc::Sender<MidiInputEvent>) {
    if message.is_empty() {
        return;
    }

    // System realtime messages (single byte, no channel)
    match message[0] {
        MIDI_CLOCK => {
            let _ = tx.send(MidiInputEvent::Clock);
            return;
        }
        MIDI_START => {
            let _ = tx.send(MidiInputEvent::Start);
            return;
        }
        MIDI_STOP => {
            let _ = tx.send(MidiInputEvent::Stop);
            return;
        }
        0xFB => {
            let _ = tx.send(MidiInputEvent::Continue);
            return;
        }
        _ => {}
    }

    if message.len() < 2 {
        return;
    }

    let status = message[0] & 0xF0;
    let ch = message[0] & 0x0F;

    match status {
        MIDI_NOTE_ON if message.len() >= 3 => {
            if message[2] > 0 {
                let _ = tx.send(MidiInputEvent::NoteOn {
                    channel: ch,
                    note: message[1],
                    velocity: message[2],
                });
            } else {
                let _ = tx.send(MidiInputEvent::NoteOff {
                    channel: ch,
                    note: message[1],
                });
            }
        }
        MIDI_NOTE_OFF if message.len() >= 3 => {
            let _ = tx.send(MidiInputEvent::NoteOff {
                channel: ch,
                note: message[1],
            });
        }
        MIDI_CC if message.len() >= 3 => {
            let _ = tx.send(MidiInputEvent::CC {
                channel: ch,
                controller: message[1],
                value: message[2],
            });
        }
        MIDI_PROGRAM_CHANGE => {
            let _ = tx.send(MidiInputEvent::ProgramChange {
                channel: ch,
                program: message[1],
            });
        }
        MIDI_CHANNEL_PRESSURE => {
            let _ = tx.send(MidiInputEvent::ChannelPressure {
                channel: ch,
                pressure: message[1],
            });
        }
        MIDI_POLY_PRESSURE if message.len() >= 3 => {
            let _ = tx.send(MidiInputEvent::PolyPressure {
                channel: ch,
                note: message[1],
                pressure: message[2],
            });
        }
        MIDI_PITCH_BEND if message.len() >= 3 => {
            let value = ((message[2] as u16 & 0x7F) << 7) | (message[1] as u16 & 0x7F);
            let _ = tx.send(MidiInputEvent::PitchBend { channel: ch, value });
        }
        _ => {}
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
        assert!(engine.session.active_notes.iter().all(|n| n.is_empty()));
    }

    #[test]
    fn port_names_match_exactly_then_without_alsa_numbers() {
        let listed: Vec<String> = [
            "Midi Through:Midi Through Port-0 14:0",
            "USB MIDI:USB MIDI MIDI 1 24:0",
            "IAC Driver Bus 1",
            "2- Scarlett 2i4 USB",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(match_port_name("IAC Driver Bus 1", &listed), Some(2));
        assert_eq!(match_port_name("2- Scarlett 2i4 USB", &listed), Some(3));
        // Replugged: the client number moved from 20 to 24.
        assert_eq!(
            match_port_name("USB MIDI:USB MIDI MIDI 1 20:0", &listed),
            Some(1)
        );
        assert_eq!(
            match_port_name("USB MIDI:USB MIDI MIDI 2 20:0", &listed),
            None
        );
        // A name ending in a word with a colon is not numbered.
        assert_eq!(match_port_name("Synth a:b", &["Synth".to_string()]), None);
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
        assert_eq!(engine.session.active_notes[0], [60]);

        engine.note_off(0, 60).unwrap();
        assert!(engine.session.active_notes[0].is_empty());
    }

    #[test]
    fn test_channel_note_off() {
        let mut engine = MidiEngine::new();
        engine.note_on(2, 64, 80).unwrap();
        assert_eq!(engine.session.active_notes[2], [64]);

        engine.channel_note_off(2).unwrap();
        assert!(engine.session.active_notes[2].is_empty());

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
    fn test_parse_midi_note_on() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0x90, 60, 100], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            } => {}
            other => panic!("Expected NoteOn, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_note_on_vel0_is_off() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0x91, 60, 0], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::NoteOff {
                channel: 1,
                note: 60,
            } => {}
            other => panic!("Expected NoteOff, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_note_off() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0x82, 64, 50], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::NoteOff {
                channel: 2,
                note: 64,
            } => {}
            other => panic!("Expected NoteOff, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_cc() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0xB0, 7, 100], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::CC {
                channel: 0,
                controller: 7,
                value: 100,
            } => {}
            other => panic!("Expected CC, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_program_change() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0xC3, 42], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::ProgramChange {
                channel: 3,
                program: 42,
            } => {}
            other => panic!("Expected ProgramChange, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_pitch_bend() {
        let (tx, rx) = mpsc::channel();
        // Center: LSB=0, MSB=64 -> 0x2000
        parse_midi_input(&[0xE0, 0x00, 0x40], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::PitchBend {
                channel: 0,
                value: 0x2000,
            } => {}
            other => panic!("Expected PitchBend center, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_clock() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0xF8], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::Clock => {}
            other => panic!("Expected Clock, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_start_stop_continue() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[0xFA], &tx);
        assert!(matches!(rx.try_recv().unwrap(), MidiInputEvent::Start));

        parse_midi_input(&[0xFC], &tx);
        assert!(matches!(rx.try_recv().unwrap(), MidiInputEvent::Stop));

        parse_midi_input(&[0xFB], &tx);
        assert!(matches!(rx.try_recv().unwrap(), MidiInputEvent::Continue));
    }

    #[test]
    fn test_parse_midi_channel_pressure() {
        let (tx, rx) = mpsc::channel();
        // Channel 2, pressure 100
        parse_midi_input(&[0xD2, 100], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::ChannelPressure {
                channel: 2,
                pressure: 100,
            } => {}
            other => panic!("Expected ChannelPressure, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_poly_pressure() {
        let (tx, rx) = mpsc::channel();
        // Channel 1, note 60, pressure 80
        parse_midi_input(&[0xA1, 60, 80], &tx);
        match rx.try_recv().unwrap() {
            MidiInputEvent::PolyPressure {
                channel: 1,
                note: 60,
                pressure: 80,
            } => {}
            other => panic!("Expected PolyPressure, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_midi_empty_message() {
        let (tx, rx) = mpsc::channel();
        parse_midi_input(&[], &tx);
        assert!(
            rx.try_recv().is_err(),
            "Empty message should not produce an event"
        );
    }

    #[test]
    fn test_midi_input_engine_new() {
        let engine = MidiInputEngine::new();
        assert!(!engine.is_connected());
        assert!(engine.poll().is_none());
    }

    #[test]
    fn test_list_ports() {
        // Two different "no MIDI" situations exist and only one of them is an
        // empty list. A machine whose MIDI backend works but has nothing
        // plugged in returns `Ok(vec![])`; a headless container with no ALSA
        // sequencer cannot open the backend at all and returns midir's
        // `InitError`. Both are valid here -- what must hold is that the call
        // returns rather than panicking, and that any names it does report are
        // usable.
        match MidiEngine::list_ports() {
            Ok(names) => assert!(
                names.iter().all(|n| !n.is_empty()),
                "port names must be non-empty: {names:?}"
            ),
            Err(e) => assert!(
                e.downcast_ref::<midir::InitError>().is_some(),
                "listing ports failed for a reason other than an unavailable \
                 MIDI backend: {e:?}"
            ),
        }
    }
}
