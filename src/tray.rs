#![cfg(windows)]
//! 托盘图标：主窗口最小化时藏进托盘，点图标恢复。
//!
//! 用 Win32 直接显示/隐藏主窗口，不走 egui 的 ViewportCommand::Visible：
//! 窗口隐藏后 eframe 不再跑 update，靠 egui 自己把窗口叫回来会卡死。
//! 最小化用 SetWinEventHook 监听（事件驱动，不轮询）。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW};

const WM_APP: u32 = 0x8000;
const WM_COMMAND: u32 = 0x0111;
const WM_CLOSE: u32 = 0x0010;
const WM_NULL: u32 = 0x0000;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const NIF_INFO: u32 = 0x10;
const NIIF_INFO: u32 = 1;
const NIIF_NOSOUND: u32 = 0x10;
const MF_STRING: u32 = 0;
const MF_SEPARATOR: u32 = 0x800;
const MF_CHECKED: u32 = 8;
const TPM_RIGHTBUTTON: u32 = 2;
const TPM_BOTTOMALIGN: u32 = 0x20;
const SW_HIDE: i32 = 0;
const SW_RESTORE: i32 = 9;
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const WINEVENT_OUTOFCONTEXT: u32 = 0;
const TRAY_CALLBACK: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;
const ID_SHOW: usize = 1;
const ID_MUTE: usize = 2;
const ID_QUIT: usize = 3;

/// 主窗口（eframe 设置窗口）。0 = 还没拿到（命令行模式一直是 0）
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// 托盘消息窗口
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
/// 最小化时藏进托盘（设置里可关）
pub static MINIMIZE_TO_TRAY: AtomicBool = AtomicBool::new(true);
/// 第一次藏进托盘时弹一句提示，免得以为程序没了
static HINTED: AtomicBool = AtomicBool::new(false);
/// 资源管理器重启后托盘会清空，要重新加图标
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    let w = wide(s);
    let n = w.len().min(dst.len());
    dst[..n].copy_from_slice(&w[..n]);
    if let Some(last) = dst.last_mut() {
        *last = 0;
    }
}

/// 启动托盘线程（只启动一次）
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| unsafe {
        if let Err(e) = run() {
            eprintln!("[tray] {e}");
            STARTED.store(false, Ordering::SeqCst);
        }
    });
}

/// 设置窗口创建后登记它的句柄
pub fn set_main_window(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
}

/// 程序退出前移除托盘图标（否则任务栏会留一个"幽灵图标"，鼠标划过才消失）
pub fn shutdown() {
    let tray = TRAY_HWND.swap(0, Ordering::SeqCst);
    if tray == 0 {
        return;
    }
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = tray as HWND;
        nid.uID = TRAY_UID;
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        // 让托盘线程的消息循环结束
        let _ = PostMessageW(tray as HWND, WM_CLOSE, 0, 0);
    }
}

/// 把主窗口叫回来并放到最前
pub fn show_main() {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd == 0 {
        return;
    }
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow};
        ShowWindow(hwnd as HWND, SW_RESTORE);
        SetForegroundWindow(hwnd as HWND);
    }
}

fn hide_main(hwnd: HWND) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow;
        ShowWindow(hwnd, SW_HIDE);
    }
    if !HINTED.swap(true, Ordering::SeqCst) {
        balloon("SimulVoice 还在运行", "已收到右下角托盘，翻译不会中断。点托盘图标可以打开窗口。");
    }
}

fn base_nid() -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = TRAY_HWND.load(Ordering::SeqCst) as HWND;
    nid.uID = TRAY_UID;
    nid
}

fn balloon(title: &str, text: &str) {
    if TRAY_HWND.load(Ordering::SeqCst) == 0 {
        return;
    }
    let mut nid = base_nid();
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
    copy_into(&mut nid.szInfoTitle, title);
    copy_into(&mut nid.szInfo, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// 32×32 应用图标（和窗口、exe 图标同一张图，见 icon.rs）
unsafe fn make_icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, LoadIconW, ICONINFO, IDI_APPLICATION};
    let size = 32u32;
    let rgba = crate::icon::app_icon_rgba(size);
    let bgra: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let mask = vec![0u8; (size * size / 8) as usize];
    let color = CreateBitmap(size as i32, size as i32, 1, 32, bgra.as_ptr() as _);
    let mono = CreateBitmap(size as i32, size as i32, 1, 1, mask.as_ptr() as _);
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mono, hbmColor: color };
    let icon = CreateIconIndirect(&info);
    DeleteObject(color);
    DeleteObject(mono);
    if icon.is_null() {
        LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}

