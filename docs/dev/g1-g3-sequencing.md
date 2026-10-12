# G1 + G3: reuse and polymeter

Closes G1 (reuse is whole-pattern only) and the polymeter half of G3 (no per-track play position) from [design-gaps.md](design-gaps.md). Code refers to `rtrack-core/src` as of 2026-10-13.

Status: implemented 2026-10-13: option C, with the proposal for each decision.

## Requirements

From the 2026-10-12 answer to G1's open question:

1. **Reuse per channel.** Change a bass line once, and every section that uses it changes.
2. **Polymeter across pattern boundaries.** A 3-row figure against a 4-row one keeps its phase through pattern changes, without the user aligning pattern lengths to the least common multiple.
3. **Easy to write.** No bookkeeping beyond choosing what repeats and how long it is.

The constraints in design-gaps.md still hold: one source of truth, old files load, MIDI import and export work, render matches live, and the audio callback does not allocate.

## Current state

- `Song.order: Vec<usize>` indexes full-width `Pattern`s. `order_repeats` plays an entry 0, 1 or more times.
- `TrackerEngine` keeps one position: `order`, `row` and `repeat_count`.
- `Song.tempo_map` is keyed by `(order, row)` (`Song::tempo_at`).
- `JMP` and `BRK` move the one position.
- The engine reads cells through `Song::cell_at`, in one place. The editors read and write `Pattern` cells directly, in about 230 places across both frontends and core.

## Options

**A. Per-channel order lists over single-channel patterns** (design-gaps G3 option 1). Each channel has its own sequence and its own position.

**B. Column aliases plus a per-column loop length** (G1 option 1 plus G3 option 3). A pattern column can alias the same column of another pattern, and can loop after `n` rows. One global position remains.

**C. Phrase links with a running phase** (new). A pattern column can link to a phrase: a reusable single-channel row sequence. A link either aligns with the pattern or runs on its own phase across pattern boundaries. One global position remains.

| | A | B | C |
|-|-|-|-|
| Reuse per channel | yes | yes | yes |
| Polymeter across pattern boundaries | yes | no: loops restart with each pattern | yes, in running mode |
| Different section lengths per channel | yes | no | no |
| Per-track live launch (G11 option 1) | yes | no | no |
| Transpose per use (G2) | on the sequence entry | on the alias | on the link |
| `tempo_map`, `JMP`, `BRK` | must be redefined | unchanged | unchanged |
| MIDI export | must flatten channels | unchanged | unchanged |
| Pattern grid editing | replaced by a per-channel view | unchanged | unchanged; linked columns are marked |
| Engine position | one per channel | one | one, plus a running row counter |

A's extra capabilities are per-channel section lengths and per-track launch. Neither was asked for. B fails requirement 2.

## Recommendation: C

C meets all three requirements and keeps one global position, so `tempo_map`, `JMP`/`BRK`, MIDI export, render and the pattern grid keep working as they are. Reuse and polymeter come from one concept, the phrase, instead of two.

Choose A instead if per-track launch (G11) or per-channel section lengths become goals. C does not lead towards A: the two models differ in how a song is structured.

## Model (C)

```rust
pub struct Song {
    // ...existing fields...
    pub phrases: Vec<Phrase>,
}

/// A single-channel run of rows, usable by any column of any pattern.
pub struct Phrase {
    pub name: String,
    pub rows: Vec<Cell>,
}

pub struct Pattern {
    // ...existing fields...
    /// Per channel: the phrase this column plays instead of its own cells.
    pub links: Vec<Option<PhraseLink>>,
}

pub struct PhraseLink {
    pub phrase: usize,
    /// Semitones added at playback (G2). Never written back to the phrase.
    pub transpose: i8,
    pub timing: PhraseTiming,
}

pub enum PhraseTiming {
    /// Pattern row `r` plays phrase row `r`.
    Aligned,
    /// Plays phrase row `n % len`, where `n` counts rows played since the
    /// song started. Continues across patterns and repeats.
    Running,
}
```

