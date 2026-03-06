/// Built-in subtractive synthesizer with ADSR envelopes and state-variable filter.
/// Provides 8 waveform patches, each with per-patch envelope and filter parameters.
/// Voice management is manual (one voice per tracker channel, polyphony via channels).

const MAX_VOICES: usize = 32;
const VOICE_GAIN: f32 = 0.25;

/// Available synth patches
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patch {
    Saw,
    Square,
    Sine,
    Triangle,
    Pulse,
    FmBell,
    Organ,
    Noise,
}

impl Patch {
    pub fn from_program(program: u8) -> Self {
        match program % 8 {
            0 => Patch::Saw,
            1 => Patch::Square,
            2 => Patch::Sine,
            3 => Patch::Triangle,
            4 => Patch::Pulse,
            5 => Patch::FmBell,
            6 => Patch::Organ,
            7 => Patch::Noise,
            _ => Patch::Saw,
        }
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Patch::Saw => "Saw",
            Patch::Square => "Square",
            Patch::Sine => "Sine",
            Patch::Triangle => "Triangle",
            Patch::Pulse => "Pulse",
            Patch::FmBell => "FM Bell",
            Patch::Organ => "Organ",
            Patch::Noise => "Noise",
        }
    }
}

/// ADSR envelope parameters (all in seconds, sustain is 0..1 level)
#[derive(Clone, Copy)]
struct EnvParams {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// Per-patch synthesis parameters
#[derive(Clone, Copy)]
struct PatchParams {
    env: EnvParams,
    /// Filter cutoff as multiplier of note frequency (e.g. 4.0 = 4x note freq)
    filter_cutoff_mul: f32,
    /// Filter resonance (0..1, higher = more resonant)
    filter_resonance: f32,
    /// How much the envelope modulates filter cutoff (in octaves)
    filter_env_amount: f32,
    /// Second oscillator detune in cents (for chorus/thickening)
    detune_cents: f32,
}

fn patch_params(patch: Patch) -> PatchParams {
    match patch {
        Patch::Saw => PatchParams {
            env: EnvParams { attack: 0.005, decay: 0.1, sustain: 0.7, release: 0.15 },
            filter_cutoff_mul: 4.0,
            filter_resonance: 0.3,
            filter_env_amount: 2.0,
            detune_cents: 8.0,
        },
        Patch::Square => PatchParams {
            env: EnvParams { attack: 0.005, decay: 0.15, sustain: 0.6, release: 0.12 },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.4,
            filter_env_amount: 1.5,
            detune_cents: 5.0,
        },
        Patch::Sine => PatchParams {
            env: EnvParams { attack: 0.01, decay: 0.0, sustain: 1.0, release: 0.08 },
            filter_cutoff_mul: 20.0, // wide open for sine
            filter_resonance: 0.0,
            filter_env_amount: 0.0,
            detune_cents: 0.0,
        },
        Patch::Triangle => PatchParams {
            env: EnvParams { attack: 0.01, decay: 0.2, sustain: 0.8, release: 0.2 },
            filter_cutoff_mul: 6.0,
            filter_resonance: 0.2,
            filter_env_amount: 1.0,
            detune_cents: 6.0,
        },
        Patch::Pulse => PatchParams {
            env: EnvParams { attack: 0.003, decay: 0.08, sustain: 0.5, release: 0.1 },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.5,
            filter_env_amount: 2.5,
            detune_cents: 10.0,
        },
        Patch::FmBell => PatchParams {
            env: EnvParams { attack: 0.001, decay: 1.5, sustain: 0.0, release: 0.5 },
            filter_cutoff_mul: 12.0,
            filter_resonance: 0.1,
            filter_env_amount: 3.0,
            detune_cents: 0.0,
        },
        Patch::Organ => PatchParams {
            env: EnvParams { attack: 0.008, decay: 0.0, sustain: 1.0, release: 0.05 },
            filter_cutoff_mul: 8.0,
            filter_resonance: 0.15,
            filter_env_amount: 0.5,
            detune_cents: 4.0,
        },
        Patch::Noise => PatchParams {
            env: EnvParams { attack: 0.002, decay: 0.3, sustain: 0.0, release: 0.1 },
            filter_cutoff_mul: 2.0,
            filter_resonance: 0.6,
            filter_env_amount: 3.0,
            detune_cents: 0.0,
        },
    }
}

/// Convert MIDI note number to frequency in Hz
fn midi_to_freq(note: f32) -> f32 {
    440.0 * 2.0_f32.powf((note - 69.0) / 12.0)
}

/// ADSR envelope stage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

/// State-variable filter state
#[derive(Clone)]
struct SvfState {
    low: f32,
    band: f32,
}

impl SvfState {
    fn new() -> Self {
        Self { low: 0.0, band: 0.0 }
    }

