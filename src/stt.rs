use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

pub enum SttEngine {
    Soniox(SonioxStt),
    OpenAi(OpenAiStt),
    Vosk(VoskStt),
    Sherpa(SherpaStt),
    Zipformer(ZipformerStt),
    None,
}

impl SttEngine {
    pub fn begin_utterance(&mut self) {
        match self {
            SttEngine::OpenAi(s) => s.buffer.clear(),
            SttEngine::Sherpa(s) => s.buffer.clear(),
            SttEngine::Zipformer(s) => s.begin(),
            _ => {}
        }
    }

    pub fn feed(&mut self, pcm: &[i16]) -> Result<()> {
        match self {
            SttEngine::Soniox(s) => s.feed(pcm),
            SttEngine::OpenAi(s) => {
                s.buffer.extend_from_slice(pcm);
                Ok(())
            }
            SttEngine::Vosk(s) => s.feed(pcm),
            SttEngine::Sherpa(s) => {
                s.buffer.extend_from_slice(pcm);
                Ok(())
            }
            SttEngine::Zipformer(s) => s.feed(pcm),
            SttEngine::None => Ok(()),
        }
    }

    /// Final texts produced so far by streaming engines.
    pub fn poll_final(&mut self) -> Vec<String> {
        match self {
            SttEngine::Soniox(s) => s.poll(),
            SttEngine::Vosk(s) => s.poll(),
            _ => Vec::new(),
        }
    }

    pub fn flush(&mut self) -> Option<String> {
        match self {
            SttEngine::Vosk(s) => s.flush(),
            _ => None,
        }
    }

    pub fn partial(&mut self) -> Option<String> {
        match self {
            SttEngine::Vosk(s) => s.partial(),
            SttEngine::Soniox(s) => s.partial(),
            SttEngine::Zipformer(s) => s.partial(),
            _ => None,
        }
    }

    /// Called when VAD says the utterance ended. Batch engines transcribe here.
    pub fn end_utterance(&mut self) -> Result<Option<String>> {
        match self {
            SttEngine::OpenAi(s) => {
                if s.buffer.is_empty() {
                    return Ok(None);
                }
                let audio = std::mem::take(&mut s.buffer);
                let text = s.transcribe(&audio)?;
                Ok(if text.trim().is_empty() { None } else { Some(text) })
            }
            SttEngine::Sherpa(s) => {
                if s.buffer.is_empty() {
                    return Ok(None);
                }
                let text = s.transcribe()?;
                Ok(if text.trim().is_empty() { None } else { Some(text) })
            }
            SttEngine::Zipformer(s) => s.end_utterance(),
            _ => Ok(None),
        }
    }
}

// ───────────────────────────── Soniox (cloud streaming) ─────────────────────────────

/// 离线整句识别（SenseVoice / Whisper）：VAD 切出一句后一次性解码。
pub struct SherpaStt {
    recognizer: sherpa_onnx::OfflineRecognizer,
    buffer: Vec<i16>,
    /// Whisper 在静音/噪声上会"脑补"固定套话，需要过滤
    whisper: bool,
}

fn onnx_provider() -> String {
    crate::ONNX_PROVIDER
        .get()
        .cloned()
        .unwrap_or_else(|| "cpu".into())
}

/// 识别线程数：两路同传各占一份，留出核给翻译与 VAD。
fn asr_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 4).clamp(2, 4) as i32)
        .unwrap_or(2)
}

pub fn sensevoice_dir() -> std::path::PathBuf {
    crate::config::default_models_dir().join("sensevoice")
}

pub fn whisper_dir() -> std::path::PathBuf {
    crate::config::default_models_dir().join("whisper")
}

pub fn sensevoice_ready() -> bool {
    let d = sensevoice_dir();
    d.join("tokens.txt").exists()
        && (d.join("model.int8.onnx").exists() || d.join("model.onnx").exists())
}

