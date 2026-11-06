use anyhow::{anyhow, Result};
use std::fs::File;
use std::path::PathBuf;

pub static DOWNLOAD_PCT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

const DLL_URL: &str =
    "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip";

const VOSK_MODELS: &[(&str, &str)] = &[
    ("cn", "https://alphacephei.com/vosk/models/vosk-model-small-cn-0.22.zip"),
    ("en", "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip"),
    ("ru", "https://alphacephei.com/vosk/models/vosk-model-small-ru-0.22.zip"),
    ("ja", "https://alphacephei.com/vosk/models/vosk-model-small-ja-0.22.zip"),
    ("ko", "https://alphacephei.com/vosk/models/vosk-model-small-ko-0.22.zip"),
    ("de", "https://alphacephei.com/vosk/models/vosk-model-small-de-0.15.zip"),
    ("fr", "https://alphacephei.com/vosk/models/vosk-model-small-fr-0.22.zip"),
    ("es", "https://alphacephei.com/vosk/models/vosk-model-small-es-0.42.zip"),
    ("cn-big", "https://alphacephei.com/vosk/models/vosk-model-cn-0.22.zip"),
    ("en-big", "https://alphacephei.com/vosk/models/vosk-model-en-us-0.22.zip"),
];

pub fn list() {
    println!("推荐（全本地，任意语言对）：");
    println!("  sensevoice   语音识别 中/英/日/韩/粤（230 MB）");
    println!("  whisper      语音识别 其他语言（380 MB）");
    println!("  nllb         翻译 200 种语言任意互译（910 MB）");
    println!("  marian [a-b] 翻译 单一语言对，快且准（每对约 250 MB，默认中英两对）");
    println!("  piper <语言> 本地朗读，可选：{}",
        crate::lang::LANGS.iter().filter(|l| !l.piper.is_empty()).map(|l| l.code).collect::<Vec<_>>().join(" "));
    println!("  silero       语音检测（2 MB）");
    println!("\n旧引擎 (Vosk, 离线 STT):");
    for (lang, url) in VOSK_MODELS {
        println!("  {lang:>3}  {}", url.rsplit('/').next().unwrap_or(url));
    }
}

/// 下载到 `dest.part`，完整收到后才改名为 `dest`。
/// 中途断网/关程序只会留下 .part，下次重下，不会把半个模型当成已就绪。
fn download(url: &str, dest: &PathBuf) -> Result<()> {
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let r = download_raw(url, &part);
    match r {
        Ok(()) => {
            std::fs::rename(&part, dest)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn download_raw(url: &str, dest: &PathBuf) -> Result<()> {
    use std::io::{Read, Write};
    println!("下载 {url}");
    DOWNLOAD_PCT.store(0, std::sync::atomic::Ordering::Relaxed);
    let resp = crate::http::agent().get(url).call()?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut file = File::create(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if crate::STOP.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(anyhow!("下载已取消"));
        }
        let pct = if total > 0 { (done * 100 / total).min(100) as u8 } else { 0 };
        DOWNLOAD_PCT.store(pct, std::sync::atomic::Ordering::Relaxed);
    }
    if total > 0 && done < total {
        return Err(anyhow!("下载不完整（{done}/{total} 字节），请重试"));
    }
    file.flush()?;
    DOWNLOAD_PCT.store(100, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Downloads the Vosk model for `lang` plus the Windows libvosk.dll.
pub fn download_vosk(lang: &str) -> Result<()> {
    let url = VOSK_MODELS
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, u)| *u)
        .ok_or_else(|| anyhow!("不支持的 Vosk 语言: {lang}（用 models list 查看）"))?;

    let base = crate::config::default_models_dir();
    std::fs::create_dir_all(&base)?;
    let tmp = base.join("download.zip");
    download(url, &tmp)?;

    println!("解压模型...");
    let target = base.join("vosk");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    let file = File::open(&tmp)?;
    let mut zip = zip::ZipArchive::new(file)?;
    zip.extract(&base)?;
    // 压缩包内是 vosk-model-small-xx-0.xx/ 目录，改名成 vosk/
    let inner = std::fs::read_dir(&base)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("vosk-model"))
                    .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("压缩包中未找到模型目录"))?;
    std::fs::rename(inner, &target)?;
    let _ = std::fs::remove_file(&tmp);
    println!("模型已就绪: {}", target.display());

    let dll_tmp = base.join("vosk-win64.zip");
    download(DLL_URL, &dll_tmp)?;
    println!("解压 libvosk.dll...");
    let file = File::open(&dll_tmp)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if name.ends_with("libvosk.dll") {
            let mut out = File::create(base.join("libvosk.dll"))?;
            std::io::copy(&mut entry, &mut out)?;
        } else if name.ends_with("libgcc_s_seh-1.dll")
            || name.ends_with("libstdc++-6.dll")
            || name.ends_with("libwinpthread-1.dll")
        {
            let fname = name.rsplit('/').next().unwrap_or(&name);
            let mut out = File::create(base.join(fname))?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    let _ = std::fs::remove_file(&dll_tmp);

    // 复制到 exe 目录，方便 vosk crate 直接加载
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in [
                "libvosk.dll",
                "libgcc_s_seh-1.dll",
                "libstdc++-6.dll",
                "libwinpthread-1.dll",
            ] {
                let src = base.join(name);
                if src.exists() {
                    let _ = std::fs::copy(&src, dir.join(name));
                }
            }
            println!("运行库已复制到: {}", dir.display());
        }
    }
    Ok(())
}


