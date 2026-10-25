# G12: chords

Lets one track play a chord: a `CHD` effect adds up to two notes to the row's note, and both effect columns together give up to five. Closes G12 option 2 in [design-gaps.md](design-gaps.md). Note columns (option 1) and the "Chord type" item under Ambitious in TODO.md stay out of scope. Code refers to `rtrack-core/src` as of 2026-10-25.

Status: implemented 2026-10-25, with the proposal for each decision. Decision 4 is implemented without the mark in the table editors: `CHD` in a table row is ignored, which README states.

## Current state

One note per track is assumed at every stage:

- **Engine.** `TrackerEvent::NoteOn` carries one `midi_note`; `ChannelState.note` is one note; `NoteOff { channel }` stops the track.
- **Built-in synth.** `note_on` and `note_on_with_params` call `note_off_all_channel` first, so a second note-on on a channel replaces the first.
- **Samples.** A pattern note uses `NewNoteAction::Cut`, which fades every voice on the channel.
- **SoundFont.** Render calls `note_off_all_channel` before each note-on (`sample/export.rs`).
- **MIDI out.** `MidiEngine` keeps one `active_notes` entry per channel per port, and `note_on_to` sends a note-off for it first.
- **MIDI export.** `active_notes` holds one note per tracker channel (`midi_file.rs`).
- **Channel-wide state.** Pitch bend, volume and timbre (G6) are per channel at every stage, so they would move all notes of a chord together.
- **Effect names.** `CHD` is reserved (`g4-g8-format.md`). Every hex key is taken, so a new command is typed by name with `=`.
- **Scale steps.** `Song::step_offset` turns a step count into semitones, by scale degree when the song has a scale and `scale_steps` is on (G10).

## Options

**A. `CHD` command.** `CHDxy` adds notes `x` and `y` steps above the row's note; `0` adds none. Steps count as `ARP`'s do: semitones, or scale degrees under G10. A second `CHD` in the other column adds two more.
- One new command; no change to cells, the grid, entry or MIDI import.
- The grid shows `CHD47`, not the notes.
- Pitch effects move the whole chord, since bend is per channel. `ARP` on a chord arpeggiates the chord's root position as a block, not its notes.

**B. Note columns (Renoise).** A cell holds up to N notes.
- Each note is visible and editable, and voices can lead independently.
- Changes `Cell`, both grids, cursor movement, entry, copy and paste, undo, MIDI import and the file format. design-gaps.md rates it large.

**C. Chord on the instrument.** An instrument field lists intervals, and every note it plays sounds them.
- No pattern data; a pad instrument always plays its chord.
- Instruments do not reach the engine (G6, G10), so render and live would expand chords in routing, and MIDI export, which reads only the song, would write single notes.

**D. Status quo.** One track per voice; `MAX_CHANNELS` is 16.

| | A | B | C | D |
|-|-|-|-|-|
| Chord per row | yes | yes | no, per instrument | across tracks |
| Notes visible in the grid | no | yes | no | yes |
| Live = render = export | yes | yes | export differs | yes |
| Size | one command, five backends | large | routing, three backends | none |

## Recommendation: A

A is the option design-gaps.md already recommended after G8, and G8 is done. It covers progressions and stabs from one track and plays the same in live, render and export, since the engine expands the chord. B remains possible later; A's data would convert to note columns, one note per column.

## Model (A)

```rust
pub enum TrackerEvent {
    NoteOn {
        channel: usize,
        midi_note: u8,
        /// Further notes sounding with `midi_note`, from `CHD`, already
        /// resolved through the scale. Unused slots are `None`.
        chord: [Option<u8>; 4],
        // ... velocity, instrument, sample_offset as now
    },
    // ...
}
```

- **Engine.** At note-on, each `CHD` among the row's columns adds `base + step_offset(base, x)` and the same for `y`, for nonzero `x` and `y`. A note outside 0-127, or equal to one already in the chord, is dropped. A `DEL` note keeps its chord. `PSL` with a note glides the whole chord, since no new note-on happens. `CHD` without a note does nothing. In a table row it is ignored, since a table runs after the note-on.
- **Built-in synth.** The note-on cuts the channel's voices once, then starts one voice per chord note. The `note_on*` functions gain a chord argument; the cut moves out of them.
- **Samples.** `Cut` once, then the extra notes as `Continue`, so they do not cut each other.
- **SoundFont.** One `note_off_all_channel`, then a note-on per chord note, live and in render.
- **MIDI out.** `active_notes` becomes up to five notes per channel per port; a channel note-off stops them all.
- **MIDI export.** `active_notes` per tracker channel becomes a list; note-off and the next note-on end every note.
- **Velocity.** Every chord note takes the row's velocity.
- **Voices.** A five-note chord takes five voices. The built-in synth's `MAX_VOICES` (32) and the sample engine's 32 voices bound it; five-note chords on all 16 tracks would need 80; past them, the quietest voice is stolen, as now.
- **Live scheduling.** One `AudioCommand` per note, all scheduled at the note-on's frame, so they start in the same callback.
- **Format.** `CHD` joins `CUT`/`RES`/`DRV` in `Song::format_version`, so a song using it is written as version 3.

## Decisions

1. **A, B, C or D.** Proposal: A.
2. **What a step counts.** Proposal: as `ARP` does, so `CHD24` in C major with Steps on plays C E G, and `CHD47` with Steps off plays the same. One rule for both commands.
   - Alternative: always semitones. Simpler to read; ignores G10's setting.
3. **`ARP` on a chord.** Proposal: as now, it bends the whole channel, so the chord moves as a block.
   - Alternative: cycle through the chord's notes, a strum. Needs per-voice pitch, which no backend has.
4. **Chords in tables.** Proposal: `CHD` in a table row is ignored, and the table editor accepts it but marks it. A chord changing mid-note would need a retrigger.
   - Alternative: a table row with `CHD` retriggers the note as a chord.
5. **Previews.** Previews bypass the engine, so a note typed on a `CHD` row previews as one note. Proposal: accept it; previews already skip tables and `CUT`.

## Plan

1. `FxCmd::Chord` (`CHD`), `NoteOn.chord`, engine expansion and the version-3 rule. Tests: `CHD47` gives three notes; two columns give five; a duplicate and an out-of-range note are dropped; `CHD24` in C major gives C E G; `DEL` keeps the chord; no note means no chord.
2. Backends in order: built-in synth, samples, SoundFont live and render, MIDI out, MIDI export. A test each: three voices or three note-ons sound, and a note-off stops all.
3. README effect table, CHANGELOG, design-gaps G12, TODO.

## Not in scope

- Note columns (option B) and per-voice pitch.
- Live MIDI chord capture: `TrackerCore::preview_note` holds one note (TODO.md, Chord type item).
- Importing chords from a MIDI file into `CHD`.
