pub mod export;
pub mod playback;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

/// Which part of a sample a slicing operation divides.
///
/// A slice is a span of a shared buffer, so "slice this slot into N" is two
/// different requests depending on what the span means. Re-running a slice
/// action with a new count should re-derive from the sample; deliberately
/// subdividing a slice should stay inside it. Conflating them means a second
/// pass at a different count silently discards everything outside the first
/// slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceRange {
    /// The whole buffer, ignoring the slot's current span. Applying this
    /// repeatedly at different counts always divides the same audio, which
    /// is what a slice-count control needs.
    ///
    /// Note that this also ignores a trim the user set by hand on an
    /// unsliced sample; see `Span` for the other reading.
    Source,
    /// The slot's own span, so subdividing a slice nests inside it.
    Span,
}

/// Whether a slice action may write over instruments that are not its own.
///
/// Slicing writes into consecutive slots from its target, which is what makes
/// a sliced break playable by walking instrument numbers up a pattern. That
/// makes it destructive, and it is not undoable, so it asks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceOverwrite {
    /// Stop and report if anything unrelated is in the way. The default.
    Refuse,
    /// Go ahead -- the user has seen what would be overwritten and said yes.
    Allow,
}

/// A loaded audio sample stored as stereo f32 frames.
///
/// A sample is a *view* of a frame buffer: `data` holds the frames and
/// `trim_start`/`trim_end` bound the part that plays. Slicing produces
/// several samples that share one buffer and differ only in their bounds,
/// which is both how the `.rtrk` format stores them -- a source path plus a
/// span -- and why slicing does not multiply memory by the slice count.
#[derive(Clone)]
pub struct Sample {
    pub name: String,
    /// Stereo frames: [left, right] pairs. Shared between slices of the same
    /// source, and never mutated after loading.
    pub data: Arc<[[f32; 2]]>,
    pub sample_rate: f64,
    /// MIDI note at which the sample plays at original pitch (default 60 = C5)
    pub base_note: u8,
    /// Playback start frame (trim)
    pub trim_start: usize,
    /// Playback end frame (exclusive); 0 means use data.len()
    pub trim_end: usize,
    pub loop_enabled: bool,
    pub loop_start: usize,
    pub loop_end: usize,
    /// Original file path this sample was loaded from (for saving/reloading)
    pub source_path: Option<String>,
}

impl Sample {
    /// Effective end frame (trim_end or data length)
    pub fn end(&self) -> usize {
        if self.trim_end == 0 || self.trim_end > self.data.len() {
            self.data.len()
        } else {
            self.trim_end
        }
    }

    /// Effective loop end (loop_end or end())
    pub fn effective_loop_end(&self) -> usize {
        if self.loop_end <= self.trim_start || self.loop_end > self.end() {
            self.end()
        } else {
            self.loop_end
        }
    }

    /// Effective loop start, clamped into the part of the buffer that plays.
    ///
    /// A slice is a span of a larger buffer, so a loop point below
    /// `trim_start` would send playback into audio belonging to the
    /// neighbouring slice -- with the default `loop_start` of 0, back to the
    /// beginning of the whole file.
    pub fn effective_loop_start(&self) -> usize {
        let end = self.effective_loop_end();
        self.loop_start
            .max(self.trim_start)
            .min(end.saturating_sub(1))
    }

    /// The frame range a slicing operation covers, per [`SliceRange`].
    pub fn slice_bounds(&self, range: SliceRange) -> (usize, usize) {
        match range {
            SliceRange::Source => (0, self.data.len()),
            SliceRange::Span => (self.trim_start, self.end()),
        }
    }

    /// Get a stereo frame at the given index, or silence if out of bounds
    pub fn frame_at(&self, idx: usize) -> [f32; 2] {
        if idx < self.data.len() {
            self.data[idx]
        } else {
            [0.0, 0.0]
        }
    }

    /// Total number of frames
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the sample has no frames
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Duration of the whole buffer in seconds
    pub fn duration(&self) -> f64 {
        self.data.len() as f64 / self.sample_rate
    }

    /// Number of frames that actually play, honouring the trim bounds.
    ///
    /// Differs from [`Sample::len`] for a slice, which views a span of a
    /// larger buffer.
    pub fn played_len(&self) -> usize {
        self.end().saturating_sub(self.trim_start)
    }

    /// Duration of the part that plays, in seconds.
    pub fn played_duration(&self) -> f64 {
        self.played_len() as f64 / self.sample_rate
    }
}

/// Bank of up to 256 sample slots (matching instrument slots).
/// Each slot holds an `Arc<Sample>` so cloning the bank is cheap
/// (reference count bump, not frame-data copy).
#[derive(Clone)]
pub struct SampleBank {
    pub samples: Vec<Option<Arc<Sample>>>,
}

impl SampleBank {
    pub fn new() -> Self {
        Self {
            samples: (0..256).map(|_| None).collect(),
        }
    }
}

impl Default for SampleBank {
    fn default() -> Self {
        Self::new()
    }
}

impl SampleBank {
    /// Load a sample from a file (WAV or AIFF), auto-detected by extension.
    pub fn load(&mut self, slot: usize, path: &Path) -> Result<()> {
        if slot >= self.samples.len() {
            anyhow::bail!("Sample slot {} out of range", slot);
        }
        if let Ok(meta) = std::fs::metadata(path) {
            let size = meta.len();
            if size > crate::constants::MAX_SAMPLE_FILE_BYTES {
                anyhow::bail!(
                    "{} is {} MB, over the {} MB sample limit",
                    path.display(),
                    size / (1024 * 1024),
                    crate::constants::MAX_SAMPLE_FILE_BYTES / (1024 * 1024)
                );
            }
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let sample = match ext.as_str() {
            "wav" => load_wav(path)?,
            "aif" | "aiff" => load_aiff(path)?,
            _ => anyhow::bail!("Unsupported audio format: {}", ext),
        };
        self.samples[slot] = Some(Arc::new(sample));
        Ok(())
    }

    pub fn get(&self, slot: usize) -> Option<&Sample> {
        self.samples.get(slot).and_then(|s| s.as_deref())
    }

    /// Return sorted list of slot indices that have samples loaded.
    pub fn loaded_slots(&self) -> Vec<usize> {
        self.samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| if s.is_some() { Some(i) } else { None })
            .collect()
    }

