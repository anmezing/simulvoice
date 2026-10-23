use anyhow::Result;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;

/// Streaming resampler, one instance per audio stream.
pub struct StreamResampler {
    inner: Option<SincFixedIn<f32>>,
    pending: Vec<f32>,
    from: u32,
    to: u32,
}

impl StreamResampler {
    pub fn new(from: u32, to: u32) -> Self {
        let inner = if from == to {
            None
        } else {
            let params = SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };
            SincFixedIn::<f32>::new(to as f64 / from as f64, 2.0, params, 256, 1).ok()
        };
        Self {
            inner,
            pending: Vec::new(),
            from,
            to,
        }
    }

    pub fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        if self.from == self.to {
            return samples.to_vec();
        }
        self.pending.extend(samples.iter().map(|&s| s as f32 / 32768.0));
        let mut out = Vec::new();
        if let Some(r) = self.inner.as_mut() {
            let need = r.input_frames_next();
            while self.pending.len() >= need {
                let chunk: Vec<f32> = self.pending.drain(..need).collect();
                if let Ok(res) = r.process(&[chunk], None) {
                    out.extend(res[0].iter().map(|&x| to_i16(x)));
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<i16> {
        let mut out = Vec::new();
        if let Some(r) = self.inner.as_mut() {
            let need = r.input_frames_next();
            if !self.pending.is_empty() {
                let mut chunk = std::mem::take(&mut self.pending);
                chunk.resize(need, 0.0);
                if let Ok(res) = r.process(&[chunk], None) {
                    out.extend(res[0].iter().map(|&x| to_i16(x)));
                }
            }
        }
        out
    }
}

/// Resampler specialized to 16 kHz (kept for call sites).
pub struct Resampler16k(StreamResampler);

impl Resampler16k {
    pub fn new(from: u32) -> Self {
        Self(StreamResampler::new(from, 16_000))
    }
    pub fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        self.0.process(samples)
    }
    pub fn finish(&mut self) -> Vec<i16> {
        self.0.finish()
    }
}
fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// One-shot convenience: resample a whole buffer to 16 kHz.
pub fn resample_all_16k(samples: &[i16], from: u32) -> Vec<i16> {
    let mut r = Resampler16k::new(from);
    let mut out = r.process(samples);
    out.extend(r.finish());
    out
}

/// Downmix interleaved audio. For stereo, picks the louder channel per block
/// so out-of-phase channels do not cancel each other.
pub fn downmix(interleaved: &[i16], channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let frames = interleaved.len() / channels;
    let mut out = Vec::with_capacity(frames);

    if channels == 2 {
        let mut e0 = 0u64;
        let mut e1 = 0u64;
        for f in 0..frames {
            let a = interleaved[f * 2] as i64;
            let b = interleaved[f * 2 + 1] as i64;
            e0 += (a * a) as u64;
            e1 += (b * b) as u64;
        }
        let use_left = e0 >= e1;
        for f in 0..frames {
            out.push(interleaved[f * 2 + if use_left { 0 } else { 1 }]);
        }
        return out;
    }

    for f in 0..frames {
        let mut acc: i32 = 0;
        for c in 0..channels {
            acc += interleaved[f * channels + c] as i32;
        }
        out.push((acc / channels as i32) as i16);
    }
    out
}

/// Simple RMS-based automatic gain control (mic path).
pub struct Agc {
    env: f32,
    gain: f32,
    target: f32,
}

impl Agc {
    pub fn new() -> Self {
        Self {
            env: 0.05,
            gain: 1.0,
            target: 0.10,
        }
    }

    pub fn process(&mut self, samples: &mut [i16]) {
        if samples.is_empty() {
            return;
        }
        let mut sum = 0f32;
        for &s in samples.iter() {
            let v = s as f32 / 32768.0;
            sum += v * v;
        }
        let rms = (sum / samples.len() as f32).sqrt().max(1e-6);
        self.env = self.env * 0.9 + rms * 0.1;
        let desired = (self.target / self.env).clamp(0.5, 8.0);
        self.gain = self.gain * 0.9 + desired * 0.1;
        for s in samples.iter_mut() {
            let v = (*s as f32 / 32768.0) * self.gain;
            *s = to_i16(v);
        }
    }
}

/// Small jitter buffer for streamed network audio (e.g. 24 kHz deltas).
pub struct JitterBuffer {
    rate: u32,
    pending: Vec<f32>,
    prefill: usize,
}

impl JitterBuffer {
    pub fn new(rate: u32) -> Self {
        let prefill = (rate as usize / 10).max(1); // 100 ms
        Self {
            rate,
            pending: Vec::new(),
            prefill,
        }
    }

    pub fn push(&mut self, sink: &rodio::Sink, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        if self.pending.len() >= self.prefill {
            let ready = std::mem::take(&mut self.pending);
            sink.append(rodio::buffer::SamplesBuffer::new(1, self.rate, ready));
        }
    }

    pub fn flush(&mut self, sink: &rodio::Sink) {
        if !self.pending.is_empty() {
            let ready = std::mem::take(&mut self.pending);
            sink.append(rodio::buffer::SamplesBuffer::new(1, self.rate, ready));
        }
    }
}

/// Pre-roll ring for VAD so the first syllable is not clipped.
pub struct PreRoll {
    frames: VecDeque<Vec<i16>>,
    cap: usize,
}

impl PreRoll {
    pub fn new(cap: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            cap,
        }
    }
    pub fn push(&mut self, frame: Vec<i16>) {
        self.frames.push_back(frame);
        while self.frames.len() > self.cap {
            self.frames.pop_front();
        }
    }
    pub fn drain(&mut self) -> Vec<Vec<i16>> {
        self.frames.drain(..).collect()
    }
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}



