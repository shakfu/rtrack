//! Malformed and adversarial input for every parser that reads a file.
//!
//! rtrack opens four formats it did not write: `.rtrk`, `.mid`, `.wav`, and
//! `.aiff`. All four are reached by double-clicking something, and none of
//! them can assume the bytes are well-formed -- a truncated download and a
//! file built to break the parser look the same from inside.
//!
//! The rule these tests enforce is the weak one, deliberately: **reject or
//! bound, but never panic, and never size an allocation from a number the
//! file chose.** Nothing here asserts that a broken file produces good audio.
//! A parser is free to refuse anything it likes; what it may not do is
//! unwind through a caller that has no way to recover, or ask the allocator
//! for a petabyte because two `u32`s in a header multiplied.
//!
//! That last one is not hypothetical and is the reason for the sizing rule:
//! an AIFF declaring `u32::MAX` frames of 65535 channels reserved 1.1PB and
//! aborted the process -- past the point where a panic could have been caught
//! and turned into "could not open that file".
//!
//! These are hand-picked shapes plus a systematic truncation sweep, not a
//! fuzzer. A fuzzer would be better and is on the TODO; it needs a nightly
//! toolchain and `cargo-fuzz`, so it cannot run here or in CI. Every case
//! below was a real defect or is the boundary next to one.

use std::path::Path;

use rtrack_core::sample::SampleBank;
use rtrack_core::tracker::{Song, SongFile};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load bytes as a sample of the given extension. Returns whether it was
/// accepted. A panic here fails the test, which is the whole point.
fn load_sample(dir: &Path, name: &str, bytes: &[u8]) -> bool {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write probe file");
    let mut bank = SampleBank::new();
    bank.load(0, &path).is_ok()
}

fn aiff(chunks: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"FORM");
    v.extend_from_slice(&((chunks.len() + 4) as u32).to_be_bytes());
    v.extend_from_slice(b"AIFF");
    v.extend_from_slice(chunks);
    v
}

/// A chunk whose declared size need not match the body that follows it --
/// which is the point of most of these cases.
fn chunk(id: &[u8; 4], declared_size: u32, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(id);
    v.extend_from_slice(&declared_size.to_be_bytes());
    v.extend_from_slice(body);
    v
}

fn comm_body(channels: u16, frames: u32, bits: u16) -> Vec<u8> {
    let mut comm = vec![0u8; 18];
    comm[0..2].copy_from_slice(&channels.to_be_bytes());
    comm[2..6].copy_from_slice(&frames.to_be_bytes());
    comm[6..8].copy_from_slice(&bits.to_be_bytes());
    // Leave the 80-bit extended sample rate zeroed; it is not what is under
    // test and a zero rate is itself worth not crashing on.
    comm
}

// ---------------------------------------------------------------------------
// AIFF -- hand-rolled parser, so every field is ours to get wrong
// ---------------------------------------------------------------------------

#[test]
fn aiff_comm_chunk_shorter_than_its_own_fields_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    // The parser reads 18 bytes of fields out of COMM. A chunk declaring 4
    // was read into a 4-byte buffer and then indexed at [6], [7], and [8..18]
    // anyway.
    let bytes = aiff(&chunk(b"COMM", 4, &[0, 1, 0, 0]));
    assert!(
        !load_sample(dir.path(), "short_comm.aiff", &bytes),
        "a COMM chunk too short to hold its fields must be refused"
    );
}

#[test]
fn aiff_ssnd_chunk_smaller_than_its_header_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    // SSND's declared size includes the 8-byte offset/block_size pair. A
    // chunk claiming fewer than 8 wrapped `chunk_size - 8` round to near
    // usize::MAX, which went straight into `Vec::resize`.
    let mut body = chunk(b"COMM", 18, &comm_body(1, 10, 16));
    body.extend_from_slice(&chunk(b"SSND", 2, &[0u8; 8]));
    let bytes = aiff(&body);
    assert!(
        !load_sample(dir.path(), "short_ssnd.aiff", &bytes),
        "an SSND chunk smaller than its own header must be refused"
    );
}

#[test]
fn aiff_frames_times_channels_is_not_an_allocation_size() {
    let dir = tempfile::tempdir().expect("temp dir");
    // u32::MAX frames x 65535 channels x 2 bytes reserved 1.1PB and aborted
    // the process. The file carries 16 bytes of audio; that is the bound.
    let mut body = chunk(b"COMM", 18, &comm_body(65535, u32::MAX, 16));
    body.extend_from_slice(&chunk(b"SSND", 16, &[0u8; 16]));
    let bytes = aiff(&body);

    // Either answer is fine. Surviving the call is the assertion.
    let _ = load_sample(dir.path(), "huge_comm.aiff", &bytes);
}

#[test]
fn aiff_chunk_claiming_more_than_the_whole_file_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut v = b"FORM\x00\x00\x00\x64AIFF".to_vec();
    v.extend_from_slice(b"COMM");
    v.extend_from_slice(&u32::MAX.to_be_bytes());
    v.extend_from_slice(&[0u8; 18]);
    assert!(
        !load_sample(dir.path(), "lying_chunk.aiff", &v),
        "a chunk larger than the file it lives in must be refused"
    );
}

#[test]
fn aiff_degenerate_headers_are_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, bytes) in [
        ("empty.aiff", Vec::new()),
        ("form_only.aiff", b"FORM\x00\x00\x00\x04AIFF".to_vec()),
        ("not_aiff.aiff", b"FORM\x00\x00\x00\x04XXXX".to_vec()),
        ("truncated.aiff", b"FOR".to_vec()),
        // Zero bits per sample: `bits / 8` is the stride the sample loop
        // divides and indexes by.
        (
            "zero_bits.aiff",
            aiff(&chunk(b"COMM", 18, &comm_body(1, 4, 0))),
        ),
        (
            "zero_channels.aiff",
            aiff(&chunk(b"COMM", 18, &comm_body(0, 4, 16))),
        ),
    ] {
        assert!(
            !load_sample(dir.path(), name, &bytes),
            "{name} should have been refused"
        );
    }
}

