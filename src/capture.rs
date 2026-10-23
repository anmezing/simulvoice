use anyhow::{anyhow, Result};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;
use wasapi::{get_default_device, initialize_mta, AudioClient, Direction, SampleType, StreamMode};

use crate::dsp::{downmix, Agc, StreamResampler};

#[derive(Clone)]
pub enum Source {
    /// Capture everything rendered by the system (optionally one process tree).
    Loopback(Option<u32>),
    /// Capture a microphone / capture device by name (empty = default).
    Mic(Option<String>),
}

pub struct Capture {
    pub rx: Receiver<Vec<i16>>,
    pub sample_rate: u32,
}

#[derive(Clone, Copy)]
enum Fmt {
    F32,
    I16,
    I24,
    I32,
    I24in32,
}

pub fn open(source: Source, denoise: bool, agc: bool) -> Result<Capture> {
    let (tx, rx) = sync_channel::<Vec<i16>>(100);
    let (meta_tx, meta_rx) = std::sync::mpsc::channel::<Result<u32>>();

    thread::spawn(move || {
        let mut meta_sent = false;
        loop {
            if crate::STOP.load(Ordering::SeqCst) {
                break;
            }
            match run_capture(&source, denoise, agc, &tx, &meta_tx, &mut meta_sent) {
                Ok(()) => {
                    if !meta_sent {
                        return;
                    }
                    if crate::STOP.load(Ordering::SeqCst) {
                        break;
                    }
                    eprintln!("[capture] 音频设备断开，0.5s 后重连");
                }
                Err(e) => {
                    if !meta_sent {
                        let _ = meta_tx.send(Err(e));
                        return;
                    }
                    eprintln!("[capture] {e}，0.5s 后重连");
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
    });

    let sample_rate = meta_rx
        .recv()
        .map_err(|_| anyhow!("capture thread exited before start"))??;

    Ok(Capture { rx, sample_rate })
}

fn run_capture(
    source: &Source,
    denoise: bool,
    agc: bool,
    tx: &SyncSender<Vec<i16>>,
    meta_tx: &Sender<Result<u32>>,
    meta_sent: &mut bool,
) -> Result<()> {
    initialize_mta()
        .ok()
        .map_err(|e| anyhow!("COM init failed: {e}"))?;

    let mut client = match source {
        Source::Loopback(pid) => match pid {
            Some(p) => AudioClient::new_application_loopback_client(*p, true)?,
            None => get_default_device(&Direction::Render)?.get_iaudioclient()?,
        },
        Source::Mic(name) => match name {
            Some(n) if !n.trim().is_empty() => wasapi::DeviceCollection::new(&Direction::Capture)?
                .get_device_with_name(n)?
                .get_iaudioclient()?,
            _ => get_default_device(&Direction::Capture)?.get_iaudioclient()?,
        },
    };

    let fmt = client.get_mixformat()?;
    let rate = fmt.get_samplespersec();
    let ch = fmt.get_nchannels() as usize;
    let bits = fmt.get_bitspersample() as usize;
    let valid = fmt.get_validbitspersample() as usize;
    let sub = fmt.get_subformat()?;
    let kind = match sub {
        SampleType::Float => Fmt::F32,
        _ => match bits {
            16 => Fmt::I16,
            24 => Fmt::I24,
            32 if valid == 24 => Fmt::I24in32,
            32 => Fmt::I32,
            _ => Fmt::I16,
        },
    };

    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 100_000,
        },
    )?;
    client.start_stream()?;
    let cap = client.get_audiocaptureclient()?;

    let upmix = denoise && rate != 48_000;
    let effective = if upmix { 48_000 } else { rate };
    if !*meta_sent {
        let _ = meta_tx.send(Ok(effective));
        *meta_sent = true;
    }

    let mut to48 = if upmix {
        Some(StreamResampler::new(rate, 48_000))
    } else {
        None
    };
    let mut agc = if agc && matches!(source, Source::Mic(_)) {
        Some(Agc::new())
    } else {
        None
    };
    let mut denoiser = if denoise {
        Some(nnnoiseless::DenoiseState::new())
    } else {
        None
    };
    let mut dsp_buf: Vec<f32> = Vec::new();
    let mut dq: VecDeque<u8> = VecDeque::new();

    let bytes_ps = match kind {
        Fmt::F32 | Fmt::I32 | Fmt::I24in32 => 4,
        Fmt::I24 => 3,
        Fmt::I16 => 2,
    };
    let frame_bytes = bytes_ps * ch;

    while !crate::STOP.load(Ordering::SeqCst) {
        if cap.read_from_device_to_deque(&mut dq).is_err() {
            break;
        }
        if frame_bytes == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let frames = dq.len() / frame_bytes;
        if frames == 0 {
            thread::sleep(Duration::from_millis(3));
            continue;
        }

        let mut inter: Vec<i16> = Vec::with_capacity(frames * ch);
        for _ in 0..frames {
            for _ in 0..ch {
                let b0 = dq.pop_front().unwrap_or(0);
                let b1 = dq.pop_front().unwrap_or(0);
                let b2 = dq.pop_front().unwrap_or(0);
                let b3 = dq.pop_front().unwrap_or(0);
                let v: i16 = match kind {
                    Fmt::F32 => {
                        let f = f32::from_le_bytes([b0, b1, b2, b3]);
                        (f.clamp(-1.0, 1.0) * 32767.0) as i16
                    }
                    Fmt::I16 => {
                        dq.push_front(b3);
                        dq.push_front(b2);
                        i16::from_le_bytes([b0, b1])
                    }
                    Fmt::I24 => {
                        dq.push_front(b3);
                        let v = (b0 as i32) | ((b1 as i32) << 8) | ((b2 as i32) << 16);
                        ((v << 8) >> 16) as i16
                    }
                    Fmt::I24in32 => {
                        let v = i32::from_le_bytes([b0, b1, b2, b3]);
                        (v >> 16) as i16
                    }
                    Fmt::I32 => {
                        let v = i32::from_le_bytes([b0, b1, b2, b3]);
                        (v >> 16) as i16
                    }
                };
                inter.push(v);
            }
        }

        let mut mono = downmix(&inter, ch);
        if let Some(r) = to48.as_mut() {
            mono = r.process(&mono);
        }
        if let Some(a) = agc.as_mut() {
            a.process(&mut mono);
        }

        let out = match denoiser.as_mut() {
            Some(state) => {
                dsp_buf.extend(mono.iter().map(|&s| s as f32));
                let mut clean = Vec::with_capacity(mono.len());
                let mut frame_out = [0f32; 480];
                while dsp_buf.len() >= 480 {
                    let input: Vec<f32> = dsp_buf.drain(..480).collect();
                    state.process_frame(&mut frame_out, &input);
                    clean.extend(frame_out.iter().map(|&s| s as i16));
                }
                clean
            }
            None => mono,
        };
        if out.is_empty() {
            continue;
        }
        match tx.try_send(out) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {}
        }
    }
    Ok(())
}

pub fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}