- **Reading.** Every cell read goes through `Song::cell_at`, which resolves links. The roughly 230 direct reads and writes move onto it, or onto a matching `cell_at_mut`.
- **Writing.** Editing a linked cell writes the phrase, so every use changes. The editors mark linked columns, so this is visible before typing.
- **Linking and unlinking.** Linking a column turns its cells into a new phrase, or points it at an existing one. Unlinking copies the phrase rows into the column's cells.
- **Engine.** Adds a running row counter, `rows_played: u64`. Playback that starts mid-song sets it to the linear row position of the start point, counting repeats. `JMP` and `BRK` do not reset it.
- **Render and MIDI export.** Both are driven by the engine, so they resolve links without changes.
- **MIDI import** writes ordinary columns, unchanged.
- **File format.** `phrases` and `links` are new optional fields. Older files load unchanged. They join `FORMAT_VERSION` 2, which is not yet released; rtrack 0.1.3 already warns on a version 2 file.

## Decisions

1. **An aligned phrase shorter than the pattern.** Options: rows past its end are empty, or it loops.
   - Proposal: it loops. That makes `Aligned` G3 option 3 (polymeter within a pattern) at no extra cost.
2. **Running phase after a jump.** When `JMP`, `BRK` or the end-of-song wrap moves the position, `rows_played` either keeps counting or is recomputed from the new position.
   - Proposal: keep counting. Polymeter then never re-aligns unless the user arranges it. Recomputing makes jumps deterministic but restarts phrases.
3. **What the grid shows in a running column.** The row a phrase plays depends on how playback reached the pattern. Options:
   - the first pass (proposal);
   - the current pass while playing;
   - a "running" marker with no cells shown.
4. **Phrase length limit.** Proposal: 1-256 rows, the same as patterns.
5. **The drum opt-out for transpose** (G2 option 3). It belongs on the instrument and lands with G2. Links carry `transpose` from the start, so G2 needs no second format change.

## Work

1. **Done.** `Phrase`, `PhraseLink` and `PhraseTiming` (`tracker/phrase.rs`), `Song.phrases`, `Pattern.links` and `Pattern::link`. `Song::repair` gives an empty phrase one row, truncates one past 256 rows, and unlinks a link to a missing phrase. Neither field is written when unused, so existing files and their fixtures are unchanged.
2. **Done.** Cell access goes through `Song`. `Song::address(order, row, channel)` returns a `CellAddr` (a pattern cell or a phrase row); `cell` and `cell_mut` access it, and `cell_at` / `cell_at_mut` / `set_cell` are built on them. Undo records `CellEdit { at: CellAddr, .. }` and applies old values in reverse. `Song::addresses` lists a block's distinct cells, for transforms such as transpose. Both grids read through `cell_at`. Pattern-level operations (row insert and delete, clone, resize) still act on the pattern; their behaviour on linked columns is settled in step 4. GUI transpose now records undo.
3. **Done.** `Song::address` resolves links: aligned row `r` to phrase row `r % len`, running row `r` to `(first_pass_row(order) + r) % len`. A link to a missing phrase reads as no link. Playback reads through `Song::playback_cell`, where a running link uses `TrackerEngine::rows_played`; `reset` sets it to the first-pass position of the start point, and each row adds one, through repeats, jumps and the wrap. `Song::uses_second_fx` also looks at linked phrases. Tests: aligned looping and editing, first-pass display, running phase across patterns and repeats, a jump that keeps counting, a mid-song start matching the grid, and a linked render equal to the notes written out. Transpose on the link is stored but not yet applied (G2).
4. **Done.** Core: `Song::link_new_phrase`, `link_column`, `unlink_column` (keeps the first-pass rows as the column's cells), `set_link_timing`, `resize_phrase`. TUI: `:phrase`, `:phrase NN`, `:run`, `:unlink`, and a `:phrases` list (rename, Ctrl+Left/Right length, Enter links). GUI: View > Phrases. Headers show `[Pnn]`/`[Rnn]`. Row insert and delete act on the pattern's cells and say that linked phrases are unchanged; pattern clone keeps links; a new phrase takes the pattern's row count. All of it is undoable.
5. **Done.** `examples/polymeter.rtrk`, generated by `cargo xtask regen-examples`; README section and CHANGELOG entry.

## Not in scope

- Per-track launch (G11), and per-channel section lengths. These need option A.
- Transpose at playback: done in G2 (design-gaps.md).
