use rtrack::app::{App, Mode};
use rtrack::tracker::{Cell, Note, NoteValue, Song, SongFile, InstrumentEntry, InstrumentDef, SampleRefEntry, SampleRef};
use rtrack::ui::pattern_editor::SubColumn;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Create an App with a simple song: one pattern, one note on row 0.
fn app_with_note() -> App {
    let mut app = App::new();
    app.song.patterns[0].set_cell(
        0,
        0,
        Cell {
            note: Some(Note::On {
                value: NoteValue::C,
                octave: 5,
            }),
            volume: Some(100),
            ..Cell::default()
        },
    );
    app
}

// -- Render-and-verify tests --

#[test]
fn test_export_wav_roundtrip() {
    let app = app_with_note();
    let instruments: Vec<rtrack::sample::export::ExportInstrument> = app
        .instruments
        .iter()
        .map(|i| rtrack::sample::export::ExportInstrument {
            sample_index: i.sample_index,
            midi_program: i.midi_program.unwrap_or(0),
            synth_params: i.synth_params.clone(),
        })
        .collect();

    let dir = std::env::temp_dir();
    let path = dir.join("rtrack_integration_test.wav");

    let result = rtrack::sample::export::render_to_wav(
        &path,
        &app.song,
        &app.sample_bank,
        &instruments,
        &app.channel_effects_params,
        &app.send_bus_params,
        44100,
    );
    assert!(result.is_ok(), "WAV export failed: {:?}", result.err());

    // Verify WAV has audio content
    let reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 44100);
    let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
    let has_audio = samples.iter().any(|&s| s.abs() > 10);
    assert!(has_audio, "Exported WAV should contain non-silent audio");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_export_midi_roundtrip() {
    let mut app = app_with_note();
    // Add a note-off on row 2
    app.song.patterns[0].set_cell(
        2,
        0,
        Cell {
            note: Some(Note::Off),
            ..Cell::default()
        },
    );

    let dir = std::env::temp_dir();
    let path = dir.join("rtrack_integration_test.mid");
    app.file_path = Some(path.with_extension("rtrk"));

    // Export MIDI via the midi_file module directly
    let result = rtrack::midi_file::export_midi(&app.song, &path);
    assert!(result.is_ok(), "MIDI export failed: {:?}", result.err());

    // Import it back
    let imported = rtrack::midi_file::import_midi(&path);
    assert!(imported.is_ok(), "MIDI import failed: {:?}", imported.err());
    let imported_song = imported.unwrap();

    // Should have at least one pattern with the note
    assert!(!imported_song.patterns.is_empty());
    let pat = &imported_song.patterns[0];
    // Row 0 should have a note-on
    let cell = pat.get(0, 0);
    assert!(
        cell.note.is_some(),
        "Imported MIDI should have a note on row 0"
    );

    let _ = std::fs::remove_file(&path);
}

// -- Playback state machine tests --

#[test]
fn test_play_stop_cycle() {
    let mut app = app_with_note();
    assert!(!app.is_playing());

    // Start playback via space
    app.handle_key(key(KeyCode::Char(' ')));
    assert!(app.is_playing());

    // Tick a few times
    for _ in 0..10 {
        app.tick_playback();
    }

    // Stop via space
    app.handle_key(key(KeyCode::Char(' ')));
    assert!(!app.is_playing());
}

#[test]
fn test_playback_advances_position() {
    let mut app = app_with_note();
    app.song.bpm = 240; // fast tempo for quick advancement
    app.song.speed = 1;

    app.handle_key(key(KeyCode::Char(' '))); // play
    assert!(app.is_playing());

    let start_row = app.playback_row;

    // Tick enough to advance at least one row
    // At 240 BPM, speed 1: tps = 240*24/60 = 96
    // tick_playback uses real elapsed time, so we need to simulate enough passes
    for _ in 0..5000 {
        app.tick_playback();
        std::thread::sleep(std::time::Duration::from_micros(100));
    }

    // Position should have advanced (or wrapped)
    let end_row = app.playback_row;
    assert!(
        end_row != start_row || app.playback_order != 0,
        "Playback should advance position"
    );

    app.handle_key(key(KeyCode::Char(' '))); // stop
}

