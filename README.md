# rtrack

A terminal-based music tracker written in Rust. Compose music using classic tracker-style pattern editing, hear it immediately through the built-in synthesizer, and export to MIDI or WAV.

rtrack makes sound out of the box -- no external synth, DAW, or SoundFont required. Connect to external gear via MIDI, sync with Ableton Link, or load your own samples.

## Quick Start

```sh
cargo run                    # launch with built-in synth
cargo run -- song.rtrk       # open a saved song
cargo run -- recording.mid   # import a MIDI file
```

Press **Esc** to enter Insert mode, play notes with the keyboard (piano layout), and hit **Space** to play back. Press **F1** for the full help screen.

## Audio Modes

rtrack supports three ways to produce sound, and they can be combined:

| Mode | How to activate | What it does |
|------|----------------|--------------|
| **Built-in synth** | Default (always on) | 8 waveform patches via [fundsp](https://crates.io/crates/fundsp). Select with `Exx` effect (0-7). |
| **SoundFont** | `--sf2 path/to/file.sf2` | General MIDI playback via [rustysynth](https://github.com/sinshu/rustysynth). Replaces built-in synth for note playback. |
| **Samples** | `--sample 0:kick.wav` | Load WAV/AIFF files into instrument slots. Pitch-shifted playback with loop points. |

All modes output through [cpal](https://crates.io/crates/cpal) with a stereo reverb effects chain. MIDI output runs in parallel regardless of audio mode.

```sh
cargo run -- --sf2 gm.sf2                                # SoundFont mode
cargo run -- --sample 0:kick.wav --sample 1:snare.aiff   # sample mode
cargo run -- --sf2 gm.sf2 --sample 0:kick.wav song.rtrk  # all together
```

## How Tracking Works

A tracker arranges music in **patterns** -- grids where each row is a point in time and each column is a channel. Patterns are sequenced in an **order list** to form a full song.

Each cell has four fields:

```
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

```
Lower octave:  z s x d c v g b h n j m
               C C#D D#E F F#G G#A A#B

Upper octave:  q 2 w 3 e r 5 t 6 y 7 u
               C C#D D#E F F#G G#A A#B
```

Use `+`/`-` to shift octave. Tab between channels, arrow keys to navigate.

## Features

### Pattern Editing
- Configurable channels (default 4) and rows per pattern (default 64)
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

### Instruments & Samples
- 256 instrument slots (F7 to browse)
- Per-instrument sample assignment -- load WAV/AIFF files into slots
- Sample editor (Enter from instrument list): trim, loop points, base note, waveform preview
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
- Save/load songs as `.rtrk` (JSON)
- Import from standard MIDI files (`.mid`)
- Export to MIDI (Ctrl+E)
- Export to WAV (Ctrl+W) -- offline render with synth, samples, and effects
- Color themes: dark (default), light, monokai (F8 to cycle)

## Keybindings

### Global (all modes)

| Key | Action |
|-----|--------|
| Space | Play / stop |
| Esc | Toggle Normal / Insert mode |
| Tab / Shift-Tab | Next / previous channel |
| Arrows | Move cursor |
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
| F9-F12 | Mute channels 1-4 |
| Ctrl+F9-F12 | Solo channels 1-4 |
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
| Ctrl+1 | Note off (`===`) |

## Requirements

- Rust 1.70+
- CMake 3.14+ (builds Ableton Link C++ dependency)
- macOS/Linux: virtual MIDI ports created automatically
- Windows: requires a third-party virtual MIDI driver (e.g., [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html))

## Build

```sh
make build    # compile
make run      # compile and run
make test     # run tests (158 tests)
```

## Architecture

```
src/
  main.rs               Entry point, event loop, clap CLI
  app.rs                App state, input, playback engine, undo/redo, file I/O
  midi_file.rs          MIDI file (.mid) export and import
  audio/
    mod.rs              Unified audio engine (SF2 + synth + samples + effects, cpal)
    synth.rs            Built-in synthesizer (8 patches, fundsp Sequencer)
    effects.rs          Stereo reverb (fundsp)
  sample/
    mod.rs              Sample loading (WAV via hound, AIFF parser, dasp conversion)
    playback.rs         Sample voice manager, pitch-shifted rendering
    export.rs           Offline song render to WAV
  tracker/
    pattern.rs          Pattern grid, Cell, Note (serde)
    song.rs             Song, order list, BPM, speed
  link/
    mod.rs              Ableton Link (rusty_link)
  midi/
    mod.rs              MIDI output + input (midir)
  ui/
    mod.rs              Header, status bar, popups
    pattern_editor.rs   Pattern grid renderer
    sample_editor.rs    Sample editor (waveform, trim, loop)
    theme.rs            Color themes (dark, light, monokai)
```

## License

See [LICENSE](LICENSE).
