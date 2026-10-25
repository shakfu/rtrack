//! Smoothing for a channel's [`Timbre`], which the engine changes once per
//! tick (`docs/dev/g6-modulation.md`).

use crate::types::Timbre;

/// Seconds a voice takes to reach about 63% of a new timbre.
const GLIDE_SECS: f32 = 0.005;

/// A voice's timbre, moving towards its channel's latest one.
#[derive(Debug, Clone, Copy)]
pub struct TimbreGlide {
    current: Timbre,
    target: Timbre,
    coeff: f32,
}

impl TimbreGlide {
    /// Start at `timbre`, with nothing to glide.
    pub fn at(timbre: Timbre, sample_rate: f32) -> Self {
        Self {
            current: timbre,
            target: timbre,
            coeff: 1.0 - (-1.0 / (GLIDE_SECS * sample_rate)).exp(),
        }
    }

    pub fn set_target(&mut self, target: Timbre) {
        self.target = target;
    }

    /// Advance one frame and return the timbre to apply.
    #[inline]
    pub fn advance(&mut self) -> Timbre {
        if self.current != self.target {
            let step = |c: &mut f32, t: f32| {
                *c += (t - *c) * self.coeff;
                // Land exactly, so a settled glide costs one comparison.
                if (t - *c).abs() < 1e-4 {
                    *c = t;
                }
            };
            step(&mut self.current.cutoff, self.target.cutoff);
            step(&mut self.current.resonance, self.target.resonance);
            step(&mut self.current.drive, self.target.drive);
        }
        self.current
    }

    pub fn settled(&self) -> bool {
        self.current == self.target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glide_moves_smoothly_and_lands_on_its_target() {
        let mut glide = TimbreGlide::at(Timbre::default(), 48000.0);
        glide.set_target(Timbre {
            cutoff: -4.0,
            ..Timbre::default()
        });
        let first = glide.advance().cutoff;
        assert!(
            first < 0.0 && first > -0.1,
            "one frame moves a little: {first}"
        );
        // 5 ms per time constant: 75 ms is 15, leaving 4 * e^-15 of the step.
        for _ in 0..3600 {
            glide.advance();
        }
        assert!(glide.settled());
        assert_eq!(glide.advance().cutoff, -4.0);
    }
}
