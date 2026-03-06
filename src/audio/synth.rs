use fundsp::prelude32::*;
use fundsp::realseq::SequencerBackend;

/// Built-in synthesizer using fundsp. Provides basic waveform patches with
/// ADSR envelopes and filtering. Uses Sequencer for polyphonic voice management.
pub struct FundspSynth {
    /// Frontend: add/edit events from the main thread
    pub sequencer: Sequencer,
    /// Track active notes: (channel, note) -> EventId for note-off
    active_voices: Vec<(u8, u8, EventId)>,
    /// Current program per channel (selects waveform patch)
    programs: [u8; 16],
    sample_rate: f64,
}

/// Available synth patches
#[derive(Debug, Clone, Copy)]
pub enum Patch {
    Saw,
    Square,
    Sine,
    Triangle,
    Pulse,
    FmBell,
    Organ,
    Noise,
}

impl Patch {
    pub fn from_program(program: u8) -> Self {
        match program % 8 {
            0 => Patch::Saw,
            1 => Patch::Square,
            2 => Patch::Sine,
            3 => Patch::Triangle,
            4 => Patch::Pulse,
            5 => Patch::FmBell,
            6 => Patch::Organ,
            7 => Patch::Noise,
            _ => Patch::Saw,
        }
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Patch::Saw => "Saw",
            Patch::Square => "Square",
            Patch::Sine => "Sine",
            Patch::Triangle => "Triangle",
            Patch::Pulse => "Pulse (Sq)",
            Patch::FmBell => "FM Bell",
            Patch::Organ => "Organ",
            Patch::Noise => "Noise",
        }
    }
}

/// Convert MIDI note number to frequency in Hz
fn midi_to_freq(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
}

/// Master volume applied to all voices to keep output within headroom.
/// Raw oscillators peak at +/-1.0; with effects summing dry+wet the total
/// can exceed 1.0, so we attenuate here to prevent clipping.
const VOICE_GAIN: f32 = 0.25;

/// Create a stereo voice (fundsp AudioUnit) for a given patch and frequency
fn make_voice(patch: Patch, freq: f32, velocity: f32) -> Box<dyn AudioUnit> {
    let f = freq;
    let vol = velocity * VOICE_GAIN;

    match patch {
        Patch::Saw => Box::new(
            (saw_hz(f) >> lowpass_hz(f * 4.0, 0.7)) * vol
                | (saw_hz(f) >> lowpass_hz(f * 4.0, 0.7)) * vol,
        ),
        Patch::Square => Box::new(
            (square_hz(f) >> lowpass_hz(f * 3.0, 0.7)) * vol
                | (square_hz(f) >> lowpass_hz(f * 3.0, 0.7)) * vol,
        ),
        Patch::Sine => {
            Box::new(sine_hz(f) * vol | sine_hz(f) * vol)
        }
        Patch::Triangle => Box::new(
            (triangle_hz(f) >> lowpass_hz(f * 6.0, 0.7)) * vol
                | (triangle_hz(f) >> lowpass_hz(f * 6.0, 0.7)) * vol,
        ),
        Patch::Pulse => Box::new(
            (square_hz(f) >> lowpass_hz(f * 3.0, 0.5)) * vol
                | (square_hz(f) >> lowpass_hz(f * 3.0, 0.5)) * vol,
        ),
        Patch::FmBell => {
            // Simple FM: modulator modulates carrier frequency
            let ratio = 3.5;
            let mod_index = 2.0;
            Box::new(
                (sine_hz(f * ratio) * f * mod_index + f >> sine()) * vol
                    | (sine_hz(f * ratio) * f * mod_index + f >> sine()) * vol,
            )
        }
        Patch::Organ => Box::new(
            organ_hz(f) * vol | organ_hz(f) * vol,
        ),
        Patch::Noise => Box::new(
            (noise() >> lowpass_hz(f * 2.0, 1.0)) * vol
                | (noise() >> lowpass_hz(f * 2.0, 1.0)) * vol,
        ),
    }
}

