//! 「只听译文」：翻译期间把会议软件所用的播放设备整体静音，你只听到耳机里的译文。
//!
//! 为什么是整台设备而不是会议软件本身：Windows 的按进程收音拿到的是应用音量/静音之后的声音
//! （把会议软件静音后收到的全是 0），但在"设备静音"之前（实测设备静音时照样收到原音量）。
//! 所以会议声音和译文必须走两个不同的播放设备：会议软件 → 设备 A（被静音），译文 → 耳机。
//!
//! 失败模式：
//! - 会议软件和译文在同一台设备上：不能静音（会把译文也静音），通过 on_conflict 提示用户怎么改
//! - 绝不静音"给对方的虚拟麦克风"（CABLE Input）和监听设备，否则对方听不到你的译音
//! - 会议软件中途换了设备：每 2 秒重新看一遍，新设备也静音
//! - 用户本来就静音的设备：不接管，结束时也不替他取消
//! - 程序崩溃：Windows 会一直保持设备静音。静音前把设备 ID 写进数据目录的 muted-devices.txt，
//!   正常结束时删掉；下次启动若还在，就取消这些设备的静音（recover）

use std::collections::HashSet;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{Interface, PWSTR};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

const RESCAN: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

fn marker_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("muted-devices.txt")
}

unsafe fn take_pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

unsafe fn enumerator() -> Option<IMMDeviceEnumerator> {
    CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
}

unsafe fn describe(device: &IMMDevice) -> Option<OutputDevice> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::System::Com::StructuredStorage::PropVariantToString;
    let id = take_pwstr(device.GetId().ok()?);
    let mut name = String::new();
    if let Ok(store) = device.OpenPropertyStore(STGM_READ) {
        if let Ok(v) = store.GetValue(&PKEY_Device_FriendlyName) {
            let mut buf = [0u16; 256];
            if PropVariantToString(&v, &mut buf).is_ok() {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                name = String::from_utf16_lossy(&buf[..len]);
            }
        }
    }
    Some(OutputDevice { id, name })
}

/// 按名字（子串，和设置里"译文播放到"的写法一样）找播放设备；空 = 系统默认
pub fn find_output(name: &str) -> Option<OutputDevice> {
    unsafe {
        let en = enumerator()?;
        let want = name.trim();
        if want.is_empty() {
            return describe(&en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?);
        }
        let devices = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).ok()?;
        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(d) = devices.Item(i) else { continue };
            if let Some(info) = describe(&d) {
                if info.name.to_lowercase().contains(&want.to_lowercase()) {
                    return Some(info);
                }
            }
        }
        None
    }
}

/// 这组进程正在用哪些播放设备出声（有活动会话的优先；都不活动时返回有会话的设备）
pub fn devices_of(tree: &HashSet<u32>) -> Vec<OutputDevice> {
    unsafe {
        let Some(en) = enumerator() else { return Vec::new() };
        let Ok(devices) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return Vec::new();
        };
        let (mut active, mut any) = (Vec::new(), Vec::new());
        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(i) else { continue };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
            let (mut has, mut has_active) = (false, false);
            for s in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(ctrl) = sessions.GetSession(s) else { continue };
                let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else { continue };
                if tree.contains(&ctrl2.GetProcessId().unwrap_or(0)) {
                    has = true;
                    has_active |= ctrl.GetState().map(|st| st == AudioSessionStateActive).unwrap_or(false);
                }
            }
            if let (true, Some(info)) = (has, describe(&device)) {
                if has_active {
                    active.push(info.clone());
                }
                any.push(info);
            }
        }
        if active.is_empty() {
            any
        } else {
            active
        }
    }
}

unsafe fn endpoint_volume(en: &IMMDeviceEnumerator, id: &str) -> Option<IAudioEndpointVolume> {
    let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
    let device = en.GetDevice(windows::core::PCWSTR(wide.as_ptr())).ok()?;
    device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// 翻译期间持有；丢弃时取消静音
pub struct DeviceMute {
    stop: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl DeviceMute {
    /// root_pid：会议软件；keep：绝不能静音的设备（译文播放、给对方的虚拟麦、监听）。
    /// on_conflict(设备名)：会议软件和译文在同一台设备上时调用（每台设备只提示一次）
    pub fn start(root_pid: u32, keep: Vec<OutputDevice>, on_conflict: Box<dyn Fn(&str) + Send>) -> Self {
        let (tx, rx) = channel::<()>();
        let handle = std::thread::spawn(move || unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let keep_ids: HashSet<String> = keep.iter().map(|d| d.id.clone()).collect();
            let mut muted: Vec<String> = Vec::new();
            let mut warned: HashSet<String> = HashSet::new();
            if let Some(en) = enumerator() {
                loop {
                    let tree = crate::capture::process_tree(root_pid);
                    for dev in devices_of(&tree) {
                        if muted.contains(&dev.id) {
                            continue;
                        }
                        if keep_ids.contains(&dev.id) {
                            if warned.insert(dev.id.clone()) {
                                on_conflict(&dev.name);
                            }
                            continue;
                        }
                        let Some(vol) = endpoint_volume(&en, &dev.id) else { continue };
                        // 用户本来就静音的设备不接管
                        if vol.GetMute().map(|b| b.as_bool()).unwrap_or(true) {
                            continue;
                        }
                        muted.push(dev.id.clone());
                        // 先记下再静音：静音后崩溃也能在下次启动恢复
                        if let Err(e) = std::fs::write(marker_path(), muted.join("\n")) {
                            eprintln!("[只听译文] 写恢复记录失败: {e}");
                        }
                        match vol.SetMute(true, std::ptr::null()) {
                            Ok(()) => eprintln!("[只听译文] 已静音会议软件的播放设备：{}", dev.name),
                            Err(e) => {
                                eprintln!("[只听译文] 静音 {} 失败: {e}", dev.name);
                                muted.pop();
                            }
                        }
                    }
                    match rx.recv_timeout(RESCAN) {
                        Err(RecvTimeoutError::Timeout) => continue,
                        _ => break,
                    }
                }
                for id in &muted {
                    if let Some(vol) = endpoint_volume(&en, id) {
                        let _ = vol.SetMute(false, std::ptr::null());
                    }
                }
            }
            if !muted.is_empty() {
                eprintln!("[只听译文] 已恢复 {} 台设备的声音", muted.len());
            }
            let _ = std::fs::remove_file(marker_path());
            CoUninitialize();
        });
        Self { stop: Some(tx), handle: Some(handle) }
    }
}

impl Drop for DeviceMute {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 启动时调用：上次异常退出没来得及取消的设备静音，在这里取消
pub fn recover() {
    let Ok(text) = std::fs::read_to_string(marker_path()) else {
        return;
    };
    let mut restored = 0;
    unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if let Some(en) = enumerator() {
            for id in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if let Some(vol) = endpoint_volume(&en, id) {
                    if vol.SetMute(false, std::ptr::null()).is_ok() {
                        restored += 1;
                    }
                }
            }
        }
        CoUninitialize();
    }
    eprintln!("[只听译文] 上次没正常结束，已取消 {restored} 台设备的静音");
    let _ = std::fs::remove_file(marker_path());
}
