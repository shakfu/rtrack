# G5: instrument filter and drive

Gives every audio instrument its own filter and drive, so an instrument sounds the same on any track. Closes G5 in [design-gaps.md](design-gaps.md). Code refers to `rtrack-core/src` as of 2026-10-13.

Status: implemented 2026-10-13, with the proposal for each decision: add and keep the track's, filter for samples only, pattern commands deferred to G6.

## Current state

- **Track effects.** `ChannelConfig.effects_params: ChannelEffectsParams` holds drive, filter, chorus, delay, reverb and send levels. `ChannelEffects::process` runs them in that order on the track's summed signal (`audio/channel_effects.rs`).
- **Synth instruments** already have a per-voice filter: `SynthParams.filter_cutoff` (a multiple of the note frequency), resonance, `filter_env` and `filter_type`, in `Voice.filter` (`audio/synth.rs`). They have no drive. Built-in presets (`InstrumentKind::Midi` on the built-in synth) take a filter from `patch_params`.
- **Sample instruments** have neither. `SampleVoice` (`sample/playback.rs`) has an envelope and no filter state.
- **SoundFont output** is mixed into master before the per-track path (`audio/mod.rs`, `render_block`), so track effects never reach it. External MIDI has no audio.
- **Live control of the track filter.** MIDI-learned CCs (`LearnableParam`) and aftertouch (`apply_aftertouch_to_filter`) set track effect parameters.
- **Pattern `Cnn`** goes to MIDI out and the SoundFont only. No pattern command reaches a track or instrument filter.

So the gap is narrower than design-gaps.md states: synth instruments already carry a filter; samples carry nothing; no instrument has drive.

## Options

**A. Move filter and drive to the instrument** (design-gaps option 2). Track effects keep chorus, delay, reverb and sends.
- Old songs change sound. A track filter applies to whatever plays on the track; there is no exact instrument equivalent when one instrument plays on two tracks with different settings, or two instruments share one filtered track.
- MIDI learn and aftertouch lose their target, or need a rule for which instrument they change.
- A sweep over a whole track, a mixing move, is no longer possible.

**B. Add an instrument filter and drive; keep the track's.** An insert per instrument and an insert per track, as a DAW has.
- Old songs play unchanged; no migration.
- MIDI learn, aftertouch and the track editors keep working.
- Two filters can be in series on one note. Both default off.

| | A | B |
|-|-|-|
| Old songs | sound changes; lossy migration | unchanged |
| Instrument sounds the same on any track | yes | yes, if its track filter is off |
| Track-wide sweep (MIDI learn, aftertouch) | lost or retargeted | kept |
| Settings to explain | one filter per note | up to two |

## Recommendation: B

B delivers what G5 asks for, an instrument that carries its sound, without breaking any saved song or the live-control path. A's only gain is removing a control, and the cost of that is a lossy migration.

Per-voice and per-track drive also sound different: drive on a summed track intermodulates the voices; per-voice drive does not. B keeps both.

## Model (B)

```rust
/// Per-voice processing an audio instrument applies before its envelope
/// ends in the track mix. Off by default.
pub struct VoiceFx {
    /// 1.0 is clean.
    pub drive: f32,
    pub filter: Option<VoiceFilter>,
}

pub struct VoiceFilter {
    pub kind: FilterType,  // reused from synth: low, high, band pass
    pub cutoff_hz: f32,    // 20..20000
    pub resonance: f32,    // 0..1
}

pub struct Instrument {
    // ...
    #[serde(default, skip_serializing_if = "VoiceFx::is_off")]
    pub voice_fx: VoiceFx,
}
```

- **Order.** Drive, then filter, as on the track.
- **Which kinds.** Sample voices get both. Synth and built-in preset voices get drive; their own filter stays, so `VoiceFx.filter` is not offered for them (decision 2). SoundFont and external MIDI instruments produce no per-voice audio here; the editor hides the fields.
- **Engine state.** `SampleVoice` gains a stereo SVF (4 floats) and the drive amount. The SVF code moves out of `ChannelEffects` into a shared `Svf` so the track and the voice use one implementation.
- **Cost.** Coefficients are computed at note-on, since nothing modulates them until G6, so the per-frame cost is one `tanh` per channel when drive is on and the SVF update when the filter is on. 32 sample voices bound it.
- **Delivery.** The parameters travel with the note-on, as synth params do. Editing an instrument changes its next note, not one already sounding.
- **Render** uses the same note-on path, so it matches live.
- **File format.** `voice_fx` is a new optional field, omitted when off. It joins unreleased version 2.

## Decisions

1. **Model A or B.** Proposal: B.
2. **Filter on synth instruments.** Synth voices already have a filter with an envelope. A second `VoiceFx` filter would stack on it.
   - Proposal: no. `VoiceFx.filter` is for samples; synths and presets get drive only.
3. **Pattern control of the instrument filter** (M8's `CUT`, `RES`, `DRV`). Without it, a table cannot step the filter on internal instruments, which G7 listed as a table use; today that works only through `Cnn` on the SoundFont and MIDI out.
   - Proposal: defer to G6. Mod slots and pattern commands need the same per-note parameter path, and G6 designs it once. Until then, coefficients stay fixed per note.

## Work

1. **Done.** `audio/svf.rs`: `SvfCoeffs` and `Svf`, with low-, band- and high-pass outputs. The track filter uses it with the same operations as before. The data moved off the proposed `Instrument.voice_fx`: `InstrumentKind::Sample { slot, fx: SampleFx }` and `SynthParams.drive`, so a filter cannot be set on a kind that ignores it. Both are omitted from files when off. Values are clamped where they reach audio (`VoiceFx::new`, the synth voice), which covers files, editors and tests in one place; resonance stops at 0.95, since an undamped voice filter grows without bound on a resonant input.
2. **Done.** `Route::Sample` carries the instrument's `SampleFx`, so live notes, previews and render take it from one routing decision. Built-in presets (`InstrumentKind::Midi`) get no drive: the same instrument may route to a SoundFont or a port, where drive cannot apply. Fundsp pad voices skip drive, as they skip the filter. Tests: filter types pass their bands; a low-passed sample renders 5x smoother, and a driven quiet one 3x louder; synth drive squares a sine; hostile values (NaN cutoff, infinite drive, resonance 50) render finite audio; default settings serialize as before.
3. **Done.** TUI: Drive, Filter, Cutoff (semitone steps) and Resonance rows in the sample editor, each run of presses one undo step; Drive in the synth editor. GUI: a Drive and Filter section for sample instruments (a drag is one undo step), and a Drive slider for synths. Synth parameter edits record no undo in either editor; that predates this work.
4. **Done.**

## Not in scope

- Moving or removing track effects.
- Per-instrument sends.
- Modulating cutoff or drive (G6), and pattern commands for them (decision 3).
- Filtering SoundFont output, which is mixed before the per-track path.

## Found while reading

- Render ignores pattern `Cnn` (`sample/export.rs`, no `MidiCC` arm), so CC changes reach a SoundFont live but not in render. Added to TODO.
