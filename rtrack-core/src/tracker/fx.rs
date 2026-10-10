//! Named effect commands.
//!
//! A cell holds [`FX_COLUMNS`] effect slots. Each holds a three-letter command
//! and a one-byte parameter, written `ARP37`. Format version 2 replaced the
//! single hex digit of version 1, which had one free value left
//! (`docs/dev/design-gaps.md` G8).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Effect slots per cell.
pub const FX_COLUMNS: usize = 2;

/// An effect command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FxCmd {
    /// `ARPxy`: cycle note, note+x, note+y each tick.
    Arp,
    /// `SLUxx`: slide pitch up by xx/16 semitone per tick.
    SlideUp,
    /// `SLDxx`: slide pitch down by xx/16 semitone per tick.
    SlideDown,
    /// `PORxx`: slide toward the row's note at xx/16 semitone per tick.
    Portamento,
    /// `VIBxy`: vibrato, speed x, depth y.
    Vibrato,
    /// `VSLxy`: volume slide, up x and down y per tick.
    VolumeSlide,
    /// `DLYxx`: delay the row's note or note-off by xx ticks.
    Delay,
    /// `CHAxx`: play the row's note with probability xx/255.
    Chance,
    /// `RNDxx`: re-run the channel's last continuous effect, parameter
    /// varied by up to xx either way.
    Random,
    /// `OFSxx`: start the row's sample note xx/256 into its played span.
    Offset,
    /// `JMPxx`: jump to order position xx.
    Jump,
    /// `CCNxx`: set the channel's MIDI CC number for `CCV`.
    CcNumber,
    /// `CCVxx`: send value xx on the channel's CC number.
    CcValue,
    /// `BRKxx`: break to row xx of the next order position.
    Break,
    /// `PRGxx`: program change.
    Program,
    /// `SPDxx`: ticks per row.
    Speed,
    /// `TPOxx`: tempo in BPM.
    Tempo,
    /// `TBLxx`: run table xx with the channel's note, from its first row.
    Table,
}

impl FxCmd {
    /// Every command, in display order.
    pub const ALL: [FxCmd; 18] = [
        FxCmd::Arp,
        FxCmd::Break,
        FxCmd::CcNumber,
        FxCmd::CcValue,
        FxCmd::Chance,
        FxCmd::Delay,
        FxCmd::Jump,
        FxCmd::Offset,
        FxCmd::Portamento,
        FxCmd::Program,
        FxCmd::Random,
        FxCmd::SlideDown,
        FxCmd::SlideUp,
        FxCmd::Speed,
        FxCmd::Table,
        FxCmd::Tempo,
        FxCmd::Vibrato,
        FxCmd::VolumeSlide,
    ];

