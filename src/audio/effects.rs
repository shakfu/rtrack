use fundsp::prelude32::*;

/// Stereo effects chain: lightweight stereo delay. Processes audio in-place.
pub struct EffectsChain {
    unit: Box<dyn AudioUnit>,
    pub enabled: bool,
}

impl EffectsChain {
    pub fn new(sample_rate: f64) -> Self {
        // Lightweight stereo delay: dry signal + delayed wet signal.
        // Left delay 80ms, right delay 120ms, both at 15% wet mix.
        // Much cheaper than FDN reverb (reverb_stereo) which caused
        // buffer underruns and distortion.
        let mut unit: Box<dyn AudioUnit> = Box::new(
            multipass::<U2>() & (delay(0.08) * 0.15 | delay(0.12) * 0.15),
        );
        unit.set_sample_rate(sample_rate);
        Self {
            unit,
            enabled: true,
        }
    }

    /// Process a buffer of interleaved stereo samples in-place
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let frames = left.len();
        for i in 0..frames {
            let input = [left[i], right[i]];
            let mut output = [0f32; 2];
            self.unit.tick(&input, &mut output);
            left[i] = output[0];
            right[i] = output[1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effects_chain_creates() {
        let chain = EffectsChain::new(44100.0);
        assert!(chain.enabled);
    }

    #[test]
    fn test_effects_chain_processes_silence() {
        let mut chain = EffectsChain::new(44100.0);
        let mut left = vec![0.0f32; 64];
        let mut right = vec![0.0f32; 64];
        chain.process(&mut left, &mut right);
        // Should not crash, silence in -> near-silence out
    }

    #[test]
    fn test_effects_chain_signal_levels() {
        let mut chain = EffectsChain::new(44100.0);
        // Feed a sine wave at 0.25 amplitude (typical synth level)
        let frames = 44100; // 1 second
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for i in 0..frames {
            let t = i as f32 / 44100.0;
            let s = 0.25 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
            left[i] = s;
            right[i] = s;
        }
        chain.process(&mut left, &mut right);

        let peak = left.iter().chain(right.iter())
            .fold(0.0_f32, |acc, &s| acc.max(s.abs()));
        let has_nan = left.iter().chain(right.iter()).any(|s| !s.is_finite());
        eprintln!("Effects chain: input peak=0.25, output peak={:.4}, has_nan={}", peak, has_nan);
        assert!(!has_nan, "Effects chain produced NaN/Inf");
        assert!(peak < 2.0, "Effects chain excessive amplification: {:.4}", peak);
    }

    #[test]
    fn test_effects_chain_disabled() {
        let mut chain = EffectsChain::new(44100.0);
        chain.enabled = false;
        let mut left = vec![1.0f32; 64];
        let mut right = vec![1.0f32; 64];
        chain.process(&mut left, &mut right);
        // Disabled: samples unchanged
        assert_eq!(left[0], 1.0);
        assert_eq!(right[0], 1.0);
    }
}
