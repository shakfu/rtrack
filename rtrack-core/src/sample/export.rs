use std::path::Path;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

use super::playback::{NewNoteAction, SamplePlaybackEngine};
use super::SampleBank;
use crate::audio::channel_effects::{ChannelEffects, ChannelEffectsParams, MAX_EFFECT_CHANNELS};
use crate::audio::effects::{self, EffectsChain};
use crate::audio::synth::{BuiltinSynth, SynthParams};
use crate::engine::{TrackerEngine, TrackerEvent};
use crate::tracker::Song;

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
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("Failed to create WAV: {}", path.display()))?;

    // Each block is written as it is rendered, so the whole song is never
    // held in memory at once.
    render_song_streaming(
        song,
        bank,
        instruments,
        channel_fx_params,
        send_bus_params,
        sample_rate,
        &mut |left, right| write_block(&mut writer, left, right),
    )?;

    writer.finalize().context("Failed to finalize WAV file")?;
    Ok(())
}

/// Append one rendered block to an open WAV writer.
fn write_block<W: std::io::Write + std::io::Seek>(
    writer: &mut hound::WavWriter<W>,
    left: &[f32],
    right: &[f32],
) -> Result<()> {
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
    Ok(())
}

/// Where each rendered block goes. Called once per block, in order.
type BlockSink<'a> = dyn FnMut(&[f32], &[f32]) -> Result<()> + 'a;

/// Buffers the offline renderer reuses from block to block.
///
/// Rendering used to allocate these inside the tick loop: two master buffers
/// plus, with channel effects or send buses engaged, one pair per effect
/// channel -- thirty-four `Vec<f32>`s per tick, zeroed and dropped. A tick at
/// 170bpm is about 15ms, so a five-minute export spent well over a million
/// allocations on buffers whose size barely changes.
struct RenderScratch {
    left: Vec<f32>,
    right: Vec<f32>,
    ch_left: Vec<Vec<f32>>,
    ch_right: Vec<Vec<f32>>,
}

impl RenderScratch {
    fn new() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
            ch_left: (0..MAX_EFFECT_CHANNELS).map(|_| Vec::new()).collect(),
            ch_right: (0..MAX_EFFECT_CHANNELS).map(|_| Vec::new()).collect(),
        }
    }

    /// Size every buffer to `frames` and clear it. Tempo changes move the
    /// frames-per-tick around, so the length is set per block rather than
    /// once; only a block longer than any before it allocates.
    fn prepare(&mut self, frames: usize, per_channel: bool) {
        prepare_buffer(&mut self.left, frames);
        prepare_buffer(&mut self.right, frames);
        if per_channel {
            for ch in 0..MAX_EFFECT_CHANNELS {
                prepare_buffer(&mut self.ch_left[ch], frames);
                prepare_buffer(&mut self.ch_right[ch], frames);
            }
        }
    }
}

fn prepare_buffer(buf: &mut Vec<f32>, frames: usize) {
    buf.clear();
    buf.resize(frames, 0.0);
}

/// Render `frames` frames of the mix into `scratch.left`/`scratch.right`.
///
/// The song body and the release tail differ only in how long they are and
/// in whether notes are still being started, so they share this.
#[allow(clippy::too_many_arguments)]
fn render_block(
    scratch: &mut RenderScratch,
    frames: usize,
    per_channel: bool,
    synth: &mut BuiltinSynth,
    sample_engine: &mut SamplePlaybackEngine,
    bank: &SampleBank,
    channel_effects: &mut [ChannelEffects],
    send_buses: &mut [effects::SendBus],
    master_fx: &mut EffectsChain,
) {
    scratch.prepare(frames, per_channel);

    if per_channel {
        for i in 0..frames {
            let mut ch_out = [[0.0f32; 2]; MAX_EFFECT_CHANNELS];
            synth.render_sample_per_channel(&mut ch_out);
            for (ch, out) in ch_out.iter().enumerate() {
                scratch.ch_left[ch][i] += out[0];
                scratch.ch_right[ch][i] += out[1];
            }
        }

        sample_engine.render_per_channel(
            bank,
            &mut scratch.ch_left,
            &mut scratch.ch_right,
            0..frames,
        );

        for bus in send_buses.iter_mut() {
            bus.ensure_size(frames);
            bus.clear_inputs(frames);
        }

        for (ch, fx) in channel_effects
            .iter_mut()
            .enumerate()
            .take(MAX_EFFECT_CHANNELS)
        {
            fx.process(&mut scratch.ch_left[ch], &mut scratch.ch_right[ch]);
            let send_levels = fx.params.send_levels;
            for (bus_idx, bus) in send_buses.iter_mut().enumerate() {
                if bus.params.enabled && send_levels[bus_idx] > 0.0 {
                    bus.add_send(
                        &scratch.ch_left[ch],
                        &scratch.ch_right[ch],
                        send_levels[bus_idx],
                    );
                }
            }
            for i in 0..frames {
                scratch.left[i] += scratch.ch_left[ch][i];
                scratch.right[i] += scratch.ch_right[ch][i];
            }
        }

        for bus in send_buses.iter_mut() {
            bus.process_to_master(&mut scratch.left, &mut scratch.right, frames);
        }
    } else {
        for i in 0..frames {
            let (l, r) = synth.render_sample();
            scratch.left[i] += l;
            scratch.right[i] += r;
        }
        sample_engine.render(bank, &mut scratch.left, &mut scratch.right);
    }

    master_fx.process(&mut scratch.left, &mut scratch.right);
}