    /// The three-letter name.
    pub fn name(self) -> &'static str {
        match self {
            FxCmd::Arp => "ARP",
            FxCmd::SlideUp => "SLU",
            FxCmd::SlideDown => "SLD",
            FxCmd::Portamento => "POR",
            FxCmd::Vibrato => "VIB",
            FxCmd::VolumeSlide => "VSL",
            FxCmd::Delay => "DLY",
            FxCmd::Chance => "CHA",
            FxCmd::Random => "RND",
            FxCmd::Offset => "OFS",
            FxCmd::Jump => "JMP",
            FxCmd::CcNumber => "CCN",
            FxCmd::CcValue => "CCV",
            FxCmd::Break => "BRK",
            FxCmd::Program => "PRG",
            FxCmd::Speed => "SPD",
            FxCmd::Tempo => "TPO",
            FxCmd::Table => "TBL",
        }
    }

    /// One-line description, for help text.
    pub fn summary(self) -> &'static str {
        match self {
            FxCmd::Arp => "arpeggio: note, +x, +y semitones",
            FxCmd::SlideUp => "slide pitch up xx/16 semitone per tick",
            FxCmd::SlideDown => "slide pitch down xx/16 semitone per tick",
            FxCmd::Portamento => "slide to the row's note at xx/16 per tick",
            FxCmd::Vibrato => "vibrato: speed x, depth y",
            FxCmd::VolumeSlide => "volume slide: up x, down y per tick",
            FxCmd::Delay => "delay the note by xx ticks",
            FxCmd::Chance => "play the note with probability xx/255",
            FxCmd::Random => "repeat last continuous effect, varied by +/-xx",
            FxCmd::Offset => "start the sample xx/256 into its span",
            FxCmd::Jump => "jump to order position xx",
            FxCmd::CcNumber => "set the CC number CCV sends on",
            FxCmd::CcValue => "send CC value xx",
            FxCmd::Break => "break to row xx of the next position",
            FxCmd::Program => "program change xx",
            FxCmd::Speed => "ticks per row",
            FxCmd::Tempo => "tempo in BPM",
            FxCmd::Table => "run table xx with the note",
        }
    }

    /// Parse a three-letter name, any case.
    pub fn from_name(name: &str) -> Option<FxCmd> {
        FxCmd::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// Effects read on every tick, whose parameter `RND` may vary. A
    /// randomized jump, break or program change would scramble playback.
    pub fn is_continuous(self) -> bool {
        matches!(
            self,
            FxCmd::Arp | FxCmd::SlideUp | FxCmd::SlideDown | FxCmd::Vibrato | FxCmd::VolumeSlide
        )
    }
}

/// One effect slot: a command and its parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fx {
    pub cmd: FxCmd,
    pub param: u8,
}

impl Fx {
    pub fn new(cmd: FxCmd, param: u8) -> Self {
        Self { cmd, param }
    }

    /// A cell's effect slots holding only this effect, in the first slot.
    pub fn only(cmd: FxCmd, param: u8) -> [Option<Fx>; FX_COLUMNS] {
        let mut fx = [None; FX_COLUMNS];
        fx[0] = Some(Fx::new(cmd, param));
        fx
    }

    /// Five-character display form, e.g. `ARP37`.
    pub fn display(self) -> String {
        format!("{}{:02X}", self.cmd.name(), self.param)
    }

    /// Parse the display form.
    pub fn parse(s: &str) -> Option<Fx> {
        if s.len() != 5 || !s.is_ascii() {
            return None;
        }
        let cmd = FxCmd::from_name(&s[..3])?;
        let param = u8::from_str_radix(&s[3..], 16).ok()?;
        Some(Fx { cmd, param })
    }

    /// Convert a version 1 effect: one hex digit plus a parameter.
    ///
    /// `Cxx` took its controller number from the instrument column, so it
    /// becomes `CCN` with that number followed by `CCV`. `Fxx` set the speed
    /// below 0x20 and the tempo from 0x20. `A` was unused.
    pub fn from_legacy(
        effect: Option<u8>,
        value: Option<u8>,
        instrument: Option<u8>,
    ) -> [Option<Fx>; FX_COLUMNS] {
        let Some(effect) = effect else {
            return [None; FX_COLUMNS];
        };
        let p = value.unwrap_or(0);
        let cmd = match effect {
            0x0 => FxCmd::Arp,
            0x1 => FxCmd::SlideUp,
            0x2 => FxCmd::SlideDown,
            0x3 => FxCmd::Portamento,
            0x4 => FxCmd::Vibrato,
            0x5 => FxCmd::VolumeSlide,
            0x6 => FxCmd::Delay,
            0x7 => FxCmd::Chance,
            0x8 => FxCmd::Random,
            0x9 => FxCmd::Offset,
            0xB => FxCmd::Jump,
            0xC => {
                let mut fx = [None; FX_COLUMNS];
                fx[0] = Some(Fx::new(FxCmd::CcNumber, instrument.unwrap_or(0)));
                fx[1] = Some(Fx::new(FxCmd::CcValue, p));
                return fx;
            }
            0xD => FxCmd::Break,
            0xE => FxCmd::Program,
            0xF if p < 0x20 => FxCmd::Speed,
            0xF => FxCmd::Tempo,
            _ => return [None; FX_COLUMNS],
        };
        Fx::only(cmd, p)
    }
}

