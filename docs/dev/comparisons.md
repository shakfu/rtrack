# rtrack vs Dirtywave M8 and Renoise: structural comparison

Sources: M8 Operation Manual v20260421 (page numbers below); rtrack at `efeb1da`.

## 1. Sequencing hierarchy

| | M8 | rtrack |
|-|-|-|
| Levels | Song -> Chain -> Phrase (p.5) | Song -> order list -> Pattern (`rtrack-core/src/tracker/song.rs`, `Song`) |
| Unit of reuse | Phrase: 1 track x 16 steps (p.14) | Pattern: all channels x up to 256 rows (`MAX_ROWS_PER_PATTERN`) |
| Song grid | 256 rows x 8 tracks of chain IDs (p.10) | `order: Vec<usize>` + `order_repeats` |
| Chain | Up to 16 (phrase, transpose) rows (p.12) | none |
| Play position | One per track; each track loops on reaching `--` (p.10) | One global `(order, row)` in `TrackerEngine` |
| Transpose on reuse | Chain `TSP` column, project transpose | Destructive cell transpose only |

Consequences:

- Reuse granularity. M8 varies a bassline independently of drums. rtrack needs a new full-width pattern to change one channel.

- Polymeter. M8 chains of unequal length drift tracks out of phase (p.10). rtrack cannot express this; all channels share one row counter.

- Flow control. rtrack `Bxx`/`Dxx` jump the single global cursor. Under per-track positions a global jump is undefined; M8 uses per-track `HOP`.

## 2. Sound-source binding

| | M8 | rtrack |
|-|-|-|
| Instruments | 128, each typed: Wavsynth, Macrosynth, Sampler, FM, Hypersynth, MIDI Out, External (p.16) | 256 slots; type implied by `sample_index` / `synth_params` / `midi_program` (`types.rs`, `Instrument`) |
| Track type | None; 8 identical tracks (p.5) | `ChannelType` {Midi, Synth, Sample} per channel |
| Per-track polyphony | 1 note | Pattern rows cut the channel's voice; global pool of 32 voices (`audio/synth.rs`, `MAX_VOICES`) |

rtrack holds type in two places:

- The channel carries a `ChannelType`.

- `route_note_on` (`core.rs`) routes by instrument, not channel type.

- `ChannelType` only gates the `default_instrument` auto-fill (`engine/mod.rs` `resolve_instrument`, `core.rs` `track_default_instrument`).

- `midi.note_on` is sent even for sample-routed notes.

M8 has one rule: the instrument is the type. A track is a play position plus a mixer strip.

## 3. Effects and modulation

| | M8 | rtrack |
|-|-|-|
| Filter, drive, send levels | Per instrument (instrument "Mixer Parameters": `MFX`, `DEL`, `REV`) | Per channel (`ChannelConfig.effects_params`) |
| Modulators | 4 slots per instrument: AHD, ADSR, drum, LFO, trig, tracking; can modulate each other (p.18) | ADSR inside `SynthParams`; no general mod matrix |
| Tables | 1 per instrument + 128 free; 16-row sub-sequencers at their own tick rate (p.24) | none (`0xy` arpeggio and `8xx` randomize are the nearest) |
| FX columns | 3 per step | 1 (`Cell.effect`) |
| EQ | 128 shared instrument EQ banks, plus mix and send EQ (p.32) | none |

Consequence: an M8 instrument sounds the same on any track. An rtrack instrument takes on the effects of whichever channel plays it.

## 4. Timing

| | M8 | rtrack |
|-|-|-|
| Step length | Per-track groove: ticks per step, 24/48/96/192 PPQN (p.26) | Global `speed` (ticks per row) and global `swing` |
| Scale | 16 scales, per track via `SCA`, global via `SCG`; quantize playback events (p.28) | One `Song.scale`, applied to note entry and transpose only |
| Tempo automation | `TPO` command | `tempo_map` keyed by `(order, row)` (`TempoPoint`) |

## 5. Live performance

M8 live mode cues, stops and launches each track with quantization (p.10). It also queues song loads. rtrack has no equivalent; one play position cannot launch per track.

## Renoise model