/// Whisper 目录里的 (encoder, decoder, tokens)，优先 int8。
fn whisper_files(
    dir: &std::path::Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)> {
    let names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    let pick = |part: &str| -> Option<std::path::PathBuf> {
        let mut c: Vec<&String> = names
            .iter()
            .filter(|n| n.ends_with(".onnx") && n.contains(&format!("-{part}")))
            .collect();
        c.sort_by_key(|n| if n.contains(".int8.") { 0 } else { 1 });
        c.first().map(|n| dir.join(n))
    };
    let tokens = names
        .iter()
        .find(|n| n.ends_with("tokens.txt"))
        .map(|n| dir.join(n))?;
    Some((pick("encoder")?, pick("decoder")?, tokens))
}

pub fn whisper_ready() -> bool {
    whisper_files(&whisper_dir()).is_some()
}

/// Whisper 在非语音片段上常见的幻觉输出（训练数据里的字幕套话）。
fn whisper_hallucination(text: &str) -> bool {
    let t = text
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || "。，！？…「」 ".contains(c))
        .to_lowercase();
    const JUNK: &[&str] = &[
        "thank you",
        "thanks for watching",
        "thank you for watching",
        "please subscribe",
        "you",
        "bye",
        "谢谢观看",
        "ご視聴ありがとうございました",
    ];
    t.is_empty()
        || JUNK.iter().any(|j| t == *j)
        || (t.chars().count() < 60 && (t.contains("amara.org") || t.contains("字幕") || t.contains("sous-titr") || t.contains("untertitel")))
}

impl SherpaStt {
    pub fn whisper(model_dir: &str, language: &str) -> Result<Self> {
        use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig};
        let dir = std::path::Path::new(model_dir);
        let (encoder, decoder, tokens) = whisper_files(dir).ok_or_else(|| {
            anyhow!("Whisper 模型不完整: {}（先运行 simulvoice models whisper）", dir.display())
        })?;
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.whisper = OfflineWhisperModelConfig {
            encoder: Some(encoder.display().to_string()),
            decoder: Some(decoder.display().to_string()),
            // 空 = Whisper 自动判语种；会议里语种已知，指定后更准也更快
            language: Some(language.trim().to_string()),
            task: Some("transcribe".into()),
            ..Default::default()
        };
        config.model_config.tokens = Some(tokens.display().to_string());
        config.model_config.num_threads = asr_threads();
        config.model_config.provider = Some(onnx_provider());
        config.model_config.debug = false;
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("Whisper 模型加载失败: {}", dir.display()))?;
        Ok(Self {
            recognizer,
            buffer: Vec::new(),
            whisper: true,
        })
    }

    pub fn new(model_dir: &str, language: &str) -> Result<Self> {
        use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
        let dir = std::path::Path::new(model_dir);
        let model = ["model.int8.onnx", "model.onnx"]
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.exists())
            .ok_or_else(|| anyhow!("SenseVoice 模型不存在: {}", dir.display()))?;
        let tokens = dir.join("tokens.txt");
        if !tokens.exists() {
            return Err(anyhow!("SenseVoice tokens.txt 不存在: {}", dir.display()));
        }
        let lang = match crate::lang::find(language) {
            Some(l) if l.sensevoice => l.code.to_string(),
            Some(l) => {
                return Err(anyhow!(
                    "SenseVoice 不支持{}（只支持中/英/日/韩/粤），改用 stt=local 自动切 Whisper",
                    l.label
                ))
            }
            None => "auto".to_string(),
        };
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(model.display().to_string()),
            language: Some(lang),
            use_itn: true,
        };
        config.model_config.tokens = Some(tokens.display().to_string());
        config.model_config.num_threads = asr_threads();
        config.model_config.provider = Some(onnx_provider());
        config.model_config.debug = false;
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("SenseVoice 模型加载失败: {}", dir.display()))?;
        Ok(Self {
            recognizer,
            buffer: Vec::new(),
            whisper: false,
        })
    }

    fn transcribe(&mut self) -> Result<String> {
        let samples: Vec<f32> = self
            .buffer
            .iter()
            .map(|&s| s as f32 / 32768.0)
            .collect();
        self.buffer.clear();
        // Whisper 对 <0.3 秒的碎片几乎只会输出幻觉
        if self.whisper && samples.len() < 16_000 * 3 / 10 {
            return Ok(String::new());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16_000, &samples);
        self.recognizer.decode(&stream);
        let text = stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default();
        if self.whisper && whisper_hallucination(&text) {
            return Ok(String::new());
        }
        Ok(text)
    }
}

