# Changelog

All notable changes to rtrack will be documented in this file.

## [Unreleased]

### Added (File Format)

- Extended `.rtrk` file format to persist instrument definitions and sample references
  - Instrument slots: name, MIDI program, and sample assignment saved per non-empty slot
  - Sample refs: source file path (relative to `.rtrk`), base note, trim, and loop points
  - On load, samples are reloaded from disk and metadata re-applied; missing files warn but do not block
  - Fully backwards-compatible: old `.rtrk` files without these fields load fine
- 2 new tests: SongFile roundtrip with instruments/samples, backwards compatibility with old format

### Added (Track Pages & Sample Directory)

- Track page navigation: Tab/Shift-Tab cycles between pages of 4 tracks (e.g., tracks 1-4 and 5-8)
- Direct track selection: Ctrl+1 through Ctrl+8 jump to specific tracks and auto-switch page
- Up to 8 channels supported with page-relative F9-F12 mute/solo bindings
- Arrow key navigation auto-switches page at channel boundaries
- Header shows current channel number and track page
- Sample directory loading: `--sample-dir <path>` loads all `<slot>-<name>.wav` files from a directory
- Optional `samples.json` metadata file for BPM, base notes, and loop points per sample
- Note-off keybinding changed from Ctrl+1 to `=` (on Note sub-column in Insert mode)
- 5 new tests: track page toggling, Ctrl+track selection, sample directory loading, metadata parsing

### Added (Samples)

