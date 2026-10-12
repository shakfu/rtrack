//! Named effect commands: the G8 cell format (`docs/dev/g4-g8-format.md`).
//!
//! A command is three characters and a two-digit hex parameter, written as
//! one string in `.rtrk`: `"PVB44"`. MIDI CC carries its controller in the
//! command, `C07`, so the instrument column no longer has to hold it.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::constants::*;

/// An effect command, without its parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxCmd {
    Arp,
    PitchUp,
    PitchDown,
    PitchSlide,
    Vibrato,
    VolSlide,
    Delay,
    Chance,
    Random,
    Offset,
    Jump,
    Break,
    Program,
    Ticks,
    Tempo,
    /// Play table `xx` for the row's note, or restart it on the sounding one.
    Table,
    /// MIDI CC to the controller it holds.
    Cc(u8),
}

/// Every command with a fixed name, paired with it. `Cc` is spelled `Cnn`.
///
/// No name may match `C[0-9A-F]{2}`, which `Cc` uses.
const NAMES: [(FxCmd, &str); 16] = [
    (FxCmd::Arp, "ARP"),
    (FxCmd::PitchUp, "PUP"),
    (FxCmd::PitchDown, "PDN"),
    (FxCmd::PitchSlide, "PSL"),
    (FxCmd::Vibrato, "PVB"),
    (FxCmd::VolSlide, "VSL"),
    (FxCmd::Delay, "DEL"),
    (FxCmd::Chance, "CHA"),
    (FxCmd::Random, "RND"),
    (FxCmd::Offset, "OFS"),
    (FxCmd::Jump, "JMP"),
    (FxCmd::Break, "BRK"),
    (FxCmd::Program, "PRG"),
    (FxCmd::Ticks, "TIC"),
    (FxCmd::Tempo, "TPO"),
    (FxCmd::Table, "TBL"),
];

impl FxCmd {
    /// The three-character name shown in the effect column.
    pub fn name(self) -> String {
        match self {
            Self::Cc(controller) => format!("C{controller:02X}"),
            cmd => NAMES
                .iter()
                .find(|(c, _)| *c == cmd)
                .map(|(_, n)| (*n).to_string())
                .unwrap_or_default(),
        }
    }

    /// Parse a three-character name. Case-insensitive.
    pub fn from_name(name: &str) -> Option<Self> {
        let upper = name.to_ascii_uppercase();
        if let Some((cmd, _)) = NAMES.iter().find(|(_, n)| *n == upper) {
            return Some(*cmd);
        }
        let hex = upper.strip_prefix('C')?;
        if hex.len() != 2 {
            return None;
        }
        u8::from_str_radix(hex, 16).ok().map(Self::Cc)
    }
}

/// A command and its parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fx {
    pub cmd: FxCmd,
    pub param: u8,
}

impl FxCmd {
    /// The command an old single-digit effect selected (versions 0 and 1).
    ///
    /// `param` splits `F` into speed and tempo; `controller` is the instrument
    /// column, which held the controller for `C`. Compared unmasked, as the
    /// engine compared it: `0x14` was a no-op, not `4`. `A` was never
    /// assigned.
    pub fn from_digit(digit: u8, param: u8, controller: Option<u8>) -> Option<Self> {
        Some(match digit {
            EFFECT_ARPEGGIO => Self::Arp,
            EFFECT_PORTA_UP => Self::PitchUp,
            EFFECT_PORTA_DOWN => Self::PitchDown,
            EFFECT_TONE_PORTA => Self::PitchSlide,
            EFFECT_VIBRATO => Self::Vibrato,
            EFFECT_VOLUME_SLIDE => Self::VolSlide,
            EFFECT_NOTE_DELAY => Self::Delay,
            EFFECT_PROBABILITY => Self::Chance,
            EFFECT_RANDOMIZE => Self::Random,
            EFFECT_SAMPLE_OFFSET => Self::Offset,
            EFFECT_POSITION_JUMP => Self::Jump,
            EFFECT_MIDI_CC => Self::Cc(controller.unwrap_or(0)),
            EFFECT_PATTERN_BREAK => Self::Break,
            EFFECT_PROGRAM_CHANGE => Self::Program,
            EFFECT_SET_SPEED if param < 0x20 => Self::Ticks,
            EFFECT_SET_SPEED => Self::Tempo,
            _ => return None,
        })
    }
}

impl Fx {
    pub const fn new(cmd: FxCmd, param: u8) -> Self {
        Self { cmd, param }
    }

