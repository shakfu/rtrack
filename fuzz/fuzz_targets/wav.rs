//! A sample file, loaded into a bank slot.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rtrack_core::sample::SampleBank;

mod common;

fuzz_target!(|data: &[u8]| {
    let path = common::probe(data, "wav");
    let _ = SampleBank::new().load(0, &path);
});
