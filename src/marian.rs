use anyhow::{anyhow, Result};
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const START_ID: i64 = 65000;
const EOS_ID: i64 = 0;
const MAX_NEW: usize = 128;
const LAYERS: usize = 6;

pub struct MarianTranslator {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

unsafe impl Send for MarianTranslator {}

impl MarianTranslator {
    pub fn new(target: &str, override_dir: &str) -> Result<Self> {
        let dir = model_dir(target, override_dir)?;
        let encoder_path = dir.join("encoder_model_int8.onnx");
        let decoder_path = dir.join("decoder_model_int8.onnx");
        let tokenizer_path = dir.join("tokenizer.json");
        for p in [&encoder_path, &decoder_path, &tokenizer_path] {
            if !p.exists() {
                return Err(anyhow!(
                    "Marian 模型不完整: {}（先运行 models marian）",
                    p.display()
                ));
            }
        }
        let env = ort_env()?;
        let encoder = Session::builder(env)?.commit_from_file(&encoder_path)?;
        let decoder = Session::builder(env)?.commit_from_file(&decoder_path)?;
        let patched = dir.join("tokenizer_fixed.json");
        if !patched.exists() {
            let raw = std::fs::read_to_string(&tokenizer_path)?;
            let mut v: serde_json::Value = serde_json::from_str(&raw)?;
            if v.get("normalizer")
                .and_then(|n| n.get("precompiled_charsmap"))
                .map(|m| m.is_null())
                .unwrap_or(false)
            {
                v["normalizer"] = serde_json::Value::Null;
            }
            std::fs::write(&patched, serde_json::to_string(&v)?)?;
        }
        let tokenizer = Tokenizer::from_file(&patched)
            .map_err(|e| anyhow!("加载 tokenizer 失败: {e}"))?;
        Ok(Self {
            encoder,
            decoder,
            tokenizer,
        })
    }

    pub fn translate(&mut self, text: &str) -> Result<String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow!("tokenize 失败: {e}"))?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        if ids.is_empty() {
            return Ok(String::new());
        }
        let n = ids.len();
        let input_ids = Tensor::from_array(([1usize, n], ids.into_boxed_slice()))?;
        let mask = Tensor::from_array(([1usize, n], vec![1i64; n].into_boxed_slice()))?;
        let enc_out = self.encoder.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => mask
        ])?;
        let (shape, data) = enc_out["last_hidden_state"].try_extract_tensor::<f32>()?;
        let hidden_dims: Vec<i64> = shape.iter().map(|&d| d as i64).collect();
        let hidden = data.to_vec();
        drop(enc_out);

        let mut decoder_ids: Vec<i64> = vec![START_ID];
        for _ in 0..MAX_NEW {
            let seq = decoder_ids.len();
            let ids_tensor = Tensor::from_array((
                [1usize, seq],
                decoder_ids.clone().into_boxed_slice(),
            ))?;
            let mask_tensor = Tensor::from_array(([1usize, n], vec![1i64; n].into_boxed_slice()))?;
            let hidden_tensor = Tensor::from_array((
                [1usize, n, 512],
                hidden.clone().into_boxed_slice(),
            ))?;
            let outputs = self.decoder.run(ort::inputs![
                "input_ids" => ids_tensor,
                "encoder_attention_mask" => mask_tensor,
                "encoder_hidden_states" => hidden_tensor
            ])?;
            let (_, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
            let vocab = logits.len() / seq;
            let last = &logits[(seq - 1) * vocab..seq * vocab];
            let mut best = 0usize;
            for (i, &v) in last.iter().enumerate() {
                if v > last[best] {
                    best = i;
                }
            }
            decoder_ids.push(best as i64);
            if best as i64 == EOS_ID {
                break;
            }
            drop(outputs);
        }        let out_ids: Vec<u32> = decoder_ids
            .iter()
            .skip(1)
            .filter(|&&id| id != EOS_ID && id != START_ID)
            .map(|&id| id as u32)
            .collect();
        let text = self
            .tokenizer
            .decode(&out_ids, true)
            .map_err(|e| anyhow!("detokenize 失败: {e}"))?;
        Ok(text.trim().to_string())
    }
}

fn model_dir(target: &str, override_dir: &str) -> Result<PathBuf> {
    let t = target.trim().to_ascii_lowercase();
    let pair = if t.contains("zh") || t.contains("chinese") || target.contains("中") {
        "en-zh"
    } else if t.contains("en") || t.contains("english") || target.contains("英") {
        "zh-en"
    } else {
        return Err(anyhow!("Marian 只支持中英互译，当前目标: {target}"));
    };
    let base = if override_dir.trim().is_empty() {
        crate::config::default_models_dir().join("marian")
    } else {
        PathBuf::from(override_dir.trim())
    };
    Ok(base.join(pair))
}

pub(crate) fn ort_env() -> Result<&'static ort::environment::Environment> {
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(exe.join("onnxruntime_marian.dll"));
    candidates.push(exe.join("onnxruntime.dll"));
    candidates.push(PathBuf::from("onnxruntime_marian.dll"));
    let dll = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| anyhow!("找不到 onnxruntime_marian.dll"))?;
    let env = ort::init_from(&dll)
        .map_err(|e| anyhow!("加载 ORT 失败: {e}"))?
        .build()
        .map_err(|e| anyhow!("初始化 ORT 失败: {e}"))?;
    let _ = ENV.set(env);
    Ok(ENV.get().expect("env just set"))
}





impl crate::translate::Translator for MarianTranslator {
    fn translate(&mut self, text: &str) -> Result<String> {
        MarianTranslator::translate(self, text)
    }
}







