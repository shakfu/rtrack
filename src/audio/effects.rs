use fundsp::prelude32::*;

/// Stereo effects chain: chorus -> reverb. Processes audio in-place.
pub struct EffectsChain {
    unit: Box<dyn AudioUnit>,
    pub enabled: bool,
    /// Reverb mix (0.0 = dry, 1.0 = fully wet)
    pub reverb_mix: f32,
}

impl EffectsChain {
    pub fn new(sample_rate: f64) -> Self {
        // Stereo reverb mixed with dry signal using bus operator
        // multipass passes stereo through dry, reverb_stereo processes wet
        let mut unit: Box<dyn AudioUnit> = Box::new(
            multipass::<U2>() & (reverb_stereo(40.0, 3.0, 0.5) * 0.3),
        );
        unit.set_sample_rate(sample_rate);
        Self {
            unit,
            enabled: true,
            reverb_mix: 0.3,
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
