//! Old `.rtrk` files convert to typed instruments and named effects.
//!
//! The fixtures are version 0 and 1 files; both convert in the reader
//! (`docs/dev/g4-g8-format.md`).

use std::path::Path;

use rtrack_core::tracker::SongFile;
use rtrack_core::{InstrumentKind, MidiOutput};

fn load(name: &str) -> SongFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy")
        .join(name);
    SongFile::load(&path).expect("fixture loads")
}

/// Channel 0's first effect column, as text, and its instrument column, row
/// by row, as loaded.
fn loaded_column(file: &SongFile) -> Vec<(Option<String>, Option<u8>)> {
    let pattern = &file.song.patterns[0];
    (0..pattern.rows)
        .map(|row| {
            let cell = pattern.get(row, 0);
            (cell.fx[0].map(|fx| fx.to_string()), cell.instrument)
        })
        .collect()
}

fn expect(rows: &[(Option<&str>, Option<u8>)]) -> Vec<(Option<String>, Option<u8>)> {
    rows.iter()
        .map(|(fx, inst)| (fx.map(String::from), *inst))
        .collect()
}

#[test]
fn version_0_effects_convert_on_load() {
    let file = load("v0.rtrk");
    assert_eq!(file.version, 0);
    assert_eq!(
        loaded_column(&file),
        expect(&[(Some("PVB44"), Some(0)), (Some("TPO80"), None)])
    );
}

#[test]
fn version_1_effects_convert_on_load() {
    let file = load("v1.rtrk");
    assert_eq!(file.version, 1);
    assert_eq!(
        loaded_column(&file),
        expect(&[
            (Some("PVB44"), Some(0)),
            (Some("TIC03"), None),
            (Some("TPO80"), None),
            // The controller leaves the instrument column for the command.
            (Some("C0740"), None),
            // `A` was never assigned. `F00` did nothing, as `TIC00` does.
            (None, None),
            (Some("TIC00"), None),
            (Some("C007F"), None),
            (Some("OFS00"), None),
        ])
    );
    assert!(file.song.patterns[0]
        .get(0, 1)
        .fx
        .iter()
        .all(Option::is_none));
}

#[test]
fn converted_effects_save_in_the_new_form() {
    let file = load("v1.rtrk");
    let json = serde_json::to_string(&file.song.patterns[0].get(3, 0)).unwrap();
    assert_eq!(
        json,
        r#"{"note":{"On":{"value":"D","octave":4}},"fx":["C0740"]}"#
    );
}

#[test]
fn version_1_instruments_convert_with_repairs() {
    let file = load("v1.rtrk");
    let converted: Vec<(usize, InstrumentKind, Vec<String>)> = file
        .instruments
        .iter()
        .map(|entry| {
            let (inst, repairs) = entry.def.to_instrument(entry.slot);
            (entry.slot, inst.kind, repairs)
        })
        .collect();

    let (_, kind, repairs) = &converted[0];
    assert!(matches!(kind, InstrumentKind::Sample { slot: 0, .. }));
    assert_eq!(repairs.len(), 1, "unused synth params: {repairs:?}");

    let (_, kind, repairs) = &converted[1];
    assert!(matches!(kind, InstrumentKind::Synth(_)));
    assert_eq!(repairs.len(), 1, "unsent MIDI program: {repairs:?}");

    let (_, kind, repairs) = &converted[2];
    assert!(matches!(
        kind,
        InstrumentKind::Midi {
            program: 40,
            output: MidiOutput::Builtin,
            ..
        }
    ));
    assert_eq!(repairs.len(), 1, "sound change from preset 2: {repairs:?}");

    let (_, kind, repairs) = &converted[3];
    assert!(matches!(kind, InstrumentKind::Midi { program: 3, .. }));
    assert!(repairs.is_empty());
}

#[test]
fn a_midi_track_loses_the_default_it_never_used() {
    // Only `Synth` and `Sample` tracks filled in their default instrument.
    // Every track does now, so a `Midi` track's is dropped on load.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/v1.rtrk");
    let mut core = rtrack_core::TrackerCoreBuilder::new().headless().build();
    core.load_file(&path).unwrap();
    assert_eq!(
        core.channels[0].default_instrument,
        Some(0),
        "the Sample track"
    );
    assert_eq!(core.channels[1].default_instrument, None, "the Midi track");
}

#[test]
fn loading_converts_instruments_once_and_reports_what_changed() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/v1.rtrk");
    let mut core = rtrack_core::TrackerCoreBuilder::new().headless().build();
    let report = core.load_file(&path).unwrap();
    let notes: Vec<_> = report
        .repairs
        .iter()
        .filter(|r| r.starts_with("instrument"))
        .collect();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert_eq!(core.instruments[0].sample_index(), Some(0));
    assert!(core.instruments[1].synth_params().is_some());
    assert_eq!(core.instruments[2].program(), Some(40));
    assert!(core.instruments[4].is_default(4));

    // Saved as version 2, the instruments load back unchanged and need no
    // further conversion.
    let dir = std::env::temp_dir().join("rtrack_legacy_resave");
    let _ = std::fs::create_dir_all(&dir);
    let saved = dir.join("v1-resaved.rtrk");
    core.file_path = Some(saved.clone());
    core.save().unwrap();
    let before = core.instruments.clone();

    let mut reloaded = rtrack_core::TrackerCoreBuilder::new().headless().build();
    let report = reloaded.load_file(&saved).unwrap();
    assert!(
        !report.repairs.iter().any(|r| r.starts_with("instrument")),
        "{:?}",
        report.repairs
    );
    assert_eq!(reloaded.instruments, before);
    let _ = std::fs::remove_file(&saved);
}