pub struct ZipformerStt {
    recognizer: sherpa_onnx::OnlineRecognizer,
    stream: sherpa_onnx::OnlineStream,
}

impl ZipformerStt {
    pub fn new(model_dir: &str) -> Result<Self> {
        use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineTransducerModelConfig};
        let dir = std::path::Path::new(model_dir);
        let pick = |prefix: &str| -> Option<std::path::PathBuf> {
            let mut candidates: Vec<_> = std::fs::read_dir(dir)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()) == Some("onnx")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with(prefix))
                            .unwrap_or(false)
                })
                .collect();
            candidates.sort_by_key(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.contains(".int8.") { 0 } else { 1 }
            });
            candidates.into_iter().next()
        };
        let encoder = pick("encoder").ok_or_else(|| anyhow!("Zipformer encoder 不存在: {}", dir.display()))?;
        let decoder = pick("decoder").ok_or_else(|| anyhow!("Zipformer decoder 不存在: {}", dir.display()))?;
        let joiner = pick("joiner").ok_or_else(|| anyhow!("Zipformer joiner 不存在: {}", dir.display()))?;
        let tokens = dir.join("tokens.txt");
        if !tokens.exists() {
            return Err(anyhow!("Zipformer tokens.txt 不存在: {}", dir.display()));
        }
        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer = OnlineTransducerModelConfig {
            encoder: Some(encoder.display().to_string()),
            decoder: Some(decoder.display().to_string()),
            joiner: Some(joiner.display().to_string()),
        };
        config.model_config.tokens = Some(tokens.display().to_string());
        config.model_config.num_threads = 2;
        config.model_config.provider = Some(
            crate::ONNX_PROVIDER
                .get()
                .cloned()
                .unwrap_or_else(|| "cpu".into()),
        );
        config.model_config.debug = false;
        config.decoding_method = Some("greedy_search".into());
        config.enable_endpoint = false;
        let recognizer = OnlineRecognizer::create(&config)
            .ok_or_else(|| anyhow!("Zipformer 模型加载失败: {}", dir.display()))?;
        let stream = recognizer.create_stream();
        Ok(Self { recognizer, stream })
    }

    fn begin(&mut self) {
        self.recognizer.reset(&self.stream);
    }

    fn feed(&mut self, pcm: &[i16]) -> Result<()> {
        let samples: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
        self.stream.accept_waveform(16_000, &samples);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        Ok(())
    }

    fn partial(&mut self) -> Option<String> {
        self.recognizer
            .get_result(&self.stream)
            .map(|r| r.text.trim().to_string())
            .filter(|t| !t.is_empty())
    }

    fn end_utterance(&mut self) -> Result<Option<String>> {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        let text = self
            .recognizer
            .get_result(&self.stream)
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default();
        self.recognizer.reset(&self.stream);
        Ok(if text.is_empty() { None } else { Some(text) })
    }
}

pub struct SonioxStt {
    audio_tx: Sender<Vec<i16>>,
    text_rx: Receiver<String>,
    partial: Arc<Mutex<String>>,
}

impl SonioxStt {
    pub fn new(api_key: String, language: String) -> Result<Self> {
        let (audio_tx, audio_rx) = channel::<Vec<i16>>();
        let (text_tx, text_rx) = channel::<String>();
        let partial = Arc::new(Mutex::new(String::new()));
        let partial_worker = partial.clone();

        std::thread::spawn(move || {
            if let Err(e) = soniox_worker(api_key, language, audio_rx, text_tx.clone(), partial_worker) {
                let _ = text_tx.send(String::new());
                eprintln!("[soniox] {e}");
            }
        });

        Ok(Self {
            audio_tx,
            text_rx,
            partial,
        })
    }

