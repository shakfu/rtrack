# TODO

## GUI - High Priority (editing workflow)

- [x] Synth parameter editor -- already implemented in instrument_editor.rs (patch selector, ADSR, filter, oscillator, FM)
- [x] Sample editor panel -- already implemented in instrument_editor.rs (load, trim, loop, base note, waveform preview)
- [x] Horizontal channel scrolling -- auto-scrolls to follow cursor, computes visible count from available width

## GUI - Medium Priority (workflow polish)

- [ ] Keyboard shortcut help overlay -- quick-reference popup for discoverability
- [ ] Timing/position display -- show MM:SS:CC elapsed time, pattern position (P: XX/XX), mode indicator in transport bar
- [ ] Recording indicator -- visual feedback for record-armed state beyond the button

## GUI - Lower Priority (nice to have)

- [ ] Recent files in File menu
- [ ] Drag-to-select pattern regions
- [ ] Drag-and-drop sample loading

## TUI - Low Priority (nice-to-have or high effort)

- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] UI snapshot tests (ratatui TestBackend)
- [ ] Fuzz testing for MIDI file parser, AIFF parser, .rtrk deserializer

## Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
- [ ] Live granular editing / waveform scrubbing -- would require per-slot `Arc<Sample>` in `SampleBank` to avoid O(256 x frames) clone on every edit. Current clone-and-swap is fine for load-once-play-many but not for real-time sample manipulation.
