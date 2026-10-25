//! The parsers read files, so each input is written to one, per process.

use std::path::PathBuf;

/// Write `data` to this process's probe file with extension `ext`.
pub fn probe(data: &[u8], ext: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rtrack-fuzz-{}.{ext}", std::process::id()));
    std::fs::write(&path, data).expect("write probe file");
    path
}
