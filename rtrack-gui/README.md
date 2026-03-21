# rtrack-gui

GUI frontend for the rtrack music tracker.

A native desktop application built on [egui](https://docs.rs/egui)/[eframe](https://docs.rs/eframe), wrapping the headless `rtrack-core` library with a graphical pattern editor, transport controls, visualization, and interactive dialogs.

## Install

```sh
cargo install rtrack-gui
```

## Usage

```sh
rtrack-gui
```

Drop `.rtrk` files onto the window to open them, or drop WAV/AIFF files to load samples.

## Features

- **Pattern grid** -- custom-painted monospace grid with click-to-position cursor, drag-to-select block regions, scroll wheel navigation, and color-coded sub-columns (note, instrument, volume, effect)
- **Transport bar** -- drag-adjustable BPM, speed, octave, and edit step; play/stop/record buttons; order/row/pattern position display; playback elapsed time; follow mode toggle; mode indicator (Normal/Insert); audio engine status (SF2/Synth); clickable MIDI status and clock mode; Ableton Link toggle with peer count
- **Menu bar** -- File (New, Open, Save, Save As, Load SF2, Load Sample Dir, Recent Files, Export WAV/FLAC/MIDI, Quit), Edit (Undo, Redo, Copy, Cut, Paste, Song Settings), View (Instruments, Pattern Matrix, Help, MIDI Ports, Spectrum, Theme)
- **Sidebar** -- clickable order list with append, channel list with mute/solo toggles
- **Visualization panel** -- real-time FFT spectrum analyzer (64 log-frequency bars, -72 to 0 dB range) with stereo level meters and peak hold; sample waveform viewer with trim/loop markers, playhead tracking across sliced samples, and interactive slicing controls
- **Instrument editor** -- scrollable sidebar with type indicators ([SYN]/[SMP]/[MID]), create/clear buttons; central panel with name editing, type selector (Empty/Synth/Sample/MIDI), pitch bend range
  - Synth: patch selector (30 presets), ADSR envelope sliders, filter (LP/HP/BP with cutoff/resonance/env amount), oscillator (detune/sub-osc/pulse width), FM synthesis (ratio/index)
  - Sample: load file or directory via native dialog, waveform preview with trim/loop markers, base note, trim start/end, loop enable/start/end
  - MIDI: program number selector
- **Sample slicing** -- equal-segment or transient-detection modes with live preview markers on the waveform; auto-applies when parameters change
- **Pattern matrix** -- full-screen view of order list with per-channel data indicators, pattern assignment, repeat counts; create, clone, duplicate, and remove order entries
- **Track config dialog** -- per-channel name, type, MIDI channel, default instrument, volume, pan; effects chain (filter, distortion, chorus, delay, reverb) with per-parameter sliders; MIDI learn/unlearn for all effect parameters
- **Song settings dialog** -- title, BPM, speed, beat/bar highlight intervals, swing, rows per pattern, channel count
- **MIDI ports dialog** -- output/input port selection, virtual port creation, clock source (Internal/External MIDI), MIDI clock output toggle, refresh
- **Help dialog** -- categorized keyboard shortcut reference
- **Drag-and-drop** -- drop `.rtrk` files to open songs, drop WAV/AIFF files to load into sample slots (targets instrument editor selection or first empty slot)
- **Undo/redo** -- 100 levels with dual-stack history
- **Clipboard** -- cell and block cut/copy/paste
- **Block selection** -- drag or Ctrl+B to select regions, with interpolation (Ctrl+I) and transpose (Shift+Up/Down)
- **Color themes** -- Dark (default), Light, Monokai (F8 to cycle)
- **Auto-save** -- periodic save to temp file every 60 seconds
- **Recent files** -- persisted across sessions, accessible from File menu
- **Unsaved changes guard** -- quit interception with Save & Quit / Quit without saving / Cancel

## Keybindings

### General

| Key | Action |
|-----|--------|
| Space | Play / stop |
| Ctrl+Space | Play from start |
| i | Enter Insert mode |
| Escape | Return to Normal mode / close dialog |
| F1 | Toggle help |
| F2 | MIDI ports dialog |
| F4 | Toggle spectrum/visualization panel |
| F7 | Instrument editor |
| F8 | Cycle color theme (Dark/Light/Monokai) |
| Ctrl+P | Pattern matrix |
| Ctrl+S | Save |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |
| Ctrl+F | Toggle follow mode |
| Ctrl+R | Toggle recording |

### Navigation

| Key | Action |
|-----|--------|
| Up / Down | Move cursor row |
| Left / Right | Move sub-column (Note/Instrument/Volume/Effect) |
| Tab / Shift+Tab | Next / previous channel |
| Page Up / Page Down | Jump 16 rows |
| Home / End | First / last row |
| Ctrl+Left / Ctrl+Right | Previous / next order position |

### Editing (Insert Mode)

| Key | Action |
|-----|--------|
| z s x d c v g b h n j m | Notes C C# D D# E F F# G G# A A# B (lower octave) |
| q 2 w 3 e r 5 t 6 y 7 u | Notes C C# D D# E F F# G G# A A# B (upper octave) |
| = (equals) | Note off (`===`) |
| 0-9, a-f | Hex digit (instrument/volume/effect columns) |
| Delete | Clear cell |
| Insert | Insert row |
| Backspace | Delete row |
| + / - | Octave up / down |

### Block Operations

| Key | Action |
|-----|--------|
| Ctrl+B | Start/toggle block selection |
| Ctrl+C | Copy (cell or block) |
| Ctrl+X | Cut (cell or block) |
| Ctrl+V | Paste (cell or block) |
| Ctrl+I | Interpolate (volume/effect values in block) |
| Shift+Up / Down | Transpose note(s) up/down by semitone |

### Pattern / Song

| Key | Action |
|-----|--------|
| Ctrl+N | New pattern (insert after current) |
| Ctrl+D | Clone pattern (insert after current) |
| Enter | Open track config (Normal mode) |

### Export

| Key | Action |
|-----|--------|
| Ctrl+E | Export MIDI |
| Ctrl+W | Export WAV |
| Ctrl+L | Export FLAC |
| Ctrl+M | Toggle MIDI clock output |

## Build requirements

- Rust 1.87+
- CMake 3.14+ (required by Ableton Link)

## License

GPL-3.0-or-later. See [LICENSE](../LICENSE) for the full text.
