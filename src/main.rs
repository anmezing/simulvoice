mod aec;
mod capture;
mod config;
mod dsp;
mod ducking;
mod edge_tts;
mod gemini_live;
mod hotkeys;
mod http;
mod keys;
mod livetest;
mod marian;
mod recorder;
mod palabra;
mod paths;
mod palabratest;
mod neural_aec;
mod models;
mod silero_vad;
mod sink;
mod stt;
mod translate;
mod tray;
mod tts;
mod ui;
mod vad;
mod volc;
mod volctest;

use anyhow::{anyhow, Result};
use dsp::Resampler16k;
use config::Config;
use edge_tts::EdgeTts;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;

pub static STOP: AtomicBool = AtomicBool::new(false);
pub static DUCK: AtomicBool = AtomicBool::new(true);
pub static BARGE_IN: AtomicBool = AtomicBool::new(false);
pub static STOP_TTS: AtomicBool = AtomicBool::new(false);
pub static STOP_TTS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static QUEUE_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(32);
pub static PTT_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 0=auto 1=toggle 2=hold 3=hold_translate
pub static VOICE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub static PTT_VK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x76);
pub static PTT_REQ_CTRL: AtomicBool = AtomicBool::new(true);
pub static PTT_REQ_ALT: AtomicBool = AtomicBool::new(true);
pub static PTT_DOWN: AtomicBool = AtomicBool::new(false);
pub static ECHO_GUARD: AtomicBool = AtomicBool::new(false);
pub static TTS_PLAYING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static RT_PLAYING: AtomicBool = AtomicBool::new(false);
pub static AEC_ON: AtomicBool = AtomicBool::new(false);
pub static ONNX_PROVIDER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub static VAD_THREADS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
pub static TTS_BACKLOG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static NEURAL_AEC: AtomicBool = AtomicBool::new(false);
pub static AEC_RUNNING: AtomicBool = AtomicBool::new(false);
pub static JUMP_SECS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static CROSS_BARGE: AtomicBool = AtomicBool::new(false);
pub static MUTE_IN: AtomicBool = AtomicBool::new(false);
pub static MUTE_OUT: AtomicBool = AtomicBool::new(false);
pub static INBOUND_SINK: std::sync::OnceLock<std::sync::Arc<rodio::Sink>> = std::sync::OnceLock::new();
pub static OUTBOUND_SINK: std::sync::OnceLock<std::sync::Arc<rodio::Sink>> = std::sync::OnceLock::new();
pub static NEURAL_AEC_DELAY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static NEURAL_AEC_MODEL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static TRANSCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn log_transcript(side: &str, src: &str, dst: &str) {
    use std::io::Write;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().ok();
    let Some(path) = TRANSCRIPT.get() else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let ts = chrono::Local::now().format("%H:%M:%S");
        let _ = writeln!(f, "[{ts}] {side} | {src} => {dst}");
    }
}
use std::time::Duration;
use vad::{Vad, VadEngine, VadEvent};

fn preset(args: &[String]) -> Result<()> {
    let name = args.get(1).map(String::as_str).unwrap_or("");
    let path = flag(args, "--config").unwrap_or_else(|| "simulvoice.toml".into());
    let mut cfg = Config::default();
    config::apply_preset(&mut cfg, name)?;
    if name == "teams" {
        let (cable, headset) =
            sink::auto_pick_teams(&mut cfg.audio.output_device, &mut cfg.audio.remote_output_device);
        if !cable {
            println!("未找到 CABLE 虚拟声卡：请先安装 VB-Audio Cable");
        }
        if !headset {
            println!("未找到与默认输出不同的耳机设备：请连接耳机并在设置里选择");
        }
    }
    cfg.save(&path)?;
    println!("已写入 {path}");
    match name {
        "teams" => println!("Teams 双向：麦克风选 CABLE Output，务必把 output_device 设为耳机（留空会自激）"),
        "game" => println!("游戏听译：input=loopback 或 pid，output_device 建议耳机"),
        "record" => println!("会议记录：仅字幕 + transcript.txt"),
        _ => {}
    }
    Ok(())
}
fn doctor(args: &[String]) -> Result<()> {
    let path = flag(args, "--config");
    let cfg = Config::load(path.as_deref())?;
    println!("配置: {}", path.clone().unwrap_or_else(|| "<默认>".into()));
    for (name, env) in [
        ("OpenAI", "OPENAI_API_KEY"),
        ("Gemini", "GEMINI_API_KEY"),
        ("Soniox", "SONIOX_API_KEY"),
        ("Palabra", "PALABRA_API_KEY"),
        ("火山 AppID", "VOLC_APP_ID"),
        ("火山 Token", "VOLC_ACCESS_TOKEN"),
    ] {
        println!("  {} {}", if std::env::var(env).is_ok() { "✓" } else { "-" }, name);
    }
    let mut missing = Vec::new();
    match cfg.stt.provider.as_str() {
        "soniox" if std::env::var("SONIOX_API_KEY").is_err() => {
            missing.push("STT=soniox 需要 SONIOX_API_KEY".to_string())
        }
        "openai" if std::env::var("OPENAI_API_KEY").is_err() => {
            missing.push("STT=openai 需要 OPENAI_API_KEY".to_string())
        }
        "vosk" => {
            let p = cfg.stt.model_path.trim();
            let p = if p.is_empty() {
                config::default_models_dir().join("vosk")
            } else {
                std::path::PathBuf::from(p)
            };
            if !p.exists() {
                missing.push(format!("STT=vosk 模型不存在: {}", p.display()));
            }
        }
        _ => {}
    }
    match cfg.translate.provider.as_str() {
        "openai" if std::env::var("OPENAI_API_KEY").is_err() => {
            missing.push("翻译=openai 需要 OPENAI_API_KEY".to_string())
        }
        "gemini" if std::env::var("GEMINI_API_KEY").is_err() => {
            missing.push("翻译=gemini 需要 GEMINI_API_KEY".to_string())
        }
        "qwen" if std::env::var("DASHSCOPE_API_KEY").is_err() => {
            missing.push("翻译=qwen 需要 DASHSCOPE_API_KEY".to_string())
        }
        "custom" if std::env::var("CUSTOM_API_KEY").is_err() => {
            missing.push("翻译=custom 需要 CUSTOM_API_KEY".to_string())
        }
        _ => {}
    }
    match cfg.tts.provider.as_str() {
        "openai" if std::env::var("OPENAI_API_KEY").is_err() => {
            missing.push("TTS=openai 需要 OPENAI_API_KEY".to_string())
        }
        "sherpa" => {
            let d = if cfg.tts.model_dir.trim().is_empty() {
                config::default_models_dir().join("tts")
            } else {
                std::path::PathBuf::from(cfg.tts.model_dir.trim())
            };
            if !d.exists() {
                missing.push(format!("TTS=sherpa 模型目录不存在: {}", d.display()));
            }
        }
        _ => {}
    }
    match cfg.realtime.provider.as_str() {
        "gemini-live" if std::env::var("GEMINI_API_KEY").is_err() => {
            missing.push("实时=gemini-live 需要 GEMINI_API_KEY".to_string())
        }
        "palabra" if std::env::var("PALABRA_API_KEY").is_err() => {
            missing.push("实时=palabra 需要 PALABRA_API_KEY".to_string())
        }
        "volcengine" => {
            if std::env::var("VOLC_APP_ID").is_err() {
                missing.push("实时=volcengine 缺少 VOLC_APP_ID".to_string());
            }
            if std::env::var("VOLC_ACCESS_TOKEN").is_err() {
                missing.push("实时=volcengine 缺少 VOLC_ACCESS_TOKEN".to_string());
            }
        }
        _ => {}
    }
    if missing.is_empty() {
        println!("  ✓ 所选引擎所需密钥/模型齐全");
    } else {
        for m in missing {
            println!("  ✗ {m}");
        }
    }
    let models = config::default_models_dir();    for (label, p) in [
        ("Vosk 模型", models.join("vosk")),
        ("Silero VAD", models.join("silero_vad_v5.onnx")),
        ("Piper TTS", models.join("tts")),
    ] {
        println!("  {} {}", if p.exists() { "✓" } else { "-" }, label);
    }
    println!("输出设备:");
    for n in sink::list_output_devices()? {
        println!("  - {n}");
    }
    println!("输入设备:");
    for n in sink::list_input_devices()? {
        println!("  - {n}");
    }
    if args.iter().any(|a| a == "--online") {
        if let Ok(k) = std::env::var("OPENAI_API_KEY") {
            match crate::http::agent()
                .get("https://api.openai.com/v1/models")
                .set("Authorization", &format!("Bearer {k}"))
                .call()
            {
                Ok(_) => println!("  ✓ OpenAI Key 有效"),
                Err(ureq::Error::Status(code, _)) => println!("  ✗ OpenAI Key 无效 (HTTP {code})"),
                Err(e) => println!("  - OpenAI 校验失败: {e}"),
            }
        }
        if let Ok(k) = std::env::var("GEMINI_API_KEY") {
            let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-flash-latest:generateContent";
            match crate::http::agent()
                .post(url)
                .set("Content-Type", "application/json")
                .set("x-goog-api-key", &k)
                .send_string(r#"{"contents":[{"parts":[{"text":"ping"}]}],"generationConfig":{"maxOutputTokens":1}}"#)
            {
                Ok(_) => println!("  ✓ Gemini Key 可用（生成调用成功）"),
                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    if body.contains("API key not valid") || body.contains("API_KEY_INVALID") {
                        println!("  ✗ Gemini Key 无效 (HTTP {code})");
                    } else if code == 404 {
                        println!("  - Gemini 模型不可用，Key 有效性未知 (HTTP 404)");
                    } else {
                        println!("  ✗ Gemini 生成失败 (HTTP {code})");
                    }
                }
                Err(e) => println!("  - Gemini 校验失败: {e}"),
            }
        }
        if let Ok(k) = std::env::var("SONIOX_API_KEY") {
            match crate::http::agent()
                .get("https://api.soniox.com/v1/models")
                .set("Authorization", &format!("Bearer {k}"))
                .call()
            {
                Ok(_) => println!("  ✓ Soniox Key 有效"),
                Err(ureq::Error::Status(code, _)) => println!("  ✗ Soniox Key 无效 (HTTP {code})"),
                Err(e) => println!("  - Soniox 校验失败: {e}"),
            }
        }
    }

    if cfg.audio.input == "loopback" && !cfg.audio.output_device.trim().is_empty() {
        if let Some(def) = sink::default_output_name() {
            let a = cfg.audio.output_device.to_lowercase();
            let b = def.to_lowercase();
            if a.contains(&b) || b.contains(&a) {
                println!("  ✗ 输出设备与采集设备相同，会自激循环");
            } else {
                println!("  ✓ 输出设备与采集设备不同");
            }
        }
    }
    if cfg.audio.duplex {
        let out = cfg.audio.output_device.trim();
        let rem = cfg.audio.remote_output_device.trim();
        if out.is_empty() {
            println!("  ! 双向同传的输出是系统默认设备：外放会被麦克风拾取，建议戴耳机");
        }
        if !rem.is_empty() && out.eq_ignore_ascii_case(rem) {
            println!("  ✗ 入向输出与出向虚拟麦是同一设备");
        }
        if cfg.audio.voice_mode == "hold_translate" && rem.is_empty() {
            println!("  ! 按住翻译需要配置 remote_output_device（虚拟麦），否则直通会走默认设备");
        }
    }
    Ok(())
}

