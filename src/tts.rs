use anyhow::{anyhow, Result};

/// Local Windows TTS through System.Speech (offline, no downloads).
pub fn speak_sapi(text: &str, voice: Option<&str>) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("simulvoice-{}.txt", std::process::id()));
    std::fs::write(&tmp, text)?;
    let select = match voice {
        Some(v) if !v.is_empty() => format!(
            "try {{ $s.SelectVoice('{}') }} catch {{}}; ",
            v.replace('\'', "''")
        ),
        _ => String::new(),
    };
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         {select}$t = Get-Content -Raw -Encoding UTF8 '{}'; $s.Speak($t)",
        tmp.display()
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        return Err(anyhow!("SAPI 播报失败 (exit {status})"));
    }
    Ok(())
}

/// OpenAI TTS (online). Returns MP3 bytes.
pub fn openai_tts(text: &str, api_key: &str, model: &str, voice: &str) -> Result<Vec<u8>> {
    let body = serde_json::json!({
        "model": model,
        "voice": voice,
        "input": text,
        "response_format": "mp3"
    });
    let base = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".into());
    let resp = crate::http::agent().post(&format!("{base}/audio/speech"))
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(body);
    match resp {
        Ok(r) => {
            let mut buf = Vec::new();
            r.into_reader().read_to_end(&mut buf)?;
            Ok(buf)
        }
        Err(ureq::Error::Status(code, r)) => Err(anyhow!(
            "openai tts HTTP {code}: {}",
            r.into_string().unwrap_or_default()
        )),
        Err(e) => Err(anyhow!("openai tts 请求失败: {e}")),
    }
}

/// Local neural TTS via sherpa-onnx (Piper VITS, fully offline).
pub struct SherpaTts {
    tts: sherpa_onnx::OfflineTts,
}

impl SherpaTts {
    pub fn new(model_dir: &str) -> Result<Self> {
        use sherpa_onnx::{
            OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
        };
        let dir = std::path::Path::new(model_dir);
        let model_path = if dir.join("model.onnx").exists() {
            dir.join("model.onnx")
        } else {
            std::fs::read_dir(dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| {
                    p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.eq_ignore_ascii_case("onnx"))
                        .unwrap_or(false)
                })
                .ok_or_else(|| anyhow!("目录中没有 .onnx 模型: {}", dir.display()))?
        };
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: Some(model_path.display().to_string()),
                    tokens: Some(dir.join("tokens.txt").display().to_string()),
                    data_dir: Some(dir.join("espeak-ng-data").display().to_string()),
                    lexicon: {
                        let p = dir.join("lexicon.txt");
                        if p.exists() {
                            Some(p.display().to_string())
                        } else {
                            None
                        }
                    },
                    ..Default::default()
                },
                num_threads: 2,
                debug: false,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            max_num_sentences: 1,
            silence_scale: 0.2,
            ..Default::default()
        };
        let tts = OfflineTts::create(&config)
            .ok_or_else(|| anyhow!("Piper 模型加载失败: {}", dir.display()))?;
        Ok(Self { tts })
    }

    pub fn render(&self, text: &str, speed: f32) -> Result<(Vec<f32>, u32)> {
        let mut cfg = sherpa_onnx::GenerationConfig::default();
        cfg.speed = speed;
        let audio = self
            .tts
            .generate_with_config::<fn(&[f32], f32) -> bool>(text, &cfg, None)
            .ok_or_else(|| anyhow!("Piper 合成失败"))?;
        Ok((audio.samples().to_vec(), audio.sample_rate() as u32))
    }

    pub fn speak(&self, text: &str, speed: f32, device: Option<&str>) -> Result<()> {
        let (samples, rate) = self.render(text, speed)?;
        crate::sink::play_pcm(&samples, rate, device)
    }
}



/// Split long text into speakable clauses (punctuation aware).
pub fn split_clauses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        cur.push(ch);
        count += 1;
        let hard = "。！？!?；;\n".contains(ch);
        let soft = "，,、".contains(ch);
        if (hard && count >= 2) || (soft && count >= 40) || count >= 80 {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
            count = 0;
        }
    }
    let s = cur.trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
    out
}



