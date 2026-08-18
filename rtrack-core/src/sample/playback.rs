use super::{Sample, SampleBank};
use crate::audio::envelope::Envelope;
use crate::constants::{MIDI_MAX_VALUE, SEMITONES_PER_OCTAVE};

/// Type alias for backward compatibility (previously a separate struct)
pub type SampleEnvelope = Envelope;

/// A single playing voice (one sample instance)
pub struct SampleVoice {
    pub sample_index: usize,
    /// Current fractional frame position within the sample
    pub position: f64,
    /// Playback rate: 1.0 = original pitch, 2.0 = one octave up, 0.5 = one octave down.
    /// Incorporates both pitch shifting (note vs base_note) and sample rate conversion.
    pub rate: f64,
    pub velocity: f32,
    pub channel: u8,
    pub note: u8,
    pub active: bool,
    pub envelope: SampleEnvelope,
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
    pub fn note_on(
        &mut self,
        sample_index: usize,
        note: u8,
        velocity: u8,
        channel: u8,
        sample: &Sample,
        output_rate: f64,
    ) {
        // Kill existing voice for same channel+note
        self.note_off(channel, note);

        // Calculate playback rate:
        //   pitch_ratio = 2^((note - base_note) / 12)
        //   rate_ratio  = sample.sample_rate / output_rate
        //   effective_rate = pitch_ratio * rate_ratio
        let pitch_ratio =
            2.0_f64.powf((note as f64 - sample.base_note as f64) / SEMITONES_PER_OCTAVE as f64);
        let rate_ratio = sample.sample_rate / output_rate;
        let rate = pitch_ratio * rate_ratio;
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;

        // Evict quietest voice if at capacity
        if self.voices.len() >= self.max_voices {
            if let Some(idx) = self.voices.iter().position(|v| !v.active) {
                self.voices.remove(idx);
            } else {
                // Steal the quietest voice
                let quietest = self
                    .voices
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let a_level = a.envelope.level * a.velocity;
                        let b_level = b.envelope.level * b.velocity;
                        a_level
                            .partial_cmp(&b_level)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i);
                if let Some(idx) = quietest {
                    self.voices.remove(idx);
                }
            }
        }

        self.voices.push(SampleVoice {
            sample_index,
            position: sample.trim_start as f64,
            rate,
            velocity: vel,
            channel,
            note,
            active: true,
            envelope: Envelope::sample_default(output_rate as f32),
        });
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
        let frames = left.len();

        self.voices.retain(|v| v.active);

