use std::sync::atomic::{AtomicBool, Ordering};

pub static MUTE_TTS: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn start() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    std::thread::spawn(|| unsafe {
        const MOD_ALT: u32 = 0x0001;
        const MOD_CONTROL: u32 = 0x0002;
        const VK_F8: u32 = 0x77;
        const VK_F9: u32 = 0x78;
        const VK_F10: u32 = 0x79;

        // Best effort: ignore failures when the combination is taken.
        let _ = RegisterHotKey(std::ptr::null_mut(), 1, MOD_CONTROL | MOD_ALT, VK_F9);
        let _ = RegisterHotKey(std::ptr::null_mut(), 2, MOD_CONTROL | MOD_ALT, VK_F10);
        let _ = RegisterHotKey(std::ptr::null_mut(), 3, MOD_CONTROL | MOD_ALT, VK_F8);

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
                    }
                    _ => {}
                }
            }
        }
    });
}

#[cfg(not(windows))]
pub fn start() {}




