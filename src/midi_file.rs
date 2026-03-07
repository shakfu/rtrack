//! Standard MIDI file (.mid) export and import.
//!
//! Implements SMF format 1 reading and writing without external MIDI crates.
//! All multi-byte integers are big-endian per the MIDI file specification.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::tracker::{Note, NoteValue, Pattern, Song};

/// Events collected during MIDI import
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ImportEvent {
    NoteOn { tick: u32, note: u8, velocity: u8 },
    NoteOff { tick: u32, note: u8 },
    CC { tick: u32, controller: u8, value: u8 },
    ProgramChange { tick: u32, program: u8 },
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TICKS_PER_BEAT: u16 = 480;
const DEFAULT_ROWS_PER_PATTERN: usize = 64;
const DEFAULT_VELOCITY: u8 = 0x7F;

// ---------------------------------------------------------------------------
// Variable-length quantity helpers
// ---------------------------------------------------------------------------

/// Encode a value as a MIDI variable-length quantity and append to `buf`.
fn write_variable_length(buf: &mut Vec<u8>, mut value: u32) {
    // Build bytes in reverse order, then push in correct order.
    let mut tmp = [0u8; 4];
    let mut count = 0usize;
    loop {
        tmp[count] = (value & 0x7F) as u8;
        count += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    // Write most-significant first; all but the last byte have bit 7 set.
    for i in (0..count).rev() {
        let byte = if i == 0 {
            tmp[i] // last byte: bit 7 clear
        } else {
            tmp[i] | 0x80
        };
        buf.push(byte);
    }
}

/// Read a MIDI variable-length quantity starting at `data[*pos]`.
/// Advances `*pos` past the bytes consumed.
fn read_variable_length(data: &[u8], pos: &mut usize) -> Result<u32> {
    let mut value: u32 = 0;
    for _ in 0..4 {
        if *pos >= data.len() {
            bail!("Unexpected end of data while reading variable-length quantity");
        }
        let byte = data[*pos];
        *pos += 1;
        value = (value << 7) | (byte & 0x7F) as u32;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("Variable-length quantity exceeds 4 bytes");
}

// ---------------------------------------------------------------------------
// Chunk writing helpers
// ---------------------------------------------------------------------------

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

/// Write a MIDI track event: delta-time + event bytes.
fn write_track_event(track: &mut Vec<u8>, delta: u32, event_bytes: &[u8]) {
    write_variable_length(track, delta);
    track.extend_from_slice(event_bytes);
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export a `Song` to a standard MIDI file (format 1).
///
/// Layout:
/// - Track 0: tempo meta-event only.
/// - Tracks 1..=channels: one per tracker channel, containing note and CC data.
pub fn export_midi(song: &Song, path: &Path) -> Result<()> {
    let num_tracks = song.channels as u16 + 1; // tempo track + channel tracks
    let ticks_per_row = ticks_per_row(song.speed);

    // ---- Header chunk ----
    let mut file_buf: Vec<u8> = Vec::new();
    {
        let mut hdr_data = Vec::with_capacity(6);
        hdr_data.extend_from_slice(&1u16.to_be_bytes()); // format 1
        hdr_data.extend_from_slice(&num_tracks.to_be_bytes());
        hdr_data.extend_from_slice(&TICKS_PER_BEAT.to_be_bytes());
        write_chunk(&mut file_buf, b"MThd", &hdr_data);
    }

    // ---- Track 0: tempo ----
    {
        let mut trk = Vec::new();
        let us_per_beat = bpm_to_microseconds(song.bpm);
        // Meta event: FF 51 03 tt tt tt
        let tempo_bytes = [
            0xFF,
            0x51,
            0x03,
            ((us_per_beat >> 16) & 0xFF) as u8,
            ((us_per_beat >> 8) & 0xFF) as u8,
            (us_per_beat & 0xFF) as u8,
        ];
        write_track_event(&mut trk, 0, &tempo_bytes);
        // End of track
        write_track_event(&mut trk, 0, &[0xFF, 0x2F, 0x00]);
        write_chunk(&mut file_buf, b"MTrk", &trk);
    }

    // ---- Tracks 1..N: one per channel ----
    for ch in 0..song.channels {
        let mut trk = Vec::new();
        let midi_ch = (ch & 0x0F) as u8;
        let mut active_note: Option<u8> = None; // currently sounding MIDI note
        let mut accumulated_delta: u32 = 0;

        for &order_idx in &song.order {
            let pattern = match song.patterns.get(order_idx) {
                Some(p) => p,
                None => continue,
            };
            for row in 0..pattern.rows {
                let cell = pattern.get(row, ch);

                // -- Handle effects first (program change, CC) --
                if let Some(eff) = cell.effect {
                    match eff {
                        0x0E => {
                            // Program change
                            let prog = cell.effect_value.unwrap_or(0);
                            write_track_event(
                                &mut trk,
                                accumulated_delta,
                                &[0xC0 | midi_ch, prog & 0x7F],
                            );
                            accumulated_delta = 0;
                        }
                        0x0C => {
                            // MIDI CC: controller = instrument column, value = effect_value
                            let controller = cell.instrument.unwrap_or(0) & 0x7F;
                            let value = cell.effect_value.unwrap_or(0) & 0x7F;
                            write_track_event(
                                &mut trk,
                                accumulated_delta,
                                &[0xB0 | midi_ch, controller, value],
                            );
                            accumulated_delta = 0;
                        }
                        _ => {} // Other effects: not mapped to MIDI
                    }
                }

                // -- Handle notes --
                if let Some(note) = &cell.note {
                    match note {
                        Note::On { .. } => {
                            if let Some(midi_note) = note.to_midi_note() {
                                // Kill previous note on this channel if active
                                if let Some(prev) = active_note.take() {
                                    write_track_event(
                                        &mut trk,
                                        accumulated_delta,
                                        &[0x80 | midi_ch, prev, 0x00],
                                    );
                                    accumulated_delta = 0;
                                }
                                let velocity = cell.volume.unwrap_or(DEFAULT_VELOCITY).min(0x7F);
                                write_track_event(
                                    &mut trk,
                                    accumulated_delta,
                                    &[0x90 | midi_ch, midi_note, velocity],
                                );
                                accumulated_delta = 0;
                                active_note = Some(midi_note);
                            }
                        }
                        Note::Off => {
                            if let Some(prev) = active_note.take() {
                                write_track_event(
                                    &mut trk,
                                    accumulated_delta,
                                    &[0x80 | midi_ch, prev, 0x00],
                                );
                                accumulated_delta = 0;
                            }
                        }
                    }
                }

                accumulated_delta += ticks_per_row;
            }
        }

        // Kill any lingering note
        if let Some(prev) = active_note.take() {
            write_track_event(&mut trk, accumulated_delta, &[0x80 | midi_ch, prev, 0x00]);
            accumulated_delta = 0;
        }
        let _ = accumulated_delta; // suppress unused warning

        // End of track
        write_track_event(&mut trk, 0, &[0xFF, 0x2F, 0x00]);
        write_chunk(&mut file_buf, b"MTrk", &trk);
    }

    std::fs::write(path, &file_buf)
        .with_context(|| format!("Failed to write MIDI file: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a standard MIDI file and convert it to a `Song`.
///
/// Reads format 0 or 1 files. For format 0, all channels found in a single
/// track are split across tracker channels. For format 1, each track beyond
/// the tempo track maps to a tracker channel.
pub fn import_midi(path: &Path) -> Result<Song> {
    let data = std::fs::read(path)
        .with_context(|| format!("Failed to read MIDI file: {}", path.display()))?;

    if data.len() < 14 {
        bail!("File is too small to be a valid MIDI file");
    }

    // ---- Parse header ----
    let mut pos: usize = 0;
    if &data[pos..pos + 4] != b"MThd" {
        bail!("Missing MThd header -- not a MIDI file");
    }
    pos += 4;
    let hdr_len = read_u32_be(&data, &mut pos)?;
    if hdr_len < 6 {
        bail!("MThd chunk too short");
    }
    let _format = read_u16_be(&data, &mut pos)?;
    let num_tracks = read_u16_be(&data, &mut pos)? as usize;
    let division = read_u16_be(&data, &mut pos)?;

    // Skip any extra header bytes
    let hdr_extra = hdr_len as usize - 6;
    if hdr_extra > 0 {
        pos += hdr_extra;
    }

    // Ticks per beat from division (we only handle metric timing, not SMPTE)
    if division & 0x8000 != 0 {
        bail!("SMPTE timing division is not supported");
    }
    let file_tpb = division as u32;

    // ---- Parse tracks ----
    let mut bpm: u16 = 120;
    // Collected events per MIDI channel
    let mut channel_events: Vec<Vec<ImportEvent>> = vec![Vec::new(); 16];

    for _t in 0..num_tracks {
        if pos + 8 > data.len() {
            break;
        }
        if &data[pos..pos + 4] != b"MTrk" {
            bail!("Expected MTrk chunk at offset {}", pos);
        }
        pos += 4;
        let trk_len = read_u32_be(&data, &mut pos)? as usize;
        let trk_end = pos + trk_len;
        if trk_end > data.len() {
            bail!("Track chunk extends beyond file");
        }

        let mut abs_tick: u32 = 0;
        let mut running_status: u8 = 0;

        while pos < trk_end {
            let delta = read_variable_length(&data, &mut pos)?;
            abs_tick += delta;

            if pos >= trk_end {
                break;
            }

            let mut status = data[pos];
            if status & 0x80 != 0 {
                running_status = status;
                pos += 1;
            } else {
                // Running status
                status = running_status;
            }

            let high = status & 0xF0;
            let ch = (status & 0x0F) as usize;

            match high {
                0x80 => {
                    // Note off
                    let note = read_byte(&data, &mut pos)?;
                    let _vel = read_byte(&data, &mut pos)?;
                    channel_events[ch].push(ImportEvent::NoteOff { tick: abs_tick, note: note & 0x7F });
                }
                0x90 => {
                    // Note on (velocity 0 = note off)
                    let note = read_byte(&data, &mut pos)?;
                    let vel = read_byte(&data, &mut pos)?;
                    if vel == 0 {
                        channel_events[ch].push(ImportEvent::NoteOff { tick: abs_tick, note: note & 0x7F });
                    } else {
                        channel_events[ch].push(ImportEvent::NoteOn { tick: abs_tick, note: note & 0x7F, velocity: vel });
                    }
                }
                0xA0 => {
                    // Poly aftertouch -- skip 2 bytes
                    pos += 2;
                }
                0xB0 => {
                    // Control change
                    let controller = read_byte(&data, &mut pos)?;
                    let value = read_byte(&data, &mut pos)?;
                    channel_events[ch].push(ImportEvent::CC { tick: abs_tick, controller: controller & 0x7F, value: value & 0x7F });
                }
                0xC0 => {
                    // Program change
                    let program = read_byte(&data, &mut pos)?;
                    channel_events[ch].push(ImportEvent::ProgramChange { tick: abs_tick, program: program & 0x7F });
                }
                0xD0 => {
                    // Channel aftertouch -- skip 1 byte
                    pos += 1;
                }
                0xE0 => {
                    // Pitch bend -- convert to CC-like event for import
                    let lsb = read_byte(&data, &mut pos)?;
                    let msb = read_byte(&data, &mut pos)?;
                    let bend = ((msb as u16 & 0x7F) << 7) | (lsb as u16 & 0x7F);
                    // Convert 14-bit pitch bend to 7-bit value for effect column
                    let value = (bend >> 7) as u8;
                    if bend != 0x2000 { // Skip center (no bend)
                        // Use controller 128 as sentinel for pitch bend (not a valid MIDI CC)
                        channel_events[ch].push(ImportEvent::CC { tick: abs_tick, controller: 128, value });
                    }
                }
                0xF0 => {
                    if status == 0xFF {
                        // Meta event
                        let meta_type = read_byte(&data, &mut pos)?;
                        let meta_len = read_variable_length(&data, &mut pos)? as usize;
                        if meta_type == 0x51 && meta_len == 3 && pos + 3 <= data.len() {
                            // Tempo
                            let us = ((data[pos] as u32) << 16)
                                | ((data[pos + 1] as u32) << 8)
                                | (data[pos + 2] as u32);
                            if us > 0 {
                                bpm = (60_000_000 / us) as u16;
                                if bpm == 0 {
                                    bpm = 1;
                                }
                            }
                        }
                        pos += meta_len;
                    } else if status == 0xF0 || status == 0xF7 {
                        // SysEx
                        let sysex_len = read_variable_length(&data, &mut pos)? as usize;
                        pos += sysex_len;
                    } else {
                        // Unknown system message -- try to skip gracefully
                        break;
                    }
                }
                _ => {
                    // Should not happen, but be defensive
                    break;
                }
            }
        }

        // Ensure we advance past any remaining track bytes
        pos = trk_end;
    }

    // ---- Convert events to Song ----
    // Determine which MIDI channels have data
    let active_channels: Vec<usize> = (0..16)
        .filter(|ch| !channel_events[*ch].is_empty())
        .collect();

    let num_channels = if active_channels.is_empty() {
        1
    } else {
        active_channels.len()
    };

    let speed: u8 = 6;
    let tpr = ticks_per_row_with_tpb(speed, file_tpb);

    // Find the maximum tick across all events to determine total rows needed
    let max_tick: u32 = channel_events
        .iter()
        .flat_map(|evts| evts.iter().map(|e| match e {
            ImportEvent::NoteOn { tick, .. } | ImportEvent::NoteOff { tick, .. } |
            ImportEvent::CC { tick, .. } | ImportEvent::ProgramChange { tick, .. } => *tick,
        }))
        .max()
        .unwrap_or(0);

    let total_rows = if tpr > 0 {
        (max_tick / tpr + 1) as usize
    } else {
        1
    };

    let rows_per_pattern = DEFAULT_ROWS_PER_PATTERN;
    let num_patterns = (total_rows + rows_per_pattern - 1) / rows_per_pattern;
    let num_patterns = num_patterns.max(1);

    // Build empty patterns
    let mut patterns: Vec<Pattern> = (0..num_patterns)
        .map(|_| Pattern::new(rows_per_pattern, num_channels))
        .collect();
    let order: Vec<usize> = (0..num_patterns).collect();

    // Place events into the grid
    for (tracker_ch, &midi_ch) in active_channels.iter().enumerate() {
        for event in &channel_events[midi_ch] {
            let tick = match event {
                ImportEvent::NoteOn { tick, .. } | ImportEvent::NoteOff { tick, .. } |
                ImportEvent::CC { tick, .. } | ImportEvent::ProgramChange { tick, .. } => *tick,
            };
            let global_row = if tpr > 0 {
                (tick / tpr) as usize
            } else {
                0
            };
            let pat_idx = global_row / rows_per_pattern;
            let row_in_pat = global_row % rows_per_pattern;
            if pat_idx >= patterns.len() {
                continue;
            }
            let cell = patterns[pat_idx].get_mut(row_in_pat, tracker_ch);

            match event {
                ImportEvent::NoteOn { note: midi_note, velocity: vel, .. } => {
                    let octave = midi_note / 12;
                    let note_idx = midi_note % 12;
                    if let Some(nv) = NoteValue::from_index(note_idx) {
                        cell.note = Some(Note::On {
                            value: nv,
                            octave,
                        });
                        if *vel != 0x7F {
                            cell.volume = Some(*vel);
                        }
                    }
                }
                ImportEvent::NoteOff { .. } => {
                    // Only place note-off if the cell does not already contain a note-on
                    if cell.note.is_none() {
                        cell.note = Some(Note::Off);
                    }
                }
                ImportEvent::CC { controller, value, .. } => {
                    // Map CC to Cxx effect (controller in instrument column, value in effect)
                    // Only if the cell does not already have an effect
                    if cell.effect.is_none() {
                        cell.effect = Some(0x0C); // MIDI CC effect
                        cell.effect_value = Some(*value);
                        cell.instrument = Some(*controller);
                    }
                }
                ImportEvent::ProgramChange { program, .. } => {
                    if cell.effect.is_none() {
                        cell.effect = Some(0x0E); // Program change effect
                        cell.effect_value = Some(*program);
                    }
                }
            }
        }
    }

    Ok(Song {
        title: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string(),
        bpm,
        speed,
        patterns,
        order_repeats: vec![1; order.len()],
        order,
        channels: num_channels,
        rows_per_pattern,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute ticks per row for export (always uses TICKS_PER_BEAT = 480).
fn ticks_per_row(speed: u8) -> u32 {
    speed as u32 * (TICKS_PER_BEAT as u32 / 24)
}

/// Compute ticks per row for a given file's ticks-per-beat value.
fn ticks_per_row_with_tpb(speed: u8, tpb: u32) -> u32 {
    if tpb >= 24 {
        speed as u32 * (tpb / 24)
    } else {
        // Fallback for very low tpb values
        speed as u32
    }
}

fn bpm_to_microseconds(bpm: u16) -> u32 {
    if bpm == 0 {
        500_000 // fallback 120 BPM
    } else {
        60_000_000 / bpm as u32
    }
}

fn read_u32_be(data: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > data.len() {
        bail!("Unexpected end of data reading u32");
    }
    let val = u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(val)
}

fn read_u16_be(data: &[u8], pos: &mut usize) -> Result<u16> {
    if *pos + 2 > data.len() {
        bail!("Unexpected end of data reading u16");
    }
    let val = u16::from_be_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(val)
}

fn read_byte(data: &[u8], pos: &mut usize) -> Result<u8> {
    if *pos >= data.len() {
        bail!("Unexpected end of data reading byte");
    }
    let b = data[*pos];
    *pos += 1;
    Ok(b)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Cell;
    use std::io::Write as _;

    #[test]
    fn test_variable_length_roundtrip() {
        let test_values: &[u32] = &[0, 1, 0x7F, 0x80, 0x3FFF, 0x4000, 0x001FFFFF, 0x0FFFFFFF];
        for &val in test_values {
            let mut buf = Vec::new();
            write_variable_length(&mut buf, val);
            let mut pos = 0usize;
            let decoded = read_variable_length(&buf, &mut pos).unwrap();
            assert_eq!(
                decoded, val,
                "Roundtrip failed for value 0x{:X}: encoded {:?}, decoded 0x{:X}",
                val, buf, decoded
            );
            assert_eq!(pos, buf.len(), "Not all bytes consumed for 0x{:X}", val);
        }
    }

    #[test]
    fn test_variable_length_known_encodings() {
        // Known encodings from the MIDI spec:
        // 0x00 -> [0x00]
        // 0x7F -> [0x7F]
        // 0x80 -> [0x81, 0x00]
        // 0x2000 -> [0xC0, 0x00]
        // 0x3FFF -> [0xFF, 0x7F]
        // 0x4000 -> [0x81, 0x80, 0x00]
        let cases: &[(u32, &[u8])] = &[
            (0x00, &[0x00]),
            (0x7F, &[0x7F]),
            (0x80, &[0x81, 0x00]),
            (0x2000, &[0xC0, 0x00]),
            (0x3FFF, &[0xFF, 0x7F]),
            (0x4000, &[0x81, 0x80, 0x00]),
        ];
        for &(val, expected) in cases {
            let mut buf = Vec::new();
            write_variable_length(&mut buf, val);
            assert_eq!(
                buf, expected,
                "Encoding mismatch for 0x{:X}: got {:?}, expected {:?}",
                val, buf, expected
            );
        }
    }

    #[test]
    fn test_export_starts_with_mthd() {
        let song = Song::new(2, 16);
        let tmp = std::env::temp_dir().join("rtrack_test_export_mthd.mid");
        export_midi(&song, &tmp).unwrap();

        let data = std::fs::read(&tmp).unwrap();
        assert!(data.len() >= 14, "File too small");
        assert_eq!(&data[0..4], b"MThd", "File must start with MThd");

        // Verify format = 1
        assert_eq!(u16::from_be_bytes([data[8], data[9]]), 1);
        // Verify division = 480
        assert_eq!(u16::from_be_bytes([data[12], data[13]]), TICKS_PER_BEAT);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_export_import_roundtrip() {
        let mut song = Song::new(2, 16);
        song.title = "TestRT".to_string();
        song.bpm = 140;
        song.speed = 6;

        // Place some notes
        song.patterns[0].set_cell(
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                instrument: None,
                volume: Some(100),
                effect: None,
                effect_value: None,
            },
        );
        song.patterns[0].set_cell(
            4,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::E,
                    octave: 4,
                }),
                instrument: None,
                volume: None,
                effect: None,
                effect_value: None,
            },
        );
        song.patterns[0].set_cell(
            8,
            0,
            Cell {
                note: Some(Note::Off),
                instrument: None,
                volume: None,
                effect: None,
                effect_value: None,
            },
        );
        // Second channel
        song.patterns[0].set_cell(
            0,
            1,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::G,
                    octave: 3,
                }),
                instrument: None,
                volume: Some(80),
                effect: None,
                effect_value: None,
            },
        );

        let tmp = std::env::temp_dir().join("rtrack_test_roundtrip.mid");
        export_midi(&song, &tmp).unwrap();

        let imported = import_midi(&tmp).unwrap();

        // BPM should match (integer truncation may cause off-by-one for some values)
        assert!(
            (imported.bpm as i32 - song.bpm as i32).unsigned_abs() <= 1,
            "BPM mismatch: exported {}, imported {}",
            song.bpm,
            imported.bpm
        );

        // Check that the first channel has the C-4 at row 0
        let cell_0_0 = imported.patterns[0].get(0, 0);
        assert_eq!(
            cell_0_0.note,
            Some(Note::On {
                value: NoteValue::C,
                octave: 4
            }),
            "Expected C-4 at (0,0)"
        );

        // Check that the E-4 at row 4 survived
        let cell_4_0 = imported.patterns[0].get(4, 0);
        assert_eq!(
            cell_4_0.note,
            Some(Note::On {
                value: NoteValue::E,
                octave: 4
            }),
            "Expected E-4 at (4,0)"
        );

        // Check the second channel has G-3 at row 0
        let cell_0_1 = imported.patterns[0].get(0, 1);
        assert_eq!(
            cell_0_1.note,
            Some(Note::On {
                value: NoteValue::G,
                octave: 3
            }),
            "Expected G-3 at (0,1)"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_import_empty_file_returns_error() {
        let tmp = std::env::temp_dir().join("rtrack_test_empty.mid");
        {
            let mut f = std::fs::File::create(&tmp).unwrap();
            f.write_all(b"").unwrap();
        }
        let result = import_midi(&tmp);
        assert!(result.is_err(), "Importing an empty file should fail");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_import_invalid_header_returns_error() {
        let tmp = std::env::temp_dir().join("rtrack_test_bad_header.mid");
        std::fs::write(&tmp, b"NOT_A_MIDI_FILE_AT_ALL").unwrap();
        let result = import_midi(&tmp);
        assert!(result.is_err(), "Importing a non-MIDI file should fail");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_bpm_to_microseconds() {
        assert_eq!(bpm_to_microseconds(120), 500_000);
        assert_eq!(bpm_to_microseconds(60), 1_000_000);
        assert_eq!(bpm_to_microseconds(0), 500_000); // fallback
    }

    #[test]
    fn test_ticks_per_row_calculation() {
        // speed=6, tpb=480 => 6 * (480/24) = 6 * 20 = 120
        assert_eq!(ticks_per_row(6), 120);
        // speed=3 => 3 * 20 = 60
        assert_eq!(ticks_per_row(3), 60);
    }
}