/// Render an entire song, handing each rendered block to `sink` as it is
/// produced.
///
/// Streaming rather than returning the whole song lets a caller that can
/// write incrementally -- WAV -- keep only one block in memory instead of
/// the entire render, which for five stereo minutes at 44.1kHz was about
/// 105MB of `f32` before encoding even started.
#[allow(clippy::too_many_arguments)]
fn render_song_streaming(
    song: &Song,
    bank: &SampleBank,
    instruments: &[ExportInstrument],
    channel_fx_params: &[ChannelEffectsParams],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
    sink: &mut BlockSink<'_>,
) -> Result<()> {
    let sr = sample_rate as f64;

    // Create offline audio components
    let mut synth = BuiltinSynth::new(sr);
    let mut sample_engine = SamplePlaybackEngine::new(32);
    let mut master_fx = EffectsChain::new(sr);

    let mut channel_effects: Vec<ChannelEffects> = (0..MAX_EFFECT_CHANNELS)
        .map(|ch| {
            let mut fx = ChannelEffects::new(sr);
            if let Some(params) = channel_fx_params.get(ch) {
                fx.params = params.clone();
            }
            fx
        })
        .collect();
    let mut send_buses: Vec<effects::SendBus> = (0..effects::MAX_SEND_BUSES)
        .map(|i| {
            // Offline: the block size follows the tempo, so this is a
            // starting size and `ensure_size` grows it as needed. Nothing
            // here has a deadline to miss.
            let mut bus = effects::SendBus::new(sr, 0);
            if let Some(params) = send_bus_params.get(i) {
                bus.params = params.clone();
            }
            bus
        })
        .collect();
    let any_send_bus = send_buses.iter().any(|b| b.params.enabled);
    let any_ch_fx = channel_effects.iter().any(|fx| fx.any_enabled());

    let per_channel = any_ch_fx || any_send_bus;
    let mut scratch = RenderScratch::new();

    // Drive playback via TrackerEngine (no wrap = stop at end)
    let mut engine = TrackerEngine::new(song, false);
    let num_channels = song.channels.max(1);
    let mut active_notes: Vec<Option<u8>> = vec![None; num_channels];
    let mut frames_per_tick = (sr * engine.seconds_per_tick(song)) as usize;

    while !engine.finished {
        // Snapshot timing before tick 0 advances the row
        if engine.tick == 0 {
            frames_per_tick = (sr * engine.seconds_per_tick(song)) as usize;
        }

        engine.process_tick(song);
        let events = engine.drain_events();

        // Dispatch engine events to synth/sample engines
        for event in &events {
            match event {
                TrackerEvent::NoteOn {
                    channel,
                    midi_note,
                    velocity,
                    instrument,
                } => {
                    let midi_ch = (*channel & 0x0F) as u8;
                    // Turn off previous note on this channel
                    if let Some(prev) = active_notes.get(*channel).copied().flatten() {
                        synth.note_off(midi_ch, prev);
                        sample_engine.note_off(midi_ch, prev);
                    }
                    while active_notes.len() <= *channel {
                        active_notes.push(None);
                    }
                    // Route based on instrument
                    let inst_idx = instrument.unwrap_or(0) as usize;
                    let inst = instruments.get(inst_idx);
                    let has_sample = inst
                        .and_then(|i| i.sample_index)
                        .and_then(|idx| bank.get(idx))
                        .is_some();
                    if has_sample {
                        let sample_idx = inst.unwrap().sample_index.unwrap();
                        let sample = bank.get(sample_idx).unwrap();
                        sample_engine.note_on(
                            sample_idx,
                            *midi_note,
                            *velocity,
                            midi_ch,
                            sample,
                            sr,
                            NewNoteAction::Cut,
                        );
                    } else if let Some(sp) = inst.and_then(|i| i.synth_params.as_ref()) {
                        synth.note_on_with_params(midi_ch, *midi_note, *velocity, sp);
                    } else {
                        synth.note_on(midi_ch, *midi_note, *velocity);
                    }
                    active_notes[*channel] = Some(*midi_note);
                }
                TrackerEvent::NoteOff { channel } => {
                    let midi_ch = (*channel & 0x0F) as u8;
                    if let Some(prev) = active_notes.get(*channel).copied().flatten() {
                        synth.note_off(midi_ch, prev);
                        sample_engine.note_off(midi_ch, prev);
                    }
                    if *channel < active_notes.len() {
                        active_notes[*channel] = None;
                    }
                }
                TrackerEvent::PitchBend {
                    channel,
                    semitone_offset,
                } => {
                    let midi_ch = (*channel & 0x0F) as u8;
                    synth.set_channel_pitch_offset(midi_ch, *semitone_offset as f32);
                    sample_engine.set_channel_pitch_offset(midi_ch, *semitone_offset, bank, sr);
                }
                TrackerEvent::VolumeChange { channel, volume } => {
                    let midi_ch = (*channel & 0x0F) as u8;
                    synth.set_channel_volume(midi_ch, *volume);
                    sample_engine.set_channel_volume(midi_ch, *volume);
                }
                TrackerEvent::ProgramChange { channel, program } => {
                    let midi_ch = (*channel & 0x0F) as u8;
                    synth.program_change(midi_ch, *program);
                }
                _ => {} // RowAdvanced, SpeedChanged, TempoChanged, GenerationAdvanced handled by engine
            }
        }

        // Render audio for this tick
        render_block(
            &mut scratch,
            frames_per_tick,
            per_channel,
            &mut synth,
            &mut sample_engine,
            bank,
            &mut channel_effects,
            &mut send_buses,
            &mut master_fx,
        );
        sink(&scratch.left, &scratch.right)?;
    }

    // Turn off all remaining notes and render a short tail for reverb/release
    synth.note_off_all();
    sample_engine.note_off_all();
    let tail_frames = (sr * 2.0) as usize;
    render_block(
        &mut scratch,
        tail_frames,
        per_channel,
        &mut synth,
        &mut sample_engine,
        bank,
        &mut channel_effects,
        &mut send_buses,
        &mut master_fx,
    );
    sink(&scratch.left, &scratch.right)?;

    Ok(())
}

