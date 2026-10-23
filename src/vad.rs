/// VAD engine selected by configuration.
pub enum VadEngine {
    Energy(Vad),
    Silero(crate::silero_vad::SileroVad),
}

impl VadEngine {
    pub fn push(&mut self, samples: &[i16]) -> Vec<VadEvent> {
        match self {
            VadEngine::Energy(v) => v.push(samples),
            VadEngine::Silero(v) => v.push(samples),
        }
    }
}

/// Lightweight energy VAD that groups 16 kHz mono PCM into utterances.
pub struct Vad {
    rms_threshold: f64,
    silence_samples: usize,
    max_samples: usize,
    voiced: bool,
    silence_run: usize,
    utterance: Vec<i16>,
    noise_floor: f64,
}

pub enum VadEvent {
    /// Speech started.
    Start,
    /// Some audio is ready to feed to the recognizer.
    Feed(Vec<i16>),
    /// The utterance ended and should be recognized/translated.
    End,
}

impl Vad {
    pub fn new(rms_threshold: f64, silence_ms: u64, max_utterance_ms: u64) -> Self {
        Self {
            rms_threshold,
            silence_samples: (silence_ms as usize * 16) as usize,
            max_samples: (max_utterance_ms as usize * 16) as usize,
            voiced: false,
            silence_run: 0,
            utterance: Vec::new(),
            noise_floor: 50.0,
        }
    }

    pub fn push(&mut self, samples: &[i16]) -> Vec<VadEvent> {
        let mut events = Vec::new();
        if samples.is_empty() {
            return events;
        }
        let rms = {
            let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
            (sum / samples.len() as f64).sqrt()
        };
        if !self.voiced {
            self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
        }
        let threshold = self.rms_threshold.max(self.noise_floor * 3.0);
        let loud = rms >= threshold;

        if loud {
            if !self.voiced {
                self.voiced = true;
                self.silence_run = 0;
                events.push(VadEvent::Start);
                // keep a little pre-roll (already buffered below)
            }
            self.silence_run = 0;
            self.utterance.extend_from_slice(samples);
            events.push(VadEvent::Feed(samples.to_vec()));
        } else if self.voiced {
            self.silence_run += samples.len();
            // Still feed trailing silence so recognizers can finalize cleanly.
            self.utterance.extend_from_slice(samples);
            events.push(VadEvent::Feed(samples.to_vec()));
            if self.silence_run >= self.silence_samples {
                events.push(VadEvent::End);
                self.voiced = false;
                self.silence_run = 0;
                self.utterance.clear();
            }
        }

        if self.voiced && self.utterance.len() >= self.max_samples {
            events.push(VadEvent::End);
            self.voiced = false;
            self.silence_run = 0;
            self.utterance.clear();
        }
        events
    }
}