impl FundspSynth {
    pub fn new(sample_rate: f64) -> Self {
        let mut sequencer = Sequencer::new(0, 2, ReplayMode::None);
        sequencer.set_sample_rate(sample_rate);
        Self {
            sequencer,
            active_voices: Vec::new(),
            programs: [0; 16],
            sample_rate,
        }
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        // Kill all existing voices on this channel (tracker: one note per channel)
        self.note_off_all_channel(channel);

        let patch = Patch::from_program(self.programs[channel as usize & 0x0F]);
        let freq = midi_to_freq(note);
        let vel = velocity as f32 / 127.0;

        let mut voice = make_voice(patch, freq, vel);
        voice.set_sample_rate(self.sample_rate);

        // Push with indefinite duration (f64::MAX), short fade-in, will edit on note-off
        let id = self.sequencer.push_relative(
            0.0,
            f64::MAX,
            Fade::Smooth,
            0.005, // 5ms fade-in to avoid click
            0.05,  // 50ms fade-out (used when note-off edits the event)
            voice,
        );

        self.active_voices.push((channel, note, id));
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        let mut i = 0;
        while i < self.active_voices.len() {
            if self.active_voices[i].0 == channel && self.active_voices[i].1 == note {
                let (_, _, id) = self.active_voices.remove(i);
                // End the event: end_time must be >= fade_out so the fade
                // window starts at current_time and ends at current_time + fade.
                self.sequencer.edit_relative(id, 0.05, 0.05);
            } else {
                i += 1;
            }
        }
    }

    pub fn note_off_all_channel(&mut self, channel: u8) {
        let mut i = 0;
        while i < self.active_voices.len() {
            if self.active_voices[i].0 == channel {
                let (_, _, id) = self.active_voices.remove(i);
                self.sequencer.edit_relative(id, 0.02, 0.02);
            } else {
                i += 1;
            }
        }
    }

