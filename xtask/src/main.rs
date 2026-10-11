//! Repository maintenance tasks.
//!
//! Run with `cargo xtask <command>`. This is a maintainer tool, not part of
//! the shipped application, and it is deliberately not a test: generating a
//! file that is committed to the repository is something you ask for, not
//! something that happens as a side effect of `cargo test`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rtrack_core::core::TrackerCoreBuilder;
use rtrack_core::tracker::{Cell, Fx, FxCmd, Note, NoteValue, FX_COLUMNS};

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
    ("new-features.rtrk", build_new_features),
    ("chords.rtrk", build_chords),
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
                fx: match offset {
                    Some(p) => Fx::only(FxCmd::Offset, p),
                    None => [None; FX_COLUMNS],
                },
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

/// One cell: an optional note, instrument, volume and both effect slots.
fn cell(
    note: Option<(NoteValue, u8)>,
    inst: Option<u8>,
    vol: Option<u8>,
    fx: [Option<Fx>; FX_COLUMNS],
) -> Cell {
    Cell {
        note: note.map(|(value, octave)| Note::On { value, octave }),
        instrument: inst,
        volume: vol,
        fx,
    }
}

fn fx1(cmd: FxCmd, p: u8) -> [Option<Fx>; FX_COLUMNS] {
    Fx::only(cmd, p)
}

fn fx2(a: (FxCmd, u8), b: (FxCmd, u8)) -> [Option<Fx>; FX_COLUMNS] {
    [Some(Fx::new(a.0, a.1)), Some(Fx::new(b.0, b.1))]
}