    /// Process one sample through a 2x oversampled SVF low-pass
    #[inline]
    fn tick_lowpass(&mut self, input: f32, cutoff_hz: f32, resonance: f32, sample_rate: f32) -> f32 {
        // Clamp cutoff to Nyquist
        let max_freq = sample_rate * 0.45;
        let freq = cutoff_hz.min(max_freq).max(20.0);
        let f = (2.0 * (std::f32::consts::PI * freq / (sample_rate * 2.0)).sin()).min(0.95);
        let q = 1.0 - resonance.clamp(0.0, 0.95);

        // 2x oversample for stability at high frequencies
        for _ in 0..2 {
            let high = input - self.low - q * self.band;
            self.band += f * high;
            self.low += f * self.band;
        }
        // Guard against filter blowup
        if !self.low.is_finite() || !self.band.is_finite() {
            self.low = 0.0;
            self.band = 0.0;
        }
        self.low
    }
}

/// Simple LCG noise generator (no std dependency in hot path)
#[derive(Clone)]
struct NoiseGen {
    state: u32,
}

impl NoiseGen {
    fn new(seed: u32) -> Self {
        Self { state: seed.wrapping_add(1) }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        // LCG: fast, low-quality but fine for audio noise
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        // Map to -1..1
        (self.state as i32) as f32 / i32::MAX as f32
    }
}

/// A single synth voice
#[derive(Clone)]
struct Voice {
    active: bool,
    channel: u8,
    note: u8,
    frequency: f32,
    velocity: f32,
    patch: Patch,
    params: PatchParams,

    // Oscillator state
    phase: f64,
    phase2: f64, // detuned second oscillator
    fm_mod_phase: f64, // for FM bell

    // ADSR envelope
    env_stage: EnvStage,
    env_level: f32,

    // Filter
    filter: SvfState,

    // Noise generator (for Noise patch)
    noise: NoiseGen,

    sample_rate: f32,
}

impl Voice {
    fn new(channel: u8, note: u8, velocity: f32, patch: Patch, sample_rate: f32) -> Self {
        let frequency = midi_to_freq(note as f32);
        let params = patch_params(patch);
        Self {
            active: true,
            channel,
            note,
            frequency,
            velocity,
            patch,
            params,
            phase: 0.0,
            phase2: 0.0,
            fm_mod_phase: 0.0,
            env_stage: EnvStage::Attack,
            env_level: 0.0,
            filter: SvfState::new(),
            noise: NoiseGen::new(note as u32 * 7919 + channel as u32 * 104729),
            sample_rate,
        }
    }

    /// Trigger the release phase
    fn release(&mut self) {
        if self.env_stage != EnvStage::Off {
            self.env_stage = EnvStage::Release;
        }
    }

