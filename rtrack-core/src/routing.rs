//! Where a note plays: one decision for live playback and offline render
//! (`docs/dev/g4-g8-format.md`). Each note plays in exactly one place.

use crate::audio::synth::SynthParams;
use crate::sample::SampleBank;
use crate::types::{Instrument, InstrumentKind, MidiOutput, SampleFx};

/// The sound source for one note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route<'a> {
    /// The sample in this bank slot, through the instrument's drive and filter.
    Sample(usize, SampleFx),
    /// The built-in synth with these parameters.
    Synth(&'a SynthParams),
    /// The built-in voice for this program: SF2 if one is loaded, else the
    /// built-in synth's preset.
    Preset(u8),
    /// The channel's current voice: SF2 if loaded, else the built-in synth
    /// with the channel's current program.
    ChannelVoice,
    /// The selected MIDI port only, on `channel` when the instrument
    /// overrides the track's. Without a selected port, as in offline render,
    /// it plays as `Preset` with the same program.
    External { program: u8, channel: Option<u8> },
    /// Nothing: a sample instrument whose sample is not loaded.
    Silent,
}

/// Route a note played with `inst`.
///
/// `explicit` is whether the note named its instrument. A note without one
/// is given instrument 0 by the caller; it plays that instrument only when
/// it is a loaded sample or a synth, and the channel's voice otherwise.
pub fn route<'a>(inst: Option<&'a Instrument>, explicit: bool, bank: &SampleBank) -> Route<'a> {
    let Some(inst) = inst else {
        return Route::ChannelVoice;
    };
    match &inst.kind {
        InstrumentKind::Sample { slot, fx } if bank.get(*slot).is_some() => {
            Route::Sample(*slot, *fx)
        }
        InstrumentKind::Synth(params) => Route::Synth(params),
        _ if !explicit => Route::ChannelVoice,
        InstrumentKind::Sample { .. } => Route::Silent,
        InstrumentKind::Midi {
            program,
            channel,
            output: MidiOutput::External { .. },
        } => Route::External {
            program: *program,
            channel: *channel,
        },
        InstrumentKind::Midi { program, .. } => Route::Preset(*program),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Sample;
    use std::sync::Arc;

    fn bank_with_slot_0() -> SampleBank {
        let mut bank = SampleBank::new();
        bank.samples[0] = Some(Arc::new(Sample {
            name: "s".into(),
            data: vec![[0.0, 0.0]; 4].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }));
        bank
    }

    fn with(kind: InstrumentKind) -> Instrument {
        Instrument {
            kind,
            ..Instrument::for_slot(0)
        }
    }

    #[test]
    fn each_kind_routes_to_one_place() {
        let bank = bank_with_slot_0();
        let params = SynthParams::from_patch(1);
        assert_eq!(
            route(Some(&with(InstrumentKind::sample(0))), true, &bank),
            Route::Sample(0, SampleFx::default())
        );
        assert_eq!(
            route(
                Some(&with(InstrumentKind::Synth(params.clone()))),
                true,
                &bank
            ),
            Route::Synth(&params)
        );
        assert_eq!(
            route(Some(&Instrument::for_slot(7)), true, &bank),
            Route::Preset(7)
        );
        let external = with(InstrumentKind::Midi {
            program: 0,
            channel: None,
            output: MidiOutput::External { port: None },
        });
        assert_eq!(
            route(Some(&external), true, &bank),
            Route::External {
                program: 0,
                channel: None
            }
        );
    }

    #[test]
    fn a_missing_sample_is_silent() {
        let bank = SampleBank::new();
        let inst = with(InstrumentKind::sample(0));
        assert_eq!(route(Some(&inst), true, &bank), Route::Silent);
    }

    #[test]
    fn a_note_without_an_instrument_uses_instrument_0_only_if_it_sounds() {
        let bank = bank_with_slot_0();
        let sample = with(InstrumentKind::sample(0));
        assert_eq!(
            route(Some(&sample), false, &bank),
            Route::Sample(0, SampleFx::default())
        );
        assert_eq!(
            route(Some(&Instrument::for_slot(0)), false, &bank),
            Route::ChannelVoice
        );
        let missing = with(InstrumentKind::sample(3));
        assert_eq!(route(Some(&missing), false, &bank), Route::ChannelVoice);
    }
}