    pub fn note_off_all(&mut self) {
        for &(_, _, id) in &self.active_voices {
            self.sequencer.edit_relative(id, 0.02, 0.02);
        }
        self.active_voices.clear();
    }

    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.programs[(channel & 0x0F) as usize] = program;
    }

    /// Get the backend for use in the audio callback thread
    pub fn backend(&mut self) -> SequencerBackend {
        self.sequencer.backend()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_to_freq() {
        let a4 = midi_to_freq(69);
        assert!((a4 - 440.0).abs() < 0.01);

        let a5 = midi_to_freq(81);
        assert!((a5 - 880.0).abs() < 0.1);
    }

    #[test]
    fn test_patch_from_program() {
        assert!(matches!(Patch::from_program(0), Patch::Saw));
        assert!(matches!(Patch::from_program(2), Patch::Sine));
        assert!(matches!(Patch::from_program(8), Patch::Saw)); // wraps
    }

    #[test]
    fn test_synth_voice_lifecycle() {
        let mut synth = FundspSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        assert_eq!(synth.active_voices.len(), 1);

        synth.note_off(0, 60);
        assert_eq!(synth.active_voices.len(), 0);
    }

    #[test]
    fn test_synth_polyphony_across_channels() {
        // Tracker semantics: one note per channel, polyphony via multiple channels
        let mut synth = FundspSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(1, 64, 100);
        synth.note_on(2, 67, 100);
        assert_eq!(synth.active_voices.len(), 3);

        synth.note_off_all();
        assert_eq!(synth.active_voices.len(), 0);
    }

    #[test]
    fn test_synth_same_channel_replaces_note() {
        // Playing a new note on the same channel should kill the previous one
        let mut synth = FundspSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        assert_eq!(synth.active_voices.len(), 1);

        synth.note_on(0, 64, 100);
        assert_eq!(synth.active_voices.len(), 1);
        assert_eq!(synth.active_voices[0].1, 64); // new note replaced old
    }

    #[test]
    fn test_synth_note_off_channel() {
        let mut synth = FundspSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(1, 64, 100);
        synth.note_off_all_channel(0);
        assert_eq!(synth.active_voices.len(), 1);
        assert_eq!(synth.active_voices[0].0, 1); // channel 1 still active
    }

    #[test]
    fn test_program_change() {
        let mut synth = FundspSynth::new(44100.0);
        synth.program_change(0, 2);
        assert_eq!(synth.programs[0], 2);
    }

    #[test]
    fn test_make_all_patches() {
        // Ensure all patches create valid voices without panicking
        for prog in 0..8 {
            let patch = Patch::from_program(prog);
            let mut voice = make_voice(patch, 440.0, 0.5);
            voice.set_sample_rate(44100.0);
        }
    }

    #[test]
    fn test_synth_render_output_levels() {
        // Render audio from the sequencer backend and verify output is clean
        let sr = 44100.0;
        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);

        // Trigger a note
        synth.note_on(0, 69, 127); // A4, max velocity

        // Render 4410 samples (100ms) and check levels
        let frames = 4410;
        let mut peak = 0.0_f32;
        let mut has_nonzero = false;
        for _ in 0..frames {
            let mut output = [0f32; 2];
            backend.tick(&[], &mut output);
            for &s in &output {
                if s.abs() > peak {
                    peak = s.abs();
                }
                if s.abs() > 1e-6 {
                    has_nonzero = true;
                }
                // Check for NaN/Inf which would cause distortion
                assert!(s.is_finite(), "Non-finite sample detected: {}", s);
            }
        }

        assert!(has_nonzero, "Synth produced only silence");
        assert!(peak < 1.0, "Peak level {} exceeds 1.0 (clipping)", peak);
        eprintln!("Saw patch peak level at max velocity: {:.4}", peak);
    }

    #[test]
    fn test_note_off_fade_behavior() {
        // Test that note-off actually produces a smooth fade, not an instant cutoff.
        let sr = 44100.0;
        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);

        // Trigger a sine note (cleanest waveform)
        synth.program_change(0, 2); // Sine
        synth.note_on(0, 69, 127);

        // Render 100ms to let the note settle
        for _ in 0..4410 {
            let mut output = [0f32; 2];
            backend.tick(&[], &mut output);
        }

        // Capture pre-note-off level
        let mut pre_off_peak = 0.0_f32;
        for _ in 0..100 {
            let mut output = [0f32; 2];
            backend.tick(&[], &mut output);
            pre_off_peak = pre_off_peak.max(output[0].abs());
        }

        // Send note-off
        synth.note_off(0, 69);

        // Capture first 10ms after note-off (441 samples)
        let mut post_off_samples = Vec::new();
        for _ in 0..441 {
            let mut output = [0f32; 2];
            backend.tick(&[], &mut output);
            post_off_samples.push(output[0]);
        }

        // Analyze envelope: peak over 1ms windows
        let window = 44; // ~1ms at 44100
        let first_window_peak = post_off_samples[..window].iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        let last_window_peak = post_off_samples[post_off_samples.len()-window..].iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));

        eprintln!("Pre-off peak: {:.6}", pre_off_peak);
        eprintln!("First 1ms window peak after note-off: {:.6}", first_window_peak);
        eprintln!("Last 1ms window peak (at 10ms): {:.6}", last_window_peak);

        // If fade works, the first window should have significant signal
        assert!(
            first_window_peak > pre_off_peak * 0.3,
            "Note-off caused instant cutoff! first_window={:.6}, pre_peak={:.6}",
            first_window_peak, pre_off_peak
        );
        // The fade should be decreasing over time
        assert!(
            last_window_peak < first_window_peak,
            "Fade not decreasing: first={:.6}, last={:.6}",
            first_window_peak, last_window_peak
        );
    }

    #[test]
    fn test_voice_direct_render() {
        // All patches produce audio at correct levels (no clipping, no silence)
        let sr = 44100.0;
        for prog in 0..8u8 {
            let patch = Patch::from_program(prog);
            let mut voice = make_voice(patch, 440.0, 1.0);
            voice.set_sample_rate(sr);
            voice.reset();

            let frames = 4410;
            let mut peak = 0.0_f32;
            for _ in 0..frames {
                let mut output = [0f32; 2];
                voice.tick(&[], &mut output);
                for &s in &output {
                    assert!(s.is_finite(), "Patch {:?}: non-finite sample", patch);
                    peak = peak.max(s.abs());
                }
            }
            assert!(peak > 0.01, "Patch {:?} produced near-silence: {:.4}", patch, peak);
            assert!(peak <= 1.0, "Patch {:?} clips: {:.4}", patch, peak);
        }
    }

    #[test]
    fn test_full_pipeline_48khz() {
        // Test at 48000 Hz (actual macOS device rate)
        use crate::audio::effects::EffectsChain;

        let sr = 48000.0;
        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);
        let mut effects = EffectsChain::new(sr);

        synth.note_on(0, 69, 127); // A4

        // Render 0.5 seconds
        let total = 24000;
        let chunk = 512;
        let mut all_left = Vec::with_capacity(total);

        for _ in 0..(total / chunk) {
            let mut left = vec![0f32; chunk];
            let mut right = vec![0f32; chunk];
            for i in 0..chunk {
                let mut output = [0f32; 2];
                backend.tick(&[], &mut output);
                left[i] = output[0];
                right[i] = output[1];
            }
            effects.process(&mut left, &mut right);
            all_left.extend_from_slice(&left);
        }

        let peak = all_left.iter().fold(0f32, |a, &s| a.max(s.abs()));
        let rms = (all_left.iter().map(|&s| (s * s) as f64).sum::<f64>() / all_left.len() as f64).sqrt();

        // Count zero crossings in last quarter (settled)
        let start = all_left.len() * 3 / 4;
        let mut crossings = 0;
        for i in (start + 1)..all_left.len() {
            if all_left[i].signum() != all_left[i - 1].signum() {
                crossings += 1;
            }
        }
        let duration = (all_left.len() - start) as f64 / sr;
        let measured_freq = crossings as f64 / (2.0 * duration);

        eprintln!("48kHz pipeline: peak={:.4}, rms={:.4}, freq={:.1}Hz (expect 440)",
            peak, rms, measured_freq);

        assert!(peak < 1.0, "Clips at 48kHz: {:.4}", peak);
        assert!(rms > 0.01, "Silent at 48kHz: {:.6}", rms);
        assert!((measured_freq - 440.0).abs() < 20.0,
            "Wrong frequency at 48kHz: {:.1} (expected 440)", measured_freq);
    }

    #[test]
    fn test_full_pipeline_waveform() {
        // End-to-end: sequencer backend -> effects -> verify clean output
        use crate::audio::effects::EffectsChain;

        let sr = 44100.0;
        let mut synth = FundspSynth::new(sr);
        let mut backend = synth.backend();
        backend.set_sample_rate(sr);
        let mut effects = EffectsChain::new(sr);

        synth.note_on(0, 60, 127); // C4

        let chunk = 512;
        let total_frames = 44100;
        let mut peak = 0.0_f32;
        let mut rms_sum = 0.0_f64;
        let mut count = 0usize;

        for _ in 0..(total_frames / chunk) {
            let mut left = vec![0f32; chunk];
            let mut right = vec![0f32; chunk];
            for i in 0..chunk {
                let mut output = [0f32; 2];
                backend.tick(&[], &mut output);
                left[i] = output[0];
                right[i] = output[1];
            }
            effects.process(&mut left, &mut right);
            for &s in left.iter().chain(right.iter()) {
                assert!(s.is_finite(), "Non-finite sample in pipeline");
                peak = peak.max(s.abs());
                rms_sum += (s as f64) * (s as f64);
                count += 1;
            }
        }

        let rms = (rms_sum / count as f64).sqrt() as f32;
        assert!(peak < 1.0, "Pipeline clips: {:.4}", peak);
        assert!(rms > 0.01, "Pipeline near-silent: {:.6}", rms);
    }
}
