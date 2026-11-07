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
//!
//! 「只听译文」也在这里（Hush）：翻译期间把会议软件的音量压到千分之一（-60 dB，耳朵听不到），
//! 采集端放大 1000 倍还原。按进程收到的是 32 位浮点，在浮点里缩放没有精度损失，识别不受影响。
//! 这样不需要第二个播放设备，也不用改会议软件的设置。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

enum Cmd {
    Duck,
    Restore,
}

static TX: OnceLock<Sender<Cmd>> = OnceLock::new();
/// 当前调低的倍数（f32 位模式）；1.0 = 没在调低
static ACTIVE_LEVEL: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// 当前被调低了的进程
static DUCKED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

/// 只听译文时会议软件的音量倍数（-60 dB）
const HUSH: f32 = 0.001;
/// 当前被"只听译文"压低的进程
static HUSHED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
/// 程序正在退出（关机、覆盖安装、关窗口）：已经恢复过音量，别再压低
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn marker_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("ducked-apps.txt")
}

fn hush_marker_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("hushed-apps.txt")
}

fn in_set(set: &Mutex<Option<HashSet<u32>>>, tree: &HashSet<u32>) -> bool {
    set.lock().map(|g| g.as_ref().map(|d| !d.is_disjoint(tree)).unwrap_or(false)).unwrap_or(false)
}

fn is_hushed(pid: u32) -> bool {
    HUSHED.lock().map(|g| g.as_ref().map(|d| d.contains(&pid)).unwrap_or(false)).unwrap_or(false)
}

/// 采集 `tree` 这组进程时应乘的增益：(只听译文那部分, 念译文时压低那部分)。没在调低就是 1
pub fn capture_gain(tree: &HashSet<u32>) -> (f32, f32) {
    let hush = if in_set(&HUSHED, tree) { 1.0 / HUSH } else { 1.0 };
    let level = f32::from_bits(ACTIVE_LEVEL.load(Ordering::Relaxed));
    let duck = if level < 0.999 && level > 0.0 && in_set(&DUCKED, tree) { 1.0 / level } else { 1.0 };
    (hush, duck)
}

/// 只听译文：持有期间会议软件（含子进程）几乎静音；丢弃时恢复原音量
pub struct Hush {
    stop: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl Hush {
    pub fn start(root_pid: u32) -> Self {
        let (tx, rx) = channel::<()>();
        let handle = std::thread::spawn(move || unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            // (音量控制, 原音量)；会议软件中途新开的声音、被人手动调大的，下一轮再压
            let mut saved: Vec<(windows::Win32::Media::Audio::ISimpleAudioVolume, f32)> = Vec::new();
            // 软件名 -> 第一次见到时的原音量（写进恢复记录）
            let mut record: HashMap<String, f32> = HashMap::new();
            let mut announced = false;
            loop {
                if SHUTTING_DOWN.load(Ordering::SeqCst) {
                    break;
                }
                let tree = crate::capture::process_tree(root_pid);
                let names = crate::capture::process_table();
                let mut lowered = 0;
                for_each_session(|pid, vol| {
                    if !tree.contains(&pid) || SHUTTING_DOWN.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(current) = vol.GetMasterVolume() else { return };
                    // 已经压过（或本来就几乎为 0）
                    if current <= HUSH * 1.01 {
                        return;
                    }
                    // 先记下再压：压低后崩溃，下次启动也能按记录恢复
                    if let Some((_, exe)) = names.get(&pid) {
                        if !record.contains_key(exe) {
                            record.insert(exe.clone(), current);
                            let text: String = record.iter().map(|(n, v)| format!("{n}\t{v}\n")).collect();
                            if let Err(e) = std::fs::write(hush_marker_path(), text) {
                                eprintln!("[只听译文] 写恢复记录失败: {e}");
                            }
                        }
                    }
                    if vol.SetMasterVolume(current * HUSH, std::ptr::null()).is_ok() {
                        saved.push((vol, current));
                        lowered += 1;
                    }
                });
                // 压低之后才让采集端放大，而且等已经在路上的声音（几十毫秒）先过去：
                // 不然压低前的原音量会被放大 1000 倍，削顶成一阵噪声送进识别
                if lowered > 0 {
                    std::thread::sleep(Duration::from_millis(150));
                }
                if let Ok(mut g) = HUSHED.lock() {
                    *g = Some(tree);
                }
                if lowered > 0 && !std::mem::replace(&mut announced, true) {
                    eprintln!("[只听译文] 已把会议软件的声音关到听不见（照常识别翻译）");
                }
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Err(RecvTimeoutError::Timeout) => continue,
                    _ => break,
                }
            }
            // 先停止放大再恢复音量：中间那一下只会偏小，不会爆音
            if let Ok(mut g) = HUSHED.lock() {
                *g = None;
            }
            // 倒着恢复：同一路声音被压过两次时，最后留下的是最早的原音量
            for (vol, previous) in saved.iter().rev() {
                let _ = vol.SetMasterVolume(*previous, std::ptr::null());
            }
            if !saved.is_empty() {
                eprintln!("[只听译文] 会议软件的声音已恢复");
            }
            let _ = std::fs::remove_file(hush_marker_path());
            CoUninitialize();
        });
        Self { stop: Some(tx), handle: Some(handle) }
    }
}

impl Drop for Hush {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 程序退出（关窗口、关机、覆盖安装）时调用：不再压低任何声音，并把被压低的都恢复
pub fn restore_for_exit() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    recover();
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
                        // 只听译文压着的会议软件不再叠加调低（恢复时会把它调回"压着"的音量，乱套）
                        if pid == our_pid || is_hushed(pid) {
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
    recover_from(&marker_path(), "压低");
    recover_from(&hush_marker_path(), "只听译文");
}

fn recover_from(path: &std::path::Path, label: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
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
    eprintln!("[{label}] 有没恢复的记录（上次异常退出或翻译中关闭），已恢复 {restored} 个应用的音量");
    let _ = std::fs::remove_file(path);
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
