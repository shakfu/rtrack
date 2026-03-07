# Changelog

All notable changes to rtrack will be documented in this file.

## [Unreleased]

### Added (Timing, Groove & Auto-save)

- Auto-save: periodically saves to `.{filename}.autosave` every 60 seconds when the song has unsaved changes
  - Autosave file is cleaned up on manual save or quit
  - Uses the same atomic write strategy (temp file + rename) as manual save
- Row highlight configurability: `highlight_beat` and `highlight_bar` fields on Song (defaults 4 and 16)
  - Configurable via Song Settings dialog (F6): Beat Highlight (1-64) and Bar Highlight (1-256)
  - Pattern editor uses song values instead of hardcoded 4/16
  - Backwards-compatible via `#[serde(default)]`
- Tempo automation: `tempo_map` field on Song stores `Vec<TempoPoint>` (order, row, bpm)
  - Tempo changes checked during `advance_playback()` and applied to live BPM
  - Offline export (WAV/FLAC) also respects tempo automation points
- Swing/groove: `swing` field on Song (0-100, default 50 = no swing)
  - Even rows get `swing/50` of base tick time, odd rows get `(100-swing)/50`
  - Total time of an even+odd row pair is conserved (equals 2x base time)
  - Applied in both live playback and offline export
  - Configurable via Song Settings dialog as percentage
- Configurable pitch bend range per instrument: `pitch_bend_range` field on InstrumentDef (default 2.0 semitones)
  - All pitch bend effects (arpeggio, portamento, tone portamento, vibrato) use per-instrument range
  - `ChannelState.active_instrument` tracks which instrument is playing on each channel
  - `channel_pitch_bend_per_semitone()` helper replaces the old global constant
- Link beat timeline: `LinkEngine::beat_at_time_now()` captures current beat position
  - `tick_playback()` uses beat delta instead of wall-clock delta when Link is enabled
  - More accurate sync with Link peers, avoids drift from accumulating time deltas
- 16 new tests: auto-save (3), highlight (2), swing (4), tempo automation (2), pitch bend range (3), Link beat (1), backwards compat (1)
- Test count increased from 285 to 301 (289 unit + 12 integration)

### Added (Sample Slicing & File Browser)

- Sample slicing in the sample editor (Enter from instrument list):
  - Equal-segment slicing: divides sample into N equal parts (configurable slice count)
  - Transient detection: RMS energy envelope derivative with ~5ms windows, 50ms minimum onset gap, configurable sensitivity (0.0-1.0)
  - Slice results automatically create new instruments and sample refs with correct trim points
  - Three new functions: `slice_equal()`, `detect_transients()`, `slice_at_points()`
- File browser dialog (`:load` / `:open` commands, or Enter on Load field in Track Config):
  - Directory navigation with keyboard (Up/Down/PgUp/PgDn/Home/End/Backspace/Enter/Esc)
  - Extension filtering (`.wav`/`.aiff` for samples, `.rtrk`/`.mid` for songs)
  - Scrollable file list with cursor highlight
  - Reusable `FileBrowserAction` callback pattern (LoadSample, OpenSong)
- Track Config "Load" field for Sample-type channels: press Enter to open file browser for sample loading
- New vim commands: `:load` (sample file browser), `:open` (song file browser)
- New example files:
  - `examples/sliced-amen.rtrk`: 8 equal slices of amen.wav at 170 BPM, speed 3, 32 rows
  - `examples/drumloops.rtrk`: 8 amen.wav slices with loop points enabled at 130 BPM, speed 6, 64 rows
- 22 new tests: 12 sample slicing (equal/transient/edge cases), 9 file browser + slice integration, 1 example generation
- Test count increased from 263 to 285 (273 unit + 12 integration)

### Added (Extended Synth & Track Config)

- 30 built-in synth patches (up from 9), selected via `Exx` program change (0-29):
  - Original 9: Saw, Square, Sine, Triangle, Pulse, FM Bell, Organ, Noise, Fundsp Pad
  - Batch 1 (9): Bass, Pluck, Pad, Lead, Keys, Brass, Strings, Perc, Sub
  - Batch 2 (12): Acid, Chip, Stab, Mallet, Flute, Reese, Wire, Chime, Growl, Whistle, Siren, Dist
