use std::collections::VecDeque;

const FRAME: usize = 160;

/// Speex echo canceller fed with a loopback reference of the playback device.
pub struct AecProcessor {
    aec: aec_rs::Aec,
    reference: VecDeque<i16>,
    max_ref: usize,
}

impl AecProcessor {
    pub fn new(filter_ms: u32) -> Self {
        let filter_length = ((16_000u64 * filter_ms as u64) / 1000).clamp(FRAME as u64, 16_000) as i32;
        let config = aec_rs::AecConfig {
            frame_size: FRAME,
            filter_length,
            sample_rate: 16_000,
            enable_preprocess: true,
        };
        Self {
            aec: aec_rs::Aec::new(&config),
            reference: VecDeque::with_capacity(16_000),
            max_ref: 16_000 * 2,
        }
    }

    pub fn push_reference(&mut self, samples: &[i16]) {
        self.reference.extend(samples.iter().copied());
        while self.reference.len() > self.max_ref {
            self.reference.pop_front();
        }
    }

    pub fn process(&mut self, mic: &[i16]) -> Vec<i16> {
        let mut out = Vec::with_capacity(mic.len());
        for chunk in mic.chunks(FRAME) {
            let mut m = [0i16; FRAME];
            m[..chunk.len()].copy_from_slice(chunk);
            let mut r = [0i16; FRAME];
            for slot in r.iter_mut() {
                *slot = self.reference.pop_front().unwrap_or(0);
            }
            let mut o = [0i16; FRAME];
            self.aec.cancel_echo(&m, &r, &mut o);
            out.extend_from_slice(&o[..chunk.len()]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: &mut u64) -> i16 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((*seed >> 33) as i32 - (1 << 30)) as i16
    }

    #[test]
    fn cancels_delayed_echo() {
        let mut seed = 42u64;
        let mut reference = vec![0i16; 16_000 * 3];
        for s in reference.iter_mut() {
            *s = noise(&mut seed) / 4;
        }
        let delay = 80usize;
        let gain = 0.5f32;
        let mut mic = vec![0i16; reference.len()];
        for i in delay..reference.len() {
            mic[i] = (reference[i - delay] as f32 * gain) as i16;
        }
        let mut aec = AecProcessor::new(300);
        let mut out = Vec::new();
        for chunk in reference.chunks(320) {
            aec.push_reference(chunk);
        }
        // Reference must be available in the same timeline as the mic; replay it
        // chunk by chunk so the canceller sees aligned frames.
        aec.reference.clear();
        for (i, chunk) in mic.chunks(320).enumerate() {
            let from = (i * 320).saturating_sub(delay).min(reference.len());
            let to = (from + chunk.len()).min(reference.len());
            aec.push_reference(&reference[from..to]);
            out.extend_from_slice(&aec.process(chunk));
        }
        let start = out.len() / 2;
        let echo_rms = rms(&mic[start..]);
        let out_rms = rms(&out[start..]);
        eprintln!("echo rms={echo_rms:.0} out rms={out_rms:.0}");
        assert!(out_rms < echo_rms * 0.5, "echo not reduced enough: {echo_rms} -> {out_rms}");
    }

    fn rms(samples: &[i16]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum / samples.len() as f64).sqrt()
    }
}
