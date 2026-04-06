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
- [ ] Add CI workflow (GitHub Actions): `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test --workspace`
- [ ] Add `fmt-check` Makefile target (`cargo fmt --all -- --check`) -- current `make lint` mutates files via `cargo fmt --all`, unsuitable for CI
- [x] GUI: read audio/SF2 device config from `config.toml` before initializing `AudioEngine` -- currently hardcodes `None` in `RtrackApp::new`
- [x] Replace eager `Sample`/`SampleBank` clones with `Arc<Sample>` per slot -- prerequisite for live sample editing and reduces undo/clone allocation cost
- [ ] Extract shared editor state (cursor, undo/redo, clipboard, block selection) from TUI and GUI into a common crate to prevent drift

## TUI - Low Priority (nice-to-have or high effort)

- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] UI snapshot tests (ratatui TestBackend)
- [ ] Fuzz testing for MIDI file parser, AIFF parser, .rtrk deserializer

## Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
- [ ] Live granular editing / waveform scrubbing -- depends on `Arc<Sample>` refactor above to avoid O(256 x frames) clone on every edit
