use super::{Sample, SampleBank};
use crate::audio::envelope::Envelope;
use crate::constants::{
    MIDI_MAX_VALUE, SAMPLE_DECLICK_SECS, SAMPLE_INAUDIBLE_LEVEL, SEMITONES_PER_OCTAVE,
};

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
) {
    let end = sample.end() as f64;
    let loop_start = sample.effective_loop_start() as f64;
    let loop_end = sample.effective_loop_end() as f64;
    let looping = sample.loop_enabled && loop_end > loop_start;

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

        let fm1 = sample.frame_at(idx.saturating_sub(1));
        let f0 = sample.frame_at(idx);
        let f1 = sample.frame_at(idx + 1);
        let f2 = sample.frame_at(idx + 2);

        let l = cubic_hermite(fm1[0], f0[0], f1[0], f2[0], frac);
        let r = cubic_hermite(fm1[1], f0[1], f1[1], f2[1], frac);

        // Fade towards the end of a one-shot. A slice ends at an arbitrary
        // frame, so stopping on it would leave a step in the output.
        let gain = if looping {
            env_level
        } else {
            env_level * (((end - pos) / declick).clamp(0.0, 1.0) as f32)
        };

        left[i] += l * voice.velocity * gain;
        right[i] += r * voice.velocity * gain;

        // Advance position
        voice.position += voice.rate;

        // Handle loop or end
        if looping {
            if voice.position >= loop_end {
                // Wrap modulo the loop length rather than subtracting it
                // once: a voice pitched up far enough to cover the whole
                // loop in a single frame would otherwise walk past the loop
                // end and never come back.
                let len = loop_end - loop_start;
                voice.position = loop_start + (voice.position - loop_start).rem_euclid(len);
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
}

impl SamplePlaybackEngine {
    pub fn new(max_voices: usize) -> Self {
        Self {
            voices: Vec::with_capacity(max_voices),
            max_voices,
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
            position: sample.trim_start as f64,
            rate,
            output_rate,
            velocity: vel,
            channel,
            note,
            active: true,
            envelope: Envelope::sample_default(output_rate as f32),
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
            render_voice(voice, sample, left, right, 0..frames);
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
            render_voice(
                voice,
                sample,
                &mut channel_left[ch],
                &mut channel_right[ch],
                range.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::envelope::EnvStage;
    use crate::sample::Sample;
    use std::sync::Arc;

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
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }
    }

    fn make_test_bank() -> SampleBank {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(make_test_sample()));
        bank
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
            loop_start: 0,
            loop_end: 0,
            source_path: None,
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
            loop_start: 2,
            loop_end: 8,
            source_path: None,
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
            loop_start: 0,
            loop_end: 1000,
            source_path: None,
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
}
