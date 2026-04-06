# New Feature Ideas

Analysis of LSDj (Game Boy tracker) and picoTracker (Xiphonics) for ideas applicable to rtrack.

## 1. Song > Chain > Phrase Architecture

**Source:** Both LSDj and picoTracker.
**Priority:** High -- foundational change that unlocks composition power.
**Status:** Not started.

### Problem

rtrack currently has a flat two-tier model: `Song` contains `patterns: Vec<Pattern>` and `order: Vec<usize>` (indices into patterns). Every order entry maps to one full pattern across all channels. This means:

- To reuse a melodic idea at a different pitch, you must duplicate the entire pattern and manually transpose every note.
- Per-channel variation requires creating new full patterns even when only one channel differs.
- The order list grows quickly for songs with recurring sections.

### How LSDj/picoTracker solve this

Both use a three-tier hierarchy:

```
Song Screen      4-8 columns (one per channel), each cell is a Chain index
                 Each channel has its own independent chain sequence
    |
    v
Chain            A short list (up to 16 entries) of Phrase references
                 Each entry has: phrase index + transpose (semitones)
    |
    v
Phrase           16 rows of note data (note, instrument, effects)
                 Equivalent to one channel-column of an rtrack Pattern
```

Key benefits:
- **Per-channel independence**: Channel A can play chain 05 while channel B plays chain 12. No need to create a full multi-channel pattern for every combination.
- **Transpose reuse**: The same phrase can appear in multiple chains at different transpositions. A 4-bar chord progression becomes 4 chain entries pointing to the same phrase with different transpose values.
- **Compact representation**: A song with 8 variations of a drum pattern only stores the base phrase once.

### Proposed design for rtrack

Rather than a full LSDj-style rewrite, extend the existing model incrementally. The goal is to get the compositional benefits while preserving backwards compatibility with the current `.rtrk` format.

#### Data model changes (rtrack-core/src/tracker/)

```rust
/// A phrase is a single-channel column of note data (replaces one channel
/// within a Pattern). Variable row count.
#[derive(Clone, Serialize, Deserialize)]
pub struct Phrase {
    pub rows: usize,
    pub data: Vec<Cell>,  // length = rows
}

/// A chain entry: play a phrase with optional transpose.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChainEntry {
    pub phrase: usize,        // index into Song::phrases
    pub transpose: i8,        // semitones (-128..+127)
}

/// A chain is a sequence of phrase references for one channel.
#[derive(Clone, Serialize, Deserialize)]
pub struct Chain {
    pub entries: Vec<ChainEntry>,  // typically 1-16 entries
}

/// Updated Song structure.
pub struct Song {
    pub title: String,
    pub bpm: u16,
    pub speed: u8,

    // -- New three-tier data --
    pub phrases: Vec<Phrase>,     // pool of reusable phrases
    pub chains: Vec<Chain>,       // pool of reusable chains

    // -- Song arrangement --
    // Each row is one "order entry". Each column is a channel.
    // Cell value is an Option<usize> index into chains.
    // None = channel silent for this row.
    pub arrangement: Vec<Vec<Option<usize>>>,  // [row][channel] -> chain index

    pub channels: usize,
    pub rows_per_phrase: usize,   // default phrase length (e.g. 16 or 64)
    // ... (existing fields: highlight_beat, highlight_bar, tempo_map, etc.)
}
```

#### How it maps to the current model

The current `Pattern` (multi-channel, N rows) decomposes into one `Phrase` per channel. The current `order: Vec<usize>` becomes `arrangement` rows where all channels in a row point to chains wrapping those phrases. Migration path:

- **Loading old .rtrk files**: Each `Pattern` with C channels splits into C `Phrase` objects. Each phrase gets wrapped in a trivial `Chain` (one entry, transpose=0). The order list maps to arrangement rows. This is lossless.
- **Saving**: Can round-trip to the new format, or optionally save in legacy format (recombining phrases into patterns) if all arrangement rows have the same chain length.

#### Engine changes (rtrack-core/src/engine/mod.rs)

The engine currently tracks a single `(order, row)` position. With chains, each channel has its own playback position:

```rust
pub struct ChannelPlayback {
    pub arrangement_row: usize,   // which row in the arrangement
    pub chain_entry: usize,       // which entry within the chain
    pub phrase_row: usize,        // which row within the phrase
}
```

The engine's `process_tick()` reads note data from `phrases[chain.entries[chain_entry].phrase]` and applies `chain.entries[chain_entry].transpose` to the MIDI note. Row advancement per channel:
1. Advance `phrase_row`. If it reaches the end of the phrase:
2. Advance `chain_entry`. If it reaches the end of the chain:
3. Advance `arrangement_row` (next song row). Pick up the next chain for this channel.

