use super::{Sample, SampleBank};

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
        let pitch_ratio = 2.0_f64.powf((note as f64 - sample.base_note as f64) / 12.0);
        let rate_ratio = sample.sample_rate / output_rate;
        let rate = pitch_ratio * rate_ratio;
        let vel = velocity as f32 / 127.0;

        // Evict oldest voice if at capacity
        if self.voices.len() >= self.max_voices {
            // Remove first inactive, or first voice
            if let Some(idx) = self.voices.iter().position(|v| !v.active) {
                self.voices.remove(idx);
            } else {
                self.voices.remove(0);
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
        });
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        for voice in &mut self.voices {
            if voice.channel == channel && voice.note == note && voice.active {
                voice.active = false;
            }
        }
    }

    pub fn note_off_channel(&mut self, channel: u8) {
        for voice in &mut self.voices {
            if voice.channel == channel && voice.active {
                voice.active = false;
            }
        }
    }

    pub fn note_off_all(&mut self) {
        self.voices.clear();
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

                // Linear interpolation between two adjacent frames
                let pos = voice.position;
                let idx = pos as usize;
                let frac = (pos - idx as f64) as f32;

                let f0 = sample.frame_at(idx);
                let f1 = sample.frame_at(idx + 1);
                let l = f0[0] * (1.0 - frac) + f1[0] * frac;
                let r = f0[1] * (1.0 - frac) + f1[1] * frac;

                left[i] += l * voice.velocity;
                right[i] += r * voice.velocity;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Sample;

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
        }
    }

    fn make_test_bank() -> SampleBank {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(make_test_sample());
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
        assert!(!engine.voices[0].active);
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
        bank.samples[0] = Some(Sample {
            name: "short".into(),
            data: vec![[1.0, 1.0]; 5],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
        });

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
        bank.samples[0] = Some(Sample {
            name: "loop".into(),
            data: vec![[0.5, 0.5]; 10],
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: true,
            loop_start: 2,
            loop_end: 8,
        });

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
        let active: Vec<_> = engine.voices.iter().filter(|v| v.active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].channel, 1);
    }
}
