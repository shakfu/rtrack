//! `examples/chords.rtrk` shows `CHD`. This plays it through the engine and
//! checks the chords it claims.

use rtrack_core::engine::{InstrumentInfo, TrackerEngine, TrackerEvent};
use rtrack_core::TrackerCoreBuilder;

const PAD: usize = 0;
const STABS: usize = 1;

fn play() -> Vec<(usize, TrackerEvent)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/chords.rtrk");
    let mut core = TrackerCoreBuilder::new().headless().build();
    let report = core.load_file(&path).expect("example loads");
    assert!(report.is_clean(), "{report:?}");

    let mut engine = TrackerEngine::new(&core.song, false);
    engine.set_instruments(core.instruments.iter().map(InstrumentInfo::from).collect());
    let mut events = Vec::new();
    while !engine.finished {
        engine.process_tick(&core.song);
        let order = engine.sounding_order;
        events.extend(engine.drain_events().into_iter().map(|e| (order, e)));
    }
    events
}

/// The pad's chord in each bar: root, then chord tones.
fn pad_chords(events: &[(usize, TrackerEvent)]) -> Vec<Vec<u8>> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            TrackerEvent::NoteOn {
                channel: PAD,
                midi_note,
                chord,
                ..
            } => Some(
                std::iter::once(*midi_note)
                    .chain(chord.iter().flatten().copied())
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn one_chord_shape_plays_i_vi_iv_v_in_c_major() {
    let events = play();
    assert_eq!(
        pad_chords(&events),
        vec![
            vec![48, 52, 55], // C major
            vec![45, 48, 52], // A minor: A + 4 is C#, snapped to C
            vec![53, 57, 60], // F major
            vec![55, 59, 62], // G major
        ]
    );
}

#[test]
fn the_stabs_are_cut_short_by_kil() {
    let events = play();
    let count = |want: fn(&TrackerEvent) -> bool| events.iter().filter(|(_, e)| want(e)).count();
    let stabs = count(|e| {
        matches!(
            e,
            TrackerEvent::NoteOn {
                channel: STABS,
                chord: [Some(_), Some(_)],
                ..
            }
        )
    });
    let cuts = count(|e| matches!(e, TrackerEvent::NoteOff { channel: STABS }));
    assert_eq!((stabs, cuts), (16, 16), "four stabs a bar, each cut");
}
