# rtrack-gui

GUI frontend for the rtrack music tracker.

A native desktop application built on [egui](https://docs.rs/egui)/[eframe](https://docs.rs/eframe), wrapping the headless `rtrack-core` library with a graphical pattern editor, transport controls, and interactive dialogs.

## Install

```sh
cargo install rtrack-gui
```

## Usage

```sh
rtrack-gui
```

## Features

- Graphical pattern grid with click-to-position cursor and drag-to-select
- Menu bar with native file dialogs (New/Open/Save/SaveAs) and recent files
- Interactive transport controls (drag to adjust BPM, speed, octave)
- Sidebar with order list and channel mute/solo toggles
- Real-time audio visualization (spectrum analyzer, sample waveform viewer)
- Instrument editor with synth parameter editing and sample slicing
- Undo/redo (100 levels), clipboard (cut/copy/paste)
- Drag-and-drop sample loading
- Song settings, track config, MIDI port selection, and pattern matrix dialogs

## Build requirements

- Rust 1.70+
- CMake 3.14+ (required by Ableton Link)

## License

GPL-3.0-or-later. See [LICENSE](../LICENSE) for the full text.
