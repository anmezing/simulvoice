use anyhow::{anyhow, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;
use wasapi::{get_default_device, initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

use crate::dsp::{downmix, Agc, StreamResampler};

#[derive(Clone)]
pub enum Source {
    /// Capture everything rendered by the system (optionally one process tree).
    Loopback(Option<u32>),
    /// Capture a named render device (loopback).
    LoopbackDevice(String),
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
        Source::LoopbackDevice(name) => {
            find_device(&Direction::Render, name)?.get_iaudioclient()?
        }
        Source::Mic(name) => match name {
            Some(n) if !n.trim().is_empty() => {
                find_device(&Direction::Capture, n)?.get_iaudioclient()?
            }
            _ => get_default_device(&Direction::Capture)?.get_iaudioclient()?,
        },
    };

    // 按进程收音（只收某个软件的声音）时 Windows 不提供混音格式（GetMixFormat 返回
    // E_NOTIMPL "尚未实现"），只能自己指定格式、靠 autoconvert 转换，并且要用事件模式读取。
    let process_loopback = matches!(source, Source::Loopback(Some(_)));
    let fmt = if process_loopback {
        WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None)
    } else {
        client.get_mixformat()?
    };
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

    let mode = if process_loopback {
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        }
    } else {
        StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 100_000,
        }
    };
    client.initialize_client(&fmt, &Direction::Capture, &mode)?;
    let event = if process_loopback {
        Some(client.set_get_eventhandle()?)
    } else {
        None
    };
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
        LAST_TICK.store(now_secs(), Ordering::Relaxed);
        if cap.read_from_device_to_deque(&mut dq).is_err() {
            break;
        }
        if frame_bytes == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let frames = dq.len() / frame_bytes;
        if frames == 0 {
            match &event {
                // 软件没在出声时不会有事件，超时属正常：回到循环顶部，顺便检查 STOP
                Some(h) => {
                    let _ = h.wait_for_event(200);
                }
                None => thread::sleep(Duration::from_millis(3)),
            }
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

fn find_device(direction: &Direction, needle: &str) -> Result<wasapi::Device> {
    let collection = wasapi::DeviceCollection::new(direction)?;
    let count = collection.get_nbr_devices()?;
    let needle = needle.to_lowercase();
    for i in 0..count {
        let device = collection.get_device_at_index(i)?;
        if let Ok(name) = device.get_friendlyname() {
            if name.to_lowercase().contains(&needle) {
                return Ok(device);
            }
        }
    }
    Err(anyhow!("找不到设备包含 '{}'（用 simulvoice devices 查看）", needle))
}

static LAST_TICK: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn last_tick() -> u64 {
    LAST_TICK.load(Ordering::Relaxed)
}

/// 新会话开始前清零，否则看门狗会拿上一次会话的时间戳判定"采集卡死"。
pub fn reset_tick() {
    LAST_TICK.store(0, Ordering::Relaxed);
}

pub fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}






#[cfg(windows)]
pub fn list_window_processes() -> Vec<(u32, String, String)> {
    use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return 1;
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let title = String::from_utf16_lossy(&buf[..len as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return 1;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        let name = if h.is_null() {
            String::new()
        } else {
            let mut pbuf = [0u16; 1024];
            let mut plen = pbuf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, pbuf.as_mut_ptr(), &mut plen);
            let _ = CloseHandle(h);
            if ok == 0 {
                String::new()
            } else {
                String::from_utf16_lossy(&pbuf[..plen as usize])
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }
        };
        let out = &mut *(lparam as *mut Vec<(u32, String, String)>);
        if !out.iter().any(|(p, _, _)| *p == pid) {
            out.push((pid, name, title));
        }
        1
    }

    let mut out: Vec<(u32, String, String)> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut out as *mut _ as LPARAM);
    }
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    out
}

#[cfg(not(windows))]
pub fn list_window_processes() -> Vec<(u32, String, String)> {
    Vec::new()
}

/// 按可执行文件名找一个有窗口的进程（会议软件重启后 PID 会变）。
pub fn find_pid(exe: &str) -> Option<u32> {
    let want = exe.trim();
    if want.is_empty() {
        return None;
    }
    let pid = list_window_processes()
        .into_iter()
        .find(|(_, name, _)| name.eq_ignore_ascii_case(want))
        .map(|(pid, _, _)| pid)?;
    Some(topmost_same_exe(pid))
}

/// 会议软件常拆成好几个同名进程（腾讯会议有十几个），出声的不一定是窗口所在的那个。
/// 往上找到同名的最顶层进程，按进程收音时连同整棵子进程树一起收，才不会漏掉。
#[cfg(windows)]
fn topmost_same_exe(pid: u32) -> u32 {
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    // pid -> (父 pid, 可执行文件名)
    let mut procs: HashMap<u32, (u32, String)> = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return pid;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]);
            procs.insert(e.th32ProcessID, (e.th32ParentProcessID, name));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    let Some((_, exe)) = procs.get(&pid).cloned() else {
        return pid;
    };
    let mut cur = pid;
    // 上限防止父子关系成环（PID 复用时可能出现）
    for _ in 0..16 {
        match procs.get(&cur) {
            Some((parent, _)) if *parent != cur => match procs.get(parent) {
                Some((_, name)) if name.eq_ignore_ascii_case(&exe) => cur = *parent,
                _ => break,
            },
            _ => break,
        }
    }
    cur
}

#[cfg(not(windows))]
fn topmost_same_exe(pid: u32) -> u32 {
    pid
}