static INSTANCE_LISTENER: std::sync::OnceLock<std::net::TcpListener> = std::sync::OnceLock::new();
static CHILD_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(windows)]
fn foreground_exe() -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        let _ = CloseHandle(h);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .map(|s| s.to_ascii_lowercase())
    }
}

#[cfg(not(windows))]
fn foreground_exe() -> Option<String> {
    None
}

#[cfg(windows)]
unsafe extern "system" fn watch_ctrl(_kind: u32) -> i32 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid != 0 {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 130);
            let _ = CloseHandle(h);
        }
    }
    0
}

fn spawn_watch_child(config: Option<&std::path::Path>) -> Result<std::process::Child> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("run");
    if let Some(c) = config {
        cmd.arg("--config").arg(c);
    }
    let child = cmd.spawn()?;
    CHILD_PID.store(child.id(), Ordering::SeqCst);
    Ok(child)
}

fn watch(args: &[String]) -> Result<()> {
    let base_arg = flag(args, "--config");
    let base_path = base_arg
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let p = paths::default_config_path();
            if p.exists() {
                Some(p)
            } else {
                None
            }
        });
    let cfg = Config::load(base_arg.as_deref())?;
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(watch_ctrl), 1);
    }
    let base_dir = base_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(paths::data_dir);
    let mut child = spawn_watch_child(base_path.as_deref())?;
    let mut current = base_path.clone();
    let mut last_exe: Option<String> = None;
    let self_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    loop {
        std::thread::sleep(Duration::from_millis(700));
        if let Ok(Some(status)) = child.try_wait() {
            CHILD_PID.store(0, Ordering::SeqCst);
            if status.code() == Some(0) {
                eprintln!("[watch] 子进程正常退出，停止监视");
                break;
            }
            eprintln!("[watch] 子进程退出（{status}），2 秒后重启");
            std::thread::sleep(Duration::from_secs(2));
            child = spawn_watch_child(current.as_deref())?;
            continue;
        }
        let Some(exe) = foreground_exe() else { continue };
        if last_exe.as_deref() == Some(exe.as_str()) {
            continue;
        }
        last_exe = Some(exe.clone());
        if self_exe.as_deref() == Some(exe.as_str()) {
            continue;
        }
        let mapped = cfg
            .profiles
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&exe))
            .map(|(_, v)| v.clone());
        let target = match mapped {
            Some(v) => {
                let p = std::path::PathBuf::from(&v);
                let p = if p.is_absolute() { p } else { base_dir.join(p) };
                if !p.exists() {
                    eprintln!("[watch] 配置不存在: {}", p.display());
                    continue;
                }
                Some(p)
            }
            None => base_path.clone(),
        };
        if target == current {
            continue;
        }
        eprintln!(
            "[watch] 切换到 {}",
            target
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<默认>".into())
        );
        let _ = child.kill();
        let _ = child.wait();
        CHILD_PID.store(0, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(400));
        child = spawn_watch_child(target.as_deref())?;
        current = target;
    }
    Ok(())
}


fn single_instance() -> bool {
    match std::net::TcpListener::bind(("127.0.0.1", 47653)) {
        Ok(l) => {
            let _ = INSTANCE_LISTENER.set(l);
            true
        }
        Err(_) => false,
    }
}
fn help() {
    println!(
        r#"SimulVoice 0.35.0 — 实时同传（Rust 原生）

用法:
  simulvoice run [--config <文件>]            同传：采集 → 识别 → 翻译 → 朗读
  simulvoice watch [--config <文件>]          按前台进程自动切换 [profiles] 配置
                                             （配置 audio.duplex=true 后双向）
  simulvoice devices                          列出输出/输入设备
  simulvoice models list                      列出可下载的本地模型
  simulvoice models vosk <cn|en|ru|ja|ko|de|fr|es|cn-big|en-big>
  simulvoice models silero                    Silero VAD 模型（本地识别用）
  simulvoice models sensevoice                SenseVoice int8 本地识别模型（ONNX）
  simulvoice models piper <zh|en>              Piper 本地 TTS 模型（离线）
  simulvoice vadcheck <音频>                  检查 Silero VAD 切分
  simulvoice livetest <音频>                  Gemini Live 端到端翻译测试
  simulvoice palabratest <音频>               Palabra.ai 端到端翻译测试
  simulvoice volctest <音频>                  火山引擎 Doubao 翻译测试
  simulvoice config-init [文件]               生成配置模板
  simulvoice settings [--config <文件>]       图形化设置面板
  simulvoice preset <teams|game|record>       生成场景预设配置
  simulvoice doctor [--config <文件>] [--online]  自检设备/密钥/模型/路由/Key 有效性
  simulvoice keys encrypt|test [--config]     DPAPI 加密 keys.toml / 自测
  simulvoice say <文本> [--local] [--voice] [--out a.mp3]
  simulvoice replay                           回放最近一次保存的译文
  simulvoice translate <文本> [--to <语言>]
  simulvoice transcribe <音频> [--config]     离线识别音频文件

双向开会（Teams/Zoom/游戏）:
  [audio] duplex = true
          mic_device = ""                  # 你的麦克风
          remote_output_device = "CABLE"   # 对方听到的译文 → 虚拟麦克风
          output_device = "耳机"            # 你听到的译文（必须与采集设备不同）
  会议软件里：麦克风选 CABLE Output；扬声器选被你采集的那个设备。

本地 / 在线 / 混合：编辑 simulvoice.toml 的 [stt] [translate] [tts]。
"#
    );
}

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let path = dirs::home_dir()
            .unwrap_or_default()
            .join("simulvoice")
            .join("crash.log");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{}\n{info}\n", chrono::Local::now());
        }
        default_hook(info);
    }));
    paths::ensure_layout();
    keys::load_env();
    hotkeys::start();
    let _ = ctrlc::set_handler(|| {
        STOP.store(true, Ordering::SeqCst);
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("say") => say(&args),
        Some("translate") => translate_once(&args),
        Some("transcribe") => transcribe_file(&args),
        Some("--version") | Some("-V") | Some("version") => {
            println!("SimulVoice {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("devices") => {
            println!("输出设备:");
            for (i, name) in sink::list_output_devices()?.iter().enumerate() {
                println!("  out {i}: {name}");
            }
            println!("输入设备:");
            for (i, name) in sink::list_input_devices()?.iter().enumerate() {
                println!("  in  {i}: {name}");
            }
            Ok(())
        }
        Some("models") => match args.get(1).map(String::as_str) {
            Some("list") | None => {
                models::list();
                Ok(())
            }
            Some("silero") => models::download_silero(),
            Some("sensevoice") => models::download_sensevoice(),
            Some("zipformer") => models::download_zipformer(),
            Some("marian") => models::download_marian(),
            Some("aec") => models::download_aec(),
            Some("piper") => {
                let lang = args.get(2).map(String::as_str).unwrap_or("zh");
                models::download_piper(lang)
            }
            Some("vosk") => {
                let lang = args.get(2).map(String::as_str).unwrap_or("cn");
                models::download_vosk(lang)
            }
            Some(other) => Err(anyhow!("未知模型类型: {other}")),
        },
        Some("vadcheck") => vad_check(&args),
        Some("livetest") => livetest::run(&args),
        Some("multitest") => {
            let cfg = Config::load(flag(&args, "--config").as_deref())?;
            let text = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "今天天气很好".into());
            let mut primary = translate::build(&cfg.translate)?;
            println!("[primary] {}", primary.translate(&text)?);
            for (lang, mut extra) in build_extras(&cfg) {
                println!("[{lang}] {}", extra.translate(&text)?);
            }
            Ok(())
        }
        Some("replay") => {
            let cfg = Config::load(flag(&args, "--config").as_deref())?;
            let _ = recorder::RECORD_DIR.set(resolve_record_dir(&cfg));
            recorder::set_flags(cfg.ui.keep_translation, false);
            recorder::replay_latest()
        }
        Some("ttstest") => {
            let cfg = Config::load(flag(&args, "--config").as_deref())?;
            let _ = recorder::RECORD_DIR.set(resolve_record_dir(&cfg));
            recorder::set_limit(cfg.ui.record_max_mb);
            recorder::set_flags(cfg.ui.keep_translation, false);
            let text = args.get(1).cloned().unwrap_or_else(|| "这是一次较长的语音合成测试，用来验证分句与流式播放。".into());
            let mut edge = None;
            let mut cache = std::collections::HashMap::new();
            let (_stream, handle) = sink::open_output(None)?;
            let sink = rodio::Sink::try_new(&handle)?;
            speak_text(&cfg.tts, None, &mut edge, &mut cache, &sink, None, &mut 0u64, &text)
        }
        Some("palabratest") => palabratest::run(&args),
        Some("volctest") => volctest::run(&args),
        Some("settings") => {
            let path = flag(&args, "--config")
                .unwrap_or_else(|| paths::default_config_path().display().to_string());
            let cfg = Config::load(Some(&path))?;
            ui::run_settings(cfg, path)
        }
        Some("keys") => match args.get(1).map(String::as_str) {
            Some("encrypt") => {
                let p = flag(&args, "--config").unwrap_or_else(|| "keys.toml".into());
                keys::encrypt_file(&p)
            }
            Some("test") => {
                println!("DPAPI roundtrip: {}", if keys::roundtrip_test() { "ok" } else { "fail" });
                Ok(())
            }
            _ => Err(anyhow!("用法: keys encrypt|test [--config keys.toml]")),
        },
        Some("data-dir") => match args.get(1) {
            None => {
                println!("{}", paths::data_dir().display());
                Ok(())
            }
            Some(p) => {
                let old = paths::data_dir();
                paths::set_data_dir(p)?;
                if args.iter().any(|a| a == "--move") {
                    paths::move_data(&old, std::path::Path::new(p))?;
                }
                println!("数据目录: {p}（重启后生效）");
                Ok(())
            }
        },
        Some("preset") => preset(&args),
        Some("doctor") => doctor(&args),
        Some("config-init") => {
            let path = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| paths::default_config_path().display().to_string());
            Config::write_template(&path)?;
            println!("已生成配置: {path}");
            Ok(())
        }
        Some("watch") => watch(&args),
        Some("run") => {
            if !single_instance() {
                eprintln!("simulvoice 已在运行（单实例限制），本次退出。");
                return Ok(());
            }
            let path = flag(&args, "--config");
            let seconds = flag(&args, "--seconds").and_then(|s| s.parse::<u64>().ok());
            run(path.as_deref(), seconds)
        }
        _ => {
            if args.is_empty() {
                let cfg = Config::load(None)?;
                let path = paths::default_config_path().display().to_string();
                return ui::run_settings(cfg, path);
            }
            help();
            Ok(())
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn say(args: &[String]) -> Result<()> {
    let voice = flag(args, "--voice").unwrap_or_else(|| "zh-CN-XiaoxiaoNeural".into());
    let local = args.iter().any(|a| a == "--local");
    let mut text = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--local" => {
                i += 1;
                continue;
            }
            "--voice" | "--out" => {
                i += 2;
                continue;
            }
            _ => {}
        }
        text.push(args[i].clone());
        i += 1;
    }
    let text = text.join(" ");
    if text.trim().is_empty() {
        return Err(anyhow!("say: 需要文本"));
    }
    if local {
        println!("[tts] SAPI (本地)");
        return tts::speak_sapi(&text, Some(&voice));
    }
    if args.iter().any(|a| a == "--piper") {
        let dir = config::default_models_dir().join("tts").display().to_string();
        let engine = tts::SherpaTts::new(&dir)?;
        println!("[tts] Piper (本地)");
        return engine.speak(&text, 1.0, None);
    }
    let tts = EdgeTts { voice, rate: "+0%".into() };
    let mp3 = tts.synth(&text)?;
    println!("[tts] {} KB", mp3.len() / 1024);
    if let Some(path) = flag(args, "--out") {
        std::fs::write(&path, &mp3)?;
        println!("已写入 {path}");
        return Ok(());
    }
    sink::play_mp3(mp3, None)
}

