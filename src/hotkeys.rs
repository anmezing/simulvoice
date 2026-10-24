use std::sync::atomic::{AtomicBool, Ordering};

pub static MUTE_TTS: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn start() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    start_ptt_hook();
    std::thread::spawn(|| unsafe {
        const MOD_ALT: u32 = 0x0001;
        const MOD_CONTROL: u32 = 0x0002;
        const VK_F6: u32 = 0x75;
        const VK_F8: u32 = 0x77;
        const VK_F9: u32 = 0x78;
        const VK_F10: u32 = 0x79;

        // Best effort: ignore failures when the combination is taken.
        let _ = RegisterHotKey(std::ptr::null_mut(), 1, MOD_CONTROL | MOD_ALT, VK_F9);
        let _ = RegisterHotKey(std::ptr::null_mut(), 2, MOD_CONTROL | MOD_ALT, VK_F10);
        let _ = RegisterHotKey(std::ptr::null_mut(), 3, MOD_CONTROL | MOD_ALT, VK_F8);
        let _ = RegisterHotKey(std::ptr::null_mut(), 4, MOD_CONTROL | MOD_ALT, VK_F6);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_HOTKEY {
                match msg.wParam as i32 {
                    1 => {
                        let next = !MUTE_TTS.load(Ordering::SeqCst);
                        MUTE_TTS.store(next, Ordering::SeqCst);
                        eprintln!("[hotkey] 译文朗读 {}", if next { "已静音" } else { "已恢复" });
                    }
                    2 => {
                        eprintln!("[hotkey] 停止");
                        crate::STOP.store(true, Ordering::SeqCst);
                    }
                    3 => {
                        eprintln!("[hotkey] 停止当前译文");
                        crate::STOP_TTS.store(true, Ordering::SeqCst);
                        crate::STOP_TTS_GEN.fetch_add(1, Ordering::SeqCst);
                    }
                    4 => {
                        std::thread::spawn(|| {
                            if let Err(e) = crate::recorder::replay_latest() {
                                eprintln!("[hotkey] 回放失败: {e}");
                            }
                        });
                    }
                    _ => {}
                }
            }
        }
    });
}

#[cfg(windows)]
fn start_ptt_hook() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    unsafe extern "system" fn hook(code: i32, wparam: usize, lparam: isize) -> isize {
        if code >= 0 {
            let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
            let vk = crate::PTT_VK.load(Ordering::SeqCst);
            let ctrl_ok = !crate::PTT_REQ_CTRL.load(Ordering::SeqCst)
                || GetAsyncKeyState(VK_CONTROL as i32) < 0;
            let alt_ok = !crate::PTT_REQ_ALT.load(Ordering::SeqCst)
                || GetAsyncKeyState(VK_MENU as i32) < 0;
            if kb.vkCode == vk && ctrl_ok && alt_ok {
                let mode = crate::VOICE_MODE.load(Ordering::SeqCst);
                match wparam as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        if mode == 1 {
                            let now = crate::PTT_ACTIVE.load(Ordering::SeqCst);
                            crate::PTT_ACTIVE.store(!now, Ordering::SeqCst);
                        } else if mode >= 2 {
                            crate::PTT_ACTIVE.store(true, Ordering::SeqCst);
                        }
                    }
                    WM_KEYUP | WM_SYSKEYUP => {
                        if mode >= 2 {
                            crate::PTT_ACTIVE.store(false, Ordering::SeqCst);
                        }
                    }
                    _ => {}
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    std::thread::spawn(|| unsafe {
        let installed = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), std::ptr::null_mut(), 0);
        if installed.is_null() {
            eprintln!("[hotkey] PTT 键盘钩子安装失败");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
    });
}

#[cfg(not(windows))]
pub fn start() {}

/// Maps a config key name to a Windows virtual-key code.
pub fn vk_code(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_uppercase();
    let n = n.strip_prefix("VK_").unwrap_or(&n);
    if let Some(num) = n.strip_prefix('F').and_then(|s| s.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0x70 + num - 1);
        }
    }
    if n.len() == 1 {
        let b = n.as_bytes()[0];
        if b.is_ascii_alphanumeric() {
            return Some(b as u32);
        }
    }
    Some(match n {
        "SPACE" => 0x20,
        "TAB" => 0x09,
        "ENTER" => 0x0D,
        "ESC" | "ESCAPE" => 0x1B,
        "PAUSE" => 0x13,
        "SCROLLLOCK" | "SCROLL" => 0x91,
        "INSERT" => 0x2D,
        "DELETE" | "DEL" => 0x2E,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" | "PGUP" => 0x21,
        "PAGEDOWN" | "PGDN" => 0x22,
        "UP" => 0x26,
        "DOWN" => 0x28,
        "LEFT" => 0x25,
        "RIGHT" => 0x27,
        "`" => 0xC0,
        _ => return None,
    })
}












