//! Editor state shared by both frontends: cursor, selection, clipboard, undo.
//!
//! None of this is UI. It is the state an editing session has regardless of
//! whether the pattern is drawn with ratatui or egui, and it lived in both
//! frontends separately until it started to drift: the two had their own
//! `SubColumn`, their own clipboard, and -- worse -- two undo implementations
//! that had diverged into different data models with different coverage.
//!
//! ## What the two undo models each got right
//!
//! The TUI cloned the whole [`Song`] on every undoable action. Expensive --
//! for a 32-pattern song that is a few hundred kilobytes and a couple of
//! thousand allocations per keystroke, times a hundred history entries -- but
//! it covers *everything*, because everything is in the song: pattern
//! add/clone, order-list edits, and the song settings, as well as cells.
//!
//! The GUI recorded per-cell before/after diffs. Cheap, and right for the hot
//! path, but its `Edit` enum could only describe cells and the sample bank, so
//! adding a pattern or deleting an order entry was **not undoable in the GUI
//! at all**.
//!
//! Neither model is the one to standardise on. [`Edit`] keeps the cheap case
//! cheap and the rare case complete: cell edits are diffs, and a structural
//! change -- rare, and awkward to express as a diff -- carries a snapshot, the
//! way the TUI always did. The cost that mattered was paying snapshot price
//! for *typing*, not for occasionally adding a pattern.

use std::collections::VecDeque;

use crate::constants::{MAX_UNDO_BYTES, MAX_UNDO_STEPS};
use crate::tracker::{Cell, Song};

/// Which field within a channel the edit cursor is on.
///
/// A tracker cell is four fields, and which one has the cursor decides what a
/// keypress means: a letter is a note in the first and a hex digit in the
/// others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubColumn {
    #[default]
    Note,
    Instrument,
    Volume,
    Effect,
}

impl SubColumn {
    /// The next field to the right, wrapping back to the note column.
    ///
    /// Wrapping is what both frontends already did, and it is what makes Tab
    /// walk the cursor along a row rather than parking it on the effect
    /// column.
    pub fn next(self) -> Self {
        match self {
            Self::Note => Self::Instrument,
            Self::Instrument => Self::Volume,
            Self::Volume => Self::Effect,
            Self::Effect => Self::Note,
        }
    }

    /// The next field to the left, wrapping round to the effect column.
    pub fn prev(self) -> Self {
        match self {
            Self::Note => Self::Effect,
            Self::Instrument => Self::Note,
            Self::Volume => Self::Instrument,
            Self::Effect => Self::Volume,
        }
    }

    /// Every field, left to right.
    pub fn all() -> [Self; 4] {
        [Self::Note, Self::Instrument, Self::Volume, Self::Effect]
    }
}

/// One cell's value either side of an edit.
#[derive(Debug, Clone)]
pub struct CellEdit {
    pub pattern_idx: usize,
    pub row: usize,
    pub channel: usize,
    pub old_cell: Cell,
    pub new_cell: Cell,
}

/// A sample-bank change, as the state either side of it.
///
/// Slicing rewrites a run of slots and renames their instruments, so there is
/// little to be saved by recording which; and a snapshot cannot drift out of
/// step with what it describes. Cheap regardless -- slots are `Arc<Sample>`,
/// so no audio is copied.
#[derive(Clone)]
pub struct BankEdit {
    pub before: crate::core::SampleSnapshot,
    pub after: crate::core::SampleSnapshot,
}

/// One undoable step.
#[derive(Clone)]
pub enum Edit {
    /// Pattern cells, recorded as a before/after pair per cell. The common
    /// case, and the one that has to stay cheap: this is what typing produces.
    Cells(Vec<CellEdit>),
    /// The sample bank and instrument table.
    Bank(Box<BankEdit>),
    /// The whole song, either side of a change to its structure -- patterns
    /// added or cloned, order entries inserted or removed, channel or row
    /// counts, tempo, title.
    ///
    /// A snapshot rather than a diff because these changes move data around
    /// wholesale: cloning a pattern renumbers every order entry after it, and
    /// shrinking the channel count truncates every row of every pattern.
    /// Describing that as a diff costs more than copying the song, and they
    /// happen orders of magnitude less often than a keystroke.
    Structure(Box<StructureEdit>),
    /// Several changes that one action made together, undone and redone as
    /// one step.
    ///
    /// The TUI needs this: a single keypress can rewrite the sample bank and
    /// the song at once (slicing renames instruments), and the two have to
    /// come back together or undo puts the editor in a state the user never
    /// created. Applied in order to redo and in reverse to undo.
    Group(Vec<Edit>),
}

