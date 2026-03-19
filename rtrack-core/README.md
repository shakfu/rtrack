# rtrack-core

Headless music tracker core: engine, audio, MIDI, samples, and data model.

This crate provides the complete tracker engine without any UI dependencies. Both the TUI (`rtrack-tui`) and GUI (`rtrack-gui`) frontends wrap `TrackerCore` with their own input handling and rendering.

## Features

- **Tick-based playback engine** -- deterministic, sub-tick resolution with configurable speed
- **Built-in synthesizer** -- 30 preset patches, per-channel effects (filter, distortion, chorus), send/return buses (delay, reverb)
- **Sample playback** -- WAV/AIFF loading, transient-based slicing, pitched playback
- **MIDI I/O** -- virtual ports, hardware routing, MIDI learn for CC mapping
- **Ableton Link** -- tempo and transport sync with other Link-enabled apps
- **Offline export** -- render to WAV, FLAC, or standard MIDI files
- **Serializable data model** -- Song, Pattern, Note, Cell (serde JSON)

## Usage

```rust
use rtrack_core::TrackerCore;

let mut core = TrackerCore::new(); // 4 channels, 64 rows
core.play(0, 0);                   // start playback from order 0, row 0
// In your event loop: core.tick_playback() drives the engine forward.
core.stop();
```

## Build requirements

- Rust 1.70+
- CMake 3.14+ (required by the `rusty_link` C++ dependency for Ableton Link)

## License

GPL-3.0-or-later. See [LICENSE](../LICENSE) for the full text.
