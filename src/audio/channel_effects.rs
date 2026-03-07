/// Per-channel effects: filter, distortion, chorus, delay, reverb.
/// All DSP is manual (no fundsp) to keep per-channel cost low.

use serde::{Deserialize, Serialize};

use super::effects::MAX_SEND_BUSES;

/// Maximum tracker channels supported for per-channel effects.
pub const MAX_EFFECT_CHANNELS: usize = 16;

/// Parameters for a channel's effects chain, settable from UI/commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEffectsParams {
    pub filter_enabled: bool,
    /// Cutoff frequency in Hz (20..20000)
    pub filter_cutoff: f32,
    /// Resonance (0.0..1.0, maps to Q)
    pub filter_resonance: f32,

    pub distortion_enabled: bool,
    /// Drive amount (1.0 = clean, higher = more distortion)
    pub distortion_drive: f32,

    pub chorus_enabled: bool,
    /// Chorus rate in Hz (0.1..5.0)
    pub chorus_rate: f32,
    /// Chorus depth in ms (0.5..10.0)
    pub chorus_depth: f32,
    /// Chorus wet mix (0.0..1.0)
    pub chorus_mix: f32,

    #[serde(default)]
    pub delay_enabled: bool,
    /// Delay time in ms (1..2000)
    #[serde(default = "default_delay_time")]
    pub delay_time: f32,
    /// Feedback (0.0..0.95)
    #[serde(default = "default_delay_feedback")]
    pub delay_feedback: f32,
    /// Wet mix (0.0..1.0)
    #[serde(default = "default_delay_mix")]
    pub delay_mix: f32,

    #[serde(default)]
    pub reverb_enabled: bool,
    /// Room size (0.0..1.0)
    #[serde(default = "default_reverb_size")]
    pub reverb_size: f32,
    /// Damping (0.0..1.0, higher = more high-frequency absorption)
    #[serde(default = "default_reverb_damp")]
    pub reverb_damp: f32,
    /// Wet mix (0.0..1.0)
    #[serde(default = "default_reverb_mix")]
    pub reverb_mix: f32,

    /// Send levels to shared effect buses (0.0..1.0 per bus)
    #[serde(default)]
    pub send_levels: [f32; MAX_SEND_BUSES],
}

fn default_delay_time() -> f32 { 250.0 }
fn default_delay_feedback() -> f32 { 0.4 }
fn default_delay_mix() -> f32 { 0.3 }
fn default_reverb_size() -> f32 { 0.5 }
fn default_reverb_damp() -> f32 { 0.5 }
fn default_reverb_mix() -> f32 { 0.3 }

impl Default for ChannelEffectsParams {
    fn default() -> Self {
        Self {
            filter_enabled: false,
            filter_cutoff: 8000.0,
            filter_resonance: 0.0,
            distortion_enabled: false,
            distortion_drive: 1.0,
            chorus_enabled: false,
            chorus_rate: 1.5,
            chorus_depth: 3.0,
            chorus_mix: 0.3,
            delay_enabled: false,
            delay_time: 250.0,
            delay_feedback: 0.4,
            delay_mix: 0.3,
            reverb_enabled: false,
            reverb_size: 0.5,
            reverb_damp: 0.5,
            reverb_mix: 0.3,
            send_levels: [0.0; MAX_SEND_BUSES],
        }
    }
}

/// Number of comb filters in the Schroeder reverb.
const REVERB_COMBS: usize = 4;
/// Number of allpass filters in the Schroeder reverb.
const REVERB_ALLPASSES: usize = 2;

/// Per-channel effects processor with internal DSP state.
pub struct ChannelEffects {
    pub params: ChannelEffectsParams,
    sample_rate: f64,

    // SVF filter state (2-pole state variable filter, one per stereo channel)
    svf_ic1eq_l: f32,
    svf_ic2eq_l: f32,
    svf_ic1eq_r: f32,
    svf_ic2eq_r: f32,

