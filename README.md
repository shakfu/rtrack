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
| **Built-in synth** | Default (always on) | 30 waveform patches with ADSR envelopes, SVF filters, sub-oscillator, and FM synthesis. Select with `Exx` effect (0-29), or configure per-instrument (F7 > Tab). |
| **SoundFont** | `--sf2 path/to/file.sf2` | General MIDI playback via [rustysynth](https://github.com/sinshu/rustysynth). Replaces built-in synth for note playback. |
| **Samples** | `--sample 0:kick.wav` or `--sample-dir path/` | Load WAV/AIFF files into instrument slots. Pitch-shifted playback with loop points. |

All modes output through [cpal](https://crates.io/crates/cpal) with per-channel effects (distortion, filter, chorus, delay, reverb) and a master stereo delay. MIDI output runs in parallel regardless of audio mode.

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

Use `+`/`-` to shift octave. Tab/Shift+Tab to cycle tracks, arrow keys to navigate.

## Features

### Pattern Editing

- Up to 8 channels with Tab/Shift+Tab track cycling (wraps around)
- Track Config popup (Enter on channel): set channel type (Midi/Synth/Sample), MIDI channel, default instrument (used as fallback during playback and auto-filled on note entry), and per-channel effects
- Column headers above the pattern grid (shows channel name or "Not In Vl Fx" labels)
- Channel rename (Ctrl+R): name channels ("Kick", "Bass", etc.) shown in headers
- Configurable channel count and rows per pattern (default 4 channels, 64 rows)
- Note, instrument, volume, and effect columns per cell
- Normal mode (navigation) and Insert mode (data entry)
- Edit step (`(`/`)`) -- auto-advance cursor by N rows after each entry
- Row insert/delete, copy/cut/paste entire rows
- Block selection (Ctrl+B): select a rectangular region, then copy/cut/paste the block
- Interpolation tool (Ctrl+I): fill volume/effect ramps across a block selection
- Note transpose (Shift+Up/Down): shift notes up or down by semitone (works on cursor or block)
- Follow mode (Ctrl+F): cursor follows playback position (on by default, toggle off to navigate freely)
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
- Sample editor (Enter from instrument list): trim, loop points, base note, waveform preview, slice tools
- Sample slicing: auto-slice samples into equal segments or by transient detection (configurable sensitivity)
  - Slice results create new instruments + sample refs with correct trim points
  - Equal-segment: divides sample into N equal parts
  - Transient detection: RMS energy envelope derivative with ~5ms windows, 50ms minimum gap between onsets
- Synth editor (Tab from instrument list): per-instrument waveform, ADSR, filter (type/cutoff/resonance/env), detune, sub-oscillator, FM ratio/index, pulse width
- Pitch-shifted playback with cubic hermite interpolation, up to 32 simultaneous voices with ADSR envelopes
- Smart voice stealing: quietest voice is stolen when at capacity
- Per-channel volume control (applied as velocity scaling during playback)

### Built-in Synth Patches

30 patches available via `Exx` program change or the synth editor (F7 > Tab):

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

Each Synth/Sample track can have its own effects chain, configured via Track Config (Enter):

| Effect | Parameters |
|--------|-----------|
| Distortion | Enable, drive amount |
| Filter | Enable, cutoff, resonance |
| Chorus | Enable, rate, depth, mix |
| Delay | Enable, time (10-2000ms), feedback, mix |
| Reverb | Enable, room size, damping, mix |

### Pattern Effects

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
- Atomic save -- writes to temp file then renames, preventing corruption on crash
- Dirty flag -- `[*]` in header when unsaved changes exist, quit confirmation prompt
- Import from standard MIDI files (`.mid`) with CC and program change preservation
- Export to MIDI (Ctrl+E)
- Export to WAV (Ctrl+W) -- offline render with synth, samples, and effects
- Export to FLAC (Ctrl+L) -- lossless audio export
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
- **Synth params**: optional per-instrument synthesis parameters (waveform, ADSR envelope, filter type/cutoff/resonance/envelope, detune, sub-oscillator, FM ratio/index, pulse width). When present, overrides the channel's default patch. New fields use serde defaults for backwards compatibility.
- **Sample refs**: file paths stored relative to the `.rtrk` file, plus all metadata (base note, trim, loop points). Audio data is not embedded -- samples are reloaded from disk on open. Missing files produce a warning but do not block loading.
- **Backwards compatible**: old `.rtrk` files without `instruments`, `sample_refs`, or `synth_params` fields load fine.

## Keybindings

### Global (all modes)

| Key | Action |
|-----|--------|
| Space | Play / stop |
| Esc | Toggle Normal / Insert mode |
| Tab / Shift+Tab | Next / previous track (wraps around) |
| Enter | Open Track Config for current channel |
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
| Shift+Up / Down | Transpose note(s) up / down by semitone |
| Ctrl+B | Toggle block selection |
| Ctrl+I | Interpolate block (volume/effect ramp) |
| Ctrl+F | Toggle follow mode (cursor follows playback) |
| Ctrl+S | Save |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+C / X / V | Copy / cut / paste row (or block if selected) |
| Ctrl+Left / Right | Previous / next order position |
| Ctrl+E | Export MIDI |
| Ctrl+W | Export WAV |
| Ctrl+L | Export FLAC |
| Ctrl+M | Toggle MIDI clock |

### Normal Mode

| Key | Action |
|-----|--------|
| q | Quit (confirms if unsaved changes) |
| `:` | Enter command mode |
| Ctrl+N | New pattern |
| Ctrl+D | Clone current pattern |
| Insert | Insert row at cursor |
| Backspace | Delete row at cursor |

### Insert Mode

| Key | Action |
|-----|--------|
| Piano keys | Enter note (see [Note Entry](#note-entry)) |
| `0`-`9`, `a`-`f` | Hex digit (instrument / volume / effect columns) |
| Delete / Backspace | Clear cell |
| `=` | Note off (`===`) |

### Command Mode (`:` from Normal mode)

| Command | Action |
|---------|--------|
| `:p` / `:pattern` | Open pattern matrix |
| `:set` / `:settings` | Song settings |
| `:fx` / `:effects` | Track config / effects editor |
| `:inst` / `:instruments` | Instrument list |
| `:midi` | MIDI port selector |
| `:link` | Toggle Ableton Link |
| `:w` / `:write` | Save |
| `:q` / `:quit` | Quit |
| `:q!` | Force quit (discard changes) |
| `:wq` | Save and quit |
| `:h` / `:help` | Help screen |
| `:ew` / `:wav` | Export WAV |
| `:ef` / `:flac` | Export FLAC |
| `:em` / `:exportmidi` | Export MIDI |
| `:load` | Open file browser to load a sample |
| `:open` | Open file browser to load a song |

### Track Config (Enter on channel)

| Key | Action |
|---------|--------|
| Up / Down / Tab | Navigate fields |
| Left / Right | Adjust value (type, instrument, effect params) |
| Type chars | Edit channel name (when on name field) |
| Enter | Open file browser (on Load field for Sample tracks) |
| Enter / Esc | Save and close |

### Instrument List (F7)

| Key | Action |
|---------|--------|
| Up / Down | Navigate instruments |
| PgUp / PgDn | Jump 16 slots |
| Enter | Open sample editor for selected instrument |
| Tab | Open synth editor for selected instrument |
| Type chars | Edit instrument name |
| Backspace | Delete character from name |
| Esc / F7 | Close |

### Pattern Matrix (`:p`)

| Key | Action |
|---------|--------|
| Up / Down / j / k | Navigate order entries |
| PgUp / PgDn | Jump 8 entries |
| Home / End | First / last entry |
| Left / Right / +  / - | Change pattern assignment |
| `[` / `]` | Decrease / increase repeat count |
| Insert | Duplicate order entry |
| Delete / Backspace | Remove order entry |
| Ctrl+N | New empty pattern (insert after cursor) |
| Ctrl+D | Clone current pattern (insert after cursor) |
| Enter | Jump to order position and close |
| Esc / q | Close |

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
| `all-patches.rtrk` | Built-in synth patches cycled with `Exx` program change |
| `fundsp-pad.rtrk` | FundspPad patch (program 8) -- pad chord progression using fundsp synthesis |
| `speed-tempo.rtrk` | `Fxx` effect -- speed changes (< 0x20) and tempo changes (>= 0x20) |
| `sliced-amen.rtrk` | Sample slicing -- 8 equal slices of amen.wav played sequentially (170 BPM) |
| `drumloops.rtrk` | Looping drum slices -- 8 amen.wav slices with loop points enabled (130 BPM) |

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
make test     # run tests (285 tests)
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
    synth.rs            Built-in subtractive synth (30 patches, PolyBLEP + SVF + FM + sub-osc + ADSR)
    channel_effects.rs  Per-channel effects (distortion, filter, chorus, delay, reverb)
    envelope.rs         Shared ADSR envelope (used by synth + sample voices)
    effects.rs          Master stereo delay effect (fundsp)
  sample/
    mod.rs              Sample loading (WAV/AIFF), slicing (equal-segment, transient detection)
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
    pattern_editor.rs   Pattern grid renderer
    sample_editor.rs    Sample editor (waveform, trim, loop)
    synth_editor.rs     Synth editor (waveform, ADSR, filter, FM, sub-osc)
    theme.rs            Color themes (dark, light, monokai)
```

## License

See [LICENSE](LICENSE).
