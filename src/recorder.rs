use std::fs::File;
use std::io::BufWriter;
use std::sync::atomic::{AtomicU64, Ordering};

pub static RECORD_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static TTS_SEQ: AtomicU64 = AtomicU64::new(0);
static MAX_MB: AtomicU64 = AtomicU64::new(2048);
static TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);

pub fn set_limit(mb: u64) {
    MAX_MB.store(mb, Ordering::Relaxed);
}

fn over_limit() -> bool {
    let max = MAX_MB.load(Ordering::Relaxed);
    max > 0 && TOTAL_BYTES.load(Ordering::Relaxed) >= max * 1024 * 1024
}

fn dir() -> Option<String> {
    let d = RECORD_DIR.get()?;
    if d.trim().is_empty() {
        None
    } else {
        Some(d.clone())
    }
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Records 16 kHz mono source audio to WAV.
pub struct SourceRecorder {
    writer: Option<hound::WavWriter<BufWriter<File>>>,
}

impl SourceRecorder {
    pub fn new(side: &str) -> Self {
        let Some(d) = dir() else {
            return Self { writer: None };
        };
        let _ = std::fs::create_dir_all(&d);
        let path = format!("{d}/source-{side}-{}.wav", stamp());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = File::create(&path)
            .ok()
            .and_then(|f| hound::WavWriter::new(BufWriter::new(f), spec).ok());
        Self { writer }
    }

    pub fn write(&mut self, samples: &[i16]) {
        if over_limit() {
            self.writer = None;
            return;
        }
        TOTAL_BYTES.fetch_add((samples.len() * 2) as u64, Ordering::Relaxed);
        if let Some(w) = self.writer.as_mut() {
            for &s in samples {
                let _ = w.write_sample(s);
            }
        }
    }
}

/// Saves synthesized TTS audio chunks for later replay.
pub fn save_tts(bytes: &[u8]) {
    let Some(d) = dir() else {
        return;
    };
    if over_limit() {
        return;
    }
    TOTAL_BYTES.fetch_add(bytes.len() as u64, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(&d);
    let n = TTS_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = format!("{d}/tts-{}-{n}.mp3", stamp());
    let _ = std::fs::write(path, bytes);
}



