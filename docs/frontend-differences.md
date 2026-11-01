# Frontend differences

The TUI and the GUI share one core, file format and engine. They are not meant to behave identically: the GUI can show more and offer controls a terminal cannot. Every place where they differ is listed here, so each difference is either a decision or a bug.

## Process

- A change that makes the frontends differ adds a row here, with status **Pending**.
- The maintainer reviews it and sets one of:
  - **Approved**: intended. The rationale column says why, and the user documentation mentions it.
  - **Fix**: not intended. One frontend is changed to match the other; then the row is deleted.
- A row stays Pending until reviewed. It does not count as approved by default.

## Register

Recommendations are the reviewer's, not decisions.

| # | Area | TUI | GUI | Status | Rationale / recommendation |
|-|-|-|-|-|-|
| 1 | Export length | Argument to the export command: `:wav 2`, `:wav 90s` | *Export length* setting in the File menu, used by menu items and shortcuts | Pending | Recommend approve: each uses its frontend's input style; the behaviour is the same |
| 2 | Export shortcuts | `e`, `w`, `l` in the pattern matrix | Ctrl+E, Ctrl+W, Ctrl+L anywhere | Pending | Recommend review: TUI Ctrl+E already opens the table editor in the instrument list |
| 3 | MIDI file import | Command line and startup argument | Not available | Pending | Recommend fix: add *Import MIDI* to the GUI File menu |
| 4 | Redo key | Ctrl+Y | Ctrl+Shift+Z | Pending | Recommend fix: accept both in both |
| 5 | Undo in Normal mode | `u` (vi) and Ctrl+Z | Ctrl+Z | Pending | Recommend approve: the TUI follows vi; the GUI has no vi bindings |
| 6 | Settings: Rows | Sets the length of new patterns (`rows_per_pattern`) | Resizes the current pattern, or the playing one during playback | Pending | Recommend fix: expose both meanings in both, with different labels |
| 7 | Settings: BPM range | 32-300 | 20-999 | Pending | Recommend fix: one range from `rtrack-core` |
| 8 | New pattern | Appended at the end of the order list | Inserted after the current order position | Pending | Recommend fix: insert after current in both |
| 9 | Row insert / delete | Shifts rows; the pattern keeps its length | Pattern grows or shrinks by one row | Pending | Recommend fix: use `Pattern::insert_row` / `delete_row` in both |
| 10 | Visualization | Bottom panel: level meters with a spectrum, or the sample waveform | Spectrum with level meters; waveform with draggable trim, slice and loop edges | Pending | Recommend approve: the GUI can draw finer detail and take mouse input |
| 11 | Sample edges | Arrow keys on the trim and loop fields | Drag the edges on the waveform | Pending | Recommend approve: both apply the same rule (`SampleBank::move_boundary`) |
