//! Error type for the operations `TrackerCore` exposes.
//!
//! These used to be `Result<String, String>`, where the `Ok` side carried a
//! human-readable status message. That put presentation decisions in the
//! library: the wording was fixed for every frontend, and a caller could not
//! tell a missing file from a malformed one without matching on prose.
//!
//! Operations now return structured values and this error type. Rendering
//! either of them into something a user reads is the frontend's job.

use std::fmt;
use std::path::PathBuf;

/// Result alias for fallible `TrackerCore` operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Something a `TrackerCore` operation could not do.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read, written, or parsed.
    File {
        path: PathBuf,
        /// What went wrong, from the underlying I/O or parser.
        source: anyhow::Error,
    },
    /// An audio file could not be loaded into a sample slot.
    Sample { slot: usize, source: anyhow::Error },
    /// A slot index outside the bank or instrument table.
    SlotOutOfRange { slot: usize, max: usize },
    /// Slicing needs a sample in the slot, and there was none.
    NoSampleInSlot { slot: usize },
    /// The sample is too short to divide into the requested number of slices.
    SampleTooShort { slot: usize },
    /// Slicing would need more slots than the bank has.
    NotEnoughSlots { needed: usize, from_slot: usize },
    /// Slicing would have written over instruments that are not its own
    /// output. Retry allowing overwrites to go ahead anyway.
    SlotsOccupied {
        /// First slot in the way.
        first: usize,
        /// How many of the slots to be written hold unrelated instruments.
        count: usize,
    },
    /// Rendering or encoding audio failed.
    Export { source: anyhow::Error },
    /// The operation needs a file path and the song has never been saved.
    NoFilePath,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::File { path, source } => {
                write!(f, "{}: {}", path.display(), source)
            }
            Error::Sample { slot, source } => {
                write!(f, "sample slot {slot:02X}: {source}")
            }
            Error::SlotOutOfRange { slot, max } => {
                write!(f, "slot {slot} is out of range (max {max})")
            }
            Error::NoSampleInSlot { slot } => {
                write!(f, "no sample loaded in slot {slot:02X}")
            }
            Error::SampleTooShort { slot } => {
                write!(f, "sample in slot {slot:02X} is too short to slice")
            }
            Error::NotEnoughSlots { needed, from_slot } => write!(
                f,
                "not enough sample slots: need {needed} starting at {from_slot:02X}"
            ),
            Error::SlotsOccupied { first, count } => {
                if *count == 1 {
                    write!(f, "slot {first:02X} holds another instrument")
                } else {
                    write!(f, "{count} slots from {first:02X} hold other instruments")
                }
            }
            Error::Export { source } => write!(f, "export failed: {source}"),
            Error::NoFilePath => write!(f, "song has no file path"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::File { source, .. } | Error::Sample { source, .. } => Some(source.as_ref()),
            Error::Export { source } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl Error {
    /// Wrap an underlying failure as a file error.
    pub fn file(path: impl Into<PathBuf>, source: impl Into<anyhow::Error>) -> Self {
        Error::File {
            path: path.into(),
            source: source.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_what_failed_and_where() {
        let e = Error::file("/songs/x.rtrk", anyhow::anyhow!("unexpected end of input"));
        let text = e.to_string();
        assert!(text.contains("/songs/x.rtrk"), "{text}");
        assert!(text.contains("unexpected end of input"), "{text}");
    }

    #[test]
    fn slot_errors_are_distinguishable_without_parsing_prose() {
        // The point of the enum: a caller can branch on the kind.
        let e = Error::NoSampleInSlot { slot: 3 };
        assert!(matches!(e, Error::NoSampleInSlot { slot: 3 }));
        let e = Error::NotEnoughSlots {
            needed: 40,
            from_slot: 250,
        };
        assert!(matches!(e, Error::NotEnoughSlots { needed: 40, .. }));
    }

    #[test]
    fn the_source_chain_is_preserved() {
        use std::error::Error as _;
        let e = Error::file("/tmp/a", anyhow::anyhow!("permission denied"));
        assert!(e.source().is_some(), "underlying cause was dropped");
    }
}
