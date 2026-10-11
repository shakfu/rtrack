# Design gaps

Specific structural gaps in rtrack, with options to close each. Background and sources are in [comparisons.md](comparisons.md). Code refers to `rtrack-core/src` at `efeb1da` unless stated.

Each gap lists: current state, consequence, options, and a recommendation. Recommendations are proposals, not decisions.

## Constraints on any resolution

These hold for every option below:

- One source of truth. The removed chain layer failed because it was derived from patterns (`docs/new_features.md` section 1).

- `.rtrk` files from earlier versions still load.

- MIDI import and export keep working (`midi_file.rs`).

- Offline render (`--render`) and live playback produce the same events.

- The audio callback does not allocate (CHANGELOG, Unreleased).

## Index

| ID | Gap | Area | Depends on |
|-|-|-|-|
| G1 | Reuse is whole-pattern only | Sequencing | - |
| G2 | No non-destructive transpose | Sequencing | G1 or G7 |
| G3 | No per-track play position | Sequencing | G1 |
| G4 | Sound-source type split between channel and instrument | Instruments | - |
| G5 | Effects bound to channel, not instrument | Instruments | G4 |
| G6 | No general modulation | Instruments | G5 |
| G7 | No per-note sub-sequence | Instruments | - |
| G8 | Effect command space exhausted | Pattern data | - |
| G9 | Timing is global only | Timing | G3 for per-track |
| G10 | Scale affects entry only | Timing/pitch | - |
| G11 | No live launch | Performance | G3 |
| G12 | One note per channel | Pattern data | - |

---

## G1. Reuse is whole-pattern only

**Current.** `Song.order: Vec<usize>` indexes `Song.patterns`. A `Pattern` holds every channel (`data[row][channel]`). The only reuse unit is the full-width pattern, repeated through `order` or `order_repeats`.

**Consequence.** Changing one channel in one section needs a new pattern that copies all other channels. Later edits to the shared parts must be repeated in each copy. Example: 4 sections that share drums but differ in bass need 4 patterns, each with its own drum copy.

**Options.**

1. *Block aliases (Renoise).* Each `(pattern, channel)` column is either owned data or an alias to the same channel of another pattern.

   - Data: `Pattern.columns: Vec<Column>`, `enum Column { Owned(Vec<Cell>), Alias(usize) }`. This requires turning `data[row][channel]` into column-major storage.

   - Reads resolve aliases. Writes to an alias write the source. "Un-alias" copies the source.

   - Keeps one global position. `tempo_map`, `Bxx`, `Dxx` and MIDI import are unaffected.

   - Failure modes: alias cycles and dangling aliases after pattern deletion. Both are rejected on load by `Song::repair`.

   - Length mismatch: the source may differ in `rows` from the target. Choose one rule: truncate, loop, or refuse. Renoise refuses implicitly, since all tracks in a pattern share one length.

2. *Chains and phrases (M8).* Single-channel phrases are referenced from per-channel chains. This forces G3, because each channel then needs its own position.

3. *Copy-on-write sharing.* Patterns stay full-width, but an identical channel column is stored once and shared by hash. The user gets no reuse semantics: an edit unshares the column. This saves file size only, so it does not close the gap.

**Recommendation.** Option 1. It gives per-channel reuse at the lowest cost to the existing invariants. Decide the length-mismatch rule before building it.

**Open question.** Is the pain reuse or polymeter? If it is only reuse, option 1 closes G1 without G3.

## G2. No non-destructive transpose

**Current.** Transpose rewrites cells (`Cell::transpose_note`, `Note::transposed`). The removed chain layer had a chain transpose, but each rebuild reset it to zero (CHANGELOG, Unreleased > Changed).

**Consequence.** A chord progression over one riff needs one transposed copy per chord. Edits to the riff must be repeated in each copy.

**Options.**

1. *Transpose on the alias (extension of G1 option 1).* `Column::Alias { source, transpose: i8 }`. Renoise does not do this; M8 chain `TSP` does the same job.

