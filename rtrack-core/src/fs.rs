//! Filesystem helpers shared by everything that writes user data.
//!
//! Saving a song, an autosave, and the recent-files list are all "replace
//! this file, or leave the old one alone" operations. Doing that correctly --
//! durably, and without being talked into writing somewhere else -- is the
//! same problem every time, so it lives here rather than being reimplemented
//! per caller.

use std::path::Path;

use anyhow::{Context, Result};

/// Write `bytes` to `path`, replacing any existing file atomically.
///
/// The data lands in a temp file in the destination directory, is flushed to
/// disk, and is then renamed over `path`. A rename is atomic, but only with
/// respect to *ordering*: without the flush, a crash can leave the directory
/// entry pointing at a file whose contents were never written, which is worse
/// than no save at all.
///
/// The temp file is created with an unpredictable name and `O_EXCL`
/// (`create_new`). A deterministic name -- the pid, say -- lets anyone who can
/// write to the song directory pre-create that path as a symlink, and an
/// ordinary `File::create` would then follow it and write the song through to
/// whatever it points at. `create_new` refuses to open an existing path at all,
/// symlink included, so the worst such an attempt can do is fail one save; the
/// random name means it cannot even do that reliably.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().and_then(|f| f.to_str()).unwrap_or("song");

    // A pre-created path can only lose a race, never win it, so a handful of
    // attempts is enough to get past one.
    let mut last_err = None;
    for _ in 0..8 {
        let temp_path = dir.join(temp_name_for(stem));
        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("Failed to create {}", temp_path.display())))
            }
        };

        // Leaving a stray .tmp behind on failure would litter the user's song
        // directory, so clean up on every error path.
        let write_and_sync = |mut file: std::fs::File| -> Result<()> {
            file.write_all(bytes)
                .with_context(|| format!("Failed to write {}", temp_path.display()))?;
            file.sync_all()
                .with_context(|| format!("Failed to flush {}", temp_path.display()))?;
            Ok(())
        };
        if let Err(e) = write_and_sync(file) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&temp_path, path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(anyhow::Error::new(e).context(format!(
                "Failed to rename {} -> {}",
                temp_path.display(),
                path.display()
            )));
        }

        // The rename itself is only durable once the directory entry is on
        // disk. Not every platform lets a directory be opened as a file, so
        // this is best effort: the save has already succeeded either way.
        if let Ok(dir_file) = std::fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
        return Ok(());
    }

    Err(
        anyhow::Error::new(last_err.expect("loop ran at least once"))
            .context(format!("Failed to create a temp file in {}", dir.display())),
    )
}

/// An unpredictable, hidden temp file name for a save into `stem`.
///
/// Nothing here is cryptographic. `create_new` is what makes the save safe;
/// the entropy only stops an attacker from parking a file on the name we are
/// about to use and blocking saves.
fn temp_name_for(stem: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Stack addresses vary between runs under ASLR, so this mixes in a value
    // an outside process cannot derive from the clock and the pid.
    let addr = &seq as *const u64 as u64;
    let token = nanos.rotate_left(17)
        ^ addr.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ seq.wrapping_mul(0xD1B5_4A32_D192_ED03)
        ^ (std::process::id() as u64);

    format!(".rtrack_save_{token:016x}_{stem}.tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_save_writes_the_file_and_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("song.rtrk");
        write_atomic(&path, b"hello").expect("save");
        assert_eq!(std::fs::read(&path).expect("read back"), b"hello");

        // Saving again replaces the contents, still leaving nothing extra.
        write_atomic(&path, b"goodbye").expect("resave");
        assert_eq!(std::fs::read(&path).expect("read back"), b"goodbye");

        let stray: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .filter(|n| n != "song.rtrk")
            .collect();
        assert!(stray.is_empty(), "temp files left behind: {stray:?}");
    }

    #[test]
    fn temp_names_do_not_repeat() {
        // Two saves in the same directory must not collide, and a name must
        // not be derivable from the last one.
        let a = temp_name_for("song.rtrk");
        let b = temp_name_for("song.rtrk");
        assert_ne!(a, b);
        assert!(
            a.starts_with(".rtrack_save_") && a.ends_with("_song.rtrk.tmp"),
            "{a}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_does_not_write_through_a_symlink_at_the_destination() {
        // The save replaces the link with a real file; whatever it pointed at
        // is left alone. Without the rename -- writing the destination in
        // place -- the song text would land in `outside`.
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"untouched").expect("seed");

        let path = dir.path().join("song.rtrk");
        std::os::unix::fs::symlink(&outside, &path).expect("symlink");

        write_atomic(&path, b"song data").expect("save");
        assert_eq!(std::fs::read(&outside).expect("read"), b"untouched");
        assert_eq!(std::fs::read(&path).expect("read"), b"song data");
        assert!(!std::fs::symlink_metadata(&path)
            .expect("stat")
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_refuses_a_pre_created_temp_path() {
        // Stand in for an attacker who guessed the temp name: `create_new`
        // must refuse the existing path rather than open it (and, for a
        // symlink, write through it).
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"untouched").expect("seed");
        let guessed = dir.path().join(temp_name_for("song.rtrk"));
        std::os::unix::fs::symlink(&outside, &guessed).expect("symlink");

        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&guessed);
        assert!(opened.is_err(), "create_new opened an existing symlink");
        assert_eq!(std::fs::read(&outside).expect("read"), b"untouched");
    }
}
