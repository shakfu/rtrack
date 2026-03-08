use std::path::Path;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

use super::playback::SamplePlaybackEngine;
use super::SampleBank;
use crate::audio::channel_effects::{ChannelEffects, ChannelEffectsParams, MAX_EFFECT_CHANNELS};
use crate::audio::effects::{self, EffectsChain};
use crate::audio::synth::{BuiltinSynth, SynthParams};
use crate::constants::*;
use crate::tracker::{Note, Song};

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
            volume: MIDI_DEFAULT_VELOCITY,
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
/// Instrument slot info for offline render
pub struct ExportInstrument {
    pub sample_index: Option<usize>,
    #[allow(dead_code)]
    pub midi_program: u8,
    pub synth_params: Option<SynthParams>,
}

pub fn render_to_wav(
    path: &Path,
    song: &Song,
    bank: &SampleBank,
    instruments: &[ExportInstrument],
    channel_fx_params: &[ChannelEffectsParams],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
) -> Result<()> {
    let (left, right) = render_song(song, bank, instruments, channel_fx_params, send_bus_params, sample_rate)?;
    write_wav(path, &left, &right, sample_rate)
}

/// Render an entire song to stereo f32 buffers (offline, non-real-time).
fn render_song(
    song: &Song,
    bank: &SampleBank,
    instruments: &[ExportInstrument],
    channel_fx_params: &[ChannelEffectsParams],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let sr = sample_rate as f64;

    // Create offline audio components
    let mut synth = BuiltinSynth::new(sr);
    let mut sample_engine = SamplePlaybackEngine::new(32);
    let mut effects = EffectsChain::new(sr);

    // Create per-channel effects for offline render
    let mut channel_effects: Vec<ChannelEffects> = (0..MAX_EFFECT_CHANNELS)
        .map(|ch| {
            let mut fx = ChannelEffects::new(sr);
            if let Some(params) = channel_fx_params.get(ch) {
                fx.params = params.clone();
            }
            fx
        })
        .collect();
    // Create send/return buses for offline render
    let mut send_buses: Vec<effects::SendBus> = (0..effects::MAX_SEND_BUSES)
        .map(|i| {
            let mut bus = effects::SendBus::new(sr);
            if let Some(params) = send_bus_params.get(i) {
                bus.params = params.clone();
            }
            bus
        })
        .collect();
    let any_send_bus = send_buses.iter().any(|b| b.params.enabled);

    let any_ch_fx = channel_effects.iter().any(|fx| fx.any_enabled());

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
        // Check repeat count: 0 = skip
        let repeat_count = song.order_repeats.get(order_pos).copied().unwrap_or(1);
        if repeat_count == 0 {
            order_pos += 1;
            continue;
        }
        let pattern_idx = song.order[order_pos];
        if pattern_idx >= song.patterns.len() {
            break;
        }
        let pattern = &song.patterns[pattern_idx];
        let mut repeats_done = 0u8;
        let mut jump_order: Option<usize> = None;
        let mut break_row: Option<usize> = None;

      'repeat_loop: while repeats_done < repeat_count {
        let mut row = start_row;
        start_row = 0;
        jump_order = None;
        break_row = None;

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

                                // Route to sample, custom synth, or default synth
                                let inst_idx = cell.instrument.unwrap_or(0) as usize;
                                let inst = instruments.get(inst_idx);
                                let has_sample = inst
                                    .and_then(|i| i.sample_index)
                                    .and_then(|idx| bank.get(idx))
                                    .is_some();

                                if has_sample {
                                    let sample_idx = inst.unwrap().sample_index.unwrap();
                                    let sample = bank.get(sample_idx).unwrap();
                                    sample_engine.note_on(
                                        sample_idx, midi_note, vel, midi_ch, sample, sr,
                                    );
                                } else if let Some(ref sp) = inst.and_then(|i| i.synth_params.as_ref()) {
                                    synth.note_on_with_params(midi_ch, midi_note, vel, sp);
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

            // Recalculate frames_per_tick with swing
            let base_tps = (current_bpm as f64 * MIDI_CLOCKS_PER_BEAT) / 60.0;
            let base_spt = 1.0 / base_tps;
            let swing_spt = if song.swing == 50 {
                base_spt
            } else {
                let swing_f = song.swing as f64;
                if row % 2 == 0 {
                    base_spt * swing_f / 50.0
                } else {
                    base_spt * (100.0 - swing_f) / 50.0
                }
            };
            let fpt = (sr * swing_spt) as usize;

            // Check tempo automation
            if let Some(bpm) = song.tempo_at(order_pos, row) {
                let new_bpm = bpm.round() as u16;
                if new_bpm >= 1 {
                    current_bpm = new_bpm;
                }
            }

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
                                let offset = match phase {
                                    0 => 0.0,
                                    1 => x,
                                    _ => y,
                                };
                                // Apply arpeggio as pitch offset
                                let midi_ch = (ch & 0x0F) as u8;
                                synth.set_channel_pitch_offset(midi_ch, offset as f32);
                                sample_engine.set_channel_pitch_offset(midi_ch, offset, bank, sr);
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
                                let depth = (param & 0x0F) as f64;
                                ch_states[ch].vibrato_phase += speed / 64.0;
                                if ch_states[ch].vibrato_phase >= 1.0 {
                                    ch_states[ch].vibrato_phase -= 1.0;
                                }
                                let sine = (ch_states[ch].vibrato_phase * std::f64::consts::TAU).sin();
                                let vib_offset = sine * depth / 16.0;
                                // Apply pitch_offset + vibrato (vibrato is instantaneous, not cumulative)
                                let total = ch_states[ch].pitch_offset + vib_offset;
                                let midi_ch = (ch & 0x0F) as u8;
                                synth.set_channel_pitch_offset(midi_ch, total as f32);
                                sample_engine.set_channel_pitch_offset(midi_ch, total, bank, sr);
                            }
                            Some(EFFECT_VOLUME_SLIDE) => {
                                let up = (param >> 4) as i16;
                                let down = (param & 0x0F) as i16;
                                let delta = up - down;
                                let new_vol = (ch_states[ch].volume as i16 + delta).clamp(0, MIDI_MAX_VALUE as i16) as u8;
                                ch_states[ch].volume = new_vol;
                            }
                            _ => {}
                        }

                        // Apply accumulated pitch offset and volume to engines
                        // (arpeggio and vibrato handle pitch themselves above)
                        let eff = ch_states[ch].effect;
                        if eff != Some(EFFECT_ARPEGGIO) && eff != Some(EFFECT_VIBRATO) {
                            let midi_ch = (ch & 0x0F) as u8;
                            synth.set_channel_pitch_offset(midi_ch, ch_states[ch].pitch_offset as f32);
                            sample_engine.set_channel_pitch_offset(midi_ch, ch_states[ch].pitch_offset, bank, sr);
                        }
                        let midi_ch = (ch & 0x0F) as u8;
                        synth.set_channel_volume(midi_ch, ch_states[ch].volume);
                        sample_engine.set_channel_volume(midi_ch, ch_states[ch].volume);
                    }
                }

                let mut left = vec![0.0f32; fpt];
                let mut right = vec![0.0f32; fpt];

                if any_ch_fx || any_send_bus {
                    // Per-channel rendering path (needed for channel effects or send buses)
                    let mut ch_left: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
                        .map(|_| vec![0.0f32; fpt])
                        .collect();
                    let mut ch_right: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
                        .map(|_| vec![0.0f32; fpt])
                        .collect();

                    for i in 0..fpt {
                        let mut ch_out = [[0.0f32; 2]; MAX_EFFECT_CHANNELS];
                        synth.render_sample_per_channel(&mut ch_out);
                        for ch in 0..MAX_EFFECT_CHANNELS {
                            ch_left[ch][i] += ch_out[ch][0];
                            ch_right[ch][i] += ch_out[ch][1];
                        }
                    }

                    {
                        let mut slices: Vec<(&mut [f32], &mut [f32])> = Vec::with_capacity(MAX_EFFECT_CHANNELS);
                        for ch in 0..MAX_EFFECT_CHANNELS {
                            let l = &mut ch_left[ch][..fpt] as *mut [f32];
                            let r = &mut ch_right[ch][..fpt] as *mut [f32];
                            unsafe { slices.push((&mut *l, &mut *r)); }
                        }
                        sample_engine.render_per_channel(bank, &mut slices);
                    }

                    // Clear send bus inputs
                    for bus in send_buses.iter_mut() {
                        bus.ensure_size(fpt);
                        bus.clear_inputs(fpt);
                    }

                    for ch in 0..MAX_EFFECT_CHANNELS {
                        channel_effects[ch].process(&mut ch_left[ch], &mut ch_right[ch]);

                        // Feed send buses (post-channel-effects)
                        let send_levels = channel_effects[ch].params.send_levels;
                        for (bus_idx, bus) in send_buses.iter_mut().enumerate() {
                            if bus.params.enabled && send_levels[bus_idx] > 0.0 {
                                bus.add_send(&ch_left[ch], &ch_right[ch], send_levels[bus_idx]);
                            }
                        }

                        for i in 0..fpt {
                            left[i] += ch_left[ch][i];
                            right[i] += ch_right[ch][i];
                        }
                    }

                    // Process send buses to master
                    for bus in send_buses.iter_mut() {
                        bus.process_to_master(&mut left, &mut right, fpt);
                    }
                } else {
                    for i in 0..fpt {
                        let (l, r) = synth.render_sample();
                        left[i] += l;
                        right[i] += r;
                    }
                    sample_engine.render(bank, &mut left, &mut right);
                }

                // Apply master effects
                effects.process(&mut left, &mut right);

                all_left.extend_from_slice(&left);
                all_right.extend_from_slice(&right);
            }

            // Handle jumps/breaks
            if let Some(target) = jump_order {
                order_pos = target.min(song.order.len() - 1);
                start_row = break_row.unwrap_or(0);
                let target_pat = song.order[order_pos];
                if target_pat < song.patterns.len() {
                    start_row = start_row.min(song.patterns[target_pat].rows.saturating_sub(1));
                }
                break 'repeat_loop;
            }
            if let Some(target_row) = break_row {
                order_pos += 1;
                if order_pos >= song.order.len() {
                    order_pos = 0;
                }
                if order_pos < song.order.len() {
                    let target_pat = song.order[order_pos];
                    if target_pat < song.patterns.len() {
                        start_row = target_row.min(song.patterns[target_pat].rows.saturating_sub(1));
                    }
                }
                break 'repeat_loop;
            }

            row += 1;
        }

        repeats_done += 1;
      } // end repeat_loop

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
    if any_ch_fx || any_send_bus {
        let mut ch_left: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
            .map(|_| vec![0.0f32; tail_frames])
            .collect();
        let mut ch_right: Vec<Vec<f32>> = (0..MAX_EFFECT_CHANNELS)
            .map(|_| vec![0.0f32; tail_frames])
            .collect();
        for i in 0..tail_frames {
            let mut ch_out = [[0.0f32; 2]; MAX_EFFECT_CHANNELS];
            synth.render_sample_per_channel(&mut ch_out);
            for ch in 0..MAX_EFFECT_CHANNELS {
                ch_left[ch][i] += ch_out[ch][0];
                ch_right[ch][i] += ch_out[ch][1];
            }
        }
        // Clear send bus inputs for tail
        for bus in send_buses.iter_mut() {
            bus.ensure_size(tail_frames);
            bus.clear_inputs(tail_frames);
        }
        for ch in 0..MAX_EFFECT_CHANNELS {
            channel_effects[ch].process(&mut ch_left[ch], &mut ch_right[ch]);
            let send_levels = channel_effects[ch].params.send_levels;
            for (bus_idx, bus) in send_buses.iter_mut().enumerate() {
                if bus.params.enabled && send_levels[bus_idx] > 0.0 {
                    bus.add_send(&ch_left[ch], &ch_right[ch], send_levels[bus_idx]);
                }
            }
            for i in 0..tail_frames {
                tail_left[i] += ch_left[ch][i];
                tail_right[i] += ch_right[ch][i];
            }
        }
        for bus in send_buses.iter_mut() {
            bus.process_to_master(&mut tail_left, &mut tail_right, tail_frames);
        }
    } else {
        for i in 0..tail_frames {
            let (l, r) = synth.render_sample();
            tail_left[i] += l;
            tail_right[i] += r;
        }
    }
    effects.process(&mut tail_left, &mut tail_right);
    all_left.extend_from_slice(&tail_left);
    all_right.extend_from_slice(&tail_right);

    Ok((all_left, all_right))
}

