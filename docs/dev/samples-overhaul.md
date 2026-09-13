# Sample slicing and trimming: gaps and improvements, 2026-09-13

Question: how far is rtrack's sample slicing and trimming from a capable
tracker sampler, and what should change first?

Basis: code read at `76baee0`. Measurements used a scratch harness against
`rtrack-core` (release build unless stated). Claims marked *inferred* come from
reading code without a test. Behaviour of other trackers and samplers is from
memory, not checked against their manuals.

## Current state

| Area | What exists | Where |
|-|-|-|
| Model | A slice is a shared `Arc<[[f32; 2]]>` buffer plus a `trim_start..trim_end` span. `.rtrk` stores path + span | `sample/mod.rs:53` |
| Equal slicing | N segments; last takes the remainder. GUI count 2..=64 | `sample/mod.rs:619`, `visualization.rs:368` |
| Transient slicing | Log-RMS energy rise, local-mean threshold, peak picking, 50 ms minimum gap, onset moved back to the quietest frame within one ~5 ms window | `sample/mod.rs:683` |
| Range | `SliceRange::Source` (whole buffer) or `Span` (subdivide one slice) | `sample/mod.rs:19` |
| Slot placement | Consecutive slots from the target, max 256 (`MAX_INSTRUMENTS`). Refuses to overwrite unrelated instruments unless confirmed | `core.rs:1485` |
| Undo | Both frontends snapshot the sample bank around a slice; trim/loop/base-note edits are undoable | `instrument_editor.rs:829`, `input.rs:342` |
| Playback | 4-point cubic Hermite; fixed envelope 2 ms attack / 50 ms release; 5 ms tail fade scaled by rate; loop wrap by modulo | `playback.rs:85`, `envelope.rs:41` |
| Formats | WAV (8/16/24/32-bit int, float), uncompressed AIFF; 512 MB cap | `sample/mod.rs:195` |

## Gaps

### 1. Bugs

**1a. Re-slicing with a smaller count leaves the old slices behind.**
`slice_sample` writes only `slot..slot + count` and never clears what the
previous division wrote past that. Verified: slicing a 44100-frame sample into
16, then 8, leaves slots 8-15 holding the 16-way slices (`amen_S08` at
22048..24804). Those overlap the new `amen_S04`..`amen_S07`. The GUI tooltip for
*Whole sample* says it replaces the previous slices. The GUI applies on every
count change, so dragging the count down produces this state.

**1b. Loop markers use the raw loop points.** Playback clamps loop points into
the trim span (`Sample::effective_loop_start`). Two waveform views draw the
stored value instead:

- `rtrack-gui/src/instrument_editor.rs:635-636`
- `rtrack-tui/src/tui/sample_editor.rs:482`

A slice with looping enabled and the default `loop_start = 0` shows its loop
starting at frame 0 of the whole file. `loop_end = 0` draws at the left edge.
`visualization.rs:727-728` already uses the effective values.

**1c. Stale comment.** `rtrack-gui/src/visualization.rs:450-451` says slicing
"cannot be undone". It can.

### 2. Playback quality

**2a. Aliasing when pitched up.** The interpolator has no low-pass filter.
Measured at 44.1 kHz output:

| Source | Transpose | Plays at | Heard at | Alias level |
|-|-|-|-|-|
| 15 kHz sine | +12 st | 30.0 kHz | 14.1 kHz | 0.0 dB re source |
| 12 kHz sine | +12 st | 24.0 kHz | 20.1 kHz | 0.0 dB |
| 8 kHz sine | +19 st | 24.0 kHz | 20.1 kHz | -0.2 dB |

At +12 st, every source component above 11 kHz folds back at full level.
Hi-hats and cymbals in a pitched-up break are in that range.

**2b. Loop seam discontinuity.** There is no crossfade, and the interpolator's
neighbour frames are not wrapped at `loop_end` (`playback.rs:124-127`).
Measured: a 440 Hz sine looped over 5000 frames (49.9 periods) has a
sample-to-sample step of 0.326 at the seam, against 0.031 elsewhere (10.4x).
The user must find matching loop points by hand, and the frame fields make that
hard.

**2c. Fixed sample envelope.** Every sample voice uses
`Envelope::sample_default` (2 ms / 0 / 1.0 / 50 ms). No per-instrument attack
or release, so a slice cannot be gated short or given a soft start.