impl Serialize for Fx {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.display())
    }
}

impl<'de> Deserialize<'de> for Fx {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Fx::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad effect {s:?}")))
    }
}

/// What typing has produced so far in an effect slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxKey {
    /// The key changed the slot.
    Changed,
    /// The key extended a partial command name, shown by [`FxTyping::pending`].
    Pending,
    /// The key does not fit here.
    Rejected,
}

/// Typing into an effect slot.
///
/// On an empty slot, letters spell a command name; the command is set as soon
/// as the letters name exactly one. On a set slot, hex digits shift into the
/// parameter, and any other letter starts a new name. A command starting with
/// A-F replaces a set one after the slot is cleared.
#[derive(Debug, Default, Clone)]
pub struct FxTyping {
    buf: String,
    at: Option<(usize, usize, usize, usize)>,
    /// A command set from a prefix, and how many of its letters were typed.
    /// The rest of its name may still be typed and is absorbed.
    completing: Option<(FxCmd, usize)>,
}

impl FxTyping {
    /// Typing at `pos` (order, row, channel, field). A partial name typed
    /// somewhere else is dropped, so moving the cursor cannot carry it over.
    pub fn at(&mut self, pos: (usize, usize, usize, usize)) -> &mut Self {
        if self.at != Some(pos) {
            self.reset();
            self.at = Some(pos);
        }
        self
    }

    /// Whether a partial name is pending at `pos`.
    pub fn pending_at(&self, pos: (usize, usize, usize, usize)) -> Option<&str> {
        (self.at == Some(pos) && !self.buf.is_empty()).then_some(self.buf.as_str())
    }

    /// Letters typed toward a command name that is not yet decided.
    pub fn pending(&self) -> &str {
        &self.buf
    }

