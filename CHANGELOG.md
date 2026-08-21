# Changelog

All notable changes to rtrack will be documented in this file.

## [Unreleased]

### Fixed

- **A note on a tracker channel now takes that channel over rather than layering on top of what was already sounding there.** The sample route was the only one that let notes accumulate on a channel: a pattern can say only "stop this channel" (`Note::Off` emits `TrackerEvent::NoteOff { channel }`; there is no per-note off), per-channel effect state holds a single `porta_target`/`vibrato_phase`/`pitch_offset` that `set_channel_pitch_offset` applies to every voice on the channel, the SoundFont route already cut the channel on every note-on, and live MIDI is monophonic one layer up in `preview_note`. Consecutive slices of a break therefore piled up instead of chopping. Notes from a pattern row now cut the channel, fading the outgoing voice over the de-click ramp rather than dropping it; previews and live MIDI still stack. **This changes how existing songs that rely on notes overlapping on one channel play** (`sample/playback.rs`, `audio/mod.rs`, `core.rs`, `sample/export.rs`)
  - New: `NewNoteAction::{Cut, Continue}`, taken by `SamplePlaybackEngine::note_on` and carried on `AudioCommand::SampleNoteOn`
- **Reloading a sliced song decoded the source file once per slice and gave every slot its own copy of the audio.** Slices are stored as one path repeated with different spans, and the load path called `SampleBank::load` for each entry -- so the shared buffer that slicing exists to preserve was rebuilt N times on the way back in, undoing in-memory work the format itself does correctly. An eight-way slice of the bundled amen break returned as eight decodes and eight copies. Each distinct path is now decoded once and its buffer shared across every slot referencing it (`core.rs`)
- A looping sample pitched up far enough to cover its whole loop in one frame walked out of the loop and never came back, falling silent when it ran off the end of the buffer. The wrap subtracted the loop length once rather than taking the position modulo it; measured with an eight-frame loop at rate 16, the voice reached frame 1648 against a `loop_end` of 108 (`sample/playback.rs`)
- Enabling the loop on a slice replayed the whole source file. A slice is a span of a shared buffer, but `effective_loop_start` clamped only against the loop end, so the default `loop_start` of 0 meant the start of the file rather than the start of the slice. Loop points are now clamped into the played span at both ends (`sample/mod.rs`)
- Sample voices no longer click when they stop. A slice ends at an arbitrary frame, so dropping the voice on reaching it left a step in the output -- a full-scale one for a loud slice -- and voice stealing did the same by removing a sounding voice outright. Both now fade over `SAMPLE_DECLICK_SECS` (5 ms): the tail fade stays inside the slice's own span so it never reads a neighbouring slice's audio, and a stolen voice is put into a fast release instead of being removed. A voice already below `SAMPLE_INAUDIBLE_LEVEL` is still dropped, there being nothing to fade (`sample/playback.rs`, `constants.rs`)
- `detect_transients_range` panicked when given a range past the end of the buffer instead of clamping to it, and returned `vec![0]` rather than the requested start for an empty range (`sample/mod.rs`)
- A song whose source audio has been replaced with something shorter no longer loads as silently silent slots; spans and loop points are clamped on load to the audio actually present (`core.rs`)
- A sample voice on a channel with no output buffer was folded onto the last channel, running it through effects meant for other material; it is now dropped (`sample/playback.rs`)
- **Note entry on a sample track previews the sample instead of the built-in synth.** Entering or editing a note resolved its instrument only from the track default (and only for Synth- and Sample-typed tracks) or from whatever the cell already contained, so typing on an empty row of a sliced-sample track resolved to nothing and fell through to the synth -- while playback of the same pattern correctly played the slices. `examples/sliced-amen.rtrk` showed this clearly: its slices are separate instruments named per cell, and because the song predates persisted channel state it loads as a Midi-typed track with no default instrument. Instrument resolution is now shared by every entry path, adding the tracker convention that the instrument column is sticky: a note inherits the nearest instrument above it in the same channel. The resolved instrument is both previewed and written to the cell, so what you hear while editing is what plays back (`core.rs`, `rtrack-tui/src/app/input.rs`, `playback.rs`, `rtrack-gui/src/input.rs`)
  - MIDI step and punch-in recording previewed with no instrument at all, always through the synth; they now use the same resolution
  - New: `TrackerCore::resolve_edit_instrument()` and `preview_note_for_cell()`
