# Music theory crates: evaluation, 2026-09-12

Question: should rtrack depend on a music theory library for scale
quantization, and later for chord work?

Decision: no. `rtrack-core/src/theory.rs` holds the scale masks instead.

## The two candidates

| | tonal_rs 0.1.0 | rust-music-theory 0.4.0 |
|-|-|-|
| Releases | 1, 2026-08-20 | 11, 2020-01-25 to 2026-07-12 |
| Downloads | 19 | 21,077 total, 938 recent |
| GitHub stars | 2 | 692 |
| License | MIT | MIT |
| Scope | ~20 modules, incl. chord_detect, pcset, voicing, roman_numeral, progression | 4 modules: note, interval, chord, scale |
| Deps | regex, rand (optional) | strum 0.17, strum_macros 0.17, regex, clap 2.31, lazy_static, serde, serde-wasm-bindgen 0.4, wasm-bindgen 0.2, web-sys 0.3, all non-optional; midly/midir 0.9 optional |
| Pitch repr | note strings, plus an i32 MIDI path | Pitch enum + octave: i16 |
| MIDI ints | to_midi, midi_to_note_name, pcset_nearest | Note::midi_pitch() -> u8, no documented inverse |

Sources: <https://crates.io/api/v1/crates/tonal_rs>,
<https://crates.io/api/v1/crates/rust-music-theory>,
<https://raw.githubusercontent.com/ozankasikci/rust-music-theory/master/Cargo.toml>,
<https://docs.rs/tonal_rs/latest/tonal_rs/>,
<https://docs.rs/rust-music-theory/latest/rust_music_theory/>.

## Why neither

1. rust-music-theory pulls `wasm-bindgen`, `web-sys`, `serde-wasm-bindgen` and
   `clap` 2.31 unconditionally -- not target-gated, not feature-gated. A native
   audio app would carry browser bindings and an unmaintained CLI arg parser in
   every build. Its optional `midir` is 0.9 against rtrack's 0.10.

2. rust-music-theory converts `Note -> u8` only. A tracker holds the integer
   and needs the other direction, to name it or constrain it.

3. tonal_rs fits the types -- `midi::pcset_nearest(&[0,5,7][..])` returns
   `impl Fn(i32) -> Option<i32>`, which is the quantization primitive -- but is
   one release old with 19 downloads and a repo created eight days before
   publication. No external validation of the port against the JS original.

4. The feature is smaller than either dependency. A scale is a `u16` mask and
   the snap is a loop over 12 candidates. The scale table is data entry.

## If chord work happens

`TODO.md` lists a chord note type. Its open problems are cell display, `.rtrk`
storage, MIDI export, and single-voice channel state -- not chord spelling, so
neither crate addresses them. If chord *detection* or roman-numeral entry is
ever wanted, tonal_rs is the only candidate of the two with the vocabulary, and
MIT permits vendoring its tables rather than depending on a 0.1.0.
