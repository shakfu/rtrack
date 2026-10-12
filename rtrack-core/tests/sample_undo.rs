//! Loading a sample is one undo step, whichever load path made it.

use rtrack_core::core::{TrackerCore, TrackerCoreBuilder};
use std::path::{Path, PathBuf};

/// A fresh directory holding the amen fixture under each of `names`.
fn workspace(test: &str, names: &[&str]) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/data/amen.wav");
    let dir = std::env::temp_dir().join(format!("rtrack_sample_undo_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in names {
        std::fs::copy(&src, dir.join(name)).expect("fixture missing");
    }
    dir
}

fn core() -> TrackerCore {
    TrackerCoreBuilder::new()
        .song_size(4, 16)
        .headless()
        .build()
}

fn sample_name(core: &TrackerCore, slot: usize) -> Option<String> {
    core.sample_bank.get(slot).map(|s| s.name.clone())
}

#[test]
fn loading_over_an_occupied_slot_is_undone_and_redone() {
    let dir = workspace("occupied", &["kick.wav", "snare.wav"]);
    let mut core = core();
    core.load_sample(2, &dir.join("kick.wav")).unwrap();
    core.load_sample(2, &dir.join("snare.wav")).unwrap();
    assert_eq!(sample_name(&core, 2).as_deref(), Some("snare"));

    assert!(core.undo());
    assert_eq!(sample_name(&core, 2).as_deref(), Some("kick"));
    assert_eq!(core.instruments[2].name, "kick");

    assert!(core.undo());
    assert_eq!(sample_name(&core, 2), None);
    assert_eq!(core.instruments[2].name, "", "undo kept the load's rename");

    assert!(core.redo());
    assert!(core.redo());
    assert_eq!(sample_name(&core, 2).as_deref(), Some("snare"));
}

#[test]
fn undoing_a_slot_load_restores_the_track_default_instrument() {
    let dir = workspace("default_instrument", &["kick.wav"]);
    let mut core = core();
    core.load_sample_into_slot(1, &dir.join("kick.wav"))
        .unwrap();
    assert_eq!(core.channels[1].default_instrument, Some(1));

    assert!(core.undo());
    assert_eq!(
        core.channels[1].default_instrument, None,
        "the track still points at the slot the undo emptied"
    );

    assert!(core.redo());
    assert_eq!(core.channels[1].default_instrument, Some(1));
}

#[test]
fn a_directory_load_and_its_tempo_are_one_step() {
    let dir = workspace("directory", &["0-kick.wav", "1-snare.wav"]);
    std::fs::write(dir.join("samples.json"), r#"{"bpm": 93}"#).unwrap();
    let mut core = core();
    let bpm = core.song.bpm;
    assert_ne!(bpm, 93);

    assert_eq!(core.load_sample_directory(&dir).unwrap(), 2);
    assert_eq!(core.song.bpm, 93);
    assert_eq!(core.history.undo_depth(), 1);

    assert!(core.undo());
    assert_eq!(core.song.bpm, bpm);
    assert_eq!(sample_name(&core, 0), None);
    assert_eq!(sample_name(&core, 1), None);
}

#[test]
fn an_instrument_load_targets_the_slot_it_plays() {
    let dir = workspace("instrument", &["kick.wav", "snare.wav"]);
    let mut core = core();
    core.load_sample(5, &dir.join("kick.wav")).unwrap();
    core.instruments[0].kind = rtrack_core::InstrumentKind::sample(5);

    assert_eq!(
        core.load_instrument_sample(0, &dir.join("snare.wav"))
            .unwrap(),
        5
    );
    assert_eq!(sample_name(&core, 5).as_deref(), Some("snare"));

    assert!(core.undo());
    assert_eq!(sample_name(&core, 5).as_deref(), Some("kick"));
}

#[test]
fn a_failed_load_records_nothing() {
    let dir = workspace("failed", &[]);
    let mut core = core();

    assert!(core.load_sample(0, &dir.join("missing.wav")).is_err());
    assert!(core
        .load_sample_into_slot(0, &dir.join("missing.wav"))
        .is_err());
    assert!(core
        .load_instrument_sample(0, &dir.join("missing.wav"))
        .is_err());
    assert!(core.load_sample_directory(&dir.join("missing")).is_err());

    assert!(!core.history.can_undo());
    assert!(!core.dirty);
}

#[test]
fn a_load_inside_a_pending_edit_joins_it() {
    let dir = workspace("pending", &["kick.wav"]);
    let mut core = core();
    let title = core.song.title.clone();

    core.begin_edit();
    core.song.title = "changed".to_string();
    core.load_sample(0, &dir.join("kick.wav")).unwrap();
    core.commit_edit();
    assert_eq!(core.history.undo_depth(), 1);

    assert!(core.undo());
    assert_eq!(core.song.title, title);
    assert_eq!(sample_name(&core, 0), None);
}

#[test]
fn a_load_clears_the_redo_branch() {
    let dir = workspace("redo", &["kick.wav", "snare.wav"]);
    let mut core = core();
    core.load_sample(0, &dir.join("kick.wav")).unwrap();
    assert!(core.undo());
    assert!(core.history.can_redo());

    core.load_sample(0, &dir.join("snare.wav")).unwrap();
    assert!(!core.history.can_redo());
}
