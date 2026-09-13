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

/// A generated example: its file name under `examples/`, and the function
/// that builds its serialized text for a given destination path.
type Example = (&'static str, fn(&Path, &Path) -> Result<String>);

const EXAMPLES: &[Example] = &[
    ("sliced-amen.rtrk", build_sliced_amen),
    ("sample-offset.rtrk", build_sample_offset),
];

fn regen_examples(check_only: bool) -> Result<()> {
    let root = repo_root();
    let mut stale = Vec::new();

    for &(name, build) in EXAMPLES {
        let target = root.join("examples").join(name);
        let generated = build(&root, &target)?;

        let existing = std::fs::read_to_string(&target).ok();
        if existing.as_deref() == Some(generated.as_str()) {
            println!("up to date: {}", target.display());
        } else if check_only {
            stale.push(target.display().to_string());
        } else {
            std::fs::write(&target, &generated)
                .with_context(|| format!("failed to write {}", target.display()))?;
            println!("regenerated: {}", target.display());
        }
    }

    if !stale.is_empty() {
        bail!(
            "out of date: {}; run `cargo xtask regen-examples`",
            stale.join(", ")
        );
    }
    Ok(())
}

/// Load the break into `slot`, or fail naming the missing fixture.
fn load_amen(core: &mut rtrack_core::core::TrackerCore, root: &Path, slot: usize) -> Result<()> {
    let amen = root.join("examples/data/amen.wav");
    if !amen.exists() {
        bail!("missing fixture: {}", amen.display());
    }
    core.load_sample(slot, &amen)
        .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", amen.display()))?;
    Ok(())
}

/// Build `sliced-amen.rtrk` the way a user would: load the break, slice it,
/// and place one note per slice.
///
/// Going through `slice_sample` rather than computing the slice boundaries
/// here means this doubles as a check that the slicing feature still produces
/// something that survives being saved.
fn build_sliced_amen(root: &Path, target: &Path) -> Result<String> {
    const SLICES: usize = 8;
    const ROWS: usize = 32;

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, ROWS)
        .headless()
        .build();
    load_amen(&mut core, root, 0)?;

    let made = core
        .slice_sample(
            0,
            SLICES,
            0.5,
            false,
            rtrack_core::sample::SliceRange::Source,
            // A fresh bank: nothing to overwrite.
            rtrack_core::sample::SliceOverwrite::Refuse,
        )
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
    let song_file = core.build_song_file(target);
    song_file
        .to_json()
        .context("failed to serialize the generated song")
}

/// Build `sample-offset.rtrk`: the `9xx` sample offset, three patterns long.
///
/// Slot 0 holds the whole break; slots 1-8 hold it cut into 8 slices. At
/// 175 BPM and speed 6, 4 rows are one beat, which is one eighth of the break.
///
/// 1. The whole break, retriggered each beat with `9 00`..`9 E0`. It should
///    sound like the break played straight.
/// 2. The same eight points in a new order, with half-beat repeats at the end.
/// 3. `9xx` on slices, where `9 80` means halfway through the slice, not the
///    file; `9xx` with a transposed note; and a `9xx` row with no note, which
///    must not retrigger anything.
fn build_sample_offset(root: &Path, target: &Path) -> Result<String> {
    use rtrack_core::constants::EFFECT_SAMPLE_OFFSET;
    const ROWS: usize = 32;

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, ROWS)
        .headless()
        .build();
    load_amen(&mut core, root, 0)?;
    load_amen(&mut core, root, 1)?;
    let made = core
        .slice_sample(
            1,
            8,
            0.5,
            false,
            rtrack_core::sample::SliceRange::Source,
            rtrack_core::sample::SliceOverwrite::Refuse,
        )
        .map_err(|e| anyhow::anyhow!("slicing failed: {e}"))?;
    if made != 8 {
        bail!("expected 8 slices, got {made}");
    }
    core.instruments[0].name = "amen (whole)".to_string();

    core.song.title = "Sample Offset (9xx)".to_string();
    core.song.bpm = 175;
    core.song.speed = 6;
    core.song.add_pattern();
    core.song.add_pattern();
    core.song.order = vec![0, 1, 2];

    // (row, octave, instrument, offset). `None` offset: a plain note.
    // Octave `None`: no note, only the effect.
    type Hit = (usize, Option<u8>, u8, Option<u8>);
    let patterns: [&[Hit]; 3] = [
        &[
            (0, Some(5), 0, Some(0x00)),
            (4, Some(5), 0, Some(0x20)),
            (8, Some(5), 0, Some(0x40)),
            (12, Some(5), 0, Some(0x60)),
            (16, Some(5), 0, Some(0x80)),
            (20, Some(5), 0, Some(0xA0)),
            (24, Some(5), 0, Some(0xC0)),
            (28, Some(5), 0, Some(0xE0)),
        ],
        &[
            (0, Some(5), 0, Some(0x00)),
            (4, Some(5), 0, Some(0x40)),
            (8, Some(5), 0, Some(0x20)),
            (12, Some(5), 0, Some(0x60)),
            (16, Some(5), 0, Some(0x00)),
            (20, Some(5), 0, Some(0xA0)),
            (24, Some(5), 0, Some(0xC0)),
            (26, Some(5), 0, Some(0xC0)),
            (28, Some(5), 0, Some(0xE0)),
            (30, Some(5), 0, Some(0xE0)),
        ],
        &[
            (0, Some(5), 1, None),
            (2, Some(5), 1, Some(0x80)),
            (4, Some(5), 3, None),
            (8, Some(5), 5, None),
            (10, Some(5), 5, Some(0x80)),
            (12, Some(5), 7, None),
            (16, Some(6), 0, Some(0x80)),
            (20, Some(4), 0, Some(0xC0)),
            (22, None, 0, Some(0x40)),
            (24, Some(5), 8, Some(0x00)),
            (26, Some(5), 8, Some(0x40)),
            (28, Some(5), 8, Some(0x80)),
            (30, Some(5), 8, Some(0xC0)),
        ],
    ];

    for (pattern, hits) in patterns.iter().enumerate() {
        for &(row, octave, instrument, offset) in *hits {
            let mut cell = Cell {
                effect: offset.map(|_| EFFECT_SAMPLE_OFFSET),
                effect_value: offset,
                ..Cell::default()
            };
            if let Some(octave) = octave {
                cell.note = Some(Note::On {
                    value: NoteValue::C,
                    octave,
                });
                cell.instrument = Some(instrument);
                cell.volume = Some(127);
            }
            core.song.set_cell(pattern, row, 0, cell);
        }
    }

    core.build_song_file(target)
        .to_json()
        .context("failed to serialize the generated song")
}
