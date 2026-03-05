# TODO

## Core Tracker

- [ ] Order list navigation and editing (add/remove/reorder patterns)
- [ ] Multiple pattern support (create, clone, delete patterns)
- [ ] Copy/paste rows, selections, and channels
- [ ] Undo/redo
- [ ] Pattern length per-pattern (currently fixed at 64)
- [ ] Edit step configuration (currently hardcoded to 1)
- [ ] Row insert/delete within pattern

## MIDI

- [ ] MIDI port selection UI (currently auto-connects first port)
- [ ] Per-channel MIDI channel mapping (channel 0-15)
- [ ] Program change / instrument mapping
- [ ] MIDI CC support via effect column
- [ ] MIDI clock output (sync to external gear)
- [ ] MIDI input for note entry (play notes in from a controller)

## Effects

- [ ] Implement standard tracker effects (arpeggio, portamento, vibrato, etc.)
- [ ] Volume slide
- [ ] Pattern break / jump
- [ ] Tempo change effect
- [ ] Delay

## UI

- [ ] Help screen (F1)
- [ ] Song settings dialog (title, global tempo, channels)
- [ ] Instrument list view
- [ ] Order list sidebar
- [ ] Channel mute/solo
- [ ] Visual feedback on playback (scrolling highlight)
- [ ] Color theme / configuration
- [ ] Mouse support

## File I/O

- [ ] Save/load native format (JSON or binary)
- [ ] Export to standard MIDI file (.mid)
- [ ] Import from .mid

## Samples (future)

- [ ] Sample loading (WAV, AIFF)
- [ ] Sample playback engine (replacing/alongside MIDI)
- [ ] Sample editor (trim, loop points)
- [ ] Per-instrument sample assignment
- [ ] WAV/FLAC audio export (render to file)
