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
