# G6: modulation

Lets an instrument's filter, drive, pitch and volume move during a note: by pattern commands (`CUT`, `RES`, `DRV`, deferred from G5), by tables, and by two mod slots per instrument. Closes G6 in [design-gaps.md](design-gaps.md) and G5 decision 3 in [g5-instrument-fx.md](g5-instrument-fx.md). Code refers to `rtrack-core/src` as of 2026-10-25.

Status: step 1 implemented 2026-10-25, with the proposal for decisions 1-7: engine per tick, step 1 first, values last until changed, lenient load with version 3. Step 2 (mod slots) is designed below and not built.

Step 2 implemented in the `dev` merge: two slots per instrument, envelope or LFO, adding to volume, pitch, cutoff or drive. The tracking source is not built.

## Current state

- **Pitch and volume already move per tick.** The engine computes `PVB`, `ARP`, slides and table steps once per tick and emits `PitchBend` and `VolumeChange`. Live playback schedules them as `AudioCommand::ChannelPitch` / `ChannelVolume` at the tick's frame; render applies the same events; MIDI export writes them as bends and CC 7. A tick is one MIDI clock: 24 per beat, 20.8 ms at 120 bpm.
- **Timbre is fixed per note.** A synth voice reads `filter_cutoff`, `filter_resonance`, `filter_env` and `drive` from the `SynthParams` it got at note-on, and computes its filter coefficients every frame (`Voice::tick`). A sample voice builds `VoiceFx` (drive, `SvfCoeffs`) once at note-on (`sample/playback.rs`). Nothing changes either during a note.
- **Instrument data does not reach the engine.** `TrackerEngine` sees only `Song`. Tables live in `Song.tables`, keyed by instrument number ("instrument N plays table N"), for that reason. `ChannelState.active_instrument` records which instrument a channel plays.
- **Previews bypass the engine**, so tables do not play on previews.
- **SoundFont** (rustysynth 1.3.6) handles CC 1, 7, 10, 11, 64, 91, 93 and RPN/NRPN; no filter CC (`synthesizer.rs`, `0xB0` arm). External MIDI has no audio here.
- **Unknown effect names fail the load.** `Fx`'s `Deserialize` returns an error for a name it does not know, and `CellRepr` propagates it (`tracker/fx.rs`, `tracker/pattern.rs`). rtrack 0.2.0 therefore cannot open a file that uses any command added after it.

## Where modulation is computed

**A. In the engine, per tick.** Mod slots and the new commands are evaluated with the existing effects. A new event carries the channel's timbre offsets; voices apply them, smoothed.
- Live, render and MIDI export match by construction, as every engine effect does.
- Tempo sync needs nothing: rates and times are counted in ticks.
- `CUT`/`RES`/`DRV` and tables use the same path; one path serves all three sources.
- Pitch and volume destinations also reach SoundFont and external MIDI, through bends and CC 7.
- Updates arrive at 24 per beat. Voices must smooth them, or a resonant sweep steps audibly (inferred; not measured here).
- Previews hear no modulation, as they hear no tables.

**B. In the voice, per frame.** Slots travel with the note-on, as `SynthParams` and `SampleFx` do (`Route`).
- Smooth at any rate; each releasing voice keeps its own modulation.
- Previews hear it.
- The voice needs the tempo, sent at note-on; a tempo change during a note does not reach it.
- Two implementations, in `Voice` and `SampleVoice`.
- SoundFont, external MIDI and MIDI export get nothing.
- `CUT`/`RES`/`DRV` still need A's per-channel path, so B adds a second path rather than replacing it.

**C. Tables only (design-gaps option 3).** Build A's path and the three commands, no slots. A table row can then step the filter: 16 rows, up to 16 ticks each, looping.
- Smallest change. Tables already restart per note, loop, sync to tempo and play in render and export.
- An LFO becomes a hand-drawn table of up to 16 steps; smoothing in the voice turns the steps into ramps.
- One table per instrument, and its single effect per row is also needed for `ARP`, `PVB` and `Cnn`.

