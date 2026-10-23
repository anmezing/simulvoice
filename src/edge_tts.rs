use anyhow::{anyhow, Result};
use chrono::Utc;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const GEC_VERSION: &str = "1-143.0.3650.75";

pub struct EdgeTts {
    pub voice: String,
    pub rate: String,
}

impl Default for EdgeTts {
    fn default() -> Self {
        Self {
            voice: "zh-CN-XiaoxiaoNeural".into(),
            rate: "+0%".into(),
        }
    }
}

/// A reusable Edge TTS websocket: one connection, many utterances.
pub struct EdgeTtsConn {
    ws: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    voice: String,
    rate: String,
}

fn sec_ms_gec() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut ticks = (secs + 11_644_473_600) * 10_000_000;
    ticks -= ticks % 3_000_000_000;
    let mut h = Sha256::new();
    h.update(format!("{ticks}{TOKEN}"));
    format!("{:X}", h.finalize())
}

fn new_id() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| format!("{:X}", rng.gen_range(0..16u8)))
        .collect()
}

fn ts() -> String {
    Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

fn open(voice: &str, rate: &str) -> Result<EdgeTtsConn> {
    let url = format!(
        "wss://api.msedgeservices.com/tts/cognitiveservices/websocket/v1\
         ?TrustedClientToken={TOKEN}&Sec-MS-GEC={}&Sec-MS-GEC-Version={GEC_VERSION}&ConnectionId={}&Ocp-Apim-Subscription-Key={TOKEN}",
        sec_ms_gec(),
        new_id()
    );
    let (mut ws, _) = connect(&url)?;
    let config = format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{}",
        ts(),
        r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"false"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#
    );
    ws.send(Message::Text(config.into()))?;
    Ok(EdgeTtsConn {
        ws,
        voice: voice.to_string(),
        rate: rate.to_string(),
    })
}

impl EdgeTtsConn {
    pub fn connect(voice: &str, rate: &str) -> Result<Self> {
        open(voice, rate)
    }

    pub fn synth(&mut self, text: &str) -> Result<Vec<u8>> {
        let ssml = format!(
            "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
             <voice name='{}'><prosody pitch='+0Hz' rate='{}' volume='+0%'>{}</prosody></voice></speak>",
            self.voice,
            self.rate,
            escape(text)
        );
        let msg = format!(
            "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n{}",
            new_id(),
            ts(),
            ssml
        );
        self.ws.send(Message::Text(msg.into()))?;

        let mut audio = Vec::new();
        let mut last_err = String::new();
        loop {
            let message = match self.ws.read() {
                Ok(m) => m,
                Err(e) => {
                    last_err = e.to_string();
                    break;
                }
            };
            match message {
                Message::Binary(b) => {
                    if b.len() < 2 {
                        continue;
                    }
                    let hlen = u16::from_be_bytes([b[0], b[1]]) as usize;
                    if b.len() < 2 + hlen {
                        continue;
                    }
                    let header = String::from_utf8_lossy(&b[2..2 + hlen]);
                    if header.contains("Path:audio") {
                        audio.extend_from_slice(&b[2 + hlen..]);
                    }
                }
                Message::Text(t) => {
                    if t.contains("Path:turn.end") {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        if audio.is_empty() {
            return Err(anyhow!("edge-tts: empty audio ({last_err})"));
        }
        Ok(audio)
    }
}

impl EdgeTts {
    /// One-shot synthesis (opens and closes a connection).
    pub fn synth(&self, text: &str) -> Result<Vec<u8>> {
        let mut conn = open(&self.voice, &self.rate)?;
        conn.synth(text)
    }
}


