use anyhow::{anyhow, Result};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

// ── protobuf helpers ─────────────────────────────────────────────────────────

fn varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

fn tag(buf: &mut Vec<u8>, field: u32, wire: u8) {
    varint(buf, ((field as u64) << 3) | wire as u64);
}

fn put_str(buf: &mut Vec<u8>, field: u32, s: &str) {
    if s.is_empty() {
        return;
    }
    tag(buf, field, 2);
    varint(buf, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

fn put_bytes(buf: &mut Vec<u8>, field: u32, b: &[u8]) {
    if b.is_empty() {
        return;
    }
    tag(buf, field, 2);
    varint(buf, b.len() as u64);
    buf.extend_from_slice(b);
}

fn put_i32(buf: &mut Vec<u8>, field: u32, v: i32) {
    if v == 0 {
        return;
    }
    tag(buf, field, 0);
    varint(buf, v as i64 as u64);
}

fn put_msg(buf: &mut Vec<u8>, field: u32, inner: &[u8]) {
    tag(buf, field, 2);
    varint(buf, inner.len() as u64);
    buf.extend_from_slice(inner);
}

fn meta(
    endpoint: &str,
    app_key: &str,
    resource: &str,
    connection: &str,
    session: &str,
    sequence: i32,
) -> Vec<u8> {
    let mut m = Vec::new();
    put_str(&mut m, 1, endpoint);
    put_str(&mut m, 2, app_key);
    put_str(&mut m, 4, resource);
    put_str(&mut m, 5, connection);
    put_str(&mut m, 6, session);
    put_i32(&mut m, 7, sequence);
    m
}

fn audio(format: &str, rate: i32, bits: i32, channel: i32, data: &[u8]) -> Vec<u8> {
    let mut a = Vec::new();
    put_str(&mut a, 4, format);
    put_i32(&mut a, 7, rate);
    put_i32(&mut a, 8, bits);
    put_i32(&mut a, 9, channel);
    put_bytes(&mut a, 14, data);
    a
}

fn req_params(mode: &str, source: &str, target: &str) -> Vec<u8> {
    let mut r = Vec::new();
    put_str(&mut r, 1, mode);
    put_str(&mut r, 2, source);
    put_str(&mut r, 3, target);
    r
}

fn user() -> Vec<u8> {
    let mut u = Vec::new();
    put_str(&mut u, 1, "simulvoice");
    put_str(&mut u, 3, "desktop");
    u
}

// ── message builders ────────────────────────────────────────────────────────

pub struct Ids {
    pub connection: String,
    pub session: String,
}

pub fn start_session(
    ids: &Ids,
    seq: i32,
    endpoint: &str,
    app_key: &str,
    resource: &str,
    source: &str,
    target: &str,
) -> Vec<u8> {
    let mut b = Vec::new();
    put_msg(
        &mut b,
        1,
        &meta(endpoint, app_key, resource, &ids.connection, &ids.session, seq),
    );
    put_i32(&mut b, 2, EVENT_START_SESSION);
    put_msg(&mut b, 3, &user());
    put_msg(&mut b, 4, &audio("pcm", 16_000, 16, 1, &[]));
    put_msg(&mut b, 6, &req_params("s2t", source, target));
    b
}

pub fn audio_request(ids: &Ids, seq: i32, pcm: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    put_msg(&mut b, 1, &meta("", "", "", &ids.connection, &ids.session, seq));
    put_i32(&mut b, 2, EVENT_TASK_REQUEST);
    put_msg(&mut b, 4, &audio("pcm", 16_000, 16, 1, pcm));
    b
}

pub fn finish_session(ids: &Ids, seq: i32) -> Vec<u8> {
    let mut b = Vec::new();
    put_msg(&mut b, 1, &meta("", "", "", &ids.connection, &ids.session, seq));
    put_i32(&mut b, 2, EVENT_FINISH_SESSION);
    b
}

pub const EVENT_SESSION_STARTED: i32 = 150;
pub const EVENT_SESSION_FAILED: i32 = 153;
pub const EVENT_START_SESSION: i32 = 100;
pub const EVENT_FINISH_SESSION: i32 = 102;
pub const EVENT_TASK_REQUEST: i32 = 200;
pub const EVENT_SOURCE_START: i32 = 650;
pub const EVENT_SOURCE_RESPONSE: i32 = 651;
pub const EVENT_SOURCE_END: i32 = 652;
pub const EVENT_TRANSLATION_START: i32 = 653;
pub const EVENT_TRANSLATION_RESPONSE: i32 = 654;
pub const EVENT_TRANSLATION_END: i32 = 655;

// ── decoding ────────────────────────────────────────────────────────────────

#[derive(Default, Debug)]
pub struct Response {
    pub status: i32,
    pub message: String,
    pub session: String,
    pub event: i32,
    pub data: Vec<u8>,
    pub text: String,
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn done(&self) -> bool {
        self.p >= self.b.len()
    }
    fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            if self.p >= self.b.len() || shift > 63 {
                return Err(anyhow!("bad varint"));
            }
            let c = self.b[self.p];
            self.p += 1;
            v |= ((c & 0x7f) as u64) << shift;
            if c < 0x80 {
                return Ok(v);
            }
            shift += 7;
        }
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.varint()? as usize;
        if self.p + n > self.b.len() {
            return Err(anyhow!("truncated"));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).to_string())
    }
}

