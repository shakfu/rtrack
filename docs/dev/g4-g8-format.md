# G4 + G8: typed instruments and named effect columns

One `.rtrk` format change that closes G4 (sound-source split) and G8 (effect command space) from [design-gaps.md](design-gaps.md). Code refers to `rtrack-core/src` at `f1d1f87`.

Status: implemented 2026-10-12. Follow-ups are in `TODO.md`.

## Why one change

- Both rewrite serialized types: `Instrument`/`InstrumentDef` for G4, `Cell` for G8. One migration and one `FORMAT_VERSION` bump costs less than two.
- They meet at `Cxx`. Today the CC controller number sits in the instrument column (`engine/mod.rs`, tick-0 `EFFECT_MIDI_CC`). Fixing that needs the effect encoding (G8) and a decision on what the instrument column means (G4).

## Current state

Found while reading the code. Items marked (inferred) are from reading, not from a test.

1. **Every route sends MIDI.** All four branches of `TrackerCore::route_note_on` call `midi.note_on` (`core.rs:952`, `:985`, `:997`, and `send_note_on` at `:912`). design-gaps.md G4 says only the sample route does.
2. **`ChannelType::Midi` does not silence internal audio.** `route_note_on` never reads the channel type. A "MIDI" channel also plays the sample or synth (inferred).
3. **`ChannelType` gates default-instrument fill inconsistently.**
   - Playback fills it on `Synth` channels only (`TrackerEngine::resolve_instrument`, via `ChannelInfo.is_synth`).
   - Note entry fills it on `Synth` and `Sample` channels (`TrackerCore::track_default_instrument`).
   - Result: a cell with an empty instrument column on a `Sample` channel plays the default synth, not the track's sample (inferred).
4. **Live playback and offline render route differently.**
   - Instrument number set, no sample, no `synth_params`: live plays preset `SynthParams::from_patch(idx)` (route 3). Render plays the default voice (`sample/export.rs:308-311`).
   - Live sends to `midi_channel_for(channel)`. Render uses `channel & 0x0F`.
   - This breaks the "render and live produce the same output" constraint.
5. **`Instrument.midi_program` is never sent.** It is saved and passed to `ExportInstrument`, which ignores it. Only `Exx` sends a program change.
6. **One effect slot per cell.**
   - `6xx` delay and `9xx` offset cannot combine. A delayed note always starts at offset 0 (`engine/mod.rs`, "The row's effect column holds the 6xx").
   - MIDI import drops a CC or program change when the cell's effect is taken (`midi_file.rs:655`, `:662`).
7. **MIDI import of a CC on a note row changes the note's instrument.** The importer writes the controller number into `cell.instrument` (`midi_file.rs:658`). A CC7 on a note row makes that note play instrument 7 (inferred).
8. **`Fxx` is two commands in one.** Below `0x20` it sets speed; at `0x20` and above it sets BPM.

Items 1-4 and 7 are bugs today, whatever the format. This design closes items 1-5 and 7. Fixing them separately first is also an option.

## Instruments (G4)

### Model

```rust
pub struct Instrument {
    pub name: String,
    pub kind: InstrumentKind,
    pub pitch_bend_range: Option<f64>,
}

pub enum InstrumentKind {
    Midi { program: u8, channel: Option<u8>, output: MidiOutput },
    Synth(SynthParams),
    Sample { slot: usize },
}

pub enum MidiOutput {
    /// SF2 if loaded, else the built-in preset synth (`from_patch(program)`).
    Builtin,
    /// The session's selected MIDI port. Built-in sound is silent.
    External,
}
```

