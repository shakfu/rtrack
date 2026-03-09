use fundsp::prelude32::*;
use serde::{Deserialize, Serialize};

/// Maximum number of send/return effect buses.
pub const MAX_SEND_BUSES: usize = 2;

/// Parameters for a send bus effect, configurable from UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendBusParams {
    pub enabled: bool,
    pub label: String,
    pub effect_type: SendBusType,
    /// Delay time in ms (for Delay type)
    #[serde(default = "default_bus_delay_time")]
    pub delay_time: f32,
    /// Delay feedback (for Delay type)
    #[serde(default = "default_bus_delay_feedback")]
    pub delay_feedback: f32,
    /// Reverb room size (for Reverb type)
    #[serde(default = "default_bus_reverb_size")]
    pub reverb_size: f32,
    /// Reverb damping (for Reverb type)
    #[serde(default = "default_bus_reverb_damp")]
    pub reverb_damp: f32,
}

fn default_bus_delay_time() -> f32 { 300.0 }
fn default_bus_delay_feedback() -> f32 { 0.4 }
fn default_bus_reverb_size() -> f32 { 0.6 }
fn default_bus_reverb_damp() -> f32 { 0.5 }

impl Default for SendBusParams {
    fn default() -> Self {
        Self {
            enabled: false,
            label: String::new(),
            effect_type: SendBusType::Delay,
            delay_time: 300.0,
            delay_feedback: 0.4,
            reverb_size: 0.6,
            reverb_damp: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendBusType {
    Delay,
    Reverb,
}

/// A send/return effects bus that accumulates audio from multiple channels.
pub struct SendBus {
    pub params: SendBusParams,
    /// Accumulated input buffers (zeroed each callback)
    input_left: Vec<f32>,
    input_right: Vec<f32>,
    /// Delay line for bus delay effect
    delay_buf_l: Vec<f32>,
    delay_buf_r: Vec<f32>,
    delay_write_pos: usize,
    /// Schroeder reverb state (4 combs + 2 allpass, like ChannelEffects)
    comb_bufs_l: [Vec<f32>; 4],
    comb_bufs_r: [Vec<f32>; 4],
    comb_pos: [usize; 4],
    comb_state_l: [f32; 4],
    comb_state_r: [f32; 4],
    ap_bufs_l: [Vec<f32>; 2],
    ap_bufs_r: [Vec<f32>; 2],
    ap_pos: [usize; 2],
    sample_rate: f64,
}

const BUS_COMB_LENGTHS: [usize; 4] = [1116, 1188, 1277, 1356];
const BUS_AP_LENGTHS: [usize; 2] = [225, 556];

impl SendBus {
    pub fn new(sample_rate: f64) -> Self {
        let buf_size = (sample_rate * 2.0) as usize + 1;
        let sr_ratio = sample_rate / 44100.0;
        Self {
            params: SendBusParams::default(),
            input_left: vec![0.0; MAX_CALLBACK_FRAMES],
            input_right: vec![0.0; MAX_CALLBACK_FRAMES],
            delay_buf_l: vec![0.0; buf_size],
            delay_buf_r: vec![0.0; buf_size],
            delay_write_pos: 0,
            comb_bufs_l: BUS_COMB_LENGTHS.map(|l| vec![0.0; (l as f64 * sr_ratio) as usize + 1]),
            comb_bufs_r: BUS_COMB_LENGTHS.map(|l| vec![0.0; (l as f64 * sr_ratio) as usize + 1]),
            comb_pos: [0; 4],
            comb_state_l: [0.0; 4],
            comb_state_r: [0.0; 4],
            ap_bufs_l: BUS_AP_LENGTHS.map(|l| vec![0.0; (l as f64 * sr_ratio) as usize + 1]),
            ap_bufs_r: BUS_AP_LENGTHS.map(|l| vec![0.0; (l as f64 * sr_ratio) as usize + 1]),
            ap_pos: [0; 2],
            sample_rate,
        }
    }

    /// Ensure input buffers are large enough for the given frame count.
    pub fn ensure_size(&mut self, frames: usize) {
        if self.input_left.len() < frames {
            self.input_left.resize(frames, 0.0);
            self.input_right.resize(frames, 0.0);
        }
    }

    /// Zero the input accumulation buffers.
    pub fn clear_inputs(&mut self, frames: usize) {
        for s in self.input_left[..frames].iter_mut() { *s = 0.0; }
        for s in self.input_right[..frames].iter_mut() { *s = 0.0; }
    }

    /// Add a channel's output to this bus's input, scaled by send level.
    pub fn add_send(&mut self, left: &[f32], right: &[f32], send_level: f32) {
        let frames = Ord::min(Ord::min(left.len(), right.len()), self.input_left.len());
        for i in 0..frames {
            self.input_left[i] += left[i] * send_level;
            self.input_right[i] += right[i] * send_level;
        }
    }

    /// Process the accumulated input through the bus effect and add to master output.
    pub fn process_to_master(&mut self, master_left: &mut [f32], master_right: &mut [f32], frames: usize) {
        if !self.params.enabled {
            return;
        }
        match self.params.effect_type {
            SendBusType::Delay => self.process_delay(master_left, master_right, frames),
            SendBusType::Reverb => self.process_reverb(master_left, master_right, frames),
        }
    }

    fn process_delay(&mut self, master_left: &mut [f32], master_right: &mut [f32], frames: usize) {
        let buf_len = self.delay_buf_l.len();
        let delay_samples = (self.params.delay_time as f64 / 1000.0 * self.sample_rate)
            .clamp(1.0, (buf_len - 1) as f64) as usize;
        let feedback = self.params.delay_feedback.clamp(0.0, 0.95);

        for i in 0..frames {
            let read_pos = (self.delay_write_pos + buf_len - delay_samples) % buf_len;
            let dl = self.delay_buf_l[read_pos];
            let dr = self.delay_buf_r[read_pos];

            self.delay_buf_l[self.delay_write_pos] = self.input_left[i] + dl * feedback;
            self.delay_buf_r[self.delay_write_pos] = self.input_right[i] + dr * feedback;
            self.delay_write_pos = (self.delay_write_pos + 1) % buf_len;

            master_left[i] += dl;
            master_right[i] += dr;
        }
    }

    fn process_reverb(&mut self, master_left: &mut [f32], master_right: &mut [f32], frames: usize) {
        let size = self.params.reverb_size.clamp(0.0, 1.0);
        let damp = self.params.reverb_damp.clamp(0.0, 1.0);
        let feedback = 0.7 + size * 0.28;

        for i in 0..frames {
            let l = self.input_left[i];
            let r = self.input_right[i];

            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for c in 0..4 {
                let blen = self.comb_bufs_l[c].len();
                let pos = self.comb_pos[c];
                let out_l = self.comb_bufs_l[c][pos];
                let out_r = self.comb_bufs_r[c][pos];
                self.comb_state_l[c] = out_l * (1.0 - damp) + self.comb_state_l[c] * damp;
                self.comb_state_r[c] = out_r * (1.0 - damp) + self.comb_state_r[c] * damp;
                self.comb_bufs_l[c][pos] = l + self.comb_state_l[c] * feedback;
                self.comb_bufs_r[c][pos] = r + self.comb_state_r[c] * feedback;
                self.comb_pos[c] = (pos + 1) % blen;
                sum_l += out_l;
                sum_r += out_r;
            }

            let mut ap_l = sum_l;
            let mut ap_r = sum_r;
            for a in 0..2 {
                let blen = self.ap_bufs_l[a].len();
                let pos = self.ap_pos[a];
                let buf_l = self.ap_bufs_l[a][pos];
                let buf_r = self.ap_bufs_r[a][pos];
                self.ap_bufs_l[a][pos] = ap_l + buf_l * 0.5;
                self.ap_bufs_r[a][pos] = ap_r + buf_r * 0.5;
                self.ap_pos[a] = (pos + 1) % blen;
                ap_l = buf_l - ap_l * 0.5;
                ap_r = buf_r - ap_r * 0.5;
            }

            master_left[i] += ap_l;
            master_right[i] += ap_r;
        }
    }
}

/// Maximum frames per callback (must match audio/mod.rs).
const MAX_CALLBACK_FRAMES: usize = 4096;

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
            multipass::<U2>() & ((delay(0.08) * 0.15) | (delay(0.12) * 0.15)),
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

    #[test]
    fn test_send_bus_creates() {
        let bus = SendBus::new(44100.0);
        assert!(!bus.params.enabled);
        assert_eq!(bus.params.effect_type, SendBusType::Delay);
    }

    #[test]
    fn test_send_bus_disabled_produces_silence() {
        let mut bus = SendBus::new(44100.0);
        let frames = 256;
        bus.ensure_size(frames);
        bus.clear_inputs(frames);
        // Feed some signal
        let signal: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        bus.add_send(&signal, &signal, 1.0);
        let mut master_l = vec![0.0f32; frames];
        let mut master_r = vec![0.0f32; frames];
        bus.process_to_master(&mut master_l, &mut master_r, frames);
        // Disabled bus should not add anything
        let peak = master_l.iter().chain(master_r.iter()).fold(0.0f32, |a, &s| a.max(s.abs()));
        assert_eq!(peak, 0.0, "Disabled bus should produce no output");
    }

    #[test]
    fn test_send_bus_delay_produces_output() {
        let mut bus = SendBus::new(44100.0);
        bus.params.enabled = true;
        bus.params.effect_type = SendBusType::Delay;
        bus.params.delay_time = 100.0; // 100ms
        bus.params.delay_feedback = 0.3;

        let frames = 44100; // 1 second
        bus.ensure_size(frames);
        bus.clear_inputs(frames);

        // Feed an impulse at the start
        let mut input_l = vec![0.0f32; frames];
        let mut input_r = vec![0.0f32; frames];
        input_l[0] = 1.0;
        input_r[0] = 1.0;
        bus.add_send(&input_l, &input_r, 1.0);

        let mut master_l = vec![0.0f32; frames];
        let mut master_r = vec![0.0f32; frames];
        bus.process_to_master(&mut master_l, &mut master_r, frames);

        // After 100ms (~4410 samples), we should see the delayed impulse
        let delay_sample = (0.1 * 44100.0) as usize;
        let has_delayed = master_l[delay_sample..delay_sample + 10].iter().any(|&s| s.abs() > 0.1);
        assert!(has_delayed, "Delay bus should produce delayed output around sample {}", delay_sample);

        let has_nan = master_l.iter().chain(master_r.iter()).any(|s| !s.is_finite());
        assert!(!has_nan, "Delay bus produced NaN/Inf");
    }

    #[test]
    fn test_send_bus_reverb_produces_output() {
        let mut bus = SendBus::new(44100.0);
        bus.params.enabled = true;
        bus.params.effect_type = SendBusType::Reverb;
        bus.params.reverb_size = 0.5;
        bus.params.reverb_damp = 0.5;

        let frames = 44100;
        bus.ensure_size(frames);
        bus.clear_inputs(frames);

        // Feed a short burst of signal
        let mut input_l = vec![0.0f32; frames];
        let mut input_r = vec![0.0f32; frames];
        for i in 0..100 {
            let t = i as f32 / 44100.0;
            let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            input_l[i] = s;
            input_r[i] = s;
        }
        bus.add_send(&input_l, &input_r, 1.0);

        let mut master_l = vec![0.0f32; frames];
        let mut master_r = vec![0.0f32; frames];
        bus.process_to_master(&mut master_l, &mut master_r, frames);

        // Reverb should produce output beyond the initial 100 samples
        let tail_energy: f32 = master_l[1000..5000].iter().map(|s| s * s).sum();
        assert!(tail_energy > 0.001, "Reverb bus should produce tail energy, got {}", tail_energy);

        let has_nan = master_l.iter().chain(master_r.iter()).any(|s| !s.is_finite());
        assert!(!has_nan, "Reverb bus produced NaN/Inf");
    }

    #[test]
    fn test_send_bus_add_send_accumulates() {
        let mut bus = SendBus::new(44100.0);
        let frames = 64;
        bus.ensure_size(frames);
        bus.clear_inputs(frames);

        let signal = vec![1.0f32; frames];
        bus.add_send(&signal, &signal, 0.5);
        bus.add_send(&signal, &signal, 0.3);

        // Input should have accumulated: 0.5 + 0.3 = 0.8
        assert!((bus.input_left[0] - 0.8).abs() < 1e-6);
        assert!((bus.input_right[0] - 0.8).abs() < 1e-6);
    }
}