- Extended DSP capabilities for built-in synth:
  - Filter type selection: LowPass, HighPass, BandPass (SVF computes all three, now selectable)
  - Sub-oscillator: sine one octave below, mixable 0.0-1.0
  - Configurable FM synthesis: carrier:modulator ratio (0-16) and modulation index (0-10)
  - Variable pulse width (0.05-0.95) for Pulse waveform
- Per-channel audio effects chain: distortion, SVF filter, LFO chorus, stereo delay, Schroeder reverb
  - Each effect independently toggleable per track via Track Config
  - Delay: configurable time (10-2000ms), feedback (0-0.95), wet mix
  - Reverb: Schroeder algorithm (4 comb + 2 allpass filters), configurable size, damping, wet mix
  - Effects hidden for MIDI-type tracks (only apply to Synth/Sample tracks)
- Track Config popup (Enter key on channel header):
  - Channel type (Midi/Synth/Sample), MIDI channel, default instrument (Synth tracks only)
  - All per-channel effects with checkbox-style enable/disable indicators
  - Enter/Esc to close, Left/Right arrows to adjust values
- Tab/Shift+Tab now cycles through all tracks with wrapping (replaces page-based navigation)
- Per-track default instrument for Synth-type tracks, auto-filled on note entry and used as fallback during playback
- Synth editor extended with 5 new parameters: Filter Type, Sub Osc, FM Ratio, FM Index, Pulse Width
- Test count increased from 209 to 234

### Fixed (Track Config)

- Fixed track default instrument not affecting playback: cells without an explicit instrument value now fall back to the track's default instrument during playback (Synth tracks only). Previously, changing the instrument in Track Config only affected newly entered notes.

### Added (Architecture & Audio Improvements)

- Lock-free audio engine: replaced `Arc<Mutex<AudioState>>` with a lock-free SPSC command queue (`rtrb`). UI thread sends commands (note on/off, CC, pitch bend, etc.) via ring buffer; audio callback owns all synth/sample state and drains commands each callback. No mutex held during rendering.
- Shared ADSR envelope: extracted unified `Envelope` type (`src/audio/envelope.rs`) used by both the built-in synth and sample playback engine, eliminating 90+ lines of duplicated envelope code
- Per-channel volume: `channel_volumes` field on App, applied as velocity scaling before note-on during playback; resizes with channel count changes
- Cubic hermite sample interpolation: 4-point Catmull-Rom interpolation replaces linear, reducing aliasing at high pitch ratios
- CC/pitch bend MIDI import: `.mid` import now captures CC events (mapped to `Cxx` effect), program changes (mapped to `Exx` effect), and pitch bend instead of discarding them
- Smarter voice stealing: both synth and sample engines now steal the quietest voice (lowest `envelope_level * velocity`) instead of the oldest when at capacity
- Playback time display: header shows elapsed time as `M:SS` during playback
- Mute/solo indicators: "M" and "S" indicators shown in channel column headers when muted or soloed
- Minimum terminal size check: displays "Terminal too small" message instead of panicking when terminal is under 40x10
- MIDI send error feedback: status bar shows `MIDI:ERR(N)` with consecutive failure count when MIDI output disconnects mid-session

### Changed (App Organization)

- Reorganized App struct fields under clear section comments: Cursor State, Playback State, Editor State, Dialog State
- Test count increased from 203 to 209

### Added (FLAC Export, Sample ADSR)

- FLAC audio export (Ctrl+L): lossless audio export alongside existing WAV export
  - Uses `flacenc` crate (pure Rust FLAC encoder)
  - Same offline render pipeline as WAV export
- ADSR envelope on sample voices: eliminates clicks on note-on and note-off
  - 2ms attack ramp (avoids click on trigger)
  - Full sustain while note is held
  - 50ms exponential release on note-off (smooth fade instead of instant silence)
  - Envelope applied per-voice in the sample playback engine

### Fixed (WAV Export, Path Traversal)

- Fixed WAV export missing sub-tick effects: portamento, vibrato, arpeggio, and volume slide now correctly modify synth pitch and sample playback rate during offline render
  - Added `set_channel_pitch_offset()` and `set_channel_volume()` to BuiltinSynth and SamplePlaybackEngine
  - Synth voices now support a `pitch_offset` field applied in the oscillator
  - Arpeggio cycles pitch, vibrato modulates pitch, portamento slides pitch -- all audible in exported audio