- **Every note plays in exactly one place.** `Synth` and `Sample` are internal only. `Midi` goes to `Builtin` or `External`, never both. This removes the MIDI doubling of current-state item 1.
- **Default table.** Slot `i` is `Midi { program: i, channel: None, output: Builtin }`. A fresh song makes sound without a SoundFont, a software synth or hardware. Sparse storage keeps skipping instruments equal to their default.
- **One built-in path.** Routes 3 (preset by instrument number) and 4 (no instrument) merge into `Builtin`. A note with no instrument and no track default plays instrument 0 if that is a loaded sample or a synth, as before. Otherwise it plays `Builtin` with the channel's current program. That program starts at 0 and `PRG` changes it. This matches today: `BuiltinSynth::note_on` plays `Patch::from_program(programs[channel])` (`audio/synth.rs:1247`).
- **Rerouting overrides the default.** `External` sends to the session's MIDI port only.
- **Fallback.** `External` sends to the port the user selected, in the session or the config. If none is selected, or the selected port is not present, it plays through `Builtin`, and the status line says so. This follows `AudioEngine::new_with_sf2_fallback`. The setting is not rewritten, so the routing comes back when the hardware reconnects.
- **The auto-created virtual port is not a selection.** rtrack opens `RTRACK_MIDI` at startup on Linux and macOS. It counts as the destination only once the user selects it, for example to drive a DAW. Otherwise an `External` instrument would send to it and be silent with nothing listening.
- **Why a selection, not detection.** "Nothing listening" cannot be detected portably. The virtual port makes `MidiEngine::is_connected()` always true on Linux and macOS. ALSA can report port subscribers; CoreMIDI has no portable equivalent (inferred, not checked). On Windows the first port is usually the OS's built-in GS synth.
- **Remaining silent case.** The user selects `RTRACK_MIDI` and nothing listens. The instrument header shows the port name, so this is visible.
- **No audio device.** If `AudioEngine` fails to start, `Builtin` is silent too, as today.
- `channel: None` means the track's `ChannelConfig.midi_channel`.
- `program` is sent before the first note on that channel, and again after an instrument change. `Instrument.midi_program` is never sent today (item 5).

### Routing

One function maps `(&Instrument, &SampleBank)` to a route enum. Live playback (`route_note_on`) and render (`sample/export.rs`) both call it. That removes current-state item 4 by construction.

### `ChannelType`

Removed.

- Default-instrument fill applies on every track, in the engine and in note entry. This closes item 3.
- The track header label shows the kind of the track's default instrument (`SMP`, `SYN`, `MID` for built-in, `EXT` for external).

### Decisions

1. **Session port or per-instrument port.** `MidiEngine` holds one connection.
   - Proposal: one session port, with `Builtin`/`External` chosen per instrument.
   - Later: `External` gains an optional port name, with one connection per port. Only the `External` variant changes, so no second format migration is needed if the field is optional from the start.
   - Port names depend on the machine, so a song moved to another computer falls back to `Builtin`.
2. **SoundFonts in render.** The renderer does not play SF2 (`sample/export.rs:16`). With a SoundFont loaded, `Builtin` would sound different live and in a render.
   - Proposal: add SF2 to the renderer. RustySynth renders offline.
   - Alternative: `Builtin` ignores SF2 and always uses the preset synth. That is simpler, but it drops the SoundFont as the built-in sound.
3. **Where `midi_channel` lives.** The proposal keeps it on `ChannelConfig` and adds an optional per-instrument override. The doc recommended moving it to the instrument. Keeping it gives `Cnn`, `PRG` and pitch bend a defined destination on a track with no MIDI instrument.

## Effects (G8)

### Model

```rust
pub struct Cell {
    pub note: Option<Note>,
    pub instrument: Option<u8>,
    pub volume: Option<u8>,
    pub fx: [Option<Fx>; 2],
}

pub struct Fx { pub cmd: FxCmd, pub param: u8 }

pub enum FxCmd {
    Arp, PitchUp, PitchDown, PitchSlide, Vibrato, VolSlide, Delay,
    Chance, Random, Offset, Jump, Break, Program, Ticks, Tempo,
    /// MIDI CC; the controller number is part of the command.
    Cc(u8),
}
```

`Cell` stays `Copy`. Its size changes by a few bytes; measure it in step 1. The undo budget counts `CellEdit` by `size_of`, so it adjusts without code changes.

### Names

Three characters, shown with a two-digit hex parameter: `VIB44`. Names follow M8 where it has one.