| | A | B | C |
|-|-|-|-|
| Live = render = export | yes | live = render | yes |
| Tempo sync | free | at note-on only | free |
| Reaches SF2 / external (pitch, volume) | yes | no | yes |
| Update rate | per tick, smoothed | per frame | per tick, smoothed |
| Previews modulate | no | yes | no |
| New code paths | 1 | 2 | 1 |
| Mod slots | yes | yes | no |

## Recommendation: A, built in two steps

A gives one path for commands, tables and slots, and keeps render and export equal to live, which every earlier gap has held to. B duplicates the voice code and still needs A's path.

Step 1 is C: the timbre path, the three commands and smoothing. Step 2 adds the slots on that path. After step 1, tables may cover enough that step 2 can wait (decision 2).

## Model

### Timbre path (step 1)

```rust
pub enum TrackerEvent {
    // ...
    /// The channel's offsets from its instrument's filter and drive.
    /// Emitted when they change.
    Timbre { channel: usize, timbre: Timbre },
}

#[derive(Clone, Copy, PartialEq, Default)]
pub struct Timbre {
    /// Octaves added to the cutoff.
    pub cutoff: f32,
    /// Added to resonance; the filter clamps the sum to 0..0.95.
    pub resonance: f32,
    /// Octaves the drive is scaled by; the voice clamps to 1..MAX_DRIVE.
    pub drive: f32,
}
```

- **Voices.** A synth voice multiplies its cutoff by `2^cutoff` and adds `resonance`, smoothed per frame with a one-pole of about 5 ms. A sample voice smooths the same way and rebuilds its `SvfCoeffs` every 16 frames while the smoothed value moves. Fundsp pad voices ignore it, as they ignore the filter now. A sample voice with no filter has nothing to move: `CUT` on it does nothing (decision 4).
- **Live.** `AudioCommand::ChannelTimbre { channel, timbre }`, scheduled at the tick's frame like `ChannelPitch`.
- **Render.** Applies `Timbre` to the synth and sample engines, as it applies `PitchBend`.
- **MIDI export, SF2, external.** Ignore `Timbre` (decision 5).

### Commands (step 1)

All three are relative to the instrument, so one value means the same on a synth (cutoff as a multiple of the note) and a sample (cutoff in Hz). `80` is the instrument's own value.

| Command | Effect |
|-|-|
| `CUTxx` | cutoff `(xx - 80) / 32` octaves: `00` is -4, `FF` +4 |
| `RESxx` | resonance `+ (xx - 80) / 128 * 0.95` |
| `DRVxx` | drive `* 2^((xx - 80) / 32)`, clamped to 1..`MAX_DRIVE` |

- A value persists on the channel until another command of its kind or a note with a different instrument (decision 3).
- Tables carry them in their effect column, which gives stepped filter sequences per instrument: the G7 use G5 deferred.
- `RND` varies them, as it varies other continuous commands.

### Mod slots (step 2)

```rust
pub struct ModSlot {
    pub source: ModSource,
    pub dest: ModDest,
    /// -128..127; full scale per destination below.
    pub amount: i8,
}

pub enum ModSource {
    /// 0 -> 1 -> 0. Times in ticks.
    Ahd { attack: u16, hold: u16, decay: u16 },
    /// Released at note-off. Sustain 0..255.
    Adsr { attack: u16, decay: u16, sustain: u8, release: u16 },
    /// -1..1, one cycle every `period` ticks; restarts at each note unless `free`.
    Lfo { shape: LfoShape, period: u16, free: bool },
    /// Static per note: (note - 60) / 60, or velocity / 127.
    Track(TrackSource),
}

pub enum ModDest { Pitch, Volume, Cutoff, Resonance, Drive }
pub enum LfoShape { Sine, Triangle, SawUp, SawDown, Square, Random }
pub enum TrackSource { Note, Velocity }
```

