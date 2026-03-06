use rusty_link::{AblLink, SessionState};

/// Quantum: beats per phase cycle (typically 4 = one bar in 4/4 time)
const DEFAULT_QUANTUM: f64 = 4.0;

pub struct LinkEngine {
    link: AblLink,
    session_state: SessionState,
    pub quantum: f64,
    /// Last known tempo from Link, used to detect peer tempo changes
    last_tempo: f64,
}

impl LinkEngine {
    pub fn new(bpm: f64) -> Self {
        let link = AblLink::new(bpm);
        let session_state = SessionState::new();

        Self {
            link,
            session_state,
            quantum: DEFAULT_QUANTUM,
            last_tempo: bpm,
        }
    }

    pub fn enable(&self) {
        self.link.enable(true);
        self.link.enable_start_stop_sync(true);
    }

    pub fn disable(&self) {
        self.link.enable_start_stop_sync(false);
        self.link.enable(false);
    }

    pub fn is_enabled(&self) -> bool {
        self.link.is_enabled()
    }

    pub fn num_peers(&self) -> u64 {
        self.link.num_peers()
    }

    /// Capture the current session state and return tempo, beat, phase, and playing status.
    #[allow(dead_code)]
    pub fn capture(&mut self) -> LinkState {
        self.link
            .capture_app_session_state(&mut self.session_state);
        let time = self.link.clock_micros();
        let tempo = self.session_state.tempo();
        let beat = self.session_state.beat_at_time(time, self.quantum);
        let phase = self.session_state.phase_at_time(time, self.quantum);
        let is_playing = self.session_state.is_playing();

        LinkState {
            tempo,
            beat,
            phase,
            is_playing,
        }
    }

    /// Poll for tempo changes from Link peers by comparing against last known value.
    /// Returns Some(new_tempo) if changed, None otherwise.
    pub fn poll_tempo_change(&mut self) -> Option<f64> {
        self.link
            .capture_app_session_state(&mut self.session_state);
        let tempo = self.session_state.tempo();
        if (tempo - self.last_tempo).abs() > 0.01 {
            self.last_tempo = tempo;
            Some(tempo)
        } else {
            None
        }
    }

    /// Set tempo from our side and commit to the session
    pub fn set_tempo(&mut self, bpm: f64) {
        let time = self.link.clock_micros();
        self.link
            .capture_app_session_state(&mut self.session_state);
        self.session_state.set_tempo(bpm, time);
        self.link.commit_app_session_state(&self.session_state);
        self.last_tempo = bpm;
    }

    /// Request transport play, quantized to the next bar boundary
    pub fn request_play(&mut self) {
        let time = self.link.clock_micros();
        self.link
            .capture_app_session_state(&mut self.session_state);
        self.session_state.set_is_playing(true, time);
        self.session_state
            .request_beat_at_start_playing_time(0.0, self.quantum);
        self.link.commit_app_session_state(&self.session_state);
    }

    /// Request transport stop
    pub fn request_stop(&mut self) {
        let time = self.link.clock_micros();
        self.link
            .capture_app_session_state(&mut self.session_state);
        self.session_state.set_is_playing(false, time);
        self.link.commit_app_session_state(&self.session_state);
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct LinkState {
    pub tempo: f64,
    pub beat: f64,
    pub phase: f64,
    pub is_playing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_engine_new() {
        let engine = LinkEngine::new(120.0);
        assert!(!engine.is_enabled());
        assert_eq!(engine.num_peers(), 0);
        assert_eq!(engine.quantum, DEFAULT_QUANTUM);
    }

    #[test]
    fn test_link_enable_disable() {
        let engine = LinkEngine::new(120.0);
        engine.enable();
        assert!(engine.is_enabled());
        engine.disable();
        assert!(!engine.is_enabled());
    }

    #[test]
    fn test_link_capture_returns_state() {
        let mut engine = LinkEngine::new(130.0);
        engine.enable();
        let state = engine.capture();
        assert!((state.tempo - 130.0).abs() < 1.0);
        engine.disable();
    }

    #[test]
    fn test_link_set_tempo() {
        let mut engine = LinkEngine::new(120.0);
        engine.enable();
        engine.set_tempo(140.0);
        let state = engine.capture();
        assert!((state.tempo - 140.0).abs() < 1.0);
        engine.disable();
    }

    #[test]
    fn test_link_play_stop() {
        let mut engine = LinkEngine::new(120.0);
        engine.enable();

        engine.request_play();
        let state = engine.capture();
        assert!(state.is_playing);

        // Other Link test threads may interfere with state. Re-issue stop
        // and poll until the state converges.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            engine.request_stop();
            std::thread::sleep(std::time::Duration::from_millis(20));
            let state = engine.capture();
            if !state.is_playing {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("Link did not stop within 2s");
            }
        }

        engine.disable();
    }

    #[test]
    fn test_poll_tempo_no_change() {
        let mut engine = LinkEngine::new(120.0);
        engine.enable();
        // No external change, should return None
        assert!(engine.poll_tempo_change().is_none());
        engine.disable();
    }

    #[test]
    fn test_poll_tempo_after_set() {
        let mut engine = LinkEngine::new(120.0);
        engine.enable();
        engine.set_tempo(140.0);
        // We just set it ourselves, last_tempo is updated, so no "change" detected
        assert!(engine.poll_tempo_change().is_none());
        engine.disable();
    }
}
