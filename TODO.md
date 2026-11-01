# TODO

## Resume here (state at 2026-11-01)

0.3.0 is released. `dev` is merged with 0.3.0 as the base; see CHANGELOG, Unreleased. `FORMAT_VERSION` is 4.

Next: use `KIL`, tables and mod slots in a real song, and review `docs/frontend-differences.md`, whose rows came from `dev` and are not checked against 0.3.0.

Workflow used for each gap: write `docs/dev/gN-*.md` with options, a recommendation and decisions; ask; implement; mark the doc implemented; update design-gaps.md, README, CHANGELOG and this file.

## Roadmap (2026-11-01)

What remains to make rtrack expressive, ranked by musical payoff per unit of work. The sequencing layer (phrases, tables, groove, scale degrees, chords, mod slots) is ahead of the sound and performance layers. Step 1 is done; see CHANGELOG, Unreleased.

1. Per-note expression, each one `FxCmd` plus engine, render and export:
   - [x] `RET` retrigger: rolls and stutters, which tables cannot do
   - [x] `PAN` command and a heard track pan (`ChannelConfig.pan` was never applied)
   - [x] Sample amplitude envelope (`SampleFx::env`)
   - [x] `DET` detune. Microtuning is still G10 option 3
   - [x] Reverse and ping-pong sample loops
   - [x] `Pan` as a mod slot destination, in place of a pan slide command
2. Sound sources:
   - [ ] Mod slot tracking source: velocity and key to any destination (`docs/dev/g6-modulation.md`). The synth has no velocity-to-filter either
   - [ ] A second synth architecture (FM or wavetable). fundsp is already a dependency
3. Performance: G11 option 2, pattern queueing at pattern end, works with one global position. Decide first whether live use is a goal; if not, close G3 and G11
4. Entry speed:
   - [ ] Humanize or randomize a selection (volume, `DEL` timing). `RND` is per cell
   - [ ] Live MIDI chord capture (`preview_note` holds one note; see G12 below)
   - [ ] Keybinding customization step 2 (`docs/dev/keybindings.md`)
   - [ ] TUI track config: Volume and Pan fields (`docs/frontend-differences.md` row 12)

Not prioritized: plugin hosting and the piano roll (large, no sound a user cannot approximate), G3 per-track positions unless item 3 decides for live use.

These are inferences from comparing with M8 and Renoise, not observed friction. Writing one complete song and logging each friction point is still the cheapest check.

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