/// A whole-song before/after pair. See [`Edit::Structure`].
#[derive(Clone)]
pub struct StructureEdit {
    pub before: Song,
    pub after: Song,
}

impl Edit {
    /// A short description of what kind of step this is, for status messages.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Cells(_) => "cells",
            Self::Bank(_) => "samples",
            Self::Structure(_) => "song structure",
            Self::Group(_) => "several changes",
        }
    }

    /// Roughly how much heap this step holds, for the history's byte budget.
    ///
    /// Deliberately an estimate. The one term that has to be right is the
    /// song snapshot, because that is the term that grows without bound and
    /// the reason the budget exists at all.
    ///
    /// Sample audio is *not* counted. A [`crate::core::SampleSnapshot`] holds
    /// an `Arc` of the bank, so taking one copies no audio; it only starts
    /// costing memory once the live bank has moved on, and by then the figure
    /// recorded here is already fixed. Counting it at push time would charge
    /// a step for memory it does not yet hold, and slicing a large kit would
    /// evict the whole history on the spot.
    pub fn approx_bytes(&self) -> usize {
        match self {
            Self::Cells(edits) => {
                std::mem::size_of::<Self>() + edits.capacity() * std::mem::size_of::<CellEdit>()
            }
            Self::Bank(bank) => {
                std::mem::size_of::<Self>()
                    + (bank.before.instrument_count() + bank.after.instrument_count())
                        * std::mem::size_of::<crate::types::Instrument>()
            }
            Self::Structure(s) => {
                std::mem::size_of::<Self>()
                    + s.before.approx_heap_bytes()
                    + s.after.approx_heap_bytes()
            }
            Self::Group(edits) => {
                std::mem::size_of::<Self>() + edits.iter().map(|e| e.approx_bytes()).sum::<usize>()
            }
        }
    }
}

/// Which control an edit came from, so that consecutive edits to the same one
/// fold into a single undo step.
///
/// Dragging a BPM field from 120 to 160, or holding an arrow key on a sample's
/// trim point, produces one change per frame or per repeat. Recording each as
/// its own step is useless -- undo would walk back through fifty of them -- and
/// it is what kept these controls out of the history altogether. Tagging the
/// edit with its source lets the history amend the step it already has instead
/// of pushing another.
///
/// `control` names the field and `index` separates instances of it, so the
/// trim point of slot 3 does not coalesce with the trim point of slot 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditSource {
    pub control: &'static str,
    pub index: usize,
}

impl EditSource {
    pub fn new(control: &'static str, index: usize) -> Self {
        Self { control, index }
    }
}

/// A step on the undo stack, with its source and its measured size.
///
/// The size is cached rather than recomputed: `approx_bytes` walks every
/// pattern of a song snapshot, and eviction and undo would otherwise pay that
/// walk again for a figure that cannot have changed.
struct Step {
    edit: Edit,
    source: Option<EditSource>,
    bytes: usize,
}

impl Step {
    fn new(edit: Edit, source: Option<EditSource>) -> Self {
        let bytes = edit.approx_bytes();
        Self {
            edit,
            source,
            bytes,
        }
    }
}

/// Fold `next` into `existing`, keeping the original "before" side.
///
/// Hands `next` back in `Err` when the two are not a kind that folds, so the
/// caller can push it as a new step instead. Only the snapshot kinds coalesce: a run
/// of cell diffs describes distinct cells rather than successive values of
/// one, so merging them would be wrong.
fn amend(existing: &mut Edit, next: Edit) -> Result<(), Edit> {
    match (existing, next) {
        (Edit::Structure(old), Edit::Structure(new)) => {
            old.after = new.after;
            Ok(())
        }
        (Edit::Bank(old), Edit::Bank(new)) => {
            old.after = new.after;
            Ok(())
        }
        (_, next) => Err(next),
    }
}

