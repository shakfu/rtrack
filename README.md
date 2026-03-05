# rtrack

A TUI music tracker built in Rust with [ratatui](https://ratatui.rs). Currently MIDI-only, with sample support planned.

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

### Normal Mode

| Key | Action |
|-----|--------|
| q   | Quit   |

### Insert Mode

| Key              | Action                          |
|------------------|---------------------------------|
| Piano keys       | Enter note at cursor            |
| `0`-`9`,`a`-`f` | Hex entry (instrument/volume/effect columns) |
| Delete/Backspace | Clear current sub-column        |
| Ctrl+1           | Enter note-off                  |

## Architecture

```
src/
  main.rs                 Event loop (input polling + playback tick)
  app.rs                  App state, keybindings, playback engine
  tracker/
    pattern.rs            Pattern grid, Cell, Note, NoteValue
    song.rs               Song (patterns, order list, BPM, speed)
  link/
    mod.rs                Ableton Link integration (rusty_link wrapper)
  midi/
    mod.rs                MidiEngine (midir wrapper, active note tracking)
  ui/
    mod.rs                Header bar, status bar
    pattern_editor.rs     Pattern grid renderer
```

## License

See [LICENSE](LICENSE).