unsafe fn add_icon(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> bool {
    let mut nid = base_nid();
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = TRAY_CALLBACK;
    nid.hIcon = icon;
    copy_into(&mut nid.szTip, "SimulVoice 同声传译");
    Shell_NotifyIconW(NIM_ADD, &nid) != 0
}

static ICON: AtomicIsize = AtomicIsize::new(0);

unsafe fn run() -> Result<(), String> {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyIcon, DispatchMessageW, GetMessageW, RegisterClassW,
        RegisterWindowMessageW, TranslateMessage, MSG, WNDCLASSW,
    };

    let hinstance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("simulvoice_tray");
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    if RegisterClassW(&wc) == 0 {
        return Err("RegisterClassW 失败".into());
    }
    let title = wide("simulvoice tray");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
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
    TRAY_HWND.store(hwnd as isize, Ordering::SeqCst);
    TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::SeqCst);

    let icon = make_icon();
    ICON.store(icon as isize, Ordering::SeqCst);
    if !add_icon(icon) {
        TRAY_HWND.store(0, Ordering::SeqCst);
        return Err("Shell_NotifyIconW(NIM_ADD) 失败".into());
    }

    // 只听本进程窗口的最小化事件
    let hook = SetWinEventHook(
        EVENT_SYSTEM_MINIMIZESTART,
        EVENT_SYSTEM_MINIMIZESTART,
        std::ptr::null_mut(),
        Some(on_minimize),
        std::process::id(),
        0,
        WINEVENT_OUTOFCONTEXT,
    );

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    if !hook.is_null() {
        UnhookWinEvent(hook);
    }
    shutdown();
    DestroyIcon(icon);
    Ok(())
}

unsafe extern "system" fn on_minimize(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let main = MAIN_HWND.load(Ordering::SeqCst);
    if main != 0 && hwnd as isize == main && MINIMIZE_TO_TRAY.load(Ordering::SeqCst) {
        hide_main(hwnd);
    }
}

unsafe fn show_menu(hwnd: HWND) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, PostMessageW, SetForegroundWindow,
        TrackPopupMenu,
    };
    let menu = CreatePopupMenu();
    let has_main = MAIN_HWND.load(Ordering::SeqCst) != 0;
    let show = wide("打开窗口");
    let mute = wide("静音译文");
    let quit = wide("退出");
    if has_main {
        AppendMenuW(menu, MF_STRING, ID_SHOW, show.as_ptr());
    }
    let muted = crate::hotkeys::MUTE_TTS.load(Ordering::SeqCst);
    AppendMenuW(menu, MF_STRING | if muted { MF_CHECKED } else { 0 }, ID_MUTE, mute.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, ID_QUIT, quit.as_ptr());
    let mut pt: POINT = std::mem::zeroed();
    let _ = GetCursorPos(&mut pt);
    // 不先把托盘窗口设为前台，点菜单外面时菜单不会消失（Win32 老毛病）
    SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null());
    let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DefWindowProcW, DestroyWindow, PostMessageW, PostQuitMessage};
    let taskbar_created = TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar_created != 0 && msg == taskbar_created {
        add_icon(ICON.load(Ordering::SeqCst) as _);
        return 0;
    }
    match msg {
        TRAY_CALLBACK => {
            match (lparam as u32) & 0xffff {
                WM_LBUTTONUP => show_main(),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            match wparam & 0xffff {
                ID_SHOW => show_main(),
                ID_MUTE => {
                    let next = !crate::hotkeys::MUTE_TTS.load(Ordering::SeqCst);
                    crate::hotkeys::MUTE_TTS.store(next, Ordering::SeqCst);
                }
                ID_QUIT => {
                    let main = MAIN_HWND.load(Ordering::SeqCst);
                    if main != 0 {
                        // 走窗口自己的关闭流程（保存设置、停止翻译）。隐藏的窗口 eframe 不跑 update，先叫回来
                        show_main();
                        let _ = PostMessageW(main as HWND, WM_CLOSE, 0, 0);
                    } else {
                        crate::STOP.store(true, Ordering::SeqCst);
                    }
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
