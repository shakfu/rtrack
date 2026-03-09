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
