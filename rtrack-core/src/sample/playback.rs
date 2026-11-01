use super::{Sample, SampleBank};
use crate::audio::envelope::Envelope;
use crate::audio::svf::{Svf, SvfCoeffs};
use crate::audio::synth::FilterType;
use crate::audio::timbre::{PanGlide, TimbreGlide};
use crate::constants::{
    MIDI_MAX_VALUE, SAMPLE_DECLICK_SECS, SAMPLE_INAUDIBLE_LEVEL, SEMITONES_PER_OCTAVE,
};
use crate::types::{LoopMode, SampleFx, Timbre};

/// Type alias for backward compatibility (previously a separate struct)
pub type SampleEnvelope = Envelope;

/// What happens to the voices already sounding on a channel when a new note
/// arrives there.
///
/// A tracker channel is one voice: a note in a pattern row replaces whatever
/// that channel was playing, which is what makes a sliced break sound like a
/// break rather than every slice piling up on top of the last. Notes that do
/// not come from a pattern row -- previewing a sample, playing a chord in on
/// a MIDI keyboard -- are not bound by that and stack instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewNoteAction {
    /// Fade out whatever the channel was playing (tracker behaviour).
    Cut,
    /// Leave the other voices on the channel sounding.
    Continue,
}

/// A voice's drive and filter, set up from its instrument's [`SampleFx`]
/// at note-on and moved by its channel's [`Timbre`]. Values from a file are
/// clamped here, where they reach audio.
#[derive(Debug, Clone, Copy)]
pub struct VoiceFx {
    /// The instrument's drive; 1.0 or less is off.
    drive: f32,
    /// Kind, the instrument's cutoff and resonance, and the coefficients
    /// for them under `applied`.
    filter: Option<(FilterType, f32, f32, SvfCoeffs)>,
    svf: [Svf; 2],
    glide: TimbreGlide,
    /// The timbre `drive` and the coefficients were last worked out for.
    applied: Timbre,
    /// Frames since the coefficients were last worked out.
    frame: u8,
    output_rate: f32,
}

/// Frames between coefficient updates while a timbre glides.
const TIMBRE_UPDATE_FRAMES: u8 = 16;

impl VoiceFx {
    pub fn new(fx: &SampleFx, output_rate: f64) -> Self {
        Self::with_timbre(fx, output_rate, Timbre::default())
    }

    /// [`Self::new`], starting at the channel's `timbre`.
    pub fn with_timbre(fx: &SampleFx, output_rate: f64, timbre: Timbre) -> Self {
        let rate = output_rate as f32;
        // `NaN > 1.0` is false, so a non-number is off.
        let drive = if fx.drive > 1.0 {
            fx.drive.min(SampleFx::MAX_DRIVE)
        } else {
            1.0
        };
        let filter = fx.filter.map(|f| {
            let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
            let cutoff = finite(f.cutoff_hz, 20000.0).clamp(20.0, 20000.0);
            let resonance = finite(f.resonance, 0.0);
            (
                f.kind,
                cutoff,
                resonance,
                Self::coeffs(cutoff, resonance, timbre, rate),
            )
        });
        Self {
            drive,
            filter,
            svf: [Svf::default(); 2],
            glide: TimbreGlide::at(timbre, rate),
            applied: timbre,
            frame: 0,
            output_rate: rate,
        }
    }

    fn coeffs(cutoff: f32, resonance: f32, timbre: Timbre, rate: f32) -> SvfCoeffs {
        let cutoff = if timbre.cutoff == 0.0 {
            cutoff
        } else {
            (cutoff * timbre.cutoff.exp2()).clamp(20.0, 20000.0)
        };
        SvfCoeffs::new(cutoff, resonance + timbre.resonance, rate)
    }

    /// Glide towards `timbre`.
    pub fn set_timbre(&mut self, timbre: Timbre) {
        self.glide.set_target(timbre);
    }

    /// Drive, then filter, one stereo frame.
    #[inline]
    fn process(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        let timbre = self.glide.advance();
        if timbre != self.applied {
            self.frame += 1;
            if self.frame >= TIMBRE_UPDATE_FRAMES || self.glide.settled() {
                self.frame = 0;
                self.applied = timbre;
                if let Some((_, cutoff, resonance, c)) = &mut self.filter {
                    *c = Self::coeffs(*cutoff, *resonance, timbre, self.output_rate);
                }
            }
        }
        let drive = self.applied.scale_drive(self.drive);
        if drive > 1.0 {
            l = (l * drive).tanh();
            r = (r * drive).tanh();
        }
        if let Some((kind, _, _, c)) = &self.filter {
            l = self.svf[0].tick_kind(c, *kind, l);
            r = self.svf[1].tick_kind(c, *kind, r);
        }
        (l, r)
    }
}

/// A single playing voice (one sample instance)
pub struct SampleVoice {
    pub sample_index: usize,
    /// Current fractional frame position within the sample
    pub position: f64,
    /// Playback rate: 1.0 = original pitch, 2.0 = one octave up, 0.5 = one octave down.
    /// Incorporates both pitch shifting (note vs base_note) and sample rate conversion.
    pub rate: f64,
    /// Audio output sample rate, kept so the de-click ramp can be a fixed
    /// duration in real time regardless of how fast the voice reads frames.
    pub output_rate: f64,
    pub velocity: f32,
    pub channel: u8,
    pub note: u8,
    pub active: bool,
    pub envelope: SampleEnvelope,
    /// Whether the voice has wrapped from the loop end back to its start at
    /// least once. Until then, the frame before the loop start is audio that
    /// really did play just before it.
    pub looped: bool,
    /// Whether a reverse or ping-pong loop is reading backwards.
    pub reversed: bool,
    pub fx: VoiceFx,
    pan: PanGlide,
}

