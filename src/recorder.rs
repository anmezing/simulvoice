use std::fs::File;
use std::io::BufWriter;
use std::sync::atomic::{AtomicU64, Ordering};

pub static RECORD_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static TTS_SEQ: AtomicU64 = AtomicU64::new(0);

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
    let _ = std::fs::create_dir_all(&d);
    let n = TTS_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = format!("{d}/tts-{}-{n}.mp3", stamp());
    let _ = std::fs::write(path, bytes);
}


