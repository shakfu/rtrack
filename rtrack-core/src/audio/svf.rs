//! State-variable filter (Andrew Simper's trapezoidal SVF), shared by track
//! effects and sample voices.

use super::synth::FilterType;

/// Coefficients for one cutoff and resonance; compute once while they hold.
#[derive(Debug, Clone, Copy)]
pub struct SvfCoeffs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoeffs {
    /// `resonance` 0..1 maps to damping `2 - 2 * resonance`; 1 is undamped.
    /// Callers clamp; only the cutoff is held below Nyquist here, where the
    /// tangent turns negative.
    pub fn new(cutoff_hz: f32, resonance: f32, sample_rate: f32) -> Self {
        let cutoff = cutoff_hz.min(0.49 * sample_rate);
        let g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        let k = 2.0 - 2.0 * resonance;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { k, a1, a2, a3 }
    }
}

/// One channel of filter state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1eq: f32,
    ic2eq: f32,
}

impl Svf {
    /// Filter one sample; returns (low, band).
    #[inline]
    pub fn tick(&mut self, c: &SvfCoeffs, x: f32) -> (f32, f32) {
        let v3 = x - self.ic2eq;
        let v1 = c.a1 * self.ic1eq + c.a2 * v3;
        let v2 = self.ic2eq + c.a2 * self.ic1eq + c.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        (v2, v1)
    }

    /// Filter one sample and return the `kind` output.
    #[inline]
    pub fn tick_kind(&mut self, c: &SvfCoeffs, kind: FilterType, x: f32) -> f32 {
        let (low, band) = self.tick(c, x);
        match kind {
            FilterType::LowPass => low,
            FilterType::BandPass => band,
            FilterType::HighPass => x - c.k * band - low,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak output of `kind` for a sine at `freq` through a 1 kHz filter.
    fn gain(kind: FilterType, freq: f32) -> f32 {
        let sr = 48000.0;
        let c = SvfCoeffs::new(1000.0, 0.0, sr);
        let mut svf = Svf::default();
        let mut peak = 0.0f32;
        for n in 0..9600 {
            let x = (std::f32::consts::TAU * freq * n as f32 / sr).sin();
            let y = svf.tick_kind(&c, kind, x);
            if n > 4800 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn each_kind_passes_its_band() {
        assert!(gain(FilterType::LowPass, 100.0) > 0.9);
        assert!(gain(FilterType::LowPass, 10000.0) < 0.05);
        assert!(gain(FilterType::HighPass, 10000.0) > 0.9);
        assert!(gain(FilterType::HighPass, 100.0) < 0.05);
        assert!(gain(FilterType::BandPass, 1000.0) > 0.4);
        assert!(gain(FilterType::BandPass, 100.0) < 0.1);
        assert!(gain(FilterType::BandPass, 10000.0) < 0.1);
    }

    #[test]
    fn a_cutoff_above_nyquist_stays_finite() {
        let c = SvfCoeffs::new(30000.0, 0.5, 44100.0);
        let mut svf = Svf::default();
        for n in 0..1000 {
            let y = svf.tick_kind(&c, FilterType::LowPass, (n as f32 * 0.3).sin());
            assert!(y.is_finite());
        }
    }
}
