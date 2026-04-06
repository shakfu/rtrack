pub mod export;
pub mod playback;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use dasp::Sample as DaspSample;

/// A loaded audio sample stored as stereo f32 frames.
#[derive(Clone)]
pub struct Sample {
    pub name: String,
    /// Stereo frames: [left, right] pairs
    pub data: Vec<[f32; 2]>,
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
        if self.loop_end == 0 || self.loop_end > self.end() {
            self.end()
        } else {
            self.loop_end
        }
    }

    /// Effective loop start (clamped to < loop_end)
    pub fn effective_loop_start(&self) -> usize {
        self.loop_start
            .min(self.effective_loop_end().saturating_sub(1))
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

    /// Duration in seconds
    pub fn duration(&self) -> f64 {
        self.data.len() as f64 / self.sample_rate
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

                        // Apply per-sample metadata if available
                        if let Some(sample_meta) = meta.samples.get(slot_str) {
                            if let Some(arc) = self.samples[slot].as_mut() {
                                let sample = Arc::make_mut(arc);
                                if let Some(base) = sample_meta.base_note {
                                    sample.base_note = base;
                                }
                                if let Some(ls) = sample_meta.loop_start {
                                    sample.loop_start = ls;
                                }
                                if let Some(le) = sample_meta.loop_end {
                                    sample.loop_end = le;
                                }
                                if sample_meta.loop_enabled.unwrap_or(false) {
                                    sample.loop_enabled = true;
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
        data,
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
                let mut comm = vec![0u8; chunk_size];
                file.read_exact(&mut comm)
                    .context("Failed to read COMM chunk")?;
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
                // offset and block_size (usually 0)
                let remaining = chunk_size - 8;
                sound_data.resize(remaining, 0);
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
    let total_samples = num_frames as usize * channels as usize;
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
        data,
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

/// Slice a sample into `num_slices` equal-length segments.
/// Returns a Vec of new Sample objects, each named `<original>_S00`, `_S01`, etc.
pub fn slice_equal(sample: &Sample, num_slices: usize) -> Vec<Sample> {
    if num_slices == 0 {
        return Vec::new();
    }
    let start = sample.trim_start;
    let end = sample.end();
    if end <= start {
        return Vec::new();
    }
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
                name: format!("{}_S{:02}", sample.name, i),
                data: sample.data[s..e].to_vec(),
                sample_rate: sample.sample_rate,
                base_note: sample.base_note,
                trim_start: 0,
                trim_end: 0,
                loop_enabled: false,
                loop_start: 0,
                loop_end: 0,
                source_path: sample.source_path.clone(),
            }
        })
        .collect()
}

/// Detect transient onsets in a sample using energy envelope derivative.
/// `sensitivity` ranges from 0.0 (fewer slices) to 1.0 (more slices).
/// Returns frame indices of detected transients (always includes the start frame).
/// Uses the sample's trim region as the detection range.
pub fn detect_transients(sample: &Sample, sensitivity: f32) -> Vec<usize> {
    detect_transients_range(sample, sensitivity, sample.trim_start, sample.end())
}

/// Like `detect_transients`, but with an explicit frame range.
pub fn detect_transients_range(
    sample: &Sample,
    sensitivity: f32,
    start: usize,
    end: usize,
) -> Vec<usize> {
    if end <= start {
        return vec![0];
    }

    // Window size for energy calculation (in frames). Smaller = more responsive.
    let window = (sample.sample_rate as usize / 200).max(16); // ~5ms window
    let hop = window / 2;

    // Calculate RMS energy per window
    let mut energies: Vec<f32> = Vec::new();
    let mut pos = start;
    while pos + window <= end {
        let mut sum = 0.0f32;
        for i in pos..pos + window {
            let frame = sample.data[i];
            let mono = (frame[0] + frame[1]) * 0.5;
            sum += mono * mono;
        }
        energies.push((sum / window as f32).sqrt());
        pos += hop;
    }

    if energies.len() < 2 {
        return vec![start];
    }

    // Compute the positive derivative (energy increase) between consecutive windows
    let mut deltas: Vec<f32> = Vec::with_capacity(energies.len());
    deltas.push(0.0);
    for i in 1..energies.len() {
        deltas.push((energies[i] - energies[i - 1]).max(0.0));
    }

    // Find the maximum delta for threshold scaling
    let max_delta = deltas.iter().cloned().fold(0.0f32, f32::max);
    if max_delta <= 0.0 {
        return vec![start];
    }

    // Threshold: higher sensitivity = lower threshold = more transients detected
    let threshold = max_delta * (1.0 - sensitivity.clamp(0.0, 1.0)) * 0.8 + max_delta * 0.02;

    // Minimum gap between transients (50ms worth of frames)
    let min_gap = (sample.sample_rate as usize / 20).max(1);

    let mut points = vec![start];
    for (i, &d) in deltas.iter().enumerate() {
        if d >= threshold {
            let frame = start + i * hop;
            if frame > *points.last().unwrap() + min_gap && frame < end {
                points.push(frame);
            }
        }
    }

    points
}

/// Slice a sample at the given frame positions.
/// Each slice runs from points[i] to points[i+1] (last slice runs to sample end).
pub fn slice_at_points(sample: &Sample, points: &[usize]) -> Vec<Sample> {
    if points.is_empty() {
        return Vec::new();
    }
    let end = sample.end();
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
            name: format!("{}_S{:02}", sample.name, i),
            data: sample.data[p..actual_end].to_vec(),
            sample_rate: sample.sample_rate,
            base_note: sample.base_note,
            trim_start: 0,
            trim_end: 0,
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
            data: vec![[0.0; 2]; 100],
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
            data: vec![[0.0; 2]; 100],
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

    #[test]
    fn test_sample_frame_at_out_of_bounds() {
        let sample = Sample {
            name: "test".into(),
            data: vec![[1.0, -1.0]],
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
            data,
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
        let slices = slice_equal(&sample, 4);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0].data.len(), 250);
        assert_eq!(slices[1].data.len(), 250);
        assert_eq!(slices[2].data.len(), 250);
        assert_eq!(slices[3].data.len(), 250);
        assert_eq!(slices[0].name, "test_S00");
        assert_eq!(slices[3].name, "test_S03");
    }

    #[test]
    fn test_slice_equal_preserves_metadata() {
        let mut sample = make_slice_sample(1000);
        sample.base_note = 48;
        sample.sample_rate = 48000.0;
        let slices = slice_equal(&sample, 2);
        assert_eq!(slices[0].base_note, 48);
        assert_eq!(slices[0].sample_rate, 48000.0);
    }

    #[test]
    fn test_slice_equal_with_trim() {
        let mut sample = make_slice_sample(1000);
        sample.trim_start = 100;
        sample.trim_end = 500;
        let slices = slice_equal(&sample, 4);
        assert_eq!(slices.len(), 4);
        // (500-100)/4 = 100 frames each
        assert_eq!(slices[0].data.len(), 100);
        assert_eq!(slices[3].data.len(), 100);
    }

    #[test]
    fn test_slice_equal_last_gets_remainder() {
        let sample = make_slice_sample(1003);
        let slices = slice_equal(&sample, 4);
        assert_eq!(slices.len(), 4);
        // 1003/4 = 250 per slice, last gets remainder
        assert_eq!(slices[0].data.len(), 250);
        assert_eq!(slices[3].data.len(), 253); // 1003 - 250*3 = 253
    }

    #[test]
    fn test_slice_equal_zero() {
        let sample = make_slice_sample(100);
        assert!(slice_equal(&sample, 0).is_empty());
    }

    #[test]
    fn test_slice_equal_one() {
        let sample = make_slice_sample(100);
        let slices = slice_equal(&sample, 1);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].data.len(), 100);
    }

    #[test]
    fn test_detect_transients_silent() {
        let sample = Sample {
            name: "silent".into(),
            data: vec![[0.0; 2]; 44100],
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
            data,
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
            data,
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
        let slices = slice_at_points(&sample, &points);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0].data.len(), 250);
        assert_eq!(slices[3].data.len(), 250); // 750..1000
        assert_eq!(slices[0].name, "test_S00");
    }

    #[test]
    fn test_slice_at_points_empty() {
        let sample = make_slice_sample(100);
        assert!(slice_at_points(&sample, &[]).is_empty());
    }

    #[test]
    fn test_slice_at_points_single() {
        let sample = make_slice_sample(100);
        let slices = slice_at_points(&sample, &[0]);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].data.len(), 100);
    }
}
