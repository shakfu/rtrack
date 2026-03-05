use std::path::Path;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

use super::playback::SamplePlaybackEngine;
use super::SampleBank;
use crate::audio::effects::EffectsChain;
use crate::audio::synth::FundspSynth;
use crate::tracker::{Note, Song};
use fundsp::audiounit::AudioUnit;

/// Render an entire song to a WAV file (offline, non-real-time).
///
/// Renders built-in synths (fundsp) and sample playback. SF2 is not included
/// in offline render (use the built-in synth patches or samples).
pub fn render_to_wav(
    path: &Path,
    song: &Song,
    bank: &SampleBank,
    instruments: &[(Option<usize>, u8)], // (sample_index, midi_program) per instrument slot
    sample_rate: u32,
) -> Result<()> {
    let sr = sample_rate as f64;

    // Create offline audio components
    let mut synth = FundspSynth::new(sr);
    let mut backend = synth.backend();
    let mut sample_engine = SamplePlaybackEngine::new(32);
    let mut effects = EffectsChain::new(sr);

    let mut all_left = Vec::new();
    let mut all_right = Vec::new();

    let mut current_speed = song.speed;
    let mut current_bpm = song.bpm;
    let mut order_pos = 0;

    // Channel state for tracking active notes per channel
    let mut channel_notes: Vec<Option<u8>> = vec![None; 16];

    while order_pos < song.order.len() {
        let pattern_idx = song.order[order_pos];
        if pattern_idx >= song.patterns.len() {
            break;
        }
        let pattern = &song.patterns[pattern_idx];
        let mut row = 0;
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        while row < pattern.rows {
            // Process row: fire notes on tick 0
            jump_order = None;
            break_row = None;

            for ch in 0..pattern.channels {
                let cell = pattern.get(row, ch);
                let midi_ch = ch.min(15) as u8;

                // Handle pattern-level effects
                if let Some(0xB) = cell.effect {
                    jump_order = Some(cell.effect_value.unwrap_or(0) as usize);
                }
                if let Some(0xD) = cell.effect {
                    break_row = Some(cell.effect_value.unwrap_or(0) as usize);
                }
                if let Some(0xF) = cell.effect {
                    let val = cell.effect_value.unwrap_or(0);
                    if val > 0 && val < 0x20 {
                        current_speed = val;
                    } else if val >= 0x20 {
                        current_bpm = val as u16;
                    }
                }

                // Program change
                if let Some(0xE) = cell.effect {
                    let prog = cell.effect_value.unwrap_or(0);
                    synth.program_change(midi_ch, prog);
                }

                // Note events
                match cell.note {
                    Some(Note::On { .. }) => {
                        if let Some(midi_note) = cell.note.unwrap().to_midi_note() {
                            let vel = cell.volume.unwrap_or(0x7F);

                            // Note off previous
                            if let Some(prev) = channel_notes[midi_ch as usize] {
                                synth.note_off(midi_ch, prev);
                                sample_engine.note_off(midi_ch, prev);
                            }

                            // Check if instrument has a sample assigned
                            let inst_idx = cell.instrument.unwrap_or(0) as usize;
                            let has_sample = instruments
                                .get(inst_idx)
                                .and_then(|(si, _)| *si)
                                .and_then(|idx| bank.get(idx))
                                .is_some();

                            if has_sample {
                                let sample_idx = instruments[inst_idx].0.unwrap();
                                let sample = bank.get(sample_idx).unwrap();
                                sample_engine.note_on(
                                    sample_idx, midi_note, vel, midi_ch, sample, sr,
                                );
                            } else {
                                synth.note_on(midi_ch, midi_note, vel);
                            }
                            channel_notes[midi_ch as usize] = Some(midi_note);
                        }
                    }
                    Some(Note::Off) => {
                        if let Some(prev) = channel_notes[midi_ch as usize] {
                            synth.note_off(midi_ch, prev);
                            sample_engine.note_off(midi_ch, prev);
                        }
                        channel_notes[midi_ch as usize] = None;
                    }
                    None => {}
                }
            }

            // Recalculate frames_per_tick in case BPM changed
            let tps = (current_bpm as f64 * 24.0) / 60.0;
            let fpt = (sr / tps) as usize;

            // Render audio for all ticks of this row
            for _tick in 0..current_speed {
                let mut left = vec![0.0f32; fpt];
                let mut right = vec![0.0f32; fpt];

                // Render fundsp synth
                for i in 0..fpt {
                    let mut output = [0f32; 2];
                    backend.tick(&[], &mut output);
                    left[i] += output[0];
                    right[i] += output[1];
                }

                // Render samples
                sample_engine.render(bank, &mut left, &mut right);

                // Apply effects
                effects.process(&mut left, &mut right);

                all_left.extend_from_slice(&left);
                all_right.extend_from_slice(&right);
            }

            // Handle jumps/breaks
            if let Some(target) = jump_order {
                order_pos = target.min(song.order.len() - 1);
                row = break_row.unwrap_or(0);
                // Don't increment order_pos at the end of this loop
                // Instead jump directly
                break;
            }
            if break_row.is_some() {
                order_pos += 1;
                if order_pos >= song.order.len() {
                    order_pos = 0;
                }
                // Start at target_row of next pattern
                // We break out and let the outer loop handle it
                // TODO: this doesn't perfectly handle break to specific row
                break;
            }

            row += 1;
        }

        // Normal advance (only if no jump/break occurred)
        if jump_order.is_none() && break_row.is_none() {
            order_pos += 1;
        }
    }

    // Turn off all remaining notes and render a short tail for reverb/release
    synth.note_off_all();
    sample_engine.note_off_all();
    let tail_frames = (sr * 2.0) as usize; // 2 seconds tail
    let mut tail_left = vec![0.0f32; tail_frames];
    let mut tail_right = vec![0.0f32; tail_frames];
    for i in 0..tail_frames {
        let mut output = [0f32; 2];
        backend.tick(&[], &mut output);
        tail_left[i] += output[0];
        tail_right[i] += output[1];
    }
    effects.process(&mut tail_left, &mut tail_right);
    all_left.extend_from_slice(&tail_left);
    all_right.extend_from_slice(&tail_right);

    // Write WAV
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).with_context(|| format!("Failed to create WAV: {}", path.display()))?;

    for i in 0..all_left.len() {
        let l = all_left[i].clamp(-1.0, 1.0);
        let r = all_right[i].clamp(-1.0, 1.0);
        writer
            .write_sample(l.to_sample::<i16>())
            .context("Failed to write WAV sample")?;
        writer
            .write_sample(r.to_sample::<i16>())
            .context("Failed to write WAV sample")?;
    }
    writer.finalize().context("Failed to finalize WAV file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{Cell, Note, NoteValue, Song};

    #[test]
    fn test_render_empty_song() {
        let song = Song::new(4, 64);
        let bank = SampleBank::new();
        let instruments: Vec<(Option<usize>, u8)> = vec![(None, 0); 256];
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_empty.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, 44100);
        assert!(result.is_ok());

        // Verify the file exists and is a valid WAV
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 44100);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_render_with_synth_note() {
        let mut song = Song::new(1, 2);
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

        let bank = SampleBank::new();
        let instruments: Vec<(Option<usize>, u8)> = vec![(None, 0); 256];
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_synth.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, 44100);
        assert!(result.is_ok());

        // Should have some audio content (not all silence)
        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for synth note");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_render_with_sample() {
        let mut song = Song::new(1, 2);
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
                instrument: Some(0),
                ..Cell::default()
            },
        );

        let mut bank = SampleBank::new();
        // Create a test sample: 1000 frames of sine wave
        let sample_data: Vec<[f32; 2]> = (0..1000)
            .map(|i| {
                let t = i as f32 / 44100.0;
                let val = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                [val, val]
            })
            .collect();
        bank.samples[0] = Some(super::super::Sample {
            name: "sine".into(),
            data: sample_data,
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
        });

        let instruments: Vec<(Option<usize>, u8)> = {
            let mut v = vec![(None, 0); 256];
            v[0] = (Some(0), 0); // instrument 0 -> sample 0
            v
        };
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_sample.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, 44100);
        assert!(result.is_ok());

        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for sample note");

        let _ = std::fs::remove_file(&path);
    }
}
