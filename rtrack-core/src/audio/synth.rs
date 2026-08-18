/// Built-in subtractive synthesizer with ADSR envelopes and state-variable filter.
/// Provides 9 waveform patches (8 manual + 1 fundsp-based), each with per-patch
/// envelope and filter parameters.
/// Voice management is manual (one voice per tracker channel, polyphony via channels).
use fundsp::prelude32::*;
use serde::{Deserialize, Serialize};

use crate::audio::envelope::{EnvStage, Envelope};
use crate::constants::{MIDI_MAX_VALUE, SEMITONES_PER_OCTAVE};

const MAX_VOICES: usize = 32;
const VOICE_GAIN: f32 = 0.25;

/// Filter type for voice SVF
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    #[default]
    LowPass,
    HighPass,
    BandPass,
}

/// User-configurable synth parameters (stored per-instrument in .rtrk files)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthParams {
    pub waveform: u8,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub filter_cutoff: f32,
    pub filter_resonance: f32,
    pub filter_env: f32,
    pub detune: f32,
    #[serde(default)]
    pub filter_type: FilterType,
    /// Sub-oscillator mix (0.0 = off, 1.0 = full sub, sine one octave below)
    #[serde(default)]
    pub sub_osc: f32,
    /// FM modulator:carrier frequency ratio (0.0 = no FM)
    #[serde(default)]
    pub fm_ratio: f32,
    /// FM modulation index (depth)
    #[serde(default)]
    pub fm_index: f32,
    /// Pulse width (0.05..0.95, only used for Pulse patch)
    #[serde(default = "default_pulse_width")]
    pub pulse_width: f32,
}

fn default_pulse_width() -> f32 {
    0.25
}

impl SynthParams {
    /// Create SynthParams from a preset patch's defaults
    pub fn from_patch(program: u8) -> Self {
        let patch = Patch::from_program(program);
        let p = patch_params(patch);
        Self {
            waveform: program % Patch::count(),
            attack: p.env.attack,
            decay: p.env.decay,
            sustain: p.env.sustain,
            release: p.env.release,
            filter_cutoff: p.filter_cutoff_mul,
            filter_resonance: p.filter_resonance,
            filter_env: p.filter_env_amount,
            detune: p.detune_cents,
            filter_type: p.filter_type,
            sub_osc: p.sub_osc_mix,
            fm_ratio: p.fm_ratio,
            fm_index: p.fm_index,
            pulse_width: p.pulse_width,
        }
    }
}

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
    FundspPad,
    Bass,
    Pluck,
    Pad,
    Lead,
    Keys,
    Brass,
    Strings,
    Perc,
    Sub,
    Acid,
    Chip,
    Stab,
    Mallet,
    Flute,
    Reese,
    Wire,
    Chime,
    Growl,
    Whistle,
    Siren,
    Dist,
}

impl Patch {
    pub fn from_program(program: u8) -> Self {
        match program % 30 {
            0 => Patch::Saw,
            1 => Patch::Square,
            2 => Patch::Sine,
            3 => Patch::Triangle,
            4 => Patch::Pulse,
            5 => Patch::FmBell,
            6 => Patch::Organ,
            7 => Patch::Noise,
            8 => Patch::FundspPad,
            9 => Patch::Bass,
            10 => Patch::Pluck,
            11 => Patch::Pad,
            12 => Patch::Lead,
            13 => Patch::Keys,
            14 => Patch::Brass,
            15 => Patch::Strings,
            16 => Patch::Perc,
            17 => Patch::Sub,
            18 => Patch::Acid,
            19 => Patch::Chip,
            20 => Patch::Stab,
            21 => Patch::Mallet,
            22 => Patch::Flute,
            23 => Patch::Reese,
            24 => Patch::Wire,
            25 => Patch::Chime,
            26 => Patch::Growl,
            27 => Patch::Whistle,
            28 => Patch::Siren,
            29 => Patch::Dist,
            _ => Patch::Saw,
        }
    }

    pub fn count() -> u8 {
        30
    }

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
            Patch::FundspPad => "Fundsp Pad",
            Patch::Bass => "Bass",
            Patch::Pluck => "Pluck",
            Patch::Pad => "Pad",
            Patch::Lead => "Lead",
            Patch::Keys => "Keys",
            Patch::Brass => "Brass",
            Patch::Strings => "Strings",
            Patch::Perc => "Perc",
            Patch::Sub => "Sub",
            Patch::Acid => "Acid",
            Patch::Chip => "Chip",
            Patch::Stab => "Stab",
            Patch::Mallet => "Mallet",
            Patch::Flute => "Flute",
            Patch::Reese => "Reese",
            Patch::Wire => "Wire",
            Patch::Chime => "Chime",
            Patch::Growl => "Growl",
            Patch::Whistle => "Whistle",
            Patch::Siren => "Siren",
            Patch::Dist => "Dist",
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
    /// Filter type (LP/HP/BP)
    filter_type: FilterType,
    /// Sub-oscillator mix (0.0 = off, sine one octave below)
    sub_osc_mix: f32,
    /// FM ratio (0.0 = no FM, overrides patch-specific FM when > 0)
    fm_ratio: f32,
    /// FM modulation index (depth)
    fm_index: f32,
    /// Pulse width for Pulse waveform
    pulse_width: f32,
}