| Old | New | Notes |
|-|-|-|
| `0xy` | `ARP` | |
| `1xx` | `PUP` | |
| `2xx` | `PDN` | |
| `3xx` | `PSL` | M8 name for slide-to-note |
| `4xy` | `PVB` | M8 |
| `5xy` | `VSL` | |
| `6xx` | `DEL` | M8 |
| `7xx` | `CHA` | M8 |
| `8xx` | `RND` | M8 |
| `9xx` | `OFS` | |
| `Bxx` | `JMP` | |
| `Cxx` + controller in instrument column | `Cnn` | `nn` = controller in hex, e.g. `C07 40` |
| `Dxx` | `BRK` | |
| `Exx` | `PRG` | |
| `Fxx`, `xx < 0x20` | `TIC` | split by value on load |
| `Fxx`, `xx >= 0x20` | `TPO` | |

No name may match `C[0-9A-F]{2}`, which is reserved for `Cnn`. Names reserved for later gaps: `TBL` (G7), `GRV` (G9), `SCA` (G10), `CHD` (G12), `HOP` (G3).

### Two columns

- **Flow commands** (`JMP`, `BRK`, `TIC`, `TPO`) are read from both columns. If both columns set the same one, the right-hand column wins.
- **Tick-0 commands** (`Cnn`, `PRG`, `OFS`, `CHA`, `DEL`) apply from both columns. `DEL` plus `OFS` now works: the delayed note keeps its offset.
- **Continuous commands** (`ARP`, `PUP`, `PDN`, `PSL`, `PVB`, `VSL`) keep separate state per column in `ChannelState`. Pitch offsets from the two columns add. Volume slides add. If both columns hold `PSL`, the right-hand column wins, because there is one porta target.
- **`RND`** varies the last continuous command in its own column.
- **Display.** All tracks show one effect column, or two when the visible pattern uses the second or the view is toggled (see Work, 6c). One column keeps the TUI at 4 tracks per page (`CHANNELS_PER_PAGE`). A track is 18 characters wide with one column and 25 with two.
- **Entry.** Typing a hex digit in an empty effect field enters the old command (`4` gives `PVB`), so existing key habits still work. `=` opens a prompt to type the command by name.

### Decisions

4. **Two columns or three.** M8 has 3 per step. The proposal is 2: three would push a 4-track TUI page past 100 columns.
5. **Whether continuous commands combine.** Adding offsets lets `PVB` and `ARP` sound together. The alternative, where the right-hand column replaces the left, is simpler but makes the second column less useful.

## File format

- `FORMAT_VERSION` goes from 1 to 2. The writer emits version 2 only.
- The reader accepts versions 0, 1 and 2. Each type deserializes through a private raw struct that holds both the old and new fields, then converts.
  - Cells: `effect` and `effect_value` become `fx[0]`, using the table above. `Cxx` takes its controller from `instrument` and clears it.
  - Instruments, using today's routing order: `sample_index` gives `Sample`. Otherwise `synth_params` gives `Synth`. Otherwise `Midi { program: slot, output: Builtin }`. A saved `midi_program` becomes `program`.
  - Channels: drop `channel_type`. A `Midi` track's `default_instrument` becomes `None`, because it never filled before.
- **Lossy cases.** These go into `LoadReport.repairs`:
  - an instrument with both a sample and synth params: the synth params were never played;
  - a `midi_program` on a `Sample` or `Synth` instrument: it was never sent.
- **Sound changes for old songs.** These go in the CHANGELOG:
  - Notes no longer double to MIDI. External gear driven by an old song goes silent until its instruments are set to `External`.
  - With a SoundFont loaded, an empty instrument slot plays GM program `i` from the SoundFont, not preset synth `i`. Without a SoundFont, nothing changes.
  - A `Sample` instrument whose file is missing is silent. Today it falls through to preset `slot` (route 1 checks `sample_bank.get`). The load already reports the missing file.
  - An instrument with only `midi_program` set plays that program, not preset `slot`. The conversion reports each one. The bundled examples show this was the intent: "FM Bell" had program 5, which is `Patch::FmBell`, but played preset 2.
- **Newer files on older builds.** rtrack 0.1.3 ignores unknown fields. It would load a v2 file with every effect and instrument kind missing. It already sets `LoadReport.from_newer_version`, so the user is warned, but the data is lost if they save.
- **MIDI import** writes `Fx { cmd: Cc(controller), .. }` and `PRG` into the first free column. It stops writing `cell.instrument`, which closes item 7. **MIDI export** is driven by engine events, so only the engine changes.