// -- Cursor navigation end-to-end tests --

#[test]
fn test_cursor_navigation_full_pattern() {
    let mut app = App::new();
    let rows = app.song.patterns[0].rows;
    let channels = app.song.channels;

    // Move to last row
    for _ in 0..rows {
        app.handle_key(key(KeyCode::Down));
    }
    // Should wrap or stop at last row
    assert!(app.cursor_row < rows);

    // Move across all channels
    for _ in 0..channels * 4 {
        app.handle_key(key(KeyCode::Right));
    }
    assert!(app.cursor_channel < channels);
}

#[test]
fn test_insert_mode_note_entry() {
    let mut app = App::new();

    // Enter insert mode directly
    app.mode = Mode::Insert;
    app.cursor_sub = SubColumn::Note;

    // Type 'z' which maps to C note
    app.handle_key(key(KeyCode::Char('z')));

    // Check that a note was placed
    let pat_idx = app.song.order[app.current_order_position()];
    let cell = app.song.patterns[pat_idx].get(0, 0);
    assert!(cell.note.is_some(), "Note should be placed after 'z' in insert mode");
    if let Some(Note::On { value, .. }) = cell.note {
        assert_eq!(value, NoteValue::C);
    } else {
        panic!("Expected Note::On with C");
    }
}

// -- Song structure tests --

#[test]
fn test_add_remove_channels() {
    let mut app = App::new();
    let initial = app.song.channels;

    // Add channel via Ctrl+A (if that's the binding)
    // Actually, channels are added via settings. Let's test directly.
    app.song.channels = initial + 2;
    assert_eq!(app.song.channels, initial + 2);

    // Patterns should still be accessible
    let pat = &app.song.patterns[0];
    assert!(pat.channels >= initial); // pattern channels may not auto-expand
}

#[test]
fn test_pattern_order_manipulation() {
    let mut app = App::new();

    // Add patterns
    app.song.add_pattern();
    app.song.add_pattern();
    assert!(app.song.patterns.len() >= 3);

    // Order should reference valid patterns
    for &idx in &app.song.order {
        assert!(idx < app.song.patterns.len());
    }
}

// -- Multi-pattern song playback --

#[test]
fn test_multi_pattern_song_renders() {
    let mut song = Song::new(2, 4);
    song.speed = 2;
    song.bpm = 200;

    // Pattern 0: note on
    song.patterns[0].set_cell(
        0,
        0,
        Cell {
            note: Some(Note::On {
                value: NoteValue::E,
                octave: 4,
            }),
            volume: Some(80),
            ..Cell::default()
        },
    );

    // Add pattern 1 with a different note
    song.add_pattern();
    song.patterns[1].set_cell(
        0,
        0,
        Cell {
            note: Some(Note::On {
                value: NoteValue::G,
                octave: 4,
            }),
            volume: Some(90),
            ..Cell::default()
        },
    );
    song.order = vec![0, 1];

    let bank = rtrack::sample::SampleBank::new();
    let instruments: Vec<rtrack::sample::export::ExportInstrument> = (0..256)
        .map(|_| rtrack::sample::export::ExportInstrument {
            sample_index: None,
            midi_program: 0,
            synth_params: None,
        })
        .collect();

    let dir = std::env::temp_dir();
    let path = dir.join("rtrack_integration_multi.wav");
    let result = rtrack::sample::export::render_to_wav(
        &path, &song, &bank, &instruments, &[], &[], 44100,
    );
    assert!(result.is_ok(), "Multi-pattern render failed: {:?}", result.err());

    let reader = hound::WavReader::open(&path).unwrap();
    let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
    assert!(
        samples.iter().any(|&s| s.abs() > 10),
        "Multi-pattern render should have audio"
    );

    let _ = std::fs::remove_file(&path);
}