impl SampleVoice {
    /// How loud this voice currently is, for choosing which one to steal.
    fn level(&self) -> f32 {
        self.envelope.level * self.velocity
    }

    /// Whether this voice still counts against the polyphony limit.
    ///
    /// A voice that has been released is on its way out and will free its
    /// slot shortly, so it does not block a new note.
    fn is_sounding(&self) -> bool {
        self.active && self.envelope.stage != crate::audio::envelope::EnvStage::Release
    }

    /// Retire this voice quickly to make room for another.
    ///
    /// Removing it outright would leave whatever sample value it was at as a
    /// step in the mix, which is heard as a click; a fast release fades it
    /// out over a few milliseconds instead.
    fn steal(&mut self) {
        self.envelope.release = SAMPLE_DECLICK_SECS;
        self.envelope.release();
    }
}

/// Cubic Hermite interpolation between 4 points.
/// `t` is the fractional position between p1 and p2 (0..1).
#[inline]
fn cubic_hermite(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0 + 0.5 * p2;
    let d = p1;
    ((a * t + b) * t + c) * t + d
}

/// The frame the interpolator reads for index `i` of a looping voice: what
/// plays next in the voice's direction, not the audio past the loop's ends.
#[inline]
fn loop_frame(i: isize, looped: bool, mode: LoopMode, start: usize, end: usize) -> usize {
    let (first, len) = (start as isize, end as isize - start as isize);
    if i < first && !looped {
        return i.max(0) as usize; // audio that really plays before the loop
    }
    if (first..first + len).contains(&i) {
        return i as usize;
    }
    if mode == LoopMode::Forward || (mode == LoopMode::Reverse && looped) {
        return (first + (i - first).rem_euclid(len)) as usize;
    }
    // Reflected about the first and last frames, where the voice turns.
    let period = 2 * (len - 1);
    if period == 0 {
        return start;
    }
    let t = (i - first).rem_euclid(period);
    (first + if t < len { t } else { period - t }) as usize
}

/// Render one voice into `left`/`right` over `range`, mixing additively.
///
/// Shared by [`SamplePlaybackEngine::render`] and
/// [`SamplePlaybackEngine::render_per_channel`], which differ only in where
/// the output goes.
fn render_voice(
    voice: &mut SampleVoice,
    sample: &Sample,
    left: &mut [f32],
    right: &mut [f32],
    range: std::ops::Range<usize>,
    pan: f32,
) {
    let end = sample.end() as f64;
    let loop_start = sample.effective_loop_start() as f64;
    let loop_end = sample.effective_loop_end() as f64;
    let looping = sample.loop_enabled && loop_end > loop_start;
    let (loop_start_frame, loop_end_frame) = (loop_start as usize, loop_end as usize);
    let mode = sample.loop_mode;
    // The loop or its mode may have been edited under a backwards voice.
    if !looping || mode == LoopMode::Forward {
        voice.reversed = false;
    }

    // Length of the tail fade, in source frames. A voice reading at twice
    // the rate covers twice as many frames in the same amount of time, so
    // the frame count scales with the rate to keep the fade a constant
    // duration. It never eats more than half of what plays, so a very short
    // slice is quieter but not swallowed.
    let declick = if looping {
        0.0
    } else {
        (SAMPLE_DECLICK_SECS as f64 * voice.output_rate * voice.rate)
            .min(sample.played_len() as f64 * 0.5)
            .max(1.0)
    };

    for i in range {
        if !voice.active {
            break;
        }

        // Tick the ADSR envelope
        let env_level = voice.envelope.tick();
        if !voice.envelope.is_active() {
            voice.active = false;
            break;
        }

        // Cubic hermite interpolation (4-point)
        let pos = voice.position;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;

        let looped = voice.looped;
        let read = |i: isize| {
            sample.frame_at(if looping {
                loop_frame(i, looped, mode, loop_start_frame, loop_end_frame)
            } else {
                i.max(0) as usize
            })
        };
        let at = idx as isize;
        let fm1 = read(at - 1);
        let f0 = read(at);
        let f1 = read(at + 1);
        let f2 = read(at + 2);

        let l = cubic_hermite(fm1[0], f0[0], f1[0], f2[0], frac);
        let r = cubic_hermite(fm1[1], f0[1], f1[1], f2[1], frac);
        let (l, r) = voice.fx.process(l, r);

        // Fade towards the end of a one-shot. A slice ends at an arbitrary
        // frame, so stopping on it would leave a step in the output.
        let gain = if looping {
            env_level
        } else {
            env_level * (((end - pos) / declick).clamp(0.0, 1.0) as f32)
        };

        let (gain_l, gain_r) = voice.pan.gains(pan);
        left[i] += l * voice.velocity * gain * gain_l;
        right[i] += r * voice.velocity * gain * gain_r;

        // Advance position
        voice.position += if voice.reversed {
            -voice.rate
        } else {
            voice.rate
        };

        // Handle loop or end
        if looping {
            // Modulo the loop length rather than subtracting it once: a voice
            // pitched up far enough to cover the whole loop in a single frame
            // would otherwise walk past the loop end and never come back.
            let len = loop_end - loop_start;
            let wrap = |p: f64| loop_start + (p - loop_start).rem_euclid(len);
            let last = loop_end - 1.0;
            match mode {
                LoopMode::Forward => {
                    if voice.position >= loop_end {
                        voice.position = wrap(voice.position);
                        voice.looped = true;
                    }
                }
                LoopMode::Reverse => {
                    if !voice.reversed && voice.position > last {
                        voice.position = 2.0 * last - voice.position;
                        voice.reversed = true;
                        voice.looped = true;
                    }
                    if voice.reversed && voice.position < loop_start {
                        voice.position = wrap(voice.position);
                    }
                }
                LoopMode::PingPong => {
                    if voice.position > last || (voice.reversed && voice.position < loop_start) {
                        let span = last - loop_start;
                        let period = 2.0 * span;
                        let t = if period > 0.0 {
                            (voice.position - loop_start).rem_euclid(period)
                        } else {
                            0.0
                        };
                        voice.position = loop_start + if t <= span { t } else { period - t };
                        voice.reversed ^= t > span;
                        voice.looped = true;
                    }
                }
            }
        } else if voice.position >= end {
            voice.active = false;
        }
    }
}

