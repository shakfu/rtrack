//! Repository maintenance tasks.
//!
//! Run with `cargo xtask <command>`. This is a maintainer tool, not part of
//! the shipped application, and it is deliberately not a test: generating a
//! file that is committed to the repository is something you ask for, not
//! something that happens as a side effect of `cargo test`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rtrack_core::core::TrackerCoreBuilder;
use rtrack_core::tracker::{Cell, Note, NoteValue};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, flags) = args
        .split_first()
        .map(|(c, f)| (c.as_str(), f))
        .unwrap_or(("", &[]));
    let check_only = flags.iter().any(|f| f == "--check");

    match command {
        "regen-examples" => regen_examples(check_only),
        "" | "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!(
        "cargo xtask <command>

Commands:
  regen-examples [--check]   Rebuild the generated example songs.
                             --check verifies the committed files are current
                             without writing anything, for use in CI."
    );
}

fn repo_root() -> PathBuf {
    // The xtask crate sits one level below the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

fn regen_examples(check_only: bool) -> Result<()> {
    let root = repo_root();
    let generated = build_sliced_amen(&root)?;
    let target = root.join("examples/sliced-amen.rtrk");

    let existing = std::fs::read_to_string(&target).ok();
    if existing.as_deref() == Some(generated.as_str()) {
        println!("up to date: {}", target.display());
        return Ok(());
    }

    if check_only {
        bail!(
            "{} is out of date; run `cargo xtask regen-examples`",
            target.display()
        );
    }

    std::fs::write(&target, &generated)
        .with_context(|| format!("failed to write {}", target.display()))?;
    println!("regenerated: {}", target.display());
    Ok(())
}

/// Build `sliced-amen.rtrk` the way a user would: load the break, slice it,
/// and place one note per slice.
///
/// Going through `slice_sample` rather than computing the slice boundaries
/// here means this doubles as a check that the slicing feature still produces
/// something that survives being saved.
fn build_sliced_amen(root: &Path) -> Result<String> {
    const SLICES: usize = 8;
    const ROWS: usize = 32;

    let amen = root.join("examples/data/amen.wav");
    if !amen.exists() {
        bail!("missing fixture: {}", amen.display());
    }

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, ROWS)
        .headless()
        .build();
    core.load_sample(0, &amen)
        .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", amen.display()))?;

    let made = core
        .slice_sample(0, SLICES, 0.5, false)
        .map_err(|e| anyhow::anyhow!("slicing failed: {e}"))?;
    if made != SLICES {
        bail!("expected {SLICES} slices, got {made}");
    }

    // 170 BPM at speed 3: one slice every 4 rows, the classic amen tempo.
    core.song.title = "Sliced Amen".to_string();
    core.song.bpm = 170;
    core.song.speed = 3;
    for i in 0..SLICES {
        core.song.set_cell(
            0,
            i * 4,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                instrument: Some(i as u8),
                volume: Some(127),
                ..Cell::default()
            },
        );
    }

    // Serialize against the real destination so the sample paths come out
    // relative to it, then hand back the text rather than writing it, so
    // `--check` can compare without touching the tree.
    let target = root.join("examples/sliced-amen.rtrk");
    let song_file = core.build_song_file(&target);
    song_file
        .to_json()
        .context("failed to serialize the generated song")
}
