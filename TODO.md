# TODO

## High Priority (high impact, moderate effort)

- [ ] Effects -> MIDI export (portamento/vibrato -> pitch bend, volume slide -> CC7)
- [ ] MIDI input CC/pitch bend/program change handling (currently only Note On/Off)
- [ ] External MIDI clock input/sync (currently output-only)
- [ ] Send/return effects routing -- shared effect buses with per-channel wet/dry sends
- [ ] Integration tests (load .rtrk, play, verify rendered audio)

## Medium Priority (quality-of-life, moderate effort)

- [ ] Auto-save to temp file periodically
- [ ] Row highlight configurability (beat/bar intervals beyond 4/16, for time signature support)
- [ ] Tempo automation (BPM changes beyond Fxx effect)
- [ ] Swing/groove (non-uniform tick spacing)
- [ ] Configurable pitch bend range per instrument (currently hardcoded +/-2 semitones)
- [ ] Link timing: use beat timeline directly instead of accumulating deltas

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
