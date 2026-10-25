# G10: scale at playback

Make `ARP` and table pitch steps count scale degrees when the song has a scale, so arpeggios and tables stay in key. Closes G10 option 1 in [design-gaps.md](design-gaps.md). Per-channel scales (option 2) and microtonal offsets (option 3) stay out of scope. Code refers to `rtrack-core/src` as of 2026-10-25.

Status: implemented 2026-10-25, with the proposal for each decision: degrees (A), `scale_steps` off when missing, no opt-out.

## Current state

- `Song.scale: Option<ScaleSetting>` snaps keyboard entry (`Song::snap_entry`) and makes Shift+Up/Down move by degree. `theory.rs` states the rule: the engine, loader, MIDI input and paste stay unquantised.
- A phrase link's transpose already counts degrees when the song has a scale (`Song::playback_cell` -> `Cell::transpose_note` -> `ScaleSetting::transpose`). This is the only scale-aware playback path.
- `ARP xy` adds `x` and `y` semitones by pitch bend, cycling every 3 ticks (`engine/mod.rs`, tick > 0 loop).
- A table row's `transpose` adds semitones by pitch bend, on tick 0 and later ticks (`engine/mod.rs`, two sites). `[0, 4, 7]` looping equals `ARP47`, and a test pins this (`an_arpeggio_table_bends_exactly_as_arp_does`).
- `RND` varies the last continuous effect's parameter, so a randomised `ARP` can land anywhere chromatically.
- `PUP`, `PDN` and `PVB` move pitch continuously. `PSL` slides to a written note.
- Live playback, render and MIDI export all run `TrackerEngine`, so an engine change reaches all three. Previews bypass the engine.
- No bundled example sets a scale.

## What changes in sound

In C major, on a C:

| | semitones (today) | degrees (proposal) |
|-|-|-|
| `ARP47` | C E G | C G C' |
| `ARP24` | C D E | C E G |
| table `[0, 2, 4]` | C D E | C E G |

A scale with fewer notes makes each degree wider: in C major pentatonic, `ARP12` plays C D E, and `ARP24` plays C E A. `chromatic` gives today's semitones exactly.

## Options

**A. Degrees.** With a scale, `ARP` and table steps count scale degrees. The offset is `scale.transpose(base, n) - base` in semitones, sent as the pitch bend it is today.
- Matches link transpose and Shift+Up/Down, so the song has one meaning for a step.
- Every value is in key, including `RND` results.
- Changes the sound of any existing song that has both a scale and `ARP` or a table step.

**B. Snap the result.** Steps stay semitones; the sounding pitch `base + n` snaps to the scale.
- `ARP47` in C major is unchanged on C. On D (Dorian position) it becomes D F A: the off-scale F# (4) snaps down to F on the tie.
- Lossy: two parameters can give one pitch, and ties resolve downward by `snap_to_mask`'s rule, not by intent.
- Still changes existing songs, wherever a step landed off the scale.

**C. Explicit degree commands.** Keep `ARP` and table steps in semitones. Add a degree arpeggio command, for example `ARD`, and a per-table "degrees" flag.
- No existing song changes, and no mode depends on a song setting.
- Two commands for one job, and one name from a namespace that `G6` (`CUT`/`RES`/`DRV`) also draws on.
- Link transpose already counts degrees without a separate command, so C is inconsistent with G2.

| | A | B | C |
|-|-|-|-|
| Every step in key | yes | yes | only `ARD` / flagged tables |
| Same rule as link transpose | yes | no | no |
| Old songs unchanged | with decision 2 | with decision 2 | yes |
| Lossy | no | yes | no |
| New commands | 0 | 0 | 1, plus a table field |

## Recommendation: A

A gives one meaning to "a step" across link transpose, Shift+Up/Down, `ARP` and tables. B depends on tie-breaking. C keeps semitone `ARP` reachable in a scaled song, but decision 2 below provides that through a song setting at lower cost.

## Model (A)

```rust
pub struct Song {
    // ...
    /// `ARP` and table steps count degrees of `scale`; false counts
    /// semitones. Ignored without a scale.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scale_steps: bool,
}
```

- **Engine.** One helper, `fn step_offset(song, base: u8, steps: i16) -> f64`, returns `steps` without a scale or with `scale_steps` off, and `scale.transpose(base, steps) - base` otherwise. The `ARP` branch and both table sites call it. Written notes are never snapped, so step 0 of an arpeggio plays the written note even off the scale.
- **Off-scale base.** `ScaleSetting::transpose` snaps before it steps, so steps 1 and 2 of an arpeggio on an off-scale note land in the scale. This is what link transpose does today.
- **Out of range.** A step past MIDI 0-127 returns the note unchanged (offset 0), as `ScaleSetting::transpose` already does.
- **Bend range.** Degree offsets are wider than the same number in semitones: `ARPFF` in a heptatonic scale is up to 26 semitones. The built-in synth and SF2 take any offset. An external synth clips at its bend range, already true of `ARP47` at the default range of 2 (`DEFAULT_PITCH_BEND_RANGE`). No change.
- **Editing.** TUI song settings: a "Steps" row, `semitones` / `degrees`. GUI song settings: a checkbox "ARP and table steps follow the scale", disabled without a scale; the TUI row stays editable, as the setting has no effect until a scale is set. Undoable as other song settings are.

## Decisions

1. **A, B or C.** Proposal: A.

2. **Old songs.** A song saved by 0.2.0 with a scale and `ARP` would change sound on load.
   - Proposal: `scale_steps` defaults to false when missing, so 0.2.0 files play as before. `Song::new` sets it true, so new songs get degrees. Files written with false stay byte-identical to 0.2.0 output.
   - Cost: 0.2.0 opening a newer file drops the field without warning and plays semitones; saving there loses it. That is the same class of loss the newer-file guard addresses, and the guard only fires on `FORMAT_VERSION`.
   - Alternative 1: always degrees, no field. Simpler; breaks 0.2.0 songs that use a scale with `ARP`. 0.2.0 is ten days old and no example uses a scale, so few files exist (inferred, not measurable).
   - Alternative 2: the field plus `FORMAT_VERSION` 3 when it is true, so 0.2.0 warns. Costs a version bump for one flag.

3. **Per-instrument opt-out.** TODO.md and design-gaps G2 assume G10 needs one for drums. Under A no written note is snapped; only `ARP`, table steps and link transpose move pitch, and each is written by hand for one track or instrument. A GM drum track (note number selects the drum) is harmed only if someone puts `ARP` on it.
   - Proposal: no opt-out in this change. The engine would also need each instrument's flag passed in for live, render and MIDI export, as `set_channel_info` passes track state.
   - Revisit if a table shared by pitched and drum instruments, or option B, is chosen.

4. **Which commands.** Proposal: `ARP` and table steps only. `PUP`/`PDN`/`PVB` are continuous and have no steps. `PSL` targets a written note. Link transpose already counts degrees and keeps doing so whatever `scale_steps` says, so 0.2.0 links do not change.

5. **Previews.** Previews bypass the engine, so a table on a preview is already silent. No change.

## Plan

1. `Song.scale_steps` with a serde default of false; `Song::new` sets it. Round-trip and 0.2.0-file tests in `tests/persistence.rs`.
2. `step_offset` in the engine; `ARP` and both table sites use it. Tests: `ARP24` in C major bends 4 then 7; off with `scale_steps` false; chromatic equals semitones; the arpeggio-table equivalence test holds under degrees; MIDI export and render follow (one test each).
3. TUI and GUI song settings, with undo.
4. README (scales section, `ARP`, tables), CHANGELOG, design-gaps G10 marked done, TODO.md.