- One-shot sample previews are no longer cut off after 250 ms. A sliced amen break runs to roughly 340 ms, so the preview was truncated; samples that end by themselves are now left to ring, while sustaining sources (synth, external MIDI, looping samples) are still stopped on the timeout. `TrackerCore::preview_note` is now a `PreviewNote` struct rather than a tuple (`core.rs`, `constants.rs`)
- **Saving a song no longer destroys pattern reuse.** `save()` rebuilt the pattern list from the derived phrase model, emitting one pattern per order position: a song whose order was `[0, 0, 0]` became three separate patterns after saving, and edits to a shared pattern stopped applying everywhere. The rebuild is gone; saving now serializes the model as it stands (`core.rs`, `tracker/song.rs`)
- **Per-channel configuration is now persisted.** Channel name, type, volume, pan, MIDI channel, default instrument, per-channel effects, send bus settings and MIDI-learn CC mappings were never written to `.rtrk` files and were silently reset to defaults on load (`types.rs`, `tracker/song.rs`, `core.rs`)
- Songs containing an instrument with no MIDI program could not be loaded back: `InstrumentDef::midi_program` and `sample_index` had `skip_serializing_if` without a matching `default`, so rtrack wrote files it could not read. `examples/four-on-the-floor.rtrk` was among the affected files (`tracker/song.rs`)
- Note values spelled `C#`/`F#` by older versions are accepted on load again via serde aliases; `examples/four-on-the-floor.rtrk` had become unloadable (`tracker/pattern.rs`)
- A panic in the TUI no longer leaves the terminal in raw mode on the alternate screen with its backtrace written where nobody can see it; a panic hook restores the terminal first (`rtrack-tui/src/main.rs`)
- Malformed or hand-edited song files are repaired on load instead of panicking the editor on the next redraw. `Song::repair()` drops order entries pointing at missing patterns, conforms ragged pattern data to its declared geometry, replaces zero channel/row/speed/bpm values, and drops out-of-range tempo points, reporting what it changed (`tracker/song.rs`, `core.rs`)
- Render paths no longer index with `[]`. `Song::pattern_at()`/`pattern_at_mut()` are used throughout the TUI pattern editor and matrix and the GUI grid, transport, sidebar, matrix and dialogs (`tracker/song.rs`, both frontends)
- GUI note entry and note-off panicked when the edit cursor pointed at an out-of-range order position (`rtrack-gui/src/input.rs`)
- Swing applied the *next* row's timing factor to the current row. `TrackerEngine.row`/`order` are a write pointer that moves past a row as soon as its notes are emitted; timing and the UI playback cursor now use the new `sounding_row`/`sounding_order`. **This changes how swung songs play** (`engine/mod.rs`)
- `SongFile::save` flushes the temp file to disk before renaming over the target, so a crash can no longer leave the directory entry pointing at unwritten content, and removes the temp file on every failure path instead of leaking it (`tracker/song.rs`)
- CLAUDE.md stated Rust 1.70+; the workspace requires 1.87

### Added

- **Sample-accurate note scheduling.** The sequencer runs off the audio device's frame clock rather than the UI thread's frame rate. Audio commands carry the frame they should sound at, and the callback renders each buffer in segments split at those frames. Previously note timing quantised to whichever frontend's loop happened to be running -- roughly 16.7 ms in the GUI against a 20.8 ms tick. Wall-clock timing is retained for headless and MIDI-only runs, where there is no frame clock to schedule against (`audio/mod.rs`, `core.rs`, `constants.rs`)
  - `TrackerCore::playback_position()` reports the position currently audible rather than the position the sequencer has run ahead to; both frontends follow it
  - `AudioEngine::frame_clock()`, `note_on_at()`, `note_on_with_params_at()`, `note_off_all_channel_at()`, `sample_note_on_at()`, `sample_note_off_channel_at()`
