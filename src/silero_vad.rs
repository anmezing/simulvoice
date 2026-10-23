use anyhow::{anyhow, Result};

use crate::dsp::PreRoll;
use crate::vad::VadEvent;

/// Silero VAD v5 through sherpa-onnx (no separate ONNX Runtime).
pub struct SileroVad {
    vad: sherpa_onnx::VoiceActivityDetector,
    speaking: bool,
    fed_samples: usize,
    max_samples: usize,
    preroll: PreRoll,
}

impl SileroVad {
    pub fn new(model_path: &str, silence_ms: u64, max_utterance_ms: u64) -> Result<Self> {
        use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model_path.to_string()),
                threshold: 0.5,
                min_silence_duration: silence_ms as f32 / 1000.0,
                min_speech_duration: 0.15,
                window_size: 512,
                max_speech_duration: max_utterance_ms as f32 / 1000.0,
            },
            sample_rate: 16_000,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&config, 30.0)
            .ok_or_else(|| anyhow!("Silero VAD 模型加载失败: {model_path}"))?;
        Ok(Self {
            vad,
            speaking: false,
            fed_samples: 0,
            max_samples: (max_utterance_ms as usize * 16) as usize,
            preroll: PreRoll::new(8),
        })
    }

    pub fn push(&mut self, samples: &[i16]) -> Vec<VadEvent> {
        let mut events = Vec::new();
        let f32s: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        self.vad.accept_waveform(&f32s);

        if !self.speaking && self.vad.detected() {
            self.speaking = true;
            self.fed_samples = 0;
            events.push(VadEvent::Start);
            for frame in self.preroll.drain() {
                self.fed_samples += frame.len();
                events.push(VadEvent::Feed(frame));
            }
        }
        if self.speaking {
            self.preroll.push(samples.to_vec());
            events.push(VadEvent::Feed(samples.to_vec()));
            self.fed_samples += samples.len();
        } else {
            self.preroll.push(samples.to_vec());
        }
        while self.vad.front().is_some() {
            self.vad.pop();
            if self.speaking {
                events.push(VadEvent::End);
                self.speaking = false;
                self.fed_samples = 0;
                self.preroll.clear();
            }
        }
        if self.speaking && self.fed_samples >= self.max_samples {
            self.vad.reset();
            events.push(VadEvent::End);
            self.speaking = false;
            self.fed_samples = 0;
            self.preroll.clear();
        }
        events
    }

    pub fn last_prob(&self) -> f32 {
        if self.vad.detected() {
            1.0
        } else {
            0.0
        }
    }
}


