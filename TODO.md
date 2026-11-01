# TODO

## Resume here (state at 2026-11-01)

0.3.0 is released. `dev` is merged with 0.3.0 as the base; see CHANGELOG, Unreleased. `FORMAT_VERSION` is 4.

Next: use `KIL`, tables and mod slots in a real song, and review `docs/frontend-differences.md`, whose rows came from `dev` and are not checked against 0.3.0.

Workflow used for each gap: write `docs/dev/gN-*.md` with options, a recommendation and decisions; ask; implement; mark the doc implemented; update design-gaps.md, README, CHANGELOG and this file.

## Critical

## High

### Design gaps (`docs/dev/design-gaps.md`; G1-G3 placed second, ahead of the doc order, after the reuse-and-polymeter answer)

- [ ] G3: per-track play position. Reuse and polymeter are done (phrases, `docs/dev/g1-g3-sequencing.md`); a per-track position is still needed for per-track launch (G11) and per-channel section lengths, neither currently a goal (`docs/dev/g1-g3-sequencing.md`)

- [ ] G11: live launch. Pattern queueing quantised to pattern end works with one global position; per-track cueing needs G3 option 1 (`docs/dev/design-gaps.md` G11)

## Medium

## Low

### TUI - Low Priority (nice-to-have or high effort)

- [ ] Keybinding customization, step 2: one action list, `[keys]` overrides in `config.toml`, help generated from the bindings (`docs/dev/keybindings.md`). Step 1, piano layouts for QWERTZ and AZERTY, is done

- [ ] UI snapshot tests (ratatui TestBackend) -- covered in `rtrack-tui/src/tui/mod.rs`: song settings, quit prompt, synth editor, instrument list, phrase list, table editor, an empty sample slot, help (no line cut), the pattern grid's cells and phrase gutter, and the header at several widths. Not covered: track config, a loaded sample in the sample editor, and the file browser, recent files and MIDI port selector, whose contents depend on the machine

### Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)

- [ ] Piano roll view (alternative note entry)

- [ ] Audio recording to sample slots

- [ ] Live granular editing / waveform scrubbing

- [ ] Chord type -- `CHD` (G12, `docs/dev/g12-chords.md`) now plays a chord per row, sharing the track's pitch and effect state. What remains is the rest of this item: chord notes shown and edited in the cell, voices with their own effect state, and live MIDI chord capture. Original note: a note that sounds several pitches at once, as a third kind of sound source alongside samples and single monophonic notes. A tracker channel is one voice, and the engine now enforces it: a pattern note-off stops the whole channel, per-channel effect state holds a single `porta_target`/`vibrato_phase`/`pitch_offset`, and a pattern row has one note per channel. Polyphony therefore has to live inside the note rather than in overlapping notes on a channel. Open questions: how a chord is entered and shown in a pattern cell, whether its voices share the channel's effect state or each track their own, and how it is stored in `.rtrk` and written to MIDI export. Live MIDI chord entry is the same problem one layer up -- `TrackerCore::preview_note` is a single `Option<PreviewNote>` that stops the previous note before starting the next, so a keyboard chord cannot currently be captured either.