- Slicing left the first slot's instrument named after the whole sample, so a sliced break showed up as `amen` sitting alongside `amen_S01`..`amen_S07`. Slicing now names every slot it writes into, since it replaces the audio there (`core.rs`)
- **Slicing a sample no longer loses the slices when you save.** `.rtrk` files store a sample as a source path plus a frame span, not as audio, but `slice_equal` and `slice_at_points` copied the frames into detached samples with `trim_start`/`trim_end` left at zero. Saving therefore recorded no boundaries and reloading gave every slot the whole source file, so a sliced kit silently collapsed into N copies of the same break. Slices are now spans of a shared buffer -- `Sample::data` became `Arc<[[f32; 2]]>` -- which is both the representation the file format already expects and the one reloaded slices have always had (`sample/mod.rs`)
  - Total memory is unchanged for a normal slice-up (the slices previously partitioned the source between them; now they share one copy of it), and re-slicing no longer duplicates audio at all
  - New: `Sample::played_len()` and `played_duration()`, the trimmed extent, as distinct from `len()`/`duration()` which describe the whole buffer
- `test_generate_sliced_amen` silently skipped instead of running. It looked for its fixture under `CARGO_MANIFEST_DIR`, which is the crate directory rather than the workspace root, so `examples/data/amen.wav` was never found and the test returned early while still reporting success -- it had never asserted anything. It now resolves the workspace root, fails loudly if the committed fixture is missing, writes its output to a temp file rather than over `examples/sliced-amen.rtrk`, and additionally checks that the slices reference contiguous, non-empty spans of the source. Renamed to `test_sliced_sample_song_round_trips`, since it tests a round trip rather than generating anything (`rtrack-tui/tests/integration.rs`)
- `.gitignore` covers `*.autosave`, the files the editor writes beside a song while it runs
- `Ctrl+O` opens the song file browser, matching `Ctrl+S` for save. The commands `:open`, `:recent` and `:load` were previously the only route and were missing from the in-app help overlay (`F1`), so there was no discoverable way to open a file from inside the TUI; all four are now listed there (`rtrack-tui/src/app/input.rs`, `app/mod.rs`, `tui/mod.rs`, `rtrack-tui/README.md`, `README.md`)
- `cargo xtask regen-examples` rebuilds the generated example songs, and `--check` verifies the committed ones are current without writing (wired into `make ci` and the CI workflow). Regeneration goes through the app's own `slice_sample`, so it doubles as a check that slicing produces something that survives being saved. Replaces the regeneration that used to happen as a side effect of `cargo test` (`xtask/`, `.cargo/config.toml`, `Makefile`)
- `SongFile::to_json()`, so a caller can compare against a file without writing one
- CI (`.github/workflows/ci.yml`): formatting, clippy and tests on Linux and macOS, plus an MSRV build at the declared `rust-version`. `make fmt-check` (non-mutating) and `make ci` added; `make lint` still reformats in place, which is why it could not gate
- `rtrack_core::error::Error` and `Result`, replacing `Result<String, String>` across `TrackerCore`
- `rtrack_core::core::LoadReport`, returned by `load_file` and `import_midi_file`, carrying repairs, missing samples and a newer-version flag as data
- `rtrack_core::keymap`: the two-row piano key layout, previously duplicated verbatim in both frontends
- `Cell::transpose_note()`, replacing a near-identical `transpose_cell_note` in each frontend
- `.rtrk` files carry a `version` field (`FORMAT_VERSION`). Files predating it read as version 0; a file claiming a newer version loads but is reported as such
- `Song::repair()`, `Song::pattern_at()`, `Song::pattern_at_mut()`, `Song::rows_at()`, `Song::order_len()`, `Song::clamp_order_position()`, `Song::add_order_entry()`, `Song::clone_order_entry()`, `Song::remove_order_entry()`, `SongFile::from_song()`
- `AudioEngine::device_description()` and `take_stream_error()`; `config::load_config_verbose()`
- `TrackerCoreBuilder`: builder API for `TrackerCore` with `.headless()` (skips MIDI port and Link session creation), `.song_size(ch, rows)`, and `.midi()` / `.midi_input()` / `.link()` injection for tests and offline use (`core.rs`)
- Test coverage grew from 377 to 495. `rtrack-gui` went from zero tests to 27, via a `RtrackApp::with_core` constructor that needs no window or device; `rtrack-core/tests/persistence.rs` covers save/load round trips, format compatibility and load robustness; new unit tests cover the scheduler, the reclaim queue, the fundsp voice pool and the keymap; `rtrack-core/tests/slicing.rs` covers slice spans surviving a save and reload, buffer sharing across a reload and span clamping against a changed source file, and the sample engine's loop wrapping, de-click ramps and channel behaviour are covered in `sample/playback.rs`

