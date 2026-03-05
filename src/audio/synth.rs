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

/// Create a stereo voice (fundsp AudioUnit) for a given patch and frequency
fn make_voice(patch: Patch, freq: f32, velocity: f32) -> Box<dyn AudioUnit> {
    let f = freq;
    let vol = velocity;

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
        // Kill existing voice for same channel+note
        self.note_off(channel, note);

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
                // End the event with a fade-out
                self.sequencer.edit_relative(id, 0.0, 0.05);
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
                self.sequencer.edit_relative(id, 0.0, 0.02);
            } else {
                i += 1;
            }
        }
    }

    pub fn note_off_all(&mut self) {
        for &(_, _, id) in &self.active_voices {
            self.sequencer.edit_relative(id, 0.0, 0.02);
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
    fn test_synth_polyphony() {
        let mut synth = FundspSynth::new(44100.0);
        synth.note_on(0, 60, 100);
        synth.note_on(0, 64, 100);
        synth.note_on(0, 67, 100);
        assert_eq!(synth.active_voices.len(), 3);

        synth.note_off_all();
        assert_eq!(synth.active_voices.len(), 0);
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
}
