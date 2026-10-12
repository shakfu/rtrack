# G9: groove

A song-level groove that replaces `Song.swing` and covers patterns swing cannot express: a 3-row shuffle, a 4-row push and pull. Closes the song-level half of G9 in [design-gaps.md](design-gaps.md); per-track grooves need G3 option 1 and stay out of scope. Code refers to `rtrack-core/src` as of 2026-10-13.

Status: implemented 2026-10-13, with the proposal for each decision: row weights, one song groove, groove applied to MIDI export, counted by pattern row.

## Current state

- `Song.swing` is 0-100, where 50 means none.
- `TrackerEngine::seconds_per_tick` makes each tick of an even pattern row `swing / 50` as long as normal, and each tick of an odd row `(100 - swing) / 50` as long. Ticks per row (`Song.speed`, `TIC`) do not change, so a row's effects run the same number of steps, just faster or slower.
- Live playback and render both time ticks through `seconds_per_tick`.
- MIDI export places events by tick count, at a fixed spacing, so it ignores swing.
- `Song::swing_seconds_per_tick` duplicates the engine's formula and is called only by tests.

## Options

**A. Ticks per row (M8).** A groove is a list of tick counts, for example `[7, 5]`, and replaces speed.
- Resolution is limited by the tick count: at 6 ticks a row, `[7, 5]` is 58 % and `[8, 4]` is 67 %.
- `TIC` and the groove both set ticks per row, so one has to override the other.
- Migrating swing to it is lossy; that was the design-gaps proposal.
- Rows get more or fewer effect steps: `ARP` on a long row runs one more cycle.

**B. Row weights.** A groove is a list of relative row lengths, for example `[67, 33]`, applied as today's swing is: by stretching the tick length.
- Speed and `TIC` are unchanged, and resolution is 1 part in the weights' sum.
- Swing `s` becomes `[s, 100 - s]` exactly, so old songs keep their timing to the sample.
- A row's effects run the same number of steps, as with swing today.

| | A | B |
|-|-|-|
| Swing migration | lossy | exact |
| Interaction with `TIC` | conflicting | none |
| Resolution | ticks per row | weight units |
| Effect steps per row | change with the groove | constant |
| Engine change | tick counts vary per row | one formula generalised |

## Recommendation: B

B is today's swing with any number of steps. It keeps every existing song's timing, needs no rule against `TIC`, and is the smaller change. A's distinctive behaviour, more effect steps on a longer row, is a side effect that few people would want from a groove.

## Model (B)

```rust
pub struct Song {
    // ...
    /// Relative row lengths, repeating: row `r` of a pattern lasts
    /// `len * groove[r % len] / sum` of a straight row. Empty means straight.
    pub groove: Vec<u8>,
}
```

- **Timing.** `seconds_per_tick` becomes `base * len * groove[row % len] / sum(groove)`. A full cycle lasts exactly as long as the same rows played straight, so the tempo holds on average.
- **Limits.** 1-16 steps; weights 1-255. Weight 0 would make a row take no time, and repair raises it to 1.
- **Old files.** `swing` is still read. A value other than 50 becomes `[swing, 100 - swing]`; 50 becomes no groove. `swing` is no longer written. This joins unreleased format version 2.
- **Editing.** The TUI song settings take the groove as text, for example `67 33` or `55 45 52 48`; a single number `s` means `s 100-s`, which is swing. The GUI song settings get one weight field per step, with add and remove buttons.

## Decisions

1. **Model A or B.** Proposal: B.
2. **One groove or a pool.** M8 keeps several grooves and switches between them with `GRV`.
   - Proposal: one song groove now. `GRV` stays reserved, and a pool can be added later without migration: the single groove becomes groove 0.
3. **MIDI export.** It ignores swing today.
   - Proposal: apply the groove, rounding each row's length to the export's tick resolution. An exported song would then sound as it plays.
4. **What the groove counts from.** Swing counts pattern rows, so it restarts with each pattern.
   - Proposal: keep that. A 3-step groove in a 16-row pattern then restarts every pattern. Counting rows played since the song started (`rows_played`, from G1) would carry the groove across patterns, like a running phrase.

## Work

1. **Done.** `Song.groove`; `Song::adopt_legacy_swing` (called by both load functions and `repair`); `Song::grooved`, used by the engine, computed as `length * weight / mean` so a swing groove repeats the old formula's operations and timing matches to the frame (tested for swing 1-99 at six tempos and two sample rates); repair truncates past 16 steps and raises weight 0 to 1. `Song::swing_seconds_per_tick` is gone.
2. **Done.** MIDI export advances each tick by its grooved length from an exact running position, so rounding does not accumulate. Tested: groove `[3, 1]` places rows at 0, 1.5 and 2 straight rows.
3. **Done.** Both editors edit the groove as text through `Song::groove_text` / `Song::parse_groove` (TUI `:set` field, GUI song settings, applied when the field loses focus); undo coalesces under `song.groove`. The GUI got a text field rather than one field per step: the same parser, less UI.
4. **Done.**

## Not in scope

- Per-track grooves (G3 option 1).
- `GRV`, a groove pool, and sub-row delay (Renoise's delay column).
