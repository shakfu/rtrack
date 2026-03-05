pub mod export;
pub mod playback;

use std::path::Path;

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
        self.loop_start.min(self.effective_loop_end().saturating_sub(1))
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

    /// Duration in seconds
    pub fn duration(&self) -> f64 {
        self.data.len() as f64 / self.sample_rate
    }
}

/// Bank of up to 256 sample slots (matching instrument slots)
#[derive(Clone)]
pub struct SampleBank {
    pub samples: Vec<Option<Sample>>,
}

impl SampleBank {
    pub fn new() -> Self {
        Self {
            samples: (0..256).map(|_| None).collect(),
        }
    }

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
        self.samples[slot] = Some(sample);
        Ok(())
    }

    pub fn get(&self, slot: usize) -> Option<&Sample> {
        self.samples.get(slot).and_then(|s| s.as_ref())
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

        // Scan directory for sample files
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if ext != "wav" && ext != "aif" && ext != "aiff" {
                continue;
            }

            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            // Parse <slot>-<name> format
            if let Some((slot_str, _name)) = stem.split_once('-') {
                if let Ok(slot) = slot_str.parse::<usize>() {
                    if slot < self.samples.len() {
                        self.load(slot, &path)?;

                        // Apply per-sample metadata if available
                        if let Some(sample_meta) = meta.samples.get(slot_str) {
                            if let Some(sample) = self.samples[slot].as_mut() {
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
            .map(|s| s.map(|v| v))
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

    let mut file =
        std::fs::File::open(path).with_context(|| format!("Failed to open AIFF: {}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sample")
        .to_string();

    // Read FORM header
    let mut header = [0u8; 12];
    file.read_exact(&mut header).context("Failed to read AIFF header")?;
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
                file.read_exact(&mut comm).context("Failed to read COMM chunk")?;
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
    let bytes_per_sample = (bits_per_sample as usize + 7) / 8;
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
        assert!((rate - 44100.0).abs() < 1.0, "expected ~44100, got {}", rate);
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
}
