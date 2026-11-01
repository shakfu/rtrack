use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use super::playback::{NewNoteAction, SamplePlaybackEngine};
use super::SampleBank;
use crate::audio::channel_effects::{ChannelEffects, MAX_EFFECT_CHANNELS};
use crate::audio::effects::{self, EffectsChain};
use crate::audio::soft_clip;
use crate::audio::synth::{BuiltinSynth, SynthParams};
use crate::constants::{DEFAULT_PITCH_BEND_RANGE, PITCH_BEND_CENTER, PITCH_BEND_MAX};
use crate::engine::{ExportClock, ExportLength, TrackerEngine, TrackerEvent};
use crate::routing::{route, Route};
use crate::tracker::Song;
use crate::types::{ChannelConfig, Instrument};

/// Render an entire song to a WAV file (offline, non-real-time).
///
/// Notes route as in live playback (`crate::routing`). With `sound_font`,
/// built-in MIDI voices play it, as they do live with one loaded.
#[allow(clippy::too_many_arguments)]
pub fn render_to_wav(
    path: &Path,
    song: &Song,
    bank: &SampleBank,
    instruments: &[Instrument],
    sound_font: Option<&Arc<SoundFont>>,
    channels: &[ChannelConfig],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
    length: ExportLength,
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
        sound_font,
        channels,
        send_bus_params,
        sample_rate,
        length,
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
        let l = soft_clip(left[i]);
        let r = soft_clip(right[i]);
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
    sf2: Option<&mut Synthesizer>,
    sample_engine: &mut SamplePlaybackEngine,
    bank: &SampleBank,
    channel_effects: &mut [ChannelEffects],
    send_buses: &mut [effects::SendBus],
    master_fx: &mut EffectsChain,
) {
    scratch.prepare(frames, per_channel);

    // SF2 always renders to the master, as it does live: it cannot be
    // separated by channel.
    if let Some(sf2) = sf2 {
        sf2.render(&mut scratch.left, &mut scratch.right);
    }

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
    instruments: &[Instrument],
    sound_font: Option<&Arc<SoundFont>>,
    channels: &[ChannelConfig],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
    length: ExportLength,
    sink: &mut BlockSink<'_>,
) -> Result<()> {
    let sr = sample_rate as f64;

    // Create offline audio components
    let mut synth = BuiltinSynth::new(sr);
    let mut sf2 = match sound_font {
        Some(font) => Some(
            Synthesizer::new(font, &SynthesizerSettings::new(sample_rate as i32))
                .map_err(|e| anyhow::anyhow!("Failed to create synthesizer: {:?}", e))?,
        ),
        None => None,
    };
    let mut sample_engine = SamplePlaybackEngine::new(32);
    let mut master_fx = EffectsChain::new(sr);

    let mut channel_effects: Vec<ChannelEffects> = (0..MAX_EFFECT_CHANNELS)
        .map(|ch| {
            let mut fx = ChannelEffects::new(sr);
            if let Some(cfg) = channels.get(ch) {
                fx.params = cfg.effects_params.clone();
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

    let mut engine = TrackerEngine::new(song, true);
    engine.set_channel_info(crate::engine::ChannelInfo::for_export(channels));
    engine.set_instrument_mods(instruments.iter().map(|i| i.mods).collect());
    let num_channels = song.channels.max(1);
    let mut active_notes: Vec<Vec<u8>> = vec![Vec::new(); num_channels];
    let mut frames_per_tick = (sr * engine.seconds_per_tick(song)) as usize;

    // The built-in synth, samples and track effects play by track; the SF2
    // plays by the track's MIDI channel, as in live playback.
    let sf2_channel = |ch: usize| {
        channels
            .get(ch)
            .map_or((ch & 0x0F) as u8, |c| c.midi_channel & 0x0F)
    };

    let mut clock = ExportClock::new(length);
    while !clock.done(&engine)? {
        // Snapshot timing before tick 0 advances the row
        if engine.tick == 0 {
            frames_per_tick = (sr * engine.seconds_per_tick(song)) as usize;
        }
        clock.advance(frames_per_tick as f64 / sr);

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
                    sample_offset,
                    chord,
                } => {
                    let voice = (*channel & 0x0F) as u8;
                    let sf2_ch = sf2_channel(*channel);
                    // Turn off the previous notes on this channel
                    while active_notes.len() <= *channel {
                        active_notes.push(Vec::new());
                    }
                    for prev in std::mem::take(&mut active_notes[*channel]) {
                        synth.note_off(voice, prev);
                        sample_engine.note_off(voice, prev);
                    }
                    let extras = || chord.iter().flatten().copied();
                    let inst = instruments.get(instrument.unwrap_or(0) as usize);
                    // `Some(program)` for a built-in voice; `Some(None)` keeps
                    // the channel's current program.
                    let builtin = match route(inst, instrument.is_some(), bank) {
                        Route::Sample(sample_idx, fx) => {
                            if let Some(sample) = bank.get(sample_idx) {
                                sample_engine.note_on_with_offset(
                                    sample_idx,
                                    *midi_note,
                                    *velocity,
                                    voice,
                                    sample,
                                    sr,
                                    NewNoteAction::Cut,
                                    *sample_offset,
                                    &fx,
                                );
                                for n in extras() {
                                    sample_engine.note_on_with_offset(
                                        sample_idx,
                                        n,
                                        *velocity,
                                        voice,
                                        sample,
                                        sr,
                                        NewNoteAction::Continue,
                                        *sample_offset,
                                        &fx,
                                    );
                                }
                            }
                            None
                        }
                        Route::Synth(sp) => {
                            synth.note_on_with_params(voice, *midi_note, *velocity, sp);
                            for n in extras() {
                                synth.add_note_with_params(voice, n, *velocity, sp);
                            }
                            None
                        }
                        // No MIDI port here, so an external voice plays built in.
                        Route::Preset(program) | Route::External { program, .. } => {
                            Some(Some(program))
                        }
                        Route::ChannelVoice => Some(None),
                        Route::Silent => None,
                    };
                    // SF2 when loaded, else the built-in synth, as live
                    // (`AudioCommand::NoteOn`, `Sf2ProgramNoteOn`).
                    match (builtin, sf2.as_mut()) {
                        (Some(program), Some(sf2)) => {
                            let ch = sf2_ch as i32;
                            if let Some(program) = program {
                                sf2.process_midi_message(ch, 0xC0, (program & 0x7F) as i32, 0);
                            }
                            sf2.note_off_all_channel(ch, false);
                            sf2.note_on(ch, *midi_note as i32, *velocity as i32);
                            for n in extras() {
                                sf2.note_on(ch, n as i32, *velocity as i32);
                            }
                        }
                        (Some(Some(program)), None) => {
                            let params = SynthParams::from_patch(program);
                            synth.note_on_with_params(voice, *midi_note, *velocity, &params);
                            for n in extras() {
                                synth.add_note_with_params(voice, n, *velocity, &params);
                            }
                        }
                        (Some(None), None) => {
                            synth.note_on(voice, *midi_note, *velocity);
                            for n in extras() {
                                synth.add_note(voice, n, *velocity);
                            }
                        }
                        (None, _) => {}
                    }
                    active_notes[*channel] = std::iter::once(*midi_note).chain(extras()).collect();
                }
                TrackerEvent::NoteOff { channel } => {
                    let voice = (*channel & 0x0F) as u8;
                    let sf2_ch = sf2_channel(*channel);
                    if let Some(notes) = active_notes.get_mut(*channel) {
                        for prev in std::mem::take(notes) {
                            synth.note_off(voice, prev);
                            sample_engine.note_off(voice, prev);
                        }
                    }
                    if let Some(sf2) = sf2.as_mut() {
                        sf2.note_off_all_channel(sf2_ch as i32, false);
                    }
                }
                TrackerEvent::PitchBend {
                    channel,
                    semitone_offset,
                } => {
                    let voice = (*channel & 0x0F) as u8;
                    let sf2_ch = sf2_channel(*channel);
                    synth.set_channel_pitch_offset(voice, *semitone_offset as f32);
                    sample_engine.set_channel_pitch_offset(voice, *semitone_offset, bank, sr);
                    if let Some(sf2) = sf2.as_mut() {
                        // The bend range of the playing instrument, as live.
                        let range = engine
                            .channel_states
                            .get(*channel)
                            .and_then(|cs| cs.active_instrument)
                            .and_then(|i| instruments.get(i as usize))
                            .and_then(|i| i.pitch_bend_range)
                            .unwrap_or(DEFAULT_PITCH_BEND_RANGE);
                        let bend = (semitone_offset * PITCH_BEND_CENTER as f64 / range) as i32;
                        let value =
                            (PITCH_BEND_CENTER as i32 + bend).clamp(0, PITCH_BEND_MAX as i32);
                        sf2.process_midi_message(
                            sf2_ch as i32,
                            0xE0,
                            value & 0x7F,
                            (value >> 7) & 0x7F,
                        );
                    }
                }
                TrackerEvent::VolumeChange { channel, volume } => {
                    let voice = (*channel & 0x0F) as u8;
                    let sf2_ch = sf2_channel(*channel);
                    synth.set_channel_volume(voice, *volume);
                    sample_engine.set_channel_volume(voice, *volume);
                    if let Some(sf2) = sf2.as_mut() {
                        sf2.process_midi_message(sf2_ch as i32, 0xB0, 7, *volume as i32);
                    }
                }
                // As live (`AudioCommand::SendCC`), a CC reaches only the SF2.
                TrackerEvent::MidiCC {
                    channel,
                    controller,
                    value,
                } => {
                    if let Some(sf2) = sf2.as_mut() {
                        sf2.process_midi_message(
                            sf2_channel(*channel) as i32,
                            0xB0,
                            *controller as i32,
                            *value as i32,
                        );
                    }
                }
                TrackerEvent::Timbre { channel, timbre } => {
                    let voice = (*channel & 0x0F) as u8;
                    synth.set_channel_timbre(voice, *timbre);
                    sample_engine.set_channel_timbre(voice, *timbre);
                }
                TrackerEvent::ProgramChange { channel, program } => {
                    let voice = (*channel & 0x0F) as u8;
                    let sf2_ch = sf2_channel(*channel);
                    synth.program_change(voice, *program);
                    if let Some(sf2) = sf2.as_mut() {
                        sf2.process_midi_message(sf2_ch as i32, 0xC0, *program as i32, 0);
                    }
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
            sf2.as_mut(),
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
    if let Some(sf2) = sf2.as_mut() {
        sf2.note_off_all(false);
    }
    let tail_frames = (sr * 2.0) as usize;
    render_block(
        &mut scratch,
        tail_frames,
        per_channel,
        &mut synth,
        sf2.as_mut(),
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
#[allow(clippy::too_many_arguments)]
fn render_song(
    song: &Song,
    bank: &SampleBank,
    instruments: &[Instrument],
    sound_font: Option<&Arc<SoundFont>>,
    channels: &[ChannelConfig],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
    length: ExportLength,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let mut all_left = Vec::new();
    let mut all_right = Vec::new();
    render_song_streaming(
        song,
        bank,
        instruments,
        sound_font,
        channels,
        send_bus_params,
        sample_rate,
        length,
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
        let l = soft_clip(left[i]);
        let r = soft_clip(right[i]);
        samples.push(l.to_sample::<i16>());
        samples.push(r.to_sample::<i16>());
    }
    samples
}

/// Render an entire song to a FLAC file (offline, non-real-time).
#[allow(clippy::too_many_arguments)]
pub fn render_to_flac(
    path: &Path,
    song: &Song,
    bank: &SampleBank,
    instruments: &[Instrument],
    sound_font: Option<&Arc<SoundFont>>,
    channels: &[ChannelConfig],
    send_bus_params: &[effects::SendBusParams],
    sample_rate: u32,
    length: ExportLength,
) -> Result<()> {
    let (left, right) = render_song(
        song,
        bank,
        instruments,
        sound_font,
        channels,
        send_bus_params,
        sample_rate,
        length,
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
    use crate::tracker::{Cell, Fx, FxCmd, Note, NoteValue, Song};

    /// Four rows at speed 6 and 120bpm: 24 ticks of 918 frames. With `jump`,
    /// the last row loops back to the start.
    fn four_row_song(jump: bool) -> Song {
        let mut song = Song::new(1, 4);
        if jump {
            let mut fx = [None; crate::tracker::FX_COLUMNS];
            fx[0] = Some(Fx::new(FxCmd::Jump, 0));
            song.set_cell(
                0,
                3,
                0,
                Cell {
                    fx,
                    ..Cell::default()
                },
            );
        }
        song
    }

    #[test]
    fn an_export_plays_one_full_pass_even_when_the_song_loops() {
        let body = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
            .len()
                - 2 * 44100
        };
        let pass = 24 * 918;
        assert_eq!(body(&four_row_song(true)), pass, "backward JMP");
        assert_eq!(body(&four_row_song(false)), pass, "end of order list");
    }

    #[test]
    fn test_render_empty_song() {
        let song = Song::new(4, 64);
        let bank = SampleBank::new();
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_empty.wav");

        let result = render_to_wav(
            &path,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        );
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
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_synth.wav");

        let result = render_to_wav(
            &path,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        );
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
            sliced_from: None,
        }));

        let instruments: Vec<Instrument> = {
            let mut v: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
            v[0].kind = crate::types::InstrumentKind::sample(0); // instrument 0 -> sample 0
            v
        };
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_sample.wav");

        let result = render_to_wav(
            &path,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        );
        assert!(result.is_ok(), "WAV export failed: {:?}", result.err());

        let reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        let has_audio = samples.iter().any(|&s| s.abs() > 10);
        assert!(has_audio, "Expected non-silent output for sample note");
    }

    #[test]
    fn test_render_sample_offset_skips_into_the_sample() {
        // Silence, then a tone. Starting at 9 80 skips the silence.
        let render = |offset: Option<u8>| -> Vec<f32> {
            let mut song = Song::new(1, 1);
            song.speed = 1;
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
                    instrument: Some(0),
                    fx: [offset.map(|o| Fx::new(FxCmd::Offset, o)), None],
                },
            );
            let data: Vec<[f32; 2]> = (0..8000)
                .map(|i| {
                    let v = if i < 4000 {
                        0.0
                    } else {
                        (i as f32 / 44100.0 * 440.0 * std::f32::consts::TAU).sin() * 0.5
                    };
                    [v, v]
                })
                .collect();
            let mut bank = SampleBank::new();
            bank.samples[0] = Some(std::sync::Arc::new(super::super::Sample {
                name: "gap".into(),
                data: data.into(),
                sample_rate: 44100.0,
                base_note: 60,
                trim_start: 0,
                trim_end: 0,
                loop_enabled: false,
                loop_start: 0,
                loop_end: 0,
                source_path: None,
                sliced_from: None,
            }));
            let mut instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
            instruments[0].kind = crate::types::InstrumentKind::sample(0);
            render_song(
                &song,
                &bank,
                &instruments,
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        let peak = |x: &[f32]| x[200..3000].iter().fold(0.0f32, |m, v| m.max(v.abs()));

        let plain = render(None);
        let offset = render(Some(0x80));
        assert!(
            peak(&plain) < 1e-3,
            "the silent half was audible: {}",
            peak(&plain)
        );
        assert!(
            peak(&offset) > 0.05,
            "9 80 did not skip the silence: {}",
            peak(&offset)
        );
    }

    /// A sample instrument's filter and drive shape its render.
    #[test]
    fn cut_moves_a_filtered_sample_and_leaves_an_unfiltered_one() {
        use crate::types::{InstrumentKind, SampleFx, VoiceFilter};
        // A 2205 Hz square: an 8 kHz low-pass keeps its edges, 500 Hz rounds
        // them. (A Nyquist buzz would not do: the bilinear filter has a zero
        // there at any cutoff.)
        let data: Vec<[f32; 2]> = (0..22050)
            .map(|n| {
                let v = if n % 20 < 10 { 0.05 } else { -0.05 };
                [v, v]
            })
            .collect();
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(std::sync::Arc::new(super::super::Sample {
            name: "buzz".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
            sliced_from: None,
        }));
        let render = |fx: SampleFx, cut: Option<u8>| {
            let mut song = Song::new(1, 4);
            let mut fxs = [None; crate::tracker::FX_COLUMNS];
            fxs[0] = cut.map(|p| Fx::new(FxCmd::Cutoff, p));
            song.set_cell(
                0,
                0,
                0,
                Cell {
                    note: Note::from_midi(60),
                    instrument: Some(0),
                    fx: fxs,
                    ..Cell::default()
                },
            );
            let mut instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
            instruments[0].kind = InstrumentKind::Sample { slot: 0, fx };
            render_song(
                &song,
                &bank,
                &instruments,
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        let roughness = |x: &[f32]| {
            x[2000..8000]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .sum::<f32>()
        };
        let filtered = SampleFx {
            filter: Some(VoiceFilter {
                cutoff_hz: 8000.0,
                ..VoiceFilter::default()
            }),
            ..SampleFx::default()
        };
        let open = roughness(&render(filtered, None));
        let closed = roughness(&render(filtered, Some(0x00)));
        assert!(
            closed < open / 3.0,
            "CUT00 closes the filter: {closed} vs {open}"
        );
        assert!(
            render(filtered, Some(0x80)) == render(filtered, None),
            "CUT80 is the instrument's own cutoff"
        );
        let plain = SampleFx::default();
        assert!(
            render(plain, Some(0x00)) == render(plain, None),
            "a sample with no filter has none to move"
        );
    }

    #[test]
    fn cut_closes_a_synth_notes_filter_in_render() {
        let roughness = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0[2000..20000]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .sum::<f32>()
        };
        let plain = roughness(&one_note_song(None));
        let closed = roughness(&one_note_song(Some(Fx::new(FxCmd::Cutoff, 0x00))));
        assert!(closed < plain / 2.0, "CUT00: {closed} vs {plain}");
    }

    #[test]
    fn drv_drives_a_synth_note_in_render() {
        // RMS over peak: drive squares the wave, raising it towards 1.
        let fullness = |song: &Song| {
            let left = render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0;
            let x = &left[2000..20000];
            let rms = (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
            rms / x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        let plain = fullness(&one_note_song(None));
        let driven = fullness(&one_note_song(Some(Fx::new(FxCmd::Drive, 0xFF))));
        // Measured 0.28 -> 0.49.
        assert!(
            driven > plain * 1.4,
            "DRVFF drives the voice: {driven} vs {plain}"
        );
    }

    #[test]
    fn a_sample_instruments_filter_and_drive_reach_the_render() {
        let mut song = Song::new(1, 4);
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Note::from_midi(60),
                instrument: Some(0),
                ..Cell::default()
            },
        );
        // Quiet, with a Nyquist-rate buzz on top of a slow sine.
        let data: Vec<[f32; 2]> = (0..22050)
            .map(|n| {
                let v = 0.04 * (n as f32 * 0.01).sin() + if n % 2 == 0 { 0.01 } else { -0.01 };
                [v, v]
            })
            .collect();
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(std::sync::Arc::new(super::super::Sample {
            name: "buzz".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
            sliced_from: None,
        }));
        let render = |fx: crate::types::SampleFx| {
            let mut instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
            instruments[0].kind = crate::types::InstrumentKind::Sample { slot: 0, fx };
            render_song(
                &song,
                &bank,
                &instruments,
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        let window = 2000..8000;
        // Mean absolute difference between neighbouring frames: high for the buzz.
        let roughness = |x: &[f32]| {
            x[window.clone()]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .sum::<f32>()
                / window.len() as f32
        };
        let peak = |x: &[f32]| x[window.clone()].iter().fold(0.0f32, |m, v| m.max(v.abs()));

        let plain = render(crate::types::SampleFx::default());
        let low_pass = render(crate::types::SampleFx {
            filter: Some(crate::types::VoiceFilter {
                cutoff_hz: 500.0,
                ..crate::types::VoiceFilter::default()
            }),
            ..crate::types::SampleFx::default()
        });
        let driven = render(crate::types::SampleFx {
            drive: 20.0,
            ..crate::types::SampleFx::default()
        });
        assert!(
            roughness(&low_pass) < roughness(&plain) * 0.2,
            "low-pass kept the buzz: {} vs {}",
            roughness(&low_pass),
            roughness(&plain)
        );
        assert!(
            peak(&driven) > peak(&plain) * 3.0,
            "drive did not raise a quiet sample: {} vs {}",
            peak(&driven),
            peak(&plain)
        );
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
                fx: [Some(Fx::new(FxCmd::PitchUp, 0x40)), None],
                ..Cell::default()
            },
        );

        let bank = SampleBank::new();
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path_with = dir.path().join("rtrack_test_porta.wav");

        render_to_wav(
            &path_with,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        )
        .unwrap();

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
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
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
                fx: [Some(Fx::new(FxCmd::VolSlide, 0x0F)), None],
                ..Cell::default()
            },
        );

        let bank = SampleBank::new();
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        // Render with volume slide
        let path_slide = dir.path().join("rtrack_test_volslide.wav");
        render_to_wav(
            &path_slide,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        )
        .unwrap();

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
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
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

    /// Render one C-4 on instrument `inst` with `instruments`.
    fn render_one_note(inst: Option<u8>, instruments: &[Instrument]) -> Vec<f32> {
        let mut song = Song::new(1, 4);
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Some(Note::On {
                    value: NoteValue::C,
                    octave: 4,
                }),
                instrument: inst,
                ..Cell::default()
            },
        );
        render_song(
            &song,
            &SampleBank::new(),
            instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        )
        .unwrap()
        .0
    }

    fn default_instruments() -> Vec<Instrument> {
        (0..256).map(Instrument::for_slot).collect()
    }

    /// A one-preset SoundFont: a looped 441 Hz square wave on every key.
    fn tiny_sound_font() -> Arc<SoundFont> {
        fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut out = id.to_vec();
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
            out
        }
        fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
            let mut body = kind.to_vec();
            chunks.iter().for_each(|c| body.extend_from_slice(c));
            chunk(b"LIST", &body)
        }
        fn name(n: &str) -> Vec<u8> {
            let mut out = n.as_bytes().to_vec();
            out.resize(20, 0);
            out
        }
        let le16 = |v: u16| v.to_le_bytes().to_vec();
        let le32 = |v: u32| v.to_le_bytes().to_vec();

        // 10 cycles of 100 frames, then the 46 zero frames SF2 requires.
        let frames = 1000u32;
        let mut smpl = Vec::new();
        for i in 0..frames {
            let v: i16 = if i % 100 < 50 { 16000 } else { -16000 };
            smpl.extend_from_slice(&v.to_le_bytes());
        }
        smpl.resize(smpl.len() + 46 * 2, 0);

        let phdr = |n: &str, bag: u16| {
            [
                name(n),
                le16(0),
                le16(0),
                le16(bag),
                le32(0),
                le32(0),
                le32(0),
            ]
            .concat()
        };
        let inst = |n: &str, bag: u16| [name(n), le16(bag)].concat();
        let shdr = |n: &str, start: u32, end: u32, l0: u32, l1: u32, kind: u16| {
            [
                name(n),
                le32(start),
                le32(end),
                le32(l0),
                le32(l1),
                le32(44100),
                vec![69, 0],
                le16(0),
                le16(kind),
            ]
            .concat()
        };
        let pdta = list(
            b"pdta",
            &[
                chunk(b"phdr", &[phdr("square", 0), phdr("EOP", 1)].concat()),
                chunk(b"pbag", &[le16(0), le16(0), le16(1), le16(0)].concat()),
                chunk(b"pmod", &[0; 10]),
                // instrument = 0
                chunk(b"pgen", &[le16(41), le16(0), le16(0), le16(0)].concat()),
                chunk(b"inst", &[inst("square", 0), inst("EOI", 1)].concat()),
                chunk(b"ibag", &[le16(0), le16(0), le16(2), le16(0)].concat()),
                chunk(b"imod", &[0; 10]),
                // sampleModes = loop, then sampleID = 0
                chunk(
                    b"igen",
                    &[le16(54), le16(1), le16(53), le16(0), le16(0), le16(0)].concat(),
                ),
                chunk(
                    b"shdr",
                    &[
                        shdr("square", 0, frames, 100, 900, 1),
                        shdr("EOS", 0, 0, 0, 0, 0),
                    ]
                    .concat(),
                ),
            ],
        );
        let mut body = b"sfbk".to_vec();
        body.extend(list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0])]));
        body.extend(list(b"sdta", &[chunk(b"smpl", &smpl)]));
        body.extend(pdta);
        let bytes = chunk(b"RIFF", &body);
        Arc::new(SoundFont::new(&mut bytes.as_slice()).expect("tiny SoundFont parses"))
    }

    /// Sum of squares of the left channel of a render.
    fn energy(song: &Song, font: Option<&Arc<SoundFont>>) -> f64 {
        let (left, _) = render_song(
            song,
            &SampleBank::new(),
            &default_instruments(),
            font,
            &[],
            &[],
            44100,
            ExportLength::default(),
        )
        .unwrap();
        left.iter().map(|&x| (x as f64) * (x as f64)).sum()
    }

    fn one_note_song(fx: Option<Fx>) -> Song {
        let mut song = Song::new(1, 4);
        let mut fxs = [None; crate::tracker::FX_COLUMNS];
        fxs[0] = fx;
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Note::from_midi(69),
                fx: fxs,
                ..Cell::default()
            },
        );
        song
    }

    #[test]
    fn render_counts_arp_steps_in_scale_degrees() {
        use crate::theory::{Scale, ScaleSetting};
        let render = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        let mut degrees = one_note_song(Some(Fx::new(FxCmd::Arp, 0x24)));
        // The note is A4: A major puts its degrees 2 and 4 at +4 and +7.
        degrees.scale = Some(ScaleSetting::new(NoteValue::A, Scale::Major));
        let semitones = one_note_song(Some(Fx::new(FxCmd::Arp, 0x47)));
        // Not assert_eq: a mismatch would print two whole renders.
        assert!(
            render(&degrees) == render(&semitones),
            "ARP24 in A major is ARP47"
        );
    }

    #[test]
    fn a_chord_renders_louder_than_its_root_on_a_sound_font() {
        let font = tiny_sound_font();
        let root = energy(&one_note_song(None), Some(&font));
        // A fifth and an octave: in phase often enough to add up.
        let chord = energy(
            &one_note_song(Some(Fx::new(FxCmd::Chord, 0x7C))),
            Some(&font),
        );
        assert!(chord > root * 1.5, "{chord} vs {root}");
    }

    #[test]
    fn a_chord_renders_on_the_built_in_synth() {
        let render = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        let root = render(&one_note_song(None));
        let chord = render(&one_note_song(Some(Fx::new(FxCmd::Chord, 0x47))));
        let rms = |x: &[f32]| (x[2000..20000].iter().map(|v| v * v).sum::<f32>()).sqrt();
        assert!(
            rms(&chord) > rms(&root) * 1.3,
            "{} vs {}",
            rms(&chord),
            rms(&root)
        );
    }

    #[test]
    fn a_loaded_sound_font_plays_in_render() {
        let font = tiny_sound_font();
        assert!(energy(&one_note_song(None), Some(&font)) > 1.0);
        assert_eq!(energy(&Song::new(1, 4), Some(&font)), 0.0);
    }

    #[test]
    fn a_pattern_cc_reaches_the_sound_font_in_render() {
        let font = tiny_sound_font();
        let plain = energy(&one_note_song(None), Some(&font));
        // CC 11 (expression) at 0 silences the SF2 voice.
        let muted = energy(&one_note_song(Some(Fx::new(FxCmd::Cc(11), 0))), Some(&font));
        assert!(plain > 1.0);
        assert!(muted < plain * 1e-3, "muted {muted}, plain {plain}");
    }

    #[test]
    fn a_linked_phrase_renders_as_its_notes_written_out() {
        let notes: Vec<Cell> = [60u8, 64, 67]
            .iter()
            .map(|&n| Cell {
                note: Note::from_midi(n),
                instrument: Some(5),
                ..Cell::default()
            })
            .collect();
        let render = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };

        let mut linked = Song::new(1, 6);
        linked.speed = 2;
        linked.phrases.push(crate::tracker::Phrase {
            name: String::new(),
            rows: notes.clone(),
        });
        linked.patterns[0].links = vec![Some(crate::tracker::PhraseLink {
            phrase: 0,
            transpose: 0,
            timing: crate::tracker::PhraseTiming::Aligned,
        })];

        let mut written = Song::new(1, 6);
        written.speed = 2;
        for row in 0..6 {
            written.set_cell(0, row, 0, notes[row % 3]);
        }

        assert_eq!(render(&linked), render(&written));
    }

    #[test]
    fn a_transposed_link_renders_as_its_notes_written_out_transposed() {
        let cell = |n: u8| Cell {
            note: Note::from_midi(n),
            instrument: Some(5),
            ..Cell::default()
        };
        let render = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };

        let mut linked = Song::new(1, 2);
        linked.phrases.push(crate::tracker::Phrase {
            name: String::new(),
            rows: vec![cell(60), cell(64)],
        });
        linked.patterns[0].links = vec![Some(crate::tracker::PhraseLink {
            phrase: 0,
            transpose: 7,
            timing: crate::tracker::PhraseTiming::Aligned,
        })];

        let mut written = Song::new(1, 2);
        written.set_cell(0, 0, 0, cell(67));
        written.set_cell(0, 1, 0, cell(71));

        assert_eq!(render(&linked), render(&written));
    }

    #[test]
    fn an_arpeggio_table_renders_as_arp_does() {
        let song_with = |inst: u8, fx: Option<Fx>| {
            let mut song = Song::new(1, 4);
            song.set_cell(
                0,
                0,
                0,
                Cell {
                    note: Note::from_midi(60),
                    instrument: Some(inst),
                    fx: [fx, None],
                    ..Cell::default()
                },
            );
            song
        };
        let mut table_song = song_with(5, None);
        let mut t = crate::tracker::Table::new(3);
        t.rows[1].transpose = 4;
        t.rows[2].transpose = 7;
        t.loop_from = Some(0);
        table_song.tables.insert(5, t);
        let arp_song = song_with(5, Some(Fx::new(FxCmd::Arp, 0x47)));
        let render = |song: &Song| {
            render_song(
                song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &[],
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        // ARP only runs on the row it is written on; the table runs on, so
        // compare the first row only.
        // Frames per tick truncate before they are multiplied, as the renderer does.
        let row = 6 * (44100.0 * Song::new(1, 4).seconds_per_tick()) as usize;
        assert_eq!(render(&table_song)[..row], render(&arp_song)[..row]);
    }

    /// A note with no instrument column, rendered with `channels`.
    fn render_with_channels(channels: &[ChannelConfig]) -> Vec<f32> {
        let mut song = Song::new(1, 2);
        song.set_cell(
            0,
            0,
            0,
            Cell {
                note: Note::from_midi(60),
                ..Cell::default()
            },
        );
        render_song(
            &song,
            &SampleBank::new(),
            &default_instruments(),
            None,
            channels,
            &[],
            44100,
            ExportLength::default(),
        )
        .unwrap()
        .0
    }

    #[test]
    fn render_honours_track_mute_volume_and_default_instrument() {
        let track = ChannelConfig::new(0);
        let plain = render_with_channels(std::slice::from_ref(&track));
        assert!(plain.iter().any(|s| s.abs() > 1e-6));

        let mut muted = track.clone();
        muted.muted = true;
        assert!(render_with_channels(&[muted])
            .iter()
            .all(|s| s.abs() < 1e-6));

        let mut quiet = track.clone();
        quiet.volume = 0.25;
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak(&render_with_channels(&[quiet])) < peak(&plain));

        // The default instrument fills the empty instrument column, as live.
        let mut defaulted = track;
        defaulted.default_instrument = Some(5);
        assert_ne!(render_with_channels(&[defaulted]), plain);
    }

    #[test]
    fn tracks_sharing_a_midi_channel_do_not_cut_each_others_synth_notes() {
        let mut song = Song::new(2, 2);
        for (track, note) in [(0, 60u8), (1, 67)] {
            song.set_cell(
                0,
                0,
                track,
                Cell {
                    note: Note::from_midi(note),
                    instrument: Some(5),
                    ..Cell::default()
                },
            );
        }
        let render = |midi: [u8; 2]| {
            let channels: Vec<ChannelConfig> =
                midi.iter().map(|&m| ChannelConfig::new(m)).collect();
            render_song(
                &song,
                &SampleBank::new(),
                &default_instruments(),
                None,
                &channels,
                &[],
                44100,
                ExportLength::default(),
            )
            .unwrap()
            .0
        };
        assert_eq!(render([0, 0]), render([0, 1]));
    }

    #[test]
    fn a_preset_instrument_renders_its_preset_as_live_does() {
        let presets = default_instruments();
        let mut written_out = default_instruments();
        written_out[5].kind = crate::types::InstrumentKind::Synth(SynthParams::from_patch(5));

        let preset = render_one_note(Some(5), &presets);
        assert_eq!(preset, render_one_note(Some(5), &written_out));
        // Before shared routing, render played the channel voice instead.
        assert_ne!(preset, render_one_note(None, &presets));
    }

    #[test]
    fn an_external_instrument_renders_as_its_preset() {
        let mut instruments = default_instruments();
        instruments[5].kind = crate::types::InstrumentKind::Midi {
            program: 5,
            channel: None,
            output: crate::types::MidiOutput::External { port: None },
        };
        assert_eq!(
            render_one_note(Some(5), &instruments),
            render_one_note(Some(5), &default_instruments())
        );
    }

    #[test]
    fn a_missing_sample_renders_silence() {
        let mut instruments = default_instruments();
        instruments[0].kind = crate::types::InstrumentKind::sample(3);
        let out = render_one_note(Some(0), &instruments);
        assert!(out.iter().all(|v| *v == 0.0), "a missing sample made sound");
    }

    #[test]
    fn exports_saturate_like_live_output_instead_of_clipping_hard() {
        let samples = to_interleaved_i16(&[0.5, 0.9, 4.0], &[0.5, 0.9, 4.0]);
        assert_eq!(samples[0], (0.5f32).to_sample::<i16>(), "linear below 0.8");
        assert!(samples[2] < (0.9f32).to_sample::<i16>(), "0.9 is softened");
        assert!(samples[4] > 32000, "far over full scale stays near it");
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
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("rtrack_test_export.flac");

        let result = render_to_flac(
            &path,
            &song,
            &bank,
            &instruments,
            None,
            &[],
            &[],
            44100,
            ExportLength::default(),
        );
        assert!(result.is_ok(), "FLAC export failed: {:?}", result.err());

        // Verify the file exists and is non-empty
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0, "FLAC file should be non-empty");
    }

    #[test]
    fn export_length_sets_the_rendered_duration() {
        let song = four_row_song(false);
        let bank = SampleBank::new();
        let instruments: Vec<Instrument> = (0..256).map(Instrument::for_slot).collect();
        let frames = |length| {
            render_song(&song, &bank, &instruments, None, &[], &[], 44100, length)
                .unwrap()
                .0
                .len()
        };
        let pass = 24 * 918;
        let one = frames(ExportLength::Loops(1));
        assert_eq!(frames(ExportLength::Loops(3)) - one, 2 * pass);
        // Seconds ends on the first row boundary at or past the time.
        let body = frames(ExportLength::Seconds(2.0)) - (one - pass);
        assert!((88200..88200 + 6 * 918).contains(&body), "{body}");
    }
}