### Changed

- **Transient detection rebuilt.** It ran on the linear RMS envelope with a threshold scaled to the loudest rise in the whole sample, so a single loud hit suppressed everything after it and the detector reliably found only the loudest part of a break. Detection now runs on the log-energy envelope compared against a local average, keeps only local peaks so one onset does not fire on every window of its attack, and backtracks each onset to the quietest nearby frame so a slice starts in the dip before the hit rather than partway up its attack. On the bundled amen break at the default sensitivity this goes from 14 onsets with a hole between 1.72 s and 2.06 s to 19 on a regular 0.172 s grid; a pair of hits 30 dB apart, of which the old detector found neither, now both register. **Slice points already saved in a file are unaffected, but re-running transient slicing gives different boundaries** (`sample/mod.rs`)
- The audio thread's `SampleNoteOn` handler was a verbatim copy of `SamplePlaybackEngine::note_on` -- some forty lines of rate calculation and voice allocation duplicated from the method the offline renderer calls -- so any fix to voice handling had to be made twice or the two would drift. It now calls the method. The two per-voice render loops in `render` and `render_per_channel` likewise collapse into a single `render_voice` (`audio/mod.rs`, `sample/playback.rs`)
- **Removed the Song > Chain > Phrase layer** added earlier in this cycle. `patterns` + `order` is again the single source of truth. The chain model was never reachable from either frontend, was rebuilt from patterns on every edit, doubled the size of saved files, and was the direct cause of the pattern-reuse bug above. Chain transpose goes with it -- every rebuild reset it to zero, so nothing usable is lost. Files containing the old `phrases`/`chains`/`arrangement` blocks still load; the fields are ignored (`tracker/pattern.rs`, `tracker/song.rs`, `engine/mod.rs`, `core.rs`)
  - Removed: `Phrase`, `Chain`, `ChainEntry`, `build_virtual_pattern()`, `chain_transpose_at()`, `set_chain_transpose()`, the `add_/clone_/remove_arrangement_row()` family, `mark_phrases_dirty()`, `sync_phrases_if_dirty()`, `rebuild_phrases_from_patterns()`, `rebuild_patterns_from_phrases()`
  - `Note::transposed()` now refuses a transpose that would leave the MIDI range instead of clamping to it; clamping collapsed the extremes of a transposed selection onto a single pitch