fn transcribe_file(args: &[String]) -> Result<()> {
    use rodio::Source;
    let path = args
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow!("用法: transcribe <音频文件> [--config <文件>]"))?;
    let cfg = Config::load(flag(args, "--config").as_deref())?;
    let _ = ONNX_PROVIDER.set(cfg.dsp.onnx_provider.clone());
    NEURAL_AEC.store(cfg.dsp.neural_aec, Ordering::SeqCst);
    NEURAL_AEC_DELAY.store(cfg.dsp.neural_aec_delay_ms, Ordering::SeqCst);
    let _ = NEURAL_AEC_MODEL.set(if cfg.dsp.neural_aec_model.trim().is_empty() {
        config::default_models_dir()
            .join("aec")
            .join("dec-baseline.onnx")
            .display()
            .to_string()
    } else {
        cfg.dsp.neural_aec_model.clone()
    });
    let mut engine = stt::build(&cfg.stt)?;
    let file = std::fs::File::open(&path)?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels() as usize;
    let mut mono: Vec<i16> = Vec::new();
    let mut acc = 0f32;
    let mut n = 0usize;
    for s in decoder {
        acc += s as f32;
        n += 1;
        if n == channels {
            mono.push((acc / channels as f32) as i16);
            acc = 0.0;
            n = 0;
        }
    }
    let pcm = dsp::resample_all_16k(&mono, rate);
    println!(
        "[stt] {} 采样, {} Hz → 16k, provider={}",
        pcm.len(),
        rate,
        cfg.stt.provider
    );
    engine.begin_utterance();
    for chunk in pcm.chunks(1600) {
        engine.feed(chunk)?;
    }
    if let Some(t) = engine.end_utterance()? {
        println!("{t}");
    }
    if let Some(t) = engine.flush() {
        println!("{t}");
    }
    for t in engine.poll_final() {
        println!("{t}");
    }
    std::thread::sleep(Duration::from_millis(1500));
    for t in engine.poll_final() {
        println!("{t}");
    }
    Ok(())
}

fn vad_check(args: &[String]) -> Result<()> {
    use rodio::Source;
    let path = args
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow!("用法: vadcheck <音频文件> [--config <文件>]"))?;
    let cfg = Config::load(flag(args, "--config").as_deref())?;
    let file = std::fs::File::open(&path)?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels() as usize;
    let mut mono: Vec<i16> = Vec::new();
    let mut acc = 0f32;
    let mut n = 0usize;
    for s in decoder {
        acc += s as f32;
        n += 1;
        if n == channels {
            mono.push((acc / channels as f32) as i16);
            acc = 0.0;
            n = 0;
        }
    }
    let pcm = dsp::resample_all_16k(&mono, rate);
    let mut vad = crate::silero_vad::SileroVad::new(
        &cfg.dsp.vad_model,
        cfg.audio.silence_ms.clamp(100, 5000),
        cfg.audio.max_utterance_ms.clamp(1000, 60_000),
    )?;
    let mut starts = 0;
    let mut ends = 0;
    let mut fed = 0usize;
    let mut peak = 0f32;
    for chunk in pcm.chunks(512) {
        for ev in vad.push(chunk) {
            match ev {
                VadEvent::Start => starts += 1,
                VadEvent::End => ends += 1,
                VadEvent::Feed(b) => fed += b.len(),
            }
        }
        peak = peak.max(vad.last_prob());
    }
    println!(
        "silero: {} 采样, 起点={} 终点={} 送入识别={} 采样, 峰值概率={:.3}",
        pcm.len(),
        starts,
        ends,
        fed,
        peak
    );
    Ok(())
}

fn translate_once(args: &[String]) -> Result<()> {
    let to = flag(args, "--to");
    let mut text = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if matches!(args[i].as_str(), "--to" | "--config" | "--provider") {
            i += 2;
            continue;
        }
        text.push(args[i].clone());
        i += 1;
    }
    let text = text.join(" ");
    if text.trim().is_empty() {
        return Err(anyhow!("translate: 需要文本"));
    }
    let mut cfg = if let Some(path) = flag(args, "--config") {
        Config::load(Some(&path))?.translate
    } else {
        config::TranslateConfig::default()
    };
    if let Some(to) = to {
        cfg.target = to;
    }
    if let Some(p) = flag(args, "--provider") {
        cfg.provider = p;
    }
    let mut tr = translate::build(&cfg)?;
    println!("{}", tr.translate(&text)?);
    Ok(())
}

struct RealtimePipe {
    provider: String,
    api_key: String,
    model: String,
    voice: String,
    instruction: String,
    source_lang: String,
    target_lang: String,
    app_key: String,
    access_key: String,
    resource_id: String,
    voice_mode: String,
    passthrough: bool,
    passthrough_gain: f32,
    aec: bool,
    aec_filter_ms: u32,
    aec_ref: Option<capture::Source>,
    extras: Vec<(String, Box<dyn translate::Translator>)>,
}

struct Pipe {
    label: &'static str,
    source: capture::Source,
    stt: stt::SttEngine,
    translator: Box<dyn translate::Translator>,
    extras: Vec<(String, Box<dyn translate::Translator>)>,
    realtime: Option<RealtimePipe>,
    tts: config::TtsConfig,
    out_device: String,
    outbound: bool,
    vad: (f64, u64, u64),
    vad_provider: String,
    vad_model: String,
    denoise: bool,
    agc: bool,
    aec: bool,
    aec_filter_ms: u32,
    aec_ref: Option<capture::Source>,
    voice_mode: String,
    passthrough: bool,
    passthrough_gain: f32,
    monitor_device: String,
    monitor_translation: bool,
    subs: Option<ui::Shared>,
    seconds: Option<u64>,
}

