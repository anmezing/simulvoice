use anyhow::{anyhow, Result};
use std::fs::File;
use std::path::PathBuf;

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
    println!("本地可用模型 (Vosk, 离线 STT):");
    for (lang, url) in VOSK_MODELS {
        println!("  {lang:>3}  {}", url.rsplit('/').next().unwrap_or(url));
    }
    println!("\n本地翻译: ollama pull qwen2.5:3b （Ollama 本地服务）");
    println!("本地 TTS: Windows SAPI（系统自带，无需下载）");
}

fn download(url: &str, dest: &PathBuf) -> Result<()> {
    println!("下载 {url}");
    let resp = crate::http::agent().get(url).call()?;
    let mut reader = resp.into_reader();
    let mut file = File::create(dest)?;
    std::io::copy(&mut reader, &mut file)?;
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

/// Local Piper TTS model (sherpa-onnx build), fully offline.
pub fn download_piper(lang: &str) -> Result<()> {
    let url = match lang {
        "zh" => "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-zh_CN-huayan-medium.tar.bz2",
        "en" => "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-amy-medium.tar.bz2",
        other => return Err(anyhow!("不支持的 Piper 语言: {other}（可用 zh / en）")),
    };
    let base = crate::config::default_models_dir();
    std::fs::create_dir_all(&base)?;
    let tmp = base.join("piper.tar.bz2");
    download(url, &tmp)?;
    let target = base.join("tts");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;
    let file = File::open(&tmp)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(&target)?;
    let inner = std::fs::read_dir(&target)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| anyhow!("压缩包中未找到模型目录"))?;
    for entry in std::fs::read_dir(&inner)? {
        let entry = entry?;
        std::fs::rename(entry.path(), target.join(entry.file_name()))?;
    }
    let _ = std::fs::remove_dir(inner);
    let _ = std::fs::remove_file(&tmp);
    println!("Piper 模型已就绪: {}", target.display());
    Ok(())
}





/// Local ONNX speech recognition (SenseVoice, int8, zh/en/ja/ko/yue).
pub fn download_sensevoice() -> Result<()> {
    let url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2";
    let base = crate::config::default_models_dir();
    std::fs::create_dir_all(&base)?;
    let tmp = base.join("sensevoice.tar.bz2");
    download(url, &tmp)?;
    let target = base.join("sensevoice");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;
    let file = File::open(&tmp)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(&target)?;
    let inner = std::fs::read_dir(&target)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| anyhow!("压缩包内未找到模型目录"))?;
    for entry in std::fs::read_dir(&inner)? {
        let entry = entry?;
        std::fs::rename(entry.path(), target.join(entry.file_name()))?;
    }
    let _ = std::fs::remove_dir(inner);
    let _ = std::fs::remove_file(&tmp);
    println!("SenseVoice 模型已就绪: {}", target.display());
    Ok(())
}


/// Streaming Zipformer (bilingual zh/en, ONNX) for low-latency ASR.
pub fn download_zipformer() -> Result<()> {
    let url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.tar.bz2";
    let base = crate::config::default_models_dir();
    std::fs::create_dir_all(&base)?;
    let tmp = base.join("zipformer.tar.bz2");
    download(url, &tmp)?;
    let target = base.join("zipformer");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;
    let file = File::open(&tmp)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(&target)?;
    let inner = std::fs::read_dir(&target)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| anyhow!("压缩包内未找到模型目录"))?;
    for entry in std::fs::read_dir(&inner)? {
        let entry = entry?;
        std::fs::rename(entry.path(), target.join(entry.file_name()))?;
    }
    let _ = std::fs::remove_dir(inner);
    let _ = std::fs::remove_file(&tmp);
    println!("Zipformer 模型已就绪: {}", target.display());
    Ok(())
}


/// Optional offline MarianNMT (zh<->en) int8 ONNX models.
pub fn download_marian() -> Result<()> {
    let base = crate::config::default_models_dir().join("marian");
    for (pair, repo) in [("zh-en", "Xenova/opus-mt-zh-en"), ("en-zh", "Xenova/opus-mt-en-zh")] {
        let dir = base.join(pair);
        std::fs::create_dir_all(&dir)?;
        for (remote, local) in [
            ("onnx/encoder_model_int8.onnx", "encoder_model_int8.onnx"),
            ("onnx/decoder_model_int8.onnx", "decoder_model_int8.onnx"),
            ("tokenizer.json", "tokenizer.json"),
            ("config.json", "config.json"),
        ] {
            let dest = dir.join(local);
            if dest.exists() {
                continue;
            }
            let url = format!("https://huggingface.co/{repo}/resolve/main/{remote}");
            download(&url, &dest)?;
        }
        println!("Marian {pair} 已就绪: {}", dir.display());
    }
    Ok(())
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