/// Dual-stack undo/redo over [`Edit`] steps, bounded by memory.
///
/// Pushing a new step clears the redo stack, which is the usual rule: once you
/// edit after undoing, the branch you undid is gone.
///
/// The bound is [`MAX_UNDO_BYTES`], not a step count. A step carries whatever
/// it has to put back, so its size follows the song rather than being uniform:
/// a cell edit is tens of bytes and a structural snapshot of a large song is
/// megabytes. Capping the count is generous with a small song and ruinous with
/// a large one -- a hundred steps of a 64x16x256 song is roughly 290MB.
/// [`MAX_UNDO_STEPS`] is a secondary guard for the opposite case, where tiny
/// edits would otherwise pile up hundreds of thousands deep before the byte
/// budget noticed.
///
/// Eviction is from the oldest end, so what is lost is the far end of the
/// history rather than the step you are about to undo.
pub struct EditHistory {
    undo_stack: VecDeque<Step>,
    redo_stack: Vec<Step>,
    max_bytes: usize,
    max_steps: usize,
    /// Running total of `undo_stack`, kept incrementally so that pushing does
    /// not walk the whole history to re-add it up.
    bytes: usize,
}

impl EditHistory {
    /// A history with the standard budget.
    pub fn new() -> Self {
        Self::with_budget(MAX_UNDO_BYTES, MAX_UNDO_STEPS)
    }

    /// A history with a specific budget. Mainly for tests, which cannot
    /// usefully fill 64MB.
    pub fn with_budget(max_bytes: usize, max_steps: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_bytes,
            // A zero would make every push drop what it just pushed, leaving
            // `can_undo` false immediately after an edit.
            max_steps: max_steps.max(1),
            bytes: 0,
        }
    }

    /// Record a group of cell edits as one step. An empty group is not a step
    /// and is dropped, so an edit that changed nothing does not consume an
    /// undo press.
    pub fn push(&mut self, edits: Vec<CellEdit>) {
        if edits.is_empty() {
            return;
        }
        self.push_edit(Edit::Cells(edits));
    }

    /// Record a sample-bank change: slicing, which overwrites whole slots.
    pub fn push_bank(
        &mut self,
        before: crate::core::SampleSnapshot,
        after: crate::core::SampleSnapshot,
    ) {
        self.push_edit(Edit::Bank(Box::new(BankEdit { before, after })));
    }

    /// Record a change to the song's structure, as the song either side of it.
    pub fn push_structure(&mut self, before: Song, after: Song) {
        self.push_edit(Edit::Structure(Box::new(StructureEdit { before, after })));
    }

    pub fn push_edit(&mut self, edit: Edit) {
        self.push_step(Step::new(edit, None));
    }

    /// Record a step attributed to a control, folding it into the previous
    /// step when that came from the same one.
    ///
    /// Coalescing keeps the *original* "before" and takes the newest "after",
    /// so undoing a dragged BPM returns to where it stood before the drag
    /// began rather than to the frame before last. Anything else happening in
    /// between -- a note entered, a different field touched -- puts a step of
    /// its own on top, and the next edit to this control starts fresh.
    pub fn push_from(&mut self, source: EditSource, edit: Edit) {
        let mut edit = edit;
        if let Some(last) = self.undo_stack.back_mut() {
            if last.source == Some(source) {
                match amend(&mut last.edit, edit) {
                    Ok(()) => {
                        self.bytes = self.bytes.saturating_sub(last.bytes);
                        last.bytes = last.edit.approx_bytes();
                        self.bytes += last.bytes;
                        self.redo_stack.clear();
                        self.evict_to_budget();
                        return;
                    }
                    // Not a kind that folds; handed back so it can be pushed.
                    Err(returned) => edit = returned,
                }
            }
        }
        self.push_step(Step::new(edit, Some(source)));
    }

    /// End any run of coalescing, so the next edit starts a new step even if
    /// it comes from the same control.
    ///
    /// Called when focus leaves a field or a dialog closes: two visits to the
    /// same control with a gap between them are two things the user did.
    pub fn break_coalescing(&mut self) {
        if let Some(last) = self.undo_stack.back_mut() {
            last.source = None;
        }
    }

    fn push_step(&mut self, step: Step) {
        self.redo_stack.clear();
        self.bytes += step.bytes;
        self.undo_stack.push_back(step);
        self.evict_to_budget();
    }

    /// Drop the oldest steps until the history is inside both budgets.
    ///
    /// The newest step is never evicted, however large it is: a single
    /// snapshot of a song bigger than the whole budget would otherwise be
    /// pushed and immediately dropped, leaving the editor unable to undo the
    /// edit it just made. One step over budget is better than an undo key
    /// that silently does nothing.
    fn evict_to_budget(&mut self) {
        while self.undo_stack.len() > 1
            && (self.bytes > self.max_bytes || self.undo_stack.len() > self.max_steps)
        {
            if let Some(dropped) = self.undo_stack.pop_front() {
                self.bytes = self.bytes.saturating_sub(dropped.bytes);
            }
        }
    }

    /// Move the most recent step onto the redo stack and hand it back, for the
    /// caller to apply the "before" side of.
    pub fn undo(&mut self) -> Option<Edit> {
        let step = self.undo_stack.pop_back()?;
        self.bytes = self.bytes.saturating_sub(step.bytes);
        let edit = step.edit.clone();
        self.redo_stack.push(step);
        Some(edit)
    }

    /// Counterpart to [`EditHistory::undo`]: hand back the step to re-apply
    /// the "after" side of.
    pub fn redo(&mut self) -> Option<Edit> {
        let step = self.redo_stack.pop()?;
        self.bytes += step.bytes;
        let edit = step.edit.clone();
        self.undo_stack.push_back(step);
        self.evict_to_budget();
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Drop the redo branch without touching the undo stack.
    ///
    /// For an editor that snapshots before it mutates: the moment a new edit
    /// begins, the branch that was undone is unreachable, even though the
    /// step recording it is not finished yet.
    pub fn clear_redo(&mut self) {
        self.redo_stack.clear();
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.bytes = 0;
    }

    /// Steps currently on the undo stack. For tests and status displays.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Roughly how much memory the undo stack is holding.
    pub fn approx_bytes(&self) -> usize {
        self.bytes
    }
}