fn start_ref(
    src: Option<capture::Source>,
    label: &str,
) -> Option<std::sync::mpsc::Receiver<Vec<i16>>> {
    let src = src?;
    match capture::open(src, false, false) {
        Ok(ref_cap) => {
            let sr = ref_cap.sample_rate;
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
            std::thread::spawn(move || {
                let mut rs = Resampler16k::new(sr);
                while !STOP.load(Ordering::SeqCst) {
                    match ref_cap.rx.recv_timeout(Duration::from_millis(300)) {
                        Ok(chunk) => {
                            let pcm = rs.process(&chunk);
                            let _ = tx.try_send(pcm);
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            });
            Some(rx)
        }
        Err(e) => {
            eprintln!("[{label}] AEC 参考采集失败（{e}）");
            None
        }
    }
}

fn start_aec(
    enabled: bool,
    src: Option<capture::Source>,
    filter_ms: u32,
    label: &str,
) -> Option<(
    aec::AecProcessor,
    std::sync::mpsc::Receiver<Vec<i16>>,
)> {
    if !enabled {
        return None;
    }
    let rx = start_ref(src, label)?;
    Some((aec::AecProcessor::new(filter_ms), rx))
}

fn run_pipeline(mut p: Pipe) -> Result<()> {
    let cap = capture::open(p.source, p.denoise, p.agc)?;
    eprintln!("[{}] 采集 {} Hz", p.label, cap.sample_rate);

    if let Some(rt) = p.realtime.take() {
        return run_realtime(cap, rt, p.out_device, p.outbound, p.label, p.subs, p.seconds, p.tts.clone());
    }

    let mut neural = None;
    let mut neural_rx = None;
    if p.outbound && NEURAL_AEC.load(Ordering::SeqCst) {
        if let Some(rx) = start_ref(p.aec_ref.clone(), p.label) {
            let model = NEURAL_AEC_MODEL.get().cloned().unwrap_or_default();
            match crate::neural_aec::NeuralAec::new(
                &model,
                NEURAL_AEC_DELAY.load(Ordering::SeqCst),
            ) {
                Ok(n) => {
                    neural = Some(n);
                    neural_rx = Some(rx);
                }
                Err(e) => eprintln!("[{}] 神经 AEC 初始化失败（{e}），回退 Speex", p.label),
            }
        }
    }
    let mut aec_state = if neural.is_some() {
        None
    } else {
        start_aec(
            p.outbound && p.aec,
            p.aec_ref.clone(),
            p.aec_filter_ms,
            p.label,
        )
    };
    let mut rs = Resampler16k::new(cap.sample_rate);
    let subs_partial = p.subs.clone();
    let mut recorder = recorder::SourceRecorder::new(if p.outbound { "out" } else { "in" });
    let out_dev = p.out_device.clone();
    let dev_opt = if out_dev.trim().is_empty() {
        None
    } else {
        Some(out_dev.as_str())
    };
    let (_tts_stream, tts_handle) = match sink::open_output(dev_opt) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[{}] 输出设备不可用（{e}），改用默认设备", p.label);
            sink::open_output(None)?
        }
    };
    let tts_sink = std::sync::Arc::new(rodio::Sink::try_new(&tts_handle)?);
    let tts_sink_thread = tts_sink.clone();
    let (_monitor_stream, monitor_sink) = if p.outbound && p.monitor_translation {
        let dev = if p.monitor_device.trim().is_empty() {
            p.out_device.clone()
        } else {
            p.monitor_device.clone()
        };
        let opt = if dev.trim().is_empty() { None } else { Some(dev.as_str()) };
        match sink::open_output(opt) {
            Ok((stream, handle)) => (
                Some(stream),
                Some(std::sync::Arc::new(rodio::Sink::try_new(&handle)?)),
            ),
            Err(e) => {
                eprintln!("[{}] 监听设备不可用（{e}）", p.label);
                (None, None)
            }
        }
    } else {
        (None, None)
    };
    let monitor_sink_thread = monitor_sink.clone();
    let is_inbound = !p.outbound;
    let _passthrough = if p.outbound && (p.passthrough || p.voice_mode == "hold_translate") {
        let opt = if p.out_device.trim().is_empty() {
            None
        } else {
            Some(p.out_device.as_str())
        };
        match sink::open_output(opt) {
            Ok((stream, handle)) => Some((
                stream,
                std::sync::Arc::new(rodio::Sink::try_new(&handle)?),
            )),
            Err(e) => {
                eprintln!("[{}] 原声直通设备不可用（{e}）", p.label);
                None
            }
        }
    } else {
        None
    };
    let passthrough_sink = _passthrough.as_ref().map(|(_, sink)| sink.clone());
    let (text_tx, text_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let (speech_tx, speech_rx) = std::sync::mpsc::sync_channel::<String>(
            QUEUE_LIMIT.load(Ordering::SeqCst).max(1),
        );
        let tts_cfg = p.tts.clone();
        let label = p.label;
        let tts_sink = tts_sink_thread.clone();
        let monitor = monitor_sink_thread.clone();
        std::thread::spawn(move || {
            let sherpa = if tts_cfg.provider == "sherpa" {
                let dir = if tts_cfg.model_dir.trim().is_empty() {
                    config::default_models_dir().join("tts").display().to_string()
                } else {
                    tts_cfg.model_dir.clone()
                };
                match tts::SherpaTts::with_options(
                    &dir,
                    &tts_cfg.onnx_provider,
                    tts_cfg.threads,
                ) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("[{}] {e}", label);
                        None
                    }
                }
            } else {
                None
            };
            let mut edge_conn: Option<edge_tts::EdgeTtsConn> = None;
            let mut tts_cache: std::collections::HashMap<String, Vec<u8>> =
                std::collections::HashMap::new();
            let mut stop_gen = STOP_TTS_GEN.load(Ordering::SeqCst);
            while let Ok(text) = speech_rx.recv() {
                if TTS_BACKLOG.load(Ordering::SeqCst) > 0 {
                    TTS_BACKLOG.fetch_sub(1, Ordering::SeqCst);
                }
                if is_inbound {
                    TTS_PLAYING.fetch_add(1, Ordering::SeqCst);
                }
                let result = speak_text(
                    &tts_cfg,
                    sherpa.as_ref(),
                    &mut edge_conn,
                    &mut tts_cache,
                    &tts_sink,
                    monitor.as_deref(),
                    &mut stop_gen,
                    &text,
                );
                if is_inbound {
                    TTS_PLAYING.fetch_sub(1, Ordering::SeqCst);
                }
                if let Err(e) = result {
                    eprintln!("[{}] 朗读失败: {e}", label);
                }
            }
        });
        while let Ok(text) = text_rx.recv() {
            println!("[{}] 原文: {text}", p.label);
            if let Some(s) = &p.subs {
                if let Ok(mut g) = s.lock() {
                    if p.outbound {
                        g.set_out(text.clone(), String::new())
                    } else {
                        g.set_in(text.clone(), String::new())
                    }
                }
            }
            let translated = match p.translator.translate(&text) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[{}] 翻译失败: {e}", p.label);
                    continue;
                }
            };
            println!("[{}] 译文: {translated}", p.label);
            log_transcript(p.label, &text, &translated);
            if let Some(s) = &p.subs {
                if let Ok(mut g) = s.lock() {
                    if p.outbound {
                        g.set_out(String::new(), translated.clone())
                    } else {
                        g.set_in(String::new(), translated.clone())
                    }
                }
            }
            if speech_tx.send(translated.clone()).is_err() {
                break;
            }
            let mut extra_out = Vec::new();
            for (lang, tr) in p.extras.iter_mut() {
                match tr.translate(&text) {
                    Ok(t) => {
                        println!("[{}] 译文({lang}): {t}", p.label);
                        extra_out.push((lang.clone(), t));
                    }
                    Err(e) => eprintln!("[{}] 额外翻译失败({lang}): {e}", p.label),
                }
            }
            if let Some(s) = &p.subs {
                if let Ok(mut g) = s.lock() {
                    g.set_extras(p.outbound, extra_out);
                }
            }
        }
    });
    let mut was_active = false;
    let mut pass_env = 0.0f32;
    let mut vad = if p.vad_provider == "silero" {
        match crate::silero_vad::SileroVad::with_options(
            &p.vad_model,
            p.vad.1,
            p.vad.2,
            ONNX_PROVIDER.get().map(|s| s.as_str()).unwrap_or("cpu"),
            VAD_THREADS.load(Ordering::SeqCst),
        ) {
            Ok(v) => VadEngine::Silero(v),
            Err(e) => {
                eprintln!("[vad] Silero 不可用，回退能量 VAD: {e}");
                VadEngine::Energy(Vad::new(p.vad.0, p.vad.1, p.vad.2))
            }
        }
    } else {
        VadEngine::Energy(Vad::new(p.vad.0, p.vad.1, p.vad.2))
    };
    let started = std::time::Instant::now();
    while !STOP.load(Ordering::SeqCst) {
        if STOP_TTS.swap(false, Ordering::SeqCst) {
            tts_sink.stop();
            tts_sink.play();
        }
        if let Some(secs) = p.seconds {
            if started.elapsed() >= Duration::from_secs(secs) {
                eprintln!("[{}] 达到 {secs} 秒，退出", p.label);
                break;
            }
        }
        let chunk = match cap.rx.recv_timeout(Duration::from_millis(300)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => {
            eprintln!("[{}] 音频线程已退出", p.label);
            break;
        }
        };
        let mut pcm = rs.process(&chunk);
        let mut aec_lost = false;
        if let Some(n) = neural.as_mut() {
            if let Some(rx) = neural_rx.as_ref() {
                loop {
                    match rx.try_recv() {
                        Ok(reference) => n.push_reference(&reference),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            aec_lost = true;
                            break;
                        }
                    }
                }
            }
            if !aec_lost {
                pcm = n.process(&pcm);
            }
        } else if let Some((canceller, rx)) = aec_state.as_mut() {
            loop {
                match rx.try_recv() {
                    Ok(reference) => canceller.push_reference(&reference),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        aec_lost = true;
                        break;
                    }
                }
            }
            if !aec_lost {
                pcm = canceller.process(&pcm);
            }
        }
        if aec_lost {
            eprintln!("[{}] AEC 参考采集已断开，关闭 AEC（可开 echo_guard 兜底）", p.label);
            AEC_RUNNING.store(false, Ordering::SeqCst);
            neural = None;
            neural_rx = None;
            aec_state = None;
        }
        recorder.write(&pcm);
        let ptt = PTT_ACTIVE.load(Ordering::SeqCst);
        let manual = p.outbound && VOICE_MODE.load(Ordering::SeqCst) != 0;
        let echo_block = p.outbound
            && !AEC_RUNNING.load(Ordering::SeqCst)
            && ECHO_GUARD.load(Ordering::SeqCst)
            && TTS_PLAYING.load(Ordering::SeqCst) > 0;
        let passthrough_now = !echo_block
            && (p.passthrough || (p.voice_mode == "hold_translate" && !ptt));
        if passthrough_now {
            if let Some(sink) = passthrough_sink.as_ref() {
                let gain = p.passthrough_gain.clamp(0.1, 4.0);
                let mut f = Vec::with_capacity(pcm.len());
                for &s in &pcm {
                    if pass_env < 1.0 {
                        pass_env = (pass_env + 1.0 / 160.0).min(1.0);
                    }
                    f.push((s as f32 / 32768.0) * gain * pass_env);
                }
                sink.append(rodio::buffer::SamplesBuffer::new(1, 16_000, f));
            }
        } else {
            pass_env = 0.0;
        }
        let recognize = (!manual || ptt) && !echo_block;
        if recognize {
            feed_vad(&pcm, &mut vad, &mut p.stt, &tts_sink, &text_tx, p.label, p.outbound);
        }
        if was_active && !recognize {
            let zeros = vec![0i16; 9_600];
            feed_vad(&zeros, &mut vad, &mut p.stt, &tts_sink, &text_tx, p.label, p.outbound);
        }
        was_active = recognize;
        for text in p.stt.poll_final() {
            enqueue_text(&text_tx, text, p.label);
        }
        if let Some(pt) = p.stt.partial() {
            if let Some(s) = &subs_partial {
                if let Ok(mut g) = s.lock() {
                    if p.outbound {
                        g.out_src = pt;
                    } else {
                        g.in_src = pt;
                    }
                }
            }
        }
    }
    Ok(())
}