**2d. Neighbour bleed.** Interpolation reads 1 frame before `trim_start` and 2
after the end, from the adjacent audio. *Inferred* inaudible: the 2 ms attack
and 5 ms tail fade cover both edges. Listed for completeness; no action.

### 3. Transient detection

**3a. Broadband only.** Detection sums L+R and measures RMS over the whole band.
The code comment calls it "spectral-flux-style"; it is energy flux. *Inferred:*
a hit that adds little total energy (a hi-hat over a sustained bass) goes
undetected.

**3b. Fixed 50 ms gap.** Onsets closer than 50 ms merge. At 170 BPM a 1/32 note
is 55 ms; flams and rolls are shorter.

**3c. Preview recomputes on every repaint.** The GUI calls
`detect_transients_range` inside the draw code while transient mode is shown
(`visualization.rs:435`). The app requests continuous repaints while audio is
active (`app.rs:349`). Measured cost per call:

| Buffer | Release | Unoptimised (opt-level 0) |
|-|-|-|
| 5 s stereo | 0.40 ms | 4.2 ms |
| 60 s stereo | 3.67 ms | 51 ms |

The workspace dev profile uses opt-level 1, so a dev build falls between the
two columns. At 60 fps (16.7 ms frame), a 60 s sample costs 22% of each frame in
release. The TUI does the same in `compute_slice_preview`
(`sample_editor.rs:364`) on each draw.

### 4. Editing

- **No manual slice points.** Count and sensitivity are the only controls. A
  single boundary cannot be added, moved or removed.
- **Trim and loop are numeric frame fields.** GUI: `DragValue`s. TUI: +/-100 or
  +/-1000 frames per key (`input.rs:284-293`). A 3-minute file at 44.1 kHz is
  7.9 M frames, about 7,900 presses at the large step. Neither waveform view
  accepts clicks or drags for trim or loop.
- **No zero-crossing snap** for trim, loop or equal-slice boundaries.
- **No tempo awareness.** Equal slicing matches a beat grid only when the trim
  is already an exact number of bars. Nothing sets that trim from the song BPM.
- **Trim provenance.** `Source` ignores a hand-set trim. Already recorded in
  `TODO.md`.
- **Loading over an occupied slot is not undoable.** Already recorded in
  `TODO.md`.

### 5. Missing features

| Feature | Reference | Notes |
|-|-|-|
| Sample offset effect (`9xx`) | ProTracker `9xx`, Renoise `0Sxx` | Plays part of a sample without slicing. Commands 0-8 and B-F are taken; 9 and A are free (`constants.rs:48-61`) |
| Reverse playback | Renoise, Ableton Simpler | Per note or per sample |
| Key-mapped slice instrument | Renoise, Ableton Simpler, MPC | One instrument, slices across keys. rtrack spends one slot per slice; `Instrument` holds one `sample_index` (`types.rs:319`) |
| Per-slice gain, pitch, envelope | Renoise, MPC | Follows from 2c |
| Normalize, destructive crop | Most sample editors | Conflicts with the shared-buffer model; see below |
| Time-stretch | Renoise, Ableton | Large; external crate or DSP work |
| FLAC, MP3, OGG | Renoise | Needs a decoder dependency |

## Improvements

Each entry gives the options and the trade-off.

### Re-slice leftovers (1a)

- **Clear the trailing run.** After writing, clear consecutive slots past the
  new end that hold samples from the same source and carry `_Sxx` names. Inside
  the existing undo snapshot, so a mistaken clear can be undone.
- **Refuse instead.** Report the leftover slots and do nothing. Safer, but the
  GUI re-applies on every count change, so it would refuse on every drag down.

Clear is better. The GUI's apply-on-change design assumes replacement.

### Preview cost (3c)

- **Memoise on the inputs.** Recompute only when (slot, span, sensitivity,
  range) changes. Few lines per frontend.
- **Split the envelope from the threshold.** Compute the dB envelope once per
  span; rerun only thresholding on a sensitivity change. Faster during a
  sensitivity drag, but changes the core API.

Memoise first. Split only if a sensitivity drag on a long file still lags.

### Loop seam (2b)

- **Wrapped neighbours.** Read interpolation frames modulo the loop. Removes
  interpolation error at the seam only. Small.
- **Loop crossfade.** Blend the last N ms before `loop_end` with the audio
  before `loop_start`. Removes the step for arbitrary loop points. Needs a
  crossfade-length field and a second read position per voice.
- **Zero-crossing snap.** Edit-time only. Cheap. Matches value, not slope, so a
  seam can still click.

