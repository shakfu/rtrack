# TODO

## Low Priority (nice-to-have or high effort)

- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] UI snapshot tests (ratatui TestBackend)
- [ ] Fuzz testing for MIDI file parser, AIFF parser, .rtrk deserializer

## Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
- [ ] Live granular editing / waveform scrubbing -- would require per-slot `Arc<Sample>` in `SampleBank` to avoid O(256 x frames) clone on every edit. Current clone-and-swap is fine for load-once-play-many but not for real-time sample manipulation.
