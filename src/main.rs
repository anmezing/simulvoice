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
mod recorder;
mod palabra;
mod paths;
mod palabratest;
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
static TRANSCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn log_transcript(side: &str, src: &str, dst: &str) {
    use std::io::Write;
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
    cfg.save(&path)?;
    println!("已写入 {path}");
    match name {
        "teams" => println!("Teams 双向：麦克风选 CABLE Output，output_device 改成耳机名"),
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
    let models = config::default_models_dir();
    for (label, p) in [
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
    Ok(())
}

static INSTANCE_LISTENER: std::sync::OnceLock<std::net::TcpListener> = std::sync::OnceLock::new();

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
        r#"SimulVoice 0.14.1 — 实时同传（Rust 原生）

用法:
  simulvoice run [--config <文件>]            同传：采集 → 识别 → 翻译 → 朗读
                                             （配置 audio.duplex=true 后双向）
  simulvoice devices                          列出输出/输入设备
  simulvoice models list                      列出可下载的本地模型
  simulvoice models vosk <cn|en|ru|ja|ko|de|fr|es|cn-big|en-big>
  simulvoice models silero                    Silero VAD 模型（本地识别用）
  simulvoice models piper <zh|en>              Piper 本地 TTS 模型（离线）
  simulvoice vadcheck <音频>                  检查 Silero VAD 切分
  simulvoice livetest <音频>                  Gemini Live 端到端翻译测试
  simulvoice palabratest <音频>               Palabra.ai 端到端翻译测试
  simulvoice volctest <音频>                  火山引擎 Doubao 翻译测试
  simulvoice config-init [文件]               生成配置模板
  simulvoice settings [--config <文件>]       图形化设置面板
  simulvoice preset <teams|game|record>       生成场景预设配置
  simulvoice doctor [--config <文件>]         自检设备/密钥/模型/路由
  simulvoice keys encrypt|test [--config]     DPAPI 加密 keys.toml / 自测
  simulvoice say <文本> [--local] [--voice] [--out a.mp3]
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
        Some("ttstest") => {
            let cfg = Config::load(flag(&args, "--config").as_deref())?;
            let _ = recorder::RECORD_DIR.set(cfg.ui.record_dir.clone());
            let text = args.get(1).cloned().unwrap_or_else(|| "这是一次较长的语音合成测试，用来验证分句与流式播放。".into());
            let mut edge = None;
            let mut cache = std::collections::HashMap::new();
            let (_stream, handle) = sink::open_output(None)?;
            let sink = rodio::Sink::try_new(&handle)?;
            speak_text(&cfg.tts, None, &mut edge, &mut cache, &sink, &text)
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
        cfg.audio.silence_ms,
        cfg.audio.max_utterance_ms,
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
    let to = flag(args, "--to").unwrap_or_else(|| "Chinese".into());
    let mut text = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--to" {
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
    let mut cfg = config::TranslateConfig {
        target: to,
        ..Default::default()
    };
    if let Some(p) = flag(args, "--provider") {
        cfg.provider = p;
    }
    let mut tr = translate::build(&cfg)?;
    println!("{}", tr.translate(&text)?);
    Ok(())
}

#[derive(Clone)]
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
    subs: Option<ui::Shared>,
    seconds: Option<u64>,
}

fn run_pipeline(mut p: Pipe) -> Result<()> {
    let cap = capture::open(p.source, p.denoise, p.agc)?;
    eprintln!("[{}] 采集 {} Hz", p.label, cap.sample_rate);

    if let Some(rt) = p.realtime.take() {
        return run_realtime(cap, rt, p.out_device, p.outbound, p.label, p.subs, p.seconds, p.tts.clone());
    }

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
    let (text_tx, text_rx) = channel::<String>();
    std::thread::spawn(move || {
        let sherpa = if p.tts.provider == "sherpa" {
            let dir = if p.tts.model_dir.trim().is_empty() {
                config::default_models_dir().join("tts").display().to_string()
            } else {
                p.tts.model_dir.clone()
            };
            match tts::SherpaTts::new(&dir) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("[{}] {e}", p.label);
                    None
                }
            }
        } else {
            None
        };
        let mut edge_conn: Option<edge_tts::EdgeTtsConn> = None;
        let mut tts_cache: std::collections::HashMap<String, Vec<u8>> =
            std::collections::HashMap::new();
        while let Ok(text) = text_rx.recv() {
            println!("[{}] 原声: {text}", p.label);
            if let Some(s) = &p.subs {
                if let Ok(mut g) = s.lock() {
                    if p.outbound { g.set_out(text.clone(), String::new()) } else { g.set_in(text.clone(), String::new()) }
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
            let mut extra_out = Vec::new();
            for (lang, tr) in p.extras.iter_mut() {
                match tr.translate(&text) {
                    Ok(t) => {
                        println!("[{}] 额外({lang}): {t}", p.label);
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
            if let Some(s) = &p.subs {
                if let Ok(mut g) = s.lock() {
                    if p.outbound { g.set_out(String::new(), translated.clone()) } else { g.set_in(String::new(), translated.clone()) }
                }
            }
            let result = speak_text(
                &p.tts,
                sherpa.as_ref(),
                &mut edge_conn,
                &mut tts_cache,
                &tts_sink_thread,
                &translated,
            );
            if let Err(e) = result {
                eprintln!("[{}] 播放失败: {e}", p.label);
            }
        }
    });

    let mut vad = if p.vad_provider == "silero" {
        match crate::silero_vad::SileroVad::new(&p.vad_model, p.vad.1, p.vad.2) {
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
        let pcm = rs.process(&chunk);
        recorder.write(&pcm);
        for ev in vad.push(&pcm) {
            match ev {
                VadEvent::Start => {
                    p.stt.begin_utterance();
                    if BARGE_IN.load(Ordering::SeqCst) {
                        tts_sink.stop();
                        tts_sink.play();
                    }
                }
                VadEvent::Feed(buf) => {
                    if let Err(e) = p.stt.feed(&buf) {
                        eprintln!("[{}] 识别失败: {e}", p.label);
                    }
                }
                VadEvent::End => match p.stt.end_utterance() {
                    Ok(Some(text)) => {
                        let _ = text_tx.send(text);
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("[{}] 识别失败: {e}", p.label),
                },
            }
        }
        for text in p.stt.poll_final() {
            let _ = text_tx.send(text);
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
                let (samples, rate) = engine.render(&clause, 1.0)?;
                sink.append(rodio::buffer::SamplesBuffer::new(1, rate, samples));
            }
            sink.sleep_until_end();
            Ok(())
        }
        provider => {
            for clause in tts::split_clauses(text) {
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
                let src = rodio::Decoder::new(std::io::Cursor::new(mp3))?;
                sink.append(src);
            }
            sink.sleep_until_end();
            Ok(())
        }
    }
}
fn run_realtime(
    cap: capture::Capture,
    rt: RealtimePipe,
    out_device: String,
    outbound: bool,
    label: &'static str,
    subs: Option<ui::Shared>,
    seconds: Option<u64>,
    tts_cfg: config::TtsConfig,
) -> Result<()> {
    let mut rs = Resampler16k::new(cap.sample_rate);
    let mut recorder = recorder::SourceRecorder::new(if outbound { "out" } else { "in" });

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
                    eprintln!("[{label}] Palabra 连接失败: {e}，重连中...");
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
                        let pcm = rs.process(&chunk);
                        recorder.write(&pcm);
                        if let Err(e) = session.send_audio(&pcm) {
                            eprintln!("[{label}] {e}");
                            disconnected = true;
                        }
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
                        }
                        palabra::PalabraEvent::Audio => {}
                        palabra::PalabraEvent::Error(e) => eprintln!("[{label}] palabra: {e}"),
                        palabra::PalabraEvent::Closed => {
                            disconnected = true;
                        }
                        _ => {}
                    }
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
        let source = crate::palabra::lang_code(&rt.source_lang).to_string();
        let target = crate::palabra::lang_code(&rt.target_lang).to_string();
        let (say_tx, say_rx) = std::sync::mpsc::channel::<String>();
        let (_volc_stream, volc_handle) = match sink::open_output(Some(&out_device)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[{label}] 输出设备不可用（{e}），改用默认设备");
                sink::open_output(None)?
            }
        };
        let volc_sink = std::sync::Arc::new(rodio::Sink::try_new(&volc_handle)?);
        let sink_w = volc_sink.clone();
        let tts_handle = {
            let cfg_w = tts_cfg.clone();
            std::thread::spawn(move || {
                let sherpa = if cfg_w.provider == "sherpa" {
                    let dir = if cfg_w.model_dir.trim().is_empty() {
                        config::default_models_dir().join("tts").display().to_string()
                    } else {
                        cfg_w.model_dir.clone()
                    };
                    tts::SherpaTts::new(&dir).ok()
                } else {
                    None
                };
                let mut edge_conn: Option<edge_tts::EdgeTtsConn> = None;
                let mut tts_cache: std::collections::HashMap<String, Vec<u8>> =
                    std::collections::HashMap::new();
                while let Ok(text) = say_rx.recv() {
                    if let Err(e) = speak_text(
                        &cfg_w,
                        sherpa.as_ref(),
                        &mut edge_conn,
                        &mut tts_cache,
                        &sink_w,
                        &text,
                    ) {
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
                        let pcm = rs.process(&chunk);
                        recorder.write(&pcm);
                        if let Err(e) = session.send_audio(&pcm) {
                            eprintln!("[{label}] {e}");
                            failed = true;
                            break;
                        }
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
                                let _ = say_tx.send(r.text.trim().to_string());
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
                        let _ = say_tx.send(r.text.trim().to_string());
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
    )?;
    let started = std::time::Instant::now();
    let mut src_buf = String::new();
    let mut dst_buf = String::new();
    let mut drain = |live: &mut gemini_live::GeminiLive,
                     src_buf: &mut String,
                     dst_buf: &mut String,
                     subs: &Option<ui::Shared>| {
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
                drain(&mut live, &mut src_buf, &mut dst_buf, &subs);
                continue;
            }
            Err(_) => break,
        };
        let pcm = rs.process(&chunk);
        if let Err(e) = live.feed(&pcm) {
            eprintln!("[{label}] live feed: {e}");
        }
        drain(&mut live, &mut src_buf, &mut dst_buf, &subs);
    }
    Ok(())
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
    let _ = recorder::RECORD_DIR.set(cfg.ui.record_dir.clone());
    DUCK.store(cfg.dsp.duck, Ordering::SeqCst);
    BARGE_IN.store(cfg.audio.barge_in, Ordering::SeqCst);
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
    let inbound = Pipe {
        label: "对方",
        source: if cfg.audio.input == "mic" {
            capture::Source::Mic(Some(cfg.audio.mic_device.clone()))
        } else {
            capture::Source::Loopback(pid)
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
            })
        } else {
            None
        },
        tts: cfg.tts.clone(),
        out_device: cfg.audio.output_device.clone(),
        outbound: false,
        vad: (
            cfg.audio.vad_rms,
            cfg.audio.silence_ms,
            cfg.audio.max_utterance_ms,
        ),
        vad_provider: cfg.dsp.vad.clone(),
        vad_model: cfg.dsp.vad_model.clone(),
        denoise: cfg.dsp.denoise,
        agc: cfg.dsp.agc,
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
        eprintln!("[警告] 入向译文走默认设备；若与采集设备相同会自激循环，建议填耳机设备名。");
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
            })
        } else {
            None
        },
        tts: out_tts_cfg,
        out_device: cfg.audio.remote_output_device.clone(),
        outbound: true,
        vad: (
            cfg.audio.vad_rms,
            cfg.audio.silence_ms,
            cfg.audio.max_utterance_ms,
        ),
        vad_provider: cfg.dsp.vad.clone(),
        vad_model: cfg.dsp.vad_model.clone(),
        denoise: cfg.dsp.denoise,
        agc: cfg.dsp.agc,
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



































































