## Work

1. **Done.** Types and conversions, not yet wired into the reader: `tracker/fx.rs` (`Fx`, `FxCmd`, `Fx::from_legacy`) and `InstrumentKind`/`MidiOutput` in `types.rs` (`InstrumentKind::from_legacy`). Fixtures `tests/fixtures/legacy/v0.rtrk` and `v1.rtrk` cover each `Fxx` range, `Cxx` with and without a controller, `A`, a `Midi` track, and every instrument shape (`tests/legacy_format.rs`). `MidiOutput::External` carries an optional `port` from the start, so per-instrument ports need no second migration.
2. Reader and writer, in two parts:
   - **2a, done.** `Cell.fx` and `FORMAT_VERSION` 2. The engine matches on `FxCmd` and reads `fx[0]` only. Both frontends show `fx[0]` at 5 characters and share `Cell::enter_effect_digit`. MIDI import fills the first free column.
   - **2b, done.** `Instrument.kind`, written as `"kind": {"type": ...}`. `InstrumentDef` reads the untyped fields and converts through `InstrumentKind::from_legacy`; its notes go into `LoadReport.repairs`. Routing keeps the old order and the MIDI doubling. The bundled examples are saved as version 2. The `Midi`-track default-instrument rule moves to step 6, where `ChannelType` is removed; until then the engine still gates fill by type.
3. **Done.** Engine: per-column effect state and a flow scan over both columns. `rtrack_core::routing::route` decides each note's source; live playback uses it, and notes no longer double to MIDI. Moved out:
   - `Builtin` through SF2 for an instrument note needs a frame-scheduled program change on the audio thread; it goes with step 4.
   - `External` sends on the track's MIDI channel. The per-instrument `channel` and the program change go with step 5, where note-offs follow the override.
4. **Done.** Render calls `routing::route` and plays SF2 when given one; `rtrack --render` loads `--sf2` or the configured font. Live `Preset` notes play SF2 through `AudioCommand::Sf2ProgramNoteOn`, which changes the program and starts the note in one scheduled step. Tests compare renders: a preset against the same preset as synth parameters, `External` against its preset, and a missing sample against silence. Not covered: SF2 output itself, since no SoundFont is in the repo. Still divergent: render maps tracker channel `n` to MIDI channel `n & 0x0F`, live uses `ChannelConfig.midi_channel`.
5. **Done.** MIDI out: `MidiEngine::user_selected` records whether the user chose the open port; `TrackerCore::select_midi_port` sets it and saves the name to `midi_out.json` (not `config.toml`, which the user edits). `MidiEngine::open_at_startup` reopens it. `External` falls back to `Builtin` without a selected port and reports it once through `TrackerCore::take_notice`. The program is sent on first use and on change; a channel override carries the note-off, bends and CCs, and a track's next note elsewhere stops the external one.
6. TUI and GUI, in three parts:
   - **6a, done.** MIDI instrument settings (output, program, channel) in both instrument editors.
   - **6b, done.** `ChannelType` removed. Every track fills its default instrument; a legacy `Midi` track's is dropped on load (`ChannelConfig::drop_legacy_type`). The header label comes from `TrackerCore::track_label`.
   - **6c, done.** Decided 2026-10-12: one width for all tracks rather than `ChannelConfig.fx_columns`, since per-track widths would ripple through both layouts and hit tests. The second column shows when the visible pattern uses it (`Pattern::uses_second_fx`) or when toggled on. Entry by name is a prompt (`=` on an effect field) parsed by `Fx::from_str`; letters cannot be name prefixes because hex keys `a`-`f` already enter old commands.
7. **Done.** `examples/*.rtrk` are version 2: generated ones through `cargo xtask regen-examples`, hand-written ones re-saved by rtrack's own writer.
8. **Done.** CHANGELOG entries for each step.

## Not in scope

- G5: moving filter and drive onto `InstrumentKind`. `Synth` already holds its own. `Sample` gains them in G5.
- New commands. Only names are reserved.