/// Render an entire song to stereo f32 buffers (offline, non-real-time).
///
/// For callers that need the whole render at once; FLAC encoding does,
/// since the encoder takes its input as a single source.
fn render_song(
    song: &Song,
    bank: &SampleBank,
    instruments: &[ExportInstrument],
    channel_fx_params: &[ChannelEffectsParams],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let mut all_left = Vec::new();
    let mut all_right = Vec::new();
    render_song_streaming(
        song,
        bank,
        instruments,
        channel_fx_params,
        send_bus_params,
        sample_rate,
        &mut |left, right| {
            all_left.extend_from_slice(left);
            all_right.extend_from_slice(right);
            Ok(())
        },
    )?;
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
    let (left, right) = render_song(
        song,
        bank,
        instruments,
        channel_fx_params,
        send_bus_params,
        sample_rate,
    )?;
    let samples_i16 = to_interleaved_i16(&left, &right);
    write_flac(path, &samples_i16, sample_rate)
}

fn write_flac(path: &Path, samples_i16: &[i16], sample_rate: u32) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let config = flacenc::config::Encoder::default()
        .into_verified()
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
    flac_stream
        .write(&mut bw)
        .map_err(|e| anyhow::anyhow!("FLAC write failed: {:?}", e))?;
    std::io::Write::write_all(&mut file, bw.as_slice()).context("Failed to write FLAC data")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::*;
    use crate::tracker::{Cell, Note, NoteValue, Song};

    #[test]
    fn test_render_empty_song() {
        let song = Song::new(4, 64);
        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument {
                sample_index: None,
                midi_program: 0,
                synth_params: None,
            })
            .collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_empty.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok(), "WAV export failed: {:?}", result.err());

        // Verify the file exists and is a valid WAV
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 44100);
    }

    #[test]
    fn test_render_with_synth_note() {
        let mut song = Song::new(1, 2);
        song.speed = 2;
        song.set_cell(
            0,
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
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument {
                sample_index: None,
                midi_program: 0,
                synth_params: None,
            })
            .collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_synth.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok(), "WAV export failed: {:?}", result.err());

        // Should have some audio content (not all silence)
        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for synth note");
    }

    #[test]
    fn test_render_with_sample() {
        let mut song = Song::new(1, 2);
        song.speed = 2;
        song.set_cell(
            0,
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
        bank.samples[0] = Some(std::sync::Arc::new(super::super::Sample {
            name: "sine".into(),
            data: sample_data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }));

        let instruments: Vec<ExportInstrument> = {
            let mut v: Vec<ExportInstrument> = (0..256)
                .map(|_| ExportInstrument {
                    sample_index: None,
                    midi_program: 0,
                    synth_params: None,
                })
                .collect();
            v[0].sample_index = Some(0); // instrument 0 -> sample 0
            v
        };
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_sample.wav");

        let result = render_to_wav(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok(), "WAV export failed: {:?}", result.err());

        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for sample note");
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
        song.set_cell(
            0,
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
        // Row 1: portamento up (1xx with param 0x40 = fast slide)
        song.set_cell(
            0,
            1,
            0,
            Cell {
                effect: Some(EFFECT_PORTA_UP),
                effect_value: Some(0x40),
                ..Cell::default()
            },
        );

        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument {
                sample_index: None,
                midi_program: 0,
                synth_params: None,
            })
            .collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path_with = dir.path().join("rtrack_test_porta.wav");

        render_to_wav(&path_with, &song, &bank, &instruments, &[], &[], 44100).unwrap();

        // Now render without the effect for comparison
        let mut song_no_fx = Song::new(1, 4);
        song_no_fx.speed = 6;
        song_no_fx.bpm = 120;
        song_no_fx.set_cell(
            0,
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

        let path_without = dir.path().join("rtrack_test_no_porta.wav");
        render_to_wav(
            &path_without,
            &song_no_fx,
            &bank,
            &instruments,
            &[],
            &[],
            44100,
        )
        .unwrap();

        // Read both and compare -- they should differ
        let r1 = hound::WavReader::open(&path_with).unwrap();
        let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let r2 = hound::WavReader::open(&path_without).unwrap();
        let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

        // Both should have audio
        assert!(
            s1.iter().any(|&s| s.abs() > 10),
            "porta render should have audio"
        );
        assert!(
            s2.iter().any(|&s| s.abs() > 10),
            "no-fx render should have audio"
        );

        // Samples should differ (portamento shifted pitch)
        let min_len = s1.len().min(s2.len());
        let diff_count = s1[..min_len]
            .iter()
            .zip(&s2[..min_len])
            .filter(|(a, b)| a != b)
            .count();
        assert!(diff_count > min_len / 4, "Expected portamento to produce audibly different output, but only {}/{} samples differed", diff_count, min_len);
    }

    #[test]
    fn test_render_volume_slide() {
        // Verify volume slide actually changes the output level
        let mut song = Song::new(1, 4);
        song.speed = 6;
        song.bpm = 120;
        // Row 0: note on at full volume
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                volume: Some(127),
                ..Cell::default()
            },
        );
        // Row 1: volume slide down (50F = slide down by 15 per tick)
        song.set_cell(
            0,
            1,
            0,
            Cell {
                effect: Some(EFFECT_VOLUME_SLIDE),
                effect_value: Some(0x0F),
                ..Cell::default()
            },
        );

        let bank = SampleBank::new();
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument {
                sample_index: None,
                midi_program: 0,
                synth_params: None,
            })
            .collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        // Render with volume slide
        let path_slide = dir.path().join("rtrack_test_volslide.wav");
        render_to_wav(&path_slide, &song, &bank, &instruments, &[], &[], 44100).unwrap();

        // Render without (static volume)
        let mut song_static = Song::new(1, 4);
        song_static.speed = 6;
        song_static.bpm = 120;
        song_static.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 5,
                }),
                volume: Some(127),
                ..Cell::default()
            },
        );
        let path_static = dir.path().join("rtrack_test_volstatic.wav");
        render_to_wav(
            &path_static,
            &song_static,
            &bank,
            &instruments,
            &[],
            &[],
            44100,
        )
        .unwrap();

        let r1 = hound::WavReader::open(&path_slide).unwrap();
        let s1: Vec<i16> = r1.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let r2 = hound::WavReader::open(&path_static).unwrap();
        let s2: Vec<i16> = r2.into_samples::<i16>().map(|s| s.unwrap()).collect();

        // Both should have audio
        assert!(s1.iter().any(|&s| s.abs() > 10));
        assert!(s2.iter().any(|&s| s.abs() > 10));

        // Volume-slid version should differ from static
        let min_len = s1.len().min(s2.len());
        let diff_count = s1[..min_len]
            .iter()
            .zip(&s2[..min_len])
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            diff_count > 0,
            "Volume slide should produce different output than static volume"
        );
    }

    #[test]
    fn test_render_to_flac() {
        let mut song = Song::new(1, 2);
        song.speed = 2;
        song.set_cell(
            0,
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
        let instruments: Vec<ExportInstrument> = (0..256)
            .map(|_| ExportInstrument {
                sample_index: None,
                midi_program: 0,
                synth_params: None,
            })
            .collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_export.flac");

        let result = render_to_flac(&path, &song, &bank, &instruments, &[], &[], 44100);
        assert!(result.is_ok(), "FLAC export failed: {:?}", result.err());

        // Verify the file exists and is non-empty
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0, "FLAC file should be non-empty");
    }
}
