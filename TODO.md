# TODO

## Tier 1 -- Foundational

- [x] Save/load native format (JSON .rtrk files)
- [x] Undo/redo
- [x] Copy/paste rows, selections, and channels

## Tier 2 -- Real Sequencing

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
- [x] Sample directory loading (`--sample-dir`) with `<slot>-<name>.wav` convention
- [x] Optional `samples.json` metadata (BPM, base note, loop points)
- [x] FLAC audio export (Ctrl+L)

## Tier 6 -- Track Navigation

- [x] Up to 8 tracks with track page navigation (Tab toggles pages of 4)
- [x] Direct track selection via Ctrl+1..8
- [x] Page-relative mute/solo (F9-F12 / Ctrl+F9-F12)
- [x] Cursor auto-switches page when navigating across page boundaries

## Tier 7 -- Editing UX

- [x] Block selection (Ctrl+B: rectangular region select, copy/cut/paste blocks)
- [x] Quit confirmation and dirty flag (`[*]` indicator, Y/S/Cancel dialog)
- [x] Note transpose (Shift+Up/Down: semitone up/down, works on cursor or block)
- [x] Pattern column headers ("Not In Vl Fx" labels above the grid)
- [x] Atomic save (write to temp file then rename)
- [x] Interpolation tool -- fill volume/effect ramps between two points in a selection (Ctrl+I)
- [x] Follow mode toggle -- option to make cursor follow playback position (Ctrl+F, on by default)
- [x] Channel rename -- name channels ("Kick", "Bass") shown in header (Ctrl+R)

## Done

- [x] MIDI port selection UI (F2)
- [x] Help screen (F1) with scroll support
- [x] Visual feedback on playback (scrolling highlight)
- [x] Ableton Link integration (F3)
- [x] Built-in synth rewrite (9 patches: PolyBLEP oscillators, SVF/Moog filters, ADSR envelopes)
- [x] User-configurable synth patches (per-instrument waveform, ADSR, filter, detune via Tab from F7)
- [x] FundspPad patch proving fundsp synthesis works in audio callback
- [x] Headless playback (`--play`, `--loops N`)
- [x] App module split (app.rs -> app/mod.rs, input.rs, playback.rs)

## Effects

- [x] Arpeggio (0xy)
- [x] Portamento up/down (1xx/2xx)
- [x] Tone portamento (3xx)
- [x] Vibrato (4xy)
- [x] Volume slide (5xy)
- [x] Set speed/tempo (Fxx)
- [x] Note delay (6xx)

## Bugs (from code review)

- [x] WAV export missing sub-tick effects -- exported audio loses portamento, vibrato, arpeggio, volume slide (ticks 1+ not processed in `render_to_wav`)
- [x] Pattern break row ignored in WAV export -- Dxx jumps to row 0 instead of row xx (already uses break_row correctly)
- [x] Mouse click bounds check -- clicking beyond channel count can set `cursor_channel` out of bounds (bounds check exists)
- [x] MIDI velocity import reversed -- `midi_file.rs` drops standard-velocity (0x7F) notes on import (logic is correct: skips 0x7F to avoid cluttering tracker, preserves non-default velocities)
- [x] MIDI channel clamping -- tracker channels >16 get invalid MIDI channel numbers (already uses `ch & 0x0F`)
- [x] Path traversal on sample load -- `resolve_relative()` strips `..` components and reduces absolute paths to filename

## Audio Engine Improvements

- [ ] Better sample interpolation -- cubic or sinc instead of linear, to reduce aliasing at high pitch ratios
- [ ] Smarter voice stealing -- consider voice loudness/importance, not just FIFO age
- [x] ADSR envelope on sample voices -- 2ms attack, full sustain, 50ms exponential release
- [ ] More effects -- chorus, filter, distortion/saturation
- [ ] Per-channel effects routing -- route individual channels through different effect chains
- [ ] Replace `Arc<Mutex<AudioState>>` with lock-free approach (triple-buffering or crossbeam channel) for real-time audio callback

## MIDI Improvements

- [ ] MIDI channel pressure (aftertouch) support
- [ ] CC/pitch bend import from .mid files
- [ ] MIDI input CC/pitch bend/program change handling (currently only Note On/Off)
- [ ] MIDI learn -- map a CC to a parameter
- [ ] External MIDI clock input/sync (currently output-only)

## Longer-term Features

- [ ] Per-channel volume faders / mixer view
- [ ] Row highlight configurability (beat/bar intervals beyond 4/16, for time signature support)
- [ ] Recent files list for quick re-open
- [ ] Auto-save to temp file periodically
- [ ] Pattern matrix view (arrangement overview)
- [ ] Piano roll view (alternative note entry)
- [ ] Plugin hosting (VST/CLAP)
- [ ] Tempo automation (BPM changes beyond Fxx effect)
- [ ] Swing/groove (non-uniform tick spacing)
- [ ] Audio recording to sample slots
- [ ] More export formats (OGG)