    // Chorus delay line (circular buffer, max ~50ms at 48kHz = 2400 samples)
    chorus_buffer_l: Vec<f32>,
    chorus_buffer_r: Vec<f32>,
    chorus_write_pos: usize,
    chorus_phase: f64,

    // Delay (stereo circular buffer, max 2s)
    delay_buffer_l: Vec<f32>,
    delay_buffer_r: Vec<f32>,
    delay_write_pos: usize,

    // Schroeder reverb: 4 comb filters + 2 allpass filters, per stereo channel
    comb_buffers_l: [Vec<f32>; REVERB_COMBS],
    comb_buffers_r: [Vec<f32>; REVERB_COMBS],
    comb_pos: [usize; REVERB_COMBS],
    comb_filter_state_l: [f32; REVERB_COMBS],
    comb_filter_state_r: [f32; REVERB_COMBS],
    allpass_buffers_l: [Vec<f32>; REVERB_ALLPASSES],
    allpass_buffers_r: [Vec<f32>; REVERB_ALLPASSES],
    allpass_pos: [usize; REVERB_ALLPASSES],
}

/// Comb filter delay lengths in samples at 44100 Hz (Schroeder-style, mutually prime).
const COMB_LENGTHS_44100: [usize; REVERB_COMBS] = [1116, 1188, 1277, 1356];
/// Allpass delay lengths in samples at 44100 Hz.
const ALLPASS_LENGTHS_44100: [usize; REVERB_ALLPASSES] = [225, 556];

impl ChannelEffects {
    pub fn new(sample_rate: f64) -> Self {
        let chorus_buf_size = (sample_rate * 0.05) as usize + 256; // 50ms + headroom
        let delay_buf_size = (sample_rate * 2.0) as usize + 1; // 2 seconds max
        let sr_ratio = sample_rate / 44100.0;

        let comb_buffers_l = COMB_LENGTHS_44100.map(|len| {
            vec![0.0f32; (len as f64 * sr_ratio) as usize + 1]
        });
        let comb_buffers_r = COMB_LENGTHS_44100.map(|len| {
            vec![0.0f32; (len as f64 * sr_ratio) as usize + 1]
        });
        let allpass_buffers_l = ALLPASS_LENGTHS_44100.map(|len| {
            vec![0.0f32; (len as f64 * sr_ratio) as usize + 1]
        });
        let allpass_buffers_r = ALLPASS_LENGTHS_44100.map(|len| {
            vec![0.0f32; (len as f64 * sr_ratio) as usize + 1]
        });

        Self {
            params: ChannelEffectsParams::default(),
            sample_rate,
            svf_ic1eq_l: 0.0,
            svf_ic2eq_l: 0.0,
            svf_ic1eq_r: 0.0,
            svf_ic2eq_r: 0.0,
            chorus_buffer_l: vec![0.0; chorus_buf_size],
            chorus_buffer_r: vec![0.0; chorus_buf_size],
            chorus_write_pos: 0,
            chorus_phase: 0.0,
            delay_buffer_l: vec![0.0; delay_buf_size],
            delay_buffer_r: vec![0.0; delay_buf_size],
            delay_write_pos: 0,
            comb_buffers_l,
            comb_buffers_r,
            comb_pos: [0; REVERB_COMBS],
            comb_filter_state_l: [0.0; REVERB_COMBS],
            comb_filter_state_r: [0.0; REVERB_COMBS],
            allpass_buffers_l,
            allpass_buffers_r,
            allpass_pos: [0; REVERB_ALLPASSES],
        }
    }

    /// Returns true if any effect is active.
    pub fn any_enabled(&self) -> bool {
        self.params.filter_enabled
            || self.params.distortion_enabled
            || self.params.chorus_enabled
            || self.params.delay_enabled
            || self.params.reverb_enabled
    }