- Sample loading from WAV and AIFF files via [hound](https://crates.io/crates/hound) + [dasp](https://crates.io/crates/dasp)
  - WAV: 8/16/24/32-bit integer and float formats
  - AIFF: uncompressed 8/16/24/32-bit with 80-bit extended sample rate parsing
  - Automatic mono-to-stereo conversion
- Sample playback engine with linear-interpolated pitch shifting
  - Playback rate derived from MIDI note vs sample base note: `2^((note - base_note) / 12)`
  - Sample rate conversion integrated into playback rate
  - Loop support (start/end points) for sustained playback
  - Up to 32 simultaneous voices with automatic voice stealing
- Per-instrument sample assignment: each instrument slot can reference a sample bank slot
  - When a note triggers with a sample-assigned instrument, audio routes to sample engine
  - Without a sample, falls back to fundsp synth or SF2
- Sample editor (Enter from instrument list):
  - Editable fields: base note, trim start/end, loop enable/start/end
  - Text-based waveform preview
  - Tab navigation between fields, Up/Down/Left/Right to adjust values
- WAV audio export (Ctrl+W): offline renders entire song to 16-bit stereo WAV
  - Renders fundsp synth + sample playback + effects chain
  - 2-second reverb tail appended automatically
- CLI sample loading: `--sample 0:kick.wav --sample 1:snare.wav`
- SampleEditor mode with waveform display and parameter editing
- 22 new tests covering sample loading, playback, voice management, WAV export, waveform rendering, and AIFF parsing

### Added (Effects)

- Sub-tick playback engine: each row is divided into `speed` ticks (default 6), enabling per-tick effect processing
- Per-channel effect state tracking (pitch offset, volume, vibrato phase, portamento target)
- Arpeggio effect (0xy): cycles pitch between note, note+x, note+y semitones each tick
- Portamento up (1xx): slides pitch up by xx per tick via MIDI pitch bend
- Portamento down (2xx): slides pitch down by xx per tick via MIDI pitch bend
- Tone portamento (3xx): glides from current note toward a target note at speed xx
- Vibrato (4xy): sine-wave pitch modulation with speed x and depth y
- Volume slide (5xy): increases volume by x or decreases by y per tick (sent as MIDI CC 7)
- Set speed/tempo (Fxx): xx < 0x20 sets ticks-per-row, xx >= 0x20 sets BPM
- Note delay (6xx): delays note trigger by xx ticks within the row (works for both note-on and note-off)
- MIDI pitch bend support in MidiEngine and AudioEngine
- Pitch bend reset on new notes and on playback stop
- 13 new tests covering arpeggio, portamento up/down, tone portamento, vibrato, volume slide (up/down/clamp), set speed, set tempo, sub-tick timing, note delay (on/off)

### Added (Audio Engine)

- Built-in synthesizer via [fundsp](https://crates.io/crates/fundsp) -- rtrack now makes sound out of the box with no external synth or SF2 file required
  - 8 built-in patches: Saw, Square, Sine, Triangle, Pulse, FM Bell, Organ, Noise
  - Per-channel program change (effect `Exx`) selects patch (0-7, wraps)
  - Polyphonic voice management via fundsp Sequencer with fade-in/out to avoid clicks
  - Status bar shows "SYNTH" when built-in synth is active
- Optional SoundFont audio engine via `--sf2 path/to/file.sf2` CLI flag
  - Uses [rustysynth](https://github.com/sinshu/rustysynth) (pure Rust SF2 synthesizer) + [cpal](https://crates.io/crates/cpal) (cross-platform audio output)
  - When SF2 is loaded, it handles note playback instead of the built-in synth
  - Status bar shows "SF2" indicator when SoundFont is active
- Stereo reverb effects chain via fundsp, applied to mixed audio output
  - Status bar shows "FX" when effects are enabled
- MIDI output remains primary path; audio engine runs alongside
- All note playback, CC, and program change messages dispatched to both MIDI and audio simultaneously
- CLI parsing via [clap](https://crates.io/crates/clap) with `--sf2` flag and positional file argument
- 11 new audio tests covering synth patches, voice lifecycle, polyphony, effects chain, and error handling

### Added (Tier 4 - Polish)

- Song settings dialog (F6): edit title, BPM, speed, channel count, and default rows with Tab navigation
- Instrument list view (F7): 256 instrument slots with editable names, scrollable list
- Order list sidebar: always-visible sidebar showing order positions with current position highlighted
- Color theme system (F8): cycle between dark (default), light, and monokai themes
- Mouse support: left-click to position cursor in pattern editor, scroll wheel to navigate rows
- Export to standard MIDI file (Ctrl+E): writes format-1 .mid file with tempo track + channel tracks
- Import from .mid: pass a .mid file as CLI argument to load it into the tracker
- MIDI clock output (Ctrl+M): sends 24 ppqn clock, start (0xFA) and stop (0xFC) messages for external sync
- 27 new tests covering song settings, instrument list, theme cycling, MIDI clock, mouse, MIDI export/import

### Added (Tier 3 - Quality of Life)

- Edit step configuration: `(` / `)` keys to decrease/increase (0-16), displayed in header
- Row insert/delete within pattern: `Insert` to insert empty row, `Backspace` to delete row (Normal mode)
- MIDI input for note entry: creates virtual port `RTRACK_MIDI_IN`, auto-enters notes in Insert mode with velocity
- Per-pattern length: each pattern tracks its own row count independently
- MIDI CC support via effect column: `Cxx` effect sends CC (controller number from instrument column, value xx)
- Program change via effect column: `Exx` effect sends MIDI program change to program xx
- 18 new tests covering edit step, row insert/delete, per-pattern length, MIDI CC/program change effects, MIDI input

### Added

- Save/load songs as `.rtrk` JSON files (Ctrl+S to save, CLI arg to load)
- Undo/redo with snapshot-based history, capped at 100 levels (Ctrl+Z / Ctrl+Y)
- Copy/cut/paste entire rows across all channels (Ctrl+C / Ctrl+X / Ctrl+V)
- Multiple pattern support: create new pattern (Ctrl+N), clone current (Ctrl+D)
- Order list navigation with Ctrl+Left/Right to move between order positions
- Order list editing: insert entry (F4), remove entry (F5)
- Per-channel MIDI channel mapping (tracker channel -> MIDI channel 0-15)
- Channel mute/unmute via F9-F12 with visual dimming on muted channels
- Status message bar that shows feedback for save/load/undo/copy/mute actions
- File path argument support (`cargo run -- song.rtrk`)
- serde serialization for all data model types (Song, Pattern, Cell, Note, NoteValue)
- 17 new tests covering save/load roundtrip, undo/redo, copy/cut/paste, order navigation, pattern create/clone, order insert/remove, channel mute, MIDI channel mapping, keybinding dispatch

### Changed

- Order position is now tracked per-session (`edit_order` field) instead of hardcoded to 0
- Note preview and playback now use per-channel MIDI mapping instead of positional channel index
- Help popup updated with new keybindings
- Status bar shows transient status messages, falls back to key hints when idle

## [0.1.0] - 2026-03-05

### Added

- Pattern editor with 4 channels x 64 rows
- Cursor navigation (arrows, PgUp/PgDn, Home/End)
- Normal and Insert input modes (Esc to toggle)
- Piano keyboard note entry (two-row layout spanning two octaves)
- Hex digit entry for instrument, volume, and effect columns
- Note-off entry and cell clearing (Delete/Backspace)
- Virtual MIDI port (`RTRACK_MIDI`) created on startup (macOS/Linux), visible to DAWs
- Fallback to first available MIDI port on platforms without virtual port support
- MIDI port selection popup (F2) to switch between virtual and hardware ports
- Ableton Link tempo synchronization via rusty_link (F3 to toggle)
  - Bidirectional BPM sync with Link peers
  - Transport start/stop sync with Link session
  - Peer count displayed in header bar
- Tab/Shift-Tab to jump between tracks
- Help screen (F1) showing all keybindings
- Note preview on entry (sends MIDI note-on when inserting notes)
- Real-time pattern playback with classic tracker timing (BPM * speed)
- Playback toggle via Space
- Active note tracking with automatic note-off on channel reuse
- Beat highlighting (every 4th row) and bar highlighting (every 16th row)
- BPM adjustment with [ / ] keys
- Octave selection with + / - keys
- Header bar showing song title, BPM, speed, pattern/order position, octave
- Status bar showing current mode, MIDI connection status, and key hints
- Song data model with multiple patterns and order list
- 40 unit tests covering data model, MIDI engine, Link engine, input handling, and UI logic