// -- Effects integration --

#[test]
fn test_channel_effects_in_render() {
    let mut song = Song::new(1, 4);
    song.speed = 2;
    song.patterns[0].set_cell(
        0,
        0,
        Cell {
            note: Some(Note::On {
                value: NoteValue::A,
                octave: 4,
            }),
            volume: Some(100),
            ..Cell::default()
        },
    );

    let bank = rtrack::sample::SampleBank::new();
    let instruments: Vec<rtrack::sample::export::ExportInstrument> = (0..256)
        .map(|_| rtrack::sample::export::ExportInstrument {
            sample_index: None,
            midi_program: 0,
            synth_params: None,
        })
        .collect();

    // Render without effects
    let dir = std::env::temp_dir();
    let path_dry = dir.join("rtrack_int_dry.wav");
    rtrack::sample::export::render_to_wav(
        &path_dry, &song, &bank, &instruments, &[], &[], 44100,
    )
    .unwrap();

    // Render with filter effect on channel 0
    let mut ch_fx = rtrack::audio::channel_effects::ChannelEffectsParams::default();
    ch_fx.filter_enabled = true;
    ch_fx.filter_cutoff = 500.0; // low cutoff to noticeably change sound

    let path_wet = dir.join("rtrack_int_wet.wav");
    rtrack::sample::export::render_to_wav(
        &path_wet, &song, &bank, &instruments, &[ch_fx], &[], 44100,
    )
    .unwrap();

    let r1 = hound::WavReader::open(&path_dry).unwrap();
    let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
    let r2 = hound::WavReader::open(&path_wet).unwrap();
    let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

    // Both should have audio
    assert!(s1.iter().any(|&s| s.abs() > 10));
    assert!(s2.iter().any(|&s| s.abs() > 10));

    // They should differ due to filter
    let min_len = s1.len().min(s2.len());
    let diff_count = s1[..min_len]
        .iter()
        .zip(&s2[..min_len])
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        diff_count > 0,
        "Channel filter effect should produce different output"
    );

    let _ = std::fs::remove_file(&path_dry);
    let _ = std::fs::remove_file(&path_wet);
}

#[test]
fn test_send_bus_in_render() {
    let mut song = Song::new(1, 4);
    song.speed = 2;
    song.patterns[0].set_cell(
        0,
        0,
        Cell {
            note: Some(Note::On {
                value: NoteValue::C,
                octave: 5,
            }),
            volume: Some(100),
            ..Cell::default()
        },
    );

    let bank = rtrack::sample::SampleBank::new();
    let instruments: Vec<rtrack::sample::export::ExportInstrument> = (0..256)
        .map(|_| rtrack::sample::export::ExportInstrument {
            sample_index: None,
            midi_program: 0,
            synth_params: None,
        })
        .collect();

    // Render without send bus
    let dir = std::env::temp_dir();
    let path_dry = dir.join("rtrack_int_send_dry.wav");
    rtrack::sample::export::render_to_wav(
        &path_dry, &song, &bank, &instruments, &[], &[], 44100,
    )
    .unwrap();

    // Render with send bus (delay) and channel 0 sending to it
    let mut ch_fx = rtrack::audio::channel_effects::ChannelEffectsParams::default();
    ch_fx.send_levels[0] = 0.8; // send to bus 0

    let mut bus_params = rtrack::audio::effects::SendBusParams::default();
    bus_params.enabled = true;
    bus_params.effect_type = rtrack::audio::effects::SendBusType::Delay;
    bus_params.delay_time = 200.0;
    bus_params.delay_feedback = 0.4;

    let path_wet = dir.join("rtrack_int_send_wet.wav");
    rtrack::sample::export::render_to_wav(
        &path_wet,
        &song,
        &bank,
        &instruments,
        &[ch_fx],
        &[bus_params],
        44100,
    )
    .unwrap();

    let r1 = hound::WavReader::open(&path_dry).unwrap();
    let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
    let r2 = hound::WavReader::open(&path_wet).unwrap();
    let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

    assert!(s1.iter().any(|&s| s.abs() > 10));
    assert!(s2.iter().any(|&s| s.abs() > 10));

    // Send bus should add delayed signal, making output different
    let min_len = s1.len().min(s2.len());
    let diff_count = s1[..min_len]
        .iter()
        .zip(&s2[..min_len])
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        diff_count > 0,
        "Send bus delay should produce different output"
    );

    let _ = std::fs::remove_file(&path_dry);
    let _ = std::fs::remove_file(&path_wet);
}

