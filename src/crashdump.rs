#![cfg(windows)]
//! 原生崩溃（访问违规等，Rust 的 panic 钩子管不到）时把进程现场存成 minidump，
//! 放在 数据目录/crashdumps/。配合打包时留下的 simulvoice.pdb 就能看到崩在哪个函数。
//!
//! 崩溃处理函数里尽量不分配内存（崩溃常常就是堆坏了）：文件路径在启动时就准备好。
//! 写完 dump 返回 EXCEPTION_CONTINUE_SEARCH，Windows 照常记录这次崩溃。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// 最多保留几份 dump（每份几 MB 到几十 MB）
const KEEP: usize = 5;

static DUMP_PATH: OnceLock<Vec<u16>> = OnceLock::new();
static IN_CRASH: AtomicBool = AtomicBool::new(false);

pub fn dir() -> PathBuf {
    crate::paths::data_dir().join("crashdumps")
}

/// 进程启动时调用一次
pub fn install() {
    let dir = dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[崩溃诊断] 无法创建 {}: {e}", dir.display());
        return;
    }
    report_previous(&dir);
    prune(&dir, KEEP - 1); // 给这次可能的崩溃留一个位置
    let name = format!(
        "crash-{}-v{}-pid{}.dmp",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );
    let wide: Vec<u16> = dir
        .join(name)
        .as_os_str()
        .encode_wide_vec();
    let _ = DUMP_PATH.set(wide);
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(filter));
    }
}

trait EncodeWideVec {
    fn encode_wide_vec(&self) -> Vec<u16>;
}

impl EncodeWideVec for std::ffi::OsStr {
    fn encode_wide_vec(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

fn dumps(dir: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "dmp").unwrap_or(false))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0)); // 新的在前
    out
}

/// 只留最新的 keep 份
fn prune(dir: &Path, keep: usize) {
    for (_, p) in dumps(dir).into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_file(&p) {
            eprintln!("[崩溃诊断] 清理旧文件失败 {}: {e}", p.display());
        }
    }
}

/// 上次运行崩溃过就在日志里提一句（用 .seen 标记，每份 dump 只提一次）
fn report_previous(dir: &Path) {
    for (_, p) in dumps(dir) {
        let seen = p.with_extension("seen");
        if seen.exists() {
            continue;
        }
        eprintln!("[崩溃诊断] 上次运行异常退出，现场已保存：{}", p.display());
        let _ = std::fs::write(&seen, b"");
    }
    // 对应的 dump 已被清理的 .seen 一并删掉
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().map(|x| x == "seen").unwrap_or(false) && !p.with_extension("dmp").exists() {
            let _ = std::fs::remove_file(p);
        }
    }
}

unsafe extern "system" fn filter(
    info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithProcessThreadData, MiniDumpWithThreadInfo,
        MiniDumpWithUnloadedModules, MiniDumpWriteDump, MINIDUMP_EXCEPTION_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    // 多个线程同时崩、或写 dump 时又崩：只写一次
    if IN_CRASH.swap(true, Ordering::SeqCst) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let Some(path) = DUMP_PATH.get() else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    let file = CreateFileW(
        path.as_ptr(),
        GENERIC_WRITE,
        0,
        std::ptr::null(),
        CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        std::ptr::null_mut(),
    );
    if file == INVALID_HANDLE_VALUE {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let mei = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: GetCurrentThreadId(),
        ExceptionPointers: info as *mut _,
        ClientPointers: 0,
    };
    MiniDumpWriteDump(
        GetCurrentProcess(),
        GetCurrentProcessId(),
        file,
        MiniDumpWithIndirectlyReferencedMemory
            | MiniDumpWithThreadInfo
            | MiniDumpWithProcessThreadData
            | MiniDumpWithUnloadedModules,
        &mei,
        std::ptr::null(),
        std::ptr::null(),
    );
    CloseHandle(file);
    EXCEPTION_CONTINUE_SEARCH
}

/// 开发用：`simulvoice crashtest` 故意访问空指针，检验 dump 能否生成
pub fn crash_now() -> ! {
    eprintln!("[崩溃诊断] 故意崩溃，dump 写到 {}", dir().display());
    unsafe {
        let p: *mut u32 = std::hint::black_box(std::ptr::null_mut());
        p.write_volatile(1);
    }
    unreachable!()
}
