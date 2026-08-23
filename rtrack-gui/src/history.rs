//! Undo/redo for the GUI.
//!
//! The types live in `rtrack-core` so the TUI shares them; this module is the
//! re-export plus the GUI's own tests over it. What used to be here could
//! describe cell and sample-bank edits only, which meant adding a pattern or
//! deleting an order entry was not undoable in the GUI at all -- see
//! [`rtrack_core::editor`] for why the shared model has a third case.

pub use rtrack_core::editor::{CellEdit, Edit, EditHistory};

#[cfg(test)]
mod tests {
    use super::*;
    use rtrack_core::tracker::Cell;

    /// The cell edits of a step, for tests that only deal in cell edits.
    fn cells(edit: Edit) -> Vec<CellEdit> {
        match edit {
            Edit::Cells(edits) => edits,
            other => panic!("expected a cell edit, got a {} edit", other.describe()),
        }
    }
    use rtrack_core::tracker::{Note, NoteValue};

    fn edit(row: usize, new: Option<Note>) -> CellEdit {
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

    fn c4() -> Option<Note> {
        Some(Note::On {
            value: NoteValue::C,
            octave: 4,
        })
    }

    #[test]
    fn new_history_has_nothing_to_undo_or_redo() {
        let h = EditHistory::with_budget(usize::MAX, 10);
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn push_then_undo_then_redo_round_trips() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![edit(0, c4())]);
        assert!(h.can_undo());

        let undone = cells(h.undo().expect("undo available"));
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0].new_cell.note, c4());
        assert!(!h.can_undo());
        assert!(h.can_redo());

        let redone = cells(h.redo().expect("redo available"));
        assert_eq!(redone[0].new_cell.note, c4());
        assert!(h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn empty_edit_groups_are_not_recorded() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(Vec::new());
        assert!(!h.can_undo(), "an empty group would be a no-op undo step");
    }

    #[test]
    fn a_new_edit_discards_the_redo_stack() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![edit(0, c4())]);
        h.undo();
        assert!(h.can_redo());
        h.push(vec![edit(1, c4())]);
        assert!(!h.can_redo(), "redo must not survive a divergent edit");
    }

    #[test]
    fn history_is_capped_and_drops_the_oldest_entry() {
        let mut h = EditHistory::with_budget(usize::MAX, 3);
        for row in 0..5 {
            h.push(vec![edit(row, c4())]);
        }
        // Only the last three survive: rows 4, 3, 2 in undo order.
        let mut seen = Vec::new();
        while let Some(edit) = h.undo() {
            seen.push(cells(edit)[0].row);
        }
        assert_eq!(seen, vec![4, 3, 2]);
    }

    #[test]
    fn multi_cell_groups_undo_as_one_step() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![edit(0, c4()), edit(1, c4()), edit(2, c4())]);
        let undone = cells(h.undo().expect("undo available"));
        assert_eq!(undone.len(), 3, "a block edit is one undo step");
        assert!(!h.can_undo());
    }

    #[test]
    fn clear_drops_both_stacks() {
        let mut h = EditHistory::with_budget(usize::MAX, 10);
        h.push(vec![edit(0, c4())]);
        h.undo();
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }
}
