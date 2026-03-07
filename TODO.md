# TODO

## High Priority (high impact, moderate effort)

- [x] Effects -> MIDI export (portamento/vibrato -> pitch bend, volume slide -> CC7)
- [x] MIDI input CC/pitch bend/program change handling (currently only Note On/Off)
- [x] External MIDI clock input/sync (currently output-only)
- [x] Send/return effects routing -- shared effect buses with per-channel wet/dry sends
- [x] Integration tests (load .rtrk, play, verify rendered audio)

## Medium Priority (quality-of-life, moderate effort)

- [x] Sample slicing (equal-segment auto-slice, transient-detection sensitivity, slice-to-pattern mapping)
- [x] Auto-save to temp file periodically
- [x] Row highlight configurability (beat/bar intervals beyond 4/16, for time signature support)
- [x] Tempo automation (BPM changes beyond Fxx effect)
- [x] Swing/groove (non-uniform tick spacing)
- [x] Configurable pitch bend range per instrument (currently hardcoded +/-2 semitones)
- [x] Link timing: use beat timeline directly instead of accumulating deltas

## Low Priority (nice-to-have or high effort)

- [ ] Recent files list for quick re-open
- [ ] Header truncation handling on narrow terminals
- [ ] Keybinding customization (config file with tracker presets)
- [ ] MIDI channel pressure (aftertouch) support
- [ ] MIDI learn -- map a CC to a parameter
- [ ] UI snapshot tests (ratatui TestBackend)
- [ ] Fuzz testing for MIDI file parser, AIFF parser, .rtrk deserializer

## Ambitious (significant effort, transformative)

- [ ] Plugin hosting (VST/CLAP) - (see: <https://crates.io/crates/rack>)
- [ ] Piano roll view (alternative note entry)
- [ ] Audio recording to sample slots
