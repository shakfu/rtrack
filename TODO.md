# TODO

## Critical

## High

## Medium

### Architecture & Quality

- [ ] Loading a sample over an occupied slot is still not undoable -- the last part of the sample bank outside the history

## Low

### TUI - Low Priority (nice-to-have or high effort)

- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] UI snapshot tests (ratatui TestBackend) -- the song settings dialog is covered in `rtrack-tui/src/tui/mod.rs`; the pattern grid and the other dialogs are not
- [ ] Fuzz testing (`cargo-fuzz`, needs nightly) for the MIDI, AIFF and `.rtrk` parsers. `rtrack-core/tests/hostile_input.rs` is the stable-toolchain stand-in and runs in CI: hand-picked malformed shapes plus truncation and byte-flip sweeps over all four formats. It found nothing the hand audit had not, but the hand audit found four allocation bugs in two sittings, so the yield is not exhausted
- [ ] Decide whether slicing should respect a trim the user set by hand. `SliceRange::Source` ignores it and divides the whole file, which is right for a slice (whose span is a slicing artifact) and wrong for a sample someone trimmed to the part they wanted. Telling the two apart needs provenance on `Sample` -- the span a slot was cut out of -- persisted in `.rtrk`, not just a range argument. `SliceRange::Span` is the workaround in the meantime: it divides exactly the trimmed region.

### Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
- [ ] Live granular editing / waveform scrubbing -- depends on `Arc<Sample>` refactor above to avoid O(256 x frames) clone on every edit
- [ ] Chord type -- a note that sounds several pitches at once, as a third kind of sound source alongside samples and single monophonic notes. A tracker channel is one voice, and the engine now enforces it: a pattern note-off stops the whole channel, per-channel effect state holds a single `porta_target`/`vibrato_phase`/`pitch_offset`, and a pattern row has one note per channel. Polyphony therefore has to live inside the note rather than in overlapping notes on a channel. Open questions: how a chord is entered and shown in a pattern cell, whether its voices share the channel's effect state or each track their own, and how it is stored in `.rtrk` and written to MIDI export. Live MIDI chord entry is the same problem one layer up -- `TrackerCore::preview_note` is a single `Option<PreviewNote>` that stops the previous note before starting the next, so a keyboard chord cannot currently be captured either.
