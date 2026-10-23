use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

#[derive(Debug)]
pub enum LiveEvent {
    Ready,
    Audio,
    Input(String),
    Output(String),
    Error(String),
}

pub struct GeminiLive {
    tx: Sender<Vec<i16>>,
    end_tx: Sender<()>,
    rx: Receiver<LiveEvent>,
}

impl GeminiLive {
    pub fn connect(
        api_key: &str,
        model: &str,
        instruction: &str,
        voice: &str,
        out_device: &str,
    ) -> Result<Self> {
        let (tx, audio_rx) = channel::<Vec<i16>>();
        let (end_tx, end_rx) = channel::<()>();
        let (ev_tx, rx) = channel::<LiveEvent>();
        let (key, model, instr, voice, device) = (
            api_key.to_string(),
            model.to_string(),
            instruction.to_string(),
            voice.to_string(),
            out_device.to_string(),
        );
        std::thread::spawn(move || {
            if let Err(e) = worker(key, model, instr, voice, device, audio_rx, end_rx, ev_tx.clone()) {
                let _ = ev_tx.send(LiveEvent::Error(e.to_string()));
            }
        });
        Ok(Self { tx, end_tx, rx })
    }

    pub fn feed(&mut self, pcm: &[i16]) -> Result<()> {
        let _ = self.tx.send(pcm.to_vec());
        Ok(())
    }

    pub fn end_input(&mut self) {
        let _ = self.end_tx.send(());
    }

    pub fn poll(&mut self) -> Vec<LiveEvent> {
        self.rx.try_iter().collect()
    }
}

fn worker(
    api_key: String,
    model: String,
    instruction: String,
    voice: String,
    out_device: String,
    audio_rx: Receiver<Vec<i16>>,
    end_rx: Receiver<()>,
    ev_tx: Sender<LiveEvent>,
) -> Result<()> {
    let (_stream, handle) = crate::sink::open_output(Some(&out_device))?;
    let sink = rodio::Sink::try_new(&handle)?;
    let mut jitter = crate::dsp::JitterBuffer::new(24_000);

    loop {
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        if crate::STOP_TTS.swap(false, std::sync::atomic::Ordering::SeqCst) {
            sink.stop();
            sink.play();
        }
        match gemini_once(
            &api_key,
            &model,
            &instruction,
            &voice,
            &audio_rx,
            &end_rx,
            &ev_tx,
            &sink,
            &mut jitter,
        ) {
            Ok(()) => {}
            Err(e) => eprintln!("[live] {e}"),
        }
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        eprintln!("[live] 断线，重连中...");
        std::thread::sleep(Duration::from_millis(800));
    }
}

fn gemini_once(
    api_key: &str,
    model: &str,
    instruction: &str,
    voice: &str,
    audio_rx: &Receiver<Vec<i16>>,
    end_rx: &Receiver<()>,
    ev_tx: &Sender<LiveEvent>,
    sink: &rodio::Sink,
    jitter: &mut crate::dsp::JitterBuffer,
) -> Result<()> {
    let url = format!(
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={api_key}"
    );
    let (mut ws, _) = connect(&url)?;
    if let MaybeTlsStream::Rustls(s) = ws.get_mut() {
        let _ = s.sock.set_read_timeout(Some(Duration::from_millis(30)));
    }

    let setup = json!({
        "setup": {
            "model": model,
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {
                    "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": voice } }
                }
            },
            "systemInstruction": { "parts": [{ "text": instruction }] },
            "inputAudioTranscription": {},
            "outputAudioTranscription": {}
        }
    });
    ws.send(Message::Text(setup.to_string().into()))?;

    loop {
        if crate::STOP.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        if crate::STOP_TTS.swap(false, std::sync::atomic::Ordering::SeqCst) {
            sink.stop();
            sink.play();
        }
        if end_rx.try_recv().is_ok() {
            let msg = json!({"realtimeInput": {"activityEnd": {}}});
            let _ = ws.send(Message::Text(msg.to_string().into()));
        }
        while let Ok(chunk) = audio_rx.try_recv() {
            if crate::BARGE_IN.load(std::sync::atomic::Ordering::SeqCst)
                && rms_i16(&chunk) > 1000.0
                && !sink.empty()
            {
                sink.stop();
                sink.play();
            }
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for s in chunk {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            let msg = json!({
                "realtimeInput": {
                    "audio": { "mimeType": "audio/pcm;rate=16000", "data": B64.encode(bytes) }
                }
            });
            if ws.send(Message::Text(msg.to_string().into())).is_err() {
                return Ok(());
            }
        }

        match ws.read() {
            Ok(Message::Text(t)) => handle_server(&t, sink, ev_tx, jitter)?,
            Ok(Message::Binary(b)) => {
                if let Ok(t) = String::from_utf8(b.to_vec()) {
                    let _ = handle_server(&t, sink, ev_tx, jitter);
                }
            }
            Ok(Message::Close(c)) => {
                jitter.flush(sink);
                eprintln!("[live] server closed: {c:?}");
                return Ok(());
            }
            Ok(other) => {
                eprintln!("[live] other frame: {other:?}");
            }
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(anyhow!("Gemini Live 连接错误: {e}")),
        }
        if sink.empty() {
            crate::ducking::restore();
        }
    }
}
fn rms_i16(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}

fn handle_server(
    text: &str,
    sink: &rodio::Sink,
    ev: &Sender<LiveEvent>,
    jitter: &mut crate::dsp::JitterBuffer,
) -> Result<()> {
    if std::env::var("SIMULVOICE_DEBUG").map(|v| !v.is_empty()).unwrap_or(false) {
        eprintln!("[live] recv: {}", &text[..text.len().min(300)]);
    }
    let v: Value = serde_json::from_str(text)?;
    if v.get("setupComplete").is_some() {
        let _ = ev.send(LiveEvent::Ready);
    }
    if let Some(err) = v.get("error") {
        let _ = ev.send(LiveEvent::Error(err.to_string()));
        return Ok(());
    }
    if let Some(sc) = v.get("serverContent") {
        if let Some(t) = sc.pointer("/inputTranscription/text").and_then(|x| x.as_str()) {
            if !t.trim().is_empty() {
                let _ = ev.send(LiveEvent::Input(t.to_string()));
            }
        }
        if let Some(t) = sc.pointer("/outputTranscription/text").and_then(|x| x.as_str()) {
            if !t.trim().is_empty() {
                let _ = ev.send(LiveEvent::Output(t.to_string()));
            }
        }
        if let Some(parts) = sc.pointer("/modelTurn/parts").and_then(|p| p.as_array()) {
            for part in parts {
                if let Some(d) = part.pointer("/inlineData/data").and_then(|d| d.as_str()) {
                    if let Ok(bytes) = B64.decode(d) {
                        let samples: Vec<f32> = bytes
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                            .collect();
                        if !samples.is_empty() {
                            crate::ducking::duck();
                            jitter.push(sink, &samples);
                            let _ = ev.send(LiveEvent::Audio);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}











