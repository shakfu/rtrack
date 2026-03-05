# rtrack

A TUI music tracker built in Rust with [ratatui](https://ratatui.rs). Outputs MIDI to external synths/DAWs, or plays directly through a SoundFont (.sf2) via built-in audio engine.

## Features

- Pattern editor with configurable channels (default 4) and rows (default 64)
- Per-cell editing: note, instrument, volume, and effect columns
- Two input modes: Normal (navigation) and Insert (data entry)
- Piano keyboard note entry (two-row layout: `z/s/x/d/c/v/g/b/h/n/j/m` + `q/2/w/3/e/r/5/t/6/y/7/u`)
- Creates a virtual MIDI port (`RTRACK_MIDI`) visible to DAWs and other MIDI software (macOS/Linux)
- Falls back to connecting to first available MIDI port on platforms without virtual port support
- MIDI port selection UI (F2) -- switch between virtual and hardware ports at runtime
- [Ableton Link](https://www.ableton.com/en/link/) tempo synchronization (F3) -- sync BPM and transport with Ableton Live and other Link-enabled apps
- Real-time pattern playback with classic tracker timing (BPM + speed)
- Beat and bar row highlighting (every 4th and 16th row)
- Save/load songs as `.rtrk` JSON files (Ctrl+S to save, pass file path as argument to load)
- Undo/redo (Ctrl+Z / Ctrl+Y) with 100-level history
- Copy/cut/paste entire rows (Ctrl+C / Ctrl+X / Ctrl+V)
- Multiple pattern support: create new (Ctrl+N), clone current (Ctrl+D)
- Order list navigation (Ctrl+Left/Right) and editing (F4 insert, F5 remove)
- Per-channel MIDI channel mapping (tracker channel -> MIDI channel 0-15)
- Channel mute/unmute (F9-F12) with visual dimming on muted channels
- Edit step configuration (`(` / `)` to adjust, 0-16 range)
- Row insert/delete within patterns (Insert/Backspace in Normal mode)
- MIDI input for note entry from external controllers (virtual port `RTRACK_MIDI_IN`)
- Per-pattern length (each pattern can have its own number of rows)
- MIDI CC support via `Cxx` effect (controller from instrument column, value xx)
- Program change via `Exx` effect (sends MIDI program change)
- Song settings dialog (F6) for editing title, BPM, speed, channels, rows
- Instrument list view (F7) with 256 editable instrument slots
- Order list sidebar (always visible) showing order positions
- Color theme cycling (F8) with dark, light, and monokai themes
- Mouse support: click to position cursor, scroll wheel to navigate
- Export to standard MIDI file (Ctrl+E)
- Import from .mid files (pass as CLI argument)
- MIDI clock output (Ctrl+M) at 24 ppqn with start/stop messages
- Built-in SoundFont audio engine (`--sf2 file.sf2`) for direct audio output without external DAW

## Requirements

- Rust 1.70+
- CMake 3.14+ (required to build Ableton Link C++ dependency)
- On macOS/Linux: rtrack creates its own virtual MIDI port (`RTRACK_MIDI`) -- no external setup needed. Just point your DAW or synth to the `RTRACK_MIDI` source.
- On Windows: requires a third-party virtual MIDI driver (e.g., loopMIDI)

## Build and Run

```sh
make build   # compile
make run     # compile and run
make test    # run tests

cargo run -- song.rtrk   # open an existing song
cargo run -- --sf2 gm.sf2           # play through SoundFont (built-in audio)
cargo run -- song.rtrk --sf2 gm.sf2 # open song with SoundFont audio
```

## Keybindings

### Both Modes

| Key           | Action                    |
|---------------|---------------------------|
| Space         | Toggle play/stop          |
| Esc           | Toggle Normal/Insert mode |
| Tab / Shift-Tab | Next / previous track   |
| Arrows        | Move cursor               |
| PgUp/PgDn     | Move cursor 16 rows       |
| Home/End      | Jump to first/last row    |
| `+` / `-`     | Octave up/down            |
| `[` / `]`     | BPM down/up               |
| F1            | Help                      |
| F2            | MIDI port selector        |
| F3            | Toggle Ableton Link       |
| Ctrl+S        | Save                      |
| Ctrl+Z        | Undo                      |
| Ctrl+Y        | Redo                      |
| Ctrl+C        | Copy row                  |
| Ctrl+V        | Paste row                 |
| Ctrl+X        | Cut row                   |
| Ctrl+Right/Left | Next/prev order position |
| F9-F12        | Toggle mute ch 1-4        |
| `(` / `)`     | Edit step down/up         |
| F6            | Song settings dialog      |
| F7            | Instrument list           |
| F8            | Cycle color theme         |
| Ctrl+E        | Export to MIDI file       |
| Ctrl+M        | Toggle MIDI clock output  |

### Normal Mode

| Key | Action |
|-----|--------|
| q   | Quit   |
| Ctrl+N | New pattern (append to order) |
| Ctrl+D | Clone current pattern |
| F4  | Insert order entry |
| F5  | Remove order entry |
| Insert | Insert row at cursor |
| Backspace | Delete row at cursor |

### Insert Mode

| Key              | Action                          |
|------------------|---------------------------------|
| Piano keys       | Enter note at cursor            |
| `0`-`9`,`a`-`f` | Hex entry (instrument/volume/effect columns) |
| Delete/Backspace | Clear current sub-column        |
| Ctrl+1           | Enter note-off                  |

## Cell Format

Each cell in the pattern grid has four columns:

```
C-4 01 80 1F0
 |   |  |  |
 |   |  |  +-- Effect: command (1 hex digit) + value (2 hex digits)
 |   |  +----- Volume: 00-FF (maps to MIDI velocity)
 |   +-------- Instrument: 00-FF
 +------------ Note: pitch + octave (e.g., C-4, F#5) or === (note off)
```

Empty sub-columns display as dashes: `--- -- -- ---`

### Effect Commands

| Cmd | Name            | Description                                      |
|-----|-----------------|--------------------------------------------------|
| 0xy | Arpeggio        | Cycle note, note+x, note+y semitones each tick  |
| 1xx | Portamento up   | Slide pitch up by xx per tick                    |
| 2xx | Portamento down | Slide pitch down by xx per tick                  |
| 3xx | Tone portamento | Slide toward target note at speed xx             |
| 4xy | Vibrato         | Pitch vibrato (speed x, depth y)                 |
| 5xy | Volume slide    | Slide volume up by x, down by y per tick         |
| Bxx | Position jump   | Jump to order position xx                        |
| Cxx | MIDI CC         | Send CC (controller from instrument col, value xx)|
| Dxx | Pattern break   | Break to row xx of next pattern                  |
| Exx | Program change  | Send MIDI program change to program xx           |
| Fxx | Set speed/tempo | xx < 20: set speed (ticks/row), xx >= 20: set BPM|

## Architecture

```
src/
  main.rs                 Event loop (input + mouse polling + playback tick), file arg
  app.rs                  App state, keybindings, playback, undo/redo, clipboard, file I/O
  midi_file.rs            Standard MIDI file (.mid) export and import
  audio/
    mod.rs                SoundFont audio engine (rustysynth + cpal)
  tracker/
    pattern.rs            Pattern grid, Cell, Note, NoteValue (serde)
    song.rs               Song (patterns, order list, BPM, speed, save/load)
  link/
    mod.rs                Ableton Link integration (rusty_link wrapper)
  midi/
    mod.rs                MidiEngine (midir wrapper, active note tracking)
  ui/
    mod.rs                Header bar, status bar, help/port-select popups
    pattern_editor.rs     Pattern grid renderer (mute dimming, cursor highlight)
    theme.rs              Color theme definitions (dark, light, monokai)
```

## License

See [LICENSE](LICENSE).