Channels can have chains of different total lengths -- the engine advances each independently and re-syncs at arrangement row boundaries.

#### UI changes

**Song/Arrangement screen** (replaces current order list):
- Grid view: rows = arrangement entries, columns = channels.
- Each cell shows a chain index (hex). Cursor can assign/create/clone chains.
- Per-cell transpose override visible as a small offset indicator.

**Chain screen** (new):
- List of phrase references with transpose column.
- Navigate here from the arrangement screen by pressing Enter on a chain cell.
- Add/remove/reorder phrase entries. Set transpose per entry.

**Phrase screen** (replaces current pattern editor for single-channel view):
- Same note/instrument/volume/effect columns as today, but for one channel.
- Navigate here from the chain screen by pressing Enter on a phrase entry.
- The current multi-channel pattern editor can remain as a "combined view" that shows all channels' current phrases side by side (read from the arrangement).

#### Migration strategy

Phase 1: Add `Phrase`, `Chain`, and arrangement data structures to the Song model. Keep the existing `Pattern` and `order` fields for backwards compatibility. Add conversion functions (`patterns_to_phrases`, `phrases_to_patterns`).

Phase 2: Update the engine to read from the new chain/phrase model. The engine becomes the source of truth; the old `Pattern`/`order` fields are populated on save for legacy compat.

Phase 3: Add Chain and Phrase editing screens to TUI and GUI. The existing pattern editor becomes a "combined phrase view."

Phase 4: Remove the legacy `Pattern`/`order` fields once the migration is stable. Old files auto-convert on load.

#### Interaction with other proposed features

- **Live Mode**: Chains are the natural unit for queuing -- queue a chain per channel, it starts when the current chain finishes.
- **Tables**: Tables attach to instruments and run per-phrase-trigger, orthogonal to the chain/phrase split.
- **Grooves**: Grooves apply per-phrase (each phrase can select its groove).
- **Deep/slim cloning**: Clone a chain = new chain referencing same phrases (slim) or new chain + copies of all phrases (deep).

## 2. Groove System

**Source:** Both LSDj and picoTracker.
**Priority:** High value, moderate effort.
**Status:** Not started.

Both trackers have dedicated Groove screens: arrays of tick counts that redistribute timing across steps. LSDj's default is 6/6 (even), but 7/5 creates swing, 8/4 creates shuffle, etc. picoTracker supports multi-step grooves like 4/8/4 for a 3-step cycle.

rtrack currently has a single `swing` field on Song -- not per-pattern, not selectable per-channel, and not a reusable named groove.

**Proposal:** Add named groove slots (like existing instrument slots) with a `GRV` effect command to switch grooves mid-pattern. Each groove is a variable-length array of tick counts that cycle across rows. Default groove is 6/6 (equivalent to speed=6 with no swing).

**Design notes:**
- Groove 0 is the default for all phrases.
- `GRV xx` command switches to groove xx.
- Grooves can have different lengths (e.g., 3-step groove for triplet feel).
- Swing becomes a special case: groove with two alternating tick values.

## 2. MayBe Command (Probabilistic Notes)

**Source:** LSDj (`B` command).
**Priority:** High value, trivial effort.
**Status:** Not started.

LSDj's `Bxx` command gives each note an xx% probability of playing. In tables, it controls probability of a HOP executing.

**Proposal:** Add a new effect command (e.g., `Bxx`) where xx is the probability (00=never, 80=~50%, FF=always). When the engine processes a note trigger on a row with this effect, it checks `random() < xx/255` and skips the note if false.

**Implementation:** One conditional in `process_tick()` at tick 0, checking a PRNG value against the effect parameter before triggering the note. No new state needed beyond a simple RNG.

## 3. Randomize Command

**Source:** LSDj (`Z` command).
**Priority:** Low effort, unique.
**Status:** Not started.

LSDj's `Z` command randomizes the value of the last-used command within a range. For example, if the previous command was a volume set, `Z` varies the volume randomly each time the row plays.

**Proposal:** Add `Zxx` where xx defines the randomization range applied to the previous effect command's parameter. Store the last effect type per channel; on `Z`, re-execute that effect with its original value +/- random offset within range.

## 4. Live Mode (Pattern Queuing)

**Source:** Both LSDj and picoTracker.
**Priority:** High value, moderate effort.
**Status:** Not started.

Both trackers have a Live Mode where you queue chains/patterns per-channel and they start when the current one finishes. LSDj: press START to queue a chain, press twice for immediate switch at phrase boundary. picoTracker: queued items blink, ALT+PLAY queues entire rows.

rtrack has no live performance mode -- playback is strictly linear through the order list.

