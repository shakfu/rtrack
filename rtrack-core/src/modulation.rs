//! Per-instrument modulation slots (M8 model, `docs/dev/design-gaps.md` G6).
//!
//! Each instrument has [`MOD_SLOTS`] slots. A slot runs a source from each
//! note's start, in engine ticks, so modulation follows the tempo, and adds
//! `source * amount` to one destination. Slots do not modulate each other.

use serde::{Deserialize, Serialize};

/// Modulation slots per instrument.
pub const MOD_SLOTS: usize = 2;

/// LFO waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    Square,
    /// Falls from +1 to -1 over each period.
    SawDown,
}

impl LfoShape {
    pub const ALL: [LfoShape; 4] = [
        LfoShape::Sine,
        LfoShape::Triangle,
        LfoShape::Square,
        LfoShape::SawDown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LfoShape::Sine => "sine",
            LfoShape::Triangle => "triangle",
            LfoShape::Square => "square",
            LfoShape::SawDown => "saw",
        }
    }
}

/// What a slot's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModSource {
    /// Rises 0 to 1 over `attack` ticks, holds for `hold`, falls to 0 over
    /// `decay`, then stays at 0.
    Envelope { attack: u8, hold: u8, decay: u8 },
    /// Swings between -1 and 1 once every `period` ticks.
    Lfo { shape: LfoShape, period: u8 },
}

impl ModSource {
    /// The source's value `tick` engine ticks after the note started.
    pub fn value(&self, tick: u32) -> f64 {
        match *self {
            ModSource::Envelope {
                attack,
                hold,
                decay,
            } => {
                let (a, h, d) = (attack as u32, hold as u32, decay as u32);
                if tick < a {
                    tick as f64 / a as f64
                } else if tick < a + h {
                    1.0
                } else if tick < a + h + d {
                    1.0 - (tick - a - h) as f64 / d as f64
                } else {
                    0.0
                }
            }
            ModSource::Lfo { shape, period } => {
                let period = period.max(1) as u32;
                let phase = (tick % period) as f64 / period as f64;
                match shape {
                    LfoShape::Sine => (phase * std::f64::consts::TAU).sin(),
                    // 0 at the start, 1 at a quarter, -1 at three quarters.
                    LfoShape::Triangle => {
                        if phase < 0.25 {
                            4.0 * phase
                        } else if phase < 0.75 {
                            2.0 - 4.0 * phase
                        } else {
                            4.0 * phase - 4.0
                        }
                    }
                    LfoShape::Square => {
                        if phase < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    LfoShape::SawDown => 1.0 - 2.0 * phase,
                }
            }
        }
    }
}

/// What a slot changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModDest {
    /// Channel volume, `amount` volume steps (0-127 scale) at full.
    #[default]
    Volume,
    /// Pitch, `amount / 8` semitones at full.
    Pitch,
    /// The instrument filter's cutoff, `amount / 16` octaves at full.
    Cutoff,
    /// The instrument's drive, `amount / 16` at full.
    Drive,
}

impl ModDest {
    pub const ALL: [ModDest; 4] = [
        ModDest::Volume,
        ModDest::Pitch,
        ModDest::Cutoff,
        ModDest::Drive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ModDest::Volume => "volume",
            ModDest::Pitch => "pitch",
            ModDest::Cutoff => "cutoff",
            ModDest::Drive => "drive",
        }
    }
}

/// One modulation slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModSlot {
    pub source: ModSource,
    pub dest: ModDest,
    /// Signed depth; see [`ModDest`] for the units.
    pub amount: i8,
}

impl Default for ModSlot {
    fn default() -> Self {
        Self {
            source: ModSource::Lfo {
                shape: LfoShape::Sine,
                period: 24,
            },
            dest: ModDest::Pitch,
            amount: 0,
        }
    }
}

/// The summed effect of an instrument's slots at one tick, in each
/// destination's units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModOffsets {
    /// Volume steps.
    pub volume: f64,
    /// Semitones.
    pub pitch: f64,
    /// Octaves.
    pub cutoff: f64,
    /// Drive units.
    pub drive: f64,
}

impl ModOffsets {
    /// Sum `slots` at `tick` ticks after the note started.
    pub fn at(slots: &[Option<ModSlot>; MOD_SLOTS], tick: u32) -> Self {
        let mut out = ModOffsets::default();
        for slot in slots.iter().flatten() {
            let v = slot.source.value(tick) * slot.amount as f64;
            match slot.dest {
                ModDest::Volume => out.volume += v,
                ModDest::Pitch => out.pitch += v / 8.0,
                ModDest::Cutoff => out.cutoff += v / 16.0,
                ModDest::Drive => out.drive += v / 16.0,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_envelope_rises_holds_and_falls() {
        let env = ModSource::Envelope {
            attack: 2,
            hold: 1,
            decay: 2,
        };
        let values: Vec<f64> = (0..7).map(|t| env.value(t)).collect();
        assert_eq!(values, vec![0.0, 0.5, 1.0, 1.0, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn a_zero_attack_starts_at_full() {
        let env = ModSource::Envelope {
            attack: 0,
            hold: 0,
            decay: 4,
        };
        assert_eq!(env.value(0), 1.0);
        assert_eq!(env.value(2), 0.5);
    }

    #[test]
    fn lfo_shapes_cover_minus_one_to_one_over_a_period() {
        let lfo = |shape| ModSource::Lfo { shape, period: 4 };
        let sample = |shape| -> Vec<f64> {
            (0..4)
                .map(|t| (lfo(shape).value(t) * 1e9).round() / 1e9)
                .collect()
        };
        assert_eq!(sample(LfoShape::Sine), vec![0.0, 1.0, 0.0, -1.0]);
        assert_eq!(sample(LfoShape::Triangle), vec![0.0, 1.0, 0.0, -1.0]);
        assert_eq!(sample(LfoShape::Square), vec![1.0, 1.0, -1.0, -1.0]);
        assert_eq!(sample(LfoShape::SawDown), vec![1.0, 0.5, 0.0, -0.5]);
        assert_eq!(lfo(LfoShape::Sine).value(5), lfo(LfoShape::Sine).value(1));
    }

    #[test]
    fn slots_sum_into_their_destinations_units() {
        let full = ModSource::Envelope {
            attack: 0,
            hold: 10,
            decay: 0,
        };
        let slots = [
            Some(ModSlot {
                source: full,
                dest: ModDest::Pitch,
                amount: 24,
            }),
            Some(ModSlot {
                source: full,
                dest: ModDest::Cutoff,
                amount: -32,
            }),
        ];
        let at = ModOffsets::at(&slots, 3);
        assert_eq!(at.pitch, 3.0);
        assert_eq!(at.cutoff, -2.0);
        assert_eq!(at.volume, 0.0);
    }
}
