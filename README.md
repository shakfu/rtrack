# rtrack

A music tracker written in Rust with both TUI and GUI frontends. Compose music using classic tracker-style pattern editing, hear it immediately through the built-in synthesizer, and export to MIDI or WAV.

rtrack makes sound out of the box -- no external synth, DAW, or SoundFont required. Connect to external gear via MIDI, sync with Ableton Link, or load your own samples.

### Highlights

- **Built-in synthesizer** -- 30 patches (saw, square, FM bell, acid, chiptune, etc.) with ADSR, SVF filter, sub-oscillator, FM synthesis, and per-channel effects (distortion, filter, chorus, delay, reverb)

- **Sample engine** -- WAV/AIFF loading, pitch-shifted playback, loop points, transient-based slicing, up to 32 simultaneous voices with de-clicked voice stealing

- **MIDI I/O** -- virtual ports, hardware routing, step and punch-in recording, aftertouch-to-filter, MIDI learn for CC mapping, clock output/input

- **Ableton Link** -- bidirectional tempo and transport sync with any Link-enabled application

- **Pattern editing** -- modal input (Normal/Insert), piano keyboard layout, optional scale-constrained note entry and transpose (26 scales), block selection, interpolation, undo/redo, 14 effect commands

- **Song structure** -- multiple patterns with per-pattern row counts, order list with repeats, position jump and pattern break effects

- **Export** -- offline render to WAV/FLAC (no audio device needed), standard MIDI file import/export