- Fixed path traversal vulnerability in sample loading: `resolve_relative()` now strips `..` components from relative paths and reduces absolute paths to just the filename, preventing malicious .rtrk files from accessing files outside the song directory
- 6 new tests: portamento in export, volume slide in export, path traversal sanitization, FLAC export, sample envelope fade, sample note-off release
- Test count increased from 198 to 203

### Added (Interpolation, Follow Mode, Channel Names)

- Interpolation tool (Ctrl+I): fill volume and effect value ramps across a block selection
  - Linearly interpolates between first and last row values for each channel in the block
  - Volume interpolation: fills when both endpoints have a volume value
  - Effect interpolation: fills when both endpoints have the same effect command
  - Requires an active block selection (Ctrl+B) with at least 2 rows
- Follow mode toggle (Ctrl+F): cursor follows playback position
  - When enabled, cursor_row and edit_order sync to playback position each tick
  - On by default; "FLW" indicator shown in header when active
  - Toggle off to freely navigate while playback continues
- Channel rename (Ctrl+R): name channels with custom labels
  - Opens a text input popup for the current channel (max 10 characters)
  - Channel names display in the column header row, replacing "Not In Vl Fx" labels
  - Unnamed channels continue to show the default column labels
  - Names resize with channel count changes and reset on file load
- 6 new tests covering follow mode toggle, channel rename, interpolation (volume, effect, no-block error)
- Test count increased from 192 to 198

### Added (Block Selection, Transpose, UX)

- Block selection (Ctrl+B): rectangular region select in the pattern grid
  - Toggle anchor at cursor position, then move cursor to define the block
  - Ctrl+C/X/V copies, cuts, or pastes the 2D block (rows x channels)
  - Block is visually highlighted in the pattern editor
  - Separate block clipboard (`Vec<Vec<Cell>>`) independent of row clipboard
  - Paste inserts at cursor position, clipping to pattern bounds
- Note transpose (Shift+Up/Down): transpose notes by semitone
  - Works on the note at the cursor position
  - When a block selection is active, transposes all notes in the block
  - Handles octave wrapping and clamps to valid MIDI range (0-127)
- Quit confirmation and dirty flag:
  - `dirty` flag tracks unsaved changes (set on any edit, cleared on save/load)
  - `[*]` indicator in header bar when song has unsaved modifications
  - Pressing `q` with unsaved changes shows a confirmation dialog: [Y] Quit, [S] Save & Quit, [Any] Cancel
  - Clean songs quit immediately as before
- Pattern column headers: "Not In Vl Fx" labels displayed above the pattern grid for each visible channel
- Atomic save: file writes go to a temp file first (`.rtrack_save_<pid>_<filename>.tmp`), then rename to the target path, preventing corruption on crash
- 11 new tests covering dirty flag, quit confirmation, note transpose, block select/copy/cut/paste, and atomic save
- Test count increased from 181 to 192

### Added (Built-in Synth Rewrite)

- Replaced fundsp Sequencer-based synth with custom subtractive synthesizer
  - PolyBLEP anti-aliased oscillators (saw, square, pulse) for clean waveforms
  - State-variable filter (SVF) with 2x oversampling per voice
  - Per-voice ADSR envelope with exponential release
  - Filter envelope modulation: cutoff tracks envelope depth in octaves
  - Detuned second oscillator for chorus/thickening (per-patch configurable)
  - Manual voice pool (up to 32 voices) with automatic voice stealing
- 9 built-in patches (up from 8), selected via `Exx` program change (0-8):
  - 0: Saw -- PolyBLEP, detuned pair, filtered
  - 1: Square -- PolyBLEP, detuned pair, filtered
  - 2: Sine -- clean, wide-open filter
  - 3: Triangle -- detuned pair, lightly filtered
  - 4: Pulse -- 25% duty cycle, heavy filter envelope
  - 5: FM Bell -- 2-operator FM (ratio 3.5), envelope-modulated depth
  - 6: Organ -- additive (3 harmonics)
  - 7: Noise -- LCG noise through resonant filter with envelope
  - 8: Fundsp Pad -- fundsp-based synthesis (detuned saws through Moog filter)
