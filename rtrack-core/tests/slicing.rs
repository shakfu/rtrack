//! Slicing a sample and saving the result.
//!
//! `.rtrk` files store samples as a path plus a span, not as audio. Slices
//! therefore have to *be* spans of their source: when `slice_equal` copied the
//! frames into detached samples instead, saving recorded no boundaries and
//! every slot reloaded as the whole file, so a sliced kit silently collapsed
//! into N copies of the same break.

use rtrack_core::core::TrackerCoreBuilder;
use rtrack_core::sample::{SliceOverwrite, SliceRange};
use std::path::PathBuf;

/// Copy the shared amen fixture into its own directory, so a song saved
/// alongside it resolves its relative sample reference on reload.
fn workspace_with_amen(name: &str) -> (PathBuf, PathBuf) {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/data/amen.wav");
    let dir = std::env::temp_dir().join(format!("rtrack_slicing_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("amen.wav");
    std::fs::copy(&src, &wav).expect("fixture missing");
    (dir, wav)
}

#[test]
fn slices_survive_a_save_and_reload() {
    let (dir, wav) = workspace_with_amen("roundtrip");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    let count = core
        .slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    assert_eq!(count, 8);

    // What the slices look like in memory.
    let before: Vec<(usize, usize, usize)> = (0..8)
        .map(|slot| {
            let s = core.sample_bank.get(slot).unwrap();
            (s.trim_start, s.end(), s.played_len())
        })
        .collect();
    // They tile the source: contiguous, non-empty, covering it exactly.
    assert_eq!(before[0].0, 0);
    for i in 0..8 {
        assert!(before[i].2 > 0, "slice {i} plays nothing");
        if i > 0 {
            assert_eq!(before[i].0, before[i - 1].1, "slice {i} is not contiguous");
        }
    }

    let song = dir.join("song.rtrk");
    core.file_path = Some(song.clone());
    core.save().unwrap();

    let mut reloaded = TrackerCoreBuilder::new().headless().build();
    let report = reloaded.load_file(&song).unwrap();
    assert!(
        report.missing_samples.is_empty(),
        "samples did not reload: {:?}",
        report.missing_samples
    );

    let after: Vec<(usize, usize, usize)> = (0..8)
        .map(|slot| {
            let s = reloaded
                .sample_bank
                .get(slot)
                .unwrap_or_else(|| panic!("slot {slot} came back empty"));
            (s.trim_start, s.end(), s.played_len())
        })
        .collect();

    assert_eq!(
        after, before,
        "slice boundaries changed across a save/load cycle"
    );
    // The specific regression: every slot must not become the whole file.
    let distinct: std::collections::HashSet<_> = after.iter().collect();
    assert_eq!(distinct.len(), 8, "slices collapsed into duplicates");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slices_share_one_buffer_rather_than_copying_it() {
    let (dir, wav) = workspace_with_amen("sharing");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    let source_frames = core.sample_bank.get(0).unwrap().len();
    core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();

    for slot in 0..8 {
        let s = core.sample_bank.get(slot).unwrap();
        assert_eq!(
            s.len(),
            source_frames,
            "slice {slot} holds its own copy of the audio"
        );
        assert!(s.played_len() < source_frames, "slice {slot} is untrimmed");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transient_slices_also_survive_a_save_and_reload() {
    let (dir, wav) = workspace_with_amen("transient");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    let count = core
        .slice_sample(0, 0, 0.5, true, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    assert!(count > 1, "transient detection found nothing to slice");

    let before: Vec<(usize, usize)> = (0..count)
        .map(|slot| {
            let s = core.sample_bank.get(slot).unwrap();
            (s.trim_start, s.end())
        })
        .collect();

    let song = dir.join("song.rtrk");
    core.file_path = Some(song.clone());
    core.save().unwrap();

    let mut reloaded = TrackerCoreBuilder::new().headless().build();
    reloaded.load_file(&song).unwrap();
    let after: Vec<(usize, usize)> = (0..count)
        .map(|slot| {
            let s = reloaded.sample_bank.get(slot).unwrap();
            (s.trim_start, s.end())
        })
        .collect();

    assert_eq!(after, before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn subdividing_a_slice_stays_within_its_bounds() {
    // `Span` treats a slice as the thing being divided, so subdividing one
    // nests inside it rather than restarting from the shared buffer.
    let (dir, wav) = workspace_with_amen("reslice");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();

    let second = core.sample_bank.get(1).unwrap();
    let (outer_start, outer_end) = (second.trim_start, second.end());
    assert!(outer_start > 0);

    core.slice_sample(1, 2, 0.5, false, SliceRange::Span, SliceOverwrite::Allow)
        .unwrap();
    for slot in 1..=2 {
        let s = core.sample_bank.get(slot).unwrap();
        assert!(
            s.trim_start >= outer_start && s.end() <= outer_end,
            "sub-slice {slot} ({}..{}) escaped its parent ({outer_start}..{outer_end})",
            s.trim_start,
            s.end()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reloaded_slices_still_share_one_buffer() {
    // Slices are stored as one path repeated with different spans. Decoding
    // that path once per slot would read the file N times and leave N copies
    // of the audio in memory -- exactly what spans exist to avoid.
    let (dir, wav) = workspace_with_amen("reload_sharing");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();

    let song = dir.join("song.rtrk");
    core.file_path = Some(song.clone());
    core.save().unwrap();

    let mut reloaded = TrackerCoreBuilder::new().headless().build();
    reloaded.load_file(&song).unwrap();

    let first = reloaded.sample_bank.get(0).unwrap().data.clone();
    for slot in 1..8 {
        let other = &reloaded.sample_bank.get(slot).unwrap().data;
        assert!(
            std::sync::Arc::ptr_eq(&first, other),
            "slot {slot} holds its own copy of the source audio after reload"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn spans_are_clamped_when_the_source_file_shrinks() {
    // The path in a .rtrk file can point at different audio by the time the
    // song is opened again. A span past the end of the new file would leave
    // the slot silent with nothing to explain it.
    let (dir, wav) = workspace_with_amen("shrunk");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    let song = dir.join("song.rtrk");
    core.file_path = Some(song.clone());
    core.save().unwrap();

    // Replace the source with a much shorter file.
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
    for i in 0..1000 {
        writer.write_sample((i as i16).wrapping_mul(17)).unwrap();
    }
    writer.finalize().unwrap();

    let mut reloaded = TrackerCoreBuilder::new().headless().build();
    reloaded.load_file(&song).unwrap();

    for slot in 0..4 {
        let s = reloaded.sample_bank.get(slot).unwrap();
        assert!(
            s.trim_start <= s.len() && s.end() <= s.len(),
            "slot {slot} span {}..{} is outside the {}-frame file",
            s.trim_start,
            s.end(),
            s.len()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn re_slicing_from_source_re_derives_from_the_whole_sample() {
    // What a slice-count control does: apply, change the count, apply again.
    // The second pass must divide the sample, not the first slice -- slicing
    // the span instead left the eight new slices covering an eighth of the
    // break, and every further change quartered what was left.
    let (dir, wav) = workspace_with_amen("reslice_source");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    let total = core.sample_bank.get(0).unwrap().len();

    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();

    let first = core.sample_bank.get(0).unwrap();
    let last = core.sample_bank.get(7).unwrap();
    assert_eq!(first.trim_start, 0, "slicing did not start at the sample");
    assert_eq!(last.end(), total, "slicing did not reach the end");
    for slot in 1..8 {
        let prev = core.sample_bank.get(slot - 1).unwrap().end();
        assert_eq!(
            core.sample_bank.get(slot).unwrap().trim_start,
            prev,
            "slice {slot} is not contiguous with the one before it"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slicing_from_source_is_idempotent() {
    // Applying the same count twice must not change anything, or a control
    // that re-applies on every redraw would walk the boundaries.
    let (dir, wav) = workspace_with_amen("reslice_idempotent");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();

    let spans = |c: &rtrack_core::core::TrackerCore| -> Vec<(usize, usize)> {
        (0..8)
            .map(|slot| {
                let s = c.sample_bank.get(slot).unwrap();
                (s.trim_start, s.end())
            })
            .collect()
    };

    core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    let once = spans(&core);
    core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    assert_eq!(
        once,
        spans(&core),
        "re-applying the same count moved the slices"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transient_slicing_from_source_also_re_derives() {
    let (dir, wav) = workspace_with_amen("reslice_transient_source");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    let total = core.sample_bank.get(0).unwrap().len();

    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();
    let count = core
        .slice_sample(0, 0, 0.5, true, SliceRange::Source, SliceOverwrite::Allow)
        .unwrap();

    assert_eq!(core.sample_bank.get(0).unwrap().trim_start, 0);
    assert_eq!(core.sample_bank.get(count - 1).unwrap().end(), total);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slicing_refuses_to_overwrite_unrelated_instruments() {
    // Slicing writes into consecutive slots and cannot be undone, so it must
    // not quietly eat an instrument it did not put there.
    let (dir, wav) = workspace_with_amen("occupied");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.instruments[3].name = "Bass".to_string();

    let err = core
        .slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Refuse)
        .expect_err("slicing over an instrument should have been refused");
    match err {
        rtrack_core::error::Error::SlotsOccupied { first, count } => {
            assert_eq!(first, 3);
            assert_eq!(count, 1);
        }
        other => panic!("wrong error: {other}"),
    }
    // Nothing was written.
    assert_eq!(core.instruments[3].name, "Bass");
    assert!(
        core.sample_bank.get(1).is_none(),
        "slot 1 was written anyway"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slicing_may_replace_its_own_slices() {
    // Re-cutting a kit from 4 pieces to 8 is the operation working, so the
    // guard has to stay out of its way.
    let (dir, wav) = workspace_with_amen("own_output");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Refuse)
        .unwrap();
    let again = core.slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Refuse);
    assert!(
        again.is_ok(),
        "re-slicing its own output was refused: {:?}",
        again.err().map(|e| e.to_string())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn allowing_overwrites_goes_ahead() {
    let (dir, wav) = workspace_with_amen("forced");

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.instruments[3].name = "Bass".to_string();

    let made = core
        .slice_sample(0, 8, 0.5, false, SliceRange::Source, SliceOverwrite::Allow)
        .expect("an allowed overwrite should go ahead");
    assert_eq!(made, 8);
    assert_eq!(core.instruments[3].name, "amen_S03");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn subdividing_refuses_to_eat_the_next_slice_of_another_sample() {
    // The neighbouring slots of a slice set are the same source, so
    // subdividing may replace them; a different sample next door may not.
    let (dir, wav) = workspace_with_amen("subdivide_guard");
    let other = dir.join("other.wav");
    std::fs::copy(&wav, &other).unwrap();

    let mut core = TrackerCoreBuilder::new()
        .song_size(1, 32)
        .headless()
        .build();
    core.load_sample(0, &wav).unwrap();
    core.slice_sample(0, 4, 0.5, false, SliceRange::Source, SliceOverwrite::Refuse)
        .unwrap();
    // A different file lands in slot 2, where subdividing slice 1 would spill.
    core.load_sample(2, &other).unwrap();

    let err = core
        .slice_sample(1, 2, 0.5, false, SliceRange::Span, SliceOverwrite::Refuse)
        .expect_err("subdividing over a different sample should have been refused");
    assert!(matches!(
        err,
        rtrack_core::error::Error::SlotsOccupied { first: 2, count: 1 }
    ));

    let _ = std::fs::remove_dir_all(&dir);
}