/// Silero VAD model (downloaded on demand).
pub fn download_silero() -> Result<()> {
    let dir = crate::config::default_models_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("silero_vad_v5.onnx");
    if dest.exists() {
        println!("已存在: {}", dest.display());
        return Ok(());
    }    download(
        "https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx",
        &dest,
    )?;
    println!("已下载: {}", dest.display());
    Ok(())
}

/// 本地 Piper 音色（sherpa-onnx 打包），按语言放到 models/piper/<code>/。
pub fn download_piper(lang: &str) -> Result<()> {
    let l = crate::lang::get(lang)?;
    if l.piper.is_empty() {
        return Err(anyhow!("{} 暂无本地 Piper 音色，请用 edge 在线朗读", l.label));
    }
    let url = format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-{}.tar.bz2",
        l.piper
    );
    let base = crate::config::default_models_dir();
    let target = base.join("piper").join(l.code);
    extract_tarball(&url, &base.join(format!("piper-{}.tar.bz2", l.code)), &target, |_| true)?;
    println!("Piper {} 音色已就绪: {}", l.label, target.display());
    Ok(())
}

/// 下载 .tar.bz2 并把其中唯一的顶层目录的内容解到 `target`（先解到临时目录，成功后整体替换）。
fn extract_tarball(
    url: &str,
    tmp: &PathBuf,
    target: &PathBuf,
    keep: impl Fn(&str) -> bool,
) -> Result<()> {
    std::fs::create_dir_all(tmp.parent().unwrap_or(target))?;
    download(url, tmp)?;
    let staging = PathBuf::from(format!("{}.extract", target.display()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let unpack = || -> Result<()> {
        let file = File::open(tmp)?;
        let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(file));
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            // 去掉顶层目录：pkg-name/xxx → xxx
            let rel: PathBuf = path.components().skip(1).collect();
            if rel.as_os_str().is_empty() {
                continue;
            }
            let name = rel.to_string_lossy().replace('\\', "/");
            if !keep(&name) {
                continue;
            }
            let out = staging.join(&rel);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            entry.unpack(&out)?;
        }
        Ok(())
    };
    let r = unpack();
    let _ = std::fs::remove_file(tmp);
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if target.exists() {
        std::fs::remove_dir_all(target)?;
    }
    std::fs::rename(&staging, target)?;
    Ok(())
}

