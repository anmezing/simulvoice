use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

pub fn lang_code(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "english" | "en" => "en",
        "chinese" | "zh" | "中文" => "zh",
        "japanese" | "ja" => "ja",
        "korean" | "ko" => "ko",
        "russian" | "ru" => "ru",
        "french" | "fr" => "fr",
        "german" | "de" => "de",
        "spanish" | "es" => "es",
        "portuguese" | "pt" => "pt",
        "italian" | "it" => "it",
        _ => "en",
    }
}

pub struct PalabraConfig {
    pub api_key: String,
    pub source: String,
    pub target: String,
    pub voice: String,
    pub speech: bool,
}

fn set_task(cfg: &PalabraConfig) -> Value {
    json!({
        "message_type": "set_task",
        "data": {
            "input_stream": {
                "content_type": "audio",
                "source": { "type": "ws", "format": "pcm_s16le", "sample_rate": 16000, "channels": 1 }
            },
            "output_stream": if cfg.speech {
                json!({ "content_type": "audio", "target": { "type": "ws", "format": "pcm_s16le" } })
            } else {
                Value::Null
            },
            "pipeline": {
                "transcription": {
                    "source_language": lang_code(&cfg.source),
                    "detectable_languages": [],
                    "segment_confirmation_silence_threshold": 0.7,
                    "sentence_splitter": { "enabled": true }
                },
                "translations": [{
                    "target_language": lang_code(&cfg.target),
                    "translate_partial_transcriptions": true,
                    "speech_generation": {
                        "voice_cloning": false,
                        "voice_id": if cfg.voice.is_empty() { "default_low" } else { &cfg.voice },
                        "voice_timbre_detection": {
                            "enabled": false,
                            "high_timbre_voices": ["default_high"],
                            "low_timbre_voices": ["default_low"]
                        }
                    }
                }],
                "translation_queue_configs": {
                    "global": { "desired_queue_level_ms": 8000, "max_queue_level_ms": 24000, "auto_tempo": false }
                },
                "allowed_message_types": [
                    "translated_transcription",
                    "partial_transcription",
                    "partial_translated_transcription",
                    "validated_transcription"
                ]
            }
        }
    })
}

fn audio_msg(pcm: &[i16]) -> Message {
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    Message::Text(
        json!({
            "message_type": "input_audio_data",
            "data": { "data": B64.encode(bytes) }
        })
        .to_string()
        .into(),
    )
}

pub struct PalabraSession {
    ws: tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    pub out_rate: u32,
    sink: Option<rodio::Sink>,
    jitter: crate::dsp::JitterBuffer,
    _stream: Option<rodio::OutputStream>,
}

impl PalabraSession {
    pub fn connect(cfg: &PalabraConfig, out_device: Option<&str>) -> Result<Self> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let url = format!(
            "wss://streaming.palabra.ai/streaming-api/{id}/v1/speech-to-speech/stream?token={}",
            urlencoding(&cfg.api_key)
        );
        let (mut ws, _) = connect(&url)?;
        if let MaybeTlsStream::Rustls(s) = ws.get_mut() {
            let _ = s.sock.set_read_timeout(Some(Duration::from_millis(30)));
        }
        ws.send(Message::Text(set_task(cfg).to_string().into()))?;
        let (stream, sink) = if cfg.speech {
            let (stream, handle) = crate::sink::open_output(out_device)?;
            (Some(stream), Some(rodio::Sink::try_new(&handle)?))
        } else {
            (None, None)
        };
        Ok(Self {
            ws,
            out_rate: 24_000,
            sink,
            jitter: crate::dsp::JitterBuffer::new(24_000),
            _stream: stream,
        })
    }

    pub fn send_audio(&mut self, pcm: &[i16]) -> Result<()> {
        self.ws.send(audio_msg(pcm))?;
        Ok(())
    }

    pub fn end(&mut self) -> Result<()> {
        let msg = json!({"message_type": "end_task", "data": { "force": true }});
        let _ = self.ws.send(Message::Text(msg.to_string().into()));
        Ok(())
    }

    /// Reads one batch of pending events without blocking.
    pub fn poll(&mut self) -> Vec<PalabraEvent> {
        let mut out = Vec::new();
        loop {
            match self.ws.read() {
                Ok(Message::Text(t)) => self.handle(&t, &mut out),
                Ok(Message::Binary(b)) => {
                    if let Ok(t) = String::from_utf8(b.to_vec()) {
                        self.handle(&t, &mut out);
                    }
                }
                Ok(Message::Close(_)) => {
                    out.push(PalabraEvent::Closed);
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(e) => {
                    out.push(PalabraEvent::Error(e.to_string()));
                    break;
                }
            }
        }
        out
    }

    fn handle(&mut self, text: &str, out: &mut Vec<PalabraEvent>) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let kind = v["message_type"].as_str().unwrap_or("");
        match kind {
            "partial_transcription" => {
                if let Some(t) = v["data"]["transcription"]["text"].as_str() {
                    out.push(PalabraEvent::SourcePartial(t.to_string()));
                }
            }
            "validated_transcription" => {
                if let Some(t) = v["data"]["transcription"]["text"].as_str() {
                    out.push(PalabraEvent::SourceFinal(t.to_string()));
                }
            }
            "partial_translated_transcription" => {
                if let Some(t) = v["data"]["transcription"]["text"].as_str() {
                    out.push(PalabraEvent::TargetPartial(t.to_string()));
                }
            }
            "translated_transcription" => {
                if let Some(t) = v["data"]["transcription"]["text"].as_str() {
                    out.push(PalabraEvent::TargetFinal(t.to_string()));
                }
            }
            "output_audio_data" => {
                if let Some(d) = v["data"]["data"].as_str() {
                    if let Ok(bytes) = B64.decode(d) {
                        let samples: Vec<f32> = bytes
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                            .collect();
                        if let Some(sink) = &self.sink {
                            self.jitter.push(sink, &samples);
                        }
                        out.push(PalabraEvent::Audio);
                    }
                }
            }
            "error" => out.push(PalabraEvent::Error(v["data"].to_string())),
            "current_task" => out.push(PalabraEvent::Task(
                v["data"]["status"].as_str().unwrap_or("").to_string(),
            )),
            "end_of_stream" => out.push(PalabraEvent::Closed),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub enum PalabraEvent {
    Task(String),
    SourcePartial(String),
    SourceFinal(String),
    TargetPartial(String),
    TargetFinal(String),
    Audio,
    Error(String),
    Closed,
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}





