# Changelog

All notable changes to rtrack will be documented in this file.

## [Unreleased]

### Added
- Pattern editor with 4 channels x 64 rows
- Cursor navigation (arrows, PgUp/PgDn, Home/End)
- Normal and Insert input modes (Esc to toggle)
- Piano keyboard note entry (two-row layout spanning two octaves)
- Hex digit entry for instrument, volume, and effect columns
- Note-off entry (Ctrl+1) and cell clearing (Delete/Backspace)
- Virtual MIDI port (`RTRACK_MIDI`) created on startup (macOS/Linux), visible to DAWs
- Fallback to first available MIDI port on platforms without virtual port support
- MIDI port selection popup (F2) to switch between virtual and hardware ports
- Ableton Link tempo synchronization via rusty_link (F3 to toggle)
  - Bidirectional BPM sync with Link peers
  - Transport start/stop sync with Link session
  - Peer count displayed in header bar
- Tab/Shift-Tab to jump between tracks
- Help screen (F1) showing all keybindings
- Note preview on entry (sends MIDI note-on when inserting notes)
- Real-time pattern playback with classic tracker timing (BPM * speed)
- Playback toggle via Space
- Active note tracking with automatic note-off on channel reuse
- Beat highlighting (every 4th row) and bar highlighting (every 16th row)
- BPM adjustment with [ / ] keys
- Octave selection with + / - keys
- Header bar showing song title, BPM, speed, pattern/order position, octave
- Status bar showing current mode, MIDI connection status, and key hints
- Song data model with multiple patterns and order list
- 40 unit tests covering data model, MIDI engine, Link engine, input handling, and UI logic