- The audio callback no longer allocates or frees. The `FundspPad` DSP graph is drawn from a pre-built pool and retuned through `Shared` values rather than constructed per note; a reclaim queue returns finished values -- a replaced `SampleBank`, boxed parameter structs -- to the UI thread to be dropped. Per-channel sample rendering takes buffers and a range directly, removing a per-callback allocation and a pointer-aliasing `unsafe` block (`audio/mod.rs`, `audio/synth.rs`, `sample/playback.rs`)
- Empty cells serialize as `{}` instead of five explicit nulls. Re-saving the bundled examples measured 12% to 70% smaller, the sparser the pattern the larger the saving (`tracker/pattern.rs`)
- `rtrack-core` no longer writes to stdout or stderr, and no longer formats messages for users. Success values are data (`PathBuf`, counts, `LoadReport`) and diagnostics are returned rather than printed; each frontend does its own wording. `toggle_solo` returns the soloed channel, `toggle_channel_mute` the new muted state, `toggle_midi_clock` a bool (`core.rs`, `config.rs`, `audio/mod.rs`, both frontends)
- GUI undo/redo application was duplicated between the Edit menu and the keyboard handler; both now call `apply_undo`/`apply_redo`, which skip history entries naming a pattern that no longer exists rather than indexing into it (`rtrack-gui/src/input.rs`, `menu.rs`)
- `SampleBank::load` refuses files larger than `MAX_SAMPLE_FILE_BYTES` (512 MB), so a mistyped path cannot become a multi-gigabyte allocation (`sample/mod.rs`, `constants.rs`)
- Cleared all 19 clippy warnings, including 16 `float_literal_f32_fallback` in `rtrack-gui` that were scheduled to become hard errors in a future rustc. `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` both pass
- `SampleBank` slots are now `Option<Arc<Sample>>` instead of `Option<Sample>`, making bank clones O(256 refcount bumps) instead of O(total audio frames). Mutations use `Arc::make_mut` for copy-on-write semantics (`sample/mod.rs`, `core.rs`, `app/input.rs`, `instrument_editor.rs`)
- `SampleBank::load_directory` now sorts directory entries by filename before loading, ensuring deterministic slot assignment across platforms (`sample/mod.rs`)
- Export tests (`test_render_empty_song`, etc.) no longer flake due to temp-dir races; switched from `std::env::temp_dir()` to `tempfile::tempdir()` for isolated per-test directories (`sample/export.rs`)
- GUI frontend now reads `sf2` and `sample_dir` from `~/.config/rtrack/config.toml` at startup and on File > New, matching TUI behavior (`app.rs`, `menu.rs`)

## [0.1.2]

### Fixed

- Loading a new song no longer keeps stale sample slots from the previous session; the sample bank is now cleared before loading referenced samples (`core.rs`)
- Fractional tempo automation values (e.g. 127.5 BPM from `tempo_map`) are no longer rounded to integers during playback; `TrackerEngine.bpm` and `TrackerEvent::TempoChanged` now use `f64` precision (`engine/mod.rs`)
- MIDI export now includes tempo changes from Fxx effects and tempo automation as `0xFF 0x51` meta-events on track 0, instead of writing only the initial song BPM (`midi_file.rs`)
- MIDI import now populates `Song.tempo_map` from tempo meta-events encountered after tick 0, preserving multi-tempo MIDI files instead of reducing them to a single BPM (`midi_file.rs`)

### Added (GUI Frontend - rtrack-gui)