/// Manages sample voice allocation and rendering.
pub struct SamplePlaybackEngine {
    pub voices: Vec<SampleVoice>,
    pub max_voices: usize,
    /// Each channel's timbre, which its next voice starts at.
    timbre: [Timbre; 16],
    /// Each channel's pan, -1 left to 1 right.
    pan: [f32; 16],
}

impl SamplePlaybackEngine {
    pub fn new(max_voices: usize) -> Self {
        Self {
            voices: Vec::with_capacity(max_voices),
            max_voices,
            timbre: [Timbre::default(); 16],
            pan: [0.0; 16],
        }
    }

    /// Pan `channel`'s voices, -1 left to 1 right.
    pub fn set_channel_pan(&mut self, channel: u8, pan: f32) {
        self.pan[(channel & 0x0F) as usize] = pan;
    }

    /// Move the filter and drive of `channel`'s voices, and of its next ones.
    pub fn set_channel_timbre(&mut self, channel: u8, timbre: Timbre) {
        self.timbre[(channel & 0x0F) as usize] = timbre;
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                voice.fx.set_timbre(timbre);
            }
        }
    }

    /// Start playing a sample. `output_rate` is the audio output sample rate.
    ///
    /// `action` decides what happens to the channel's existing voices; see
    /// [`NewNoteAction`].
    #[allow(clippy::too_many_arguments)]
    pub fn note_on(
        &mut self,
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        sample: &Sample,
        output_rate: f64,
        action: NewNoteAction,
    ) {
        self.note_on_with_offset(
            sample_index,
            note,
            velocity,
            channel,
            sample,
            output_rate,
            action,
            0,
            &SampleFx::default(),
        );
    }

    /// [`Self::note_on`], starting `offset`/256 of the way into the played
    /// span (`9xx`). Relative to the span rather than a fixed frame count,
    /// so the same parameter lands at the same point in a slice of any length.
    #[allow(clippy::too_many_arguments)]
    pub fn note_on_with_offset(
        &mut self,
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        sample: &Sample,
        output_rate: f64,
        action: NewNoteAction,
        offset: u8,
        fx: &SampleFx,
    ) {
        match action {
            NewNoteAction::Cut => {
                // Fade rather than drop: the outgoing voice is stopped
                // mid-waveform, so cutting it outright would click.
                for voice in &mut self.voices {
                    if voice.active && voice.channel == channel {
                        voice.steal();
                    }
                }
            }
            // Release any voice already playing this note on this channel
            NewNoteAction::Continue => self.note_off(channel, note),
        }

        // Calculate playback rate:
        //   pitch_ratio = 2^((note - base_note) / 12)
        //   rate_ratio  = sample.sample_rate / output_rate
        //   effective_rate = pitch_ratio * rate_ratio
        let pitch_ratio =
            2.0_f64.powf((note as f64 - sample.base_note as f64) / SEMITONES_PER_OCTAVE as f64);
        let rate_ratio = sample.sample_rate / output_rate;
        let rate = pitch_ratio * rate_ratio;
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;

        self.make_room();

        self.voices.push(SampleVoice {
            sample_index,
            position: (sample.trim_start + sample.played_len() * offset as usize / 256) as f64,
            rate,
            output_rate,
            velocity: vel,
            channel,
            note,
            active: true,
            envelope: match fx.env {
                Some(e) => {
                    Envelope::new(e.attack, e.decay, e.sustain, e.release, output_rate as f32)
                }
                None => Envelope::sample_default(output_rate as f32),
            },
            looped: false,
            reversed: false,
            pan: PanGlide::at(self.pan[(channel & 0x0F) as usize], output_rate as f32),
            fx: VoiceFx::with_timbre(fx, output_rate, self.timbre[(channel & 0x0F) as usize]),
        });
    }

    /// Free a voice slot if the pool is full.
    ///
    /// Prefers a voice that has finished, then the quietest one. A quiet
    /// enough voice is dropped outright; anything still audible is faded
    /// rather than cut, so stealing does not click.
    fn make_room(&mut self) {
        if self.voices.iter().filter(|v| v.is_sounding()).count() < self.max_voices {
            // Fading voices are not counted against the limit, but they do
            // occupy the pool, so cap the total as a backstop against a
            // burst of notes outrunning the fades.
            if self.voices.len() < self.max_voices * 2 {
                return;
            }
        }

        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            self.voices.remove(idx);
            return;
        }

        let quietest = self
            .voices
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.level()
                    .partial_cmp(&b.level())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        if let Some(idx) = quietest {
            if self.voices[idx].level() < SAMPLE_INAUDIBLE_LEVEL
                || self.voices.len() >= self.max_voices * 2
            {
                self.voices.remove(idx);
            } else {
                self.voices[idx].steal();
            }
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        for voice in &mut self.voices {
            if voice.channel == channel && voice.note == note && voice.active {
                voice.envelope.release();
            }
        }
    }

    pub fn note_off_channel(&mut self, channel: u8) {
        for voice in &mut self.voices {
            if voice.channel == channel && voice.active {
                voice.envelope.release();
            }
        }
    }

    pub fn note_off_all(&mut self) {
        self.voices.clear();
    }

    /// Adjust pitch offset (in semitones) for all active voices on a channel.
    /// Recalculates the playback rate to reflect the offset.
    pub fn set_channel_pitch_offset(
        &mut self,
        channel: u8,
        semitones: f64,
        bank: &SampleBank,
        output_rate: f64,
    ) {
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                if let Some(sample) = bank.get(voice.sample_index) {
                    let effective_note = voice.note as f64 + semitones;
                    let pitch_ratio = 2.0_f64.powf(
                        (effective_note - sample.base_note as f64) / SEMITONES_PER_OCTAVE as f64,
                    );
                    let rate_ratio = sample.sample_rate / output_rate;
                    voice.rate = pitch_ratio * rate_ratio;
                }
            }
        }
    }

    /// Set volume (velocity 0-127) for all active voices on a channel.
    pub fn set_channel_volume(&mut self, channel: u8, velocity: u8) {
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                voice.velocity = vel;
            }
        }
    }

    /// Render all active voices into left/right buffers (additive mix).
    /// Inactive voices are removed after rendering.
    pub fn render(&mut self, bank: &SampleBank, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());

        self.voices.retain(|v| v.active);

        for voice in &mut self.voices {
            let sample = match bank.get(voice.sample_index) {
                Some(s) => s,
                None => {
                    voice.active = false;
                    continue;
                }
            };
            let pan = self.pan[(voice.channel & 0x0F) as usize];
            render_voice(voice, sample, left, right, 0..frames, pan);
        }
    }

    /// Render active voices into per-channel buffers, writing only the
    /// sample range `range` of each buffer.
    ///
    /// `channel_left`/`channel_right` are indexed by tracker channel. Takes
    /// them separately rather than a slice of `(&mut [f32], &mut [f32])`
    /// pairs: building that pair list required a heap allocation and a
    /// pointer-aliasing `unsafe` block on every audio callback.
    pub fn render_per_channel(
        &mut self,
        bank: &SampleBank,
        channel_left: &mut [Vec<f32>],
        channel_right: &mut [Vec<f32>],
        range: std::ops::Range<usize>,
    ) {
        self.voices.retain(|v| v.active);

        let channel_count = channel_left.len().min(channel_right.len());
        if channel_count == 0 || range.is_empty() {
            return;
        }

        for voice in &mut self.voices {
            let ch = voice.channel as usize;
            if ch >= channel_count {
                // No buffer for this channel. Folding the voice onto the
                // last channel instead would run it through effects meant
                // for other material.
                voice.active = false;
                continue;
            }
            let sample = match bank.get(voice.sample_index) {
                Some(s) => s,
                None => {
                    voice.active = false;
                    continue;
                }
            };
            let pan = self.pan[(voice.channel & 0x0F) as usize];
            render_voice(
                voice,
                sample,
                &mut channel_left[ch],
                &mut channel_right[ch],
                range.clone(),
                pan,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::envelope::EnvStage;
    use crate::sample::Sample;
    use crate::types::AmpEnv;
    use std::sync::Arc;

    fn bank_with(sample: Sample) -> SampleBank {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(sample));
        bank
    }

    fn make_test_sample() -> Sample {
        // 10-frame sine-ish sample at 44100 Hz
        let data: Vec<[f32; 2]> = (0..100)
            .map(|i| {
                let t = i as f32 / 100.0;
                let val = (t * std::f32::consts::TAU).sin();
                [val, val]
            })
            .collect();
        Sample {
            name: "test".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_mode: Default::default(),
            loop_start: 0,
            loop_end: 0,
            source_path: None,
            sliced_from: None,
        }
    }

    fn make_test_bank() -> SampleBank {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(make_test_sample()));
        bank
    }

    #[test]
    fn test_loop_seam_interpolates_across_the_wrap_not_past_it() {
        // Eight whole periods of a 50-frame sine loop seamlessly, and the
        // audio around the loop is a constant 1.0. At a non-integer rate the
        // interpolator reads frames either side of the seam; reading the
        // audio outside the loop bends the waveform once per pass.
        const PERIOD: f64 = 50.0;
        let data: Vec<[f32; 2]> = (0..1000)
            .map(|i| {
                let v = if (100..500).contains(&i) {
                    (std::f64::consts::TAU * (i - 100) as f64 / PERIOD).sin() as f32
                } else {
                    1.0
                };
                [v, v]
            })
            .collect();
        let smp = Sample {
            data: data.into(),
            trim_start: 100,
            loop_enabled: true,
            loop_mode: Default::default(),
            loop_start: 100,
            loop_end: 500,
            ..make_test_sample()
        };
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(smp.clone()));
        let mut engine = SamplePlaybackEngine::new(8);
        engine.note_on(0, 61, 127, 0, &smp, 44100.0, NewNoteAction::Cut);
        let rate = engine.voices[0].rate;

        let frames = 4000;
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        engine.render(&bank, &mut l, &mut r);

        // Past the 2 ms attack, every output frame is the sine at the voice's
        // position. Hermite error on a 50-frame period is under 0.001.
        let worst = (200..frames)
            .map(|n| {
                let ideal = (std::f64::consts::TAU * (n as f64 * rate) / PERIOD).sin();
                (l[n] as f64 - ideal).abs()
            })
            .fold(0.0, f64::max);
        assert!(worst < 0.001, "deviation {worst} at the loop seam");
    }

    #[test]
    fn hostile_voice_fx_values_render_finite_audio() {
        let fx = SampleFx {
            env: None,
            drive: f32::INFINITY,
            filter: Some(crate::types::VoiceFilter {
                kind: FilterType::BandPass,
                cutoff_hz: f32::NAN,
                resonance: 50.0,
            }),
        };
        let smp = make_test_sample();
        let mut engine = SamplePlaybackEngine::new(4);
        engine.note_on_with_offset(0, 60, 127, 0, &smp, 44100.0, NewNoteAction::Cut, 0, &fx);
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(std::sync::Arc::new(smp));
        let (mut l, mut r) = (vec![0.0; 2048], vec![0.0; 2048]);
        engine.render(&bank, &mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 2.0));
    }

    #[test]
    fn test_sample_offset_starts_part_way_into_the_played_span() {
        let mut smp = make_test_sample();
        smp.data = vec![[0.1; 2]; 4000].into();
        smp.trim_start = 1000;
        smp.trim_end = 3000;
        for (offset, start) in [(0x00u8, 1000.0), (0x80, 2000.0), (0xFF, 2992.0)] {
            let mut engine = SamplePlaybackEngine::new(8);
            engine.note_on_with_offset(
                0,
                60,
                127,
                0,
                &smp,
                44100.0,
                NewNoteAction::Cut,
                offset,
                &SampleFx::default(),
            );
            assert_eq!(engine.voices[0].position, start, "offset {offset:02X}");
        }
        // Past the end of a loop, playback wraps into it rather than stopping.
        smp.loop_enabled = true;
        smp.loop_start = 1000;
        smp.loop_end = 1500;
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(smp.clone()));
        let mut engine = SamplePlaybackEngine::new(8);
        engine.note_on_with_offset(
            0,
            60,
            127,
            0,
            &smp,
            44100.0,
            NewNoteAction::Cut,
            0xC0,
            &SampleFx::default(),
        );
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        engine.render(&bank, &mut l, &mut r);
        let pos = engine.voices[0].position;
        assert!(
            (1000.0..1500.0).contains(&pos),
            "position {pos} left the loop"
        );
    }

    #[test]
    fn test_note_on_creates_voice() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        assert_eq!(engine.voices.len(), 1);
        assert!(engine.voices[0].active);
        // Same base_note and sample_rate -> rate should be ~1.0
        assert!((engine.voices[0].rate - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_note_on_pitch_shift() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        // Note 72 = one octave above base_note 60 -> rate should be ~2.0
        engine.note_on(0, 72, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        assert!((engine.voices[0].rate - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_note_off() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        engine.note_off(0, 60);
        // Voice enters release stage (not instantly deactivated)
        assert_eq!(engine.voices[0].envelope.stage, EnvStage::Release);
        // After rendering enough samples, the envelope fades out and voice deactivates
        let mut left = vec![0.0f32; 44100]; // 1 second -- way past the 50ms release
        let mut right = vec![0.0f32; 44100];
        engine.render(&bank, &mut left, &mut right);
        // Voice should be gone after release fades out
        assert!(engine.voices.is_empty() || !engine.voices[0].active);
    }

    #[test]
    fn test_render_produces_audio() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, sample, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 10];
        let mut right = vec![0.0f32; 10];
        engine.render(&bank, &mut left, &mut right);

        // Should have non-zero output (sine wave sample)
        let energy: f32 = left.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "Expected non-zero audio output");
    }

    #[test]
    fn test_render_stops_at_end() {
        let mut bank = SampleBank::new();
        // Short 5-frame sample
        bank.samples[0] = Some(Arc::new(Sample {
            name: "short".into(),
            data: vec![[1.0, 1.0]; 5].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_mode: Default::default(),
            loop_start: 0,
            loop_end: 0,
            source_path: None,
            sliced_from: None,
        }));

        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, sample, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 20];
        let mut right = vec![0.0f32; 20];
        engine.render(&bank, &mut left, &mut right);

        // Voice should deactivate after sample ends
        // After render, inactive voices are retained until next render call
        // Frames beyond sample end should be silent
        assert!(left[10] == 0.0, "Expected silence after sample end");
    }

    #[test]
    fn test_render_loops() {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(Sample {
            name: "loop".into(),
            data: vec![[0.5, 0.5]; 10].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: true,
            loop_mode: Default::default(),
            loop_start: 2,
            loop_end: 8,
            source_path: None,
            sliced_from: None,
        }));

        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, sample, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 100];
        let mut right = vec![0.0f32; 100];
        engine.render(&bank, &mut left, &mut right);

        // Should still be active after 100 frames (looping)
        assert!(engine.voices[0].active);
        // All frames should have output
        assert!(left[99] != 0.0 || right[99] != 0.0);
    }

    #[test]
    fn test_note_off_all() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        engine.note_on(0, 64, 100, 1, sample, 44100.0, NewNoteAction::Continue);
        engine.note_off_all();
        assert_eq!(engine.voices.len(), 0);
    }

    #[test]
    fn test_note_off_channel() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        engine.note_on(0, 64, 100, 1, sample, 44100.0, NewNoteAction::Continue);
        engine.note_off_channel(0);
        // Channel 0 voice should be in release, channel 1 still sustaining
        let ch0 = engine.voices.iter().find(|v| v.channel == 0).unwrap();
        assert_eq!(ch0.envelope.stage, EnvStage::Release);
        let ch1 = engine.voices.iter().find(|v| v.channel == 1).unwrap();
        assert!(ch1.active);
        assert_ne!(ch1.envelope.stage, EnvStage::Release);
    }

    /// A loop shorter than the distance covered in one frame must still
    /// loop. Wrapping by a single subtraction let the voice walk out of the
    /// loop and off the end of the buffer.
    #[test]
    fn test_loop_wraps_when_rate_exceeds_loop_length() {
        let mut bank = SampleBank::new();
        let mut smp = make_test_sample();
        smp.data = vec![[0.5, 0.5]; 2000].into();
        smp.loop_enabled = true;
        smp.loop_start = 100;
        smp.loop_end = 108; // 8 frames, shorter than the rate below
        bank.samples[0] = Some(Arc::new(smp.clone()));

        let mut engine = SamplePlaybackEngine::new(16);
        // Four octaves above base_note 60 -> rate 16, twice the loop length
        engine.note_on(0, 108, 127, 0, &smp, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 200];
        let mut right = vec![0.0f32; 200];
        engine.render(&bank, &mut left, &mut right);

        let pos = engine.voices[0].position;
        assert!(
            (100.0..108.0 + 16.0).contains(&pos),
            "voice left the loop: position {pos} for loop 100..108"
        );
        assert!(engine.voices[0].active, "looping voice stopped");
        assert!(left[199] != 0.0, "looping voice went silent");
    }

    /// A slice ends at an arbitrary frame, so it has to be faded out rather
    /// than dropped, or the mix takes a step.
    #[test]
    fn test_slice_end_fades_out() {
        let mut bank = SampleBank::new();
        let mut smp = make_test_sample();
        // Every frame at full scale: any hard stop is a full-scale step.
        smp.data = vec![[1.0, 1.0]; 1000].into();
        smp.trim_end = 500;
        bank.samples[0] = Some(Arc::new(smp.clone()));

        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, &smp, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 600];
        let mut right = vec![0.0f32; 600];
        engine.render(&bank, &mut left, &mut right);

        let last = left
            .iter()
            .rposition(|&v| v != 0.0)
            .expect("slice produced no audio");
        let step = (left[last] - left[last + 1]).abs();
        assert!(
            step < 0.05,
            "slice ended on a {step} step (frame {last} = {})",
            left[last]
        );
        // The fade must not eat the slice: it is still at full level well
        // before the end.
        assert!(left[300] > 0.9, "fade started too early: {}", left[300]);
    }

    /// Stealing a sounding voice by dropping it steps the mix; it has to
    /// fade instead.
    #[test]
    fn test_voice_steal_fades_rather_than_cuts() {
        let mut bank = SampleBank::new();
        let mut smp = make_test_sample();
        smp.data = vec![[1.0, 1.0]; 44100].into();
        bank.samples[0] = Some(Arc::new(smp.clone()));

        let mut engine = SamplePlaybackEngine::new(2);
        engine.note_on(0, 60, 127, 0, &smp, 44100.0, NewNoteAction::Continue);
        engine.note_on(0, 62, 127, 1, &smp, 44100.0, NewNoteAction::Continue);

        // Render far enough that both voices are at full level.
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        engine.render(&bank, &mut left, &mut right);
        let before = left[511];
        assert!(before > 1.9, "expected two voices at full level: {before}");

        // A third note has to displace one of them.
        engine.note_on(0, 64, 1, 2, &smp, 44100.0, NewNoteAction::Continue);
        let mut left2 = vec![0.0f32; 512];
        let mut right2 = vec![0.0f32; 512];
        engine.render(&bank, &mut left2, &mut right2);

        let step = (before - left2[0]).abs();
        assert!(step < 0.05, "voice steal stepped the mix by {step}");
        // The stolen voice is on its way out, not gone.
        assert_eq!(
            engine.voices.iter().filter(|v| v.is_sounding()).count(),
            2,
            "polyphony limit not held"
        );
    }

    /// A note from a pattern row takes over its channel: a tracker channel
    /// is one voice, so consecutive slices chop rather than pile up.
    #[test]
    fn test_cut_replaces_the_channel_voice() {
        let mut bank = SampleBank::new();
        let mut smp = make_test_sample();
        smp.data = vec![[1.0, 1.0]; 44100].into();
        bank.samples[0] = Some(Arc::new(smp.clone()));

        let mut engine = SamplePlaybackEngine::new(32);
        engine.note_on(0, 60, 127, 0, &smp, 44100.0, NewNoteAction::Cut);
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        engine.render(&bank, &mut left, &mut right);
        let before = left[511];
        assert!(before > 0.9, "first note is not sounding: {before}");

        engine.note_on(0, 64, 127, 0, &smp, 44100.0, NewNoteAction::Cut);
        let mut left2 = vec![0.0f32; 4410]; // 100ms, past the fade
        let mut right2 = vec![0.0f32; 4410];
        engine.render(&bank, &mut left2, &mut right2);

        assert_eq!(
            engine.voices.iter().filter(|v| v.is_sounding()).count(),
            1,
            "the channel is playing more than one note"
        );
        // The outgoing note fades rather than stopping dead.
        let step = (before - left2[0]).abs();
        assert!(step < 0.6, "cut stepped the mix by {step}");
    }

    /// Previews and live MIDI are not pattern rows: a chord stays a chord.
    #[test]
    fn test_continue_leaves_the_channel_polyphonic() {
        let mut bank = SampleBank::new();
        let mut smp = make_test_sample();
        smp.data = vec![[1.0, 1.0]; 44100].into();
        bank.samples[0] = Some(Arc::new(smp.clone()));

        let mut engine = SamplePlaybackEngine::new(32);
        for note in [60, 64, 67] {
            engine.note_on(0, note, 127, 0, &smp, 44100.0, NewNoteAction::Continue);
        }
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        engine.render(&bank, &mut left, &mut right);

        assert_eq!(
            engine.voices.iter().filter(|v| v.is_sounding()).count(),
            3,
            "chord lost notes"
        );
    }

    /// A silent voice can be dropped outright -- there is nothing to fade.
    #[test]
    fn test_inaudible_voice_is_dropped_not_faded() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(4);
        for note in 60..65 {
            engine.note_on(0, note, 100, 0, sample, 44100.0, NewNoteAction::Continue);
        }
        assert_eq!(engine.voices.len(), 4);
    }

    /// A voice on a channel with no output buffer must not be folded onto
    /// another channel, where it would pick up that channel's effects.
    #[test]
    fn test_voice_on_missing_channel_is_dropped() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, sample, 44100.0, NewNoteAction::Continue);
        engine.note_on(0, 60, 127, 7, sample, 44100.0, NewNoteAction::Continue);

        let mut left = vec![vec![0.0f32; 16]; 2];
        let mut right = vec![vec![0.0f32; 16]; 2];
        engine.render_per_channel(&bank, &mut left, &mut right, 0..16);

        assert!(left[0].iter().any(|&v| v != 0.0), "channel 0 is silent");
        assert!(
            left[1].iter().all(|&v| v == 0.0),
            "voice from channel 7 leaked onto channel 1"
        );
    }

    /// Looping a slice must stay inside the slice. With the default loop
    /// points of 0 it would otherwise replay the whole source file.
    #[test]
    fn test_slice_loop_stays_inside_the_slice() {
        let mut bank = SampleBank::new();
        let mut data = vec![[1.0f32, 1.0]; 1000];
        for frame in data.iter_mut().skip(500) {
            *frame = [0.0, 0.0];
        }
        let mut slice = make_test_sample();
        slice.data = data.into();
        slice.trim_start = 500; // the silent half
        slice.trim_end = 1000;
        slice.loop_enabled = true; // loop points left at their defaults
        bank.samples[0] = Some(Arc::new(slice.clone()));

        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, &slice, 44100.0, NewNoteAction::Continue);

        let mut left = vec![0.0f32; 2000];
        let mut right = vec![0.0f32; 2000];
        engine.render(&bank, &mut left, &mut right);

        let leaked: f32 = left.iter().map(|v| v.abs()).sum();
        assert_eq!(
            leaked, 0.0,
            "loop played {leaked} of audio from before the slice"
        );
    }

    #[test]
    fn test_envelope_fade_on_note_off() {
        // Use a looping sample so it doesn't end before we test
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(Sample {
            name: "loop".into(),
            data: vec![[0.5, 0.5]; 1000].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: true,
            loop_mode: Default::default(),
            loop_start: 0,
            loop_end: 1000,
            source_path: None,
            sliced_from: None,
        }));
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 127, 0, sample, 44100.0, NewNoteAction::Continue);

        // Render a few frames to get past attack
        let mut left = vec![0.0f32; 500];
        let mut right = vec![0.0f32; 500];
        engine.render(&bank, &mut left, &mut right);
        let pre_off_energy: f32 = left[200..500].iter().map(|s| s * s).sum();
        assert!(pre_off_energy > 0.0, "Should have audio before note off");

        // Trigger release
        engine.note_off(0, 60);

        // Render enough for release to fully fade (exponential, ~50ms time constant)
        let mut left2 = vec![0.0f32; 22050]; // 500ms -- well past release
        let mut right2 = vec![0.0f32; 22050];
        engine.render(&bank, &mut left2, &mut right2);

        // End should be silence (voice deactivated after envelope reaches < 0.001)
        let tail_energy: f32 = left2[20000..22050].iter().map(|s| s * s).sum();
        assert!(
            tail_energy < 0.001,
            "Expected silence after full release, but tail_energy={}",
            tail_energy
        );
    }

    /// A ramp sample with `mode` looping over frames 10..20, read at one frame
    /// per output frame. Returns the voice's position after each frame.
    fn loop_positions(mode: LoopMode, frames: usize) -> Vec<f64> {
        let mut smp = make_test_sample();
        smp.loop_enabled = true;
        smp.loop_mode = mode;
        smp.loop_start = 10;
        smp.loop_end = 20;
        smp.base_note = 60;
        let bank = bank_with(smp);
        let mut engine = SamplePlaybackEngine::new(8);
        let rate = bank.get(0).unwrap().sample_rate;
        engine.note_on(
            0,
            60,
            100,
            0,
            bank.get(0).unwrap(),
            rate,
            NewNoteAction::Cut,
        );
        let (mut l, mut r) = ([0.0], [0.0]);
        (0..frames)
            .map(|_| {
                engine.render(&bank, &mut l, &mut r);
                engine.voices[0].position
            })
            .collect()
    }

    #[test]
    fn a_ping_pong_loop_turns_at_both_ends() {
        let p = loop_positions(LoopMode::PingPong, 40);
        // Frame 19 is the last; position 19 is reached after 19 steps.
        assert_eq!(p[18], 19.0);
        assert_eq!(p[19], 18.0);
        assert_eq!(p[27], 10.0);
        assert_eq!(p[28], 11.0);
    }

    #[test]
    fn a_reverse_loop_plays_through_then_repeats_backwards() {
        let p = loop_positions(LoopMode::Reverse, 40);
        assert_eq!(p[18], 19.0);
        assert_eq!(p[19], 18.0);
        assert_eq!(p[27], 10.0);
        assert_eq!(p[28], 19.0, "wraps to the top, still going down");
        assert_eq!(p[29], 18.0);
    }

    #[test]
    fn a_fast_voice_stays_inside_every_loop_mode() {
        for mode in LoopMode::ALL {
            let mut smp = make_test_sample();
            smp.loop_enabled = true;
            smp.loop_mode = mode;
            smp.loop_start = 10;
            smp.loop_end = 13;
            let bank = bank_with(smp);
            let mut engine = SamplePlaybackEngine::new(8);
            let s = bank.get(0).unwrap();
            engine.note_on(0, 127, 100, 0, s, 1000.0, NewNoteAction::Cut);
            let (mut l, mut r) = ([0.0], [0.0]);
            for _ in 0..500 {
                engine.render(&bank, &mut l, &mut r);
                let pos = engine.voices[0].position;
                assert!((0.0..13.0).contains(&pos), "{mode:?} at {pos}");
                assert!(l[0].is_finite());
            }
        }
    }

    #[test]
    fn a_ping_pong_turn_does_not_step_the_output() {
        let mut smp = make_test_sample();
        let wave: Vec<[f32; 2]> = (0..4000).map(|i| [(i as f32 * 0.03).sin(); 2]).collect();
        smp.data = wave.into();
        smp.loop_enabled = true;
        smp.loop_mode = LoopMode::PingPong;
        smp.loop_start = 1000;
        smp.loop_end = 1317; // not a zero crossing or a whole cycle
        let bank = bank_with(smp);
        let mut engine = SamplePlaybackEngine::new(8);
        let s = bank.get(0).unwrap();
        engine.note_on(
            0,
            60,
            127,
            0,
            s,
            s.sample_rate * 2f64.powf(-0.77 / 12.0),
            NewNoteAction::Cut,
        );
        let (mut l, mut r) = (vec![0.0; 6000], vec![0.0; 6000]);
        engine.render(&bank, &mut l, &mut r);
        let jump = l[1000..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.06, "largest step {jump}");
    }

    #[test]
    fn panning_a_sounding_voice_glides() {
        let bank = make_test_bank();
        let mut engine = SamplePlaybackEngine::new(8);
        let s = bank.get(0).unwrap();
        engine.note_on(0, 60, 100, 0, s, 44100.0, NewNoteAction::Cut);
        engine.set_channel_pan(0, 1.0);
        let gain = engine.voices[0].pan.gains(1.0).0;
        assert!(gain > 0.9, "one frame moves a little: {gain}");
        for _ in 0..4000 {
            engine.voices[0].pan.gains(1.0);
        }
        assert_eq!(engine.voices[0].pan.gains(1.0).0, 0.0);
    }

    #[test]
    fn a_hard_left_pan_silences_the_right_side() {
        let bank = make_test_bank();
        let mut engine = SamplePlaybackEngine::new(8);
        engine.set_channel_pan(0, -1.0);
        engine.note_on(
            0,
            60,
            100,
            0,
            bank.get(0).unwrap(),
            44100.0,
            NewNoteAction::Cut,
        );
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        engine.render(&bank, &mut l, &mut r);
        assert!(l.iter().any(|v| v.abs() > 0.0), "left is silent");
        assert!(r.iter().all(|v| *v == 0.0), "right still sounds");
    }

    #[test]
    fn a_sample_envelope_releases_at_its_own_rate() {
        // Looping, so the voice ends by its envelope and not by running out.
        let mut smp = make_test_sample();
        smp.loop_enabled = true;
        smp.loop_end = smp.data.len();
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(smp));
        let sample = bank.get(0).unwrap();
        let sounding_after_release = |env: Option<AmpEnv>| {
            let mut engine = SamplePlaybackEngine::new(8);
            let fx = SampleFx {
                env,
                ..SampleFx::default()
            };
            engine.note_on_with_offset(0, 60, 100, 0, sample, 44100.0, NewNoteAction::Cut, 0, &fx);
            // Past the attack, then release; a release from level 0 ends at once.
            let (mut l, mut r) = (vec![0.0; 22050], vec![0.0; 22050]);
            engine.render(&bank, &mut l[..1000], &mut r[..1000]);
            engine.note_off(0, 60);
            // 0.5 s: past the default 50 ms release, inside a 2 s one.
            engine.render(&bank, &mut l, &mut r);
            engine.voices.iter().any(|v| v.active)
        };
        assert!(!sounding_after_release(None));
        assert!(sounding_after_release(Some(AmpEnv {
            release: 2.0,
            ..AmpEnv::default()
        })));
    }
}
