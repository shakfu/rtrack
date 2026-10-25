# Per-instrument MIDI ports

Lets an `External` MIDI instrument send to a port of its own instead of the session's one port, so two hardware synths can play from one song. Closes the G4 + G8 follow-up in TODO.md; `MidiOutput::External { port }` has carried the field since format version 2 (`g4-g8-format.md` decision 1). Code refers to `rtrack-core/src` as of 2026-10-25.

Status: implemented 2026-10-25, with the proposal for each decision: a port per instrument, built in while absent, clock to every open port, exact then number-stripped matching. Named ports also open at song load and lazily on an instrument's first note after an edit, which costs one port listing.

## Current state

- **One connection.** `MidiEngine` holds one `MidiOutputConnection`, its `port_name`, and `user_selected` (`midi/mod.rs`). Every MIDI message goes there: notes, bends, CCs, programs, clock, start and stop.
- **External routing.** `Route::External { program, channel }` sends to that port only when `user_selected`; otherwise it plays as `Preset` and `take_notice` reports it once (`TrackerCore::route_note_on`). `port` is read from files and ignored.
- **Follow-up messages.** `external_channel[track]` remembers the MIDI channel of a track's last external note, so its note-off, bends and CCs follow it there. `sent_program[channel]` records the last program per channel, so a program change goes out only when it differs.
- **The chosen port** is saved by name in `midi_out.json` and reopened by exact name at startup (`MidiEngine::open_at_startup`).
- **ALSA port names carry numbers.** midir 0.10.4 names an ALSA port `"<client>:<port> <c>:<p>"`, for example `"USB MIDI:USB MIDI MIDI 1 20:0"` (`backend/alsa/mod.rs`, `get_port_name`). The client number is assigned when the device appears, so it can change after a replug or reboot (inferred from ALSA's allocation; not tested here). An exact-name match then fails, today for `midi_out.json` and in future for a port saved in a song.
- **Tests** read `MidiEngine.sent`, a log of every message, with no port attached.

## Options

**A. A port name per instrument.** `External { port: Some(name) }` sends to the port with that name; `None` keeps today's session port. `MidiEngine` keeps one connection per named port in use.
- Uses the field already in the format; 0.2.0 reads it and sends to the session port, so no version change.
- A song carries machine-specific names. On another machine the port is absent and the instrument falls back (decision 2).

**B. Logical outputs.** A song names outputs `A`-`D`; each machine maps them to real ports in its config, beside `midi_out.json`. An instrument picks an output.
- A song moves between machines and plays once each output is mapped there.
- One more layer to explain and a mapping dialog in both frontends. The format field would hold `"A"`, which 0.2.0 ignores as it ignores any name.

**C. Defer.** One external port per session, as now. A DAW or a MIDI router (ALSA `aconnect`, macOS IAC, loopMIDI on Windows) can split channels to devices.
- No code. Covers the case by channel, since an instrument can already pick its MIDI channel.

| | A | B | C |
|-|-|-|-|
| Two devices from one song | yes | yes | through a router |
| Song plays on another machine | falls back | once mapped | unchanged |
| Format change | none | none (field reused) | none |
| New UI | port picker | port picker + mapping dialog | none |

## Recommendation: A

A is what the reserved field was designed for, and its failure on another machine is a fallback with a notice, the same rule `External` follows today without a selected port. B solves portability at the cost of a second concept; it can be added later by reading a one-letter `port` as an output. C is a fair answer if no one has asked for two devices; this doc assumes someone has.

## Model (A)

```rust
pub struct MidiEngine {
    connection: Option<MidiOutputConnection>,   // the session port, as now
    // ...
    /// Connections to ports instruments name, by the name they use.
    /// `None` while the port is absent.
    named: Vec<NamedPort>,
}

struct NamedPort {
    name: String,
    connection: Option<MidiOutputConnection>,
    active_notes: [Option<u8>; 16],
}

/// Where a message goes: the session port or a named one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PortId { Session, Named(usize) }
```

- **Sending.** Every send function takes a `PortId`. `note_on(port, channel, note, velocity)` and so on; the session-port functions keep their names and call them with `PortId::Session`, so frontends do not change.
- **Follow-up messages.** `external_channel[track]` becomes `Option<(PortId, u8)>`, so a track's bends, CCs and note-off follow its last external note to the port as well as the channel. `sent_program` is kept per `(PortId, channel)`.
- **Opening.** Named ports open when a song loads and when an instrument's port is set. Absent ones are retried when playback starts: one `list_ports` call, not a poll.
- **Matching.** A name matches a listed port exactly, or, failing that, with a trailing `" <c>:<p>"` removed from both. The same rule applies to `midi_out.json`, which fixes the session port's reopen after a replug.
- **Panic and stop.** All-notes-off, stop and the start/stop messages go to every open port.
- **Editing.** TUI instrument list (F7): Ctrl+O steps Builtin -> session port -> each listed port -> Builtin. GUI MIDI panel: a combo box with "Built in", "Selected port" and each listed port. Undoable, as the other MIDI instrument keys are.
- **Tests.** `sent` records `(PortId, message)`. Name matching is a pure function, tested on ALSA-, CoreMIDI- and Windows-shaped names.

## Decisions

1. **A, B or C.** Proposal: A.
2. **An absent named port.** Proposal: play as `Builtin`, report once per port through `take_notice`, and keep the name, so the routing returns when the device does. This matches the session rule.
   - Alternative: send to the session port. That keeps the music on hardware, but may play it on a different device with different sounds.
3. **Clock.** Proposal: with clock output on, send clock to every open port, since a hardware arpeggiator or sequencer on any of them needs it.
   - Alternative: the session port only, as now. Cheaper, and a device that should not follow tempo is unaffected.
4. **Name matching.** Proposal: exact, then numbers stripped (above).
   - Alternative: exact only. Predictable, but a replugged ALSA device stops matching.
5. **Virtual port.** rtrack's `RTRACK_MIDI` virtual port is one port. Proposal: an instrument cannot name it; it is reachable as the session port. Naming it would need a second virtual port per name.

## Plan

1. `MidiEngine`: `PortId`, `named`, send functions by port, name matching, `sent` by port. Tests without hardware.
2. Core: routing to `port`, follow-up messages and programs by `(PortId, channel)`, fallback notice, open on load and retry on play.
3. Both editors; README MIDI section, CHANGELOG, g4-g8 doc, TODO.

## Not in scope

- Logical outputs (option B).
- MIDI input per instrument.
- Per-port clock settings.
