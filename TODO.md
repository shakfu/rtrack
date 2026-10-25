# TODO

## Resume here (state at 2026-10-25)

0.2.0 is cut: G1-G3 phrases and the phrase editor (`docs/dev/phrase-editor.md`), G2 link transpose, G4+G8 format, G5 instrument drive and filter, G7 tables, G9 groove, MIDI channel keying, live/render fixes. Design records: `docs/dev/g1-g3-sequencing.md`, `g4-g8-format.md`, `g5-instrument-fx.md`, `g7-tables.md`, `g9-groove.md`, `phrase-editor.md`; G2 is in `design-gaps.md`. 874 tests passed at 0.2.0; 976 pass now, and `make ci` is clean.

`FORMAT_VERSION` 2 is released with 0.2.0. G10's drum opt-out should be an optional field that 0.2.0 ignores, so it needs no version 3.

Next, in order:

1. Use step 1 of G6; decide whether step 2 (mod slots) is needed.

Done in 0.3.0 (CHANGELOG): the quick fixes (GUI synth sliders, render `Cnn`, MIDI export channels), SF2 render tests, synth undo, phrase delete, the link-over-cells notice, TUI phrase borders, the examples' `swing` field, core gesture undo (`TrackerCore::record_song_edit`/`record_bank_edit`), G10 (`docs/dev/g10-scale-playback.md`), G6 step 1 (`docs/dev/g6-modulation.md`), per-instrument MIDI ports (`docs/dev/midi-ports.md`), and G12 `CHD` chords (`docs/dev/g12-chords.md`).

Workflow used for each gap: write `docs/dev/gN-*.md` with options, a recommendation and decisions; ask; implement; mark the doc implemented; update design-gaps.md, README, CHANGELOG and this file.

## Critical

## High

### Design gaps (`docs/dev/design-gaps.md`; G1-G3 placed second, ahead of the doc order, after the reuse-and-polymeter answer)

- [ ] G3: per-track play position. Reuse and polymeter are done (phrases, `docs/dev/g1-g3-sequencing.md`); a per-track position is still needed for per-track launch (G11) and per-channel section lengths, neither currently a goal (`docs/dev/g1-g3-sequencing.md`)

- [ ] G6 step 2: two mod slots per instrument (AHD, ADSR, LFO, tracking) on the timbre path step 1 built. Designed in `docs/dev/g6-modulation.md`; build it if tables with `CUT`/`RES`/`DRV` turn out not to be enough

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
