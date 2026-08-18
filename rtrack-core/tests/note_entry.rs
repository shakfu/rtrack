//! How note entry picks the instrument a previewed or written note sounds with.
//!
//! The case these guard is `examples/sliced-amen.rtrk`: a sliced sample, where
//! every slice is a separate instrument named in the pattern's instrument
//! column. Because the song predates persisted channel state it loads as a
//! plain Midi-typed track with no default instrument, so a note typed on an
//! empty row resolved to no instrument at all and fell through to the built-in
//! synth, while playback of the existing rows correctly played the slices.

use rtrack_core::core::TrackerCoreBuilder;

#[test]
fn every_row_of_the_sliced_amen_example_previews_a_sample() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/sliced-amen.rtrk");

    let mut core = TrackerCoreBuilder::new().headless().build();
    core.load_file(&path).expect("example failed to load");

    // Preconditions: this is the shape that used to break.
    assert_eq!(
        core.channels[0].default_instrument, None,
        "the track has no selected instrument to fall back on"
    );
    assert!(
        !core.sample_bank.loaded_slots().is_empty(),
        "the slices did not load, so this test would prove nothing"
    );

    // Every row -- including the empty ones between the hits -- must resolve
    // to an instrument backed by a loaded sample.
    let rows = core.song.rows_at(0);
    for row in 0..rows {
        let instrument = core
            .resolve_edit_instrument(0, row, 0)
            .unwrap_or_else(|| panic!("row {row} resolved to no instrument"));
        let sample_index = core.instruments[instrument as usize]
            .sample_index
            .unwrap_or_else(|| panic!("row {row} resolved to a non-sample instrument"));
        assert!(
            core.sample_bank.get(sample_index).is_some(),
            "row {row} resolved to sample slot {sample_index}, which is empty"
        );
    }
}

#[test]
fn a_one_shot_sample_preview_is_not_cut_off_by_the_timeout() {
    // An amen slice runs to roughly 340ms, well past the 250ms preview
    // timeout. One-shot samples end by themselves, so they are left to ring.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/sliced-amen.rtrk");

    let mut core = TrackerCoreBuilder::new().headless().build();
    core.load_file(&path).unwrap();

    core.preview_note_for_cell(0, 0, 0, 60, 100);
    let preview = core.preview_note.expect("no preview was registered");
    assert!(
        !preview.needs_note_off,
        "a one-shot sample would be truncated by an explicit note-off"
    );
}

#[test]
fn a_synth_preview_is_still_stopped_by_the_timeout() {
    // Sustaining sources have to be cut off, or a previewed note rings for ever.
    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 16)
        .headless()
        .build();
    core.preview_note_for_cell(0, 0, 0, 60, 100);
    let preview = core.preview_note.expect("no preview was registered");
    assert!(
        preview.needs_note_off,
        "a synth note must be stopped explicitly"
    );
}

#[test]
fn a_looping_sample_preview_is_still_stopped_by_the_timeout() {
    use rtrack_core::sample::{Sample, SampleBank};
    use std::sync::Arc;

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 16)
        .headless()
        .build();
    let mut bank = SampleBank::new();
    let sample = Sample {
        name: "loop".to_string(),
        data: vec![[0.0, 0.0]; 1000].into(),
        sample_rate: 44100.0,
        base_note: 60,
        trim_start: 0,
        trim_end: 0,
        loop_enabled: true,
        loop_start: 0,
        loop_end: 1000,
        source_path: None,
    };
    bank.samples[0] = Some(Arc::new(sample));
    core.sample_bank = Arc::new(bank);
    core.instruments[0].sample_index = Some(0);
    core.channels[0].channel_type = rtrack_core::types::ChannelType::Sample;
    core.channels[0].default_instrument = Some(0);

    core.preview_note_for_cell(0, 0, 0, 60, 100);
    let preview = core.preview_note.expect("no preview was registered");
    assert!(
        preview.needs_note_off,
        "a looping sample never ends on its own"
    );
}
