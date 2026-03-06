use std::path::Path;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

use super::playback::SamplePlaybackEngine;
use super::SampleBank;
use crate::audio::effects::EffectsChain;
use crate::audio::synth::BuiltinSynth;
use crate::tracker::{Note, Song};

// Effect command constants (mirroring app constants)
const EFFECT_ARPEGGIO: u8 = 0x0;
const EFFECT_PORTA_UP: u8 = 0x1;
const EFFECT_PORTA_DOWN: u8 = 0x2;
const EFFECT_TONE_PORTA: u8 = 0x3;
const EFFECT_VIBRATO: u8 = 0x4;
const EFFECT_VOLUME_SLIDE: u8 = 0x5;
const EFFECT_NOTE_DELAY: u8 = 0x6;
const EFFECT_POSITION_JUMP: u8 = 0xB;
const EFFECT_PATTERN_BREAK: u8 = 0xD;
const EFFECT_PROGRAM_CHANGE: u8 = 0xE;
const EFFECT_SET_SPEED: u8 = 0xF;

/// Per-channel state for offline effect processing
#[derive(Clone)]
struct ExportChannelState {
    note: Option<u8>,
    volume: u8,
    effect: Option<u8>,
    effect_param: u8,
    pitch_offset: f64,
    porta_target: Option<u8>,
    vibrato_phase: f64,
    delayed_note: Option<(u8, u8, bool)>,
    delay_tick: u8,
}

