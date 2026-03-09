#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubColumn {
    Note,
    Instrument,
    Volume,
    Effect,
}

impl SubColumn {
    pub fn next(self) -> Self {
        match self {
            SubColumn::Note => SubColumn::Instrument,
            SubColumn::Instrument => SubColumn::Volume,
            SubColumn::Volume => SubColumn::Effect,
            SubColumn::Effect => SubColumn::Note,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SubColumn::Note => SubColumn::Effect,
            SubColumn::Instrument => SubColumn::Note,
            SubColumn::Volume => SubColumn::Instrument,
            SubColumn::Effect => SubColumn::Volume,
        }
    }
}
