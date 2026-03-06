/// Shared ADSR envelope generator used by both the built-in synth and sample
/// playback engine, eliminating the previous duplication between `EnvStage` in
/// `synth.rs` and `SampleEnvStage`/`SampleEnvelope` in `sample/playback.rs`.

/// ADSR envelope stage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

/// Reusable ADSR envelope generator
#[derive(Clone)]
pub struct Envelope {
    pub stage: EnvStage,
    pub level: f32,
    pub attack: f32,   // seconds
    pub decay: f32,    // seconds
    pub sustain: f32,  // 0..1
    pub release: f32,  // seconds (exponential time constant)
    sample_rate: f32,
}

impl Envelope {
    pub fn new(attack: f32, decay: f32, sustain: f32, release: f32, sample_rate: f32) -> Self {
        Self {
            stage: EnvStage::Attack,
            level: 0.0,
            attack,
            decay,
            sustain,
            release,
            sample_rate,
        }
    }

    /// Create with default sample voice parameters (2ms attack, 50ms release)
    pub fn sample_default(sample_rate: f32) -> Self {
        Self::new(0.002, 0.0, 1.0, 0.05, sample_rate)
    }

    pub fn release(&mut self) {
        if self.stage != EnvStage::Off {
            self.stage = EnvStage::Release;
        }
    }

    pub fn is_active(&self) -> bool {
        self.stage != EnvStage::Off
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        // Attack
        if self.stage == EnvStage::Attack {
            if self.attack > 0.0 {
                self.level += 1.0 / (self.attack * self.sample_rate);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            } else {
                self.level = 1.0;
                self.stage = EnvStage::Decay;
            }
        }
        // Decay (fall through from Attack if zero-duration)
        if self.stage == EnvStage::Decay {
            if self.decay > 0.0 {
                self.level -= (1.0 - self.sustain) / (self.decay * self.sample_rate);
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.stage = EnvStage::Sustain;
                }
            } else {
                self.level = self.sustain;
                self.stage = EnvStage::Sustain;
            }
        }
        // Sustain: hold level (no processing needed)
        // Release
        if self.stage == EnvStage::Release {
            if self.release > 0.0 {
                self.level -= self.level / (self.release * self.sample_rate);
                if self.level < 0.001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Off;
                }
            } else {
                self.level = 0.0;
                self.stage = EnvStage::Off;
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_attack_decay_sustain() {
        let sr = 44100.0;
        let mut env = Envelope::new(0.01, 0.05, 0.5, 0.1, sr);

        // During attack, level should rise toward 1.0
        for _ in 0..200 {
            env.tick();
        }
        assert!(env.level > 0.0, "Level should rise during attack");

        // Run through attack + decay into sustain
        for _ in 0..44100 {
            env.tick();
        }
        assert!((env.level - 0.5).abs() < 0.01,
            "Level should settle at sustain=0.5, got {:.4}", env.level);
        assert_eq!(env.stage, EnvStage::Sustain);
    }

    #[test]
    fn test_envelope_release() {
        let sr = 44100.0;
        let mut env = Envelope::new(0.001, 0.0, 1.0, 0.05, sr);

        // Run into sustain
        for _ in 0..4410 {
            env.tick();
        }
        assert_eq!(env.stage, EnvStage::Sustain);

        env.release();
        assert_eq!(env.stage, EnvStage::Release);

        // Run until off
        for _ in 0..44100 {
            env.tick();
        }
        assert_eq!(env.stage, EnvStage::Off);
        assert_eq!(env.level, 0.0);
        assert!(!env.is_active());
    }

    #[test]
    fn test_envelope_sample_default() {
        let env = Envelope::sample_default(44100.0);
        assert!((env.attack - 0.002).abs() < 1e-6);
        assert!((env.sustain - 1.0).abs() < 1e-6);
        assert!((env.release - 0.05).abs() < 1e-6);
    }

    #[test]
    fn test_envelope_zero_attack() {
        let sr = 44100.0;
        let mut env = Envelope::new(0.0, 0.0, 0.8, 0.1, sr);
        // Single tick: Attack->Decay->Sustain (fall-through for zero durations)
        env.tick();
        assert_eq!(env.stage, EnvStage::Sustain);
        assert!((env.level - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_envelope_zero_release() {
        let sr = 44100.0;
        let mut env = Envelope::new(0.0, 0.0, 1.0, 0.0, sr);
        // Single tick to reach sustain (fall-through)
        env.tick();
        assert_eq!(env.stage, EnvStage::Sustain);

        env.release();
        env.tick();
        assert_eq!(env.stage, EnvStage::Off);
        assert_eq!(env.level, 0.0);
    }

    #[test]
    fn test_release_from_off_is_noop() {
        let mut env = Envelope::new(0.0, 0.0, 1.0, 0.0, 44100.0);
        env.stage = EnvStage::Off;
        env.release();
        // Should remain Off, not transition to Release
        assert_eq!(env.stage, EnvStage::Off);
    }
}