| Destination | Full scale (`amount` 127, source 1) | Reaches |
|-|-|-|
| `Pitch` | +8 semitones (1/16 per step, as `PVB`) | every output |
| `Volume` | x4 amplitude (1/32 octave of gain per step) | every output |
| `Cutoff` | +4 octaves | built-in synth, samples |
| `Resonance` | +0.95 | built-in synth, samples |
| `Drive` | x16 | built-in synth, samples |

- **Where they live.** `Song.mods: BTreeMap<u8, [Option<ModSlot>; 2]>`, keyed by instrument number like `Song.tables`, so the engine reads them without instrument data (decision 6).
- **Evaluation.** Each tick, per channel, the engine sums slot outputs with the command values per destination, adds pitch to the existing bend and scales volume on top of `state.volume` without storing it. A slot runs from the note-on of its instrument; `Adsr` releases at note-off; a new note restarts all but free LFOs. `Random` is drawn from the engine's seeded generator, so playback repeats, as `RND` does.
- **Editing.** An instrument editor section in both frontends; edits are undoable song edits.

### Format

- `CUT`, `RES` and `DRV` are new `FxCmd` names; `Song.mods` is a new optional field. 0.2.0 ignores the field but fails to load a file with an unknown effect name (decision 7).

## Decisions

1. **A, B or C.** Proposal: A.
2. **Scope.** Proposal: step 1 (timbre path, commands, smoothing) now; step 2 (slots) designed here and built after step 1 is used. Alternative: both in one change.
3. **Command persistence.** Proposal: a value lasts until the next command of its kind, or a note with a different instrument; stopping resets it.
   - Alternative: reset at every note, as `pitch_offset` does. Then a held filter setting needs a command on every note row.
4. **`CUT` on a sample with no filter.** Proposal: nothing happens; the editor shows the filter as off. Alternative: an implicit low-pass at 20 kHz, which would make every sample voice run a filter.
5. **Timbre in MIDI export and external MIDI.** Proposal: not sent. CC 74 (brightness) is the common mapping, but receivers differ and rustysynth ignores it. `Cnn` in a table already sends a CC sequence by hand.
6. **Where slots live.** Proposal: `Song.mods` by instrument number, beside tables. Cost: moving an instrument to another slot does not move its mods, as it does not move its table.
   - Alternative: on `Instrument`, passed to the engine through a setter like `set_channel_info`. Live, render and MIDI export each have to call it, and live has to call it again after every instrument edit.
7. **Old versions.** 0.2.0 parses a song in one `serde_json::from_str`, so an unknown effect fails the load before the version is read. It reports the error and opens nothing; no data is lost. No change here can alter 0.2.0.
   - Proposal, for 0.3.0 onward: unknown effect names load as empty with a repair note, and a song using a new command or mods is written as `FORMAT_VERSION` 3. A later build's file then opens with the newer-version notice, and the existing guard stops a save over it, which would drop the effects.
   - Alternative: keep strict parsing. Every future command then makes files unreadable on every older build.

## Plan

1. `Fx` tolerates unknown names (repair note) and `FORMAT_VERSION` 3 is written when needed; tests in `tests/legacy_format.rs`.
2. `Timbre`, `TrackerEvent::Timbre`, `AudioCommand::ChannelTimbre`; voice smoothing in `Voice` and `SampleVoice`. Tests: a `CUT` sweep renders smoother than an unsmoothed step (high-frequency energy), hostile values stay finite, an unmodulated note renders bit-identical to today.
3. `CUT`, `RES`, `DRV` in the engine, tables and `RND`; README effect table.
4. Step 2, if decision 2 says so: `Song.mods`, engine evaluation, editors, then README, CHANGELOG, design-gaps, TODO.

## Not in scope

- Slot-to-slot modulation (M8 has it, per design-gaps; not verified here).
- Pan as a destination: built-in voices are mono and pan is a track setting.
- Modulation on previews.
- FM index, pulse width and sub-oscillator as destinations. Each is one more `Timbre` field if wanted.
