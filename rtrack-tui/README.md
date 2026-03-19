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
- Sample loading, waveform editing, transient-based slicing
- MIDI I/O with virtual ports and MIDI learn
- Ableton Link tempo/transport sync
- Export to WAV, FLAC, and standard MIDI files
- Undo/redo, clipboard, autosave

## Build requirements

- Rust 1.70+
- CMake 3.14+ (required by Ableton Link)

## License

GPL-3.0-or-later. See [LICENSE](../LICENSE) for the full text.