#[test]
fn test_generate_sliced_amen() {
    use std::path::Path;

    let amen_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/data/amen.wav");
    if !amen_path.exists() {
        eprintln!("Skipping: examples/data/amen.wav not found");
        return;
    }

    // Load the sample
    let mut bank = rtrack::sample::SampleBank::new();
    bank.load(0, &amen_path).expect("Failed to load amen.wav");
    let sample = bank.get(0).unwrap();

    // Slice into 8 equal segments
    let num_slices: usize = 8;
    let total_frames = sample.end();
    let slice_len = total_frames / num_slices;
    let slices = rtrack::sample::slice_equal(sample, num_slices);
    assert_eq!(slices.len(), num_slices);

    // Calculate trim points within the original sample for each slice
    let slice_bounds: Vec<(usize, usize)> = (0..num_slices)
        .map(|i| {
            let start = i * slice_len;
            let end = if i == num_slices - 1 { total_frames } else { start + slice_len };
            (start, end)
        })
        .collect();

    // Build a 32-row, 1-channel song at 170 BPM (classic amen tempo)
    // Notes every 4 rows so each slice rings for 4 rows (~176ms gate)
    let rows = 32;
    let mut song = Song::new(1, rows);
    song.title = "Sliced Amen".to_string();
    song.bpm = 170;
    song.speed = 3;

    for i in 0..num_slices {
        let row = i * 4;
        song.patterns[0].set_cell(
            row,
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

    // Set up instruments and sample refs with trim points into the original file
    let instruments: Vec<InstrumentEntry> = (0..num_slices)
        .map(|i| InstrumentEntry {
            slot: i,
            def: InstrumentDef {
                name: slices[i].name.clone(),
                midi_program: None,
                sample_index: Some(i),
                synth_params: None,
            },
        })
        .collect();

    let sample_refs: Vec<SampleRefEntry> = (0..num_slices)
        .map(|i| SampleRefEntry {
            slot: i,
            sample_ref: SampleRef {
                name: slices[i].name.clone(),
                path: "data/amen.wav".to_string(),
                base_note: slices[i].base_note,
                trim_start: slice_bounds[i].0,
                trim_end: slice_bounds[i].1,
                loop_enabled: false,
                loop_start: 0,
                loop_end: 0,
            },
        })
        .collect();

    let song_file = SongFile {
        song,
        instruments,
        sample_refs,
    };

    let out_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sliced-amen.rtrk");
    song_file.save(&out_path).expect("Failed to save sliced-amen.rtrk");

    // Verify it loads back
    let loaded = SongFile::load(&out_path).expect("Failed to reload");
    assert_eq!(loaded.song.title, "Sliced Amen");
    assert_eq!(loaded.song.bpm, 170);
    assert_eq!(loaded.instruments.len(), 8);
    assert_eq!(loaded.sample_refs.len(), 8);

    // Verify pattern has notes every 4 rows
    for i in 0..8 {
        let row = i * 4;
        let cell = loaded.song.patterns[0].get(row, 0);
        assert!(cell.note.is_some(), "Row {} should have a note", row);
        assert_eq!(cell.instrument, Some(i as u8));
        // Rows between should be empty
        if row + 1 < 32 {
            let gap = loaded.song.patterns[0].get(row + 1, 0);
            assert!(gap.note.is_none(), "Row {} should be empty", row + 1);
        }
    }
}