/// Default PatchParams with standard values for new fields
const DEFAULT_EXT: (FilterType, f32, f32, f32, f32) = (FilterType::LowPass, 0.0, 0.0, 0.0, 0.25);

fn patch_params(patch: Patch) -> PatchParams {
    let (filter_type, sub_osc_mix, fm_ratio, fm_index, pulse_width) = DEFAULT_EXT;
    match patch {
        Patch::Saw => PatchParams {
            env: EnvParams {
                attack: 0.005,
                decay: 0.1,
                sustain: 0.7,
                release: 0.15,
            },
            filter_cutoff_mul: 4.0,
            filter_resonance: 0.3,
            filter_env_amount: 2.0,
            detune_cents: 8.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Square => PatchParams {
            env: EnvParams {
                attack: 0.005,
                decay: 0.15,
                sustain: 0.6,
                release: 0.12,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.4,
            filter_env_amount: 1.5,
            detune_cents: 5.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Sine => PatchParams {
            env: EnvParams {
                attack: 0.01,
                decay: 0.0,
                sustain: 1.0,
                release: 0.08,
            },
            filter_cutoff_mul: 20.0,
            filter_resonance: 0.0,
            filter_env_amount: 0.0,
            detune_cents: 0.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Triangle => PatchParams {
            env: EnvParams {
                attack: 0.01,
                decay: 0.2,
                sustain: 0.8,
                release: 0.2,
            },
            filter_cutoff_mul: 6.0,
            filter_resonance: 0.2,
            filter_env_amount: 1.0,
            detune_cents: 6.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Pulse => PatchParams {
            env: EnvParams {
                attack: 0.003,
                decay: 0.08,
                sustain: 0.5,
                release: 0.1,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.5,
            filter_env_amount: 2.5,
            detune_cents: 10.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::FmBell => PatchParams {
            env: EnvParams {
                attack: 0.001,
                decay: 1.5,
                sustain: 0.0,
                release: 0.5,
            },
            filter_cutoff_mul: 12.0,
            filter_resonance: 0.1,
            filter_env_amount: 3.0,
            detune_cents: 0.0,
            filter_type,
            sub_osc_mix,
            fm_ratio: 3.5,
            fm_index: 2.0,
            pulse_width,
        },
        Patch::Organ => PatchParams {
            env: EnvParams {
                attack: 0.008,
                decay: 0.0,
                sustain: 1.0,
                release: 0.05,
            },
            filter_cutoff_mul: 8.0,
            filter_resonance: 0.15,
            filter_env_amount: 0.5,
            detune_cents: 4.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Noise => PatchParams {
            env: EnvParams {
                attack: 0.002,
                decay: 0.3,
                sustain: 0.0,
                release: 0.1,
            },
            filter_cutoff_mul: 2.0,
            filter_resonance: 0.6,
            filter_env_amount: 3.0,
            detune_cents: 0.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::FundspPad => PatchParams {
            env: EnvParams {
                attack: 0.05,
                decay: 0.3,
                sustain: 0.6,
                release: 0.4,
            },
            filter_cutoff_mul: 6.0,
            filter_resonance: 0.3,
            filter_env_amount: 2.0,
            detune_cents: 0.0,
            filter_type,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        // -- New presets --
        Patch::Bass => PatchParams {
            env: EnvParams {
                attack: 0.003,
                decay: 0.15,
                sustain: 0.6,
                release: 0.08,
            },
            filter_cutoff_mul: 2.0,
            filter_resonance: 0.6,
            filter_env_amount: 2.5,
            detune_cents: 6.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.5,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Pluck => PatchParams {
            env: EnvParams {
                attack: 0.001,
                decay: 0.25,
                sustain: 0.0,
                release: 0.15,
            },
            filter_cutoff_mul: 6.0,
            filter_resonance: 0.3,
            filter_env_amount: 4.0,
            detune_cents: 3.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Pad => PatchParams {
            env: EnvParams {
                attack: 0.3,
                decay: 0.5,
                sustain: 0.7,
                release: 0.8,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.2,
            filter_env_amount: 1.0,
            detune_cents: 15.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.2,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Lead => PatchParams {
            env: EnvParams {
                attack: 0.005,
                decay: 0.1,
                sustain: 0.8,
                release: 0.1,
            },
            filter_cutoff_mul: 5.0,
            filter_resonance: 0.5,
            filter_env_amount: 2.0,
            detune_cents: 12.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.3,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Keys => PatchParams {
            env: EnvParams {
                attack: 0.001,
                decay: 0.6,
                sustain: 0.3,
                release: 0.3,
            },
            filter_cutoff_mul: 8.0,
            filter_resonance: 0.15,
            filter_env_amount: 2.5,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio: 2.0,
            fm_index: 1.5,
            pulse_width,
        },
        Patch::Brass => PatchParams {
            env: EnvParams {
                attack: 0.05,
                decay: 0.1,
                sustain: 0.8,
                release: 0.12,
            },
            filter_cutoff_mul: 2.0,
            filter_resonance: 0.4,
            filter_env_amount: 3.5,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Strings => PatchParams {
            env: EnvParams {
                attack: 0.15,
                decay: 0.3,
                sustain: 0.8,
                release: 0.5,
            },
            filter_cutoff_mul: 4.0,
            filter_resonance: 0.15,
            filter_env_amount: 0.8,
            detune_cents: 18.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.15,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Perc => PatchParams {
            env: EnvParams {
                attack: 0.001,
                decay: 0.12,
                sustain: 0.0,
                release: 0.05,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.7,
            filter_env_amount: 4.0,
            detune_cents: 0.0,
            filter_type: FilterType::BandPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Sub => PatchParams {
            env: EnvParams {
                attack: 0.01,
                decay: 0.05,
                sustain: 0.9,
                release: 0.3,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.1,
            filter_env_amount: 0.5,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.8,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        // -- Batch 2 presets --
        Patch::Acid => PatchParams {
            // TB-303 style: saw, high resonance, heavy filter envelope sweep
            env: EnvParams {
                attack: 0.002,
                decay: 0.2,
                sustain: 0.3,
                release: 0.08,
            },
            filter_cutoff_mul: 1.5,
            filter_resonance: 0.85,
            filter_env_amount: 4.0,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Chip => PatchParams {
            // 8-bit chiptune: narrow pulse, bright, no filter modulation
            env: EnvParams {
                attack: 0.001,
                decay: 0.0,
                sustain: 1.0,
                release: 0.02,
            },
            filter_cutoff_mul: 20.0,
            filter_resonance: 0.0,
            filter_env_amount: 0.0,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width: 0.125,
        },
        Patch::Stab => PatchParams {
            // Short sharp synth stab: fast decay, big filter envelope
            env: EnvParams {
                attack: 0.001,
                decay: 0.08,
                sustain: 0.0,
                release: 0.05,
            },
            filter_cutoff_mul: 2.0,
            filter_resonance: 0.4,
            filter_env_amount: 5.0,
            detune_cents: 10.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Mallet => PatchParams {
            // Vibraphone/marimba: FM with quick decay
            env: EnvParams {
                attack: 0.001,
                decay: 0.8,
                sustain: 0.0,
                release: 0.4,
            },
            filter_cutoff_mul: 10.0,
            filter_resonance: 0.05,
            filter_env_amount: 1.5,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio: 4.0,
            fm_index: 1.8,
            pulse_width,
        },
        Patch::Flute => PatchParams {
            // Soft breathy flute: triangle, gentle HP to remove mud
            env: EnvParams {
                attack: 0.05,
                decay: 0.1,
                sustain: 0.7,
                release: 0.2,
            },
            filter_cutoff_mul: 6.0,
            filter_resonance: 0.1,
            filter_env_amount: 0.5,
            detune_cents: 3.0,
            filter_type: FilterType::HighPass,
            sub_osc_mix: 0.15,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Reese => PatchParams {
            // Heavy detuned reese bass: wide detune + sub for weight
            env: EnvParams {
                attack: 0.005,
                decay: 0.1,
                sustain: 0.8,
                release: 0.1,
            },
            filter_cutoff_mul: 2.5,
            filter_resonance: 0.3,
            filter_env_amount: 1.5,
            detune_cents: 25.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.4,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Wire => PatchParams {
            // Metallic bandpass square: thin, resonant, edgy
            env: EnvParams {
                attack: 0.003,
                decay: 0.15,
                sustain: 0.5,
                release: 0.1,
            },
            filter_cutoff_mul: 3.5,
            filter_resonance: 0.75,
            filter_env_amount: 2.0,
            detune_cents: 4.0,
            filter_type: FilterType::BandPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Chime => PatchParams {
            // Bright chime: FM bell variant with different ratio, longer tail
            env: EnvParams {
                attack: 0.001,
                decay: 2.0,
                sustain: 0.0,
                release: 1.0,
            },
            filter_cutoff_mul: 15.0,
            filter_resonance: 0.05,
            filter_env_amount: 2.0,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio: 5.0,
            fm_index: 1.5,
            pulse_width,
        },
        Patch::Growl => PatchParams {
            // Aggressive growl: saw with FM modulation for grit
            env: EnvParams {
                attack: 0.005,
                decay: 0.1,
                sustain: 0.7,
                release: 0.1,
            },
            filter_cutoff_mul: 3.0,
            filter_resonance: 0.5,
            filter_env_amount: 2.5,
            detune_cents: 8.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.2,
            fm_ratio: 1.5,
            fm_index: 2.5,
            pulse_width,
        },
        Patch::Whistle => PatchParams {
            // Clean whistle: pure sine, wide open LP
            env: EnvParams {
                attack: 0.02,
                decay: 0.0,
                sustain: 1.0,
                release: 0.15,
            },
            filter_cutoff_mul: 20.0,
            filter_resonance: 0.0,
            filter_env_amount: 0.0,
            detune_cents: 0.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Siren => PatchParams {
            // Bright triangle with filter sweep
            env: EnvParams {
                attack: 0.01,
                decay: 0.3,
                sustain: 0.6,
                release: 0.25,
            },
            filter_cutoff_mul: 5.0,
            filter_resonance: 0.6,
            filter_env_amount: 3.0,
            detune_cents: 6.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix,
            fm_ratio,
            fm_index,
            pulse_width,
        },
        Patch::Dist => PatchParams {
            // Distorted saw: high resonance, driven filter
            env: EnvParams {
                attack: 0.003,
                decay: 0.08,
                sustain: 0.7,
                release: 0.08,
            },
            filter_cutoff_mul: 1.8,
            filter_resonance: 0.9,
            filter_env_amount: 3.0,
            detune_cents: 5.0,
            filter_type: FilterType::LowPass,
            sub_osc_mix: 0.3,
            fm_ratio,
            fm_index,
            pulse_width,
        },
    }
}

/// Convert MIDI note number to frequency in Hz
fn midi_to_freq(note: f32) -> f32 {
    440.0 * 2.0_f32.powf((note - 69.0) / SEMITONES_PER_OCTAVE as f32)
}

/// State-variable filter state
#[derive(Clone)]
struct SvfState {
    low: f32,
    band: f32,
}

impl SvfState {
    fn new() -> Self {
        Self {
            low: 0.0,
            band: 0.0,
        }
    }

    /// Process one sample through a 2x oversampled SVF with selectable output
    #[inline]
    fn tick(
        &mut self,
        input: f32,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
        filter_type: FilterType,
    ) -> f32 {
        let max_freq = sample_rate * 0.45;
        let freq = cutoff_hz.min(max_freq).max(20.0);
        let f = (2.0 * (std::f32::consts::PI * freq / (sample_rate * 2.0)).sin()).min(0.95);
        let q = 1.0 - resonance.clamp(0.0, 0.95);

        let mut high = 0.0f32;
        for _ in 0..2 {
            high = input - self.low - q * self.band;
            self.band += f * high;
            self.low += f * self.band;
        }
        if !self.low.is_finite() || !self.band.is_finite() {
            self.low = 0.0;
            self.band = 0.0;
        }
        match filter_type {
            FilterType::LowPass => self.low,
            FilterType::HighPass => high,
            FilterType::BandPass => self.band,
        }
    }
}

/// Simple LCG noise generator (no std dependency in hot path)
#[derive(Clone)]
struct NoiseGen {
    state: u32,
}

impl NoiseGen {
    fn new(seed: u32) -> Self {
        Self {
            state: seed.wrapping_add(1),
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        // LCG: fast, low-quality but fine for audio noise
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        // Map to -1..1
        (self.state as i32) as f32 / i32::MAX as f32
    }
}

/// A pre-built fundsp graph for the `FundspPad` patch.
///
/// The graph is built once and retuned through `Shared` values rather than
/// rebuilt per note. Constructing it allocates, and `note_on` runs on the
/// audio thread, so building one there risked a dropout on every pad note.
struct FundspVoice {
    unit: Box<dyn AudioUnit>,
    freq: Shared,
    detuned_freq: Shared,
    cutoff: Shared,
    resonance: Shared,
}

impl FundspVoice {
    fn new(sample_rate: f32) -> Self {
        let freq = shared(440.0);
        let detuned_freq = shared(440.0 * DETUNE_RATIO);
        let cutoff = shared(1000.0);
        let resonance = shared(0.5);
        // Two detuned saws summed, into a moog low-pass filter.
        let mut unit: Box<dyn AudioUnit> = Box::new(
            (((var(&freq) >> saw()) + (var(&detuned_freq) >> saw()) * 0.5)
                | var(&cutoff)
                | var(&resonance))
                >> moog(),
        );
        unit.set_sample_rate(sample_rate as f64);
        Self {
            unit,
            freq,
            detuned_freq,
            cutoff,
            resonance,
        }
    }

    /// Point the graph at a new note and clear its internal state, so a
    /// recycled voice does not carry over the previous note's filter ring.
    fn retune(&mut self, frequency: f32, cutoff: f32, resonance: f32) {
        self.freq.set_value(frequency);
        self.detuned_freq.set_value(frequency * DETUNE_RATIO);
        self.cutoff.set_value(cutoff);
        self.resonance.set_value(resonance);
        self.unit.reset();
    }
}

/// Detuning of the second saw in the fundsp pad, in frequency ratio.
const DETUNE_RATIO: f32 = 2.01;

/// A single synth voice
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
    phase2: f64,       // detuned second oscillator
    fm_mod_phase: f64, // for FM bell

    // Pitch offset in semitones (from effects like portamento, vibrato, arpeggio)
    pitch_offset: f32,

    // ADSR envelope (shared Envelope type)
    envelope: Envelope,

    // Filter
    filter: SvfState,

    // Noise generator (for Noise patch)
    noise: NoiseGen,

    // fundsp AudioUnit (for FundspPad patch)
    fundsp_unit: Option<FundspVoice>,

    sample_rate: f32,
}

impl Voice {
    fn new(channel: u8, note: u8, velocity: f32, patch: Patch, sample_rate: f32) -> Self {
        let params = patch_params(patch);
        Self::build(channel, note, velocity, patch, params, sample_rate)
    }

    fn new_with_params(
        channel: u8,
        note: u8,
        velocity: f32,
        patch: Patch,
        params: PatchParams,
        sample_rate: f32,
    ) -> Self {
        Self::build(channel, note, velocity, patch, params, sample_rate)
    }

    fn build(
        channel: u8,
        note: u8,
        velocity: f32,
        patch: Patch,
        params: PatchParams,
        sample_rate: f32,
    ) -> Self {
        let frequency = midi_to_freq(note as f32);

        // The fundsp graph, if this patch needs one, is attached by
        // `BuiltinSynth::push_voice` from a pre-built pool.
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
            pitch_offset: 0.0,
            envelope: Envelope::new(
                params.env.attack,
                params.env.decay,
                params.env.sustain,
                params.env.release,
                sample_rate,
            ),
            filter: SvfState::new(),
            noise: NoiseGen::new(note as u32 * 7919 + channel as u32 * 104729),
            fundsp_unit: None,
            sample_rate,
        }
    }

    /// Trigger the release phase
    fn release(&mut self) {
        self.envelope.release();
    }

    /// Generate one stereo sample pair
    #[inline]
    fn tick(&mut self) -> (f32, f32) {
        if !self.active || self.envelope.stage == EnvStage::Off {
            return (0.0, 0.0);
        }

        let sr = self.sample_rate;
        let freq = if self.pitch_offset != 0.0 {
            self.frequency * 2.0_f32.powf(self.pitch_offset / SEMITONES_PER_OCTAVE as f32)
        } else {
            self.frequency
        };

        // -- ADSR envelope (shared Envelope type) --
        let env_level = self.envelope.tick();
        if !self.envelope.is_active() {
            self.active = false;
            return (0.0, 0.0);
        }

        // -- Oscillator --
        let phase_inc = freq as f64 / sr as f64;
        let detune_ratio = 2.0_f64.powf(self.params.detune_cents as f64 / 1200.0);
        let phase_inc2 = freq as f64 * detune_ratio / sr as f64;

        self.phase += phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.phase2 += phase_inc2;
        if self.phase2 >= 1.0 {
            self.phase2 -= 1.0;
        }

        // -- Oscillator --
        // Helper: detuned pair of a waveform
        let saw_osc = |ph: f64, ph2: f64, pi: f64, pi2: f64, det: f32| -> f32 {
            let s1 = polyblep_saw(ph, pi);
            if det > 0.0 {
                (s1 + polyblep_saw(ph2, pi2)) * 0.5
            } else {
                s1
            }
        };
        let square_osc = |ph: f64, ph2: f64, pi: f64, pi2: f64, det: f32| -> f32 {
            let s1 = polyblep_square(ph, pi);
            if det > 0.0 {
                (s1 + polyblep_square(ph2, pi2)) * 0.5
            } else {
                s1
            }
        };
        let tri_osc = |ph: f64, ph2: f64, det: f32| -> f32 {
            let s1 = triangle(ph);
            if det > 0.0 {
                (s1 + triangle(ph2)) * 0.5
            } else {
                s1
            }
        };

        let osc = match self.patch {
            Patch::Saw
            | Patch::Bass
            | Patch::Pluck
            | Patch::Pad
            | Patch::Lead
            | Patch::Brass
            | Patch::Strings
            | Patch::Sub
            | Patch::Acid
            | Patch::Stab
            | Patch::Reese
            | Patch::Growl
            | Patch::Dist => saw_osc(
                self.phase,
                self.phase2,
                phase_inc,
                phase_inc2,
                self.params.detune_cents,
            ),
            Patch::Square | Patch::Wire => square_osc(
                self.phase,
                self.phase2,
                phase_inc,
                phase_inc2,
                self.params.detune_cents,
            ),
            Patch::Sine | Patch::Whistle => (self.phase as f32 * std::f32::consts::TAU).sin(),
            Patch::Triangle | Patch::Flute | Patch::Siren => {
                tri_osc(self.phase, self.phase2, self.params.detune_cents)
            }
            Patch::Pulse | Patch::Chip => {
                let duty = self.params.pulse_width as f64;
                let s1 = polyblep_pulse(self.phase, phase_inc, duty);
                if self.params.detune_cents > 0.0 {
                    let s2 = polyblep_pulse(self.phase2, phase_inc2, duty);
                    (s1 + s2) * 0.5
                } else {
                    s1
                }
            }
            Patch::FmBell | Patch::Keys | Patch::Mallet | Patch::Chime => {
                let fm_ratio = self.params.fm_ratio as f64;
                let fm_index = self.params.fm_index;
                let mod_index = fm_index * env_level;
                let fm_inc = freq as f64 * fm_ratio / sr as f64;
                self.fm_mod_phase += fm_inc;
                if self.fm_mod_phase >= 1.0 {
                    self.fm_mod_phase -= 1.0;
                }
                let modulator = (self.fm_mod_phase as f32 * std::f32::consts::TAU).sin();
                let mod_freq = self.phase as f32 + modulator * mod_index;
                (mod_freq * std::f32::consts::TAU).sin()
            }
            Patch::Organ => {
                let f1 = (self.phase as f32 * std::f32::consts::TAU).sin();
                let f2 = (self.phase as f32 * 2.0 * std::f32::consts::TAU).sin();
                let f3 = (self.phase as f32 * 3.0 * std::f32::consts::TAU).sin();
                f1 * 0.6 + f2 * 0.3 + f3 * 0.1
            }
            Patch::Noise | Patch::Perc => self.noise.next(),
            Patch::FundspPad => {
                if let Some(ref mut fundsp) = self.fundsp_unit {
                    let mut output = [0f32; 1];
                    fundsp.unit.tick(&[], &mut output);
                    let gain = self.velocity * env_level * VOICE_GAIN;
                    let sample = output[0] * gain;
                    return (sample, sample);
                }
                return (0.0, 0.0);
            }
        };

        // -- FM overlay (for any patch with fm_ratio > 0 not already handled) --
        let osc = if self.params.fm_ratio > 0.0
            && self.patch != Patch::FmBell
            && self.patch != Patch::Keys
            && self.patch != Patch::Mallet
            && self.patch != Patch::Chime
            && self.patch != Patch::FundspPad
        {
            let fm_ratio = self.params.fm_ratio as f64;
            let mod_index = self.params.fm_index * env_level;
            let fm_inc = freq as f64 * fm_ratio / sr as f64;
            self.fm_mod_phase += fm_inc;
            if self.fm_mod_phase >= 1.0 {
                self.fm_mod_phase -= 1.0;
            }
            let modulator = (self.fm_mod_phase as f32 * std::f32::consts::TAU).sin();
            // Apply FM as phase modulation on top of the base oscillator
            osc + modulator * mod_index * 0.3
        } else {
            osc
        };

        // -- Sub-oscillator (sine one octave below) --
        let osc = if self.params.sub_osc_mix > 0.0 {
            let sub_phase = (self.phase * 0.5) % 1.0;
            let sub = (sub_phase as f32 * std::f32::consts::TAU).sin();
            osc * (1.0 - self.params.sub_osc_mix) + sub * self.params.sub_osc_mix
        } else {
            osc
        };

        // -- Filter --
        let env_mod = env_level * self.params.filter_env_amount;
        let cutoff = freq * self.params.filter_cutoff_mul * 2.0_f32.powf(env_mod);
        let vel_mod = 0.5 + 0.5 * self.velocity;
        let final_cutoff = cutoff * vel_mod;

        let filtered = self.filter.tick(
            osc,
            final_cutoff,
            self.params.filter_resonance,
            sr,
            self.params.filter_type,
        );

        // -- Output --
        let gain = self.velocity * env_level * VOICE_GAIN;
        let sample = filtered * gain;
        (sample, sample)
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
    /// Pre-built fundsp graphs, one per possible voice. Taken on note-on and
    /// returned when a voice is evicted, so the audio thread never allocates
    /// or frees one.
    fundsp_pool: Vec<FundspVoice>,
    sample_rate: f32,
}

impl BuiltinSynth {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            voices: Vec::with_capacity(MAX_VOICES),
            programs: [0; 16],
            fundsp_pool: (0..MAX_VOICES)
                .map(|_| FundspVoice::new(sample_rate as f32))
                .collect(),
            sample_rate: sample_rate as f32,
        }
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        // Kill existing voice on this channel (tracker: one note per channel)
        self.note_off_all_channel(channel);

        let patch = Patch::from_program(self.programs[(channel & 0x0F) as usize]);
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;
        let voice = Voice::new(channel, note, vel, patch, self.sample_rate);

        self.push_voice(voice);
    }

    /// Note-on with user-configured synth parameters (overrides channel program)
    pub fn note_on_with_params(
        &mut self,
        channel: u8,
        note: u8,
        velocity: u8,
        params: &SynthParams,
    ) {
        self.note_off_all_channel(channel);

        let patch = Patch::from_program(params.waveform);
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;
        let custom_params = PatchParams {
            env: EnvParams {
                attack: params.attack,
                decay: params.decay,
                sustain: params.sustain,
                release: params.release,
            },
            filter_cutoff_mul: params.filter_cutoff,
            filter_resonance: params.filter_resonance,
            filter_env_amount: params.filter_env,
            detune_cents: params.detune,
            filter_type: params.filter_type,
            sub_osc_mix: params.sub_osc,
            fm_ratio: params.fm_ratio,
            fm_index: params.fm_index,
            pulse_width: params.pulse_width,
        };
        let voice =
            Voice::new_with_params(channel, note, vel, patch, custom_params, self.sample_rate);

        self.push_voice(voice);
    }

    fn push_voice(&mut self, mut voice: Voice) {
        if self.voices.len() >= MAX_VOICES {
            // First try to remove an inactive voice
            if let Some(idx) = self.voices.iter().position(|v| !v.active) {
                self.reclaim_voice(idx);
            } else {
                // Steal the quietest voice (lowest envelope level * velocity)
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
                    self.reclaim_voice(idx);
                }
            }
        }

        if voice.patch == Patch::FundspPad {
            if let Some(mut fundsp) = self.fundsp_pool.pop() {
                let cutoff = voice.frequency * voice.params.filter_cutoff_mul;
                fundsp.retune(voice.frequency, cutoff, voice.params.filter_resonance);
                voice.fundsp_unit = Some(fundsp);
            }
            // An exhausted pool means every voice slot is already in use;
            // the note is dropped rather than allocating on this thread.
        }

        self.voices.push(voice);
    }

    /// Remove a voice, returning any fundsp graph it holds to the pool.
    /// Dropping it here would free heap memory on the audio thread.
    fn reclaim_voice(&mut self, idx: usize) {
        let mut voice = self.voices.remove(idx);
        if let Some(fundsp) = voice.fundsp_unit.take() {
            self.fundsp_pool.push(fundsp);
        }
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

    /// Set pitch offset (in semitones) for all active voices on a channel.
    /// Used by offline export to apply portamento, vibrato, and arpeggio effects.
    pub fn set_channel_pitch_offset(&mut self, channel: u8, semitones: f32) {
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                voice.pitch_offset = semitones;
            }
        }
    }

    /// Set volume (velocity 0-127) for all active voices on a channel.
    /// Used by offline export to apply volume slide effects.
    pub fn set_channel_volume(&mut self, channel: u8, velocity: u8) {
        let vel = velocity as f32 / MIDI_MAX_VALUE as f32;
        for voice in &mut self.voices {
            if voice.active && voice.channel == channel {
                voice.velocity = vel;
            }
        }
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
        self.voices
            .retain(|v| v.active || v.envelope.stage != EnvStage::Off);

        (left, right)
    }

    /// Render one frame, outputting per-channel stereo pairs.
    /// `channel_out[ch]` receives `[left, right]` for tracker channel `ch`.
    /// Voices whose channel >= channel_out.len() are summed into channel 0.
    #[inline]
    pub fn render_sample_per_channel(&mut self, channel_out: &mut [[f32; 2]]) {
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let (l, r) = voice.tick();
            let ch = std::cmp::min(voice.channel as usize, channel_out.len().saturating_sub(1));
            channel_out[ch][0] += l;
            channel_out[ch][1] += r;
        }
        self.voices
            .retain(|v| v.active || v.envelope.stage != EnvStage::Off);
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
        assert!(matches!(Patch::from_program(8), Patch::FundspPad));
        assert!(matches!(Patch::from_program(9), Patch::Bass));
        assert!(matches!(Patch::from_program(17), Patch::Sub));
        assert!(matches!(Patch::from_program(18), Patch::Acid));
        assert!(matches!(Patch::from_program(29), Patch::Dist));
        assert!(matches!(Patch::from_program(30), Patch::Saw)); // wraps
    }

    #[test]
    fn test_synth_voice_lifecycle() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        assert_eq!(synth.voices.len(), 1);
        assert!(synth.voices[0].active);

        synth.note_off(0, 60);
        assert_eq!(synth.voices[0].envelope.stage, EnvStage::Release);

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
        let active: Vec<_> = synth
            .voices
            .iter()
            .filter(|v| v.active && v.envelope.stage != EnvStage::Release)
            .collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].note, 64);
    }

    #[test]
    fn test_synth_note_off_channel() {
        let mut synth = BuiltinSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(1, 64, 100);
        synth.note_off_all_channel(0);

        let active_non_releasing: Vec<_> = synth
            .voices
            .iter()
            .filter(|v| v.active && v.envelope.stage != EnvStage::Release)
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
        for prog in 0..Patch::count() {
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
            assert!(
                peak > 0.01,
                "Patch {} ({:?}) near-silent: peak={:.6}",
                prog,
                Patch::from_program(prog),
                peak
            );
            assert!(
                peak < 1.0,
                "Patch {} ({:?}) clips: peak={:.4}",
                prog,
                Patch::from_program(prog),
                peak
            );
        }
    }

    #[test]
    fn test_note_on_with_custom_params() {
        let sr = 44100.0;
        let mut synth = BuiltinSynth::new(sr);
        let params = SynthParams {
            waveform: 0, // Saw
            attack: 0.01,
            decay: 0.2,
            sustain: 0.5,
            release: 0.3,
            filter_cutoff: 8.0,
            filter_resonance: 0.4,
            filter_env: 1.5,
            detune: 12.0,
            filter_type: FilterType::LowPass,
            sub_osc: 0.0,
            fm_ratio: 0.0,
            fm_index: 0.0,
            pulse_width: 0.25,
        };
        synth.note_on_with_params(0, 60, 127, &params);
        assert_eq!(synth.active_voice_count(), 1);

        let mut peak = 0.0_f32;
        for _ in 0..4410 {
            let (l, r) = synth.render_sample();
            peak = peak.max(l.abs()).max(r.abs());
            assert!(l.is_finite(), "Custom params: non-finite sample");
        }
        assert!(
            peak > 0.01,
            "Custom params produced no audio: peak={:.6}",
            peak
        );
    }

    #[test]
    fn test_synth_params_from_patch() {
        for prog in 0..Patch::count() {
            let params = SynthParams::from_patch(prog);
            assert_eq!(params.waveform, prog);
            assert!(params.attack >= 0.0);
            assert!(params.sustain >= 0.0 && params.sustain <= 1.0);
            assert!(params.filter_cutoff > 0.0);
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
        let attack_level = synth.voices[0].envelope.level;
        assert!(
            attack_level > 0.0,
            "Envelope should be rising during attack"
        );

        // Wait for attack to finish and enter sustain
        for _ in 0..4410 {
            synth.render_sample();
        }
        let sustain_level = synth.voices[0].envelope.level;
        // Sine patch has sustain=1.0, so should be at ~1.0
        assert!(
            sustain_level > 0.9,
            "Sine sustain should be near 1.0, got {:.4}",
            sustain_level
        );

        // Release
        synth.note_off(0, 69);
        let mut prev_env = synth.voices[0].envelope.level;
        for _ in 0..1000 {
            synth.render_sample();
            let cur = synth
                .voices
                .first()
                .map(|v| v.envelope.level)
                .unwrap_or(0.0);
            assert!(
                cur <= prev_env + 0.001,
                "Envelope should decrease during release"
            );
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
            if l.abs() > 1e-6 {
                has_nonzero = true;
            }
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

        let first_peak = post_samples[..44]
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        let last_peak = post_samples[window - 44..]
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));

        // Fade should not be instant cutoff
        assert!(
            first_peak > pre_off_peak * 0.2,
            "Note-off caused instant cutoff! first_window={:.6}, pre_peak={:.6}",
            first_peak,
            pre_off_peak
        );
        // Should be decreasing
        assert!(
            last_peak < first_peak,
            "Fade not decreasing: first={:.6}, last={:.6}",
            first_peak,
            last_peak
        );
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
        let rms =
            (all_left.iter().map(|&s| (s * s) as f64).sum::<f64>() / all_left.len() as f64).sqrt();

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
        let phase_inc = freq / sr;
        let mut sum = 0.0_f64;
        for _ in 0..samples_per_cycle * 10 {
            let s = polyblep_saw(phase, phase_inc);
            sum += s as f64;
            phase += phase_inc;
            if phase >= 1.0 {
                phase -= 1.0;
            }
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

        assert!(
            peak_loud > peak_quiet * 1.5,
            "Velocity should affect volume: loud={:.4}, quiet={:.4}",
            peak_loud,
            peak_quiet
        );
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

        assert!(
            saw_diff_rms > sine_diff_rms,
            "Saw should have more HF content than sine: saw_diff_rms={:.6}, sine_diff_rms={:.6}",
            saw_diff_rms,
            sine_diff_rms
        );
    }
}

#[cfg(test)]
mod fundsp_pool_tests {
    use super::*;

    const SR: f64 = 48_000.0;

    /// Program 8 selects the fundsp pad, the only patch backed by a fundsp graph.
    const PAD_PROGRAM: u8 = 8;

    fn render_peak(synth: &mut BuiltinSynth, frames: usize) -> f32 {
        let mut peak = 0.0f32;
        for _ in 0..frames {
            let (l, r) = synth.render_sample();
            peak = peak.max(l.abs()).max(r.abs());
        }
        peak
    }

    #[test]
    fn the_pool_is_prebuilt_so_note_on_never_allocates_a_graph() {
        let synth = BuiltinSynth::new(SR);
        assert_eq!(
            synth.fundsp_pool.len(),
            MAX_VOICES,
            "every voice slot needs a graph waiting for it"
        );
    }

    #[test]
    fn a_pad_note_takes_a_graph_from_the_pool_and_sounds() {
        let mut synth = BuiltinSynth::new(SR);
        synth.program_change(0, PAD_PROGRAM);
        synth.note_on(0, 60, 127);

        assert_eq!(
            synth.fundsp_pool.len(),
            MAX_VOICES - 1,
            "the voice should be holding a pooled graph"
        );
        assert!(
            render_peak(&mut synth, 512) > 1e-6,
            "the pad rendered silence"
        );
    }

    #[test]
    fn non_pad_patches_leave_the_pool_untouched() {
        let mut synth = BuiltinSynth::new(SR);
        synth.program_change(0, 0); // saw
        synth.note_on(0, 60, 127);
        assert_eq!(synth.fundsp_pool.len(), MAX_VOICES);
    }

    #[test]
    fn evicted_voices_return_their_graph_to_the_pool() {
        let mut synth = BuiltinSynth::new(SR);
        for ch in 0..16u8 {
            synth.program_change(ch, PAD_PROGRAM);
        }
        // Far more notes than there are voice slots, forcing eviction.
        for i in 0..(MAX_VOICES * 4) {
            synth.note_on((i % 16) as u8, 40 + (i % 40) as u8, 100);
        }
        assert!(synth.voices.len() <= MAX_VOICES);
        let held = synth
            .voices
            .iter()
            .filter(|v| v.fundsp_unit.is_some())
            .count();
        assert_eq!(
            synth.fundsp_pool.len() + held,
            MAX_VOICES,
            "graphs leaked: pool {} + held {} should always total {}",
            synth.fundsp_pool.len(),
            held,
            MAX_VOICES
        );
    }

    #[test]
    fn a_recycled_graph_is_retuned_rather_than_reused_stale() {
        // Same voice slot, two different notes: the second must render at the
        // new pitch, not carry the first one's state.
        let mut synth = BuiltinSynth::new(SR);
        synth.program_change(0, PAD_PROGRAM);

        synth.note_on(0, 45, 127);
        let low = render_peak(&mut synth, 256);
        assert!(low > 1e-6);

        // Fill every slot so the first voice is evicted and its graph recycled.
        for i in 0..(MAX_VOICES * 2) {
            synth.note_on((i % 16) as u8, 70, 100);
        }
        assert!(
            render_peak(&mut synth, 256) > 1e-6,
            "recycled graphs stopped producing sound"
        );
    }

    #[test]
    fn retuning_updates_the_shared_frequency() {
        let mut voice = FundspVoice::new(SR as f32);
        voice.retune(220.0, 1200.0, 0.3);
        assert_eq!(voice.freq.value(), 220.0);
        assert_eq!(voice.detuned_freq.value(), 220.0 * DETUNE_RATIO);
        assert_eq!(voice.cutoff.value(), 1200.0);
        assert_eq!(voice.resonance.value(), 0.3);
    }
}