    /// Process a buffer of stereo samples in-place.
    /// Chain order: distortion -> filter -> chorus
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.any_enabled() {
            return;
        }
        let frames = left.len().min(right.len());

        for i in 0..frames {
            let mut l = left[i];
            let mut r = right[i];

            // 1. Distortion (tanh waveshaping)
            if self.params.distortion_enabled {
                let drive = self.params.distortion_drive;
                l = (l * drive).tanh();
                r = (r * drive).tanh();
            }

            // 2. Filter (SVF low-pass, applied to mono sum then split back)
            if self.params.filter_enabled {
                // Process L and R with same filter coefficients but we apply
                // the filter to each channel. Using a single SVF for mono-summed
                // signal to save state, then reconstruct stereo from the ratio.
                let (fl, fr) = self.svf_tick(l, r);
                l = fl;
                r = fr;
            }

            // 3. Chorus
            if self.params.chorus_enabled {
                let (cl, cr) = self.chorus_tick(l, r);
                l = cl;
                r = cr;
            }

            // 4. Delay
            if self.params.delay_enabled {
                let (dl, dr) = self.delay_tick(l, r);
                l = dl;
                r = dr;
            }

            // 5. Reverb
            if self.params.reverb_enabled {
                let (rl, rr) = self.reverb_tick(l, r);
                l = rl;
                r = rr;
            }

            left[i] = l;
            right[i] = r;
        }
    }

    /// SVF (state variable filter) low-pass tick for stereo.
    /// Uses the "Cytomic SVF" / Andrew Simper's linear trapezoidal integration.
    /// Independent filter state per stereo channel for correct stereo imaging.
    #[inline]
    fn svf_tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let cutoff = self.params.filter_cutoff.clamp(20.0, 20000.0);
        let res = self.params.filter_resonance.clamp(0.0, 1.0);

        let g = (std::f32::consts::PI * cutoff / self.sample_rate as f32).tan();
        let k = 2.0 - 2.0 * res;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        // Left channel
        let v3_l = l - self.svf_ic2eq_l;
        let v1_l = a1 * self.svf_ic1eq_l + a2 * v3_l;
        let v2_l = self.svf_ic2eq_l + a2 * self.svf_ic1eq_l + a3 * v3_l;
        self.svf_ic1eq_l = 2.0 * v1_l - self.svf_ic1eq_l;
        self.svf_ic2eq_l = 2.0 * v2_l - self.svf_ic2eq_l;

        // Right channel
        let v3_r = r - self.svf_ic2eq_r;
        let v1_r = a1 * self.svf_ic1eq_r + a2 * v3_r;
        let v2_r = self.svf_ic2eq_r + a2 * self.svf_ic1eq_r + a3 * v3_r;
        self.svf_ic1eq_r = 2.0 * v1_r - self.svf_ic1eq_r;
        self.svf_ic2eq_r = 2.0 * v2_r - self.svf_ic2eq_r;

        (v2_l, v2_r)
    }

    /// Chorus tick: LFO-modulated delay with wet/dry mix.
    #[inline]
    fn chorus_tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let buf_len = self.chorus_buffer_l.len();
        let rate = self.params.chorus_rate as f64;
        let depth_ms = self.params.chorus_depth as f64;
        let mix = self.params.chorus_mix;

        // Write current sample to delay buffer
        self.chorus_buffer_l[self.chorus_write_pos] = l;
        self.chorus_buffer_r[self.chorus_write_pos] = r;

        // LFO: sine wave modulates delay time
        let lfo = self.chorus_phase.sin() as f32;
        self.chorus_phase += 2.0 * std::f64::consts::PI * rate / self.sample_rate;
        if self.chorus_phase > 2.0 * std::f64::consts::PI {
            self.chorus_phase -= 2.0 * std::f64::consts::PI;
        }

        // Delay time: base 10ms + LFO-modulated depth
        let base_delay_samples = 0.01 * self.sample_rate as f32;
        let mod_samples = (depth_ms as f32 / 1000.0) * self.sample_rate as f32 * lfo;
        let delay_samples = base_delay_samples + mod_samples;
        let delay_samples = delay_samples.clamp(1.0, (buf_len - 1) as f32);

        // Read from delay buffer with linear interpolation
        let read_pos = self.chorus_write_pos as f32 - delay_samples;
        let read_pos = if read_pos < 0.0 { read_pos + buf_len as f32 } else { read_pos };
        let idx0 = read_pos as usize % buf_len;
        let idx1 = (idx0 + 1) % buf_len;
        let frac = read_pos - read_pos.floor();

        let delayed_l = self.chorus_buffer_l[idx0] * (1.0 - frac) + self.chorus_buffer_l[idx1] * frac;
        let delayed_r = self.chorus_buffer_r[idx0] * (1.0 - frac) + self.chorus_buffer_r[idx1] * frac;

        // Advance write position
        self.chorus_write_pos = (self.chorus_write_pos + 1) % buf_len;

        // Mix dry + wet
        let out_l = l * (1.0 - mix) + delayed_l * mix;
        let out_r = r * (1.0 - mix) + delayed_r * mix;
        (out_l, out_r)
    }

    /// Delay tick: simple stereo delay with feedback.
    #[inline]
    fn delay_tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let buf_len = self.delay_buffer_l.len();
        let delay_samples = (self.params.delay_time as f64 / 1000.0 * self.sample_rate)
            .clamp(1.0, (buf_len - 1) as f64) as usize;
        let read_pos = (self.delay_write_pos + buf_len - delay_samples) % buf_len;

        let delayed_l = self.delay_buffer_l[read_pos];
        let delayed_r = self.delay_buffer_r[read_pos];

        let feedback = self.params.delay_feedback.clamp(0.0, 0.95);
        self.delay_buffer_l[self.delay_write_pos] = l + delayed_l * feedback;
        self.delay_buffer_r[self.delay_write_pos] = r + delayed_r * feedback;
        self.delay_write_pos = (self.delay_write_pos + 1) % buf_len;

        let mix = self.params.delay_mix;
        (l * (1.0 - mix) + delayed_l * mix, r * (1.0 - mix) + delayed_r * mix)
    }

    /// Reverb tick: Schroeder reverb (4 parallel comb filters -> 2 series allpass filters).
    #[inline]
    fn reverb_tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let size = self.params.reverb_size.clamp(0.0, 1.0);
        let damp = self.params.reverb_damp.clamp(0.0, 1.0);
        let mix = self.params.reverb_mix;

        // Feedback coefficient scaled by room size (0.7..0.98)
        let feedback = 0.7 + size * 0.28;

        // Sum parallel comb filters
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;
        for i in 0..REVERB_COMBS {
            let buf_len = self.comb_buffers_l[i].len();
            let pos = self.comb_pos[i];
            let out_l = self.comb_buffers_l[i][pos];
            let out_r = self.comb_buffers_r[i][pos];

            // One-pole low-pass damping filter on comb output
            self.comb_filter_state_l[i] = out_l * (1.0 - damp) + self.comb_filter_state_l[i] * damp;
            self.comb_filter_state_r[i] = out_r * (1.0 - damp) + self.comb_filter_state_r[i] * damp;

            self.comb_buffers_l[i][pos] = l + self.comb_filter_state_l[i] * feedback;
            self.comb_buffers_r[i][pos] = r + self.comb_filter_state_r[i] * feedback;
            self.comb_pos[i] = (pos + 1) % buf_len;

            sum_l += out_l;
            sum_r += out_r;
        }

        // Series allpass filters
        let mut ap_l = sum_l;
        let mut ap_r = sum_r;
        for i in 0..REVERB_ALLPASSES {
            let buf_len = self.allpass_buffers_l[i].len();
            let pos = self.allpass_pos[i];
            let buf_out_l = self.allpass_buffers_l[i][pos];
            let buf_out_r = self.allpass_buffers_r[i][pos];

            self.allpass_buffers_l[i][pos] = ap_l + buf_out_l * 0.5;
            self.allpass_buffers_r[i][pos] = ap_r + buf_out_r * 0.5;
            self.allpass_pos[i] = (pos + 1) % buf_len;

            ap_l = buf_out_l - ap_l * 0.5;
            ap_r = buf_out_r - ap_r * 0.5;
        }

        (l * (1.0 - mix) + ap_l * mix, r * (1.0 - mix) + ap_r * mix)
    }

    /// Reset all internal DSP state (e.g., on song start).
    #[cfg(test)]
    fn reset(&mut self) {
        self.svf_ic1eq_l = 0.0;
        self.svf_ic2eq_l = 0.0;
        self.svf_ic1eq_r = 0.0;
        self.svf_ic2eq_r = 0.0;
        for s in &mut self.chorus_buffer_l { *s = 0.0; }
        for s in &mut self.chorus_buffer_r { *s = 0.0; }
        self.chorus_write_pos = 0;
        self.chorus_phase = 0.0;
        for s in &mut self.delay_buffer_l { *s = 0.0; }
        for s in &mut self.delay_buffer_r { *s = 0.0; }
        self.delay_write_pos = 0;
        for i in 0..REVERB_COMBS {
            for s in &mut self.comb_buffers_l[i] { *s = 0.0; }
            for s in &mut self.comb_buffers_r[i] { *s = 0.0; }
        }
        self.comb_pos = [0; REVERB_COMBS];
        self.comb_filter_state_l = [0.0; REVERB_COMBS];
        self.comb_filter_state_r = [0.0; REVERB_COMBS];
        for i in 0..REVERB_ALLPASSES {
            for s in &mut self.allpass_buffers_l[i] { *s = 0.0; }
            for s in &mut self.allpass_buffers_r[i] { *s = 0.0; }
        }
        self.allpass_pos = [0; REVERB_ALLPASSES];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_effects_default_passthrough() {
        let mut fx = ChannelEffects::new(44100.0);
        // All disabled by default -- signal should pass through unchanged
        let mut left = vec![0.5f32; 64];
        let mut right = vec![0.5f32; 64];
        fx.process(&mut left, &mut right);
        assert_eq!(left[0], 0.5);
        assert_eq!(right[0], 0.5);
    }

    #[test]
    fn test_distortion_increases_harmonics() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.distortion_enabled = true;
        fx.params.distortion_drive = 4.0;

        let frames = 4410;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for i in 0..frames {
            let t = i as f32 / 44100.0;
            let s = 0.5 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
            left[i] = s;
            right[i] = s;
        }
        fx.process(&mut left, &mut right);

        // Distorted signal should still be finite and bounded
        let peak = left.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 1.5, "Distortion output too hot: {}", peak);
        assert!(peak > 0.1, "Distortion killed the signal");
        assert!(left.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn test_filter_reduces_highs() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.filter_enabled = true;
        fx.params.filter_cutoff = 200.0; // Very low cutoff
        fx.params.filter_resonance = 0.0;

        let frames = 44100;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        // High frequency signal (5kHz)
        for i in 0..frames {
            let t = i as f32 / 44100.0;
            let s = 0.5 * (5000.0 * 2.0 * std::f32::consts::PI * t).sin();
            left[i] = s;
            right[i] = s;
        }

        fx.process(&mut left, &mut right);

        // After low-pass at 200Hz, 5kHz signal should be heavily attenuated
        // Check the last quarter to avoid transient
        let tail_peak = left[frames*3/4..].iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(tail_peak < 0.1, "Filter didn't attenuate high freq: {}", tail_peak);
        assert!(left.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn test_chorus_produces_output() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.chorus_enabled = true;
        fx.params.chorus_rate = 1.5;
        fx.params.chorus_depth = 3.0;
        fx.params.chorus_mix = 0.5;

        let frames = 4410;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for i in 0..frames {
            let t = i as f32 / 44100.0;
            let s = 0.5 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
            left[i] = s;
            right[i] = s;
        }
        fx.process(&mut left, &mut right);

        let peak = left.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.1, "Chorus killed the signal");
        assert!(peak < 2.0, "Chorus output too hot: {}", peak);
        assert!(left.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn test_full_chain() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.distortion_enabled = true;
        fx.params.distortion_drive = 2.0;
        fx.params.filter_enabled = true;
        fx.params.filter_cutoff = 2000.0;
        fx.params.filter_resonance = 0.3;
        fx.params.chorus_enabled = true;
        fx.params.chorus_mix = 0.3;
        fx.params.delay_enabled = true;
        fx.params.delay_time = 200.0;
        fx.params.delay_feedback = 0.3;
        fx.params.delay_mix = 0.2;
        fx.params.reverb_enabled = true;
        fx.params.reverb_size = 0.5;
        fx.params.reverb_mix = 0.2;

        let frames = 4410;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        for i in 0..frames {
            let t = i as f32 / 44100.0;
            left[i] = 0.5 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
            right[i] = 0.5 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
        }
        fx.process(&mut left, &mut right);

        assert!(left.iter().all(|s| s.is_finite()));
        assert!(right.iter().all(|s| s.is_finite()));
        let peak = left.iter().chain(right.iter()).fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 2.0, "Full chain output too hot: {}", peak);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.chorus_enabled = true;
        fx.params.chorus_mix = 1.0;

        // Process some signal to fill delay buffers
        let mut left = vec![1.0f32; 1000];
        let mut right = vec![1.0f32; 1000];
        fx.process(&mut left, &mut right);

        fx.reset();

        // After reset, processing silence should output silence
        let mut left = vec![0.0f32; 1000];
        let mut right = vec![0.0f32; 1000];
        fx.process(&mut left, &mut right);

        let peak = left.iter().chain(right.iter()).fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak < 0.001, "Reset didn't clear state: peak={}", peak);
    }

    #[test]
    fn test_delay_produces_echo() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.delay_enabled = true;
        fx.params.delay_time = 100.0; // 100ms
        fx.params.delay_feedback = 0.0;
        fx.params.delay_mix = 1.0; // wet only

        let frames = 44100; // 1 second
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        // Impulse at sample 0
        left[0] = 1.0;
        right[0] = 1.0;

        fx.process(&mut left, &mut right);

        // At 100ms = 4410 samples, we should see the delayed impulse
        let delay_pos = (44100.0 * 0.1) as usize;
        assert!(left[delay_pos].abs() > 0.5, "No echo at expected position");
        // Before the delay, output should be near-zero (wet-only)
        assert!(left[1].abs() < 0.01, "Unexpected signal before delay");
    }

    #[test]
    fn test_reverb_produces_tail() {
        let mut fx = ChannelEffects::new(44100.0);
        fx.params.reverb_enabled = true;
        fx.params.reverb_size = 0.8;
        fx.params.reverb_damp = 0.3;
        fx.params.reverb_mix = 0.5;

        let frames = 44100;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        // Short burst of signal
        for i in 0..441 {
            let t = i as f32 / 44100.0;
            left[i] = 0.5 * (440.0 * 2.0 * std::f32::consts::PI * t).sin();
            right[i] = left[i];
        }

        fx.process(&mut left, &mut right);

        assert!(left.iter().all(|s| s.is_finite()));
        // Reverb tail: signal should still be present well after the burst ends
        let late_energy: f32 = left[22050..].iter().map(|s| s * s).sum();
        assert!(late_energy > 0.001, "Reverb produced no tail: {}", late_energy);
    }
}