        for voice in &mut self.voices {
            let sample = match bank.get(voice.sample_index) {
                Some(s) => s,
                None => {
                    voice.active = false;
                    continue;
                }
            };

            let end = sample.end() as f64;
            let loop_start = sample.effective_loop_start() as f64;
            let loop_end = sample.effective_loop_end() as f64;

            for i in 0..frames {
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

                left[i] += l * voice.velocity * env_level;
                right[i] += r * voice.velocity * env_level;

                // Advance position
                voice.position += voice.rate;

                // Handle loop or end
                if sample.loop_enabled && loop_end > loop_start {
                    if voice.position >= loop_end {
                        voice.position = loop_start + (voice.position - loop_end);
                    }
                } else if voice.position >= end {
                    voice.active = false;
                }
            }
        }
    }
    /// Render sample voices into per-channel buffers.
    /// `channel_bufs` is indexed by tracker channel, each element is (left, right) slices.
    /// Render active voices into per-channel buffers, writing only the
    /// sample range `range` of each buffer.
    ///
    /// Takes the left and right channel buffers separately rather than a
    /// slice of `(&mut [f32], &mut [f32])` pairs: building that pair list
    /// required a heap allocation and a pointer-aliasing `unsafe` block on
    /// every audio callback.
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
            let ch = (voice.channel as usize).min(channel_count - 1);
            let sample = match bank.get(voice.sample_index) {
                Some(s) => s,
                None => {
                    voice.active = false;
                    continue;
                }
            };

            let end = sample.end() as f64;
            let loop_start = sample.effective_loop_start() as f64;
            let loop_end = sample.effective_loop_end() as f64;
            for i in range.clone() {
                if !voice.active {
                    break;
                }

                let env_level = voice.envelope.tick();
                if !voice.envelope.is_active() {
                    voice.active = false;
                    break;
                }

                let pos = voice.position;
                let idx = pos as usize;
                let frac = (pos - idx as f64) as f32;

                let fm1 = sample.frame_at(idx.saturating_sub(1));
                let f0 = sample.frame_at(idx);
                let f1 = sample.frame_at(idx + 1);
                let f2 = sample.frame_at(idx + 2);

                let l = cubic_hermite(fm1[0], f0[0], f1[0], f2[0], frac);
                let r = cubic_hermite(fm1[1], f0[1], f1[1], f2[1], frac);

                channel_left[ch][i] += l * voice.velocity * env_level;
                channel_right[ch][i] += r * voice.velocity * env_level;

                voice.position += voice.rate;

                if sample.loop_enabled && loop_end > loop_start {
                    if voice.position >= loop_end {
                        voice.position = loop_start + (voice.position - loop_end);
                    }
                } else if voice.position >= end {
                    voice.active = false;
                }
            }
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
            data,
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
        engine.note_on(0, 60, 100, 0, sample, 44100.0);
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
        engine.note_on(0, 72, 100, 0, sample, 44100.0);
        assert!((engine.voices[0].rate - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_note_off() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0);
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
        engine.note_on(0, 60, 127, 0, sample, 44100.0);

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
            data: vec![[1.0, 1.0]; 5],
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
        engine.note_on(0, 60, 127, 0, sample, 44100.0);

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
            data: vec![[0.5, 0.5]; 10],
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
        engine.note_on(0, 60, 127, 0, sample, 44100.0);

        let mut left = vec![0.0f32; 100];
        let mut right = vec![0.0f32; 100];
        engine.render(&bank, &mut left, &mut right);

        // Should still be active after 100 frames (looping)
        assert!(engine.voices[0].active);
        // All frames should have output
        assert!(left[99] != 0.0 || right[99] != 0.0);
    }

    #[test]
    fn test_voice_limit() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(4);

        for note in 60..65 {
            engine.note_on(0, note, 100, 0, sample, 44100.0);
        }
        // Should have evicted oldest to stay at max_voices
        assert_eq!(engine.voices.len(), 4);
    }

    #[test]
    fn test_note_off_all() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0);
        engine.note_on(0, 64, 100, 1, sample, 44100.0);
        engine.note_off_all();
        assert_eq!(engine.voices.len(), 0);
    }

    #[test]
    fn test_note_off_channel() {
        let bank = make_test_bank();
        let sample = bank.get(0).unwrap();
        let mut engine = SamplePlaybackEngine::new(16);
        engine.note_on(0, 60, 100, 0, sample, 44100.0);
        engine.note_on(0, 64, 100, 1, sample, 44100.0);
        engine.note_off_channel(0);
        // Channel 0 voice should be in release, channel 1 still sustaining
        let ch0 = engine.voices.iter().find(|v| v.channel == 0).unwrap();
        assert_eq!(ch0.envelope.stage, EnvStage::Release);
        let ch1 = engine.voices.iter().find(|v| v.channel == 1).unwrap();
        assert!(ch1.active);
        assert_ne!(ch1.envelope.stage, EnvStage::Release);
    }

    #[test]
    fn test_envelope_fade_on_note_off() {
        // Use a looping sample so it doesn't end before we test
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(Sample {
            name: "loop".into(),
            data: vec![[0.5, 0.5]; 1000],
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
        engine.note_on(0, 60, 127, 0, sample, 44100.0);

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
