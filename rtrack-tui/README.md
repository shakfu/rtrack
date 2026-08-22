# rtrack-tui

Terminal UI frontend for the rtrack music tracker.

A full-featured TUI built on [ratatui](https://ratatui.rs) and [crossterm](https://docs.rs/crossterm), wrapping the headless `rtrack-core` library with modal keyboard input, pattern editing, and terminal rendering.

## Install

```sh
cargo install rtrack-tui
```

## Usage

```sh
rtrack                              # launch with built-in synth
rtrack song.rtrk                    # open a saved song
rtrack recording.mid                # import a MIDI file
rtrack --sample-dir samples/        # load a directory of samples
```

Press **Esc** to enter Insert mode, play notes with the keyboard (piano layout), and hit **Space** to play back. Press **F1** for the full help screen.

## Features

- Modal editing (Normal/Insert modes) with piano keyboard note entry
- Pattern editor with sub-column cursor (Note/Instrument/Volume/Effect)
- 30 built-in synth patches, per-channel effects, send/return buses
- Sample loading, waveform editing, transient-based slicing (of the whole sample or of a single slice)
- MIDI I/O with virtual ports and MIDI learn
- Ableton Link tempo/transport sync
- Export to WAV, FLAC, and standard MIDI files
- Undo/redo (pattern edits and slicing), clipboard, autosave
- Color themes: dark (default), light, monokai (F8 to cycle)

### Note Entry

Switch to **Insert** mode (Esc) and use the piano keyboard layout:

```text
Lower octave:  z s x d c v g b h n j m
               C C#D D#E F F#G G#A A#B

Upper octave:  q 2 w 3 e r 5 t 6 y 7 u
               C C#D D#E F F#G G#A A#B
```

Use `+`/`-` to shift octave. Tab/Shift+Tab to cycle tracks, arrow keys to navigate.

## Keybindings

### Global (all modes)

| Key | Action |
|-----|--------|
| Space | Play / stop (from current position) |
| Ctrl+Space | Play / stop (from beginning) |
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
| Ctrl+O | Open song (file browser) |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+C / X / V | Copy / cut / paste row (or block if selected) |
| Ctrl+Left / Right | Previous / next order position |
| Ctrl+E | Export MIDI |
| Ctrl+W | Export WAV |
| Ctrl+L | Export FLAC |
| Ctrl+M | Toggle MIDI clock |
| Ctrl+R | Toggle recording (punch-in MIDI during playback) |

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
| `:open` | Open file browser to load a song (same as Ctrl+O) |
| `:recent` | Open recent files list (last 3 songs) |
| `:audio` / `:astat` | Report dropped or rescheduled audio commands |

### Track Config (Enter on channel)

| Key | Action |
|---------|--------|
| Up / Down / Tab | Navigate fields |
| Left / Right | Adjust value (type, instrument, effect params, sample select) |
| Type chars | Edit channel name (when on name field) |
| L | MIDI learn: bind next incoming CC to current parameter |
| U | Remove MIDI learn mapping for current parameter |
| Enter | Open file browser (on Sample field for Sample tracks) |
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

### Sample Editor (Enter on an instrument)

Tab and Shift+Tab move between fields; Up/Down adjust by one, Right/Left by ten.

| Field | Meaning |
|---------|--------|
| Base Note | MIDI note at which the sample plays at its original pitch |
| Trim Start / End | The part of the buffer that plays. For a slice, its span of the shared source |
| Loop / Loop Start / End | Loop points, clamped into the trimmed span |
| Slices | How many pieces `[Equal]` cuts |
| Sensitivity | Onset threshold for `[Transient]`: higher finds more |
| Divide | What gets cut -- `whole sample` or `this slice only` |
| `[Equal]` / `[Transient]` | Enter to slice |

`Divide` is the difference between re-cutting and subdividing. With `whole
sample`, slicing again at a different count re-derives from the sample, so
changing your mind about 8 versus 16 replaces the previous result. With
`this slice only`, the slot being edited is subdivided and its pieces are
named after it -- `amen_S03_S00`, `amen_S03_S01`.

Slices land in consecutive slots starting at the one being sliced. If that
would write over instruments the slicing did not itself create, the first
Enter refuses and says what is in the way; a second Enter goes ahead. Either
way `Ctrl+Z` puts back what was there.

### Pattern Matrix (`:p`)

| Key | Action |
|---------|--------|
| Up / Down / j / k | Navigate order entries |
| PgUp / PgDn | Jump 8 entries |
| Home / End | First / last entry |
| Left / Right / + / - | Change pattern assignment |
| `[` / `]` | Decrease / increase repeat count |
| Insert | Duplicate order entry |
| Delete / Backspace | Remove order entry |
| Ctrl+N | New empty pattern (insert after cursor) |
| Ctrl+D | Clone current pattern (insert after cursor) |
| Enter | Jump to order position and close |
| Esc / q | Close |

## Build requirements

- Rust 1.89+
- CMake 3.14+ (required by Ableton Link)

## License

GPL-3.0-or-later. See [LICENSE](../LICENSE) for the full text.