impl Default for EditHistory {
    fn default() -> Self {
        Self::new()
    }
}

/// Copied cells, waiting to be pasted. Shared by both frontends.
///
/// A run and a rectangle are kept apart rather than treating a run as a
/// one-column rectangle: copying a cell, copying a row, and copying a block
/// are different gestures, and pasting should put back whichever was copied
/// last. The two frontends differ only in what they put in `run` -- the GUI
/// copies a single cell, the TUI copies a whole row across channels -- which
/// is why it is a `Vec` rather than either a `Cell` or a fixed shape.
#[derive(Default, Clone)]
pub struct Clipboard {
    /// A run of cells: one cell, or a row across channels.
    run: Option<Vec<Cell>>,
    /// A rectangle of cells, indexed `[row][channel]`.
    block: Option<Vec<Vec<Cell>>>,
}

impl Clipboard {
    /// Store a single cell, as the GUI's copy does.
    pub fn set_cell(&mut self, cell: Cell) {
        self.run = Some(vec![cell]);
    }

    /// The single cell most recently copied, if the run holds exactly one.
    pub fn cell(&self) -> Option<Cell> {
        match self.run.as_deref() {
            Some([cell]) => Some(*cell),
            _ => None,
        }
    }

    /// Store a run of cells, as the TUI's row copy does.
    pub fn set_run(&mut self, cells: Vec<Cell>) {
        self.run = Some(cells);
    }

    pub fn run(&self) -> Option<&[Cell]> {
        self.run.as_deref()
    }

    pub fn set_block(&mut self, block: Vec<Vec<Cell>>) {
        self.block = Some(block);
    }

    pub fn block(&self) -> Option<&Vec<Vec<Cell>>> {
        self.block.as_ref()
    }

    pub fn has_block(&self) -> bool {
        self.block.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.run.is_none() && self.block.is_none()
    }

    pub fn clear(&mut self) {
        self.run = None;
        self.block = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{Note, NoteValue};

    fn c4() -> Option<Note> {
        Some(Note::On {
            value: NoteValue::C,
            octave: 4,
        })
    }

    fn cell_edit(row: usize, new: Option<Note>) -> CellEdit {
        CellEdit {
            pattern_idx: 0,
            row,
            channel: 0,
            old_cell: Cell::default(),
            new_cell: Cell {
                note: new,
                ..Cell::default()
            },
        }
    }

    fn cells_of(edit: Edit) -> Vec<CellEdit> {
        match edit {
            Edit::Cells(edits) => edits,
            other => panic!("expected cell edits, got {}", other.describe()),
        }
    }

    // -- SubColumn --

    #[test]
    fn sub_columns_walk_right_and_wrap() {
        let mut at = SubColumn::Note;
        for expected in [
            SubColumn::Instrument,
            SubColumn::Volume,
            SubColumn::Effect,
            SubColumn::Note,
        ] {
            at = at.next();
            assert_eq!(at, expected);
        }
    }

    #[test]
    fn sub_columns_walk_left_and_wrap() {
        let mut at = SubColumn::Note;
        for expected in [
            SubColumn::Effect,
            SubColumn::Volume,
            SubColumn::Instrument,
            SubColumn::Note,
        ] {
            at = at.prev();
            assert_eq!(at, expected);
        }
    }

    #[test]
    fn next_and_prev_are_inverses_for_every_field() {
        for sub in SubColumn::all() {
            assert_eq!(sub.next().prev(), sub, "{sub:?}");
            assert_eq!(sub.prev().next(), sub, "{sub:?}");
        }
    }

    // -- History --

    #[test]
    fn an_edit_that_changed_nothing_is_not_a_step() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(Vec::new());
        assert!(
            !h.can_undo(),
            "an empty group must not consume an undo press"
        );
    }

