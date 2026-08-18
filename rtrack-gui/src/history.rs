use rtrack_core::tracker::Cell;

#[derive(Clone)]
pub struct CellEdit {
    pub pattern_idx: usize,
    pub row: usize,
    pub channel: usize,
    pub old_cell: Cell,
    pub new_cell: Cell,
}

pub struct EditHistory {
    undo_stack: Vec<Vec<CellEdit>>,
    redo_stack: Vec<Vec<CellEdit>>,
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
        self.redo_stack.clear();
        self.undo_stack.push(edits);
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Pop the most recent edit group from the undo stack.
    /// Returns the edits so the caller can apply old_cell values.
    pub fn undo(&mut self) -> Option<Vec<CellEdit>> {
        let edits = self.undo_stack.pop()?;
        self.redo_stack.push(edits.clone());
        Some(edits)
    }

    /// Pop the most recent edit group from the redo stack.
    /// Returns the edits so the caller can apply new_cell values.
    pub fn redo(&mut self) -> Option<Vec<CellEdit>> {
        let edits = self.redo_stack.pop()?;
        self.undo_stack.push(edits.clone());
        Some(edits)
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

        let undone = h.undo().expect("undo available");
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0].new_cell.note, c4());
        assert!(!h.can_undo());
        assert!(h.can_redo());

        let redone = h.redo().expect("redo available");
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
        while let Some(edits) = h.undo() {
            seen.push(edits[0].row);
        }
        assert_eq!(seen, vec![4, 3, 2]);
    }

    #[test]
    fn multi_cell_groups_undo_as_one_step() {
        let mut h = EditHistory::new(10);
        h.push(vec![edit(0, c4()), edit(1, c4()), edit(2, c4())]);
        let undone = h.undo().expect("undo available");
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
