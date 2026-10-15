# Phrase editor

A view that shows one phrase in the pattern grid, so a phrase can be read, edited and stepped through without linking it to a track. Follows [g1-g3-sequencing.md](g1-g3-sequencing.md), where phrases could only be seen through a linked column. Code refers to `rtrack-core/src` as of 2026-10-15.

Status: implemented 2026-10-15.

## Problem

Before this, the only way to see a phrase's rows was to link it to a track and read that track's column. The GUI phrase window listed names and lengths only, without indices, so `[P03]` in a header could not be matched to a list entry without counting.

## Design

**One grid, two sheets.** `tracker::Sheet` names what a grid shows: `Order(pos)`, the pattern at an order position, or `Phrase { phrase, track }`. The cell accessors (`Song::address`, `cell_at`, `cell_at_mut`, `set_cell`, `addresses`, `rows_at`, `uses_second_fx`) take `impl Into<Sheet>`, and `usize` converts to `Order`, so existing callers are unchanged. A `Phrase` sheet resolves only in column `track`, to `CellAddr::Phrase`.

The frontends edit through these accessors already. Pointing them at a `Sheet` gives the phrase view every existing edit: note and hex entry, the effect prompt, transpose, block copy, cut, paste and interpolate, and undo. A separate editor, as tables have, would have needed its own copy of all of them.

**The phrase plays through a track.** `TrackerCore::resolve_edit_instrument` and `preview_note_for_cell` use the column as the track whose default instrument and routing apply. A phrase belongs to no track, so the view keeps the track the cursor was on when it opened. That track's column shows the phrase; the cursor cannot leave it.

**Shared state.** `editor::PhraseView` holds the phrase shown and the pattern row to return to. Both frontends keep an `Option<PhraseView>`.

## Behaviour

| | |
|-|-|
| Open / close | F5 in both frontends. TUI: `:pedit [NN]`, or Enter in `:phrases` while the view is open. GUI: View > Phrase editor, or Edit in the Phrases window. |
| Opens on | The phrase the cursor's track plays, at the row shown under the cursor. Otherwise phrase 00. With no phrases, nothing opens. |
| Step through phrases | Ctrl+Left / Ctrl+Right, the keys that step through order entries. |
| New / duplicate | Ctrl+N adds an empty phrase as long as the pattern; Ctrl+D duplicates the one shown. |
| Insert / delete row | Shift the phrase's rows; its length stays the same, as `Pattern::insert_row` does. |
| Length, name | TUI: `:phrases` list. GUI: the bar above the grid. |
| Playback | Plays the song as before. The grid does not follow it; Follow moves the pattern position the view returns to. Playback starts from that position. |
| Undo, load | An undo that removes the phrase shown moves the view to the last phrase, or closes it when none are left. Loading a song closes it. |

## Decisions

1. **Swap the grid, or a separate window.** Swap: it reuses every edit (above). A window would show the pattern alongside, which the swap does not.
2. **Delete a phrase.** Deferred. It renumbers every link, and tracks linked to the deleted phrase need its rows copied in, as unlink does.
3. **Row insert and delete keep the length.** The GUI's pattern insert grows the pattern and the TUI's keeps it; the phrase operations live in core and keep it, so both frontends agree.