Sources: Renoise manual pages [Pattern Matrix](https://tutorials.renoise.com/wiki/Pattern_Matrix), [Pattern Editor](https://tutorials.renoise.com/wiki/Pattern_Editor), [Phrase Editor](https://tutorials.renoise.com/wiki/Phrase_Editor), [Tracker Interface](https://tutorials.renoise.com/wiki/Tracker_Interface); forum threads where noted.

Renoise keeps the classic pattern + sequence model. It adds reuse at two levels: matrix-block aliases and instrument phrases.

### Structure

| | Renoise | rtrack |
|-|-|-|
| Levels | Pattern sequence -> Pattern -> Track; matrix block = one track of one pattern | Order list -> Pattern -> Channel |
| Pattern length | 1-512 lines; shared by all tracks of a pattern | 1-256 rows; shared by all channels |
| Columns per track | Up to 12 note columns + 8 master FX columns; each note column has a local FX sub-column | 1 note, 1 effect |
| Play position | One global position | One global position |
| Resolution | Lines per beat (LPB), changeable mid-song with `ZLxx` | `speed` (ticks per row) + `Fxx` |
| Track types | Sequencer, Group, Send, Master | Channel only; send buses outside the grid |

A track with 12 note columns is polyphonic within one track. An M8 track plays one note.

### Matrix-block aliases

- A block alias points one pattern's track at the same track of another pattern.

- "Any changes made to the content of one alias are automatically made to the others" -- Pattern Matrix page.

- Sharing is limited to Pattern Editor content. Per a [forum thread](https://forum.renoise.com/t/automating-or-fading-in-to-a-repeating-pattern/49579), automation does not vary per alias.

- "Un-alias" makes the block a separate copy.

- The manual documents no transpose or offset on an alias.

- Blocks can be muted individually, so one pattern can be reused with a track silenced.

- Track lengths cannot differ within a pattern. An 8-line source cannot loop under a 64-line pattern ([forum](https://forum.renoise.com/t/solved-aliases-how-to-use-8-row-alias-in-64-row-pattern/34665)). The suggested workaround is phrases.

### Instrument phrases

- A phrase is "the equivalent of a single track from the Pattern Editor", stored inside the instrument. Up to 120 per instrument.

- Each phrase has its own length, LPB and loop range, independent of the song.

- Trigger modes:

  - Keymap: phrases span key ranges.

  - Program: `Zxx` or MIDI program selects one phrase.

  - Off: plays the instrument directly.

- Key tracking:

  - None: the phrase plays identically on every key.

  - Transpose: pitches follow the played key, at unchanged speed.

  - Offset: the key selects the start line.

- A phrase restarts only on a new note event.

Phrases are the Renoise answer to M8 chain transpose and M8 tables. One pattern note runs a transposable sub-sequence at its own rate.

### Three models compared

| Need | M8 | Renoise | rtrack |
|-|-|-|-|
| Reuse one track's part | Phrase ID in chain | Block alias | Copy |
| Reuse transposed | Chain `TSP` | Phrase, key tracking = Transpose | Copy + transpose |
| Polymeter across tracks | Native: per-track positions | Looping phrases only | No |
| Per-note sub-sequence | Table | Phrase | `0xy` arpeggio only |
| Global flow and tempo map | Per-track `HOP`; `TPO` | Global; `ZLxx`, pattern loops | Global; `Bxx`, `Dxx`, `tempo_map` |

## Prior attempt

A chain layer was built and removed (`docs/new_features.md` section 1; `CHANGELOG.md`, Unreleased > Changed). It was a derived view rebuilt from patterns on every edit, so the song had two sources of truth. Results:

- Saving split shared patterns into copies.

- Chain transpose was reset on every rebuild.

Inference: this does not count against chains. It shows chains must be the primary representation, with patterns rendered from them or removed.

## Open questions

1. Adopt chains/phrases? Per-track positions break or force redesign of:

   - `tempo_map`, whose `(order, row)` key needs a global position;

   - `ScheduledPosition` and the playback cursor;

   - `Bxx`/`Dxx`;

   - MIDI import, which maps naturally to full-width patterns;

   - `.rtrk` compatibility.

   Which problem is being solved: reuse or polymeter? Reuse alone does not need independent positions.
2. M8's fixed 16-step phrase follows from a 320x240 screen and 8 buttons. rtrack patterns run up to 256 rows. Adopting the 16-step limit would be a regression.

3. Is `ChannelType` needed? It only drives default-instrument fill. M8-style typed instruments remove one concept. Channel effects would then move to instruments, or stay per-channel by explicit choice.

## Alternatives

- Block aliasing (Renoise model, above). Keep full-width patterns. A pattern's channel column may alias the same column of another pattern. This gives per-channel reuse. One global position remains, so `tempo_map`, `Bxx`, MIDI import and the file format survive. Renoise has no alias transpose. rtrack could add one as a per-alias field; that would be an extension, not the Renoise model. No polymeter.

  - Storage risk: an alias must be a reference, never a copy. The prior chain attempt failed on exactly this, when saving split shared patterns.

- Instrument phrases (Renoise model). A transposable, looping sub-sequence per instrument, with its own length and rate. This covers M8 tables, chain transpose, and limited polymeter, without touching the sequencer.

- Per-channel order lists (Buzz-style). One order list per channel over single-channel patterns. This is M8 without the chain level. Gives polymeter; breaks global flow commands.

- Tables first. Per-instrument tables are mostly independent of the hierarchy. They deliver much of M8's expressiveness with little change to the sequencer.

Gaps and proposed resolutions: [design-gaps.md](design-gaps.md).
