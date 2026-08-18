//! Save/load round-trip tests.
//!
//! These lock in two properties that were previously broken and are not
//! visible from unit tests of `Song` alone, because they involve state that
//! `TrackerCore` owns outside the song: pattern identity must survive a save,
//! and per-channel mixer state must be written to and read back from disk.

use std::path::{Path, PathBuf};

use rtrack_core::audio::effects::SendBusType;
use rtrack_core::core::{TrackerCore, TrackerCoreBuilder};
use rtrack_core::tracker::{Cell, Note, NoteValue, SongFile};
use rtrack_core::types::{ChannelType, LearnableParam, MidiCcMapping};

fn headless(channels: usize, rows: usize) -> TrackerCore {
    TrackerCoreBuilder::new()
        .song_size(channels, rows)
        .headless()
        .build()
}

/// A unique scratch path per test, so tests can run in parallel.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rtrack_persistence_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.rtrk"));
    let _ = std::fs::remove_file(&path);
    path
}

fn note(value: NoteValue, octave: u8) -> Cell {
    Cell {
        note: Some(Note::On { value, octave }),
        ..Cell::default()
    }
}

#[test]
fn saving_preserves_pattern_reuse() {
    let path = scratch("pattern_reuse");
    let mut core = headless(4, 16);
    // One pattern, played at three positions in the song.
    core.song.order = vec![0, 0, 0];
    core.song.sync_order_repeats();
    core.song.set_cell(0, 0, 0, note(NoteValue::C, 4));
    core.file_path = Some(path.clone());

    core.save().unwrap();

    // The in-memory song must not be restructured by saving.
    assert_eq!(core.song.patterns.len(), 1, "save mutated the live song");
    assert_eq!(core.song.order, vec![0, 0, 0]);

    let mut reloaded = headless(4, 16);
    reloaded.load_file(&path).unwrap();
    assert_eq!(
        reloaded.song.patterns.len(),
        1,
        "pattern reuse lost on disk"
    );
    assert_eq!(reloaded.song.order, vec![0, 0, 0]);

    // Editing the shared pattern is visible from every position.
    reloaded.song.set_cell(2, 1, 0, note(NoteValue::G, 5));
    for pos in 0..3 {
        assert_eq!(
            reloaded.song.cell_at(pos, 1, 0).note,
            Some(Note::On {
                value: NoteValue::G,
                octave: 5
            }),
            "edit at position 2 not shared with position {pos}"
        );
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn saving_does_not_grow_the_file_with_duplicate_patterns() {
    let path = scratch("no_pattern_duplication");
    let mut core = headless(4, 64);
    core.song.order = vec![0; 32];
    core.song.sync_order_repeats();
    core.file_path = Some(path.clone());
    core.save().unwrap();

    let file = SongFile::load(&path).unwrap();
    assert_eq!(
        file.song.patterns.len(),
        1,
        "32 order entries must not serialize 32 patterns"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn channel_configuration_survives_a_round_trip() {
    let path = scratch("channel_config");
    let mut core = headless(4, 16);

    core.channels[0].name = "kick".to_string();
    core.channels[0].channel_type = ChannelType::Sample;
    core.channels[0].muted = true;
    core.channels[0].volume = 0.42;
    core.channels[0].pan = -0.75;
    core.channels[0].midi_channel = 9;
    core.channels[0].default_instrument = Some(3);
    core.channels[0].effects_params.filter_enabled = true;
    core.channels[0].effects_params.filter_cutoff = 880.0;
    core.channels[0].effects_params.reverb_enabled = true;
    core.channels[1].channel_type = ChannelType::Synth;
    core.channels[1].name = "lead".to_string();

    core.send_bus_params[0].enabled = true;
    core.send_bus_params[0].label = "plate".to_string();
    core.send_bus_params[0].effect_type = SendBusType::Reverb;
    core.send_bus_params[0].reverb_size = 0.9;

    core.midi_cc_mappings.push(MidiCcMapping {
        cc: 74,
        channel: 0,
        param: LearnableParam::FilterCutoff,
    });

    core.file_path = Some(path.clone());
    core.save().unwrap();

    let mut reloaded = headless(4, 16);
    reloaded.load_file(&path).unwrap();

    assert_eq!(reloaded.channels[0].name, "kick");
    assert_eq!(reloaded.channels[0].channel_type, ChannelType::Sample);
    assert!(reloaded.channels[0].muted);
    assert_eq!(reloaded.channels[0].volume, 0.42);
    assert_eq!(reloaded.channels[0].pan, -0.75);
    assert_eq!(reloaded.channels[0].midi_channel, 9);
    assert_eq!(reloaded.channels[0].default_instrument, Some(3));
    assert!(reloaded.channels[0].effects_params.filter_enabled);
    assert_eq!(reloaded.channels[0].effects_params.filter_cutoff, 880.0);
    assert!(reloaded.channels[0].effects_params.reverb_enabled);
    assert_eq!(reloaded.channels[1].channel_type, ChannelType::Synth);
    assert_eq!(reloaded.channels[1].name, "lead");

    assert!(reloaded.send_bus_params[0].enabled);
    assert_eq!(reloaded.send_bus_params[0].label, "plate");
    assert_eq!(reloaded.send_bus_params[0].reverb_size, 0.9);

    assert_eq!(reloaded.midi_cc_mappings.len(), 1);
    assert_eq!(reloaded.midi_cc_mappings[0].cc, 74);
    assert_eq!(
        reloaded.midi_cc_mappings[0].param,
        LearnableParam::FilterCutoff
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn files_without_channel_state_still_load() {
    // Songs written before mixer state was persisted carry no channel data.
    // They must load and fall back to defaults rather than failing.
    let path = scratch("legacy_no_channels");
    let json = r#"{
        "title": "Legacy",
        "bpm": 140,
        "speed": 4,
        "patterns": [{"rows": 2, "channels": 2, "data": [
            [{"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null},
             {"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null}],
            [{"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null},
             {"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null}]]}],
        "order": [0],
        "channels": 2,
        "rows_per_pattern": 2
    }"#;
    std::fs::write(&path, json).unwrap();

    let mut core = headless(4, 16);
    let report = core.load_file(&path).unwrap();
    assert!(
        report.is_clean(),
        "well-formed file was altered: {report:?}"
    );
    assert_eq!(core.song.title, "Legacy");
    assert_eq!(core.song.bpm, 140);
    assert_eq!(core.channels.len(), 2);
    assert_eq!(core.channels[0].channel_type, ChannelType::Midi);
    assert_eq!(core.channels[1].midi_channel, 1);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn files_with_unknown_fields_still_load() {
    // Forward compatibility: a file written by a newer version must not be
    // rejected outright just because it carries fields we do not know.
    let path = scratch("unknown_fields");
    let json = r#"{
        "title": "FromTheFuture",
        "bpm": 120,
        "speed": 6,
        "patterns": [{"rows": 1, "channels": 1, "data": [
            [{"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null}]]}],
        "order": [0],
        "channels": 1,
        "rows_per_pattern": 1,
        "automation_lanes": [{"target": "cutoff"}],
        "phrases": [], "chains": [], "arrangement": []
    }"#;
    std::fs::write(&path, json).unwrap();

    let mut core = headless(4, 16);
    core.load_file(&path).unwrap();
    assert_eq!(core.song.title, "FromTheFuture");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn corrupt_files_are_repaired_rather_than_left_dangerous() {
    // An order entry pointing at a missing pattern used to load cleanly and
    // then panic the editor on the next redraw.
    let path = scratch("dangling_order");
    let json = r#"{
        "title": "Corrupt",
        "bpm": 120,
        "speed": 6,
        "patterns": [{"rows": 2, "channels": 1, "data": [
            [{"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null}],
            [{"note":null,"instrument":null,"volume":null,"effect":null,"effect_value":null}]]}],
        "order": [0, 99, 5],
        "channels": 1,
        "rows_per_pattern": 2
    }"#;
    std::fs::write(&path, json).unwrap();

    let mut core = headless(4, 16);
    let report = core.load_file(&path).unwrap();
    assert!(
        !report.repairs.is_empty(),
        "repair not reported: {report:?}"
    );
    assert_eq!(core.song.order, vec![0]);
    // Every order position now resolves to a real pattern.
    for pos in 0..core.song.order_len() {
        assert!(core.song.order[pos] < core.song.patterns.len());
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn bundled_example_songs_still_load() {
    // Guards against a format change silently orphaning the shipped examples.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rtrk") {
            continue;
        }
        let mut core = headless(4, 16);
        let report = core
            .load_file(&path)
            .unwrap_or_else(|e| panic!("{} failed to load: {e}", path.display()));
        assert!(
            report.is_clean(),
            "{} did not load cleanly: {report:?}",
            path.display()
        );
        assert!(!core.song.order.is_empty());
        checked += 1;
    }
    assert!(checked > 0, "no example songs found in {}", dir.display());
}

#[test]
fn empty_cells_cost_far_less_than_full_ones() {
    // A pattern is mostly empty; writing five explicit nulls per cell used to
    // make the file size proportional to the size of the grid rather than to
    // the amount of music in it.
    let sparse = scratch("sparse");
    let mut core = headless(8, 64);
    core.song.set_cell(0, 0, 0, note(NoteValue::C, 4));
    core.file_path = Some(sparse.clone());
    core.save().unwrap();
    let sparse_bytes = std::fs::metadata(&sparse).unwrap().len();

    let dense = scratch("dense");
    let mut core = headless(8, 64);
    for row in 0..64 {
        for ch in 0..8 {
            core.song.set_cell(
                0,
                row,
                ch,
                Cell {
                    note: Some(Note::On {
                        value: NoteValue::C,
                        octave: 4,
                    }),
                    instrument: Some(1),
                    volume: Some(64),
                    effect: Some(1),
                    effect_value: Some(32),
                },
            );
        }
    }
    core.file_path = Some(dense.clone());
    core.save().unwrap();
    let dense_bytes = std::fs::metadata(&dense).unwrap().len();

    assert!(
        dense_bytes > sparse_bytes * 3,
        "an empty grid ({sparse_bytes} bytes) should be far cheaper than a \
         full one ({dense_bytes} bytes); if they are close, empty cells are \
         still being written out in full"
    );

    let _ = std::fs::remove_file(&sparse);
    let _ = std::fs::remove_file(&dense);
}

#[test]
fn a_sparse_song_round_trips_exactly() {
    let path = scratch("sparse_roundtrip");
    let mut core = headless(8, 64);
    core.song.set_cell(0, 0, 0, note(NoteValue::C, 4));
    core.song.set_cell(
        0,
        17,
        3,
        Cell {
            volume: Some(0x40),
            effect: Some(4),
            effect_value: Some(0x82),
            ..Cell::default()
        },
    );
    core.file_path = Some(path.clone());
    core.save().unwrap();

    let mut reloaded = headless(8, 64);
    reloaded.load_file(&path).unwrap();
    assert_eq!(
        reloaded.song.cell_at(0, 0, 0).note,
        Some(Note::On {
            value: NoteValue::C,
            octave: 4
        })
    );
    let cell = reloaded.song.cell_at(0, 17, 3);
    assert_eq!(cell.volume, Some(0x40));
    assert_eq!(cell.effect, Some(4));
    assert_eq!(cell.effect_value, Some(0x82));
    assert!(reloaded.song.cell_at(0, 1, 0).is_empty());
    // Geometry survives even though most cells wrote nothing.
    assert_eq!(reloaded.song.patterns[0].rows, 64);
    assert_eq!(reloaded.song.patterns[0].channels, 8);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn saved_files_carry_the_format_version() {
    let path = scratch("format_version");
    let mut core = headless(2, 8);
    core.file_path = Some(path.clone());
    core.save().unwrap();

    let file = SongFile::load(&path).unwrap();
    assert_eq!(file.version, rtrack_core::tracker::FORMAT_VERSION);
    assert!(!file.is_from_newer_version());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn files_predating_the_version_field_load_as_version_zero() {
    let path = scratch("legacy_version");
    let json = r#"{
        "title": "NoVersion", "bpm": 120, "speed": 6,
        "patterns": [{"rows": 1, "channels": 1, "data": [[{}]]}],
        "order": [0], "channels": 1, "rows_per_pattern": 1
    }"#;
    std::fs::write(&path, json).unwrap();

    let file = SongFile::load(&path).unwrap();
    assert_eq!(file.version, 0);
    assert!(!file.is_from_newer_version());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_file_from_a_newer_version_loads_but_says_so() {
    let path = scratch("future_version");
    let json = r#"{
        "version": 9999,
        "title": "FromTheFuture", "bpm": 120, "speed": 6,
        "patterns": [{"rows": 1, "channels": 1, "data": [[{}]]}],
        "order": [0], "channels": 1, "rows_per_pattern": 1
    }"#;
    std::fs::write(&path, json).unwrap();

    let mut core = headless(4, 16);
    let report = core.load_file(&path).unwrap();
    assert!(
        report.from_newer_version,
        "newer-version file not flagged: {report:?}"
    );
    assert_eq!(core.song.title, "FromTheFuture");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn omitted_cell_fields_deserialize_as_empty() {
    // The compact form writes `{}` for an empty cell; loading it must not
    // fail on the missing keys.
    let path = scratch("compact_form");
    let json = r#"{
        "version": 1,
        "title": "Compact", "bpm": 120, "speed": 6,
        "patterns": [{"rows": 2, "channels": 2, "data": [
            [{"note":{"On":{"value":"C","octave":4}}}, {}],
            [{"volume":64}, {"effect":1,"effect_value":32}]]}],
        "order": [0], "channels": 2, "rows_per_pattern": 2
    }"#;
    std::fs::write(&path, json).unwrap();

    let mut core = headless(2, 2);
    core.load_file(&path).unwrap();
    assert!(core.song.cell_at(0, 0, 0).note.is_some());
    assert!(core.song.cell_at(0, 0, 1).is_empty());
    assert_eq!(core.song.cell_at(0, 1, 0).volume, Some(64));
    assert_eq!(core.song.cell_at(0, 1, 1).effect, Some(1));
    assert_eq!(core.song.cell_at(0, 1, 1).effect_value, Some(32));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_failed_save_leaves_no_temp_file_behind() {
    // Saving into a directory that does not exist must fail cleanly rather
    // than littering a `.rtrack_save_*.tmp` next to the user's songs.
    //
    // Uses its own directory: the shared scratch dir has other tests writing
    // to it, and a temp file legitimately exists there mid-save.
    let dir = std::env::temp_dir().join("rtrack_failed_save_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("no_such_subdir").join("song.rtrk");

    let mut core = headless(2, 8);
    core.file_path = Some(missing);
    assert!(
        core.save().is_err(),
        "save into a missing directory succeeded"
    );

    let strays: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".rtrack_save_"))
        .collect();
    assert!(
        strays.is_empty(),
        "left {} temp file(s) behind",
        strays.len()
    );

    // And the same for a save that fails after the temp file exists: make the
    // destination a directory, so the write succeeds but the rename cannot.
    let blocked = dir.join("blocked.rtrk");
    std::fs::create_dir_all(&blocked).unwrap();
    let mut core = headless(2, 8);
    core.file_path = Some(blocked);
    let _ = core.save();
    let strays: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".rtrack_save_"))
        .collect();
    assert!(
        strays.is_empty(),
        "a failed rename left {} temp file(s) behind",
        strays.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_over_an_existing_song_replaces_it_completely() {
    // The temp-then-rename path must not merge with, or truncate onto, the
    // previous contents.
    let path = scratch("overwrite");
    let mut core = headless(2, 8);
    core.song.title = "A song with a rather long title".to_string();
    core.file_path = Some(path.clone());
    core.save().unwrap();
    let long_len = std::fs::metadata(&path).unwrap().len();

    core.song.title = "Short".to_string();
    core.save().unwrap();
    let short_len = std::fs::metadata(&path).unwrap().len();
    assert!(
        short_len < long_len,
        "file did not shrink: stale bytes left?"
    );

    let mut reloaded = headless(2, 8);
    reloaded.load_file(&path).unwrap();
    assert_eq!(reloaded.song.title, "Short");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_oversized_sample_file_is_refused_rather_than_loaded() {
    // Guards against a mistyped path or a corrupt header turning into a
    // multi-gigabyte allocation. The check is on file size, so the content
    // does not need to be real audio.
    use rtrack_core::constants::MAX_SAMPLE_FILE_BYTES;
    use rtrack_core::sample::SampleBank;

    let dir = std::env::temp_dir().join("rtrack_persistence_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("huge.wav");
    let file = std::fs::File::create(&path).unwrap();
    // Sparse file: sets the length without writing the bytes.
    file.set_len(MAX_SAMPLE_FILE_BYTES + 1).unwrap();
    drop(file);

    let mut bank = SampleBank::new();
    let err = bank
        .load(0, &path)
        .expect_err("oversized file was accepted");
    assert!(
        err.to_string().contains("sample limit"),
        "unhelpful error: {err}"
    );

    let _ = std::fs::remove_file(&path);
}