fn parse_meta(b: &[u8]) -> (i32, String, String) {
    let mut r = R::new(b);
    let (mut status, mut message, mut session) = (0, String::new(), String::new());
    while !r.done() {
        let Ok(key) = r.varint() else { break };
        let field = (key >> 3) as u32;
        match (field, key & 7) {
            (1, 2) => session = r.string().unwrap_or_default(),
            (3, 0) => status = r.varint().unwrap_or(0) as i32,
            (4, 2) => message = r.string().unwrap_or_default(),
            _ => {
                if r.varint().is_err() {
                    break;
                }
            }
        }
    }
    (status, message, session)
}

pub fn decode_response(buf: &[u8]) -> Result<Response> {
    let mut r = R::new(buf);
    let mut out = Response::default();
    while !r.done() {
        let key = r.varint()?;
        let field = (key >> 3) as u32;
        match (field, key & 7) {
            (1, 2) => {
                let (s, m, sid) = parse_meta(r.bytes()?);
                out.status = s;
                out.message = m;
                out.session = sid;
            }
            (2, 0) => out.event = r.varint()? as i32,
            (3, 2) => out.data = r.bytes()?.to_vec(),
            (4, 2) => out.text = r.string()?,
            (_, 0) => {
                r.varint()?;
            }
            (_, 1) => r.p += 8,
            (_, 2) => {
                r.bytes()?;
            }
            (_, 5) => r.p += 4,
            _ => return Err(anyhow!("unsupported wire type")),
        }
        if r.p > r.b.len() {
            break;
        }
    }
    Ok(out)
}

// ── connection ──────────────────────────────────────────────────────────────

pub struct VolcSession {
    ws: tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    pub ids: Ids,
    pub seq: i32,
}

impl VolcSession {
    pub fn connect(
        app_key: &str,
        access_key: &str,
        resource: &str,
        source: &str,
        target: &str,
    ) -> Result<Self> {
        let url = format!(
            "wss://openspeech.bytedance.com/api/v4/ast/v2/translate\
             ?api_resource_id={resource}&api_app_key={app_key}&api_access_key={access_key}"
        );
        let (mut ws, _) = connect(&url)
        .map_err(|e| anyhow!("WS: {}", crate::http::redact(&e.to_string())))?;
        if let MaybeTlsStream::Rustls(s) = ws.get_mut() {
            let _ = s.sock.set_read_timeout(Some(Duration::from_millis(50)));
        }
        let ids = Ids {
            connection: uuid::Uuid::new_v4().simple().to_string(),
            session: uuid::Uuid::new_v4().simple().to_string(),
        };
        let mut seq = 0i32;
        let start = start_session(&ids, seq, resource, app_key, resource, source, target);
        seq += 1;
        ws.send(Message::Binary(start.into()))?;
        Ok(Self { ws, ids, seq })
    }

    pub fn send_audio(&mut self, pcm: &[i16]) -> Result<()> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let msg = audio_request(&self.ids, self.seq, &bytes);
        self.seq += 1;
        self.ws.send(Message::Binary(msg.into()))?;
        Ok(())
    }

    pub fn finish(&mut self) {
        let msg = finish_session(&self.ids, self.seq);
        self.seq += 1;
        let _ = self.ws.send(Message::Binary(msg.into()));
    }

    pub fn poll(&mut self) -> Vec<Response> {
        let mut out = Vec::new();
        loop {
            match self.ws.read() {
                Ok(Message::Binary(b)) => {
                    if let Ok(r) = decode_response(&b) {
                        if r.status != 0 && r.status != 20_000_000 {
                            out.push(Response {
                                message: format!("[{}] {}", r.status, r.message),
                                ..r
                            });
                        } else {
                            out.push(r);
                        }
                    }
                }
                Ok(Message::Text(t)) => {
                    if let Ok(r) = decode_response(t.as_bytes()) {
                        out.push(r);
                    }
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
        out
    }
}



