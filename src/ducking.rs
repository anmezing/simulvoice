//! 念译文时把其他应用（含会议软件）的音量临时调低，念完恢复。
//!
//! Windows 的按进程收音拿到的是"调过音量之后"的声音（实测：会议软件静音后收到的全是 0），
//! 所以调低会议软件时，采集端要按同样比例放大回来（capture_gain），识别才不受影响。
//!
//! 失败模式：
//! - 多个朗读线程重叠：按深度计数，最后一个结束才恢复
//! - 程序崩溃时还没恢复：Windows 会记住应用音量，会议软件会一直偏小。
//!   调低前把"软件名 原音量"写进数据目录的 ducked-apps.txt，恢复后删掉；
//!   下次启动若文件还在，就按记录把音量调回去（recover）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

enum Cmd {
    Duck,
    Restore,
}

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
/// 当前调低的倍数（f32 位模式）；1.0 = 没在调低
static ACTIVE_LEVEL: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// 当前被调低了的进程
static DUCKED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

fn marker_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("ducked-apps.txt")
}

/// 采集 `tree` 这组进程时应乘的增益：它们正被调低就放大回来，否则 1
pub fn capture_gain(tree: &HashSet<u32>) -> f32 {
    let level = f32::from_bits(ACTIVE_LEVEL.load(Ordering::Relaxed));
    if level >= 0.999 || level <= 0.0 {
        return 1.0;
    }
    let ducked = DUCKED.lock().map(|g| g.as_ref().map(|d| !d.is_disjoint(tree)).unwrap_or(false));
    if ducked.unwrap_or(false) {
        1.0 / level
    } else {
        1.0
    }
}

pub fn init(level: f32) {
    let (tx, rx) = channel::<Cmd>();
    if TX.set(tx).is_err() {
        return;
    }
    let level = level.clamp(0.05, 1.0);
    std::thread::spawn(move || unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut saved: Vec<(windows::Win32::Media::Audio::ISimpleAudioVolume, f32)> = Vec::new();
        let mut depth: usize = 0;
        let our_pid = std::process::id();

        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Duck => {
                    depth += 1;
                    if depth > 1 {
                        continue;
                    }
                    let mut pids = HashSet::new();
                    let mut record: HashMap<String, f32> = HashMap::new();
                    let names = crate::capture::process_table();
                    for_each_session(|pid, vol| {
                        if pid == our_pid {
                            return;
                        }
                        let Ok(current) = vol.GetMasterVolume() else { return };
                        if vol.SetMasterVolume((current * level).clamp(0.0, 1.0), std::ptr::null()).is_ok() {
                            if let Some((_, exe)) = names.get(&pid) {
                                record.entry(exe.clone()).or_insert(current);
                            }
                            pids.insert(pid);
                            saved.push((vol, current));
                        }
                    });
                    if !record.is_empty() {
                        let text: String = record.iter().map(|(n, v)| format!("{n}\t{v}\n")).collect();
                        if let Err(e) = std::fs::write(marker_path(), text) {
                            eprintln!("[压低] 写恢复记录失败: {e}");
                        }
                    }
                    if let Ok(mut g) = DUCKED.lock() {
                        *g = Some(pids);
                    }
                    ACTIVE_LEVEL.store(level.to_bits(), Ordering::Relaxed);
                }
                Cmd::Restore => {
                    if depth > 0 {
                        depth -= 1;
                    }
                    if depth == 0 {
                        for (vol, previous) in saved.drain(..) {
                            let _ = vol.SetMasterVolume(previous, std::ptr::null());
                        }
                        ACTIVE_LEVEL.store(1.0f32.to_bits(), Ordering::Relaxed);
                        if let Ok(mut g) = DUCKED.lock() {
                            *g = None;
                        }
                        let _ = std::fs::remove_file(marker_path());
                    }
                }
            }
        }
    });
}

pub fn duck() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd::Duck);
    }
}

pub fn restore() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Cmd::Restore);
    }
}

/// 启动时、窗口关闭后、关机/覆盖安装前调用：没来得及恢复的音量，按记录调回去
pub fn recover() {
    let Ok(text) = std::fs::read_to_string(marker_path()) else {
        return;
    };
    let record: HashMap<String, f32> = text
        .lines()
        .filter_map(|l| {
            let (name, v) = l.split_once('\t')?;
            Some((name.trim().to_ascii_lowercase(), v.trim().parse::<f32>().ok()?.clamp(0.0, 1.0)))
        })
        .collect();
    let mut restored = 0;
    if !record.is_empty() {
        unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let names = crate::capture::process_table();
            for_each_session(|pid, vol| {
                let Some((_, exe)) = names.get(&pid) else { return };
                if let Some(v) = record.get(&exe.to_ascii_lowercase()) {
                    if vol.SetMasterVolume(*v, std::ptr::null()).is_ok() {
                        restored += 1;
                    }
                }
            });
            CoUninitialize();
        }
    }
    eprintln!("[压低] 有没恢复的记录（上次异常退出或翻译中关闭），已恢复 {restored} 个应用的音量");
    let _ = std::fs::remove_file(marker_path());
}

/// 遍历所有在用播放设备上的音频会话：(进程 pid, 音量控制)
unsafe fn for_each_session(mut f: impl FnMut(u32, windows::Win32::Media::Audio::ISimpleAudioVolume)) {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
        return;
    };
    let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
        return;
    };
    for d in 0..devices.GetCount().unwrap_or(0) {
        let Ok(device) = devices.Item(d) else { continue };
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            continue;
        };
        let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
        for i in 0..sessions.GetCount().unwrap_or(0) {
            let Ok(ctrl) = sessions.GetSession(i) else { continue };
            let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = ctrl2.GetProcessId() else { continue };
            if pid == 0 {
                continue;
            }
            let Ok(vol) = ctrl.cast::<ISimpleAudioVolume>() else { continue };
            f(pid, vol);
        }
    }
}