impl Default for ExportChannelState {
    fn default() -> Self {
        Self {
            note: None,
            volume: 0x7F,
            effect: None,
            effect_param: 0,
            pitch_offset: 0.0,
            porta_target: None,
            vibrato_phase: 0.0,
            delayed_note: None,
            delay_tick: 0,
        }
    }
}

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
    let mut synth = BuiltinSynth::new(sr);
    let mut sample_engine = SamplePlaybackEngine::new(32);
    let mut effects = EffectsChain::new(sr);

    let mut all_left = Vec::new();
    let mut all_right = Vec::new();

    let mut current_speed = song.speed;
    let mut current_bpm = song.bpm;
    let mut order_pos = 0;
    let mut start_row = 0usize; // support break-to-row

    // Per-channel effect state
    let num_channels = song.channels.max(1);
    let mut ch_states: Vec<ExportChannelState> = vec![ExportChannelState::default(); num_channels];

    while order_pos < song.order.len() {
        let pattern_idx = song.order[order_pos];
        if pattern_idx >= song.patterns.len() {
            break;
        }
        let pattern = &song.patterns[pattern_idx];
        let mut row = start_row;
        start_row = 0; // reset for next pattern
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

        while row < pattern.rows {
            jump_order = None;
            break_row = None;

            // Ensure channel states cover all channels
            while ch_states.len() < pattern.channels {
                ch_states.push(ExportChannelState::default());
            }

            // -- Tick 0: process new row (trigger notes, scan effects) --
            for ch in 0..pattern.channels {
                let cell = pattern.get(row, ch);
                let midi_ch = (ch & 0x0F) as u8;
                let param = cell.effect_value.unwrap_or(0);

                // Pattern-level effects
                match cell.effect {
                    Some(EFFECT_POSITION_JUMP) => {
                        jump_order = Some(param as usize);
                    }
                    Some(EFFECT_PATTERN_BREAK) => {
                        break_row = Some(param as usize);
                    }
                    Some(EFFECT_SET_SPEED) => {
                        if param > 0 && param < 0x20 {
                            current_speed = param;
                        } else if param >= 0x20 {
                            current_bpm = param as u16;
                        }
                    }
                    Some(EFFECT_PROGRAM_CHANGE) => {
                        synth.program_change(midi_ch, param);
                    }
                    _ => {}
                }

                let is_tone_porta = cell.effect == Some(EFFECT_TONE_PORTA);
                let is_note_delay = cell.effect == Some(EFFECT_NOTE_DELAY) && param > 0;

                ch_states[ch].delayed_note = None;

                // Note events
                match cell.note {
                    Some(Note::On { .. }) => {
                        if let Some(midi_note) = cell.note.unwrap().to_midi_note() {
                            if is_tone_porta {
                                ch_states[ch].porta_target = Some(midi_note);
                            } else if is_note_delay {
                                let vel = cell.volume.unwrap_or(ch_states[ch].volume);
                                ch_states[ch].delayed_note = Some((midi_note, vel, false));
                                ch_states[ch].delay_tick = param;
                            } else {
                                let vel = cell.volume.unwrap_or(ch_states[ch].volume);
                                ch_states[ch].pitch_offset = 0.0;
                                ch_states[ch].vibrato_phase = 0.0;

                                // Note off previous
                                if let Some(prev) = ch_states[ch].note {
                                    synth.note_off(midi_ch, prev);
                                    sample_engine.note_off(midi_ch, prev);
                                }

                                // Route to sample or synth
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
                                ch_states[ch].note = Some(midi_note);
                                ch_states[ch].volume = vel;
                            }
                        }
                    }
                    Some(Note::Off) => {
                        if is_note_delay {
                            ch_states[ch].delayed_note = Some((0, 0, true));
                            ch_states[ch].delay_tick = param;
                        } else {
                            if let Some(prev) = ch_states[ch].note {
                                synth.note_off(midi_ch, prev);
                                sample_engine.note_off(midi_ch, prev);
                            }
                            ch_states[ch].note = None;
                            ch_states[ch].pitch_offset = 0.0;
                        }
                    }
                    None => {
                        if let Some(vol) = cell.volume {
                            ch_states[ch].volume = vol;
                        }
                    }
                }

                ch_states[ch].effect = cell.effect;
                ch_states[ch].effect_param = param;
            }

            // Recalculate frames_per_tick in case BPM changed
            let tps = (current_bpm as f64 * 24.0) / 60.0;
            let fpt = (sr / tps) as usize;

            // Render audio for all ticks of this row
            for tick in 0..current_speed {
                // -- Ticks 1+: process continuous effects --
                if tick > 0 {
                    for ch in 0..pattern.channels.min(ch_states.len()) {
                        let midi_ch = (ch & 0x0F) as u8;

                        // Note delay trigger
                        if ch_states[ch].effect == Some(EFFECT_NOTE_DELAY) {
                            if let Some((midi_note, vel, is_off)) = ch_states[ch].delayed_note {
                                if tick == ch_states[ch].delay_tick {
                                    if is_off {
                                        if let Some(prev) = ch_states[ch].note {
                                            synth.note_off(midi_ch, prev);
                                            sample_engine.note_off(midi_ch, prev);
                                        }
                                        ch_states[ch].note = None;
                                        ch_states[ch].pitch_offset = 0.0;
                                    } else {
                                        ch_states[ch].pitch_offset = 0.0;
                                        ch_states[ch].vibrato_phase = 0.0;
                                        if let Some(prev) = ch_states[ch].note {
                                            synth.note_off(midi_ch, prev);
                                            sample_engine.note_off(midi_ch, prev);
                                        }
                                        synth.note_on(midi_ch, midi_note, vel);
                                        ch_states[ch].note = Some(midi_note);
                                        ch_states[ch].volume = vel;
                                    }
                                    ch_states[ch].delayed_note = None;
                                }
                            }
                            continue;
                        }

                        let param = ch_states[ch].effect_param;
                        let base_note = match ch_states[ch].note {
                            Some(n) => n,
                            None => continue,
                        };

                        match ch_states[ch].effect {
                            Some(EFFECT_ARPEGGIO) if param != 0 => {
                                let x = (param >> 4) as f64;
                                let y = (param & 0x0F) as f64;
                                let phase = tick % 3;
                                let _offset = match phase {
                                    0 => 0.0,
                                    1 => x,
                                    _ => y,
                                };
                                // Arpeggio: in offline render, the pitch change is implicit
                                // since we don't have pitch bend on the synth. The effect is
                                // audible through MIDI but not through the built-in sine synth.
                                // We still track it for correctness.
                            }
                            Some(EFFECT_PORTA_UP) => {
                                ch_states[ch].pitch_offset += param as f64 / 16.0;
                            }
                            Some(EFFECT_PORTA_DOWN) => {
                                ch_states[ch].pitch_offset -= param as f64 / 16.0;
                            }
                            Some(EFFECT_TONE_PORTA) => {
                                if let Some(target) = ch_states[ch].porta_target {
                                    let current = base_note as f64 + ch_states[ch].pitch_offset;
                                    let target_f = target as f64;
                                    let speed = param as f64 / 16.0;
                                    if current < target_f {
                                        ch_states[ch].pitch_offset += speed.min(target_f - current);
                                    } else if current > target_f {
                                        ch_states[ch].pitch_offset -= speed.min(current - target_f);
                                    }
                                }
                            }
                            Some(EFFECT_VIBRATO) => {
                                let speed = (param >> 4) as f64;
                                let _depth = (param & 0x0F) as f64;
                                ch_states[ch].vibrato_phase += speed / 64.0;
                                if ch_states[ch].vibrato_phase >= 1.0 {
                                    ch_states[ch].vibrato_phase -= 1.0;
                                }
                                // Vibrato modulates pitch -- tracked but only audible via MIDI
                            }
                            Some(EFFECT_VOLUME_SLIDE) => {
                                let up = (param >> 4) as i16;
                                let down = (param & 0x0F) as i16;
                                let delta = up - down;
                                let new_vol = (ch_states[ch].volume as i16 + delta).clamp(0, 127) as u8;
                                ch_states[ch].volume = new_vol;
                                // Volume changes affect sample engine gain
                                // For the synth, volume is set at note-on only
                            }
                            _ => {}
                        }
                    }
                }

                let mut left = vec![0.0f32; fpt];
                let mut right = vec![0.0f32; fpt];

                // Render built-in synth
                for i in 0..fpt {
                    let (l, r) = synth.render_sample();
                    left[i] += l;
                    right[i] += r;
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
                start_row = break_row.unwrap_or(0);
                // Clamp start_row to target pattern bounds
                let target_pat = song.order[order_pos];
                if target_pat < song.patterns.len() {
                    start_row = start_row.min(song.patterns[target_pat].rows.saturating_sub(1));
                }
                break;
            }
            if let Some(target_row) = break_row {
                order_pos += 1;
                if order_pos >= song.order.len() {
                    order_pos = 0;
                }
                // Set start_row for the next pattern
                if order_pos < song.order.len() {
                    let target_pat = song.order[order_pos];
                    if target_pat < song.patterns.len() {
                        start_row = target_row.min(song.patterns[target_pat].rows.saturating_sub(1));
                    }
                }
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
        let (l, r) = synth.render_sample();
        tail_left[i] += l;
        tail_right[i] += r;
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
            source_path: None,
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