/// Whisper-small（多语种识别，SenseVoice 不覆盖的语言用它）。只保留 int8 文件，省 1 GB 磁盘。
pub fn download_whisper() -> Result<()> {
    let base = crate::config::default_models_dir();
    let target = base.join("whisper");
    extract_tarball(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-small.tar.bz2",
        &base.join("whisper.tar.bz2"),
        &target,
        |name| name.ends_with(".int8.onnx") || name.ends_with("tokens.txt"),
    )?;
    println!("Whisper 模型已就绪: {}", target.display());
    Ok(())
}

/// NLLB-200（600M 蒸馏版，量化 ONNX）：一个模型覆盖 200 种语言任意互译。
pub fn download_nllb() -> Result<()> {
    let dir = crate::config::default_models_dir().join("nllb");
    std::fs::create_dir_all(&dir)?;
    let repo = "https://huggingface.co/Xenova/nllb-200-distilled-600M/resolve/main";
    for remote in [
        "onnx/encoder_model_quantized.onnx",
        "onnx/decoder_model_merged_quantized.onnx",
        "tokenizer.json",
        "config.json",
    ] {
        let local = dir.join(remote.rsplit('/').next().unwrap_or(remote));
        if local.exists() {
            continue;
        }
        download(&format!("{repo}/{remote}"), &local)?;
    }
    println!("NLLB 翻译模型已就绪: {}", dir.display());
    Ok(())
}

/// 首次使用需要下载的模型（按当前语言与引擎算），供设置面板"一键准备"。
pub struct Need {
    pub id: String,
    pub label: String,
    pub size_mb: u32,
}

pub fn needed(cfg: &crate::config::Config) -> Vec<Need> {
    let mut out: Vec<Need> = Vec::new();
    let mut push = |id: String, label: String, size_mb: u32| {
        if !out.iter().any(|n| n.id == id) {
            out.push(Need { id, label, size_mb });
        }
    };
    let models = crate::config::default_models_dir();
    let dirs_active = cfg.directions();
    // 要识别的语言 = 各方向的源语言；要朗读的语言 = 各方向的目标语言
    let heard: Vec<_> = dirs_active.iter().map(|d| d.0).collect();
    let spoken: Vec<_> = dirs_active.iter().map(|d| d.1).collect();
    if cfg.dsp.vad == "silero" && !models.join("silero_vad_v5.onnx").exists() {
        push("silero".into(), "语音检测".into(), 2);
    }
    if cfg.realtime.provider == "none" {
        if cfg.stt.provider == "local" {
            for &l in &heard {
                if l.sensevoice && !crate::stt::sensevoice_ready() {
                    push("sensevoice".into(), "语音识别（中英日韩粤）".into(), 230);
                } else if !l.sensevoice && !crate::stt::whisper_ready() {
                    push("whisper".into(), format!("语音识别（{}等）", l.label), 380);
                }
            }
        }
        if matches!(cfg.translate.provider.as_str(), "local" | "marian" | "nllb") {
            let only = match cfg.translate.provider.as_str() {
                "marian" => Some(crate::nmt::Family::Marian),
                "nllb" => Some(crate::nmt::Family::Nllb),
                _ => None,
            };
            let dirs = crate::nmt::ModelDirs::new(&cfg.translate.marian_dir);
            for &(a, b) in &dirs_active {
                let legs = crate::nmt::plan(&dirs, a, b, only);
                for leg in crate::nmt::missing(&dirs, &legs) {
                    match leg.family {
                        crate::nmt::Family::Marian => push(
                            format!("marian:{}-{}", leg.src.code, leg.tgt.code),
                            format!("翻译模型（{}→{}）", leg.src.label, leg.tgt.label),
                            250,
                        ),
                        crate::nmt::Family::Nllb => {
                            push("nllb".into(), "多语种翻译模型".into(), 910)
                        }
                    }
                }
            }
        }
        if cfg.tts.provider == "sherpa" {
            for &l in &spoken {
                let dir = models.join("piper").join(l.code);
                let legacy = models.join("tts").join(format!("{}.onnx", l.piper));
                if !l.piper.is_empty() && !dir.is_dir() && !legacy.exists() {
                    push(format!("piper:{}", l.code), format!("本地朗读（{}）", l.label), 70);
                }
            }
        }
    }
    out
}

