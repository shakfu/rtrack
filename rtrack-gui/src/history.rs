use rtrack_core::core::SampleSnapshot;
use rtrack_core::tracker::Cell;

#[derive(Clone)]
pub struct CellEdit {
    pub pattern_idx: usize,
    pub row: usize,
    pub channel: usize,
    pub old_cell: Cell,
    pub new_cell: Cell,
}

/// A sample-bank change, as the state either side of it.
///
/// Slicing rewrites a run of slots and renames their instruments, so there
/// is little to be saved by recording which; and a snapshot cannot drift out
/// of step with what it describes. Cheap regardless -- slots are
/// `Arc<Sample>`, so no audio is copied.
#[derive(Clone)]
pub struct BankEdit {
    pub before: SampleSnapshot,
    pub after: SampleSnapshot,
}

/// One undoable step.
#[derive(Clone)]
pub enum Edit {
    /// Pattern cells, recorded as a before/after pair per cell.
    Cells(Vec<CellEdit>),
    /// The sample bank and instrument table.
    Bank(Box<BankEdit>),
}

pub struct EditHistory {
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
    max_history: usize,
}

impl EditHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
        }
    }

    pub fn push(&mut self, edits: Vec<CellEdit>) {
        if edits.is_empty() {
            return;
        }
        self.push_edit(Edit::Cells(edits));
    }

    /// Record a sample-bank change: slicing, which overwrites whole slots.
    pub fn push_bank(&mut self, before: SampleSnapshot, after: SampleSnapshot) {
        self.push_edit(Edit::Bank(Box::new(BankEdit { before, after })));
    }

    fn push_edit(&mut self, edit: Edit) {
        self.redo_stack.clear();
        self.undo_stack.push(edit);
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Pop the most recent edit group from the undo stack.
    /// Returns the edits so the caller can apply old_cell values.
    /// Move the most recent step onto the redo stack and hand it back, for
    /// the caller to apply the "before" side of.
    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.undo_stack.pop()?;
        self.redo_stack.push(edit.clone());
        Some(edit)
    }

    /// Counterpart to [`EditHistory::undo`]: hand back the step to re-apply
    /// the "after" side of.
    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.redo_stack.pop()?;
        self.undo_stack.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cell edits of a step, for tests that only deal in cell edits.
    fn cells(edit: Edit) -> Vec<CellEdit> {
        match edit {
            Edit::Cells(edits) => edits,
            Edit::Bank(_) => panic!("expected a cell edit, got a bank edit"),
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
        let h = EditHistory::new(10);
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn push_then_undo_then_redo_round_trips() {
        let mut h = EditHistory::new(10);
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
        let mut h = EditHistory::new(10);
        h.push(Vec::new());
        assert!(!h.can_undo(), "an empty group would be a no-op undo step");
    }

    #[test]
    fn a_new_edit_discards_the_redo_stack() {
        let mut h = EditHistory::new(10);
        h.push(vec![edit(0, c4())]);
        h.undo();
        assert!(h.can_redo());
        h.push(vec![edit(1, c4())]);
        assert!(!h.can_redo(), "redo must not survive a divergent edit");
    }

    #[test]
    fn history_is_capped_and_drops_the_oldest_entry() {
        let mut h = EditHistory::new(3);
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
        let mut h = EditHistory::new(10);
        h.push(vec![edit(0, c4()), edit(1, c4()), edit(2, c4())]);
        let undone = cells(h.undo().expect("undo available"));
        assert_eq!(undone.len(), 3, "a block edit is one undo step");
        assert!(!h.can_undo());
    }

    #[test]
    fn clear_drops_both_stacks() {
        let mut h = EditHistory::new(10);
        h.push(vec![edit(0, c4())]);
        h.undo();
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }
}
