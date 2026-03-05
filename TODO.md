# TODO

## Tier 1 -- Foundational

Without these, the tracker is a toy. Save/load makes work persistent,
undo/redo makes editing safe, copy/paste makes editing practical.

- [x] Save/load native format (JSON .rtrk files)
- [x] Undo/redo
- [x] Copy/paste rows, selections, and channels

## Tier 2 -- Real Sequencing

Minimum feature set to compose something beyond a single loop.

- [x] Multiple pattern support (create, clone)
- [x] Order list navigation and editing (add/remove entries, Ctrl+Left/Right nav)
- [x] Per-channel MIDI channel mapping (channel 0-15)
- [x] Pattern break / jump effects (Bxx position jump, Dxx pattern break)
- [x] Channel mute (F9-F12), visual dim on muted channels
- [x] Channel solo (Ctrl+F9-F12)

## Tier 3 -- Quality of Life

- [x] Edit step configuration (( / ) keys, 0-16 range)
- [x] Row insert/delete within pattern (Insert/Backspace in Normal mode)
- [x] MIDI input for note entry (virtual port RTRACK_MIDI_IN, auto-enters in Insert mode)
- [x] Pattern length per-pattern (each pattern tracks its own row count)
- [x] MIDI CC support via effect column (Cxx: controller from instrument col, value xx)
- [x] Program change / instrument mapping (Exx: program change to program xx)

## Tier 4 -- Polish

- [x] Song settings dialog (F6: title, BPM, speed, channels, rows)
- [x] Instrument list view (F7: 256 instruments, editable names)
- [x] Order list sidebar (always visible, shows current position)
- [x] Color theme / configuration (F8: dark, light, monokai)
- [x] Mouse support (click to position cursor, scroll wheel)
- [x] Export to standard MIDI file (.mid) (Ctrl+E)
- [x] Import from .mid (pass .mid file as CLI argument)
- [x] MIDI clock output (Ctrl+M: 24 ppqn, start/stop messages)

## Tier 5 -- Samples

- [x] Sample loading (WAV, AIFF) via hound + dasp
- [x] Sample playback engine (alongside MIDI/synth, pitch-shifted via linear interpolation)
- [x] Sample editor (trim start/end, loop start/end/toggle, base note)
- [x] Per-instrument sample assignment (instrument.sample_index -> SampleBank slot)
- [x] WAV audio export (Ctrl+W, offline render to 16-bit stereo WAV)
- [ ] FLAC audio export

## Done

- [x] MIDI port selection UI (F2)
- [x] Help screen (F1)
- [x] Visual feedback on playback (scrolling highlight)
- [x] Ableton Link integration (F3)

## Effects

- [x] Arpeggio (0xy)
- [x] Portamento up/down (1xx/2xx)
- [x] Tone portamento (3xx)
- [x] Vibrato (4xy)
- [x] Volume slide (5xy)
- [x] Set speed/tempo (Fxx)
- [x] Note delay (6xx)