- Built egui/eframe GUI frontend as alternative to TUI, sharing TrackerCore
  - Menu bar: File (New, Open, Save/Ctrl+S, Save As, Recent Files, Quit) and Edit (Undo, Redo, Copy, Cut, Paste) with native file dialogs via rfd
  - Pattern grid: custom Painter-based monospace text rendering with per-cell coloring, beat/bar highlighting, cursor tracking
  - Mouse interaction: click-to-position cursor in grid (row, channel, sub-column hit testing), scroll wheel navigation
  - Drag-to-select: click and drag across the pattern grid to create block selections (rows x channels)
  - Horizontal channel scrolling: grid auto-scrolls to keep cursor visible when channels exceed available width
  - Drag-and-drop: drop WAV/AIFF files onto the window to load samples (targets selected instrument slot when editor is open, otherwise first empty slot); drop .rtrk files to open projects
  - Real-time audio visualization: spectrum analyzer (FFT) and sample waveform viewer with voice position playheads
  - Sample slicing: equal-segment and transient-detection slicing with live preview in the visualization panel, auto-applied on parameter change
  - Instrument editor: sidebar list with synth parameter editor (patch selector, ADSR, filter, oscillator, FM), sample editor (load, trim, loop, base note, waveform preview), and MIDI program editor
  - Order list sidebar: clickable order entries with active highlight, append button, channel list with mute/solo toggles
  - Interactive transport: DragValue widgets for BPM/Speed/Octave/Edit Step, styled Play/Stop button, Follow mode checkbox, pattern info display, elapsed time (MM:SS), recording indicator
  - Undo/redo: dual-stack edit history (100 levels) tracking cell edits across note entry, hex entry, clear, and note-off
  - Clipboard: cut/copy/paste single cells and blocks (Ctrl+X/C/V)
  - Song settings dialog: editable title, BPM, speed, beat/bar highlights, swing; read-only channel/pattern/order info
  - Help dialog (F1): keyboard shortcut quick-reference overlay
  - Track config dialog: channel name, type, MIDI channel, instrument, volume, pan, per-channel effects with MIDI learn
  - MIDI ports dialog: output/input port selection, virtual port creation, clock mode
  - Pattern matrix: full-screen order list with pattern visualization
  - Theme support: Dark, Light, Monokai (F8 to cycle)
  - Full keyboard input: piano mapping (z-m/q-u), hex digit entry, modal Normal/Insert modes
  - Dependencies: eframe 0.31, egui 0.31, rfd 0.15

### Changed (Workspace Restructuring)

- Restructured from a single crate into a Cargo workspace with three crates:
  - `rtrack-core/`: headless library containing engine, audio, MIDI, samples, data model, and all non-UI logic
  - `rtrack-tui/`: TUI frontend binary (`rtrack`) + library wrapping `TrackerCore`
  - `rtrack-gui/`: GUI frontend (egui/eframe)
- Extracted `TrackerCore` into `rtrack-core/src/core.rs` as the main headless API for frontends
- Extracted shared types into `rtrack-core/src/types.rs`: `ChannelConfig`, `ChannelType`, `Instrument`, `ClockMode`, `PlaybackTiming`, `LearnableParam`, `MidiCcMapping`, utility functions
- Pushed recording logic (`record_note_at`, `record_note_off_at`, `handle_midi_cc`) from TUI's `handle_midi_input` into `TrackerCore`, so alternative frontends don't need to reimplement it
- Removed ~20 trivial delegation methods from `App` that just forwarded to `self.core.xxx()`; callers now use `app.core.xxx()` directly
- Methods that add TUI-specific logic (status_message, cursor updates) remain on `App`
- Makefile updated for workspace: `cargo build --workspace`, `cargo run -p rtrack-tui`, `cargo test --workspace`

### Fixed (Clippy Cleanup)

- Resolved all clippy warnings across the workspace (previously ~35 warnings):
  - Added `Default` implementations for `TrackerCore`, `MidiEngine`, `MidiInputEngine`, `SampleBank`, `PlaybackTiming`, `Instrument`, `FilterType`
  - Added `Sample::is_empty()` method (required by `len_without_is_empty` lint)
  - Replaced `map_or(false, ...)` with `is_some_and()` and `map_or(true, ...)` with `is_none_or()`
  - Replaced manual `div_ceil` with `usize::div_ceil()`
  - Replaced `% 2 == 0` with `.is_multiple_of(2)`
  - Removed unnecessary type casts, identity maps, and redundant `ref` patterns
  - Fixed operator precedence in fundsp DSP expressions
  - Merged identical if-else branches in pattern editor and pattern matrix
  - Used iterator patterns instead of index-based loop variables in tests
  - Replaced `.get(0)` with `.first()`

