# TODO

## GUI - High Priority (editing workflow)

- [x] Synth parameter editor -- already implemented in instrument_editor.rs (patch selector, ADSR, filter, oscillator, FM)
- [x] Sample editor panel -- already implemented in instrument_editor.rs (load, trim, loop, base note, waveform preview)
- [x] Horizontal channel scrolling -- auto-scrolls to follow cursor, computes visible count from available width

## GUI - Medium Priority (workflow polish)

- [x] Keyboard shortcut help overlay -- implemented in dialogs.rs (F1 toggle)
- [x] Timing/position display -- elapsed MM:SS in transport bar
- [x] Recording indicator -- REC button in red when armed, gray when inactive

## GUI - Lower Priority (nice to have)

- [x] Recent files in File menu -- submenu under File
- [x] Drag-to-select pattern regions
- [x] Drag-and-drop sample loading

## TUI - Recently Added

- [x] Audio visualization panel -- level meters (L/R) + spectrum analyzer (32-bar Goertzel) in bottom panel
- [x] Voice playhead markers -- red playhead indicators on sample editor waveform during playback
- [x] Live slice boundary preview -- cyan markers on waveform when adjusting slice count/sensitivity fields
- [x] Related slice boundaries -- yellow markers showing other slices from same source sample
- [x] Loop markers -- green markers for loop start/end on waveform
- [x] Waveform trim dimming -- regions outside trim range shown in dark gray

## Architecture & Quality

- [x] Sort entries in `SampleBank::load_directory` by filename before loading -- current `read_dir` iteration order is OS-dependent, making slot assignment non-deterministic across platforms
- [x] Fix flaky `test_render_empty_song` -- panics on missing temp path intermittently (race or platform-dependent temp dir)
- [x] Add `TrackerCoreBuilder` that can skip hardware init and accept injected `MidiEngine`/`AudioEngine`/`LinkEngine` -- current `with_song_size` unconditionally opens MIDI ports and Link sessions, even for offline/test use
- [x] Add CI workflow (GitHub Actions) -- `.github/workflows/ci.yml`: fmt, clippy, rustdoc, tests and example freshness on Linux and macOS, plus an MSRV job. `build.yml` produces release binaries for four platforms; `audit.yml` checks dependencies against the RustSec database weekly
- [x] Add `fmt-check` Makefile target -- `make ci` now runs fmt-check, clippy, doc, test and check-examples without mutating anything; `make lint` still reformats in place
- [x] GUI: read audio/SF2 device config from `config.toml` before initializing `AudioEngine` -- currently hardcodes `None` in `RtrackApp::new`
- [x] Replace eager `Sample`/`SampleBank` clones with `Arc<Sample>` per slot -- prerequisite for live sample editing and reduces undo/clone allocation cost
- [x] Extract shared editor state from TUI and GUI -- `rtrack-core::editor` holds `SubColumn`, `Clipboard`, `Edit` and `EditHistory`, and both frontends run on them. The shared `Edit` is neither of the two old models: cells stay diffs, structural changes carry a song snapshot, and `Edit::Group` covers an action that changes several at once. The GUI gained structural undo it never had; the history is bounded by bytes (`MAX_UNDO_BYTES`) rather than by step count, because a step's size follows the song
- [x] The trim/loop/base-note fields in the sample editor are under undo. `EditSource` tags an edit with the control it came from and the history amends the step it already has, so a held arrow key or a dragged value is one step and undo returns to where the field stood before the run began. The same machinery put the GUI's song settings under undo

- [ ] Loading a sample over an occupied slot is still not undoable -- the last part of the sample bank outside the history

## TUI - Low Priority (nice-to-have or high effort)

- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] UI snapshot tests (ratatui TestBackend)
- [ ] Fuzz testing (`cargo-fuzz`, needs nightly) for the MIDI, AIFF and `.rtrk` parsers. `rtrack-core/tests/hostile_input.rs` is the stable-toolchain stand-in and runs in CI: hand-picked malformed shapes plus truncation and byte-flip sweeps over all four formats. It found nothing the hand audit had not, but the hand audit found four allocation bugs in two sittings, so the yield is not exhausted

- [ ] Decide whether slicing should respect a trim the user set by hand. `SliceRange::Source` ignores it and divides the whole file, which is right for a slice (whose span is a slicing artifact) and wrong for a sample someone trimmed to the part they wanted. Telling the two apart needs provenance on `Sample` -- the span a slot was cut out of -- persisted in `.rtrk`, not just a range argument. `SliceRange::Span` is the workaround in the meantime: it divides exactly the trimmed region.

## Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
- [ ] Live granular editing / waveform scrubbing -- depends on `Arc<Sample>` refactor above to avoid O(256 x frames) clone on every edit
- [ ] Chord type -- a note that sounds several pitches at once, as a third kind of sound source alongside samples and single monophonic notes. A tracker channel is one voice, and the engine now enforces it: a pattern note-off stops the whole channel, per-channel effect state holds a single `porta_target`/`vibrato_phase`/`pitch_offset`, and a pattern row has one note per channel. Polyphony therefore has to live inside the note rather than in overlapping notes on a channel. Open questions: how a chord is entered and shown in a pattern cell, whether its voices share the channel's effect state or each track their own, and how it is stored in `.rtrk` and written to MIDI export. Live MIDI chord entry is the same problem one layer up -- `TrackerCore::preview_note` is a single `Option<PreviewNote>` that stops the previous note before starting the next, so a keyboard chord cannot currently be captured either.