    /// Generate one stereo sample pair
    #[inline]
    fn tick(&mut self) -> (f32, f32) {
        if !self.active || self.env_stage == EnvStage::Off {
            return (0.0, 0.0);
        }

        let sr = self.sample_rate;
        let freq = self.frequency;

        // -- ADSR envelope --
        let env = &self.params.env;
        match self.env_stage {
            EnvStage::Attack => {
                if env.attack > 0.0 {
                    self.env_level += 1.0 / (env.attack * sr);
                    if self.env_level >= 1.0 {
                        self.env_level = 1.0;
                        self.env_stage = EnvStage::Decay;
                    }
                } else {
                    self.env_level = 1.0;
                    self.env_stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                if env.decay > 0.0 {
                    self.env_level -= (1.0 - env.sustain) / (env.decay * sr);
                    if self.env_level <= env.sustain {
                        self.env_level = env.sustain;
                        self.env_stage = EnvStage::Sustain;
                    }
                } else {
                    self.env_level = env.sustain;
                    self.env_stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => {
                // Hold at sustain level
            }
            EnvStage::Release => {
                if env.release > 0.0 {
                    self.env_level -= self.env_level / (env.release * sr);
                    if self.env_level < 0.001 {
                        self.env_level = 0.0;
                        self.env_stage = EnvStage::Off;
                        self.active = false;
                        return (0.0, 0.0);
                    }
                } else {
                    self.env_level = 0.0;
                    self.env_stage = EnvStage::Off;
                    self.active = false;
                    return (0.0, 0.0);
                }
            }
            EnvStage::Off => {
                return (0.0, 0.0);
            }
        }

        // -- Oscillator --
        let phase_inc = freq as f64 / sr as f64;
        let detune_ratio = 2.0_f64.powf(self.params.detune_cents as f64 / 1200.0);
        let phase_inc2 = freq as f64 * detune_ratio / sr as f64;

        self.phase += phase_inc;
        if self.phase >= 1.0 { self.phase -= 1.0; }
        self.phase2 += phase_inc2;
        if self.phase2 >= 1.0 { self.phase2 -= 1.0; }

        let osc = match self.patch {
            Patch::Saw => {
                let s1 = polyblep_saw(self.phase, phase_inc);
                if self.params.detune_cents > 0.0 {
                    let s2 = polyblep_saw(self.phase2, phase_inc2);
                    (s1 + s2) * 0.5
                } else {
                    s1
                }
            }
            Patch::Square => {
                let s1 = polyblep_square(self.phase, phase_inc);
                if self.params.detune_cents > 0.0 {
                    let s2 = polyblep_square(self.phase2, phase_inc2);
                    (s1 + s2) * 0.5
                } else {
                    s1
                }
            }
            Patch::Sine => {
                (self.phase as f32 * std::f32::consts::TAU).sin()
            }
            Patch::Triangle => {
                let s1 = triangle(self.phase);
                if self.params.detune_cents > 0.0 {
                    let s2 = triangle(self.phase2);
                    (s1 + s2) * 0.5
                } else {
                    s1
                }
            }
            Patch::Pulse => {
                // 25% duty cycle pulse
                let s1 = polyblep_pulse(self.phase, phase_inc, 0.25);
                if self.params.detune_cents > 0.0 {
                    let s2 = polyblep_pulse(self.phase2, phase_inc2, 0.25);
                    (s1 + s2) * 0.5
                } else {
                    s1
                }
            }
            Patch::FmBell => {
                // FM: carrier = sine at freq, modulator = sine at freq*3.5
                let fm_ratio = 3.5;
                let mod_index = 2.0 * self.env_level; // FM depth follows envelope
                let fm_inc = freq as f64 * fm_ratio / sr as f64;
                self.fm_mod_phase += fm_inc;
                if self.fm_mod_phase >= 1.0 { self.fm_mod_phase -= 1.0; }
                let modulator = (self.fm_mod_phase as f32 * std::f32::consts::TAU).sin();
                let mod_freq = self.phase as f32 + modulator * mod_index;
                (mod_freq * std::f32::consts::TAU).sin()
            }
            Patch::Organ => {
                // Additive: fundamental + 2nd + 3rd harmonics
                let f1 = (self.phase as f32 * std::f32::consts::TAU).sin();
                let f2 = (self.phase as f32 * 2.0 * std::f32::consts::TAU).sin();
                let f3 = (self.phase as f32 * 3.0 * std::f32::consts::TAU).sin();
                f1 * 0.6 + f2 * 0.3 + f3 * 0.1
            }
            Patch::Noise => {
                self.noise.next()
            }
        };

        // -- Filter --
        // Base cutoff = note frequency * multiplier
        // Envelope modulates cutoff by filter_env_amount octaves
        let env_mod = self.env_level * self.params.filter_env_amount;
        let cutoff = freq * self.params.filter_cutoff_mul * 2.0_f32.powf(env_mod);
        // Velocity also opens the filter
        let vel_mod = 0.5 + 0.5 * self.velocity;
        let final_cutoff = cutoff * vel_mod;

        let filtered = self.filter.tick_lowpass(osc, final_cutoff, self.params.filter_resonance, sr);

        // -- Output --
        let gain = self.velocity * self.env_level * VOICE_GAIN;
        let sample = filtered * gain;
        (sample, sample) // mono -> stereo
    }
}

// -- Anti-aliased oscillator functions (PolyBLEP) --

/// PolyBLEP correction term for reducing aliasing at discontinuities
#[inline]
fn polyblep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// Band-limited sawtooth via PolyBLEP
#[inline]
fn polyblep_saw(phase: f64, phase_inc: f64) -> f32 {
    let naive = 2.0 * phase - 1.0;
    (naive - polyblep(phase, phase_inc)) as f32
}

/// Band-limited square wave via PolyBLEP
#[inline]
fn polyblep_square(phase: f64, phase_inc: f64) -> f32 {
    let naive = if phase < 0.5 { 1.0 } else { -1.0 };
    let mut out = naive;
    out += polyblep(phase, phase_inc);
    out -= polyblep((phase + 0.5) % 1.0, phase_inc);
    out as f32
}

/// Band-limited pulse wave with variable duty cycle via PolyBLEP
#[inline]
fn polyblep_pulse(phase: f64, phase_inc: f64, duty: f64) -> f32 {
    let naive = if phase < duty { 1.0 } else { -1.0 };
    let mut out = naive;
    out += polyblep(phase, phase_inc);
    out -= polyblep((phase + (1.0 - duty)) % 1.0, phase_inc);
    out as f32
}

/// Triangle wave (integrated square, no aliasing issues)
#[inline]
fn triangle(phase: f64) -> f32 {
    let p = phase * 4.0;
    let out = if p < 1.0 {
        p
    } else if p < 3.0 {
        2.0 - p
    } else {
        p - 4.0
    };
    out as f32
}

/// Built-in synthesizer with ADSR envelopes and SVF filter.
pub struct BuiltinSynth {
    voices: Vec<Voice>,
    /// Current program per channel (selects waveform patch)
    programs: [u8; 16],
    sample_rate: f32,
}

impl BuiltinSynth {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            programs: [0; 16],
            sample_rate: sample_rate as f32,
        }
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        // Kill existing voice on this channel (tracker: one note per channel)
        self.note_off_all_channel(channel);

        let patch = Patch::from_program(self.programs[(channel & 0x0F) as usize]);
        let vel = velocity as f32 / 127.0;
        let voice = Voice::new(channel, note, vel, patch, self.sample_rate);

        // Evict oldest inactive voice if at capacity
        if self.voices.len() >= MAX_VOICES {
            if let Some(idx) = self.voices.iter().position(|v| !v.active) {
                self.voices.remove(idx);
            } else {
                self.voices.remove(0); // steal oldest
            }
        }
        self.voices.push(voice);
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel && voice.note == note {
                voice.release();
            }
        }
    }