    /// Convert a cell from the single-digit format (versions 0 and 1).
    ///
    /// `instrument` is the cell's instrument column. Returns the effect, if
    /// any, and whether `Cxx` consumed the instrument column as its
    /// controller. A missing parameter reads as 0, as the engine read it.
    pub fn from_legacy(
        effect: Option<u8>,
        value: Option<u8>,
        instrument: Option<u8>,
    ) -> (Option<Self>, bool) {
        let param = value.unwrap_or(0);
        let Some(cmd) = effect.and_then(|e| FxCmd::from_digit(e, param, instrument)) else {
            return (None, false);
        };
        let consumed = matches!(cmd, FxCmd::Cc(_)) && instrument.is_some();
        (Some(Self::new(cmd, param)), consumed)
    }
}

impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:02X}", self.cmd.name(), self.param)
    }
}

impl std::str::FromStr for Fx {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 5 || !s.is_ascii() {
            return Err(format!(
                "effect {s:?} is not three characters and two hex digits"
            ));
        }
        let cmd = FxCmd::from_name(&s[..3]).ok_or_else(|| format!("unknown effect {s:?}"))?;
        let param = u8::from_str_radix(&s[3..], 16)
            .map_err(|_| format!("effect {s:?} has a non-hex parameter"))?;
        Ok(Self { cmd, param })
    }
}

impl Serialize for Fx {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_round_trips_and_none_collides_with_cc() {
        for (cmd, name) in NAMES {
            assert_eq!(cmd.name(), name);
            assert_eq!(FxCmd::from_name(name), Some(cmd));
            let cc_shaped =
                name.starts_with('C') && name[1..].chars().all(|c| c.is_ascii_hexdigit());
            assert!(!cc_shaped, "{name} reads as a CC");
        }
        for controller in 0..=u8::MAX {
            let cmd = FxCmd::Cc(controller);
            assert_eq!(FxCmd::from_name(&cmd.name()), Some(cmd));
        }
    }

    #[test]
    fn text_form_round_trips_through_serde() {
        let fx = Fx {
            cmd: FxCmd::Cc(7),
            param: 0x40,
        };
        let json = serde_json::to_string(&fx).unwrap();
        assert_eq!(json, r#""C0740""#);
        assert_eq!(serde_json::from_str::<Fx>(&json).unwrap(), fx);
        assert_eq!("pvb44".parse::<Fx>().unwrap().cmd, FxCmd::Vibrato);
    }

    #[test]
    fn malformed_text_is_an_error_not_a_panic() {
        for bad in [
            "",
            "PVB4",
            "PVB444",
            "XYZ44",
            "PVBZZ",
            "C0G40",
            "PVB4\u{e9}",
        ] {
            assert!(bad.parse::<Fx>().is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn legacy_digits_map_to_names() {
        let cases = [
            (0x0, "ARP"),
            (0x1, "PUP"),
            (0x2, "PDN"),
            (0x3, "PSL"),
            (0x4, "PVB"),
            (0x5, "VSL"),
            (0x6, "DEL"),
            (0x7, "CHA"),
            (0x8, "RND"),
            (0x9, "OFS"),
            (0xB, "JMP"),
            (0xD, "BRK"),
            (0xE, "PRG"),
        ];
        for (digit, name) in cases {
            let (fx, consumed) = Fx::from_legacy(Some(digit), Some(0x12), None);
            assert_eq!(fx.unwrap().to_string(), format!("{name}12"));
            assert!(!consumed);
        }
    }

    #[test]
    fn legacy_f_splits_by_value() {
        let f = |v| Fx::from_legacy(Some(0xF), Some(v), None).0;
        // `F00` did nothing, and `TIC00` does nothing.
        assert_eq!(f(0x00).unwrap().cmd, FxCmd::Ticks);
        assert_eq!(f(0x01).unwrap().cmd, FxCmd::Ticks);
        assert_eq!(f(0x1F).unwrap().cmd, FxCmd::Ticks);
        assert_eq!(f(0x20).unwrap().cmd, FxCmd::Tempo);
        assert_eq!(f(0xFF).unwrap().cmd, FxCmd::Tempo);
    }

    #[test]
    fn legacy_cc_takes_its_controller_from_the_instrument_column() {
        let (fx, consumed) = Fx::from_legacy(Some(0xC), Some(0x40), Some(7));
        assert_eq!(fx.unwrap().to_string(), "C0740");
        assert!(consumed);

        // The engine read a missing controller as 0.
        let (fx, consumed) = Fx::from_legacy(Some(0xC), Some(0x40), None);
        assert_eq!(fx.unwrap().cmd, FxCmd::Cc(0));
        assert!(!consumed);
    }

    #[test]
    fn legacy_edge_cases() {
        assert_eq!(Fx::from_legacy(None, Some(5), Some(1)), (None, false));
        assert_eq!(Fx::from_legacy(Some(0xA), Some(5), None), (None, false));
        assert_eq!(Fx::from_legacy(Some(0x14), Some(5), None), (None, false));
        // A missing parameter reads as 0, as in the engine.
        assert_eq!(Fx::from_legacy(Some(0x4), None, None).0.unwrap().param, 0);
    }
}