2. *Transpose on the order entry.* `order_transpose: Vec<i8>`, parallel to `order_repeats`. It applies to all channels, so drums need an opt-out.

3. *Per-instrument transpose opt-out (M8 `TRANSP.`).* An instrument flag that ignores all transposes. This is needed with option 2, and is useful with option 1.

4. *Phrase key tracking (Renoise).* See G7. A pattern note plays a phrase transposed to that note.

**Recommendation.** Option 1 if G1 option 1 is adopted. Add option 3 with it. Option 2 is cheaper but coarser, and the drum opt-out makes it depend on option 3 anyway.

**Invariant.** Transpose applies at playback. Transposed cells are never written back, which is how the chain layer lost data.

## G3. No per-track play position

**Current.** `TrackerEngine` has one `(order, row)`, plus `sounding_order`/`sounding_row`. `ScheduledPosition` stores `(frame, order, row)`. `tempo_map` keys on `(order, row)` (`Song::tempo_at`). `Bxx` and `Dxx` move the single position.

**Consequence.** Polymeter (for example a 3-step against a 4-step loop), per-track loop lengths, and per-track cueing (G11) cannot be expressed.

**Options.**

1. *Per-channel order lists over single-channel patterns (Buzz-like).* Each channel has its own `order` and its own position.

   - `tempo_map` must re-key to absolute time or to a master channel.

   - `Bxx`/`Dxx` become per-channel jumps, like M8 `HOP`, or are removed.

   - The playback cursor becomes one row per channel.

   - MIDI import and export need a single timeline, so they flatten.

2. *Full M8 model.* Song grid of chain IDs, chains of `(phrase, transpose)`, 16-step phrases. Same costs as option 1, plus the chain level. The fixed 16-step phrase limits rtrack's 256-row patterns.

