# TODO

## GUI - High Priority (editing workflow)

- [ ] Synth parameter editor -- instrument list sidebar exists but no way to edit synth params (waveform, ADSR, filter cutoff/resonance)
- [ ] Sample editor panel -- no way to edit trim points, loop start/end, base note; visualization shows waveforms but doesn't expose controls
- [ ] Horizontal channel scrolling -- grid hard-codes all channels visible; >8 channels becomes unusably compressed

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
