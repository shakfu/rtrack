# rtrack

A terminal-based music tracker written in Rust. Compose music using classic tracker-style pattern editing, hear it immediately through the built-in synthesizer, and export to MIDI or WAV.

rtrack makes sound out of the box -- no external synth, DAW, or SoundFont required. Connect to external gear via MIDI, sync with Ableton Link, or load your own samples.

## Quick Start

```sh
cargo run                                # launch with built-in synth
cargo run -- song.rtrk                   # open a saved song (restores instruments + samples)
cargo run -- recording.mid               # import a MIDI file
cargo run -- --sample-dir samples/       # load a directory of samples
```

Press **Esc** to enter Insert mode, play notes with the keyboard (piano layout), and hit **Space** to play back. Press **F1** for the full help screen.

### Headless Playback

Play a song from the command line without launching the TUI:

```sh
cargo run -- --play examples/multi-pattern.rtrk          # play once and exit
cargo run -- --play --loops 3 song.rtrk                   # play 3 times
cargo run -- --play --loops 0 song.rtrk                   # loop forever (Ctrl+C to stop)
cargo run -- --play --sf2 gm.sf2 --sample-dir drums/ song.rtrk  # with audio options
```

## Audio Modes

rtrack supports three ways to produce sound, and they can be combined:

| Mode | How to activate | What it does |
|------|----------------|--------------|
| **Built-in synth** | Default (always on) | 9 waveform patches with ADSR envelopes and SVF/Moog filters. Select with `Exx` effect (0-8), or configure per-instrument (F7 > Tab). |
| **SoundFont** | `--sf2 path/to/file.sf2` | General MIDI playback via [rustysynth](https://github.com/sinshu/rustysynth). Replaces built-in synth for note playback. |
| **Samples** | `--sample 0:kick.wav` or `--sample-dir path/` | Load WAV/AIFF files into instrument slots. Pitch-shifted playback with loop points. |

All modes output through [cpal](https://crates.io/crates/cpal) with a stereo delay effects chain. MIDI output runs in parallel regardless of audio mode.

```sh
cargo run -- --sf2 gm.sf2                                # SoundFont mode
cargo run -- --sample 0:kick.wav --sample 1:snare.aiff   # individual samples
cargo run -- --sample-dir drums/                         # sample directory
cargo run -- --sf2 gm.sf2 --sample 0:kick.wav song.rtrk  # all together
```

## How Tracking Works

A tracker arranges music in **patterns** -- grids where each row is a point in time and each column is a channel. Patterns are sequenced in an **order list** to form a full song.

Each cell has four fields:

```text
C-4 01 80 000
 |   |  |  |
 |   |  |  +-- Effect command + parameter
 |   |  +----- Volume (00-FF, maps to velocity)
 |   +-------- Instrument number (00-FF)
 +------------ Note (C-4, F#5, etc.) or === (note off)
```

Empty fields display as dashes: `--- -- -- ---`

### Note Entry

Switch to **Insert** mode (Esc) and use the piano keyboard layout:

```text
Lower octave:  z s x d c v g b h n j m
               C C#D D#E F F#G G#A A#B

Upper octave:  q 2 w 3 e r 5 t 6 y 7 u
               C C#D D#E F F#G G#A A#B
```

Use `+`/`-` to shift octave. Ctrl+1..8 to select tracks, arrow keys to navigate.

## Features

### Pattern Editing

- Up to 8 channels, displayed in pages of 4 (Tab/Shift-Tab to switch pages)
- Configurable channel count and rows per pattern (default 4 channels, 64 rows)
- Note, instrument, volume, and effect columns per cell
- Normal mode (navigation) and Insert mode (data entry)
- Edit step (`(`/`)`) -- auto-advance cursor by N rows after each entry
- Row insert/delete, copy/cut/paste entire rows
- Undo/redo with 100-level history
- Mouse: click to place cursor, scroll to navigate

### Song Structure

- Multiple patterns with per-pattern row counts
- Order list sidebar (always visible) with insert/remove
- Position jump (`Bxx`) and pattern break (`Dxx`) effects

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

### Instruments & Samples

- 256 instrument slots (F7 to browse)
- Per-instrument sample assignment -- load WAV/AIFF files into slots
- Sample editor (Enter from instrument list): trim, loop points, base note, waveform preview
- Synth editor (Tab from instrument list): per-instrument waveform, ADSR, filter, and detune
- Pitch-shifted playback with up to 32 simultaneous voices

### Effects

| Cmd | Name | Description |
|-----|------|-------------|
| `0xy` | Arpeggio | Cycle note, note+x, note+y semitones each tick |
| `1xx` | Portamento up | Slide pitch up by xx per tick |
| `2xx` | Portamento down | Slide pitch down by xx per tick |
| `3xx` | Tone portamento | Glide toward target note at speed xx |
| `4xy` | Vibrato | Pitch vibrato (speed x, depth y) |
| `5xy` | Volume slide | Slide volume up by x, down by y per tick |
| `6xx` | Note delay | Delay note trigger by xx ticks |
| `Bxx` | Position jump | Jump to order position xx |
| `Cxx` | MIDI CC | Send CC (controller from instrument col, value xx) |
| `Dxx` | Pattern break | Break to row xx of next pattern |
| `Exx` | Program change | Select synth patch or send MIDI program change |
| `Fxx` | Set speed/tempo | xx < 20: ticks per row; xx >= 20: set BPM |

Effects use a sub-tick engine: each row is divided into `speed` ticks (default 6). Tick 0 triggers notes; ticks 1+ process continuous effects like portamento and vibrato.

### MIDI

- Virtual output port `RTRACK_MIDI` (macOS/Linux) -- visible to any DAW
- Virtual input port `RTRACK_MIDI_IN` -- play notes from external controllers
- MIDI port selection (F2) for switching to hardware ports
- MIDI clock output (Ctrl+M) at 24 ppqn with start/stop messages
- Per-channel MIDI channel mapping

### Sync

- [Ableton Link](https://www.ableton.com/en/link/) (F3): bidirectional BPM and transport sync with Link-enabled apps

### Import / Export

- Save/load songs as `.rtrk` (JSON) -- includes instrument definitions and sample file references (see [File Format](#file-format))
- Import from standard MIDI files (`.mid`)
- Export to MIDI (Ctrl+E)
- Export to WAV (Ctrl+W) -- offline render with synth, samples, and effects
- Color themes: dark (default), light, monokai (F8 to cycle)

## File Format

`.rtrk` files are JSON. The format stores the full song (patterns, order list, BPM, speed) along with optional instrument definitions and sample references:

```json
{
  "title": "My Song",
  "bpm": 140, "speed": 6,
  "channels": 4, "rows_per_pattern": 64,
  "patterns": [ ... ],
  "order": [0, 1, 2],
  "instruments": [
    { "slot": 0, "name": "Kick", "sample_index": 0 },
    { "slot": 5, "name": "Lead", "midi_program": 80 },
    { "slot": 10, "name": "Pad", "synth_params": {
        "waveform": 0, "attack": 0.05, "decay": 0.3, "sustain": 0.6,
        "release": 0.4, "filter_cutoff": 6.0, "filter_resonance": 0.3,
        "filter_env": 2.0, "detune": 8.0 } }
  ],
  "sample_refs": [
    { "slot": 0, "name": "kick", "path": "samples/0-kick.wav",
      "base_note": 36, "loop_enabled": false }
  ]
}
```

- **Instruments**: only non-empty slots are saved (name, MIDI program, sample assignment, synth params)
- **Synth params**: optional per-instrument synthesis parameters (waveform, ADSR envelope, filter cutoff/resonance/envelope, detune). When present, overrides the channel's default patch.
- **Sample refs**: file paths stored relative to the `.rtrk` file, plus all metadata (base note, trim, loop points). Audio data is not embedded -- samples are reloaded from disk on open. Missing files produce a warning but do not block loading.
- **Backwards compatible**: old `.rtrk` files without `instruments`, `sample_refs`, or `synth_params` fields load fine.

## Keybindings

### Global (all modes)

| Key | Action |
|-----|--------|
| Space | Play / stop |
| Esc | Toggle Normal / Insert mode |
| Tab / Shift-Tab | Next / previous track page (groups of 4) |
| Ctrl+1..8 | Select track 1-8 directly |
| Arrows | Move cursor (auto-switches page at boundaries) |
| PgUp / PgDn | Jump 16 rows |
| Home / End | First / last row |
| `+` / `-` | Octave up / down |
| `[` / `]` | BPM down / up |
| `(` / `)` | Edit step down / up |
| F1 | Help |
| F2 | MIDI port selector |
| F3 | Toggle Ableton Link |
| F6 | Song settings |
| F7 | Instrument list |
| F8 | Cycle color theme |
| F9-F12 | Mute channels on current page |
| Ctrl+F9-F12 | Solo channels on current page |
| Ctrl+S | Save |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+C / X / V | Copy / cut / paste row |
| Ctrl+Left / Right | Previous / next order position |
| Ctrl+E | Export MIDI |
| Ctrl+W | Export WAV |
| Ctrl+M | Toggle MIDI clock |

### Normal Mode

| Key | Action |
|-----|--------|
| q | Quit |
| Ctrl+N | New pattern |
| Ctrl+D | Clone current pattern |
| F4 / F5 | Insert / remove order entry |
| Insert | Insert row at cursor |
| Backspace | Delete row at cursor |

### Insert Mode

| Key | Action |
|-----|--------|
| Piano keys | Enter note |
| `0`-`9`, `a`-`f` | Hex digit (instrument / volume / effect columns) |
| Delete / Backspace | Clear cell |
| `=` | Note off (`===`) |

## Examples

The `examples/` directory contains `.rtrk` files demonstrating various features:

| File | What it shows |
|------|---------------|
| `c-major-scale.rtrk` | Basic note entry -- ascending C major scale on one channel |
| `four-on-the-floor.rtrk` | Multi-channel beat -- kick, snare, hi-hat, bass with volume variation |
| `chord-progression.rtrk` | Polyphony -- I-V-vi-IV chords voiced across 3 channels |
| `arpeggio-demo.rtrk` | Effects -- `0xy` arpeggio, `4xy` vibrato, `Exx` program change |
| `portamento-slide.rtrk` | Effects -- `1xx` porta up, `3xx` tone portamento, `5xy` volume slide |
| `multi-pattern.rtrk` | Song structure -- 3 patterns, order list, `Bxx` position jump |
| `all-patches.rtrk` | All 9 built-in synth patches cycled with `Exx` program change |
| `fundsp-pad.rtrk` | FundspPad patch (program 8) -- pad chord progression using fundsp synthesis |
| `speed-tempo.rtrk` | `Fxx` effect -- speed changes (< 0x20) and tempo changes (>= 0x20) |

Load any example:

```sh
cargo run -- examples/chord-progression.rtrk
```

## Requirements

- Rust 1.70+
- CMake 3.14+ (builds Ableton Link C++ dependency)
- macOS/Linux: virtual MIDI ports created automatically
- Windows: requires a third-party virtual MIDI driver (e.g., [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html))

## Build

```sh
make build    # compile
make run      # compile and run
make test     # run tests (181 tests)
```

## Architecture

```text
src/
  main.rs               Entry point, event loop, clap CLI
  app/
    mod.rs              App state, undo/redo, file I/O, song management
    input.rs            Keyboard/mouse input handling, mode dispatch
    playback.rs         Playback engine, tick processing, effects
  midi_file.rs          MIDI file (.mid) export and import
  audio/
    mod.rs              Unified audio engine (SF2 + synth + samples + effects, cpal)
    synth.rs            Built-in subtractive synth (9 patches, PolyBLEP + SVF/Moog + ADSR)
    effects.rs          Stereo delay effect (fundsp)
  sample/
    mod.rs              Sample loading (WAV via hound, AIFF parser, dasp conversion)
    playback.rs         Sample voice manager, pitch-shifted rendering
    export.rs           Offline song render to WAV
  tracker/
    pattern.rs          Pattern grid, Cell, Note (serde)
    song.rs             Song, SongFile, order list, instrument/sample refs
  link/
    mod.rs              Ableton Link (rusty_link)
  midi/
    mod.rs              MIDI output + input (midir)
  ui/
    mod.rs              Header, status bar, popups
    pattern_editor.rs   Pattern grid renderer (page-aware)
    sample_editor.rs    Sample editor (waveform, trim, loop)
    theme.rs            Color themes (dark, light, monokai)
```

## License

See [LICENSE](LICENSE).