## [0.1.1]

### Added (Recent Files)

- Recent files list (`:recent` command): quickly re-open the last 3 songs
  - Persisted to `~/.config/rtrack/recent.json` (or `$XDG_CONFIG_HOME`)
  - Updated automatically on save and load
  - Popup shows filename + parent directory, navigate with Up/Down, Enter to open
  - Deduplicates by canonical path, limits to 3 entries
- 6 new tests: push ordering, deduplication, truncation, command/popup/navigation
- Test count: 358 (346 unit + 12 integration)

### Added (MIDI Learn & Aftertouch)

- MIDI learn: map any CC controller to a channel effects parameter
  - In Track Config, press `L` on a continuous effects parameter (cutoff, resonance, drive, etc.) to arm learn mode
  - Move a CC knob/fader on your MIDI controller to bind it
  - Press `U` to remove a mapping
  - CC labels (e.g. `CC1`) shown in yellow next to mapped parameters
  - One CC can control multiple parameters; same parameter on different channels can use different CCs
  - Mapped CCs modulate parameters in real-time; unmapped CCs pass through as MIDI thru
- MIDI aftertouch support: channel pressure (mono) and polyphonic key pressure
  - Both aftertouch types modulate filter cutoff on the cursor channel when filter is enabled
  - Exponential mapping: pressure 0 = 20 Hz, pressure 127 = 20 kHz
  - Ignored when filter is disabled (no-op, no crash)
- Sample selector in track config: Left/Right cycles through loaded sample bank slots with audition preview
  - Shows slot number, name, and duration (e.g. `02 kick (1.2s)`)
  - Falls back to file browser when no samples are loaded
  - Preview plays C at current octave on each selection change
- 16 new tests: MIDI learn bind/apply/replace/unlearn, CC range mapping, multi-param same CC, aftertouch modulation (channel/poly/disabled), sample selector cycling, sample bank loaded_slots
- Test count: 352 (340 unit + 12 integration)

### Added (MIDI Recording)

- Punch-in recording (`Ctrl+R` toggle): record incoming MIDI notes directly into the pattern during playback
  - When armed (playing + recording + Insert mode), NoteOn writes to the pattern at the engine's current playback position with velocity in the volume column
  - NoteOff during punch-in writes note-off at the current playback position
  - Instrument auto-fill from track default for Synth and Sample channel types (both punch-in and step recording)
  - Does not advance cursor (engine manages position); does not record NoteOff in step mode (key release timing is meaningless during step entry)
  - Record indicator in header bar: filled circle, bold red when armed, dim when off
  - 8 new tests: recording toggle, punch-in at engine position, no-record without flag, NoteOff punch-in, NoteOff not recorded in step mode, instrument auto-fill (punch-in + step), dirty flag
- Header transport indicators: play triangle, stop square, and record circle shown in the top bar
- Link indicator redesigned: shows `Link:N` in bold yellow when active (peers > 0), dim when enabled but alone; no longer uses reversed/red background

### Fixed

- Fixed sample track note preview using synth instead of sample engine during insert mode. Loading a sample via the file browser now auto-sets the channel's `default_instrument`, so `try_enter_note()` routes preview through the sample engine instead of falling through to the default synth.
- Fixed `sample_dir` config option having no effect: `load_sample_directory()` ran before `load_file()` in `setup_app()`, so loading a song file wiped the samples. Reordered so song loads first, then samples overlay on top.

### Changed (Testing & Tooling)

- Replaced wall-clock sleep loop in `test_playback_advances_position` with deterministic `engine.process_tick()` calls; integration suite dropped from ~670ms to ~70ms
- Makefile: added `fmt`, `clippy`, `lint`, `test-unit`, and `test-integration` targets

### Added (Configuration)