fn interpreter_instruction(target: &str, glossary: &str) -> String {
    let terms = if glossary.trim().is_empty() {
        String::new()
    } else {
        format!("\nTerminology (follow strictly):\n{}", glossary.trim())
    };
    format!(
        "You are a simultaneous interpreter. Translate everything you hear into {target}. \
         Speak only the translation, naturally, concisely and fluently. \
         Do not add commentary, never repeat the original language.{terms}"
    )
}

fn build_extras(cfg: &Config) -> Vec<(String, Box<dyn translate::Translator>)> {
    let mut out = Vec::new();
    for target in &cfg.translate.extra_targets {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let mut c = cfg.translate.clone();
        c.target = target.to_string();
        match translate::build(&c) {
            Ok(tr) => out.push((target.to_string(), tr)),
            Err(e) => eprintln!("[extras] {target}: {e}"),
        }
    }
    out
}

fn enqueue_text(tx: &std::sync::mpsc::Sender<String>, text: String, label: &str) {
    if tx.send(text).is_ok() {
        TTS_BACKLOG.fetch_add(1, Ordering::SeqCst);
    }
}

fn enqueue_tts(tx: &std::sync::mpsc::SyncSender<String>, text: String, label: &str) {
    match tx.try_send(text) {
        Ok(()) => {
            TTS_BACKLOG.fetch_add(1, Ordering::SeqCst);
        }
        Err(std::sync::mpsc::TrySendError::Full(_)) => {
            eprintln!("[{label}] 翻译队列已满，丢弃一句");
        }
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
    }
}

fn feed_vad(
    pcm: &[i16],
    vad: &mut VadEngine,
    stt: &mut stt::SttEngine,
    sink: &rodio::Sink,
    tx: &std::sync::mpsc::Sender<String>,
    label: &str,
    outbound: bool,
) {
    for ev in vad.push(pcm) {
        match ev {
            VadEvent::Start => {
                stt.begin_utterance();
                if BARGE_IN.load(Ordering::SeqCst) {
                    sink.stop();
                    sink.play();
                    if CROSS_BARGE.load(Ordering::SeqCst) {
                        let opposite = if outbound { INBOUND_SINK.get() } else { OUTBOUND_SINK.get() };
                        if let Some(s) = opposite {
                            s.stop();
                            s.play();
                        }
                    }
                }
            }
            VadEvent::Feed(buf) => {
                if let Err(e) = stt.feed(&buf) {
                    eprintln!("[{label}] 识别失败: {e}");
                }
            }
            VadEvent::End => match stt.end_utterance() {
                Ok(Some(text)) => enqueue_text(tx, text, label),
                Ok(None) => {}
                Err(e) => eprintln!("[{label}] 识别失败: {e}"),
            },
        }
    }
}

struct DuckGuard {
    active: bool,
}

impl DuckGuard {
    fn new(active: bool) -> Self {
        if active {
            crate::ducking::duck();
        }
        Self { active }
    }
}

impl Drop for DuckGuard {
    fn drop(&mut self) {
        if self.active {
            crate::ducking::restore();
        }
    }
}

fn speak_text(
    cfg: &config::TtsConfig,
    sherpa: Option<&tts::SherpaTts>,
    edge: &mut Option<edge_tts::EdgeTtsConn>,
    cache: &mut std::collections::HashMap<String, Vec<u8>>,
    sink: &rodio::Sink,
    monitor: Option<&rodio::Sink>,
    stop_gen: &mut u64,
    text: &str,
) -> Result<()> {
    if hotkeys::MUTE_TTS.load(Ordering::SeqCst) {
        return Ok(());
    }
    let _duck = DuckGuard::new(cfg.provider != "none" && DUCK.load(Ordering::SeqCst));
    match cfg.provider.as_str() {
        "none" => Ok(()),
        "sapi" => tts::speak_sapi(text, Some(&cfg.voice)),
        "sherpa" => {
            let Some(engine) = sherpa else {
                return Err(anyhow!("Piper 未初始化"));
            };
            for clause in tts::split_clauses(text) {
                let generation = STOP_TTS_GEN.load(Ordering::SeqCst);
                if generation != *stop_gen {
                    *stop_gen = generation;
                    sink.stop();
                    sink.play();
                    return Ok(());
                }
                if hotkeys::MUTE_TTS.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let (samples, rate) = engine.render(&clause, 1.0)?;
                if let Some(m) = monitor {
                    m.append(rodio::buffer::SamplesBuffer::new(1, rate, samples.clone()));
                }
                sink.append(rodio::buffer::SamplesBuffer::new(1, rate, samples));
            }
            sink.sleep_until_end();
            if let Some(m) = monitor {
                m.sleep_until_end();
            }
            Ok(())
        }
        provider => {
            for clause in tts::split_clauses(text) {
                let generation = STOP_TTS_GEN.load(Ordering::SeqCst);
                if generation != *stop_gen {
                    *stop_gen = generation;
                    sink.stop();
                    sink.play();
                    return Ok(());
                }
                if hotkeys::MUTE_TTS.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let key = format!("{}|{}|{}", provider, cfg.voice, clause);
                let mp3 = if let Some(v) = cache.get(&key) {
                    v.clone()
                } else {
                    let bytes = if provider == "openai" {
                        let env_key = std::env::var(&cfg.api_key_env)?;
                        let voice = if cfg.voice.contains("Neural") {
                            "alloy".to_string()
                        } else {
                            cfg.voice.clone()
                        };
                        tts::openai_tts(&clause, &env_key, &cfg.model, &voice)?
                    } else {
                        if edge.is_none() {
                            *edge = Some(edge_tts::EdgeTtsConn::connect(&cfg.voice, &cfg.rate)?);
                        }
                        match edge.as_mut().unwrap().synth(&clause) {
                            Ok(v) => v,
                            Err(_) => {
                                *edge = Some(edge_tts::EdgeTtsConn::connect(
                                    &cfg.voice, &cfg.rate,
                                )?);
                                edge.as_mut().unwrap().synth(&clause)?
                            }
                        }
                    };
                    if cache.len() > 128 {
                        cache.clear();
                    }
                    cache.insert(key, bytes.clone());
                    bytes
                };
                recorder::save_tts(&mp3);
                if let Some(m) = monitor {
                    if let Ok(src) = rodio::Decoder::new(std::io::Cursor::new(mp3.clone())) {
                        m.append(src);
                    }
                }
                let src = rodio::Decoder::new(std::io::Cursor::new(mp3))?;
                sink.append(src);
            }
            sink.sleep_until_end();
            Ok(())
        }
    }
}
fn realtime_extras(
    extras: &mut [(String, Box<dyn translate::Translator>)],
    source: &str,
    subs: &Option<ui::Shared>,
    outbound: bool,
) {
    if extras.is_empty() || source.trim().is_empty() {
        return;
    }
    let mut out = Vec::new();
    for (lang, tr) in extras.iter_mut() {
        match tr.translate(source) {
            Ok(t) => out.push((lang.clone(), t)),
            Err(e) => eprintln!("[extras] {lang}: {e}"),
        }
    }
    if let Some(s) = subs {
        if let Ok(mut g) = s.lock() {
            g.set_extras(outbound, out);
        }
    }
}

fn rt_gate(
    pcm: &[i16],
    outbound: bool,
    mode: &str,
    always_passthrough: bool,
    gain: f32,
    sink: Option<&rodio::Sink>,
    env: &mut f32,
) -> bool {
    let ptt = PTT_ACTIVE.load(Ordering::SeqCst);
    let manual = outbound && mode != "auto";
    let echo_block = outbound
        && ECHO_GUARD.load(Ordering::SeqCst)
        && (TTS_PLAYING.load(Ordering::SeqCst) > 0 || RT_PLAYING.load(Ordering::SeqCst));
    let pass =
        outbound && !echo_block && (always_passthrough || (mode == "hold_translate" && !ptt));
    if pass {
        if let Some(sink) = sink {
            let gain = gain.clamp(0.1, 4.0);
            let mut f = Vec::with_capacity(pcm.len());
            for &s in pcm {
                if *env < 1.0 {
                    *env = (*env + 1.0 / 160.0).min(1.0);
                }
                f.push((s as f32 / 32768.0) * gain * *env);
            }
            sink.append(rodio::buffer::SamplesBuffer::new(1, 16_000, f));
        }
    } else {
        *env = 0.0;
    }
    (!manual || ptt) && !echo_block
}

