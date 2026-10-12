# G7: tables

Per-note sequences that run below the pattern: an arpeggio shape, a roll, a stepped filter or volume move, written once and replayed by every note. Closes G7 from [design-gaps.md](design-gaps.md). Code refers to `rtrack-core/src` as of 2026-10-13.

Status: implemented 2026-10-13, with the proposal for each decision: semitones, pitch bend, table N for instrument N, no tables on previews, 1-16 rows and ticks.

## What a table is

A table is a short list of rows, read one row every few ticks while a note sounds. It restarts at row 0 with each new note. A row may:

- move the note's pitch, relative to the note;
- set the volume;
- carry one effect command.

`ARP` is a table of three pitch rows. A table can also hold shapes `ARP` cannot: 4 or more steps, uneven timing, a volume decay, or a filter CC stepping with the notes.

## Constraints from the current code

- **The engine knows instrument numbers, not instruments.** Instruments live in `TrackerCore` and in `SongFile`, not in `Song`. A table the engine plays must therefore be reachable from `Song`; this is the same reason G2 dropped its per-instrument flag.
- **Pitch effects are pitch bends.** `ARP`, `PVB` and the slides move `ChannelState.pitch_offset` and emit `PitchBend`. They do not retrigger. On an `External` MIDI instrument, a bend wider than the instrument's bend range (default 2 semitones, `DEFAULT_PITCH_BEND_RANGE`) is clipped by the receiving synth. That already limits `ARP` today.
- **Previews bypass the engine.** Keyboard and MIDI-input previews call `TrackerCore::preview_note_*` directly, so nothing the engine does reaches them.
- **Render and MIDI export are driven by the engine**, so whatever the engine plays, they match.

## Model

```rust
pub struct Song {
    // ...
    /// Tables by number, 00-FF. Instrument N plays table N, if there is one.
    pub tables: Vec<Option<Table>>,
}

pub struct Table {
    pub name: String,
    /// Ticks each row lasts, 1-16.
    pub ticks_per_row: u8,
    /// After the last row, play again from this row; `None` stops and holds.
    pub loop_from: Option<u8>,
    pub rows: Vec<TableRow>, // 1-16
}

pub struct TableRow {
    /// Semitones added to the note while this row plays.
    pub transpose: i8,
    /// Volume while this row plays; `None` keeps the current one.
    pub volume: Option<u8>,
    pub fx: Option<Fx>,
}
```

- **Numbering, as M8 does it.** Table N belongs to instrument N by default. `TBLxx` on a row plays table `xx` for that row's note instead. On a row with no note, it restarts table `xx` on the note already sounding. This keeps tables in `Song`, so the engine reaches them without knowing instruments.
- **Engine state.** `ChannelState` gains `table: Option<(table, row, tick)>`. A note-on starts it at row 0 (a `DEL` note starts it when the note sounds). A note-off stops it.
- **Per tick.** When the row's ticks are up, the engine moves to the next row, or loops, or stops at the end. Each tick it applies the current row.
- **Pitch.** The row's `transpose` adds to the pitch offset, the same way `ARP` and `PVB` add to it today (step 3a of G8). Pattern slides and the table then combine.
- **Volume.** A row's `volume` sets the channel volume. A pattern `VSL` moves it from there.
- **Effects.** A table row's effect behaves as a third effect column, with the two-column rules from G8:
  - continuous commands add;
  - tick-0 commands fire when the row starts;
  - a row may carry `ARP`, `PUP`, `PDN`, `PVB`, `VSL`, `Cnn` or `PRG` (`Table::allows`). Song commands (`JMP`, `BRK`, `TIC`, `TPO`) belong to the song; `DEL`, `CHA` and `OFS` describe how a note starts, which a running table cannot change; `PSL`, `RND` and `TBL` need a target or history a table does not have.
- **File format.** `tables` is a new optional field, written as `{"slot": N, ...}` entries like instruments. It joins version 2, which is still unreleased.

## Decisions

1. **Pitch steps: semitones, or the song's scale.** Link transpose (G2) counts scale degrees when a scale is set. `ARP` counts semitones.
   - Proposal: semitones, like `ARP`, since tables mostly replace `ARP`. G10 (scale quantisation at playback) would then quantise `ARP` and tables together.
2. **Pitch by bend, or by retriggering.** Bending keeps one note sounding, which is what tracker arpeggios do, but is limited by an external synth's bend range. Retriggering sends a new note-on per row, so it works on any MIDI device, but re-attacks the note each row.
   - Proposal: bend, as `ARP` does. Note the limit in the docs.
3. **Default tables per instrument (table N for instrument N), or explicit `TBL` only.** Defaults make a table part of the instrument's sound, but the numbering couples the two pools.
   - Proposal: defaults, as M8 does. An instrument with no table N plays plainly.
4. **Previews.** Notes typed or played from a MIDI keyboard while stopped don't run tables, so a table is heard only on playback.
   - Proposal: accept this for now. Running tables for previews would need a second engine instance for preview notes.
5. **Table length and rate limits.** Proposal: 1-16 rows, 1-16 ticks per row, like M8's 16-row tables. A longer per-note sequence is a phrase (G1).

## Work

1. **Done.** `tracker/table.rs`: `Table`, `TableRow`, `MAX_TABLE_ROWS` and `MAX_TABLE_TICKS`; `Song.tables`, a `BTreeMap<u8, Table>` saved as `"tables": {"5": {...}}` and omitted when empty. `Table::repair`, called from `Song::repair`, fixes empty or long tables, out-of-range rates, a loop from a missing row, and song commands in rows (`Table::allows`). Existing files and fixtures are unchanged.
2. **Done.** `FxCmd::Table` (`TBL`). `ChannelState.table` (`TableState`) starts with each emitted note-on, from `TBLxx` on the row or the table numbered as the note's instrument; a `DEL` note starts it when it sounds; `TBL` on a row without a note restarts it; a note-off stops it; a `PSL` note leaves it. Each tick the current row's pitch adds into the channel's one pitch bend, its effect runs as a third column (with its own vibrato phase), and a starting row sets the volume and sends `Cnn`/`PRG`. Tick 0 applies the row's pitch too, so a note does not sound one tick unshifted. Muted channels do not run tables, as they do not run effects. Tests: a `[0, 4, 7]` table bends exactly as `ARP 47` and renders the same samples; looping and holding; ticks per row; row-start volume and CC; `TBL` over the instrument's table; an instrument without one; note-off; `DEL`.
3. **Done.** TUI: `:table [NN]` opens a table editor (the cursor's instrument by default); `+`/`-`, hex digits, `=` for effects by name, Ctrl+Left/Right rows, `[`/`]` ticks, `l` loop; `:tname`, `:tdel`. GUI: View > Tables. A table is created on first edit (`Song::table_mut`); effects go through `Table::set_row_effect`, which refuses what a table cannot play. All edits are undoable. Tables are saved as `{"slot": N, ...}` entries (`table::by_slot`): `Song` is flattened into `SongFile`, and through a flatten serde cannot read map keys back as numbers. The first form, a map, failed to load from any file; the example caught it.
4. **Done.** `examples/tables.rtrk` (generated by `cargo xtask regen-examples`), README Tables section and `TBL` row, CHANGELOG entry.

## Not in scope

- Tables modulating synth parameters directly (G6, mod slots).
- More than one effect per table row (M8 has 3).
