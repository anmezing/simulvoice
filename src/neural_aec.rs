use anyhow::{anyhow, Result};
use ort::session::Session;
use ort::value::Tensor;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::sync::Arc;

const FRAME: usize = 320;
const BINS: usize = 161;
const STATE: usize = 322;

pub struct NeuralAec {
    session: Session,
    h1: Vec<f32>,
    h2: Vec<f32>,
    window: Vec<f32>,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    mic: VecDeque<i16>,
    far: VecDeque<i16>,
    out: Vec<i16>,
}

impl NeuralAec {
    pub fn new(model_path: &str, delay_ms: u32) -> Result<Self> {
        if !std::path::Path::new(model_path).exists() {
            return Err(anyhow!(
                "神经 AEC 模型不存在: {model_path}（运行 models aec）"
            ));
        }
        let env = crate::marian::ort_env()?;
        let session = Session::builder(env)?.commit_from_file(model_path)?;
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FRAME);
        let inv = planner.plan_fft_inverse(FRAME);
        let window = (0..FRAME)
            .map(|i| {
                let h = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / FRAME as f32).cos();
                h.sqrt()
            })
            .collect();
        let delay_frames = (delay_ms as usize / 20).min(100);
        let mut far = VecDeque::new();
        far.extend(std::iter::repeat(0i16).take(delay_frames * FRAME));
        Ok(Self {
            session,
            h1: vec![0.0; STATE],
            h2: vec![0.0; STATE],
            window,
            fwd,
            inv,
            mic: VecDeque::new(),
            far,
            out: Vec::new(),
        })
    }

    pub fn push_reference(&mut self, samples: &[i16]) {
        self.far.extend(samples.iter().copied());
        while self.far.len() > 16_000 * 3 {
            self.far.pop_front();
        }
    }

    pub fn process(&mut self, mic: &[i16]) -> Vec<i16> {
        self.mic.extend(mic.iter().copied());
        while self.mic.len() >= FRAME && self.far.len() >= FRAME {
            let m: Vec<i16> = self.mic.drain(..FRAME).collect();
            let r: Vec<i16> = self.far.drain(..FRAME).collect();
            match self.enhance(&m, &r) {
                Ok(frame) => self.out.extend_from_slice(&frame),
                Err(e) => {
                    eprintln!("[neural-aec] {e}");
                    self.out.extend_from_slice(&m);
                }
            }
        }
        while self.mic.len() > 16_000 * 3 {
            self.mic.pop_front();
        }
        std::mem::take(&mut self.out)
    }

    fn enhance(&mut self, mic: &[i16], far: &[i16]) -> Result<Vec<i16>> {
        let mut mic_spec = self.fwd.make_output_vec();
        let mut far_spec = self.fwd.make_output_vec();
        let mut mic_frame: Vec<f32> = mic
            .iter()
            .zip(&self.window)
            .map(|(&s, &w)| s as f32 / 32768.0 * w)
            .collect();
        let mut far_frame: Vec<f32> = far
            .iter()
            .zip(&self.window)
            .map(|(&s, &w)| s as f32 / 32768.0 * w)
            .collect();
        self.fwd
            .process(&mut mic_frame, &mut mic_spec)
            .map_err(|e| anyhow!("FFT: {e}"))?;
        self.fwd
            .process(&mut far_frame, &mut far_spec)
            .map_err(|e| anyhow!("FFT: {e}"))?;

        let mut feat = Vec::with_capacity(STATE);
        for spec in [&mic_spec, &far_spec] {
            for c in spec.iter() {
                let p = (c.re * c.re + c.im * c.im).max(1e-12);
                feat.push(p.log10() / 20.0);
            }
        }
        let input = Tensor::from_array((
            [1usize, 1, STATE],
            feat.into_boxed_slice(),
        ))?;
        let h1 = Tensor::from_array((
            [1usize, 1, STATE],
            self.h1.clone().into_boxed_slice(),
        ))?;
        let h2 = Tensor::from_array((
            [1usize, 1, STATE],
            self.h2.clone().into_boxed_slice(),
        ))?;
        let outputs = self.session.run(ort::inputs![
            "input" => input,
            "h01" => h1,
            "h02" => h2
        ])?;
        let (_, mask) = outputs["output"].try_extract_tensor::<f32>()?;
        if mask.len() < BINS {
            return Err(anyhow!("AEC mask 长度异常: {}", mask.len()));
        }
        for (dst, src) in [(&mut self.h1, "hn1"), (&mut self.h2, "hn2")] {
            if let Some(v) = outputs.get(src) {
                let (_, data) = v.try_extract_tensor::<f32>()?;
                if data.len() == STATE {
                    dst.copy_from_slice(data);
                }
            }
        }
        let mut enhanced: Vec<Complex32> = mic_spec
            .iter()
            .zip(mask.iter())
            .map(|(c, &m)| Complex32::new(c.re * m, c.im * m))
            .collect();
        let mut time = self.inv.make_output_vec();
        self.inv
            .process(&mut enhanced, &mut time)
            .map_err(|e| anyhow!("IFFT: {e}"))?;
        let mut pcm = Vec::with_capacity(FRAME);
        for (i, &s) in time.iter().enumerate() {
            let v = s / FRAME as f32 * self.window[i];
            pcm.push((v.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        Ok(pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_on_silence() {
        let path = crate::config::default_models_dir()
            .join("aec")
            .join("dec-baseline.onnx");
        if !path.exists() {
            return;
        }
        let mut aec = NeuralAec::new(&path.display().to_string(), 0).unwrap();
        let mic = vec![0i16; 640];
        let far = vec![0i16; 640];
        aec.push_reference(&far);
        let out = aec.process(&mic);
        assert_eq!(out.len(), 640);
    }
}
