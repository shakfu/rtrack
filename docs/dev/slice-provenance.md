# Slicing a trimmed sample

Makes slicing divide the part of a sample the user trimmed to, while re-slicing at a new count still re-derives from the same audio. Closes the TODO.md Low item "Decide whether slicing should respect a trim the user set by hand". Code refers to `rtrack-core/src` as of 2026-10-25.

Status: implemented 2026-10-25, with the proposal for both decisions. Added while implementing: a slot from an older file with no `sliced_from` whose name ends in `_Snn` loads with `sliced_from` set to the whole file, so its re-slicing behaves as before; without this, every slice saved by 0.2.0 would re-slice inside itself.

## Current state

- A `Sample` holds `trim_start`/`trim_end` (its span), and `data`, shared between slices of one source (`sample/mod.rs`).
- `SliceRange::Source` divides the whole buffer and ignores the span. It exists so the slice-count control can re-apply: after slicing, slot 0's span is the first slice, and dividing that span again would carve up the first slice each time the count moves.
- `SliceRange::Span` divides the slot's span: right for subdividing one slice, and the workaround for a hand-trimmed sample.
- The GUI's count control and the TUI's slice keys use `Source` by default.
- A slot does not record whether its span came from slicing or from the user. `SampleRef` saves the span and nothing else (`tracker/song.rs`).

So a user who trims a loop to bars 2-3 and slices it into 8 gets 8 slices of the whole file.

## Options

**A. Record what slicing divided.** Slicing sets `Sample.sliced_from: Option<(usize, usize)>`, the span it divided, on every slot it writes. `Source` then divides `sliced_from` if set, else the slot's current span.
- First slice of a trimmed sample: no `sliced_from`, so the trim is divided.
- Re-slice at a new count: every slice carries the same `sliced_from`, so the same audio is divided again, as `Source` does today.
- An untrimmed, unsliced sample divides the whole file, as now.
- One optional field in `SampleRef`; rtrack 0.2.0 ignores it.

**B. A sliced flag.** `Sample.sliced: bool`. `Source` divides the span when unsliced, the whole file when sliced.
- Smaller.
- A re-slice of a trimmed sample's slices divides the whole file, which loses the trim on the second pass.

**C. Always the span.** Make `Source` divide the span.
- No new data.
- Brings back the bug `Source` exists to prevent: re-slicing carves the first slice.

**D. Status quo.** `Span` stays the workaround.

| | A | B | C | D |
|-|-|-|-|-|
| First slice respects a hand trim | yes | yes | yes | no (use `Span`) |
| Re-slice at a new count keeps the same audio | yes | whole file | no | yes |
| New saved data | one optional span | one flag | none | none |

## Recommendation: A

A gives both readings the TODO names from one recorded fact, and its saved form is one optional pair that older builds ignore. B fails on the second pass of a trimmed sample; C reintroduces a fixed bug.

## Model (A)

```rust
pub struct Sample {
    // ...
    /// The span slicing divided to make this slot, if slicing made it.
    /// `SliceRange::Source` re-divides it, so a new count cuts the same audio.
    pub sliced_from: Option<(usize, usize)>,
}

pub struct SampleRef {
    // ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sliced_from: Option<(usize, usize)>,
}
```

- **Slicing.** `Source` divides `sliced_from.unwrap_or(span)`; every slot written gets that span as `sliced_from`. `Span` divides the slot's span and sets `sliced_from` to it, so a subdivided slice re-slices inside itself.
- **Editing a span.** Moving a slice's trim by hand keeps `sliced_from`: the slice is still one of that slicing. Loading a new sample into the slot clears it.
- **Repair.** A `sliced_from` outside the data, or with start not before end, is dropped on load with a repair note.
- **Format.** An optional field: no `FORMAT_VERSION` change. 0.2.0 ignores it, slices as before, and drops it if it saves the song.
- **Editors.** No new control. The GUI and TUI already offer `Source` and `Span`; the TODO's wording for them changes in README.

## Decisions

1. **A, B, C or D.** Proposal: A.
2. **A hand-moved slice boundary.** Proposal: keeps `sliced_from` (above). Alternative: clears it, so the moved slice becomes a hand trim and the next `Source` divides only it.

## Plan

1. `sliced_from` on `Sample` and `SampleRef`, saved, loaded and repaired; slicing sets and reads it. Tests: a trimmed sample slices within its trim; re-slicing at another count gives the same outer bounds; an untrimmed sample is unchanged; a file without the field loads as today; a bad span is repaired.
2. README sample section, CHANGELOG, TODO.
