//! `examples/new-features.rtrk` is the demo of format 3. This plays it
//! through the engine and checks that each feature it shows is heard.

use rtrack_core::engine::{InstrumentInfo, TrackerEngine, TrackerEvent};
use rtrack_core::TrackerCoreBuilder;

const DRUMS: usize = 0;
const BASS: usize = 1;
const LEAD: usize = 2;

/// Every event of a full playback, tagged with the order position it fell in.
fn play() -> Vec<(usize, TrackerEvent)> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/new-features.rtrk");
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

fn notes(events: &[(usize, TrackerEvent)], order: usize, ch: usize) -> Vec<u8> {
    events
        .iter()
        .filter(|(o, _)| *o == order)
        .filter_map(|(_, e)| match e {
            TrackerEvent::NoteOn {
                channel, midi_note, ..
            } if *channel == ch => Some(*midi_note),
            _ => None,
        })
        .collect()
}

#[test]
fn the_aliased_bass_is_transposed_into_the_scale() {
    let events = play();
    // C D# G A#, up 2: D F A C. A is not in C minor and snaps down to G#.
    assert_eq!(notes(&events, 0, BASS), vec![24, 24, 27, 19, 22]);
    assert_eq!(notes(&events, 1, BASS), vec![26, 26, 29, 20, 24]);
}

#[test]
fn the_fixed_pitch_drums_ignore_the_alias_transpose() {
    let events = play();
    let first = notes(&events, 0, DRUMS);
    assert!(!first.is_empty());
    // CHA may drop a hit, so compare the hits both passes played.
    assert!(notes(&events, 1, DRUMS).iter().all(|n| first.contains(n)));
}

#[test]
fn the_short_bass_phrase_loops_under_the_long_pattern() {
    let events = play();
    // C-2 C-3 G-2 A#-2, eight rows, four times under 32.
    assert_eq!(notes(&events, 2, BASS), [24, 36, 31, 34].repeat(4));
}

#[test]
fn the_lead_table_steps_its_arpeggio() {
    let events = play();
    let bends: Vec<f64> = events
        .iter()
        .filter(|(o, _)| *o == 0)
        .filter_map(|(_, e)| match e {
            TrackerEvent::PitchBend {
                channel,
                semitone_offset,
            } if *channel == LEAD => Some(semitone_offset.round()),
            _ => None,
        })
        .collect();
    for step in [12.0, 7.0] {
        assert!(bends.contains(&step), "no {step} step in {bends:?}");
    }
}

#[test]
fn the_bass_cutoff_envelope_and_pad_tremolo_run() {
    let events = play();
    assert!(events.iter().any(|(_, e)| matches!(
        e,
        TrackerEvent::InstrumentFxMod { channel: BASS, cutoff_octaves, .. } if *cutoff_octaves > 2.9
    )));
    let pad_volumes = events
        .iter()
        .filter(|(o, e)| *o == 2 && matches!(e, TrackerEvent::VolumeChange { channel: LEAD, .. }))
        .count();
    assert!(pad_volumes > 10, "{pad_volumes} volume changes");
}

#[test]
fn the_midi_channel_sends_cc_and_program() {
    let events = play();
    assert!(events.iter().any(|(_, e)| *e
        == TrackerEvent::MidiCC {
            channel: 3,
            controller: 74,
            value: 0x40
        }));
    assert!(events.iter().any(|(_, e)| *e
        == TrackerEvent::ProgramChange {
            channel: 3,
            program: 39
        }));
}