    /// Drop a partial name, e.g. when the cursor moves.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.completing = None;
    }

    /// Apply one typed character to `slot`.
    pub fn key(&mut self, slot: &mut Option<Fx>, c: char) -> FxKey {
        let c = c.to_ascii_uppercase();
        if let Some((cmd, typed)) = self.completing.take() {
            let still_set = slot.is_some_and(|fx| fx.cmd == cmd);
            if still_set && cmd.name().as_bytes().get(typed) == Some(&(c as u8)) {
                if typed + 1 < 3 {
                    self.completing = Some((cmd, typed + 1));
                }
                return FxKey::Changed;
            }
        }
        if let (Some(fx), true) = (slot.as_mut(), self.buf.is_empty()) {
            if let Some(d) = c.to_digit(16) {
                fx.param = (fx.param << 4) | d as u8;
                return FxKey::Changed;
            }
        }
        if !c.is_ascii_alphabetic() {
            self.buf.clear();
            return FxKey::Rejected;
        }
        self.buf.push(c);
        let matches: Vec<FxCmd> = FxCmd::ALL
            .into_iter()
            .filter(|cmd| cmd.name().starts_with(self.buf.as_str()))
            .collect();
        match matches.as_slice() {
            [] => {
                self.buf.clear();
                FxKey::Rejected
            }
            [cmd] => {
                if self.buf.len() < 3 {
                    self.completing = Some((*cmd, self.buf.len()));
                }
                self.buf.clear();
                *slot = Some(Fx::new(*cmd, 0));
                FxKey::Changed
            }
            _ => FxKey::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_three_letters_and_round_trip() {
        for cmd in FxCmd::ALL {
            assert_eq!(cmd.name().len(), 3);
            assert_eq!(FxCmd::from_name(cmd.name()), Some(cmd));
            let fx = Fx::new(cmd, 0x4A);
            assert_eq!(Fx::parse(&fx.display()), Some(fx));
        }
        let mut names: Vec<_> = FxCmd::ALL.iter().map(|c| c.name()).collect();
        names.dedup();
        assert_eq!(names.len(), FxCmd::ALL.len(), "ALL is sorted and unique");
    }

    #[test]
    fn legacy_effects_map_by_digit() {
        assert_eq!(
            Fx::from_legacy(Some(0x0), Some(0x37), None),
            Fx::only(FxCmd::Arp, 0x37)
        );
        assert_eq!(
            Fx::from_legacy(Some(0xF), Some(0x06), None),
            Fx::only(FxCmd::Speed, 6)
        );
        assert_eq!(
            Fx::from_legacy(Some(0xF), Some(0x8C), None),
            Fx::only(FxCmd::Tempo, 0x8C)
        );
        assert_eq!(
            Fx::from_legacy(Some(0xA), Some(1), None),
            [None; FX_COLUMNS]
        );
        assert_eq!(Fx::from_legacy(None, Some(1), None), [None; FX_COLUMNS]);
    }

    #[test]
    fn legacy_cc_moves_the_controller_out_of_the_instrument_column() {
        assert_eq!(
            Fx::from_legacy(Some(0xC), Some(0x40), Some(74)),
            [
                Some(Fx::new(FxCmd::CcNumber, 74)),
                Some(Fx::new(FxCmd::CcValue, 0x40))
            ]
        );
    }

    #[test]
    fn typing_a_unique_prefix_sets_the_command() {
        let mut t = FxTyping::default();
        let mut slot = None;
        // "V" is ambiguous (VIB, VSL), "VI" is not.
        assert_eq!(t.key(&mut slot, 'v'), FxKey::Pending);
        assert_eq!(t.pending(), "V");
        assert_eq!(t.key(&mut slot, 'i'), FxKey::Changed);
        assert_eq!(slot, Some(Fx::new(FxCmd::Vibrato, 0)));
        // Then hex digits fill the parameter, including A-F.
        t.key(&mut slot, '4');
        t.key(&mut slot, 'a');
        assert_eq!(slot, Some(Fx::new(FxCmd::Vibrato, 0x4A)));
    }

    #[test]
    fn typing_the_rest_of_a_completed_name_is_absorbed() {
        let mut t = FxTyping::default();
        let mut slot = None;
        for c in "tpo8c".chars() {
            t.key(&mut slot, c);
        }
        assert_eq!(slot, Some(Fx::new(FxCmd::Tempo, 0x8C)));
        // A letter that does not continue the name still starts a new one.
        let mut slot = None;
        t.reset();
        t.key(&mut slot, 'd');
        assert_eq!(t.key(&mut slot, 'x'), FxKey::Rejected);
        assert_eq!(slot, Some(Fx::new(FxCmd::Delay, 0)));
    }

    #[test]
    fn typing_a_name_with_no_match_is_rejected() {
        let mut t = FxTyping::default();
        let mut slot = None;
        assert_eq!(t.key(&mut slot, 'x'), FxKey::Rejected);
        assert_eq!(t.pending(), "");
        assert_eq!(slot, None);
        assert_eq!(t.key(&mut slot, '7'), FxKey::Rejected, "no command to fill");
    }

    #[test]
    fn a_non_hex_letter_on_a_set_slot_starts_a_new_command() {
        let mut t = FxTyping::default();
        let mut slot = Some(Fx::new(FxCmd::Arp, 0x37));
        assert_eq!(t.key(&mut slot, 'p'), FxKey::Pending);
        assert_eq!(t.key(&mut slot, 'r'), FxKey::Changed);
        assert_eq!(slot, Some(Fx::new(FxCmd::Program, 0)));
    }

    #[test]
    fn effects_serialize_as_their_display_form() {
        let fx = Fx::new(FxCmd::Tempo, 0x8C);
        assert_eq!(serde_json::to_string(&fx).unwrap(), "\"TPO8C\"");
        assert_eq!(serde_json::from_str::<Fx>("\"tpo8c\"").unwrap(), fx);
        assert!(serde_json::from_str::<Fx>("\"XYZ00\"").is_err());
    }
}