3. *Per-column loop length inside a pattern.* A column loops after `n` rows while the pattern continues. Renoise users request this and do not have it ([forum](https://forum.renoise.com/t/solved-aliases-how-to-use-8-row-alias-in-64-row-pattern/34665)). One global position remains. Polymeter is available within a pattern, and it resets at pattern boundaries.

4. *Looping sub-sequences (G7).* An instrument phrase with its own length and loop gives polymeter per note, not per track.

**Recommendation.** Option 3 or 4 first. Both give most of the musical effect, keep the global position, and leave `tempo_map` and flow commands as they are. Take option 1 only if live per-track launch (G11) becomes a goal.

## G4. Sound-source type split between channel and instrument

**Current.**

- `ChannelConfig.channel_type: ChannelType { Midi, Synth, Sample }`.

- `Instrument` implies its type by which of `sample_index`, `synth_params` or `midi_program` is set.

- `TrackerCore::route_note_on` (`core.rs`) routes by instrument: sample first, then synth or MIDI.

- `ChannelType` only gates `default_instrument` fill (`TrackerEngine::resolve_instrument`, `TrackerCore::track_default_instrument`) and the editor fields the TUI shows.

- The sample route also calls `midi.note_on`.

**Consequence.**

- A user who sets a channel to Sample and enters a synth instrument gets synth audio. The channel type does not say what the channel plays.

- An instrument with both `sample_index` and `synth_params` set has an implicit precedence.

- MIDI receives notes the user meant only for samples.

**Options.**

1. *Typed instruments (M8).* `enum InstrumentKind { None, Synth(SynthParams), Sample(SampleRef), Midi { channel, program } }`. Remove `ChannelType`. Default-instrument fill becomes a per-channel option that does not depend on type.

2. *Keep `ChannelType` as a hard filter.* The channel refuses instruments of another kind. This makes the split explicit, but keeps two concepts.

3. *Keep both, document precedence.* No model change.

**Recommendation.** Option 1. It removes a concept and an implicit precedence rule. Migration: map each existing instrument by its current routing precedence. MIDI channel moves from `ChannelConfig.midi_channel` to the MIDI instrument, as M8 MIDI Out does. A "MIDI on every note" behaviour, if wanted, becomes an explicit per-instrument option.

## G5. Effects bound to channel, not instrument

**Current.** `ChannelConfig.effects_params: ChannelEffectsParams`: filter, distortion, chorus, delay, and `send_levels: [f32; MAX_SEND_BUSES]`. `SynthParams` has its own filter and envelope. Sample instruments have none.

**Consequence.**

- An instrument sounds different depending on the channel that plays it.

- Two instruments on one channel cannot have different sends or filters.

- A drum kit spread over one channel shares one effects chain.

**Options.**

1. *Instrument-owned processing (M8).* Filter, drive and send levels move to the instrument. The channel keeps volume, pan and mute. The DSP state remains per voice or per channel, but its parameters come from the instrument that is playing.

2. *Both levels.* The instrument holds the sound-design effects (filter, drive). The channel holds the mixing effects (sends, chorus, delay). This is closer to a DAW: an insert per instrument and a bus per channel.

3. *Status quo* with copy and paste of channel effects.

**Recommendation.** Option 2. Filter and drive belong to the sound, so they move to the instrument. Sends belong to the mix, so they stay on the channel. Do G4 first, so every instrument kind has one place to hold these parameters.

**Cost.** Per-voice filter state for samples. The 32-voice pool (`audio/synth.rs`, `MAX_VOICES`) bounds the cost.

## G6. No general modulation

**Current.** `SynthParams` has ADSR and `filter_env`. Pattern effects provide vibrato (`4xy`), slides (`1xx`, `2xx`, `5xy`) and arpeggio (`0xy`). There is no LFO or modulation routing. `SampleRef` has no envelope fields.

**Consequence.** Evolving timbres need dense effect-column data or MIDI CC automation to external gear.

**Options.**

1. *Mod slots (M8).* N slots per instrument. Each slot has a kind (AHD, ADSR, LFO, tracking), a destination, an amount and a rate. M8 has 4 slots, and slots can modulate the next slot (p.18). Rate is tempo-synced in ticks.

2. *Renoise-style modulation sets.* More general and more UI. Not researched here.

3. *Tables (G7) as modulation.* Table rows that set parameters per tick give stepped modulation.

**Recommendation.** Option 1 with 2 slots, without slot-to-slot modulation at first. Depends on G5, since destinations must be instrument parameters.

## G7. No per-note sub-sequence

**Current.** `0xy` arpeggio is the only per-note sequence. `8xx` re-runs the last continuous effect with a random variation.

**Consequence.** Repeating per-note behaviour (arpeggio shapes, rolls, stutter, stepped filter moves) must be written out row by row.

**Options.**

1. *Tables (M8).* Each instrument owns a 16-row table, plus a free pool selected by `TBL`. A row holds transpose, volume and FX. Each table has its own tick rate, with note, velocity and octave map modes (p.24). It restarts on each note.

2. *Instrument phrases (Renoise).* A single-track sub-pattern inside the instrument, with its own length, lines per beat and loop. Selection is by key range or a program command. Key tracking is None, Transpose or Offset.

3. *Both.* M8 tables act at instrument level, below the note. Renoise phrases act at note level, above the note.

**Recommendation.** Option 1, restricted to transpose, volume and one FX column. It is the smallest addition that also covers parts of G2, G3 and G6. Tables can grow into phrases later. Phrases cannot easily shrink into tables.

**Engine impact.** `ChannelState` gains a table cursor and a tick counter. Table events must use the same frame scheduling as row events, so render and playback stay identical.

## G8. Effect command space exhausted

**Current.** `Cell.effect: Option<u8>` holds one hex digit. Digits `0`-`9` and `B`-`F` are used (`constants.rs`). Only `A` is free. `Cxx` takes its controller number from the instrument column.

**Consequence.**

- Each new feature (G7 `TBL`, G9 groove, G10 scale) needs a command, and only one digit remains.

- `Cxx` cannot address an instrument and a controller in the same cell.

- One column means a cell cannot combine, for example, a slide with a delay.

**Options.**

1. *Named commands (M8).* Three-letter mnemonics (`ARP`, `TBL`, `HOP`) stored as an enum, displayed as text. The command space becomes effectively unlimited.

2. *Two-character commands (Renoise `ZLxx`).* An extended prefix: `A` plus a sub-command digit, with one byte of parameter. Keeps hex input.

3. *More FX columns.* M8 has 3 per step and Renoise up to 8 per track. This is orthogonal to options 1 and 2.

**Recommendation.** Option 1 or 2 for namespace, plus option 3 with 2 columns. `Cell.effect` becomes `[Option<Fx>; N]`. Serde must still read the single-field form.

## G9. Timing is global only

**Current.** `Song.speed` (ticks per row) and `Song.swing` (0-100) apply to every channel. Swing alternates long and short on row pairs (`Song` row duration, `TrackerEngine::seconds_per_tick`). `Fxx` sets speed or BPM.

**Consequence.**

- No triplet feel on one channel against straight time on another.

- No shuffle pattern other than the 2-row pair.

- No ratio-based timing such as 5:4.

**Options.**

1. *Grooves (M8).* A groove is a list of ticks per step, which loops at its first empty entry. Selected per track with `GRV`, or globally. Swing becomes the groove `[7, 5]`. Requires a tick base: M8 uses 24-192 PPQN (p.26).

2. *Song-level groove only.* Same data, one groove for all channels. Keeps the global position.

3. *Renoise lines per beat plus delay column.* Sub-line delay (256 steps per line) shifts notes without changing grid speed.

**Recommendation.** Option 2 first, replacing `swing`. Migration: `swing` 50 maps to an even groove. Other values map to the nearest tick pair, which is lossy. Per-track grooves need per-track step timing, which comes from G3 option 1.

## G10. Scale affects entry only

**Current.** `Song.scale: Option<ScaleSetting>` constrains keyboard entry and transpose. Playback ignores it.

**Consequence.** Transposed or randomised notes can leave the scale. The `0xy` arpeggio counts semitones, not scale steps.

**Options.**

1. *Playback quantisation (M8).* The scale also quantises transpose, arpeggio and pitch commands at playback. Per-instrument opt-out, same flag as G2 option 3.

2. *Per-channel scale (M8 `SCA`).* Different scales per channel.

3. *Microtonal offsets (M8).* Each scale degree carries a detune of -24 to +24 semitones. Requires pitch as `f64`, which the synth already uses for `pitch_offset`.

**Recommendation.** Option 1. It is small and makes G2 and G7 musically safer. Options 2 and 3 have no demand yet.

## G11. No live launch

**Current.** One transport. Mute and solo exist per channel. There is no cue state.

**Options.**

1. *M8 live mode.* Each track has its own cue, stop and quantised launch at chain end. Requires G3 option 1.

2. *Pattern queueing.* Queue the next order position, quantised to pattern end. Works with one global position.

**Recommendation.** Option 2 if live use is a goal. Otherwise defer.

## G12. One note per channel

**Current.** A pattern-row note cuts the channel's voice (`NewNoteAction::Cut` in `route_note_on`). `MAX_CHANNELS` is 16. Chords need one channel per voice.

**Comparison.** M8 has the same limit: one note per track, 8 tracks. Renoise allows up to 12 note columns per track.

**Options.**

1. *Note columns (Renoise).* `Cell` becomes a row of up to N note sub-cells per channel. Large change to editing, MIDI import and the file format.

2. *Chord command (M8 MIDI Out `CHD`).* One effect adds intervals to the note. This depends on G8.

3. *Status quo.* Use more channels.

**Recommendation.** Option 3 now and option 2 after G8. Option 1 only if chordal writing becomes a priority.

---

## Suggested order

Ordered by dependency, then by benefit per unit of cost:

1. G4: typed instruments. Removes a concept, and unblocks G5 and G6.

2. G8: command namespace and a second FX column. Unblocks G7, G9, G10 and G12 option 2.

3. G7: tables. Covers parts of G2, G3 and G6.

4. G1 + G2: block aliases with transpose.

5. G5: instrument-owned filter and drive.

6. G9 option 2 and G10 option 1.

7. G6: mod slots.

8. G3 option 1 and G11 option 1, only if per-track launch becomes a goal.

## Status

Steps 1-7 of the suggested order are implemented. Step 8 is not: it applies only if per-track launch becomes a goal. CHANGELOG, Unreleased, has the details. Where the build departs from a recommendation above:

- G4: notes with no instrument play the built-in synth and send no MIDI. MIDI output now needs a MIDI instrument.
- G8: named commands, as recommended. `Cxx` became `CCN` then `CCV`, so the instrument column no longer carries a controller number.
- G7: tables live in a pool, `Song.tables`. An instrument refers to one by index, and `TBL` selects one per note. There are no TIC map modes and no `HOP`; `length` shortens a table instead.
- G1/G2: a shorter alias source loops. This also gives option 3 of G3, a per-column loop length within one pattern. The transpose opt-out is `Instrument.fixed_pitch`, shared with G10.
- G5: the instrument stage runs per channel, not per voice. The channel keeps its own filter and drive as a second stage, because MIDI learn and aftertouch drive them.
- G9: swing converts to the nearest tick pair. `SPD` replaces the groove for the rest of playback.
- G6: AHD envelope and LFO sources only. There is no ADSR or tracking source, and no slot-to-slot modulation.


## Remaining

After steps 1-7, and the external MIDI clock fix.

### Not built

- G12 option 2: a chord command (`CHD`). G8 was its only dependency, so it is the one open recommendation.
- G11: live launch. Option 2, queueing the next order position at the pattern end, if live use becomes a goal.
- G3 option 1: a play position per channel. Only if per-track launch becomes a goal. Aliases that loop and tables already give polymeter within a pattern.

### Built smaller than recommended

- G9: one groove for the song. Per-channel grooves (`GRV`) need per-channel step timing, which is G3.
- G10: one scale for the song. No per-channel scale (`SCA`) and no microtonal detune.
- G6: envelope and LFO sources only. No ADSR or tracking source, and no slot-to-slot modulation.
- G7: no tick-rate map modes and no `HOP`. Tables do not extend to Renoise-style phrases with a keymap.
- G5: the instrument stage runs per channel. Overlapping voices on one channel, such as previews or samples that ring on, share one filter.
- G2: transpose exists on aliases only, not per order entry. The order-entry transpose was not recommended.

### Limits the changes introduced

- Channel settings are not undoable: name, default instrument, volume, pan and effects live outside the song, as instruments did. Instrument edits are now undoable (CHANGELOG, Fixed).
- Arpeggio, table and modulation pitch reach a MIDI instrument as pitch bend, limited by the device's bend range. At the default 2 semitones an octave step fails.
- MIDI output needs a MIDI instrument. Songs that drove gear from plain channels send nothing until they have one.
- Inserting or deleting a row shifts the pattern's own columns only. An aliased column keeps reading its source, so the two can fall out of step.
- The TUI pattern view needs about 111 columns for 4 channels. There is no narrow layout.
- Under external MIDI clock, playback follows the sender, but the displayed BPM does not.
- The new TUI popups and GUI sections were tested by unit tests only, not by hand.

### Outside this document

- M8 mixer processing: master EQ, limiter, OTT, DJ filter, and EQ banks.
- Renoise automation lanes, and group and send tracks inside the grid.

### Next

1. `CHD`.
2. A hands-on pass of both frontends' new editors.
3. Undo for channel settings, by the same snapshot route as instruments.