- **Two frontends** -- terminal UI ([ratatui](https://ratatui.rs)) and native GUI ([egui](https://docs.rs/egui)) sharing the same headless core

- **SoundFont support** -- optional GM playback via .sf2 files

## Install

```sh
cargo install rtrack-tui                 # installs the `rtrack` binary
cargo install rtrack-gui                 # installs the `rtrack-gui` binary (optional)
```

Requires Rust 1.89+ (1.95+ for `rtrack-gui`) and CMake 3.14+ (for the Ableton Link C++ dependency).

## Frontends

rtrack is split into a headless core library (`rtrack-core`) and two independent frontends that wrap it. Both frontends share the same engine, audio, MIDI, and file format -- songs created in one open in the other.

### TUI (Terminal)

```sh
rtrack                                   # launch with built-in synth
rtrack song.rtrk                         # open a saved song
rtrack recording.mid                     # import a MIDI file
rtrack --sample-dir samples/             # load a directory of samples
```

Modal keyboard-driven interface built on ratatui and crossterm. Piano keyboard layout for note entry, vi-style command mode, pattern matrix, instrument/sample/synth editors, and color themes.

Once running, open a song with `Ctrl+O` (or `:open`), reopen a recent one with `:recent`, and press `F1` for the full keybinding list. See [`rtrack-tui/README.md`](rtrack-tui/README.md) for keybindings and TUI-specific details.

### GUI (Desktop)

```sh
rtrack-gui                               # launch GUI frontend
```

Native desktop application built on egui/eframe. Clickable pattern grid with drag-to-select and block operations, native file dialogs (open/save/export), interactive transport bar (drag to adjust BPM/speed/octave/step), order list and channel mute/solo sidebar, real-time audio visualization (FFT spectrum analyzer with level meters, sample waveform viewer with playhead tracking), full instrument editor (synth patch selector with ADSR/filter/oscillator/FM params, sample loader with waveform preview and trim/loop editing, MIDI program), interactive sample slicing (equal or transient detection with live preview, dividing either the whole sample or one slice), drag-and-drop sample loading, per-channel effects editing with MIDI learn, pattern matrix with channel data indicators, MIDI port selection dialog with clock mode switching, color themes (Dark/Light/Monokai), undo/redo (100 levels), and keyboard shortcuts mirroring the TUI. MIDI file *import* is currently TUI-only; the GUI exports MIDI but cannot open a `.mid`. See [`rtrack-gui/README.md`](rtrack-gui/README.md) for GUI-specific details.

### CLI (Headless)

Both offline render and headless playback work without launching either frontend:

```sh
rtrack --render song.rtrk -o out.wav     # render to WAV (no audio device needed)
rtrack --render song.rtrk -o out.flac    # render to FLAC
rtrack --play song.rtrk                  # play once and exit
rtrack --play --loops 0 song.rtrk        # loop forever (Ctrl+C to stop)
```

## How Tracking Works

A tracker arranges music in **patterns** -- grids where each row is a point in time and each column is a channel. Patterns are sequenced in an **order list** to form a full song.

Each cell has four fields:

```text
C-4 01 80 PVB44
 |   |  |  |
 |   |  |  +-- Effect: three-character command + hex parameter
 |   |  +----- Volume (00-FF, maps to velocity)
 |   +-------- Instrument number (00-FF)
 +------------ Note (C-4, F#5, etc.) or === (note off)
```

Empty fields display as dashes: `--- -- -- ---`

## Audio Modes

rtrack supports three ways to produce sound, and they can be combined:

| Mode | How to activate | What it does |
|------|----------------|--------------|
| **Built-in synth** | Default (always on) | 30 waveform patches with ADSR envelopes, SVF filters, sub-oscillator, and FM synthesis. |
| **SoundFont** | `--sf2 path/to/file.sf2` | General MIDI playback via [rustysynth](https://github.com/sinshu/rustysynth). Plays MIDI instruments in place of the built-in presets, live and in `--render`. Synth and sample instruments are unaffected. |
| **Samples** | `--sample 0:kick.wav` or `--sample-dir path/` | Load WAV/AIFF files into instrument slots. Pitch-shifted playback with loop points. |

All modes output through [cpal](https://crates.io/crates/cpal) with per-channel effects and a master stereo delay. MIDI output runs in parallel regardless of audio mode.

```sh
rtrack --sf2 gm.sf2                                # SoundFont mode
rtrack --sample 0:kick.wav --sample 1:snare.aiff   # individual samples
rtrack --sample-dir drums/                          # sample directory
rtrack --sf2 gm.sf2 --sample 0:kick.wav song.rtrk  # all together
```

## Features

### Built-in Synth Patches

30 patches selectable via `PRG` program change or per-instrument synth configuration:

| # | Name | Oscillator | Character |
|---|------|-----------|-----------|
| 0 | Saw | PolyBLEP saw | Classic detuned saw |
| 1 | Square | PolyBLEP square | Hollow, filtered |
| 2 | Sine | Sine | Clean, pure tone |
| 3 | Triangle | Triangle | Soft, detuned pair |
| 4 | Pulse | PolyBLEP pulse | 25% duty, heavy filter env |
| 5 | FM Bell | 2-op FM (3.5:1) | Metallic bell, long decay |
| 6 | Organ | Additive (3 harmonics) | Drawbar organ |
| 7 | Noise | LCG noise | Filtered noise hit |
| 8 | Fundsp Pad | fundsp saws + moog | Warm pad |
| 9 | Bass | Saw + sub-osc | Deep bass, resonant LP |
| 10 | Pluck | Saw | Fast decay, bright attack |
| 11 | Pad | Saw + sub-osc | Slow attack, wide detune |
| 12 | Lead | Saw + sub-osc | Bright, sustained |
| 13 | Keys | FM (2:1) | Electric piano |
| 14 | Brass | Saw | Slow attack, filter sweep |
| 15 | Strings | Saw + sub-osc | Wide detune, slow attack |
| 16 | Perc | Noise + BP filter | Percussive hit |
| 17 | Sub | Saw + sub-osc (0.8) | Deep sub-bass |
| 18 | Acid | Saw | TB-303 style, high resonance |
| 19 | Chip | Pulse (12.5%) | 8-bit chiptune |
| 20 | Stab | Saw | Short sharp synth stab |
| 21 | Mallet | FM (4:1) | Vibraphone/marimba |
| 22 | Flute | Triangle + HP | Soft, breathy |
| 23 | Reese | Saw + sub-osc | Heavy detuned bass |
| 24 | Wire | Square + BP | Metallic, resonant |
| 25 | Chime | FM (5:1) | Bright bell, long tail |
| 26 | Growl | Saw + FM (1.5:1) | Aggressive, gritty |
| 27 | Whistle | Sine | Clean whistle tone |
| 28 | Siren | Triangle | Bright filter sweep |
| 29 | Dist | Saw | Driven filter, high resonance |

### Per-Channel Effects

Each Synth/Sample channel can have its own effects chain, applied to everything the track plays. For drive or a filter that stays with one sound on any track, use the instrument's own (Instruments & Samples). SoundFont output is mixed before this chain and does not pass through it. All continuous parameters support MIDI learn:

| Effect | Parameters |
|--------|-----------|
| Distortion | Enable, drive amount |
| Filter | Enable, cutoff, resonance |
| Chorus | Enable, rate, depth, mix |
| Delay | Enable, time (10-2000ms), feedback, mix |
| Reverb | Enable, room size, damping, mix |

### Pattern Effects

Type the key in an empty effect field to enter the command, then two hex digits for its parameter. Or press `=` on the field and type the command by name, e.g. `PVB44` or `C0740`; empty text clears it.

Each cell has two effect columns, and both play. The second is shown where the pattern uses it, or always after `:fx2` (TUI) or View > Second effect column (GUI). Slides from both columns move one pitch offset, arpeggio and vibrato add to it, and volume slides add. Where both set the same flow command, the right-hand one wins.

| Command | Key | Name | Description |
|---------|-----|------|-------------|
| `ARPxy` | `0` | Arpeggio | Cycle note, note+x, note+y semitones each tick |
| `PUPxx` | `1` | Portamento up | Slide pitch up by xx per tick |
| `PDNxx` | `2` | Portamento down | Slide pitch down by xx per tick |
| `PSLxx` | `3` | Tone portamento | Glide toward target note at speed xx |
| `PVBxy` | `4` | Vibrato | Pitch vibrato (speed x, depth y) |
| `VSLxy` | `5` | Volume slide | Slide volume up by x, down by y per tick |
| `DELxx` | `6` | Note delay | Delay note trigger by xx ticks |
| `CHAxx` | `7` | Probability | Play the note with probability xx/255 (00 never, FF always) |
| `RNDxx` | `8` | Randomize | Re-run this channel's last continuous effect, parameter varied by +/-xx |
| `OFSxx` | `9` | Sample offset | Start the row's sample note xx/256 into its played span (00 start, 80 halfway) |
| `JMPxx` | `B` | Position jump | Jump to order position xx |
| `Cnnxx` | `C` | MIDI CC | Send value xx to controller nn. The key takes nn from the instrument field |
| `BRKxx` | `D` | Pattern break | Break to row xx of next pattern |
| `PRGxx` | `E` | Program change | Select synth patch or send MIDI program change |
| `TICxx` | `F` | Ticks per row | Set speed; the key gives `TIC` below 20 |
| `TPOxx` | `F` | Tempo | Set BPM; the key gives `TPO` from 20 |
| `TBLxx` | -- | Table | Play table xx for the row's note; with no note, restart it on the sounding note |

Songs saved before format version 2 stored the key digit; they convert on load.

Effects use a sub-tick engine: each row is divided into `speed` ticks (default 6). Tick 0 triggers notes; ticks 1+ process continuous effects like portamento and vibrato.

`OFS` needs a note on the same row and affects samples only. The offset is a fraction of the slice's trimmed span, not ProTracker's fixed 256-frame steps, so one value marks the same point in slices of any length. A looping sample started past its loop end wraps into the loop.

`CHA` and `RND` draw from a generator seeded the same way on every playback, so a song sounds the same each time it runs and an offline render matches the editor. `RND` varies only `ARP`, `PUP`, `PDN`, `PVB` and `VSL`; it ignores structural ones, where a random parameter would scramble playback rather than colour it.

### Phrases

A phrase is a single-track run of 1-256 rows that any track of any pattern can play instead of its own cells. Editing a phrase changes every place it plays.

- **Aligned** phrases restart with each pattern and loop inside it, so a 3-row phrase in a 16-row pattern repeats 5 times and starts once more.
- **Running** phrases keep counting rows across patterns, repeats and jumps. A 3-row and a 4-row phrase then drift against each other and line up every 12 rows, whatever the pattern lengths.

A link can transpose its phrase at playback: by scale degrees when the song has a scale, by semitones otherwise, as Shift+Up/Down moves notes. One riff can then play under several chords. The grid shows the phrase as stored.

The grid shows a running phrase as it plays the first time playback reaches the pattern. A linked track's header shows `[Ann]` (aligned) or `[Rnn]` (running). In the GUI, a faint box marks each repetition of a linked phrase: blue for aligned, orange for running (View > Phrase borders).

| TUI | GUI | |
|-|-|-|
| `:phrase` | View > Phrases > New phrase from track | Turn the track's cells into a new phrase and link to it |
| `:phrase NN` | Phrases > Link track | Link the track to phrase `NN` |
| `:run` | Phrases > Runs across patterns | Switch between aligned and running |
| `:tr N` | Phrases > Transpose | Transpose the linked phrase at playback by N steps |
| `:unlink` | Phrases > Unlink | Make the phrase rows the track's own cells again |
| `:phrases` | Phrases window | Rename and resize phrases |
| F5, `:pedit [NN]` | F5, View > Phrase Editor, Phrases > Edit | Show the track's phrase, or phrase `NN`, in the grid |

Inserting or deleting a row changes the pattern's own cells, not a linked phrase.

#### Phrase editor

F5 shows a phrase in the grid in place of the pattern: the phrase the cursor's track plays, at the row under the cursor, or phrase 00 for an unlinked track. Every edit works there as in a pattern, and notes preview through the cursor's track. F5 again returns to the pattern.

- Ctrl+Left / Ctrl+Right step through the phrases.
- Ctrl+N adds an empty phrase; Ctrl+D duplicates the one shown.
- Insert and Backspace insert and delete phrase rows; the length stays the same.
- The TUI header shows the phrase number, length and use count. In the GUI, a bar above the grid shows these and edits the name and length.

### Tables

A table is a short sequence of 1-16 rows that runs under each note, one row every 1-16 ticks. A row can shift the pitch in semitones, set the volume, and carry one of `ARP`, `PUP`, `PDN`, `PVB`, `VSL`, `Cnn` or `PRG`. It restarts with each note, loops from a chosen row or holds its last, and stops at note-off. Instrument N plays table N; `TBLxx` plays table xx instead.

`ARP37` is a table of three rows: 0, +3, +7. A table can also hold longer or uneven arpeggios, a volume decay, or a filter CC stepping with the note (CCs reach MIDI out and a SoundFont, not built-in instruments). Pitch moves by pitch bend, as `ARP` does, so an external MIDI synth needs a bend range wide enough for the largest step. Tables play during playback and render, not while previewing notes.

TUI: `:table` opens the table of the instrument under the cursor, `:table NN` table NN. Up/Down and Left/Right move; `+`/`-` adjust; hex digits type a volume or effect; `=` names an effect; Ctrl+Left/Right change the row count; `[`/`]` the ticks per row; `l` loops from the current row. `:tname` renames, `:tdel` deletes. GUI: View > Tables.

### Instruments & Samples

- 256 instrument slots with per-instrument synth parameters or sample assignment

- Sample loading from WAV/AIFF files with pitch-shifted playback (cubic hermite interpolation)

- Sample slicing: equal segments or transient detection (log-energy onset detection against a local average, configurable sensitivity). Either divides the whole sample -- so changing the count re-derives from it -- or subdivides a single slice, selected with the Divide control. Slices land in consecutive slots; slicing over instruments it did not create asks first, and is undoable

- Up to 32 simultaneous voices with ADSR envelopes. A voice that is stolen, or that reaches the end of a one-shot, fades over a few milliseconds rather than stopping dead -- a slice ends at an arbitrary frame, so cutting it leaves an audible step

- Configurable pitch bend range per instrument (default +/-2 semitones)

- Per-instrument drive and filter, applied to each voice before its envelope, so an instrument sounds the same on any track. Sample instruments have drive and a low-, high- or band-pass filter (TUI sample editor, GUI instrument editor); synth instruments add drive in front of their own filter. Both are off by default and work alongside the track's effects. Pattern commands cannot change them yet.

### Sample Directory

Load an entire directory of samples with `--sample-dir`. Files must be named `<slot>-<name>.wav` (or `.aiff`):

```text
drums/
  0-kick.wav
  1-snare.wav
  2-hihat.wav
  samples.json   (optional metadata)
```

The optional `samples.json` can set BPM, base notes, and loop points:

```json
{
  "bpm": 140,
  "samples": {
    "0": { "base_note": 36 },
    "1": { "base_note": 38, "loop_enabled": true, "loop_start": 1000, "loop_end": 5000 }
  }
}
```

### MIDI

- Virtual output port `RTRACK_MIDI` (macOS/Linux) -- visible to any DAW

- Virtual input port `RTRACK_MIDI_IN` -- play notes from external controllers

- Step recording: MIDI input writes notes to the pattern with velocity and instrument auto-fill

- Punch-in recording: arm recording during playback to capture MIDI in real time

- Aftertouch: channel pressure and polyphonic key pressure modulate filter cutoff

- MIDI learn: map any CC to channel effects parameters

- MIDI clock output at 24 ppqn with start/stop messages

- External MIDI clock input for slaving to incoming clock

- Per-channel MIDI channel mapping

### Sync

- [Ableton Link](https://www.ableton.com/en/link/): bidirectional BPM and transport sync with Link-enabled apps

- Link beat-timeline mode: playback timing driven from Link's beat position, eliminating drift

### Timing & Groove

- **Groove**: relative row lengths that repeat by pattern row, set in song settings: `67 33` swings, `55 45 52 48` pushes and pulls over four rows, `off` is straight, and one number `s` means `s 100-s`. A groove cycle lasts as long as the same rows played straight, so the tempo holds. Live playback, render and MIDI export all follow it

- **Tempo automation**: BPM changes via `TPO` effect or `tempo_map` in the song file

- **Configurable row highlighting**: beat and bar intervals supporting time signatures like 3/4, 6/8, 5/4

- **Auto-save**: every 60 seconds when there are unsaved changes, to a hidden file beside the song (`.<name>.rtrk.autosave`), removed on a successful save

### Import / Export

- Save/load songs as `.rtrk` (JSON) -- includes instrument definitions and sample file references

- Atomic save -- writes to temp file then renames, preventing corruption on crash

- Import from standard MIDI files (`.mid`) with CC and program change preservation

- Export to MIDI, WAV (offline render with synth, samples, and effects), and FLAC

- CLI offline render (`--render song.rtrk -o out.wav`) -- no audio device needed

## File Format

`.rtrk` files are JSON. The format stores the full song (patterns, order list, BPM, speed) along with optional instrument definitions and sample references:

```json
{
  "title": "My Song",
  "bpm": 140, "speed": 6, "groove": [67, 33],
  "channels": 4, "rows_per_pattern": 64,
  "highlight_beat": 4, "highlight_bar": 16,
  "patterns": [ ... ],
  "order": [0, 1, 2],
  "instruments": [
    { "slot": 0, "name": "Kick", "sample_index": 0 },
    { "slot": 5, "name": "Lead", "midi_program": 80 },
    { "slot": 10, "name": "Pad", "synth_params": {
        "waveform": 11, "attack": 0.3, "decay": 0.5, "sustain": 0.7,
        "release": 0.8, "filter_cutoff": 3.0, "filter_resonance": 0.2,
        "filter_env": 1.0, "detune": 15.0, "filter_type": "LowPass",
        "sub_osc": 0.2, "fm_ratio": 0.0, "fm_index": 0.0,
        "pulse_width": 0.25 } }
  ],
  "sample_refs": [
    { "slot": 0, "name": "kick", "path": "samples/0-kick.wav",
      "base_note": 36, "loop_enabled": false }
  ]
}
```

- **Instruments**: only non-empty slots are saved (name, MIDI program, sample assignment, synth params)

- **Synth params**: optional per-instrument synthesis parameters. When present, overrides the channel's default patch. New fields use serde defaults for backwards compatibility.

- **Sample refs**: file paths stored relative to the `.rtrk` file, plus metadata (base note, trim, loop points). Audio data is not embedded -- samples are reloaded from disk on open. Missing files produce a warning but do not block loading.

- **Backwards compatible**: old `.rtrk` files without newer fields load fine via serde defaults.

## Examples

The `examples/` directory contains `.rtrk` files demonstrating various features:

| File | What it shows |
|------|---------------|
| `c-major-scale.rtrk` | Basic note entry -- ascending C major scale on one channel |
| `four-on-the-floor.rtrk` | Multi-channel beat -- kick, snare, hi-hat, bass with volume variation |
| `chord-progression.rtrk` | Polyphony -- I-V-vi-IV chords voiced across 3 channels |
| `arpeggio-demo.rtrk` | Effects -- `ARP` arpeggio, `PVB` vibrato, `PRG` program change |
| `portamento-slide.rtrk` | Effects -- `PUP` porta up, `PSL` tone portamento, `VSL` volume slide |
| `multi-pattern.rtrk` | Song structure -- 3 patterns, order list, `JMP` position jump |
| `all-patches.rtrk` | Built-in synth patches cycled with `PRG` program change |
| `fundsp-pad.rtrk` | FundspPad patch (program 8) -- pad chord progression using fundsp synthesis |
| `speed-tempo.rtrk` | `TIC` speed changes and `TPO` tempo changes |
| `sliced-amen.rtrk` | Sample slicing -- 8 equal slices of amen.wav played sequentially (170 BPM) |
| `drumloops.rtrk` | Looping drum slices -- 8 amen.wav slices with loop points enabled (130 BPM) |
| `sample-offset.rtrk` | `OFS` sample offset -- the whole break retriggered at 8 offsets, then reordered, then offsets within slices, with transposition and a no-note `OFS` row (175 BPM) |
| `polymeter.rtrk` | Phrases -- tracks linked to 4-, 3- and 5-row phrases; the 3 and 5 run across patterns and drift against the 4. Every track is linked, so every note shown comes from a phrase |
| `tables.rtrk` | Tables -- a looping major arpeggio per note, `TBL` switching rows to a minor one, and a volume decay |

```sh
rtrack examples/chord-progression.rtrk
```

## Requirements

- Rust 1.89+ (1.95+ for `rtrack-gui`)

- CMake 3.14+ (builds Ableton Link C++ dependency)

- macOS/Linux: virtual MIDI ports created automatically

- Windows: requires a third-party virtual MIDI driver (e.g., [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html))

## Configuration

rtrack reads optional settings from `~/.config/rtrack/config.toml` (or `$XDG_CONFIG_HOME/rtrack/config.toml`):

```toml
sf2 = "/path/to/soundfont.sf2"
sample_dir = "/path/to/samples"
```

CLI flags (`--sf2`, `--sample-dir`) override config values. Missing or malformed config files are silently ignored.

The MIDI output port you select is remembered in `midi_out.json` in the same directory and reopened at startup. Until you select one, instruments routed to external MIDI play the built-in sound.

## Build

```sh
make build            # compile all crates
make test             # run all tests (874 tests across workspace)
make fmt              # format code
make clippy           # lint with clippy
make lint             # fmt + clippy
```

## Architecture

rtrack is a Cargo workspace with three crates sharing a headless core:

```text
rtrack-core/                Headless library (engine, audio, MIDI, data model)
  src/
    core.rs                 TrackerCore: main API for frontends
    types.rs                Shared types (ChannelConfig, Instrument, ClockMode, etc.)
    constants.rs            Shared constants (MIDI protocol, music theory, effect commands, limits)
    config.rs               User config (~/.config/rtrack/config.toml)
    error.rs                Error type and Result alias
    fs.rs                   Atomic file writes (temp file + rename)
    keymap.rs               Piano keyboard layout, shared by both frontends
    midi_file.rs            Standard MIDI file import and export
    engine/mod.rs           Deterministic TrackerEngine (tick-based playback, effects, events)
    tracker/                Pattern, Song, Cell, Note (serde)
    audio/                  Unified audio engine (SF2 + synth + samples + effects, cpal)
    sample/                 Sample loading (WAV/AIFF), slicing, playback, offline export
    midi/                   MIDI output + input (midir)
    link/                   Ableton Link (rusty_link)

rtrack-tui/                 TUI frontend (binary: rtrack)
  src/
    main.rs                 Entry point, crossterm event loop, clap CLI
    app/                    App state, keyboard/mouse input, playback, undo/redo
    tui/                    ratatui rendering (pattern editor, editors, themes)

rtrack-gui/                 GUI frontend (binary: rtrack-gui)
  src/
    main.rs                 eframe entry point
    app.rs                  RtrackApp, eframe::App impl, drag-and-drop, playback tick
    state.rs                Mode (Normal/Insert), SubColumn, Theme, GridColors
    grid.rs                 Pattern grid (custom Painter rendering, click/drag/scroll)
    input.rs                Keyboard handling, piano mapping, hex entry, actions
    transport.rs            Transport bar (BPM/speed/octave/step, play/rec, Link/MIDI status)
    menu.rs                 Menu bar (File/Edit/View), save, export, recent files
    sidebar.rs              Order list + channel mute/solo
    instrument_editor.rs    Instrument list, synth params, sample editor, slicing
    pattern_matrix.rs       Full-screen pattern matrix with channel data indicators
    visualization.rs        FFT spectrum analyzer, level meters, sample waveform viewer
    dialogs.rs              Song settings, track config, MIDI ports, help
    history.rs              EditHistory (dual-stack undo/redo, max 100) over cell edits and sample-bank snapshots
```

## License

See [LICENSE](LICENSE).