pub fn fetch(id: &str) -> Result<()> {
    match id {
        "silero" => download_silero(),
        "sensevoice" => download_sensevoice(),
        "whisper" => download_whisper(),
        "nllb" => download_nllb(),
        "marian" => download_marian(),
        "zipformer" => download_zipformer(),
        "aec" => download_aec(),
        other => {
            if let Some(code) = other.strip_prefix("piper:") {
                return download_piper(code);
            }
            if let Some(pair) = other.strip_prefix("marian:") {
                let (a, b) = pair
                    .split_once('-')
                    .ok_or_else(|| anyhow!("Marian 语言对格式应为 en-zh: {pair}"))?;
                return download_marian_pair(a, b);
            }
            Err(anyhow!("未知模型: {other}"))
        }
    }
}

/// Local ONNX speech recognition (SenseVoice, int8, zh/en/ja/ko/yue).
pub fn download_sensevoice() -> Result<()> {
    let base = crate::config::default_models_dir();
    let target = base.join("sensevoice");
    extract_tarball(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2",
        &base.join("sensevoice.tar.bz2"),
        &target,
        |name| !name.starts_with("test_wavs"),
    )?;
    println!("SenseVoice 模型已就绪: {}", target.display());
    Ok(())
}

/// Streaming Zipformer (bilingual zh/en, ONNX)。准确率不如 SenseVoice，仅为兼容旧配置保留。
pub fn download_zipformer() -> Result<()> {
    let base = crate::config::default_models_dir();
    let target = base.join("zipformer");
    extract_tarball(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.tar.bz2",
        &base.join("zipformer.tar.bz2"),
        &target,
        |_| true,
    )?;
    println!("Zipformer 模型已就绪: {}", target.display());
    Ok(())
}

/// Marian（opus-mt）某一语言对的 int8 ONNX，来自 HuggingFace 的 Xenova 转换版，约 250 MB。
pub fn download_marian_pair(src: &str, tgt: &str) -> Result<()> {
    let (s, t) = (crate::lang::get(src)?, crate::lang::get(tgt)?);
    let dir = crate::config::default_models_dir()
        .join("marian")
        .join(format!("{}-{}", s.code, t.code));
    std::fs::create_dir_all(&dir)?;
    let repo = format!(
        "https://huggingface.co/Xenova/opus-mt-{}-{}/resolve/main",
        s.code, t.code
    );
    // 先下大文件、最后下 config.json：nmt::ready() 以 config+tokenizer 判定就绪，
    // 中途失败不会被误认为可用
    // decoder 用 int8 版（输出层保留 fp32）：比全量化版大，但在 CPU 上快 2~3 倍
    for remote in [
        "onnx/encoder_model_int8.onnx",
        "onnx/decoder_model_int8.onnx",
        "tokenizer.json",
        "config.json",
    ] {
        let local = dir.join(remote.rsplit('/').next().unwrap_or(remote));
        if local.exists() {
            continue;
        }
        download(&format!("{repo}/{remote}"), &local)?;
    }
    println!("Marian {}→{} 已就绪: {}", s.label, t.label, dir.display());
    Ok(())
}

/// 中英互译（兼容旧命令 `models marian`）
pub fn download_marian() -> Result<()> {
    download_marian_pair("zh", "en")?;
    download_marian_pair("en", "zh")
}

/// Optional neural AEC model (ICASSP 2022 DEC baseline, 5 MB).
pub fn download_aec() -> Result<()> {
    let url = "https://raw.githubusercontent.com/PINTO0309/onnx-aec/main/dec-baseline-model-icassp2022.onnx";
    let dir = crate::config::default_models_dir().join("aec");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("dec-baseline.onnx");
    if dest.exists() {
        println!("AEC 模型已存在: {}", dest.display());
        return Ok(());
    }
    download(url, &dest)?;
    println!("AEC 模型已就绪: {}", dest.display());
    Ok(())
}