/// Convert f32 stereo buffers to interleaved i16 samples
fn to_interleaved_i16(left: &[f32], right: &[f32]) -> Vec<i16> {
    let mut samples = Vec::with_capacity(left.len() * 2);
    for i in 0..left.len() {
        let l = left[i].clamp(-1.0, 1.0);
        let r = right[i].clamp(-1.0, 1.0);
        samples.push(l.to_sample::<i16>());
        samples.push(r.to_sample::<i16>());
    }
    samples
}

fn write_wav(path: &Path, left: &[f32], right: &[f32], sample_rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).with_context(|| format!("Failed to create WAV: {}", path.display()))?;

    for i in 0..left.len() {
        let l = left[i].clamp(-1.0, 1.0);
        let r = right[i].clamp(-1.0, 1.0);
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

/// Render an entire song to a FLAC file (offline, non-real-time).
pub fn render_to_flac(
    path: &Path,
    song: &Song,
    bank: &SampleBank,
    instruments: &[ExportInstrument],
    channel_fx_params: &[ChannelEffectsParams],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
) -> Result<()> {
    let (left, right) = render_song(song, bank, instruments, channel_fx_params, send_bus_params, sample_rate)?;
    let samples_i16 = to_interleaved_i16(&left, &right);
    write_flac(path, &samples_i16, sample_rate)
}

fn write_flac(path: &Path, samples_i16: &[i16], sample_rate: u32) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let config = flacenc::config::Encoder::default().into_verified()
        .map_err(|e| anyhow::anyhow!("FLAC config error: {:?}", e))?;
    let source = flacenc::source::MemSource::from_samples(
        &samples_i16.iter().map(|&s| s as i32).collect::<Vec<_>>(),
        2,
        16,
        sample_rate as usize,
    );
    let block_size = config.block_size;
    let flac_stream = flacenc::encode_with_fixed_block_size(&config, source, block_size)
        .map_err(|e| anyhow::anyhow!("FLAC encoding failed: {:?}", e))?;

    let mut file = std::fs::File::create(path)
        .with_context(|| format!("Failed to create FLAC: {}", path.display()))?;
    let mut bw = flacenc::bitsink::ByteSink::new();
    flac_stream.write(&mut bw)
        .map_err(|e| anyhow::anyhow!("FLAC write failed: {:?}", e))?;
    std::io::Write::write_all(&mut file, bw.as_slice())
        .context("Failed to write FLAC data")?;

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
        let instruments: Vec<ExportInstrument> = (0..256).map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None }).collect();
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_empty.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
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
        let instruments: Vec<ExportInstrument> = (0..256).map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None }).collect();
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_synth.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
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

        let instruments: Vec<ExportInstrument> = {
            let mut v: Vec<ExportInstrument> = (0..256).map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None }).collect();
            v[0].sample_index = Some(0); // instrument 0 -> sample 0
            v
        };
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_sample.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok());

        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for sample note");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_render_portamento_changes_pitch() {
        // Two rows: note on C-5, then portamento up effect on row 1
        // The portamento should shift the synth pitch, producing different audio
        // than a static note.
        let mut song = Song::new(1, 4);
        song.speed = 6;
        song.bpm = 120;
        // Row 0: note on
        song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(100),
            ..Cell::default()
        });
        // Row 1: portamento up (1xx with param 0x40 = fast slide)
        song.patterns[0].set_cell(1, 0, Cell {
            effect: Some(EFFECT_PORTA_UP),
            effect_value: Some(0x40),
            ..Cell::default()
        });

        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None })
            .collect();
        let dir = std::env::temp_dir();
        let path_with = dir.join("rtrack_test_porta.wav");

        render_to_wav(&path_with, &song, &bank, &instruments, &[], &[], 44100).unwrap();

        // Now render without the effect for comparison
        let mut song_no_fx = Song::new(1, 4);
        song_no_fx.speed = 6;
        song_no_fx.bpm = 120;
        song_no_fx.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(100),
            ..Cell::default()
        });

        let path_without = dir.join("rtrack_test_no_porta.wav");
        render_to_wav(&path_without, &song_no_fx, &bank, &instruments, &[], &[], 44100).unwrap();

        // Read both and compare -- they should differ
        let r1 = hound::WavReader::open(&path_with).unwrap();
        let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let r2 = hound::WavReader::open(&path_without).unwrap();
        let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

        // Both should have audio
        assert!(s1.iter().any(|&s| s.abs() > 10), "porta render should have audio");
        assert!(s2.iter().any(|&s| s.abs() > 10), "no-fx render should have audio");

        // Samples should differ (portamento shifted pitch)
        let min_len = s1.len().min(s2.len());
        let diff_count = s1[..min_len].iter().zip(&s2[..min_len]).filter(|(a, b)| a != b).count();
        assert!(diff_count > min_len / 4, "Expected portamento to produce audibly different output, but only {}/{} samples differed", diff_count, min_len);

        let _ = std::fs::remove_file(&path_with);
        let _ = std::fs::remove_file(&path_without);
    }

    #[test]
    fn test_render_volume_slide() {
        // Verify volume slide actually changes the output level
        let mut song = Song::new(1, 4);
        song.speed = 6;
        song.bpm = 120;
        // Row 0: note on at full volume
        song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(127),
            ..Cell::default()
        });
        // Row 1: volume slide down (50F = slide down by 15 per tick)
        song.patterns[0].set_cell(1, 0, Cell {
            effect: Some(EFFECT_VOLUME_SLIDE),
            effect_value: Some(0x0F),
            ..Cell::default()
        });

        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None })
            .collect();
        let dir = std::env::temp_dir();

        // Render with volume slide
        let path_slide = dir.join("rtrack_test_volslide.wav");
        render_to_wav(&path_slide, &song, &bank, &instruments, &[], &[], 44100).unwrap();

        // Render without (static volume)
        let mut song_static = Song::new(1, 4);
        song_static.speed = 6;
        song_static.bpm = 120;
        song_static.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(127),
            ..Cell::default()
        });
        let path_static = dir.join("rtrack_test_volstatic.wav");
        render_to_wav(&path_static, &song_static, &bank, &instruments, &[], &[], 44100).unwrap();

        let r1 = hound::WavReader::open(&path_slide).unwrap();
        let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let r2 = hound::WavReader::open(&path_static).unwrap();
        let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

        // Both should have audio
        assert!(s1.iter().any(|&s| s.abs() > 10));
        assert!(s2.iter().any(|&s| s.abs() > 10));

        // Volume-slid version should differ from static
        let min_len = s1.len().min(s2.len());
        let diff_count = s1[..min_len].iter().zip(&s2[..min_len]).filter(|(a, b)| a != b).count();
        assert!(diff_count > 0, "Volume slide should produce different output than static volume");

        let _ = std::fs::remove_file(&path_slide);
        let _ = std::fs::remove_file(&path_static);
    }

    #[test]
    fn test_render_to_flac() {
        let mut song = Song::new(1, 2);
        song.speed = 2;
        song.patterns[0].set_cell(0, 0, Cell {
            note: Some(Note::On { value: NoteValue::C, octave: 5 }),
            volume: Some(100),
            ..Cell::default()
        });

        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument { sample_index: None, midi_program: 0, synth_params: None })
            .collect();
        let dir = std::env::temp_dir();
        let path = dir.join("rtrack_test_export.flac");

        let result = render_to_flac(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok(), "FLAC export failed: {:?}", result.err());

        // Verify the file exists and is non-empty
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0, "FLAC file should be non-empty");

        let _ = std::fs::remove_file(&path);
    }
}