    pub fn feed(&mut self, pcm: &[i16]) -> Result<()> {
        let _ = self.audio_tx.send(pcm.to_vec());
        Ok(())
    }

    pub fn poll(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(t) = self.text_rx.try_recv() {
            if !t.trim().is_empty() {
                out.push(t);
            }
        }
        out
    }

    pub fn partial(&self) -> Option<String> {
        let p = self.partial.lock().ok()?;
        if p.trim().is_empty() {
            None
        } else {
            Some(p.clone())
        }
    }
}

fn soniox_worker(
    api_key: String,
    language: String,
    audio_rx: Receiver<Vec<i16>>,
    text_tx: Sender<String>,
    partial: Arc<Mutex<String>>,
) -> Result<()> {
    loop {
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        match soniox_once(&api_key, &language, &audio_rx, &text_tx, &partial) {
            Ok(()) => {}
            Err(e) => {
                let msg = e.to_string();
                eprintln!("[soniox] {msg}");
                if msg.contains("FATAL") {
                    return Ok(());
                }
            }
        }
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        eprintln!("[soniox] 断线，重连中...");
        std::thread::sleep(Duration::from_millis(800));
    }
}

fn soniox_once(
    api_key: &str,
    language: &str,
    audio_rx: &Receiver<Vec<i16>>,
    text_tx: &Sender<String>,
    partial: &Arc<Mutex<String>>,
) -> Result<()> {
    let (mut ws, _) = connect("wss://stt-rt.soniox.com/transcribe-websocket")?;
    if let MaybeTlsStream::Rustls(s) = ws.get_mut() {
        let _ = s.sock.set_read_timeout(Some(Duration::from_millis(30)));
    }

    let init = serde_json::json!({
        "api_key": api_key,
        "model": "stt-rt-v5",
        "audio_format": "pcm_s16le",
        "sample_rate": 16000,
        "num_channels": 1,
        "enable_endpoint_detection": true,
        "language_hints": if language == "auto" {
            serde_json::json!([])
        } else {
            serde_json::json!(language.split(',').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>())
        }
    });
    ws.send(Message::Text(init.to_string().into()))?;

    let mut final_text = String::new();
    loop {
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        while let Ok(chunk) = audio_rx.try_recv() {
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for s in chunk {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            if ws.send(Message::Binary(bytes.into())).is_err() {
                return Ok(());
            }
        }

        match ws.read() {
            Ok(Message::Text(t)) => {
                let v: Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(err) = v.get("error_code").and_then(|e| e.as_str()) {
                    let fatal = err.starts_with("401") || err.starts_with("403");
                    return Err(anyhow!("{}soniox error: {err}", if fatal { "FATAL: " } else { "" }));
                }
                if let Some(tokens) = v.get("tokens").and_then(|t| t.as_array()) {
                    let mut partial_now = String::new();
                    for tok in tokens {
                        let text = tok.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        let is_final = tok.get("is_final").and_then(|b| b.as_bool()).unwrap_or(false);
                        if text == "<end>" {
                            if !final_text.trim().is_empty() {
                                let _ = text_tx.send(std::mem::take(&mut final_text));
                            }
                            if let Ok(mut p) = partial.lock() {
                                p.clear();
                            }
                        } else if is_final {
                            final_text.push_str(text);
                        } else {
                            partial_now.push_str(text);
                        }
                    }
                    if let Ok(mut p) = partial.lock() {
                        *p = partial_now;
                    }
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if audio_rx.try_recv().is_err() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Err(_) => return Ok(()),
        }
    }
}
// ───────────────────────────── OpenAI (HTTPS batch) ─────────────────────────────

pub struct OpenAiStt {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    buffer: Vec<i16>,
}

impl OpenAiStt {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            buffer: Vec::new(),
        }
    }

    fn transcribe(&mut self, pcm: &[i16]) -> Result<String> {
        let mut wav = Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::new(&mut wav, spec)?;
            for &s in pcm {
                w.write_sample(s)?;
            }
            w.finalize()?;
        }
        let wav = wav.into_inner();
        let boundary = "----simulvoice7d91f0aa";
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{}\r\n",
                self.model
            )
            .as_bytes(),
        );
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&wav);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let resp = crate::http::agent().post(&format!("{}/audio/transcriptions", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body);
        match resp {
            Ok(r) => {
                let v: Value = r.into_json()?;
                Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
            }
            Err(ureq::Error::Status(code, r)) => Err(anyhow!(
                "stt HTTP {code}: {}",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(anyhow!("stt 请求失败: {e}")),
        }
    }
}

// ───────────────────────────── Vosk (local, runtime-loaded DLL) ─────────────────────────────

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

struct VoskFns {
    model_new: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    model_free: unsafe extern "C" fn(*mut c_void),
    rec_new: unsafe extern "C" fn(*mut c_void, f32) -> *mut c_void,
    rec_free: unsafe extern "C" fn(*mut c_void),
    accept: unsafe extern "C" fn(*mut c_void, *const i16, usize) -> i32,
    result: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    final_result: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    partial_result: unsafe extern "C" fn(*mut c_void) -> *const c_char,
}

pub struct VoskStt {
    _lib: libloading::Library,
    fns: VoskFns,
    model: *mut c_void,
    rec: *mut c_void,
    finals: VecDeque<String>,
    partial_text: String,
}

unsafe impl Send for VoskStt {}

impl VoskStt {
    pub fn new(model_path: &str) -> Result<Self> {
        let lib = load_vosk_library()?;
        unsafe {
            let fns = VoskFns {
                model_new: *lib.get(b"vosk_model_new\0")?,
                model_free: *lib.get(b"vosk_model_free\0")?,
                rec_new: *lib.get(b"vosk_recognizer_new\0")?,
                rec_free: *lib.get(b"vosk_recognizer_free\0")?,
                accept: *lib.get(b"vosk_recognizer_accept_waveform\0")?,
                result: *lib.get(b"vosk_recognizer_result\0")?,
                final_result: *lib.get(b"vosk_recognizer_final_result\0")?,
                partial_result: *lib.get(b"vosk_recognizer_partial_result\0")?,
            };
            let c_path = CString::new(model_path)?;
            let model = (fns.model_new)(c_path.as_ptr());
            if model.is_null() {
                return Err(anyhow!("Vosk 模型加载失败: {model_path}"));
            }
            let rec = (fns.rec_new)(model, 16_000.0);
            if rec.is_null() {
                (fns.model_free)(model);
                return Err(anyhow!("Vosk recognizer 创建失败"));
            }
            Ok(Self {
                _lib: lib,
                fns,
                model,
                rec,
                finals: VecDeque::new(),
                partial_text: String::new(),
            })
        }
    }

    pub fn feed(&mut self, pcm: &[i16]) -> Result<()> {
        let state = unsafe { (self.fns.accept)(self.rec, pcm.as_ptr(), pcm.len()) };
        if state < 0 {
            return Err(anyhow!("Vosk accept_waveform 失败: {state}"));
        }
        if state == 0 {
            let p = unsafe { (self.fns.partial_result)(self.rec) };
            if !p.is_null() {
                let json = unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string();
                if let Ok(v) = serde_json::from_str::<Value>(&json) {
                    self.partial_text = v["partial"].as_str().unwrap_or("").to_string();
                }
            }
        }
        if state == 1 {
            self.partial_text.clear();
            let p = unsafe { (self.fns.result)(self.rec) };
            if !p.is_null() {
                let json = unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string();
                if let Ok(v) = serde_json::from_str::<Value>(&json) {
                    let text = v["text"].as_str().unwrap_or("").trim().to_string();
                    if !text.is_empty() {
                        self.finals.push_back(text);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Vec<String> {
        self.finals.drain(..).collect()
    }

    pub fn partial(&self) -> Option<String> {
        if self.partial_text.trim().is_empty() {
            None
        } else {
            Some(self.partial_text.clone())
        }
    }

    pub fn flush(&mut self) -> Option<String> {
        let p = unsafe { (self.fns.final_result)(self.rec) };
        if p.is_null() {
            return None;
        }
        let json = unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string();
        let v: Value = serde_json::from_str(&json).ok()?;
        let text = v["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}

impl Drop for VoskStt {
    fn drop(&mut self) {
        unsafe {
            if !self.rec.is_null() {
                (self.fns.rec_free)(self.rec);
                self.rec = ptr::null_mut();
            }
            if !self.model.is_null() {
                (self.fns.model_free)(self.model);
                self.model = ptr::null_mut();
            }
        }
    }
}

#[cfg(windows)]
unsafe fn add_dll_dir(dir: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        AddDllDirectory, SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    };
    SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    AddDllDirectory(wide.as_ptr());
}

fn load_vosk_library() -> Result<libloading::Library> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("libvosk.dll"));
        }
    }
    candidates.push(crate::config::default_models_dir().join("libvosk.dll"));
    for path in candidates {
        if path.exists() {
            #[cfg(windows)]
            if let Some(dir) = path.parent() {
                unsafe { add_dll_dir(dir) };
            }
            if let Ok(lib) = unsafe { libloading::Library::new(&path) } {
                return Ok(lib);
            }
        }
    }
    unsafe { libloading::Library::new("libvosk.dll") }
        .map_err(|e| anyhow!("无法加载 libvosk.dll（先运行 models vosk cn）: {e}"))
}
// ───────────────────────────── factory ─────────────────────────────

/// 本地识别按语言选模型：中英日韩粤用 SenseVoice（快、准、带标点），其余用 Whisper。
fn build_local(language: &str) -> Result<SttEngine> {
    let lang = crate::lang::find(language);
    let sv = sensevoice_ready();
    let wh = whisper_ready();
    let sv_dir = sensevoice_dir().display().to_string();
    let wh_dir = whisper_dir().display().to_string();
    match lang {
        Some(l) if l.sensevoice && sv => {
            eprintln!("[识别] 本地 SenseVoice（{}）", l.label);
            Ok(SttEngine::Sherpa(SherpaStt::new(&sv_dir, l.code)?))
        }
        Some(l) if wh => {
            eprintln!("[识别] 本地 Whisper（{}）", l.label);
            Ok(SttEngine::Sherpa(SherpaStt::whisper(&wh_dir, l.whisper)?))
        }
        Some(l) if l.sensevoice => Err(anyhow!(
            "本地识别{}需要 SenseVoice 模型：先运行 simulvoice models sensevoice",
            l.label
        )),
        Some(l) => Err(anyhow!(
            "本地识别{}需要 Whisper 模型：先运行 simulvoice models whisper（约 640 MB）",
            l.label
        )),
        None if sv => Ok(SttEngine::Sherpa(SherpaStt::new(&sv_dir, "auto")?)),
        None if wh => Ok(SttEngine::Sherpa(SherpaStt::whisper(&wh_dir, "")?)),
        None => Err(anyhow!("本地识别缺模型：先运行 simulvoice models sensevoice")),
    }
}

pub fn build(cfg: &crate::config::SttConfig) -> Result<SttEngine> {
    build_multi(cfg, None)
}

/// 对方那一路：除了对方的语言，还要能认出"我的语言"（同事插话），
/// 因此不再把识别语言钉死，改成自动判语种。只在不换模型的前提下才开：
/// - 两门都是 SenseVoice 语言 → SenseVoice 自动（它只在中英日韩粤里判）
/// - 对方语言本来就走 Whisper → Whisper 自动
/// - 对方用 SenseVoice、我的语言它不支持 → 保持原样（换 Whisper 会更慢更不准，不值得）
///
/// 返回的 bool 表示是否真的开了自动判语种。
pub fn build_inbound(
    cfg: &crate::config::SttConfig,
    mine: &'static crate::lang::Lang,
) -> Result<(SttEngine, bool)> {
    let theirs = crate::lang::find(&cfg.language);
    let Some(theirs) = theirs.filter(|t| crate::lang::distinguishable(t, mine)) else {
        return Ok((build(cfg)?, false));
    };
    let auto = match cfg.provider.as_str() {
        "local" => {
            let sv_both = theirs.sensevoice && mine.sensevoice && sensevoice_ready();
            let whisper_path = !(theirs.sensevoice && sensevoice_ready()) && whisper_ready();
            sv_both || whisper_path
        }
        "sherpa" => theirs.sensevoice && mine.sensevoice,
        "whisper" | "soniox" => true,
        _ => false,
    };
    if !auto {
        return Ok((build(cfg)?, false));
    }
    Ok((build_multi(cfg, Some(mine))?, true))
}

fn build_multi(
    cfg: &crate::config::SttConfig,
    also: Option<&'static crate::lang::Lang>,
) -> Result<SttEngine> {
    Ok(match cfg.provider.as_str() {
        "none" => SttEngine::None,
        "local" if also.is_some() => {
            let theirs = crate::lang::find(&cfg.language);
            let sv_dir = sensevoice_dir().display().to_string();
            let wh_dir = whisper_dir().display().to_string();
            if theirs.map(|t| t.sensevoice).unwrap_or(false) && sensevoice_ready() {
                eprintln!("[识别] 本地 SenseVoice（自动判语种）");
                SttEngine::Sherpa(SherpaStt::new(&sv_dir, "auto")?)
            } else {
                eprintln!("[识别] 本地 Whisper（自动判语种）");
                SttEngine::Sherpa(SherpaStt::whisper(&wh_dir, "")?)
            }
        }
        "local" => build_local(&cfg.language)?,
        "whisper" => {
            // model_path 是各引擎共用的旧字段（默认指向 vosk），里面没有 Whisper 文件就用默认目录
            let path = if whisper_files(std::path::Path::new(cfg.model_path.trim())).is_none() {
                whisper_dir().display().to_string()
            } else {
                cfg.model_path.clone()
            };
            let code = if also.is_some() {
                ""
            } else {
                crate::lang::find(&cfg.language).map(|l| l.whisper).unwrap_or("")
            };
            SttEngine::Sherpa(SherpaStt::whisper(&path, code)?)
        }
        "openai" => {
            let key = std::env::var(&cfg.api_key_env)
                .map_err(|_| anyhow!("环境变量 {} 未设置", cfg.api_key_env))?;
            SttEngine::OpenAi(OpenAiStt::new(key, cfg.model.clone()))
        }
        "sherpa" => {
            let path = if !std::path::Path::new(cfg.model_path.trim()).join("tokens.txt").exists() {
                crate::config::default_models_dir()
                    .join("sensevoice")
                    .display()
                    .to_string()
            } else {
                cfg.model_path.clone()
            };
            let lang = if also.is_some() { "auto" } else { cfg.language.as_str() };
            SttEngine::Sherpa(SherpaStt::new(&path, lang)?)
        }
        "zipformer" => {
            let path = if cfg.model_path.trim().is_empty() {
                crate::config::default_models_dir()
                    .join("zipformer")
                    .display()
                    .to_string()
            } else {
                cfg.model_path.clone()
            };
            SttEngine::Zipformer(ZipformerStt::new(&path)?)
        }
        "vosk" => {
            let path = if cfg.model_path.trim().is_empty() {
                crate::config::default_models_dir()
                    .join("vosk")
                    .display()
                    .to_string()
            } else {
                cfg.model_path.clone()
            };
            SttEngine::Vosk(VoskStt::new(&path)?)
        }
        _ => {
            let key = std::env::var(&cfg.api_key_env)
                .map_err(|_| anyhow!("环境变量 {} 未设置", cfg.api_key_env))?;
            // Soniox 本身多语种，给两门语言做提示即可
            let hints = match also {
                Some(m) => format!("{},{}", cfg.language.trim(), m.code),
                None => cfg.language.clone(),
            };
            SttEngine::Soniox(SonioxStt::new(key, hints)?)
        }
    })
}

// keep base64 import used for future realtime clients
#[allow(dead_code)]
fn _b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}
