fn run_realtime(
    cap: capture::Capture,
    mut rt: RealtimePipe,
    out_device: String,
    outbound: bool,
    label: &'static str,
    subs: Option<ui::Shared>,
    seconds: Option<u64>,
    tts_cfg: config::TtsConfig,
) -> Result<()> {
    let mut rs = Resampler16k::new(cap.sample_rate);
    let mut recorder = recorder::SourceRecorder::new(if outbound { "out" } else { "in" });
    let mut aec_state = start_aec(outbound && rt.aec, rt.aec_ref.clone(), rt.aec_filter_ms, label);
    if outbound {
        AEC_RUNNING.store(aec_state.is_some(), Ordering::SeqCst);
    }
    let _pt_stream = if outbound && (rt.passthrough || rt.voice_mode == "hold_translate") {
        let opt = if out_device.trim().is_empty() {
            None
        } else {
            Some(out_device.as_str())
        };
        match sink::open_output(opt) {
            Ok((stream, handle)) => Some((
                stream,
                std::sync::Arc::new(rodio::Sink::try_new(&handle)?),
            )),
            Err(e) => {
                eprintln!("[{label}] 原声直通设备不可用（{e}）");
                None
            }
        }
    } else {
        None
    };
    let pt_sink = _pt_stream.as_ref().map(|(_, s)| s.clone());
    let mut pass_env = 0.0f32;
    let mut was_send = true;

    if rt.provider == "palabra" {
        let pcfg = palabra::PalabraConfig {
            api_key: rt.api_key.clone(),
            source: rt.source_lang.clone(),
            target: rt.target_lang.clone(),
            voice: rt.voice.clone(),
            speech: true,
        };
        let started = std::time::Instant::now();
        let mut src_buf = String::new();
        let mut dst_buf = String::new();

        'reconnect: while !STOP.load(Ordering::SeqCst) {
            if let Some(secs) = seconds {
                if started.elapsed() >= Duration::from_secs(secs) {
                    eprintln!("[{label}] 达到 {secs} 秒，退出");
                    break;
                }
            }
            let mut session = match palabra::PalabraSession::connect(&pcfg, Some(&out_device)) {
                Ok(s) => s,
                Err(e) => {
                    let msg = e.to_string();
                    eprintln!("[{label}] Palabra 连接失败: {msg}");
                    if msg.contains("401") || msg.contains("403") || msg.contains("Unauthorized") {
                        eprintln!("[{label}] Palabra 认证失败，停止重连");
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(800));
                    continue;
                }
            };
            let mut last_send = std::time::Instant::now();

            while !STOP.load(Ordering::SeqCst) {
                if let Some(secs) = seconds {
                    if started.elapsed() >= Duration::from_secs(secs) {
                        eprintln!("[{label}] 达到 {secs} 秒，退出");
                        break 'reconnect;
                    }
                }
                let mut disconnected = false;
                match cap.rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => {
                        let mut pcm = rs.process(&chunk);
                        if let Some((canceller, rx)) = aec_state.as_mut() {
                            while let Ok(reference) = rx.try_recv() {
                                canceller.push_reference(&reference);
                            }
                            pcm = canceller.process(&pcm);
                        }
                        recorder.write(&pcm);
                        let send = rt_gate(
                            &pcm,
                            outbound,
                            &rt.voice_mode,
                            rt.passthrough,
                            rt.passthrough_gain,
                            pt_sink.as_deref(),
                            &mut pass_env,
                        );
                        if send {
                            if let Err(e) = session.send_audio(&pcm) {
                                eprintln!("[{label}] {e}");
                                disconnected = true;
                            }
                        } else if was_send {
                            let _ = session.send_audio(&vec![0i16; 9_600]);
                        }
                        was_send = send;
                        last_send = std::time::Instant::now();
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if last_send.elapsed() >= Duration::from_millis(800) {
                            let _ = session.send_audio(&vec![0i16; 3200]);
                            last_send = std::time::Instant::now();
                        }
                    }
                    Err(_) => {
                        disconnected = true;
                    }
                }
                for ev in session.poll() {
                    match ev {
                        palabra::PalabraEvent::Task(s) => eprintln!("[{label}] task={s}"),
                        palabra::PalabraEvent::SourceFinal(t) => {
                            src_buf = t.clone();
                            println!("[{label}] 原声: {}", t.trim());
                        }
                        palabra::PalabraEvent::TargetFinal(t) => {
                            dst_buf = t.clone();
                            println!("[{label}] 译文: {}", t.trim());
                            log_transcript(label, &src_buf, &t);
                            realtime_extras(&mut rt.extras, &src_buf, &subs, outbound);
                        }
                        palabra::PalabraEvent::Audio => {}
                        palabra::PalabraEvent::Error(e) => {
                            eprintln!("[{label}] palabra: {e}");
                            if e.contains("401") || e.contains("403") || e.contains("UNAUTHORIZED") {
                                eprintln!("[{label}] Palabra 认证失败，停止重连");
                                return Ok(());
                            }
                        }
                        palabra::PalabraEvent::Closed => {
                            disconnected = true;
                        }
                        _ => {}
                    }
                }
                if !outbound {
                    RT_PLAYING.store(session.audio_active(), Ordering::SeqCst);
                }
                if let Some(s) = &subs {
                    if let Ok(mut g) = s.lock() {
                        if outbound {
                            g.set_out(src_buf.clone(), dst_buf.clone())
                        } else {
                            g.set_in(src_buf.clone(), dst_buf.clone())
                        }
                    }
                }
                if disconnected {
                    break;
                }
            }
            if !outbound {
                RT_PLAYING.store(false, Ordering::SeqCst);
            }
            let _ = session.end();
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            if seconds.map(|s| started.elapsed() >= Duration::from_secs(s)).unwrap_or(false) {
                break;
            }
            eprintln!("[{label}] Palabra 断线，重连中...");
            std::thread::sleep(Duration::from_millis(800));
        }
        return Ok(());
    }
    if rt.provider == "volcengine" {
        let source = crate::palabra::lang_code_owned(&rt.source_lang);
        let target = crate::palabra::lang_code_owned(&rt.target_lang);
        if source == "auto" || target == "auto" {
            eprintln!("[{label}] 注意：火山引擎使用语言 auto，若识别异常请在配置里指定明确语言");
        }
        let (say_tx, say_rx) =
            std::sync::mpsc::sync_channel::<String>(QUEUE_LIMIT.load(Ordering::SeqCst));
        let (_volc_stream, volc_handle) = match sink::open_output(Some(&out_device)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[{label}] 输出设备不可用（{e}），改用默认设备");
                sink::open_output(None)?
            }
        };
        let volc_sink = std::sync::Arc::new(rodio::Sink::try_new(&volc_handle)?);
        let sink_w = volc_sink.clone();
        let inbound_tts = !outbound;
        let tts_handle = {
            let cfg_w = tts_cfg.clone();
            std::thread::spawn(move || {
                let sherpa = if cfg_w.provider == "sherpa" {
                    let dir = if cfg_w.model_dir.trim().is_empty() {
                        config::default_models_dir().join("tts").display().to_string()
                    } else {
                        cfg_w.model_dir.clone()
                    };
                    tts::SherpaTts::with_options(&dir, &cfg_w.onnx_provider, cfg_w.threads).ok()
                } else {
                    None
                };
                let mut edge_conn: Option<edge_tts::EdgeTtsConn> = None;
                let mut tts_cache: std::collections::HashMap<String, Vec<u8>> =
                    std::collections::HashMap::new();
                let mut stop_gen = STOP_TTS_GEN.load(Ordering::SeqCst);
                while let Ok(text) = say_rx.recv() {
                    if TTS_BACKLOG.load(Ordering::SeqCst) > 0 {
                        TTS_BACKLOG.fetch_sub(1, Ordering::SeqCst);
                    }
                    if inbound_tts {
                        TTS_PLAYING.fetch_add(1, Ordering::SeqCst);
                    }
                    let result = speak_text(
                        &cfg_w,
                        sherpa.as_ref(),
                        &mut edge_conn,
                        &mut tts_cache,
                        &sink_w,
                        None,
                        &mut stop_gen,
                        &text,
                    );
                    if inbound_tts {
                        TTS_PLAYING.fetch_sub(1, Ordering::SeqCst);
                    }
                    if let Err(e) = result {
                        eprintln!("[tts] {e}");
                    }
                }
            })
        };
        let started = std::time::Instant::now();
        let mut src_text = String::new();
        let mut dst_text = String::new();

        'reconnect: while !STOP.load(Ordering::SeqCst) {
            if let Some(secs) = seconds {
                if started.elapsed() >= Duration::from_secs(secs) {
                    eprintln!("[{label}] 达到 {secs} 秒，退出");
                    break;
                }
            }
            let mut session = match volc::VolcSession::connect(
                &rt.app_key,
                &rt.access_key,
                &rt.resource_id,
                &source,
                &target,
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[{label}] 火山连接失败: {e}，重连中...");
                    std::thread::sleep(Duration::from_millis(800));
                    continue;
                }
            };
            let mut failed = false;

            while !STOP.load(Ordering::SeqCst) {
                if let Some(secs) = seconds {
                    if started.elapsed() >= Duration::from_secs(secs) {
                        eprintln!("[{label}] 达到 {secs} 秒，退出");
                        break 'reconnect;
                    }
                }
                match cap.rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => {
                        let mut pcm = rs.process(&chunk);
                        if let Some((canceller, rx)) = aec_state.as_mut() {
                            while let Ok(reference) = rx.try_recv() {
                                canceller.push_reference(&reference);
                            }
                            pcm = canceller.process(&pcm);
                        }
                        recorder.write(&pcm);
                        let send = rt_gate(
                            &pcm,
                            outbound,
                            &rt.voice_mode,
                            rt.passthrough,
                            rt.passthrough_gain,
                            pt_sink.as_deref(),
                            &mut pass_env,
                        );
                        if send {
                            if let Err(e) = session.send_audio(&pcm) {
                                eprintln!("[{label}] {e}");
                                failed = true;
                                break;
                            }
                        } else if was_send {
                            let _ = session.send_audio(&vec![0i16; 9_600]);
                        }
                        was_send = send;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
                for r in session.poll() {
                    match r.event {
                        volc::EVENT_SESSION_FAILED => {
                            eprintln!("[{label}] volc: {}", r.message);
                            if r.message.contains("401") || r.message.contains("403") {
                                eprintln!("[{label}] 火山认证失败，停止重连");
                                return Ok(());
                            }
                            failed = true;
                        }
                        volc::EVENT_SOURCE_RESPONSE => {
                            if !r.text.trim().is_empty() {
                                src_text = r.text.clone();
                            }
                        }
                        volc::EVENT_SOURCE_END => {
                            if !r.text.trim().is_empty() {
                                src_text = r.text.clone();
                                println!("[{label}] 原声: {}", r.text.trim());
                            }
                        }
                        volc::EVENT_TRANSLATION_RESPONSE => {
                            if !r.text.trim().is_empty() {
                                dst_text.push_str(&r.text);
                            }
                        }
                        volc::EVENT_TRANSLATION_END => {
                            if !r.text.trim().is_empty() {
                                dst_text = r.text.clone();
                                println!("[{label}] 译文: {}", r.text.trim());
                                log_transcript(label, &src_text, &r.text);
                                realtime_extras(&mut rt.extras, &src_text, &subs, outbound);
                                enqueue_tts(&say_tx, r.text.trim().to_string(), label);
                            }
                        }
                        _ => {}
                    }
                    if let Some(s) = &subs {
                        if let Ok(mut g) = s.lock() {
                            if outbound {
                                g.set_out(src_text.clone(), dst_text.clone())
                            } else {
                                g.set_in(src_text.clone(), dst_text.clone())
                            }
                        }
                    }
                }
                if failed {
                    break;
                }
            }
            session.finish();
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(100));
                for r in session.poll() {
                    if r.event == volc::EVENT_TRANSLATION_END && !r.text.trim().is_empty() {
                        println!("[{label}] 译文: {}", r.text.trim());
                        enqueue_tts(&say_tx, r.text.trim().to_string(), label);
                    }
                }
            }
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            if seconds.map(|s| started.elapsed() >= Duration::from_secs(s)).unwrap_or(false) {
                break;
            }
            eprintln!("[{label}] 火山断线，重连中...");
            std::thread::sleep(Duration::from_millis(800));
        }
        drop(say_tx);
        let _ = tts_handle.join();
        return Ok(());
    }
    let mut live = gemini_live::GeminiLive::connect(
        &rt.api_key,
        &rt.model,
        &rt.instruction,
        &rt.voice,
        &out_device,
        !outbound,
    )?;
    let started = std::time::Instant::now();
    let mut src_buf = String::new();
    let mut dst_buf = String::new();
    let mut extra_buf = String::new();
    let mut drain = |live: &mut gemini_live::GeminiLive,
                     src_buf: &mut String,
                     dst_buf: &mut String,
                     subs: &Option<ui::Shared>,
                     extras: &mut Vec<(String, Box<dyn translate::Translator>)>,
                     extra_buf: &mut String| {
        for ev in live.poll() {
            match ev {
                gemini_live::LiveEvent::Ready => eprintln!("[{label}] live ready"),
                gemini_live::LiveEvent::Audio => {}
                gemini_live::LiveEvent::Input(t) => {
                    src_buf.push_str(&t);
                    if src_buf.len() > 400 {
                        if let Some((i, _)) = src_buf.char_indices().rev().nth(200) {
                            *src_buf = src_buf[i..].to_string();
                        }
                    }
                    println!("[{label}] 原声: {}", t.trim());
                }
                gemini_live::LiveEvent::Output(t) => {
                    dst_buf.push_str(&t);
                    if dst_buf.len() > 400 {
                        if let Some((i, _)) = dst_buf.char_indices().rev().nth(200) {
                            *dst_buf = dst_buf[i..].to_string();
                        }
                    }
                    println!("[{label}] 译文: {}", t.trim());
                    extra_buf.push_str(&t);
                    if extra_buf
                        .chars()
                        .any(|c| matches!(c, '。' | '！' | '？' | '.' | '!' | '?' | '\n'))
                    {
                        realtime_extras(extras, extra_buf, subs, outbound);
                        extra_buf.clear();
                    }
                }
                gemini_live::LiveEvent::Error(e) => eprintln!("[{label}] live: {e}"),
            }
        }
        if let Some(s) = subs {
            if let Ok(mut g) = s.lock() {
                if outbound { g.set_out(src_buf.clone(), dst_buf.clone()) } else { g.set_in(src_buf.clone(), dst_buf.clone()) }
            }
        }
    };

    while !STOP.load(Ordering::SeqCst) {
        if let Some(secs) = seconds {
            if started.elapsed() >= Duration::from_secs(secs) {
                eprintln!("[{label}] 达到 {secs} 秒，退出");
                break;
            }
        }
        let chunk = match cap.rx.recv_timeout(Duration::from_millis(200)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                drain(&mut live, &mut src_buf, &mut dst_buf, &subs, &mut rt.extras, &mut extra_buf);
                continue;
            }
            Err(_) => break,
        };
        let mut pcm = rs.process(&chunk);
        if let Some((canceller, rx)) = aec_state.as_mut() {
            while let Ok(reference) = rx.try_recv() {
                canceller.push_reference(&reference);
            }
            pcm = canceller.process(&pcm);
        }
        recorder.write(&pcm);
        let send = rt_gate(
            &pcm,
            outbound,
            &rt.voice_mode,
            rt.passthrough,
            rt.passthrough_gain,
            pt_sink.as_deref(),
            &mut pass_env,
        );
        if send {
            if let Err(e) = live.feed(&pcm) {
                eprintln!("[{label}] live feed: {e}");
            }
        } else if was_send {
            let _ = live.feed(&vec![0i16; 9_600]);
        }
        was_send = send;
        drain(&mut live, &mut src_buf, &mut dst_buf, &subs, &mut rt.extras, &mut extra_buf);
    }
    Ok(())
}