/// Build `new-features.rtrk`: one song using what format 3 added.
///
/// - Typed instruments: amen slices (Sample, fixed pitch), a bass, lead and
///   pad (Synth), and a MIDI instrument on channel 2 doubling the bass.
/// - The bass has its own filter and drive, with an envelope on the cutoff;
///   the lead runs table 00 and an LFO on pitch; the pad an LFO on volume.
/// - Named effects, two per cell: `VIB`+`SLD`, `TBL`+`CHA`, `CCN`+`CCV`.
/// - Song scale C minor and groove `7 5`.
/// - Pattern 01 aliases pattern 00's drums and bass 2 semitones up: the bass
///   snaps into C minor, the fixed-pitch drums do not move. Pattern 02 loops
///   pattern 03's 8-row bass phrase under 32 rows.
fn build_new_features(root: &Path, target: &Path) -> Result<String> {
    use rtrack_core::audio::synth::SynthParams;
    use rtrack_core::modulation::{LfoShape, ModDest, ModSlot, ModSource};
    use rtrack_core::theory::{Scale, ScaleSetting};
    use rtrack_core::tracker::{ColumnAlias, Pattern, Table, TableRow};
    use rtrack_core::{InstrumentKind, MidiOut};
    use FxCmd::*;
    use NoteValue::*;

    const DRUMS: usize = 0;
    const BASS: usize = 1;
    const LEAD: usize = 2;
    const MIDI: usize = 3;
    const BASS_I: u8 = 0x10;
    const LEAD_I: u8 = 0x11;
    const PAD_I: u8 = 0x12;
    const EXT_I: u8 = 0x13;

    let mut core = TrackerCoreBuilder::new()
        .song_size(4, 16)
        .headless()
        .build();

    // Drums: the break in eight slices, exempt from transposes.
    load_amen(&mut core, root, 0)?;
    let made = core
        .slice_sample(
            0,
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
    for slot in 0..8 {
        core.instruments[slot].fixed_pitch = true;
    }

    let synth = |patch: u8| InstrumentKind::Synth {
        params: SynthParams::from_patch(patch),
    };
    let bass = &mut core.instruments[BASS_I as usize];
    bass.name = "bass".into();
    bass.kind = synth(9);
    bass.fx.filter_enabled = true;
    bass.fx.filter_cutoff = 300.0;
    bass.fx.filter_resonance = 0.4;
    bass.fx.distortion_enabled = true;
    bass.fx.distortion_drive = 3.0;
    bass.mods[0] = Some(ModSlot {
        source: ModSource::Envelope {
            attack: 0,
            hold: 2,
            decay: 10,
        },
        dest: ModDest::Cutoff,
        amount: 48,
    });

    let lead = &mut core.instruments[LEAD_I as usize];
    lead.name = "lead".into();
    lead.kind = synth(12);
    lead.table = Some(0);
    lead.mods[0] = Some(ModSlot {
        source: ModSource::Lfo {
            shape: LfoShape::Sine,
            period: 12,
        },
        dest: ModDest::Pitch,
        amount: 2,
    });

    let pad = &mut core.instruments[PAD_I as usize];
    pad.name = "pad".into();
    pad.kind = synth(11);
    pad.mods[0] = Some(ModSlot {
        source: ModSource::Lfo {
            shape: LfoShape::Triangle,
            period: 48,
        },
        dest: ModDest::Volume,
        amount: -30,
    });

    let ext = &mut core.instruments[EXT_I as usize];
    ext.name = "ext bass".into();
    ext.kind = InstrumentKind::Midi(MidiOut {
        channel: 1,
        program: Some(38),
    });

    // Table 00: an octave-and-fifth arpeggio that fades. Table 01: a
    // gate that chops a drum hit.
    let row = |transpose, volume| TableRow {
        transpose,
        volume,
        fx: None,
    };
    let mut arp = Table {
        length: 4,
        tic: 3,
        ..Table::default()
    };
    arp.rows[0] = row(Some(0), Some(0x7F));
    arp.rows[1] = row(Some(12), Some(0x60));
    arp.rows[2] = row(Some(7), None);
    arp.rows[3] = row(Some(12), Some(0x30));
    let mut gate = Table {
        length: 2,
        tic: 2,
        ..Table::default()
    };
    gate.rows[0] = row(None, Some(0x7F));
    gate.rows[1] = row(None, Some(0x00));

    let song = &mut core.song;
    song.title = "New Features".to_string();
    song.bpm = 124;
    song.groove = vec![7, 5];
    song.scale = Some(ScaleSetting::new(C, Scale::Minor));
    song.tables = vec![arp, gate];
    song.patterns.push(Pattern::new(16, 4));
    song.patterns.push(Pattern::new(32, 4));
    song.patterns.push(Pattern::new(8, 4));
    song.order = vec![0, 1, 2, 1];
    song.sync_order_repeats();

    let none = [None; FX_COLUMNS];
    let set = |song: &mut rtrack_core::tracker::Song, p: usize, r: usize, ch: usize, c: Cell| {
        song.edit_cell(p, r, ch, |dst| *dst = c);
    };

    // Pattern 00
    for (r, slice, fx) in [
        (0, 0, none),
        (4, 2, none),
        (8, 4, none),
        (10, 1, fx1(Delay, 2)),
        (12, 6, fx1(Offset, 0x40)),
        (14, 3, fx2((Table, 1), (Chance, 0xA0))),
    ] {
        set(
            song,
            0,
            r,
            DRUMS,
            cell(Some((C, 5)), Some(slice), Some(0x7F), fx),
        );
    }
    for (r, note, vol, fx) in [
        (0, (C, 2), Some(0x70), none),
        (3, (C, 2), Some(0x50), none),
        (6, (Ds, 2), None, none),
        (8, (G, 1), None, fx2((Vibrato, 0x44), (SlideDown, 0x02))),
        (12, (As, 1), None, none),
    ] {
        set(song, 0, r, BASS, cell(Some(note), Some(BASS_I), vol, fx));
    }
    for (r, note, fx) in [
        (0, (C, 5), none),
        (8, (Ds, 5), none),
        (12, (G, 4), fx1(Arp, 0x37)),
    ] {
        set(
            song,
            0,
            r,
            LEAD,
            cell(Some(note), Some(LEAD_I), Some(0x60), fx),
        );
    }
    set(
        song,
        0,
        0,
        MIDI,
        cell(
            Some((C, 3)),
            Some(EXT_I),
            None,
            fx2((CcNumber, 74), (CcValue, 0x40)),
        ),
    );
    set(
        song,
        0,
        8,
        MIDI,
        cell(Some((G, 2)), Some(EXT_I), None, fx1(Program, 39)),
    );

    // Pattern 01: drums and bass reused two semitones up, a new lead line
    let up2 = ColumnAlias {
        source: 0,
        transpose: 2,
    };
    for ch in [DRUMS, BASS, MIDI] {
        song.set_alias(1, ch, up2)
            .map_err(|e| anyhow::anyhow!("alias failed: {e}"))?;
    }
    for (r, note, fx) in [
        (0, (As, 4), none),
        (4, (C, 5), fx1(SlideUp, 0x08)),
        (8, (G, 4), none),
        (12, (F, 4), fx1(Delay, 3)),
    ] {
        set(
            song,
            1,
            r,
            LEAD,
            cell(Some(note), Some(LEAD_I), Some(0x60), fx),
        );
    }

    // Pattern 03: an 8-row bass phrase, played only through pattern 02
    for (r, note) in [(0, (C, 2)), (2, (C, 3)), (4, (G, 2)), (6, (As, 2))] {
        set(
            song,
            3,
            r,
            BASS,
            cell(Some(note), Some(BASS_I), Some(0x60), none),
        );
    }

    // Pattern 02: 32 rows over a 16-row drum alias and the 8-row phrase,
    // each looping; a pad on top, and a tempo change.
    song.set_alias(
        2,
        DRUMS,
        ColumnAlias {
            source: 0,
            transpose: 0,
        },
    )
    .map_err(|e| anyhow::anyhow!("alias failed: {e}"))?;
    song.set_alias(
        2,
        BASS,
        ColumnAlias {
            source: 3,
            transpose: 0,
        },
    )
    .map_err(|e| anyhow::anyhow!("alias failed: {e}"))?;
    set(
        song,
        2,
        0,
        LEAD,
        cell(Some((C, 4)), Some(PAD_I), Some(0x50), fx1(Tempo, 0x82)),
    );
    set(
        song,
        2,
        16,
        LEAD,
        cell(Some((Gs, 3)), Some(PAD_I), Some(0x50), none),
    );

    for (ch, name) in ["drums", "bass", "lead/pad", "midi"].iter().enumerate() {
        core.channels[ch].name = name.to_string();
    }

    core.build_song_file(target)
        .to_json()
        .context("failed to serialize the generated song")
}

/// Build `chords.rtrk`: `CHD` over a I-vi-IV-V progression in C major.
///
/// Pattern 00 is the C bar. Patterns 01-03 alias it -3, +5 and +7. The
/// scale snaps each aliased note and each `CHD47` chord tone, so the one
/// major-triad shape plays C major, A minor, F major and G major.
///
/// - Channel 0: a pad holding the chord for the bar.
/// - Channel 1: off-beat stabs, `CHD47` with `KIL03` for short chords.
/// - Channel 2: the bass root.
/// - Channel 3: a MIDI instrument doubling the pad at full velocity. Silent
///   in an offline render; live, it plays on MIDI channel 1.
fn build_chords(_root: &Path, target: &Path) -> Result<String> {
    use rtrack_core::audio::synth::SynthParams;
    use rtrack_core::theory::{Scale, ScaleSetting};
    use rtrack_core::tracker::{ColumnAlias, Pattern};
    use rtrack_core::{InstrumentKind, MidiOut};
    use FxCmd::*;
    use NoteValue::*;

    const PAD: u8 = 0x01;
    const STAB: u8 = 0x02;
    const BASS: u8 = 0x03;
    const KEYS: u8 = 0x04;

    let mut core = TrackerCoreBuilder::new()
        .song_size(4, 16)
        .headless()
        .build();
    for (slot, name, patch) in [(PAD, "pad", 11u8), (STAB, "stab", 10), (BASS, "bass", 9)] {
        let inst = &mut core.instruments[slot as usize];
        inst.name = name.into();
        inst.kind = InstrumentKind::Synth {
            params: SynthParams::from_patch(patch),
        };
    }
    let keys = &mut core.instruments[KEYS as usize];
    keys.name = "ext keys".into();
    keys.kind = InstrumentKind::Midi(MidiOut {
        channel: 0,
        program: Some(0),
    });

    let song = &mut core.song;
    song.title = "Chords".to_string();
    song.bpm = 110;
    song.speed = 6;
    song.scale = Some(ScaleSetting::new(C, Scale::Major));

    let none = [None; FX_COLUMNS];
    let triad = fx1(Chord, 0x47);
    let mut put = |r: usize, ch: usize, note: (NoteValue, u8), inst: u8, vol: u8, fx| {
        song.edit_cell(0, r, ch, |c| {
            *c = cell(Some(note), Some(inst), Some(vol), fx)
        });
    };
    put(0, 0, (C, 4), PAD, 0x60, triad);
    for r in [2, 6, 10, 14] {
        put(r, 1, (C, 5), STAB, 0x50, fx2((Chord, 0x47), (Kill, 3)));
    }
    put(0, 2, (C, 2), BASS, 0x70, none);
    put(8, 2, (C, 3), BASS, 0x50, none);
    put(0, 3, (C, 4), KEYS, 0x60, triad);

    // vi, IV, V: the same bar, moved and snapped into C major.
    for transpose in [-3, 5, 7] {
        let p = song.patterns.len();
        song.patterns.push(Pattern::new(16, 4));
        for ch in 0..4 {
            song.set_alias(
                p,
                ch,
                ColumnAlias {
                    source: 0,
                    transpose,
                },
            )
            .map_err(|e| anyhow::anyhow!("alias failed: {e}"))?;
        }
    }
    song.order = vec![0, 1, 2, 3];
    song.sync_order_repeats();

    for (ch, name) in ["pad", "stabs", "bass", "midi keys"].iter().enumerate() {
        core.channels[ch].name = name.to_string();
    }

    core.build_song_file(target)
        .to_json()
        .context("failed to serialize the generated song")
}
