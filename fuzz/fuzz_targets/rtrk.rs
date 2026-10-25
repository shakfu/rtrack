//! A song file: parse, repair and convert instruments, as `TrackerCore::load_file`
//! does, without loading the samples it names, which could be any local file.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rtrack_core::tracker::SongFile;

mod common;

fuzz_target!(|data: &[u8]| {
    let path = common::probe(data, "rtrk");
    if let Ok((mut file, _)) = SongFile::load_with_notes(&path) {
        let _ = file.song.repair();
        for entry in &file.instruments {
            let _ = entry.def.to_instrument(entry.slot);
        }
    }
});