    /// Load samples from a directory. Files should be named `<slot>-<name>.wav` or `.aiff`.
    /// Optionally reads `samples.json` for metadata (base_note, bpm, mappings).
    pub fn load_directory(&mut self, dir: &Path) -> Result<SampleDirMeta> {
        let mut meta = SampleDirMeta::default();

        // Read optional metadata file
        let meta_path = dir.join("samples.json");
        if meta_path.exists() {
            let data = std::fs::read_to_string(&meta_path)
                .with_context(|| format!("Failed to read {}", meta_path.display()))?;
            meta = serde_json::from_str(&data)
                .with_context(|| format!("Failed to parse {}", meta_path.display()))?;
        }

        // Scan directory for sample files, sorted by filename for deterministic slot assignment
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if ext != "wav" && ext != "aif" && ext != "aiff" {
                continue;
            }

            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

            // Parse <slot>-<name> format
            if let Some((slot_str, _name)) = stem.split_once('-') {
                if let Ok(slot) = slot_str.parse::<usize>() {
                    if slot < self.samples.len() {
                        self.load(slot, &path)?;

                        // Apply per-sample metadata if available. `samples.json`
                        // is hand-written and can outlive the audio it
                        // describes, so its loop points are clamped to the
                        // frames actually present -- the same normalisation
                        // `TrackerCore::load_file` does for a `.rtrk`, which
                        // this path used to skip.
                        if let Some(sample_meta) = meta.samples.get(slot_str) {
                            if let Some(arc) = self.samples[slot].as_mut() {
                                let sample = Arc::make_mut(arc);
                                let frames = sample.data.len();
                                if let Some(base) = sample_meta.base_note {
                                    sample.base_note = base;
                                }
                                if let Some(ls) = sample_meta.loop_start {
                                    sample.loop_start = ls.min(frames);
                                }
                                if let Some(le) = sample_meta.loop_end {
                                    sample.loop_end = le.min(frames);
                                }
                                // A loop needs somewhere to go. Points that
                                // clamped down to an empty or backwards range
                                // would leave playback wrapping on a single
                                // frame, which is a buzz rather than a loop,
                                // so the loop is left off instead. A zero end
                                // still means "to the end of the audio", as
                                // it does everywhere else.
                                if sample_meta.loop_enabled.unwrap_or(false) {
                                    let end = if sample.loop_end == 0 {
                                        frames
                                    } else {
                                        sample.loop_end
                                    };
                                    sample.loop_enabled = end > sample.loop_start;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(meta)
    }
}

/// Metadata from a samples.json file in a sample directory
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SampleDirMeta {
    /// BPM hint for the sample set
    #[serde(default)]
    pub bpm: Option<u16>,
    /// Per-sample metadata keyed by slot number (as string)
    #[serde(default)]
    pub samples: std::collections::HashMap<String, SampleMeta>,
}

/// Per-sample metadata within samples.json
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SampleMeta {
    pub base_note: Option<u8>,
    pub loop_enabled: Option<bool>,
    pub loop_start: Option<usize>,
    pub loop_end: Option<usize>,
}

/// Load a WAV file into a Sample using hound + dasp for type conversion
fn load_wav(path: &Path) -> Result<Sample> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("Failed to open WAV: {}", path.display()))?;

    let spec = reader.spec();
    let channels = spec.channels as usize;
    let sample_rate = spec.sample_rate as f64;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sample")
        .to_string();

    let mono_samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 8) => reader
            .into_samples::<i8>()
            .map(|s| s.map(|v| v.to_sample::<f32>()))
            .collect::<Result<Vec<f32>, _>>()
            .context("Failed to read WAV samples")?,
        (hound::SampleFormat::Int, 16) => reader
            .into_samples::<i16>()
            .map(|s| s.map(|v| v.to_sample::<f32>()))
            .collect::<Result<Vec<f32>, _>>()
            .context("Failed to read WAV samples")?,
        (hound::SampleFormat::Int, 24 | 32) => reader
            .into_samples::<i32>()
            .map(|s| s.map(|v| v.to_sample::<f32>()))
            .collect::<Result<Vec<f32>, _>>()
            .context("Failed to read WAV samples")?,
        (hound::SampleFormat::Float, _) => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .context("Failed to read WAV samples")?,
        _ => anyhow::bail!(
            "Unsupported WAV format: {:?} {}bit",
            spec.sample_format,
            spec.bits_per_sample
        ),
    };

    let data = to_stereo_frames(&mono_samples, channels);

    let source = path.to_string_lossy().to_string();
    Ok(Sample {
        name,
        data: data.into(),
        sample_rate,
        base_note: 60,
        trim_start: 0,
        trim_end: 0,
        loop_enabled: false,
        loop_start: 0,
        loop_end: 0,
        source_path: Some(source),
    })
}

/// Load an AIFF file into a Sample (basic uncompressed AIFF support)
fn load_aiff(path: &Path) -> Result<Sample> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open AIFF: {}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sample")
        .to_string();

    // Every length below is a number the file declares about itself, and each
    // one is allocated from before anything is read. Bound them by what the
    // file could possibly contain: a chunk that claims more than the whole
    // file is lying, and reserving for the claim is how a 60-byte file asks
    // for a petabyte.
    let file_len = file
        .metadata()
        .map(|m| m.len() as usize)
        .unwrap_or(usize::MAX);

    // Read FORM header
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .context("Failed to read AIFF header")?;
    if &header[0..4] != b"FORM" || (&header[8..12] != b"AIFF" && &header[8..12] != b"AIFC") {
        anyhow::bail!("Not a valid AIFF file");
    }

    let mut channels: u16 = 0;
    let mut num_frames: u32 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut sample_rate: f64 = 0.0;
    let mut sound_data: Vec<u8> = Vec::new();

    // Parse chunks
    loop {
        let mut chunk_header = [0u8; 8];
        if file.read_exact(&mut chunk_header).is_err() {
            break;
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_be_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as usize;

        match chunk_id {
            b"COMM" => {
                // The fields below occupy the first 18 bytes. A shorter chunk
                // used to be read into a shorter buffer and then indexed
                // anyway, which panics rather than reporting a bad file.
                const COMM_FIELDS: usize = 18;
                if chunk_size < COMM_FIELDS {
                    anyhow::bail!(
                        "AIFF COMM chunk is {} bytes, needs at least {}",
                        chunk_size,
                        COMM_FIELDS
                    );
                }
                let mut comm = vec![0u8; chunk_size.min(file_len)];
                file.read_exact(&mut comm)
                    .context("Failed to read COMM chunk")?;
                if comm.len() < COMM_FIELDS {
                    anyhow::bail!("AIFF COMM chunk is truncated");
                }
                channels = u16::from_be_bytes([comm[0], comm[1]]);
                num_frames = u32::from_be_bytes([comm[2], comm[3], comm[4], comm[5]]);
                bits_per_sample = u16::from_be_bytes([comm[6], comm[7]]);
                // Sample rate is 80-bit IEEE 754 extended precision
                sample_rate = extended_to_f64(&comm[8..18]);
            }
            b"SSND" => {
                let mut ssnd_header = vec![0u8; 8];
                file.read_exact(&mut ssnd_header)
                    .context("Failed to read SSND header")?;
                // The declared size counts the 8-byte offset/block_size pair
                // just read. A chunk claiming fewer than 8 wrapped the
                // subtraction round to something near usize::MAX, which was
                // then handed straight to `resize`.
                let Some(remaining) = chunk_size.checked_sub(8) else {
                    anyhow::bail!("AIFF SSND chunk is {} bytes, needs 8", chunk_size);
                };
                sound_data.resize(remaining.min(file_len), 0);
                file.read_exact(&mut sound_data)
                    .context("Failed to read SSND data")?;
            }
            _ => {
                // Skip unknown chunk (pad to even size)
                let skip = if chunk_size % 2 == 1 {
                    chunk_size + 1
                } else {
                    chunk_size
                };
                file.seek(SeekFrom::Current(skip as i64))
                    .context("Failed to skip chunk")?;
            }
        }
    }

    if channels == 0 || num_frames == 0 {
        anyhow::bail!("AIFF file has no audio data");
    }

    // Convert raw bytes to f32 samples (big-endian)
    let bytes_per_sample = (bits_per_sample as usize).div_ceil(8);
    if bytes_per_sample == 0 {
        anyhow::bail!("AIFF file declares a sample size of 0 bits");
    }
    // `num_frames x channels` are two numbers out of the file, and their
    // product overflows into an allocation no machine can serve: u32::MAX
    // frames of 65535 channels asked for 1.1PB and aborted the process
    // outright -- past the point where a panic could be caught and reported.
    // The loop below stops at the end of `sound_data` regardless, so the
    // audio actually present is the real bound and the only honest capacity.
    let declared_samples = (num_frames as usize).saturating_mul(channels as usize);
    let available_samples = sound_data.len() / bytes_per_sample;
    let total_samples = declared_samples.min(available_samples);
    let mut raw_samples = Vec::with_capacity(total_samples);

    for i in 0..total_samples {
        let offset = i * bytes_per_sample;
        if offset + bytes_per_sample > sound_data.len() {
            break;
        }
        let sample_f32 = match bytes_per_sample {
            1 => {
                let val = sound_data[offset] as i8;
                val.to_sample::<f32>()
            }
            2 => {
                let val = i16::from_be_bytes([sound_data[offset], sound_data[offset + 1]]);
                val.to_sample::<f32>()
            }
            3 => {
                // 24-bit big-endian, sign-extend to i32
                let val = ((sound_data[offset] as i32) << 24)
                    | ((sound_data[offset + 1] as i32) << 16)
                    | ((sound_data[offset + 2] as i32) << 8);
                (val >> 8).to_sample::<f32>()
            }
            4 => {
                let val = i32::from_be_bytes([
                    sound_data[offset],
                    sound_data[offset + 1],
                    sound_data[offset + 2],
                    sound_data[offset + 3],
                ]);
                val.to_sample::<f32>()
            }
            _ => 0.0,
        };
        raw_samples.push(sample_f32);
    }

    let data = to_stereo_frames(&raw_samples, channels as usize);
    let source = path.to_string_lossy().to_string();

    Ok(Sample {
        name,
        data: data.into(),
        sample_rate,
        base_note: 60,
        trim_start: 0,
        trim_end: 0,
        loop_enabled: false,
        loop_start: 0,
        loop_end: 0,
        source_path: Some(source),
    })
}

/// Convert 80-bit IEEE 754 extended precision float to f64.
/// The 80-bit format has an explicit integer bit in the mantissa (bit 63).
fn extended_to_f64(bytes: &[u8]) -> f64 {
    let sign = if bytes[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = (((bytes[0] as u16 & 0x7F) << 8) | bytes[1] as u16) as i32 - 16383;
    let mantissa = u64::from_be_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    if mantissa == 0 && exponent == -16383 {
        return 0.0;
    }
    // mantissa has explicit integer bit at position 63, so divide by 2^63 to normalize,
    // then apply exponent. Equivalent to: mantissa * 2^(exponent - 63).
    sign * (mantissa as f64) * 2.0_f64.powi(exponent - 63)
}

/// Convert interleaved mono/stereo/multi-channel samples to stereo [f32; 2] frames
fn to_stereo_frames(samples: &[f32], channels: usize) -> Vec<[f32; 2]> {
    let num_frames = samples.len() / channels;
    let mut frames = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let left = samples[i * channels];
        let right = if channels > 1 {
            samples[i * channels + 1]
        } else {
            left
        };
        frames.push([left, right]);
    }
    frames
}

/// Strip trailing `_Sxx` slice suffixes from a sample name.
///
/// Used when a slice action re-derives from the whole sample: the old
/// numbering describes a division that no longer exists, so `amen_S03`
/// sliced again is `amen_S00`.. rather than `amen_S03_S00`..
pub fn strip_slice_suffix(name: &str) -> &str {
    let mut s = name;
    while s.len() >= 4 {
        let tail = &s[s.len() - 4..];
        if tail.starts_with("_S") && tail[2..].chars().all(|c| c.is_ascii_digit()) {
            s = &s[..s.len() - 4];
        } else {
            break;
        }
    }
    s
}

/// The name sub-slices of `sample` are numbered under.
///
/// Re-deriving from the source discards the old numbering, since the
/// division it referred to is being replaced. Subdividing keeps it: the
/// pieces of `amen_S07` are `amen_S07_S00` and `amen_S07_S01`, which says
/// where they came from and, more practically, does not collide with the
/// `amen_S00` that is still sitting in another slot.
fn slice_base_name(sample: &Sample, range: SliceRange) -> String {
    match range {
        SliceRange::Source => strip_slice_suffix(&sample.name).to_string(),
        SliceRange::Span => sample.name.clone(),
    }
}

/// Slice a sample into `num_slices` equal-length segments.
/// Returns a Vec of new Sample objects, each named `<original>_S00`, `_S01`, etc.
pub fn slice_equal(sample: &Sample, num_slices: usize, range: SliceRange) -> Vec<Sample> {
    if num_slices == 0 {
        return Vec::new();
    }
    let (start, end) = sample.slice_bounds(range);
    if end <= start {
        return Vec::new();
    }
    let base_name = slice_base_name(sample, range);
    let total = end - start;
    let slice_len = total / num_slices;
    if slice_len == 0 {
        return Vec::new();
    }
    (0..num_slices)
        .map(|i| {
            let s = start + i * slice_len;
            let e = if i == num_slices - 1 {
                end
            } else {
                s + slice_len
            };
            Sample {
                name: format!("{base_name}_S{i:02}"),
                // Share the source buffer and bound this slice with the trim
                // range. Copying the frames instead would leave the slice
                // with no record of where it came from, and saving -- which
                // stores a path and a span, not audio -- would lose the
                // boundaries and reload every slot as the whole file.
                data: Arc::clone(&sample.data),
                sample_rate: sample.sample_rate,
                base_note: sample.base_note,
                trim_start: s,
                trim_end: e,
                loop_enabled: false,
                loop_start: 0,
                loop_end: 0,
                source_path: sample.source_path.clone(),
            }
        })
        .collect()
}

/// Detect transient onsets in a sample.
/// `sensitivity` ranges from 0.0 (fewer slices) to 1.0 (more slices).
/// Returns frame indices of detected transients (always includes the start frame).
/// Uses the sample's trim region as the detection range.
pub fn detect_transients(sample: &Sample, sensitivity: f32) -> Vec<usize> {
    detect_transients_range(sample, sensitivity, sample.trim_start, sample.end())
}

/// Like `detect_transients`, but with an explicit frame range.
///
/// The range is clamped to the buffer, so an out-of-range request yields
/// fewer onsets rather than panicking.
///
/// Detection runs on the *logarithmic* energy envelope: a rise from -60 dB
/// to -50 dB and one from -20 dB to -10 dB are equally likely to be heard as
/// hits, but in linear terms the second is three hundred times the change,
/// so a linear detector finds only the loudest part of a break. Each rise is
/// compared against a local average rather than the loudest rise in the
/// whole sample, for the same reason -- one loud hit should not hide the
/// rest -- and only local peaks are kept, so a single onset does not fire on
/// every window of its attack.
pub fn detect_transients_range(
    sample: &Sample,
    sensitivity: f32,
    start: usize,
    end: usize,
) -> Vec<usize> {
    let start = start.min(sample.data.len());
    let end = end.min(sample.data.len());
    if end <= start {
        return vec![start];
    }

    // Window size for energy calculation (in frames). Smaller = more responsive.
    let window = (sample.sample_rate as usize / 200).max(16); // ~5ms window
    let hop = (window / 2).max(1);

    // Log-domain energy envelope, one value per window.
    let mut energies: Vec<f32> = Vec::new();
    let mut pos = start;
    while pos + window <= end {
        let mut sum = 0.0f32;
        for i in pos..pos + window {
            let frame = sample.data[i];
            let mono = (frame[0] + frame[1]) * 0.5;
            sum += mono * mono;
        }
        let rms = (sum / window as f32).sqrt();
        // Floored at -120 dB so silence does not produce -inf deltas.
        energies.push(20.0 * rms.max(1e-6).log10());
        pos += hop;
    }

    if energies.len() < 3 {
        return vec![start];
    }

    // Spectral-flux-style detection function: rise in dB between windows.
    let mut flux: Vec<f32> = Vec::with_capacity(energies.len());
    flux.push(0.0);
    for i in 1..energies.len() {
        flux.push((energies[i] - energies[i - 1]).max(0.0));
    }

    // Higher sensitivity lowers both the relative and absolute thresholds.
    let sens = sensitivity.clamp(0.0, 1.0);
    let quiet = (1.0 - sens) * (1.0 - sens);
    let alpha = 1.0 + 3.0 * quiet;
    let floor_db = 1.0 + 12.0 * quiet;

    // Local average window for the adaptive threshold (~100ms either side).
    let avg_half = ((sample.sample_rate as usize / 10) / hop).max(2);

    // Minimum gap between transients (50ms worth of frames)
    let min_gap = (sample.sample_rate as usize / 20).max(1);
    // How far an onset may be pulled back to a quieter frame (~one window).
    let backtrack = window;

    let mut points = vec![start];
    for i in 1..flux.len() - 1 {
        let f = flux[i];
        if f < floor_db {
            continue;
        }
        // Peak picking: only the crest of a rise counts as one onset.
        if f < flux[i - 1] || f < flux[i + 1] {
            continue;
        }
        let lo = i.saturating_sub(avg_half);
        let hi = (i + avg_half + 1).min(flux.len());
        let local_mean = flux[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        if f < local_mean * alpha {
            continue;
        }

        let frame = start + i * hop;
        let previous = *points.last().unwrap();
        if frame <= previous + min_gap || frame >= end {
            continue;
        }
        // Move the boundary back to the quietest nearby frame, so the slice
        // starts in the dip before the hit rather than partway up its
        // attack -- which both clips the transient and starts the slice on a
        // large sample value.
        let limit = frame.saturating_sub(backtrack).max(previous + 1).max(start);
        let onset = (limit..=frame)
            .min_by(|&a, &b| {
                let amp = |i: usize| {
                    let fr = sample.data[i];
                    (fr[0] + fr[1]).abs() * 0.5
                };
                amp(a)
                    .partial_cmp(&amp(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(frame);
        if onset > previous + min_gap / 2 {
            points.push(onset);
        }
    }

    points
}

/// Slice a sample at the given frame positions.
/// Each slice runs from `points[i]` to `points[i+1]` (last slice runs to sample end).
pub fn slice_at_points(sample: &Sample, points: &[usize], range: SliceRange) -> Vec<Sample> {
    if points.is_empty() {
        return Vec::new();
    }
    let (_, end) = sample.slice_bounds(range);
    let base_name = slice_base_name(sample, range);
    let mut slices = Vec::with_capacity(points.len());
    for (i, &p) in points.iter().enumerate() {
        let slice_end = if i + 1 < points.len() {
            points[i + 1]
        } else {
            end
        };
        if p >= slice_end || p >= sample.data.len() {
            continue;
        }
        let actual_end = slice_end.min(sample.data.len());
        slices.push(Sample {
            name: format!("{base_name}_S{i:02}"),
            // Shared buffer plus a span; see `slice_equal`.
            data: Arc::clone(&sample.data),
            sample_rate: sample.sample_rate,
            base_note: sample.base_note,
            trim_start: p,
            trim_end: actual_end,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: sample.source_path.clone(),
        });
    }
    slices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_bank_new() {
        let bank = SampleBank::new();
        assert_eq!(bank.samples.len(), 256);
        assert!(bank.get(0).is_none());
    }

    #[test]
    fn test_sample_bank_slot_out_of_range() {
        let mut bank = SampleBank::new();
        assert!(bank.load(256, Path::new("test.wav")).is_err());
    }

    #[test]
    fn test_to_stereo_frames_mono() {
        let mono = vec![0.5f32, -0.5, 1.0];
        let frames = to_stereo_frames(&mono, 1);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], [0.5, 0.5]); // mono duplicated
        assert_eq!(frames[1], [-0.5, -0.5]);
    }

    #[test]
    fn test_to_stereo_frames_stereo() {
        let stereo = vec![0.5f32, -0.5, 1.0, -1.0];
        let frames = to_stereo_frames(&stereo, 2);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], [0.5, -0.5]);
        assert_eq!(frames[1], [1.0, -1.0]);
    }

    #[test]
    fn test_extended_to_f64() {
        // 44100 Hz as 80-bit extended: exponent = 16383+15 = 16398 = 0x400E
        // mantissa = 44100 << 48 = 0xAC44000000000000
        let bytes: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let rate = extended_to_f64(&bytes);
        assert!(
            (rate - 44100.0).abs() < 1.0,
            "expected ~44100, got {}",
            rate
        );
    }

    #[test]
    fn test_sample_end_default() {
        let sample = Sample {
            name: "test".into(),
            data: vec![[0.0; 2]; 100].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0, // 0 means use data.len()
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        assert_eq!(sample.end(), 100);
        assert_eq!(sample.len(), 100);
    }

    #[test]
    fn test_sample_trimmed() {
        let sample = Sample {
            name: "test".into(),
            data: vec![[0.0; 2]; 100].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 10,
            trim_end: 50,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        assert_eq!(sample.end(), 50);
    }

    fn slice_of(len: usize, trim_start: usize, trim_end: usize) -> Sample {
        Sample {
            name: "slice".into(),
            data: vec![[0.0; 2]; len].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start,
            trim_end,
            loop_enabled: true,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }
    }

    #[test]
    fn test_slice_bounds() {
        let slice = slice_of(1000, 400, 700);
        assert_eq!(slice.slice_bounds(SliceRange::Source), (0, 1000));
        assert_eq!(slice.slice_bounds(SliceRange::Span), (400, 700));

        // An untrimmed sample reads the same either way.
        let whole = slice_of(1000, 0, 0);
        assert_eq!(
            whole.slice_bounds(SliceRange::Source),
            whole.slice_bounds(SliceRange::Span)
        );
    }

    #[test]
    fn test_slice_equal_source_ignores_the_span() {
        let slice = slice_of(1000, 400, 700);
        let pieces = slice_equal(&slice, 4, SliceRange::Source);
        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces[0].trim_start, 0);
        assert_eq!(pieces[3].end(), 1000);
    }

    #[test]
    fn test_slice_equal_span_stays_inside_it() {
        let slice = slice_of(1000, 400, 700);
        let pieces = slice_equal(&slice, 3, SliceRange::Span);
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[0].trim_start, 400);
        assert_eq!(pieces[2].end(), 700);
    }

    #[test]
    fn test_strip_slice_suffix() {
        assert_eq!(strip_slice_suffix("amen"), "amen");
        assert_eq!(strip_slice_suffix("amen_S03"), "amen");
        assert_eq!(strip_slice_suffix("amen_S00_S01"), "amen");
        assert_eq!(strip_slice_suffix("_S01"), "");
        assert_eq!(strip_slice_suffix("kick_Sxx"), "kick_Sxx");
    }

    #[test]
    fn test_re_slicing_from_source_does_not_stack_name_suffixes() {
        let source = slice_of(1000, 0, 0);
        let first = slice_equal(&source, 4, SliceRange::Source);
        assert_eq!(first[1].name, "slice_S01");
        // The old numbering described a division that no longer exists.
        let second = slice_equal(&first[1], 2, SliceRange::Source);
        assert_eq!(second[0].name, "slice_S00");
        assert_eq!(second[1].name, "slice_S01");
    }

    #[test]
    fn test_subdividing_keeps_the_parent_in_the_name() {
        // The pieces of slice 1 have to be tellable from slice 1 and slice 2,
        // which are still in their own slots.
        let source = slice_of(1000, 0, 0);
        let first = slice_equal(&source, 4, SliceRange::Source);
        let second = slice_equal(&first[1], 2, SliceRange::Span);
        assert_eq!(second[0].name, "slice_S01_S00");
        assert_eq!(second[1].name, "slice_S01_S01");
    }

    #[test]
    fn test_loop_points_clamped_to_the_slice() {
        // Default loop points (0, 0) on a slice must mean "this slice",
        // not "the whole source buffer".
        let slice = slice_of(1000, 400, 700);
        assert_eq!(slice.effective_loop_start(), 400);
        assert_eq!(slice.effective_loop_end(), 700);
    }

    #[test]
    fn test_loop_points_outside_the_slice_are_pulled_in() {
        let mut slice = slice_of(1000, 400, 700);
        slice.loop_start = 100; // before the slice
        slice.loop_end = 900; // after it
        assert_eq!(slice.effective_loop_start(), 400);
        assert_eq!(slice.effective_loop_end(), 700);
    }

    #[test]
    fn test_loop_points_inside_the_slice_are_kept() {
        let mut slice = slice_of(1000, 400, 700);
        slice.loop_start = 500;
        slice.loop_end = 600;
        assert_eq!(slice.effective_loop_start(), 500);
        assert_eq!(slice.effective_loop_end(), 600);
    }

    #[test]
    fn test_detect_transients_range_beyond_the_buffer() {
        // Callers pass frame numbers; a stale or mistaken one must not
        // bring down the audio thread.
        let sample = slice_of(1000, 0, 0);
        let points = detect_transients_range(&sample, 0.5, 0, 50_000);
        assert!(!points.is_empty());
        assert!(points.iter().all(|&p| p <= 1000));

        let points = detect_transients_range(&sample, 0.5, 50_000, 60_000);
        assert_eq!(points, vec![1000]);
    }

    #[test]
    fn test_detect_transients_empty_range_returns_its_start() {
        let sample = slice_of(1000, 0, 0);
        assert_eq!(detect_transients_range(&sample, 0.5, 500, 500), vec![500]);
    }

    #[test]
    fn test_detect_transients_finds_hits_below_the_loudest() {
        // Two hits, the second 30 dB quieter. A detector that scales its
        // threshold to the loudest rise in the sample finds only the first.
        let sr = 44100.0;
        let mut data = vec![[0.0f32; 2]; 44100];
        for (i, amp) in [(2000usize, 1.0f32), (22050, 0.03)] {
            for k in 0..4000 {
                let decay = 1.0 - (k as f32 / 4000.0);
                let v = amp * decay * ((k as f32 * 0.3).sin());
                data[i + k] = [v, v];
            }
        }
        let mut sample = slice_of(1, 0, 0);
        sample.data = data.into();
        sample.sample_rate = sr;

        let points = detect_transients(&sample, 0.5);
        assert!(
            points.iter().any(|&p| (20_000..24_000).contains(&p)),
            "quiet second hit was missed: {points:?}"
        );
    }

    #[test]
    fn test_sample_frame_at_out_of_bounds() {
        let sample = Sample {
            name: "test".into(),
            data: vec![[1.0, -1.0]].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        assert_eq!(sample.frame_at(0), [1.0, -1.0]);
        assert_eq!(sample.frame_at(1), [0.0, 0.0]); // out of bounds = silence
    }

    #[test]
    fn test_load_nonexistent_wav() {
        let mut bank = SampleBank::new();
        assert!(bank.load(0, Path::new("/nonexistent/file.wav")).is_err());
    }

    #[test]
    fn test_load_unsupported_format() {
        let mut bank = SampleBank::new();
        assert!(bank.load(0, Path::new("file.mp3")).is_err());
    }

    #[test]
    fn test_sample_dir_meta_deserialize() {
        let json = r#"{
            "bpm": 140,
            "samples": {
                "0": { "base_note": 48, "loop_enabled": true, "loop_start": 100, "loop_end": 500 },
                "1": { "base_note": 60 }
            }
        }"#;
        let meta: SampleDirMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.bpm, Some(140));
        assert_eq!(meta.samples.len(), 2);
        assert_eq!(meta.samples["0"].base_note, Some(48));
        assert_eq!(meta.samples["0"].loop_enabled, Some(true));
        assert_eq!(meta.samples["0"].loop_start, Some(100));
        assert_eq!(meta.samples["1"].base_note, Some(60));
        assert_eq!(meta.samples["1"].loop_end, None);
    }

    #[test]
    fn test_sample_dir_meta_default() {
        let meta = SampleDirMeta::default();
        assert_eq!(meta.bpm, None);
        assert!(meta.samples.is_empty());
    }

    #[test]
    fn test_load_directory_nonexistent() {
        let mut bank = SampleBank::new();
        assert!(bank.load_directory(Path::new("/nonexistent/dir")).is_err());
    }

    #[test]
    fn test_load_directory_empty() {
        let dir = std::env::temp_dir().join("rtrack_test_empty_dir");
        let _ = std::fs::create_dir_all(&dir);
        let mut bank = SampleBank::new();
        let result = bank.load_directory(&dir);
        assert!(result.is_ok());
        // No samples should be loaded
        assert!(bank.get(0).is_none());
        let _ = std::fs::remove_dir(&dir);
    }

    /// A mono 16-bit WAV of `frames` rising samples, for tests that only
    /// care about how metadata is applied to it.
    fn write_test_wav(path: &std::path::Path, frames: usize) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            writer.write_sample((i * 100) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn test_load_directory_with_wav() {
        let dir = std::env::temp_dir().join("rtrack_test_sample_dir");
        let _ = std::fs::create_dir_all(&dir);

        // Create a minimal WAV file
        let path = dir.join("0-kick.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..100 {
            writer.write_sample((i * 100) as i16).unwrap();
        }
        writer.finalize().unwrap();

        // Create metadata
        let meta_json = r#"{ "bpm": 120, "samples": { "0": { "base_note": 36 } } }"#;
        std::fs::write(dir.join("samples.json"), meta_json).unwrap();

        let mut bank = SampleBank::new();
        let meta = bank.load_directory(&dir).unwrap();
        assert_eq!(meta.bpm, Some(120));
        assert!(bank.get(0).is_some());
        assert_eq!(bank.get(0).unwrap().base_note, 36);
        assert_eq!(bank.get(0).unwrap().name, "0-kick");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `samples.json` is hand-written and can outlive the audio it describes.
    /// Loop points past the end of the file used to be applied as given; a
    /// `.rtrk` load clamped them, so the same sample set behaved differently
    /// depending on which path loaded it.
    #[test]
    fn test_load_directory_clamps_out_of_range_loop_points() {
        let dir = std::env::temp_dir().join("rtrack_test_loop_clamp");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_test_wav(&dir.join("0-kick.wav"), 100);

        let meta_json = r#"{ "samples": { "0": {
            "loop_enabled": true, "loop_start": 5000, "loop_end": 6000 } } }"#;
        std::fs::write(dir.join("samples.json"), meta_json).unwrap();

        let mut bank = SampleBank::new();
        bank.load_directory(&dir).unwrap();
        let sample = bank.get(0).unwrap();

        assert!(sample.loop_start <= 100, "loop start past the audio");
        assert!(sample.loop_end <= 100, "loop end past the audio");
        // Both points clamp onto the last frame, leaving nothing to loop over.
        assert!(
            !sample.loop_enabled,
            "a loop with no frames in it was left enabled"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Clamping must not cost the ordinary case: a loop over the whole file,
    /// written as an enabled loop with no points, still loops.
    #[test]
    fn test_load_directory_keeps_a_whole_file_loop() {
        let dir = std::env::temp_dir().join("rtrack_test_loop_whole");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_test_wav(&dir.join("0-kick.wav"), 100);

        let meta_json = r#"{ "samples": { "0": { "loop_enabled": true } } }"#;
        std::fs::write(dir.join("samples.json"), meta_json).unwrap();

        let mut bank = SampleBank::new();
        bank.load_directory(&dir).unwrap();
        let sample = bank.get(0).unwrap();

        assert!(sample.loop_enabled);
        assert_eq!(sample.effective_loop_start(), 0);
        assert_eq!(sample.effective_loop_end(), 100);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Points that fit the audio are left exactly as written.
    #[test]
    fn test_load_directory_keeps_loop_points_that_fit() {
        let dir = std::env::temp_dir().join("rtrack_test_loop_fits");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_test_wav(&dir.join("0-kick.wav"), 100);

        let meta_json = r#"{ "samples": { "0": {
            "loop_enabled": true, "loop_start": 20, "loop_end": 80 } } }"#;
        std::fs::write(dir.join("samples.json"), meta_json).unwrap();

        let mut bank = SampleBank::new();
        bank.load_directory(&dir).unwrap();
        let sample = bank.get(0).unwrap();

        assert!(sample.loop_enabled);
        assert_eq!(sample.loop_start, 20);
        assert_eq!(sample.loop_end, 80);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_directory_deterministic_order() {
        // Create a temp dir with multiple samples in non-alphabetical order.
        // Verify they always land in the same slots regardless of FS iteration order.
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        // Create files in reverse order to stress iteration ordering
        for &(slot, name) in &[(2, "2-hihat"), (0, "0-kick"), (1, "1-snare")] {
            let path = dir.path().join(format!("{}-{}.wav", slot, name));
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            // Write distinct sample lengths so we can verify identity
            for i in 0..((slot + 1) * 100) {
                writer.write_sample((i * 50) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        // Load twice and verify identical slot assignment
        let mut bank1 = SampleBank::new();
        bank1.load_directory(dir.path()).unwrap();
        let mut bank2 = SampleBank::new();
        bank2.load_directory(dir.path()).unwrap();

        for slot in 0..3 {
            let s1 = bank1
                .get(slot)
                .unwrap_or_else(|| panic!("slot {} missing in bank1", slot));
            let s2 = bank2
                .get(slot)
                .unwrap_or_else(|| panic!("slot {} missing in bank2", slot));
            assert_eq!(s1.name, s2.name, "slot {} name mismatch", slot);
            assert_eq!(
                s1.data.len(),
                s2.data.len(),
                "slot {} length mismatch",
                slot
            );
        }

        // Verify correct slot assignment by sample length
        assert_eq!(bank1.get(0).unwrap().data.len(), 100);
        assert_eq!(bank1.get(1).unwrap().data.len(), 200);
        assert_eq!(bank1.get(2).unwrap().data.len(), 300);
    }

    fn make_slice_sample(len: usize) -> Sample {
        let data: Vec<[f32; 2]> = (0..len)
            .map(|i| {
                let t = i as f32 / len as f32;
                let val = (t * std::f32::consts::TAU * 4.0).sin();
                [val, val]
            })
            .collect();
        Sample {
            name: "test".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        }
    }

    #[test]
    fn test_slice_equal_basic() {
        let sample = make_slice_sample(1000);
        let slices = slice_equal(&sample, 4, SliceRange::Source);
        assert_eq!(slices.len(), 4);
        // Slices are spans of the shared buffer, so it is the played length
        // that is a quarter of the source, not the buffer length.
        for (i, slice) in slices.iter().enumerate() {
            assert_eq!(slice.played_len(), 250, "slice {i}");
            assert_eq!(slice.trim_start, i * 250, "slice {i} start");
            assert_eq!(slice.end(), (i + 1) * 250, "slice {i} end");
        }
        assert_eq!(slices[0].name, "test_S00");
        assert_eq!(slices[3].name, "test_S03");
    }

    #[test]
    fn test_slice_equal_preserves_metadata() {
        let mut sample = make_slice_sample(1000);
        sample.base_note = 48;
        sample.sample_rate = 48000.0;
        let slices = slice_equal(&sample, 2, SliceRange::Source);
        assert_eq!(slices[0].base_note, 48);
        assert_eq!(slices[0].sample_rate, 48000.0);
    }

    #[test]
    fn test_slice_equal_with_trim() {
        let mut sample = make_slice_sample(1000);
        sample.trim_start = 100;
        sample.trim_end = 500;
        let slices = slice_equal(&sample, 4, SliceRange::Span);
        assert_eq!(slices.len(), 4);
        // (500-100)/4 = 100 frames each, positioned within the trimmed range
        assert_eq!(slices[0].played_len(), 100);
        assert_eq!(slices[0].trim_start, 100);
        assert_eq!(slices[3].played_len(), 100);
        assert_eq!(slices[3].end(), 500);
    }

    #[test]
    fn test_slice_equal_last_gets_remainder() {
        let sample = make_slice_sample(1003);
        let slices = slice_equal(&sample, 4, SliceRange::Source);
        assert_eq!(slices.len(), 4);
        // 1003/4 = 250 per slice, last gets remainder
        assert_eq!(slices[0].played_len(), 250);
        assert_eq!(slices[3].played_len(), 253); // 1003 - 250*3 = 253
                                                 // The slices tile the source exactly, with no gap or overlap.
        assert_eq!(slices[0].trim_start, 0);
        assert_eq!(slices[3].end(), 1003);
        for pair in slices.windows(2) {
            assert_eq!(pair[0].end(), pair[1].trim_start);
        }
    }

    #[test]
    fn test_slice_equal_zero() {
        let sample = make_slice_sample(100);
        assert!(slice_equal(&sample, 0, SliceRange::Source).is_empty());
    }

    #[test]
    fn test_slice_equal_one() {
        let sample = make_slice_sample(100);
        let slices = slice_equal(&sample, 1, SliceRange::Source);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].data.len(), 100);
    }

    #[test]
    fn test_detect_transients_silent() {
        let sample = Sample {
            name: "silent".into(),
            data: vec![[0.0; 2]; 44100].into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let points = detect_transients(&sample, 0.5);
        assert_eq!(points.len(), 1); // just the start
        assert_eq!(points[0], 0);
    }

    #[test]
    fn test_detect_transients_with_onset() {
        // Build a sample: silence then loud burst, repeated
        let mut data = vec![[0.0f32; 2]; 44100]; // 1 second total
                                                 // Burst at ~0.25s
        for frame in &mut data[11025..13000] {
            *frame = [0.8, 0.8];
        }
        // Burst at ~0.6s
        for frame in &mut data[26460..28000] {
            *frame = [0.9, 0.9];
        }
        let sample = Sample {
            name: "bursts".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let points = detect_transients(&sample, 0.5);
        // Should detect at least the initial point and the two bursts
        assert!(
            points.len() >= 2,
            "Expected at least 2 transient points, got {}",
            points.len()
        );
    }

    #[test]
    fn test_detect_transients_sensitivity() {
        // Higher sensitivity should find more (or equal) transients
        let mut data = vec![[0.0f32; 2]; 44100];
        for frame in &mut data[11025..12000] {
            *frame = [0.5, 0.5];
        }
        for frame in &mut data[22050..23000] {
            *frame = [0.8, 0.8];
        }
        for frame in &mut data[33075..34000] {
            *frame = [0.3, 0.3];
        }
        let sample = Sample {
            name: "multi".into(),
            data: data.into(),
            sample_rate: 44100.0,
            base_note: 60,
            trim_start: 0,
            trim_end: 0,
            loop_enabled: false,
            loop_start: 0,
            loop_end: 0,
            source_path: None,
        };
        let low = detect_transients(&sample, 0.2);
        let high = detect_transients(&sample, 0.9);
        assert!(
            high.len() >= low.len(),
            "Higher sensitivity should find >= transients: low={}, high={}",
            low.len(),
            high.len()
        );
    }

    #[test]
    fn test_slice_at_points() {
        let sample = make_slice_sample(1000);
        let points = vec![0, 250, 500, 750];
        let slices = slice_at_points(&sample, &points, SliceRange::Source);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0].played_len(), 250);
        assert_eq!(slices[3].played_len(), 250); // 750..1000
        assert_eq!(slices[3].trim_start, 750);
        assert_eq!(slices[0].name, "test_S00");
    }

    #[test]
    fn test_slice_at_points_empty() {
        let sample = make_slice_sample(100);
        assert!(slice_at_points(&sample, &[], SliceRange::Source).is_empty());
    }

    #[test]
    fn test_slice_at_points_single() {
        let sample = make_slice_sample(100);
        let slices = slice_at_points(&sample, &[0], SliceRange::Source);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].played_len(), 100);
    }
}