    #[test]
    fn undo_hands_back_the_step_and_redo_hands_it_back_again() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![cell_edit(0, c4())]);

        let undone = cells_of(h.undo().expect("a step to undo"));
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0].new_cell.note, c4());
        assert!(!h.can_undo());
        assert!(h.can_redo());

        let redone = cells_of(h.redo().expect("a step to redo"));
        assert_eq!(redone[0].new_cell.note, c4());
        assert!(h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn editing_after_an_undo_discards_the_branch_that_was_undone() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![cell_edit(0, c4())]);
        h.undo();
        assert!(h.can_redo());

        h.push(vec![cell_edit(1, c4())]);
        assert!(!h.can_redo(), "the undone branch should be gone");
    }

    #[test]
    fn the_oldest_step_falls_off_when_the_step_ceiling_is_reached() {
        let mut h = EditHistory::with_budget(usize::MAX, 3);
        for row in 0..5 {
            h.push(vec![cell_edit(row, c4())]);
        }
        assert_eq!(h.undo_depth(), 3);

        // What is left is the three most recent, newest first.
        for expected_row in [4, 3, 2] {
            let step = cells_of(h.undo().expect("a step"));
            assert_eq!(step[0].row, expected_row);
        }
        assert!(!h.can_undo());
    }

    /// The bound that matters: a step's size follows the song, so counting
    /// steps caps the wrong quantity.
    #[test]
    fn the_history_stays_inside_its_byte_budget() {
        let song = Song::new(8, 64);
        let step_bytes = Edit::Structure(Box::new(StructureEdit {
            before: song.clone(),
            after: song.clone(),
        }))
        .approx_bytes();

        // Room for about four steps, and far more than four allowed by count.
        let mut h = EditHistory::with_budget(step_bytes * 4 + step_bytes / 2, 1000);
        for _ in 0..20 {
            h.push_structure(song.clone(), song.clone());
        }

        assert!(
            h.approx_bytes() <= step_bytes * 5,
            "history holds {} bytes, budget was about {}",
            h.approx_bytes(),
            step_bytes * 4
        );
        assert!(h.undo_depth() <= 5, "depth {}", h.undo_depth());
        assert!(h.undo_depth() >= 3, "budget should still allow a few steps");
    }

    /// A bigger song gets a shallower history for the same memory, which is
    /// the whole point of budgeting by bytes.
    #[test]
    fn a_larger_song_gets_fewer_steps_for_the_same_budget() {
        let small = Song::new(4, 32);
        let mut large = Song::new(16, 256);
        for _ in 0..8 {
            large.add_pattern();
        }

        let budget = 4 * 1024 * 1024;
        let depth = |song: &Song| {
            let mut h = EditHistory::with_budget(budget, 100_000);
            for _ in 0..200 {
                h.push_structure(song.clone(), song.clone());
            }
            h.undo_depth()
        };

        let small_depth = depth(&small);
        let large_depth = depth(&large);
        assert!(
            small_depth > large_depth,
            "small song got {small_depth} steps, large song {large_depth}"
        );
        assert!(large_depth >= 1, "a large song must still get some history");
    }

    /// A single step larger than the entire budget must still be undoable.
    /// Dropping it would leave the editor unable to undo the edit just made.
    #[test]
    fn a_step_bigger_than_the_budget_is_still_kept() {
        let song = Song::new(16, 256);
        let mut h = EditHistory::with_budget(1, 100);
        h.push_structure(song.clone(), song);

        assert!(
            h.can_undo(),
            "the newest step must survive its own eviction"
        );
        assert_eq!(h.undo_depth(), 1);
    }

    #[test]
    fn undoing_and_redoing_keeps_the_byte_count_honest() {
        let song = Song::new(4, 32);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        h.push_structure(song.clone(), song.clone());
        h.push(vec![cell_edit(0, c4())]);
        let full = h.approx_bytes();

        h.undo();
        h.undo();
        assert_eq!(h.approx_bytes(), 0, "an empty stack holds nothing");

        h.redo();
        h.redo();
        assert_eq!(h.approx_bytes(), full, "redo should restore the same total");
    }

    #[test]
    fn a_group_is_one_step_and_costs_the_sum_of_its_parts() {
        let song = Song::new(2, 16);
        let cells = Edit::Cells(vec![cell_edit(0, c4())]);
        let structure = Edit::Structure(Box::new(StructureEdit {
            before: song.clone(),
            after: song,
        }));
        let expected = cells.approx_bytes() + structure.approx_bytes();

        let mut h = EditHistory::with_budget(usize::MAX, 100);
        h.push_edit(Edit::Group(vec![cells, structure]));

        assert_eq!(h.undo_depth(), 1, "a group is one step");
        assert!(h.approx_bytes() >= expected);
        match h.undo().expect("a step") {
            Edit::Group(parts) => assert_eq!(parts.len(), 2),
            other => panic!("expected a group, got {}", other.describe()),
        }
    }

    #[test]
    fn clearing_drops_both_directions() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![cell_edit(0, c4())]);
        h.undo();
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    /// The whole point of `Edit::Structure`: the GUI could not undo these at
    /// all, and the TUI could only by snapshotting the song for every
    /// keystroke as well.
    #[test]
    fn a_structural_change_round_trips_through_the_same_history_as_cells() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        let before = Song::new(2, 16);
        let mut after = before.clone();
        after.add_pattern();

        h.push(vec![cell_edit(0, c4())]);
        h.push_structure(before.clone(), after.clone());

        match h.undo().expect("a step") {
            Edit::Structure(s) => {
                assert_eq!(s.before.patterns.len(), 1);
                assert_eq!(s.after.patterns.len(), 2);
            }
            other => panic!("expected a structural step, got {}", other.describe()),
        }
        // The cell step underneath is untouched and still next in line.
        assert_eq!(cells_of(h.undo().expect("a step")).len(), 1);
    }

    #[test]
    fn steps_of_different_kinds_describe_themselves() {
        assert_eq!(Edit::Cells(Vec::new()).describe(), "cells");
        assert_eq!(
            Edit::Structure(Box::new(StructureEdit {
                before: Song::new(1, 4),
                after: Song::new(1, 4),
            }))
            .describe(),
            "song structure"
        );
    }

    // -- Coalescing --

    fn structure(before: &Song, after: &Song) -> Edit {
        Edit::Structure(Box::new(StructureEdit {
            before: before.clone(),
            after: after.clone(),
        }))
    }

    fn bpm(n: u16) -> Song {
        let mut s = Song::new(2, 16);
        s.bpm = n;
        s
    }

    /// A drag produces one change per frame. All of them are one undo step,
    /// and undoing it returns to where the value stood before the drag began.
    #[test]
    fn a_run_of_edits_to_one_control_is_a_single_step() {
        let source = EditSource::new("song.bpm", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);

        let start = bpm(120);
        let mut previous = start.clone();
        for n in [121, 130, 145, 160] {
            let next = bpm(n);
            h.push_from(source, structure(&previous, &next));
            previous = next;
        }

        assert_eq!(h.undo_depth(), 1, "a drag should be one step");
        match h.undo().expect("a step") {
            Edit::Structure(s) => {
                assert_eq!(s.before.bpm, 120, "undo must return to before the drag");
                assert_eq!(s.after.bpm, 160, "redo must return to the end of it");
            }
            other => panic!("expected a structural step, got {}", other.describe()),
        }
    }

    #[test]
    fn edits_to_different_controls_are_separate_steps() {
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);
        h.push_from(EditSource::new("song.bpm", 0), structure(&song, &bpm(130)));
        h.push_from(EditSource::new("song.swing", 0), structure(&song, &song));
        assert_eq!(h.undo_depth(), 2);
    }

    /// The same field of two different samples is two controls.
    #[test]
    fn the_index_keeps_instances_of_one_control_apart() {
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);
        h.push_from(
            EditSource::new("sample.trim_start", 3),
            structure(&song, &song),
        );
        h.push_from(
            EditSource::new("sample.trim_start", 4),
            structure(&song, &song),
        );
        assert_eq!(h.undo_depth(), 2);
    }

    /// Something else happening in between ends the run, so a later edit to
    /// the same control does not reach back past it.
    #[test]
    fn an_unrelated_edit_between_two_runs_keeps_them_apart() {
        let source = EditSource::new("song.bpm", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);

        h.push_from(source, structure(&song, &bpm(130)));
        h.push(vec![cell_edit(0, c4())]);
        h.push_from(source, structure(&bpm(130), &bpm(140)));

        assert_eq!(h.undo_depth(), 3, "the note must not be swallowed");
    }

    #[test]
    fn breaking_coalescing_starts_a_new_step_for_the_same_control() {
        let source = EditSource::new("song.bpm", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);

        h.push_from(source, structure(&song, &bpm(130)));
        h.break_coalescing();
        h.push_from(source, structure(&bpm(130), &bpm(140)));

        assert_eq!(h.undo_depth(), 2, "closing the dialog should end the run");
    }

    /// Cell diffs describe distinct cells rather than successive values of
    /// one, so they must never fold together.
    #[test]
    fn cell_edits_do_not_coalesce_even_from_one_source() {
        let source = EditSource::new("pattern.cell", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        h.push_from(source, Edit::Cells(vec![cell_edit(0, c4())]));
        h.push_from(source, Edit::Cells(vec![cell_edit(1, c4())]));
        assert_eq!(h.undo_depth(), 2);
    }

    /// A coalesced run must not grow the history's byte count with every
    /// frame of a drag -- that was half the reason these controls were left
    /// out of undo.
    #[test]
    fn coalescing_does_not_grow_the_history() {
        let source = EditSource::new("song.bpm", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);

        h.push_from(source, structure(&song, &song));
        let after_one = h.approx_bytes();
        for _ in 0..500 {
            h.push_from(source, structure(&song, &song));
        }

        assert_eq!(h.undo_depth(), 1);
        assert_eq!(
            h.approx_bytes(),
            after_one,
            "500 frames of a drag should weigh the same as one step"
        );
    }

    #[test]
    fn a_coalesced_run_still_clears_the_redo_branch() {
        let source = EditSource::new("song.bpm", 0);
        let mut h = EditHistory::with_budget(usize::MAX, 100);
        let song = bpm(120);

        h.push_from(source, structure(&song, &bpm(130)));
        h.undo();
        assert!(h.can_redo());

        h.push_from(source, structure(&song, &bpm(140)));
        assert!(!h.can_redo(), "editing after an undo kills the redo branch");
    }

    // -- Clipboard --

    #[test]
    fn a_fresh_clipboard_is_empty_and_clears_back_to_empty() {
        let mut c = Clipboard::default();
        assert!(c.is_empty());

        c.set_cell(Cell::default());
        assert!(!c.is_empty());

        c.set_block(vec![vec![Cell::default()]]);
        c.clear();
        assert!(c.is_empty());
        assert!(c.run().is_none() && c.block().is_none());
    }

    /// The GUI copies one cell, the TUI copies a row. Both go in the same
    /// slot, and `cell()` only answers for the one-cell case.
    #[test]
    fn a_single_cell_and_a_row_share_one_slot() {
        let mut c = Clipboard::default();

        c.set_cell(Cell {
            note: c4(),
            ..Cell::default()
        });
        assert_eq!(c.cell().map(|x| x.note), Some(c4()));
        assert_eq!(c.run().map(|r| r.len()), Some(1));

        c.set_run(vec![Cell::default(); 4]);
        assert_eq!(c.run().map(|r| r.len()), Some(4));
        assert!(
            c.cell().is_none(),
            "a four-cell row is not a single copied cell"
        );
    }

    #[test]
    fn a_block_and_a_run_do_not_displace_each_other() {
        let mut c = Clipboard::default();
        c.set_run(vec![Cell::default(); 2]);
        c.set_block(vec![vec![Cell::default(); 2]; 3]);

        assert!(c.has_block());
        assert_eq!(c.run().map(|r| r.len()), Some(2));
        assert_eq!(c.block().map(|b| b.len()), Some(3));
    }
}