- User configuration file at `~/.config/rtrack/config.toml` (XDG-compliant):
  - `sf2` -- default SoundFont file path (overridden by `--sf2` CLI flag)
  - `sample_dir` -- default sample directory (overridden by `--sample-dir` CLI flag)
  - Missing or malformed config files silently fall back to defaults
  - 6 tests covering full/partial/empty/invalid config parsing and unknown field tolerance

### Changed (App State Organization)

- Extracted 3 inner structs from `App` to reduce cognitive load:
  - `PlaybackTiming` (6 fields): last_tick, tick_accumulator, clock_tick_accumulator, playback_elapsed, ext_clock_count, last_link_beat
  - `DialogState` (13 fields): settings, instrument list, sample/synth editor, MIDI port selector, help scroll, file browser
  - `EditHistory` (5 fields): undo/redo stacks, clipboard, block clipboard, block anchor
- Access patterns: `self.timing.*`, `self.dialogs.*`, `self.history.*`

### Changed (Architecture)

- Extracted deterministic `TrackerEngine` (`src/engine/mod.rs`) from duplicated playback logic
  - Single source of truth for tick-based playback: row advancement, order navigation, repeat counts, pattern break/position jump, speed/tempo changes, tempo automation, swing timing, and all continuous effects (arpeggio, portamento, vibrato, volume slide, note delay)
  - Engine emits typed `TrackerEvent`s (NoteOn, NoteOff, PitchBend, VolumeChange, MidiCC, ProgramChange, SpeedChanged, TempoChanged, RowAdvanced, GenerationAdvanced); consumers translate events to their domain
  - `App::engine` field replaces 7 individual playback state fields (`playback_row`, `playback_order`, `playback_generation`, `playback_tick`, `channel_states`, `playback_repeat_count`, `playback_speed`)
  - `app/playback.rs`: rewritten to drive `TrackerEngine` and dispatch events to MIDI/audio
  - `sample/export.rs`: `render_song()` reduced from ~400 lines to ~90 lines by replacing manual row/tick/effect loop with engine iteration; `ExportChannelState` struct deleted
  - `midi_file.rs`: `export_midi()` rewritten to run engine once, collect per-channel MIDI events, and write as format-1 tracks; `ExportMidiChannelState` struct deleted
  - Bug fixes and new effects now propagate automatically to live playback, WAV/FLAC export, and MIDI export
  - 17 engine unit tests covering tick advancement, note on/off, all effects, order repeats, generation wrap, muted channels, and swing timing

### Added (Offline Render)

- `--render` CLI flag with `-o`/`--output` for offline rendering to WAV or FLAC:
  - `rtrack --render song.rtrk -o out.wav` renders to WAV
  - `rtrack --render song.rtrk -o out.flac` renders to FLAC
  - Reuses the existing `sample::export` offline render pipeline (synth + samples + per-channel effects + send buses)
  - No real-time sleeping, no audio device required -- purely computational
  - Format detected from output file extension; unsupported extensions produce a clear error
- Extracted `App::export_instruments()` and `App::export_sample_rate()` helpers, deduplicating instrument-gathering code across WAV/FLAC/render export paths

### Changed (Playback)

- Space now starts playback from the current order position and cursor row instead of always restarting from order 0
- Ctrl+Space starts playback from the beginning (order 0, row 0) for when you want to hear the full song
- 3 new tests: play from edit order, play from start, Ctrl+Space keybinding
- Test count increased from 301 to 336 (324 unit + 12 integration)

### Added (Code Quality)

- `src/constants.rs`: centralized module for shared constants (MIDI protocol values, music theory, effect commands, tracker limits), eliminating magic numbers across the codebase
- `FileBrowserState` struct: extracted 6 file browser fields from App into a self-contained struct with `new()`, `refresh()`, `open()` methods
- `ChannelConfig` struct: consolidated 8 parallel channel Vecs (`muted_channels`, `channel_names`, `channel_types`, `channel_instruments`, `channel_volumes`, `channel_pans`, `channel_effects_params`, `midi_channel_map`) into `Vec<ChannelConfig>`

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