    pub fn note_off_all_channel(&mut self, channel: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                voice.release();
            }
        }
    }

    pub fn note_off_all(&mut self) {
        for voice in &mut self.voices {
            if voice.active {
                voice.release();
            }
        }
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.programs[(channel & 0x0F) as usize] = program;
    }

    /// Render one stereo sample pair. Called from the audio callback.
    #[inline]
    pub fn render_sample(&mut self) -> (f32, f32) {
        let mut left = 0.0_f32;
        let mut right = 0.0_f32;

        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let (l, r) = voice.tick();
            left += l;
            right += r;
        }

        // Garbage collect dead voices periodically
        self.voices.retain(|v| v.active || v.env_stage != EnvStage::Off);

        (left, right)
    }

    /// Get active voice count (for debugging/UI)
    #[allow(dead_code)]
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_to_freq() {
        let a4 = midi_to_freq(69.0);
        assert!((a4 - 440.0).abs() < 0.01);

        let a5 = midi_to_freq(81.0);
        assert!((a5 - 880.0).abs() < 0.1);
    }

    #[test]
    fn test_patch_from_program() {
        assert!(matches!(Patch::from_program(0), Patch::Saw));
        assert!(matches!(Patch::from_program(2), Patch::Sine));
        assert!(matches!(Patch::from_program(8), Patch::Saw)); // wraps
    }

    #[test]
    fn test_synth_voice_lifecycle() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        assert_eq!(synth.voices.len(), 1);
        assert!(synth.voices[0].active);

        synth.note_off(0, 60);
        assert_eq!(synth.voices[0].env_stage, EnvStage::Release);

        // Render until voice dies
        for _ in 0..44100 {
            synth.render_sample();
        }
        assert_eq!(synth.active_voice_count(), 0);
    }

    #[test]
    fn test_synth_polyphony_across_channels() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(1, 64, 100);
        synth.note_on(2, 67, 100);
        assert_eq!(synth.active_voice_count(), 3);

        synth.note_off_all();
        // Voices are in Release, not immediately dead
        for _ in 0..44100 {
            synth.render_sample();
        }
        assert_eq!(synth.active_voice_count(), 0);
    }

    #[test]
    fn test_synth_same_channel_replaces_note() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        assert_eq!(synth.active_voice_count(), 1);

        synth.note_on(0, 64, 100);
        // Old voice enters release, new voice is active
        let active: Vec<_> = synth.voices.iter().filter(|v| v.active && v.env_stage != EnvStage::Release).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].note, 64);
    }

    #[test]
    fn test_synth_note_off_channel() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(1, 64, 100);
        synth.note_off_all_channel(0);

        let active_non_releasing: Vec<_> = synth.voices.iter()
            .filter(|v| v.active && v.env_stage != EnvStage::Release)
            .collect();
        assert_eq!(active_non_releasing.len(), 1);
        assert_eq!(active_non_releasing[0].channel, 1);
    }

    #[test]
    fn test_program_change() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.program_change(0, 2);
        assert_eq!(synth.programs[0], 2);
    }

    #[test]
    fn test_all_patches_produce_audio() {
        let sr = 44100.0;
        for prog in 0..8u8 {
            let mut synth = BuiltinSynth::new(sr);
            synth.program_change(0, prog);
            synth.note_on(0, 60, 127);

            let mut peak = 0.0_f32;
            for _ in 0..4410 {
                let (l, r) = synth.render_sample();
                peak = peak.max(l.abs()).max(r.abs());
                assert!(l.is_finite(), "Patch {}: non-finite L sample", prog);
                assert!(r.is_finite(), "Patch {}: non-finite R sample", prog);
            }
            assert!(peak > 0.01, "Patch {} ({:?}) near-silent: peak={:.6}",
                prog, Patch::from_program(prog), peak);
            assert!(peak < 1.0, "Patch {} ({:?}) clips: peak={:.4}",
                prog, Patch::from_program(prog), peak);
        }
    }

    #[test]
    fn test_adsr_envelope_shape() {
        let sr = 44100.0;
        let mut synth = BuiltinSynth::new(sr);
        synth.program_change(0, 2); // Sine (cleanest envelope test)
        synth.note_on(0, 69, 127);

        // Attack: level should rise
        for _ in 0..200 {
            synth.render_sample();
        }
        let attack_level = synth.voices[0].env_level;
        assert!(attack_level > 0.0, "Envelope should be rising during attack");

        // Wait for attack to finish and enter sustain
        for _ in 0..4410 {
            synth.render_sample();
        }
        let sustain_level = synth.voices[0].env_level;
        // Sine patch has sustain=1.0, so should be at ~1.0
        assert!(sustain_level > 0.9, "Sine sustain should be near 1.0, got {:.4}", sustain_level);

        // Release
        synth.note_off(0, 69);
        let mut prev_env = synth.voices[0].env_level;
        for _ in 0..1000 {
            synth.render_sample();
            let cur = synth.voices.get(0).map(|v| v.env_level).unwrap_or(0.0);
            assert!(cur <= prev_env + 0.001, "Envelope should decrease during release");
            prev_env = cur;
        }
    }

    #[test]
    fn test_render_output_levels() {
        let sr = 44100.0;
        let mut synth = BuiltinSynth::new(sr);
        synth.note_on(0, 69, 127); // A4 max velocity, Saw patch

        let frames = 4410;
        let mut peak = 0.0_f32;
        let mut has_nonzero = false;
        for _ in 0..frames {
            let (l, _r) = synth.render_sample();
            peak = peak.max(l.abs());
            if l.abs() > 1e-6 { has_nonzero = true; }
        }
        assert!(has_nonzero, "Synth produced only silence");
        assert!(peak < 1.0, "Peak level {} exceeds 1.0 (clipping)", peak);
    }

    #[test]
    fn test_note_off_fade_behavior() {
        let sr = 44100.0;
        let mut synth = BuiltinSynth::new(sr);
        synth.program_change(0, 2); // Sine
        synth.note_on(0, 69, 127);

        // Let note settle
        for _ in 0..4410 {
            synth.render_sample();
        }

        // Capture pre-note-off level
        let mut pre_off_peak = 0.0_f32;
        for _ in 0..100 {
            let (l, _) = synth.render_sample();
            pre_off_peak = pre_off_peak.max(l.abs());
        }

        synth.note_off(0, 69);

        // First 10ms after note-off
        let window = 441;
        let mut post_samples = Vec::new();
        for _ in 0..window {
            let (l, _) = synth.render_sample();
            post_samples.push(l);
        }

        let first_peak = post_samples[..44].iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
        let last_peak = post_samples[window-44..].iter().fold(0.0_f32, |a, &s| a.max(s.abs()));

        // Fade should not be instant cutoff
        assert!(first_peak > pre_off_peak * 0.2,
            "Note-off caused instant cutoff! first_window={:.6}, pre_peak={:.6}",
            first_peak, pre_off_peak);
        // Should be decreasing
        assert!(last_peak < first_peak,
            "Fade not decreasing: first={:.6}, last={:.6}", first_peak, last_peak);
    }

    #[test]
    fn test_full_pipeline() {
        use crate::audio::effects::EffectsChain;

        let sr = 48000.0;
        let mut synth = BuiltinSynth::new(sr);
        let mut effects = EffectsChain::new(sr);

        synth.note_on(0, 69, 127);

        let chunk = 512;
        let total = 24000;
        let mut all_left = Vec::with_capacity(total);

        for _ in 0..(total / chunk) {
            let mut left = vec![0f32; chunk];
            let mut right = vec![0f32; chunk];
            for i in 0..chunk {
                let (l, r) = synth.render_sample();
                left[i] = l;
                right[i] = r;
            }
            effects.process(&mut left, &mut right);
            all_left.extend_from_slice(&left);
        }

        let peak = all_left.iter().fold(0f32, |a, &s| a.max(s.abs()));
        let rms = (all_left.iter().map(|&s| (s * s) as f64).sum::<f64>() / all_left.len() as f64).sqrt();

        assert!(peak < 1.0, "Pipeline clips: {:.4}", peak);
        assert!(rms > 0.01, "Pipeline silent: {:.6}", rms);
    }

    #[test]
    fn test_polyblep_saw_no_dc_offset() {
        // Rendered saw should have near-zero DC offset over a full cycle
        let sr = 44100.0;
        let freq = 440.0;
        let samples_per_cycle = (sr / freq) as usize;
        let mut phase = 0.0_f64;
        let phase_inc = freq as f64 / sr as f64;
        let mut sum = 0.0_f64;
        for _ in 0..samples_per_cycle * 10 {
            let s = polyblep_saw(phase, phase_inc);
            sum += s as f64;
            phase += phase_inc;
            if phase >= 1.0 { phase -= 1.0; }
        }
        let dc = sum / (samples_per_cycle * 10) as f64;
        assert!(dc.abs() < 0.02, "Saw DC offset too high: {:.6}", dc);
    }

    #[test]
    fn test_velocity_affects_output() {
        let sr = 44100.0;

        let mut synth_loud = BuiltinSynth::new(sr);
        synth_loud.note_on(0, 60, 127);
        let mut peak_loud = 0.0_f32;
        for _ in 0..2000 {
            let (l, _) = synth_loud.render_sample();
            peak_loud = peak_loud.max(l.abs());
        }

        let mut synth_quiet = BuiltinSynth::new(sr);
        synth_quiet.note_on(0, 60, 32);
        let mut peak_quiet = 0.0_f32;
        for _ in 0..2000 {
            let (l, _) = synth_quiet.render_sample();
            peak_quiet = peak_quiet.max(l.abs());
        }

        assert!(peak_loud > peak_quiet * 1.5,
            "Velocity should affect volume: loud={:.4}, quiet={:.4}", peak_loud, peak_quiet);
    }

    #[test]
    fn test_filter_affects_brightness() {
        // Saw patch (rich harmonics) should sound different from Sine (no harmonics)
        // We compare by measuring successive-sample difference RMS (higher = brighter/more HF content)
        let sr = 44100.0;
        let frames = 44100; // 1 second for stable comparison

        let mut synth_saw = BuiltinSynth::new(sr);
        synth_saw.program_change(0, 0); // Saw
        synth_saw.note_on(0, 48, 100); // C3

        let mut synth_sine = BuiltinSynth::new(sr);
        synth_sine.program_change(0, 2); // Sine
        synth_sine.note_on(0, 48, 100);

        let mut saw_diff_sq_sum = 0.0_f64;
        let mut sine_diff_sq_sum = 0.0_f64;
        let mut prev_saw = 0.0_f32;
        let mut prev_sine = 0.0_f32;

        for i in 0..frames {
            let (sl, _) = synth_saw.render_sample();
            let (si, _) = synth_sine.render_sample();
            if i > 0 {
                let d_saw = (sl - prev_saw) as f64;
                let d_sine = (si - prev_sine) as f64;
                saw_diff_sq_sum += d_saw * d_saw;
                sine_diff_sq_sum += d_sine * d_sine;
            }
            prev_saw = sl;
            prev_sine = si;
        }

        let saw_diff_rms = (saw_diff_sq_sum / frames as f64).sqrt();
        let sine_diff_rms = (sine_diff_sq_sum / frames as f64).sqrt();

        assert!(saw_diff_rms > sine_diff_rms,
            "Saw should have more HF content than sine: saw_diff_rms={:.6}, sine_diff_rms={:.6}",
            saw_diff_rms, sine_diff_rms);
    }
}
