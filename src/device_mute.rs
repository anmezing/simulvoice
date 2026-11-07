//! 声音设备查询：会议软件在哪台设备上出声、用哪个麦克风，哪些是虚拟设备。
//!
//! 0.45.0 的「只听译文」曾经把会议软件那台播放设备整体静音（muted-devices.txt 记录）。
//! 现在改成只把会议软件自己的音量关到听不见（见 ducking::Hush），这里只保留恢复旧记录的 recover。

use std::collections::HashSet;

use windows::core::{Interface, PWSTR};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, AudioSessionStateActive, EDataFlow, IAudioSessionControl2, IAudioSessionManager2,
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// 虚拟声卡（VB-CABLE、VoiceMeeter 等）：不会从喇叭里放出来
pub fn is_virtual(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["CABLE", "VB-AUDIO", "VOICEMEETER"].iter().any(|k| n.contains(k))
}

/// 两个虚拟设备是不是同一根虚拟线（"CABLE In 16ch (VB-Audio Virtual Cable)" 和
/// "CABLE Output (VB-Audio Virtual Cable)" 是同一根：往前者放的声音会从后者出来）
pub fn same_cable(a: &str, b: &str) -> bool {
    fn driver(n: &str) -> Option<&str> {
        let start = n.rfind('(')?;
        Some(n[start..].trim())
    }
    is_virtual(a) && is_virtual(b) && driver(a).is_some() && driver(a) == driver(b)
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
    sessions_on(tree, eRender)
}

/// 这组进程正在用哪些麦克风（录音设备）
pub fn inputs_of(tree: &HashSet<u32>) -> Vec<OutputDevice> {
    sessions_on(tree, eCapture)
}

/// 第一个真实（非虚拟）的麦克风名字
pub fn physical_input_name() -> Option<String> {
    unsafe {
        let en = enumerator()?;
        let devices = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).ok()?;
        (0..devices.GetCount().unwrap_or(0))
            .filter_map(|i| devices.Item(i).ok())
            .filter_map(|d| describe(&d))
            .map(|d| d.name)
            .find(|n| !is_virtual(n))
    }
}

/// 排查用：这组进程在每台设备上的会话（设备名、进程、状态）
pub fn session_report(tree: &HashSet<u32>) -> Vec<String> {
    let mut lines = Vec::new();
    unsafe {
        let Some(en) = enumerator() else { return lines };
        for (flow, kind) in [(eRender, "播放"), (eCapture, "录音")] {
            let Ok(devices) = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { continue };
            for i in 0..devices.GetCount().unwrap_or(0) {
                let Ok(device) = devices.Item(i) else { continue };
                let name = describe(&device).map(|d| d.name).unwrap_or_default();
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
                for s in 0..sessions.GetCount().unwrap_or(0) {
                    let Ok(ctrl) = sessions.GetSession(s) else { continue };
                    let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else { continue };
                    let pid = ctrl2.GetProcessId().unwrap_or(0);
                    if tree.contains(&pid) {
                        let state = ctrl.GetState().map(|st| st.0).unwrap_or(-1);
                        let volume = ctrl
                            .cast::<windows::Win32::Media::Audio::ISimpleAudioVolume>()
                            .and_then(|v| v.GetMasterVolume())
                            .map(|v| format!("{v:.4}"))
                            .unwrap_or_default();
                        lines.push(format!(
                            "{kind}  {name}  进程 {pid}  状态 {state}（0 不活动 1 活动 2 已过期）  音量 {volume}"
                        ));
                    }
                }
            }
        }
    }
    lines
}

fn sessions_on(tree: &HashSet<u32>, flow: EDataFlow) -> Vec<OutputDevice> {
    unsafe {
        let Some(en) = enumerator() else { return Vec::new() };
        let Ok(devices) = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else {
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

/// 启动时、窗口关闭后、关机/覆盖安装前调用：没来得及取消的设备静音，在这里取消
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
    eprintln!("[只听译文] 有没恢复的记录（上次异常退出或翻译中关闭），已取消 {restored} 台设备的静音");
    let _ = std::fs::remove_file(marker_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_devices() {
        assert!(is_virtual("CABLE In 16ch (VB-Audio Virtual Cable)"));
        assert!(is_virtual("VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)"));
        assert!(!is_virtual("扬声器 (Realtek(R) Audio)"));
        assert!(!is_virtual("耳机 (HyperX Virtual Surround Sound)"));
    }

    #[test]
    fn same_virtual_cable() {
        let out = "CABLE Output (VB-Audio Virtual Cable)";
        assert!(same_cable("CABLE In 16ch (VB-Audio Virtual Cable)", out));
        assert!(same_cable("CABLE Input (VB-Audio Virtual Cable)", out));
        assert!(!same_cable("CABLE-A Input (VB-Audio Cable A)", out));
        assert!(!same_cable("扬声器 (Realtek(R) Audio)", "麦克风 (Realtek(R) Audio)"));
    }
}