// ---------------------------------------------------------------------------
// WAV -- decoded by `hound`, so this pins that it stays strict
// ---------------------------------------------------------------------------

#[test]
fn wav_headers_that_lie_are_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");

    let build = |channels: u16, bits: u16, declared_data: u32| {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&44100u32.to_le_bytes());
        fmt.extend_from_slice(&0u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());

        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        v.extend_from_slice(&fmt);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&declared_data.to_le_bytes());
        v.extend_from_slice(&[0u8; 16]);
        v
    };

    for (name, bytes) in [
        ("many_channels.wav", build(65535, 16, u32::MAX)),
        ("zero_channels.wav", build(0, 16, 16)),
        ("zero_bits.wav", build(1, 0, 16)),
        ("empty.wav", Vec::new()),
        ("garbage.wav", b"RIFF\x00\x00\x00\x00WAVEjunk".to_vec()),
    ] {
        assert!(
            !load_sample(dir.path(), name, &bytes),
            "{name} should have been refused"
        );
    }
}

// ---------------------------------------------------------------------------
// Truncation sweep -- the systematic half
// ---------------------------------------------------------------------------

/// Every prefix of a valid file, fed back to the parser that produced it.
///
/// This is the cheapest stand-in for a fuzzer and it covers a real case: a
/// download or a copy interrupted partway leaves exactly these bytes. A
/// parser that reads a length from byte 40 and a body from byte 44 has to
/// cope with the file ending at byte 42.
fn every_prefix(bytes: &[u8], step: usize, mut load: impl FnMut(&[u8])) {
    let mut end = 0;
    while end <= bytes.len() {
        load(&bytes[..end]);
        end += step;
    }
    load(bytes);
}

#[test]
fn every_truncation_of_a_valid_midi_file_is_survivable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("full.mid");

    let mut song = Song::new(2, 16);
    song.set_cell(
        0,
        0,
        0,
        rtrack_core::tracker::Cell {
            note: Some(rtrack_core::tracker::Note::On {
                value: rtrack_core::tracker::NoteValue::C,
                octave: 4,
            }),
            ..Default::default()
        },
    );
    rtrack_core::midi_file::export_midi(&song, &src).expect("export");
    let full = std::fs::read(&src).expect("read back");

    let probe = dir.path().join("probe.mid");
    every_prefix(&full, 1, |prefix| {
        std::fs::write(&probe, prefix).expect("write prefix");
        // Ok or Err both fine; not panicking is the assertion.
        let _ = rtrack_core::midi_file::import_midi(&probe);
    });
}

#[test]
fn every_truncation_of_a_valid_song_file_is_survivable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("full.rtrk");
    SongFile::from_song(Song::new(4, 32))
        .save(&src)
        .expect("save");
    let full = std::fs::read(&src).expect("read back");

    let probe = dir.path().join("probe.rtrk");
    // JSON, so this is a few KB; step through it rather than byte by byte.
    every_prefix(&full, 7, |prefix| {
        std::fs::write(&probe, prefix).expect("write prefix");
        if let Ok(mut sf) = SongFile::load(&probe) {
            // A prefix that happens to parse must still be safe to repair
            // and then read from, which is what the app does next.
            sf.song.repair();
            let _ = sf.song.cell_at(0, 0, 0);
        }
    });
}

#[test]
fn every_truncation_of_a_valid_aiff_is_survivable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut body = chunk(b"COMM", 18, &comm_body(2, 64, 16));
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(&vec![0u8; 64 * 2 * 2]);
    body.extend_from_slice(&chunk(b"SSND", ssnd.len() as u32, &ssnd));
    let full = aiff(&body);

    every_prefix(&full, 1, |prefix| {
        load_sample(dir.path(), "probe.aiff", prefix);
    });
}

// ---------------------------------------------------------------------------
// Byte-flip sweep -- structural corruption rather than truncation
// ---------------------------------------------------------------------------

/// Set one byte of a valid file to an extreme value and reparse.
///
/// Length and count fields are what this is hunting: flipping a byte inside
/// one is how a small file comes to declare a huge structure, which is the
/// shape of every allocation bug found in this codebase so far.
#[test]
fn single_byte_corruption_of_an_aiff_is_survivable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut body = chunk(b"COMM", 18, &comm_body(2, 64, 16));
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(&vec![0u8; 64 * 2 * 2]);
    body.extend_from_slice(&chunk(b"SSND", ssnd.len() as u32, &ssnd));
    let full = aiff(&body);

    // The headers are where the length fields live; the audio body past them
    // is uninteresting and long.
    let header_bytes = full.len().min(64);
    for i in 0..header_bytes {
        for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
            let mut corrupted = full.clone();
            corrupted[i] = value;
            load_sample(dir.path(), "probe.aiff", &corrupted);
        }
    }
}

#[test]
fn single_byte_corruption_of_a_midi_file_is_survivable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("full.mid");
    rtrack_core::midi_file::export_midi(&Song::new(1, 16), &src).expect("export");
    let full = std::fs::read(&src).expect("read back");

    let probe = dir.path().join("probe.mid");
    for i in 0..full.len().min(64) {
        for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
            let mut corrupted = full.clone();
            corrupted[i] = value;
            std::fs::write(&probe, &corrupted).expect("write");
            let _ = rtrack_core::midi_file::import_midi(&probe);
        }
    }
}