fn resolve_record_dir(cfg: &config::Config) -> String {
    if !cfg.ui.record_dir.trim().is_empty() {
        return cfg.ui.record_dir.clone();
    }
    if cfg.ui.keep_translation || cfg.ui.record_source {
        let d = crate::paths::data_dir().join("recordings");
        let _ = std::fs::create_dir_all(&d);
        d.display().to_string()
    } else {
        String::new()
    }
}

fn run(config_path: Option<&str>, seconds: Option<u64>) -> Result<()> {
    let cfg = Config::load(config_path)?;
    println!(
        "[配置] stt={} translate={} tts={} realtime={} duplex={}",
        cfg.stt.provider,
        cfg.translate.provider,
        cfg.tts.provider,
        cfg.realtime.provider,
        cfg.audio.duplex
    );
    let live_palabra = cfg.realtime.provider == "palabra";
    let live_volc = cfg.realtime.provider == "volcengine";
    let live_mode = cfg.realtime.provider == "gemini-live" || live_palabra || live_volc;
    let speak_only = cfg.audio.mode == "speak";
    let listen_only = cfg.audio.mode == "listen";
    let duplex = cfg.audio.duplex && !speak_only && !listen_only;
    let _ = TRANSCRIPT.set(cfg.ui.transcript_path.clone());
    let record_dir = resolve_record_dir(&cfg);
    let _ = recorder::RECORD_DIR.set(record_dir);
    recorder::set_limit(cfg.ui.record_max_mb);
    recorder::set_flags(cfg.ui.keep_translation, cfg.ui.record_source);
    VOICE_MODE.store(
        match cfg.audio.voice_mode.as_str() {
            "toggle" => 1,
            "ptt" | "hold" => 2,
            "hold_translate" => 3,
            _ => 0,
        },
        Ordering::SeqCst,
    );
    match hotkeys::vk_code(&cfg.audio.ptt_key) {
        Some(vk) => {
            PTT_VK.store(vk, Ordering::SeqCst);
            PTT_REQ_CTRL.store(cfg.audio.ptt_ctrl, Ordering::SeqCst);
            PTT_REQ_ALT.store(cfg.audio.ptt_alt, Ordering::SeqCst);
            if cfg.audio.ptt_ctrl
                && cfg.audio.ptt_alt
                && matches!(vk, 0x75 | 0x77 | 0x78 | 0x79)
            {
                eprintln!("[热键] ptt_key 与内置 Ctrl+Alt+F6/F8/F9/F10 冲突，请换一个键");
            }
        }
        None => eprintln!(
            "[热键] 无法识别的 ptt_key={}，继续使用 F7",
            cfg.audio.ptt_key
        ),
    }
    DUCK.store(cfg.dsp.duck, Ordering::SeqCst);
    ECHO_GUARD.store(cfg.audio.echo_guard, Ordering::SeqCst);
    AEC_ON.store(cfg.dsp.aec, Ordering::SeqCst);
    let _ = ONNX_PROVIDER.set(cfg.dsp.onnx_provider.clone());
    NEURAL_AEC.store(cfg.dsp.neural_aec, Ordering::SeqCst);
    NEURAL_AEC_DELAY.store(cfg.dsp.neural_aec_delay_ms, Ordering::SeqCst);
    let _ = NEURAL_AEC_MODEL.set(if cfg.dsp.neural_aec_model.trim().is_empty() {
        config::default_models_dir()
            .join("aec")
            .join("dec-baseline.onnx")
            .display()
            .to_string()
    } else {
        cfg.dsp.neural_aec_model.clone()
    });
    VAD_THREADS.store(cfg.dsp.vad_threads.max(1), Ordering::SeqCst);
    BARGE_IN.store(cfg.audio.barge_in, Ordering::SeqCst);
    CROSS_BARGE.store(cfg.audio.cross_barge_in, Ordering::SeqCst);
    MUTE_IN.store(cfg.audio.mute_inbound, Ordering::SeqCst);
    MUTE_OUT.store(cfg.audio.mute_outbound, Ordering::SeqCst);
    JUMP_SECS.store(cfg.audio.backlog_jump_secs, Ordering::SeqCst);
    QUEUE_LIMIT.store((cfg.audio.queue_limit.max(1)) as usize, Ordering::SeqCst);
    let live_app_key = if live_volc {
        Some(std::env::var("VOLC_APP_ID").map_err(|_| anyhow!("VOLC_APP_ID 未设置"))?)
    } else {
        None
    };
    let live_access_key = if live_volc {
        Some(std::env::var("VOLC_ACCESS_TOKEN").map_err(|_| anyhow!("VOLC_ACCESS_TOKEN 未设置"))?)
    } else {
        None
    };
    let live_resource = if live_volc {
        Some(
            std::env::var("VOLC_RESOURCE_ID")
                .unwrap_or_else(|_| "volc.service_type.10053".into()),
        )
    } else {
        None
    };
    let live_key = if cfg.realtime.provider == "gemini-live" {
        Some(std::env::var("GEMINI_API_KEY").map_err(|_| {
            anyhow!("GEMINI_API_KEY 未设置（keys.toml 或环境变量）")
        })?)
    } else if live_palabra {
        Some(std::env::var("PALABRA_API_KEY").map_err(|_| {
            anyhow!("PALABRA_API_KEY 未设置（keys.toml 或环境变量）")
        })?)
    } else {
        None
    };

    if cfg.audio.input == "loopback" && !cfg.audio.output_device.trim().is_empty() {
        if let Some(def) = sink::default_output_name() {
            let a = cfg.audio.output_device.to_lowercase();
            let b = def.to_lowercase();
            if a.contains(&b) || b.contains(&a) {
                return Err(anyhow!(
                    "输出设备与采集设备相同（{def}），会把自己的译文再次识别，形成自激循环。\
                     请把 output_device 设为耳机，或改用 input=\"pid\" 采集会议进程。"
                ));
            }
        }
    }

    let pid = if cfg.audio.input == "pid" && cfg.audio.pid > 0 {
        Some(cfg.audio.pid)
    } else {
        None
    };
    let subs = if cfg.ui.subtitles { Some(ui::new_state()) } else { None };

    // 入向：对方 → 你
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            let last = capture::last_tick();
            if last > 0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if now.saturating_sub(last) > 10 {
                    eprintln!("[watchdog] 采集线程 10 秒无响应，停止");
                    STOP.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
    });

    let inbound = Pipe {
        label: "对方",
        source: if cfg.audio.input == "mic" {
            capture::Source::Mic(Some(cfg.audio.mic_device.clone()))
        } else if pid.is_some() {
            capture::Source::Loopback(pid)
        } else if !cfg.audio.loopback_device.trim().is_empty() {
            capture::Source::LoopbackDevice(cfg.audio.loopback_device.clone())
        } else {
            capture::Source::Loopback(None)
        },
        stt: if live_mode {
            stt::SttEngine::None
        } else {
            stt::build(&cfg.stt)?
        },
        translator: if live_mode {
            Box::new(translate::PassThroughTranslator)
        } else {
            translate::build(&cfg.translate)?
        },
        extras: if live_mode { Vec::new() } else { build_extras(&cfg) },
        realtime: if live_mode {
            Some(RealtimePipe {
                provider: cfg.realtime.provider.clone(),
                api_key: live_key.clone().unwrap_or_default(),
                model: cfg.realtime.model.clone(),
                voice: if live_palabra {
                    "default_low".into()
                } else {
                    cfg.realtime.voice.clone()
                },
                instruction: interpreter_instruction(&cfg.translate.target, &cfg.translate.glossary),
                source_lang: cfg.stt.language.clone(),
                target_lang: cfg.translate.target.clone(),
                app_key: live_app_key.clone().unwrap_or_default(),
                access_key: live_access_key.clone().unwrap_or_default(),
                resource_id: live_resource.clone().unwrap_or_default(),
                voice_mode: cfg.audio.voice_mode.clone(),
                passthrough: cfg.audio.passthrough,
                passthrough_gain: cfg.audio.passthrough_gain,
                aec: cfg.dsp.aec || cfg.dsp.neural_aec,
                aec_filter_ms: cfg.dsp.aec_filter_ms,
                aec_ref: if cfg.dsp.aec || cfg.dsp.neural_aec {
                    Some(if cfg.audio.output_device.trim().is_empty() {
                        capture::Source::Loopback(None)
                    } else {
                        capture::Source::LoopbackDevice(cfg.audio.output_device.clone())
                    })
                } else {
                    None
                },
                extras: build_extras(&cfg),
            })
        } else {
            None
        },
        tts: cfg.tts.clone(),
        out_device: cfg.audio.output_device.clone(),
        outbound: false,
        vad: (
            cfg.audio.vad_rms,
            cfg.audio.silence_ms.clamp(100, 5000),
            cfg.audio.max_utterance_ms.clamp(1000, 60_000),
        ),
        vad_provider: cfg.dsp.vad.clone(),
        vad_model: cfg.dsp.vad_model.clone(),
        denoise: cfg.dsp.denoise,
        agc: cfg.dsp.agc,
        aec: cfg.dsp.aec || cfg.dsp.neural_aec,
        aec_filter_ms: cfg.dsp.aec_filter_ms,
        aec_ref: if cfg.dsp.aec || cfg.dsp.neural_aec {
            Some(if cfg.audio.output_device.trim().is_empty() {
                capture::Source::Loopback(None)
            } else {
                capture::Source::LoopbackDevice(cfg.audio.output_device.clone())
            })
        } else {
            None
        },
        voice_mode: cfg.audio.voice_mode.clone(),
        passthrough: cfg.audio.passthrough,
        passthrough_gain: cfg.audio.passthrough_gain,
        monitor_device: if cfg.audio.monitor_device.trim().is_empty() {
            cfg.audio.output_device.clone()
        } else {
            cfg.audio.monitor_device.clone()
        },
        monitor_translation: cfg.audio.monitor_translation,
        subs: subs.clone(),
        seconds,
    };

    if !duplex && !speak_only {
        if let Some(state) = subs {
            let h = std::thread::spawn(move || run_pipeline(inbound));
            ui::run(state, seconds.map(Duration::from_secs))?;
            let _ = h.join();
            return Ok(());
        }
        return run_pipeline(inbound);
    }

    // 出向：你 → 对方（麦克风 → 翻译 → 虚拟麦克风）
    let mut out_stt_cfg = cfg.stt.clone();
    out_stt_cfg.language = cfg.stt.language_reverse.clone();
    let mut out_tr_cfg = cfg.translate.clone();
    out_tr_cfg.target = cfg.translate.target_reverse.clone();
    let mut out_tts_cfg = cfg.tts.clone();
    out_tts_cfg.voice = cfg.tts.voice_reverse.clone();

    if cfg.audio.remote_output_device.trim().is_empty() {
        eprintln!("[警告] duplex=true 但 remote_output_device 为空，译文会走默认设备，可能被再次采集。建议填 \"CABLE\"。");
    }
    if cfg.audio.output_device.trim().is_empty() {
        eprintln!("[警告] 入向输出是默认设备；若与采集设备相同会自激，建议指定耳机设备名");
    }
    if cfg.audio.input == "mic" {
        eprintln!("[警告] duplex=true 但 input=mic：入向也会采集麦克风，双向会互相串音；建议 input=loopback");
    }
    if !cfg.audio.loopback_device.trim().is_empty()
        && cfg.audio.loopback_device.eq_ignore_ascii_case(cfg.audio.output_device.trim())
    {
        eprintln!("[警告] 回环采集设备与入向输出设备相同，会把自己的译文再次采集");
    }
    let monitor = if cfg.audio.monitor_device.trim().is_empty() {
        cfg.audio.output_device.trim()
    } else {
        cfg.audio.monitor_device.trim()
    };
    if !monitor.is_empty() && monitor.eq_ignore_ascii_case(cfg.audio.remote_output_device.trim()) {
        eprintln!("[警告] 监听设备与出向虚拟麦相同，会把译文回灌给对方");
    }
    if cfg.audio.passthrough && cfg.audio.duplex {
        eprintln!("[警告] 原声直通 + 双向同传：对方会同时听到原声和译文");
    }
    if cfg.dsp.neural_aec && live_mode {
        eprintln!("[警告] 实时链路暂不支持神经 AEC，将使用 Speex（aec=true 时）");
    }


    let outbound = Pipe {
        label: "你",
        source: capture::Source::Mic(Some(cfg.audio.mic_device.clone())),
        stt: if live_mode {
            stt::SttEngine::None
        } else {
            stt::build(&out_stt_cfg)?
        },
        translator: if live_mode {
            Box::new(translate::PassThroughTranslator)
        } else {
            translate::build(&out_tr_cfg)?
        },
        extras: if live_mode { Vec::new() } else { build_extras(&cfg) },
        realtime: if live_mode {
            Some(RealtimePipe {
                provider: cfg.realtime.provider.clone(),
                api_key: live_key.clone().unwrap_or_default(),
                model: cfg.realtime.model.clone(),
                voice: if live_palabra {
                    "default_high".into()
                } else {
                    cfg.realtime.voice_reverse.clone()
                },
                instruction: interpreter_instruction(&cfg.translate.target_reverse, &cfg.translate.glossary),
                source_lang: cfg.stt.language_reverse.clone(),
                target_lang: cfg.translate.target_reverse.clone(),
                app_key: live_app_key.clone().unwrap_or_default(),
                access_key: live_access_key.clone().unwrap_or_default(),
                resource_id: live_resource.clone().unwrap_or_default(),
                voice_mode: cfg.audio.voice_mode.clone(),
                passthrough: cfg.audio.passthrough,
                passthrough_gain: cfg.audio.passthrough_gain,
                aec: cfg.dsp.aec || cfg.dsp.neural_aec,
                aec_filter_ms: cfg.dsp.aec_filter_ms,
                aec_ref: if cfg.dsp.aec || cfg.dsp.neural_aec {
                    Some(if cfg.audio.output_device.trim().is_empty() {
                        capture::Source::Loopback(None)
                    } else {
                        capture::Source::LoopbackDevice(cfg.audio.output_device.clone())
                    })
                } else {
                    None
                },
                extras: build_extras(&cfg),
            })
        } else {
            None
        },
        tts: out_tts_cfg,
        out_device: cfg.audio.remote_output_device.clone(),
        outbound: true,
        vad: (
            cfg.audio.vad_rms,
            cfg.audio.silence_ms.clamp(100, 5000),
            cfg.audio.max_utterance_ms.clamp(1000, 60_000),
        ),
        vad_provider: cfg.dsp.vad.clone(),
        vad_model: cfg.dsp.vad_model.clone(),
        denoise: cfg.dsp.denoise,
        agc: cfg.dsp.agc,
        aec: cfg.dsp.aec || cfg.dsp.neural_aec,
        aec_filter_ms: cfg.dsp.aec_filter_ms,
        aec_ref: if cfg.dsp.aec || cfg.dsp.neural_aec {
            Some(if cfg.audio.output_device.trim().is_empty() {
                capture::Source::Loopback(None)
            } else {
                capture::Source::LoopbackDevice(cfg.audio.output_device.clone())
            })
        } else {
            None
        },
        voice_mode: cfg.audio.voice_mode.clone(),
        passthrough: cfg.audio.passthrough,
        passthrough_gain: cfg.audio.passthrough_gain,
        monitor_device: if cfg.audio.monitor_device.trim().is_empty() {
            cfg.audio.output_device.clone()
        } else {
            cfg.audio.monitor_device.clone()
        },
        monitor_translation: cfg.audio.monitor_translation,
        subs: subs.clone(),
        seconds,
    };

    if speak_only {
        if let Some(state) = subs {
            let h = std::thread::spawn(move || run_pipeline(outbound));
            ui::run(state, seconds.map(Duration::from_secs))?;
            let _ = h.join();
            return Ok(());
        }
        return run_pipeline(outbound);
    }

    if let Some(state) = subs {
        let _a = std::thread::spawn(move || run_pipeline(inbound));
        let _b = std::thread::spawn(move || run_pipeline(outbound));
        ui::run(state, seconds.map(Duration::from_secs))?;
        return Ok(());
    }

    let a = std::thread::spawn(move || run_pipeline(inbound));
    let b = std::thread::spawn(move || run_pipeline(outbound));
    let ra = a.join().map_err(|_| anyhow!("入向线程崩溃"))?;
    let rb = b.join().map_err(|_| anyhow!("出向线程崩溃"))?;
    ra?;
    rb
}












































































































































































