Wrapped neighbours plus snap cover most cases cheaply. A crossfade is the
complete fix.

### Aliasing (2a)

- **Band-limited mip levels.** Precompute low-passed, half-rate copies of each
  source buffer. A voice at rate r reads level ceil(log2 r). Runtime cost stays
  near today's. Memory up to 2x per source. Slice spans map to each level by
  shifting.
- **Windowed-sinc interpolation** with cutoff scaled by rate. Best quality.
  About 8-16 taps per voice per frame, against 4 today, across up to 32 voices.
- **Leave it.** Aliasing is a known character of classic trackers. Acceptable
  as a documented choice, not as an accident.

Mip levels, if pitched-up drums matter. Otherwise document it.

### Sample offset `9xx` (5)

Two semantics exist:

- **Absolute:** `xx * 256` frames (ProTracker). Short range at 44.1 kHz:
  `FF` is 1.5 s.
- **Relative:** `xx / 256` of the played span (Renoise-style). Scales with slice
  length.

Relative fits rtrack: slices vary in length, and the span is already known per
voice.

### Manual slice points (4)

- **Edit adjacent spans.** Dragging a boundary moves `trim_end` of slot i and
  `trim_start` of slot i+1 together. No new state; fits the slot model.
- **Explicit slice list on the source.** A per-source `Vec<usize>` that slots
  derive from. Needed for a key-mapped instrument. Changes the `.rtrk` format.

Adjacent spans first. The slice list belongs with the key-mapped instrument, if
that happens.

### Shared buffer versus destructive edits (5)

Normalize, reverse-in-place and crop rewrite frames. Every slice sharing the
`Arc` would change, and `.rtrk` would no longer describe the audio, since it
stores a path, not frames. Options:

- **Non-destructive parameters.** Gain, reverse flag, per sample. Saved in
  `.rtrk`. Recommended.
- **Write a new file.** Render the edit to disk, then point the slot at it.
  Leaves files beside the song that the user must keep.

## Recommendations

In order. Size is a rough guess.

1. **Fix 1a, 1b, 1c.** Correctness, small. 1a produces overlapping slices
   through normal GUI use. Done 2026-09-13; see CHANGELOG, Unreleased > Fixed.
2. **Memoise the transient preview (3c).** Small. Removes up to 22% of a frame
   on long samples. Done 2026-09-13 (`SlicePlanCache`).
3. **Add `9xx` with relative semantics.** Small-medium: effect parse, engine
   dispatch, `position` set at note-on. Largest capability gain per line of
   code. Done 2026-09-13.
4. **Wrapped interpolation at loops plus zero-crossing snap (2b).** Small.
   Done 2026-09-13. The 440 Hz seam from 2b drops from 10.4x to 1.4x the
   99th-percentile step after snapping.
5. **Clickable and draggable trim, loop and slice boundaries in the GUI (4).**
   Medium. Uses the adjacent-span model. TUI keeps numeric fields, with a
   snap-to-zero-crossing key. Done 2026-09-13, in the Samples tab only
   (`SampleBank::move_boundary`).
6. **Per-instrument sample envelope, gain and reverse flag (2c, 5).** Medium.
   `.rtrk` format addition, backward compatible if the fields default.
7. **Decide on aliasing (2a).** Mip levels or a documented "no". Medium if
   built.

Deferred:

- **Key-mapped slice instrument.** Large. Changes `Instrument`, the file format
  and both editors. Revisit after 5, which forces the slice-boundary model.
- **Spectral onset detection (3a).** Build only when a failing test sample
  exists.
- **Time-stretch, extra formats.** Each adds a dependency. No current demand.

## Alternative framing

The list above treats rtrack as a sampler. If rtrack is a tracker that plays
slices prepared elsewhere, items 5 and 7 drop out. The work then narrows to 1-4
and 6. Deciding which rtrack is settles the order of everything after item 4.

## Reproducing the measurements

The harness lived in the scratchpad and is not committed. To rerun, build a
binary against `rtrack-core` that:

- renders a 2 s sine through `SamplePlaybackEngine::note_on` and measures the
  alias bin with a Hann-windowed single-bin DFT;
- renders a looped 440 Hz sine with `loop_start = 1000`, `loop_end = 6000` and
  compares the largest step at the seam with the largest elsewhere;
- times `detect_transients_range` on 5 s and 60 s of decaying noise bursts at
  4 Hz;
- calls `TrackerCore::slice_sample` with count 16, then 8, on a headless core
  and inspects slots 8-15.
