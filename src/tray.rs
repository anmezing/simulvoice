#![cfg(windows)]

use std::sync::atomic::Ordering;

const WM_APP: u32 = 0x8000;
const WM_COMMAND: u32 = 0x0111;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const NIM_ADD: u32 = 0;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const MF_STRING: u32 = 0;
const TPM_RIGHTBUTTON: u32 = 2;
const TPM_BOTTOMALIGN: u32 = 0x20;
const TRAY_CALLBACK: u32 = WM_APP + 1;
const ID_MUTE: usize = 1;
const ID_QUIT: usize = 2;

pub fn start() {
    std::thread::spawn(|| unsafe {
        if let Err(e) = run() {
            eprintln!("[tray] {e}");
        }
    });
}

unsafe fn run() -> Result<(), String> {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, LoadIconW,
        RegisterClassW, TranslateMessage, IDI_APPLICATION, MSG, WNDCLASSW,
    };

    let hinstance = GetModuleHandleW(std::ptr::null());
    let class_name: Vec<u16> = "simulvoice_tray\0".encode_utf16().collect();
    let window_name: Vec<u16> = "simulvoice\0".encode_utf16().collect();

    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    if RegisterClassW(&wc) == 0 {
        return Err("RegisterClassW 失败".into());
    }

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        window_name.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return Err("CreateWindowExW 失败".into());
    }

    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = TRAY_CALLBACK;
    nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
    let tip: Vec<u16> = "SimulVoice 实时同传\0".encode_utf16().collect();
    let n = tip.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&tip[..n]);

    if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
        return Err("Shell_NotifyIconW(NIM_ADD) 失败".into());
    }
    eprintln!("[tray] 托盘图标已添加（双击静音，右键菜单退出）");

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if crate::STOP.load(Ordering::SeqCst) {
            break;
        }
    }
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    Ok(())
}

unsafe extern "system" fn wndproc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DefWindowProcW, DestroyMenu, GetCursorPos,
        TrackPopupMenu,
    };
    use windows_sys::Win32::Foundation::POINT;

    match msg {
        TRAY_CALLBACK => {
            let event = (lparam as u32) & 0xffff;
            if event == WM_RBUTTONUP {
                let menu = CreatePopupMenu();
                let m1: Vec<u16> = "静音/恢复译文\0".encode_utf16().collect();
                let m2: Vec<u16> = "退出\0".encode_utf16().collect();
                AppendMenuW(menu, MF_STRING, ID_MUTE, m1.as_ptr());
                AppendMenuW(menu, MF_STRING, ID_QUIT, m2.as_ptr());
                let mut pt: POINT = std::mem::zeroed();
                let _ = GetCursorPos(&mut pt);
                let _ = TrackPopupMenu(
                    menu,
                    TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                    pt.x,
                    pt.y,
                    0,
                    hwnd,
                    std::ptr::null(),
                );
                DestroyMenu(menu);
            } else if event == WM_LBUTTONDBLCLK {
                let next = !crate::hotkeys::MUTE_TTS.load(Ordering::SeqCst);
                crate::hotkeys::MUTE_TTS.store(next, Ordering::SeqCst);
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xffff {
                ID_MUTE => {
                    let next = !crate::hotkeys::MUTE_TTS.load(Ordering::SeqCst);
                    crate::hotkeys::MUTE_TTS.store(next, Ordering::SeqCst);
                }
                ID_QUIT => {
                    crate::STOP.store(true, Ordering::SeqCst);
                    crate::hotkeys::MUTE_TTS.store(false, Ordering::SeqCst);
                }
                _ => {}
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}