- FundspPad patch proves fundsp synthesis works correctly in the audio callback without the Sequencer pattern that caused the original issues
- New example: `examples/fundsp-pad.rtrk` -- pad chord progression (C-Am-F-G) using the FundspPad patch
- Updated `examples/all-patches.rtrk` to include all 9 patches

### Added (User-Configurable Synth Patches)

- Per-instrument synth parameters: each instrument can now define its own waveform, envelope, filter, and detune settings
  - Waveform (0-8), Attack, Decay, Sustain, Release, Filter Cutoff (freq multiplier), Filter Resonance, Filter Envelope (octaves), Detune (cents)
  - When an instrument has synth params, they override the channel's default patch (set by `Exx` effect)
  - Instruments without synth params continue to use the channel default (backwards compatible)
- Synth editor UI (Tab from instrument list):
  - 9 editable fields with real-time adjustment (Up/Down +/-1, Left/Right +/-10)
  - Tab/Shift-Tab to navigate between fields
  - Delete key clears custom params (reverts to channel default)
  - Initializes from preset defaults when first opened
- `SynthParams` struct persisted in `.rtrk` files (optional, backwards-compatible via serde defaults)
- Note routing priority: sample engine > custom synth params > channel default synth
- WAV export respects per-instrument synth params in offline render
- 4 new tests covering custom params audio output, preset roundtrip, synth editor open/close/adjust, and delete-clears

### Changed (App Module Split)

- Split monolithic `src/app.rs` (3793 lines) into focused submodules:
  - `src/app/mod.rs` -- App struct, state management, undo/redo, file I/O, tests
  - `src/app/input.rs` -- keyboard/mouse input handling, mode dispatch
  - `src/app/playback.rs` -- playback engine, tick processing, effect state
- No public API changes; all existing tests continue to pass

### Fixed
- Fixed audio distortion caused by heavy FDN reverb (`reverb_stereo`) in the real-time audio callback. Replaced with a lightweight stereo delay effect (80ms L / 120ms R, 15% wet mix).
- Fixed notes sustaining indefinitely when entered in the tracker. Added preview note tracking with 250ms auto-expiry so notes are properly released after entry.
- Fixed instant click on note-off. Corrected `edit_relative` fade window timing so voice fade-outs start from the current time rather than retroactively.
- Fixed heap allocation in the audio callback (`vec![]` per callback). Pre-allocated scratch buffers in `AudioState` to avoid real-time thread allocations.
- Fixed flaky `test_link_play_stop` test by adding a delay for Link state propagation.

### Changed
- Added `[profile.dev] opt-level = 1` so the rtrack crate's own audio callback code is optimized in debug builds, preventing buffer underruns.
- Effects chain now uses stereo delay instead of FDN reverb for reliable real-time performance.
- Test count increased from 165 to 181.

### Added (Headless Playback)

- `--play` CLI flag: play a `.rtrk` file from the command line without launching the TUI
  - Audio engine runs normally (built-in synth, SF2, samples, effects) -- just no terminal UI
  - `--loops N` option: repeat the song N times (default 1, 0 = infinite loop until Ctrl+C)
  - Prints song info (title, BPM, pattern count) to stderr on start
  - Exits cleanly after the specified number of loops
- Playback generation tracking (`playback_generation` counter) detects when the order list wraps
  - Correctly handles `Bxx` position jump effects that loop back to earlier order positions
  - `Dxx` pattern break and normal order-list wrap also tracked

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

- Built-in synthesizer -- rtrack now makes sound out of the box with no external synth or SF2 file required
  - 9 built-in patches: Saw, Square, Sine, Triangle, Pulse, FM Bell, Organ, Noise, Fundsp Pad
  - Per-channel program change (effect `Exx`) selects patch (0-8, wraps)
  - Polyphonic voice management with per-voice ADSR envelopes and filtered output
  - Status bar shows "SYNTH" when built-in synth is active
- Optional SoundFont audio engine via `--sf2 path/to/file.sf2` CLI flag
  - Uses [rustysynth](https://github.com/sinshu/rustysynth) (pure Rust SF2 synthesizer) + [cpal](https://crates.io/crates/cpal) (cross-platform audio output)
  - When SF2 is loaded, it handles note playback instead of the built-in synth
  - Status bar shows "SF2" indicator when SoundFont is active
- Stereo effects chain via fundsp, applied to mixed audio output
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
