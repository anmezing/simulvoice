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
    None,
}

impl SttEngine {
    pub fn begin_utterance(&mut self) {
        if let SttEngine::OpenAi(s) = self {
            s.buffer.clear();
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
            _ => Ok(None),
        }
    }
}

// ───────────────────────────── Soniox (cloud streaming) ─────────────────────────────

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
            Err(e) => eprintln!("[soniox] {e}"),
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
        "language_hints": if language == "auto" { serde_json::json!([]) } else { serde_json::json!([language]) }
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
                    return Err(anyhow!("soniox error: {err}"));
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
            if let Ok(lib) = unsafe { libloading::Library::new(&path) } {
                return Ok(lib);
            }
        }
    }
    unsafe { libloading::Library::new("libvosk.dll") }
        .map_err(|e| anyhow!("无法加载 libvosk.dll（先运行 models vosk cn）: {e}"))
}
// ───────────────────────────── factory ─────────────────────────────

pub fn build(cfg: &crate::config::SttConfig) -> Result<SttEngine> {
    Ok(match cfg.provider.as_str() {
        "none" => SttEngine::None,
        "openai" => {
            let key = std::env::var(&cfg.api_key_env)
                .map_err(|_| anyhow!("环境变量 {} 未设置", cfg.api_key_env))?;
            SttEngine::OpenAi(OpenAiStt::new(key, cfg.model.clone()))
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
            SttEngine::Soniox(SonioxStt::new(key, cfg.language.clone())?)
        }
    })
}

// keep base64 import used for future realtime clients
#[allow(dead_code)]
fn _b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}











