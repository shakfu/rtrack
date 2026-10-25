//! A standard MIDI file, imported as a song.
#![no_main]

use libfuzzer_sys::fuzz_target;

mod common;

fuzz_target!(|data: &[u8]| {
    let path = common::probe(data, "mid");
    if let Ok((mut song, _)) = rtrack_core::midi_file::import_midi_with_report(&path) {
        let _ = song.repair();
    }
});