**Proposal:** Add a Live Mode toggle to the pattern matrix screen. In Live Mode:
- Cursor selects which pattern to queue per channel.
- A keypress queues the pattern; it starts when the current pattern finishes.
- Double-press for immediate switch at next row boundary.
- Queued patterns show a visual indicator (blinking or highlight).
- Complements existing Ableton Link support for tempo/transport sync.

## 5. Tables (Per-Instrument Automation)

**Source:** Both LSDj and picoTracker.
**Priority:** Transformative, significant effort.
**Status:** Not started.

Tables are short looping command sequences (16 rows, 2-3 effect columns each) that run independently per-instrument on every note trigger. LSDj tables have envelope, transpose, and command columns. picoTracker tables have 3 command pairs per row.

Tables enable arpeggios, tremolo, filter sweeps, retriggering, and custom envelopes without cluttering the pattern data. rtrack's current approach requires writing effects directly into pattern rows, which is verbose and limits expressiveness.

**Proposal:** Add a new `Table` data type:
- 16 rows, each with: envelope value, transpose, and 2 effect command pairs.
- Tables are numbered slots (like instruments), referenced from instruments via a `table` field.
- On note trigger, the table starts at row 0 and advances one row per tick (speed controllable).
- `HOP` command within tables creates loops (jump to row N, repeat M times).
- `STP` command stops table execution (one-shot envelopes).
- Instrument setting: `TABLE` mode = TICK (free-running) or STEP (advance one row per note trigger for evolving sequences across multiple notes).
- `TBL xx` effect command in patterns can also trigger a table.

**Design notes:**
- Tables run on the engine's tick clock, independent of pattern rows.
- Each active channel has its own table playback state (current row, hop count).
- Tables share the same effect command vocabulary as patterns.

## 6. Scale Quantization

**Source:** picoTracker (44 built-in scales).
**Priority:** Low effort, high usability.
**Status:** Not started.

picoTracker has a project-wide scale selection (Dorian, Hirajoshi, Persian, etc.) that constrains note input to scale-valid pitches.

**Proposal:** Add an optional scale constraint for note input in Insert mode. When a scale is active, the piano keyboard mapping snaps to the nearest scale degree. Scales are defined as bit masks over 12 semitones.

**Implementation:**
- Add a `scale: Option<Scale>` field to Song or as a TUI/GUI state setting.
- Define common scales as constants (Major, Minor, Dorian, Pentatonic, Blues, etc.).
- In the piano keyboard input handler, quantize the entered note to the nearest scale degree.
- Scale selection via song settings dialog or a command.

## 7. (Moved to #1 -- Song > Chain > Phrase Architecture)

## 8. Retrigger with Transpose (IRT)

**Source:** picoTracker.
**Priority:** Low effort.
**Status:** Not started.

picoTracker's `IRT` command retriggers the current note from within a table with cumulative semitone transposition, without resetting instrument state (filter, table position, etc.). This enables "dub echo" effects and non-4/4 rhythmic patterns at tick resolution.

**Proposal:** Add a retrigger variant that transposes cumulatively. Could extend the existing retrigger effect or be a new command for use in tables (once tables are implemented).

## 9. Stem Export

**Source:** picoTracker.
**Priority:** Low effort (builds on existing export).
**Status:** Not started.

picoTracker renders separate WAV files per channel in a single pass. rtrack already has `render_to_wav` for mixdown. Per-channel stem export would be a small extension: run the render loop but write each channel's output to a separate file instead of summing to stereo.

## 10. Deep/Slim Cloning

**Source:** LSDj.
**Priority:** Low effort, UX improvement.
**Status:** Not started.

LSDj distinguishes between deep-cloning (copies the chain AND all its phrases) and slim-cloning (new chain referencing the same phrases). When duplicating patterns in rtrack's order list, offering both options (duplicate pattern data vs. reference same pattern) would reduce accidental edits to shared patterns.

## Priority Summary

| # | Feature | Effort | Impact | Depends On |
|---|---|---|---|---|
| 1 | Song/Chain/Phrase | High | Foundational | -- |
| 2 | Grooves | Moderate | High | -- |
| 3 | MayBe command | Trivial | Medium | -- |
| 4 | Randomize command | Trivial | Medium | -- |
| 5 | Live Mode | Moderate | High | Chain/Phrase model |
| 6 | Tables | Significant | Transformative | -- |
| 7 | Scale quantization | Low | Medium | -- |
| 8 | Retrigger w/ transpose | Low | Low | Tables (ideally) |
| 9 | Stem export | Low | Medium | -- |
| 10 | Deep/slim cloning | Low | Low | Chain/Phrase model |
