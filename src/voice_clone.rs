//! 「我的音色」：录一段自己的声音，出向译文用它朗读（ZipVoice 零样本克隆，sherpa-onnx，全本地）。
//!
//! 音色存放在 `<数据目录>/voices/<名字>/`：`ref.wav`（参考录音）+ `ref.txt`（录音里念的文字）。
//! 模型在 `<模型目录>/zipvoice/`，只支持朗读中文和英文。

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 参考录音最短/最长（秒）。太短学不到音色；太长每句合成都要带着它算，延迟变大
pub const MIN_REF_SECS: f32 = 3.0;
pub const MAX_REF_SECS: f32 = 20.0;
/// 录音硬上限：到点自动停，缓冲区不会无限增长
pub const MAX_RECORD_SECS: u32 = 30;
/// 最多保存多少个音色
pub const MAX_PROFILES: usize = 20;

/// 朗读用的提示文本（ZipVoice 参考文字只认中文和英文）
pub const PROMPT_ZH: &str =
    "今天的会议主要讨论三件事：项目进度、下个季度的预算，还有新同事的培训安排。大家有问题可以随时提出来。";
pub const PROMPT_EN: &str = "Thanks everyone for joining today. Let's go through the project status, \
the budget for next quarter, and the training plan for our new colleagues.";

/// 录音念哪段：要和译文（对方）语言一致。用中文录音去说英文，
/// 模型按中文语速估时长，英文会被拉慢、念糊（实测一句短话拖到 8 秒）。
pub fn prompt_for(lang: &str) -> &'static str {
    if lang == "zh" {
        PROMPT_ZH
    } else {
        PROMPT_EN
    }
}

/// 参考文字是哪种语言（有汉字就算中文）
pub fn text_lang(text: &str) -> &'static str {
    if text.chars().any(|c| (0x4e00..=0x9fff).contains(&(c as u32))) {
        "zh"
    } else {
        "en"
    }
}

/// 音色是用哪种语言录的；读不到返回 None
pub fn profile_lang(name: &str) -> Option<&'static str> {
    let dir = profile_dir(name).ok()?;
    let text = std::fs::read_to_string(dir.join("ref.txt")).ok()?;
    Some(text_lang(&text))
}

/// 采样步数：0 = 自动。实测（int8 蒸馏版，4 线程）：英文 2 步就能准确复述、RTF≈0.65；
/// 中文 2 步会错字，3 步才准（RTF≈0.5）。步数越多越细腻也越慢
pub fn steps_for(configured: u32, lang: &str) -> i32 {
    match configured {
        0 => if lang == "zh" { 3 } else { 2 },
        n => n.clamp(1, 8) as i32,
    }
}

/// ZipVoice 能朗读的语言
pub fn supports(lang: &str) -> bool {
    matches!(lang, "zh" | "en")
}

pub fn model_dir() -> PathBuf {
    crate::config::default_models_dir().join("zipvoice")
}

struct ModelFiles {
    encoder: PathBuf,
    decoder: PathBuf,
    tokens: PathBuf,
    vocoder: PathBuf,
    lexicon: String,
    data_dir: Option<PathBuf>,
}

fn find_onnx(dir: &Path, prefix: &str) -> Option<PathBuf> {
    [format!("{prefix}.int8.onnx"), format!("{prefix}.onnx")]
        .into_iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn model_files() -> Result<ModelFiles> {
    let dir = model_dir();
    let need = |p: PathBuf| -> Result<PathBuf> {
        if p.is_file() {
            Ok(p)
        } else {
            Err(anyhow!("克隆模型缺文件 {}（simulvoice models zipvoice）", p.display()))
        }
    };
    let encoder = find_onnx(&dir, "encoder")
        .ok_or_else(|| anyhow!("克隆模型缺 encoder（simulvoice models zipvoice）"))?;
    let decoder = find_onnx(&dir, "decoder")
        .ok_or_else(|| anyhow!("克隆模型缺 decoder（simulvoice models zipvoice）"))?;
    let tokens = need(dir.join("tokens.txt"))?;
    let vocoder = need(dir.join("vocos_24khz.onnx"))?;
    // 中文走 lexicon.txt，英文走 espeak-ng。sherpa 示例里的 lexicon-en-us.txt 是「词 || 音素」格式，
    // 1.13.8 不认，每行都报 Unknown token（实测一句话刷出 30 MB 日志），所以不用它
    let lexicon = need(dir.join("lexicon.txt"))?.display().to_string();
    let data = dir.join("espeak-ng-data");
    if !data.is_dir() {
        return Err(anyhow!("克隆模型缺 espeak-ng-data（simulvoice models zipvoice）"));
    }
    Ok(ModelFiles {
        encoder,
        decoder,
        tokens,
        vocoder,
        lexicon,
        data_dir: Some(data),
    })
}

pub fn model_ready() -> bool {
    model_files().is_ok()
}

// ───────────────────────────── 音色存储 ─────────────────────────────

pub fn voices_dir() -> PathBuf {
    crate::paths::data_dir().join("voices")
}

/// 音色名只允许字母、数字、汉字、空格、- 和 _，最长 32 字。
/// 名字会拼进路径，其他字符（. / \ : 等）一律拒绝。
pub fn validate_name(name: &str) -> Result<&str> {
    let n = name.trim();
    if n.is_empty() {
        return Err(anyhow!("音色名不能为空"));
    }
    if n.chars().count() > 32 {
        return Err(anyhow!("音色名最长 32 个字"));
    }
    if !n
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ')
    {
        return Err(anyhow!("音色名只能用字母、数字、汉字、空格、- 和 _"));
    }
    Ok(n)
}

fn profile_dir(name: &str) -> Result<PathBuf> {
    Ok(voices_dir().join(validate_name(name)?))
}

/// 已保存的音色（ref.wav 与 ref.txt 都在的才算）
pub fn list() -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(voices_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("ref.wav").is_file() && e.path().join("ref.txt").is_file())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| validate_name(n).is_ok())
        .collect();
    out.sort();
    out
}

pub fn exists(name: &str) -> bool {
    profile_dir(name)
        .map(|d| d.join("ref.wav").is_file() && d.join("ref.txt").is_file())
        .unwrap_or(false)
}

pub struct Profile {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub text: String,
}

pub fn load(name: &str) -> Result<Profile> {
    let dir = profile_dir(name)?;
    let text = std::fs::read_to_string(dir.join("ref.txt"))
        .with_context(|| format!("音色「{name}」缺 ref.txt"))?
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(anyhow!("音色「{name}」的参考文字为空"));
    }
    let (samples, rate) = read_wav(&dir.join("ref.wav"))
        .with_context(|| format!("音色「{name}」的录音读不出来"))?;
    Ok(Profile { samples, rate, text })
}

fn read_wav(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample.clamp(8, 32) - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mono = raw
        .chunks(ch)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

/// 读任意常见音频（wav 用 hound，mp3/flac/ogg 用 rodio 解码），转成单声道 f32。
/// 解码上限 MAX_RECORD_SECS×2 秒，防止误选一个几小时的文件把内存吃满。
fn read_audio(path: &Path) -> Result<(Vec<f32>, u32)> {
    let is_wav = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);
    if is_wav {
        return read_wav(path);
    }
    use rodio::Source;
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let dec = rodio::Decoder::new(file).map_err(|e| anyhow!("解不开这个音频（支持 wav/mp3/flac/ogg）: {e}"))?;
    let ch = dec.channels().max(1) as usize;
    let rate = dec.sample_rate();
    let limit = rate as usize * ch * (MAX_RECORD_SECS as usize * 2);
    let raw: Vec<f32> = dec.take(limit).map(|s| s as f32 / 32768.0).collect();
    let mono = raw
        .chunks(ch)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
    Ok((mono, rate))
}

/// 去掉首尾静音、检查长度和音量。返回可直接用作参考的样本。
pub fn prepare(samples: &[f32], rate: u32) -> Result<Vec<f32>> {
    let rate_f = rate.max(1) as f32;
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak < 0.02 {
        return Err(anyhow!("几乎没录到声音，检查一下麦克风"));
    }
    // 20ms 一帧，低于峰值 5% 的帧算静音
    let frame = (rate / 50).max(1) as usize;
    let thr = peak * 0.05;
    let loud = |c: &[f32]| c.iter().fold(0.0f32, |m, s| m.max(s.abs())) >= thr;
    let frames: Vec<&[f32]> = samples.chunks(frame).collect();
    let first = frames.iter().position(|c| loud(c)).unwrap_or(0);
    let last = frames.iter().rposition(|c| loud(c)).unwrap_or(frames.len().saturating_sub(1));
    // 前后各留 0.15 秒
    let pad = (rate_f * 0.15) as usize;
    let start = (first * frame).saturating_sub(pad);
    let end = ((last + 1) * frame + pad).min(samples.len());
    let out = samples[start..end].to_vec();
    let secs = out.len() as f32 / rate_f;
    if secs < MIN_REF_SECS {
        return Err(anyhow!("有效录音只有 {secs:.1} 秒，至少要 {MIN_REF_SECS:.0} 秒"));
    }
    if secs > MAX_REF_SECS {
        return Err(anyhow!(
            "录音 {secs:.1} 秒太长（最多 {MAX_REF_SECS:.0} 秒），越长每句合成越慢"
        ));
    }
    if peak >= 0.999 {
        eprintln!("[音色] 录音有削波（声音太大），音色可能失真，建议离麦克风远一点重录");
    }
    Ok(out)
}

/// 保存音色：先写到临时目录，全部成功后再替换，不会留下只有一半文件的音色
pub fn save(name: &str, samples: &[f32], rate: u32, text: &str) -> Result<()> {
    let name = validate_name(name)?;
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!("参考文字不能为空（录音里念的是什么）"));
    }
    let dir = profile_dir(name)?;
    if !dir.exists() && list().len() >= MAX_PROFILES {
        return Err(anyhow!("最多保存 {MAX_PROFILES} 个音色，先删掉不用的"));
    }
    let root = voices_dir();
    std::fs::create_dir_all(&root)?;
    let tmp = root.join(format!(".{name}.tmp"));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let write = || -> Result<()> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(tmp.join("ref.wav"), spec)?;
        for &s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        w.finalize()?;
        std::fs::write(tmp.join("ref.txt"), text)?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&tmp, &dir)?;
    Ok(())
}

pub fn delete(name: &str) -> Result<()> {
    let dir = profile_dir(name)?;
    if !dir.exists() {
        return Err(anyhow!("没有叫「{name}」的音色"));
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// 导入现成的录音（wav）作为音色
pub fn import(name: &str, wav: &Path, text: &str) -> Result<()> {
    let (samples, rate) = read_audio(wav).with_context(|| format!("读不了 {}", wav.display()))?;
    let trimmed = prepare(&samples, rate)?;
    save(name, &trimmed, rate, text)
}

// ───────────────────────────── 录音 ─────────────────────────────

/// 从麦克风录音，到 MAX_RECORD_SECS 自动停止采集。drop 即释放设备。
pub struct MicRecorder {
    _stream: cpal::Stream,
    buf: Arc<Mutex<Vec<f32>>>,
    pub rate: u32,
}

impl MicRecorder {
    /// `device`：设备名子串，空 = 系统默认麦克风
    pub fn start(device: &str) -> Result<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let dev = if device.trim().is_empty() {
            host.default_input_device()
                .ok_or_else(|| anyhow!("没有可用的麦克风"))?
        } else {
            let needle = device.trim().to_lowercase();
            host.input_devices()?
                .find(|d| d.name().map(|n| n.to_lowercase().contains(&needle)).unwrap_or(false))
                .ok_or_else(|| anyhow!("找不到麦克风「{device}」"))?
        };
        let supported = dev.default_input_config()?;
        let rate = supported.sample_rate().0;
        let channels = supported.channels().max(1) as usize;
        let cap = rate as usize * MAX_RECORD_SECS as usize;
        let buf = Arc::new(Mutex::new(Vec::with_capacity(cap)));
        let config: cpal::StreamConfig = supported.config();
        let err = |e| eprintln!("[音色] 录音出错: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let b = buf.clone();
                dev.build_input_stream(
                    &config,
                    move |data: &[f32], _| push_frames(&b, data.chunks(channels).map(|c| c.iter().sum::<f32>() / c.len() as f32), cap),
                    err,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let b = buf.clone();
                dev.build_input_stream(
                    &config,
                    move |data: &[i16], _| push_frames(&b, data.chunks(channels).map(|c| c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / c.len() as f32), cap),
                    err,
                    None,
                )?
            }
            cpal::SampleFormat::I32 => {
                let b = buf.clone();
                dev.build_input_stream(
                    &config,
                    move |data: &[i32], _| push_frames(&b, data.chunks(channels).map(|c| c.iter().map(|&s| s as f32 / 2147483648.0).sum::<f32>() / c.len() as f32), cap),
                    err,
                    None,
                )?
            }
            other => return Err(anyhow!("麦克风采样格式 {other:?} 暂不支持")),
        };
        stream.play()?;
        Ok(Self { _stream: stream, buf, rate })
    }

    pub fn seconds(&self) -> f32 {
        self.buf.lock().map(|b| b.len()).unwrap_or(0) as f32 / self.rate.max(1) as f32
    }

    /// 当前电平（最近 0.1 秒峰值），给界面画音量条
    pub fn level(&self) -> f32 {
        let Ok(b) = self.buf.lock() else { return 0.0 };
        let n = (self.rate / 10) as usize;
        b[b.len().saturating_sub(n)..].iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    pub fn full(&self) -> bool {
        self.seconds() >= MAX_RECORD_SECS as f32
    }

    pub fn finish(self) -> (Vec<f32>, u32) {
        let rate = self.rate;
        let samples = self.buf.lock().map(|b| b.clone()).unwrap_or_default();
        (samples, rate)
    }
}

fn push_frames(buf: &Mutex<Vec<f32>>, frames: impl Iterator<Item = f32>, cap: usize) {
    if let Ok(mut b) = buf.lock() {
        for f in frames {
            if b.len() >= cap {
                break;
            }
            b.push(f);
        }
    }
}

// ───────────────────────────── 合成 ─────────────────────────────

/// 克隆引擎：模型 + 一份参考录音。每句译文都带着参考录音生成，所以参考越短越快。
pub struct CloneTts {
    tts: sherpa_onnx::OfflineTts,
    profile: Profile,
    steps: i32,
}

/// 线程数：实测 4 → 8 线程几乎不再变快，留核给识别和翻译
fn clone_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(2, 4) as i32)
        .unwrap_or(2)
}

impl CloneTts {
    /// `steps`：见 steps_for
    pub fn new(name: &str, provider: &str, steps: i32) -> Result<Self> {
        use sherpa_onnx::{OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsZipvoiceModelConfig};
        // 先加载可能失败的音色，再加载模型，任何一步失败都不留半成品
        let profile = load(name)?;
        let f = model_files()?;
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                zipvoice: OfflineTtsZipvoiceModelConfig {
                    tokens: Some(f.tokens.display().to_string()),
                    encoder: Some(f.encoder.display().to_string()),
                    decoder: Some(f.decoder.display().to_string()),
                    vocoder: Some(f.vocoder.display().to_string()),
                    data_dir: f.data_dir.map(|d| d.display().to_string()),
                    lexicon: Some(f.lexicon),
                    feat_scale: 0.1,
                    t_shift: 0.5,
                    target_rms: 0.1,
                    guidance_scale: 1.0,
                },
                num_threads: clone_threads(),
                debug: false,
                provider: Some(if provider.trim().is_empty() { "cpu".into() } else { provider.to_string() }),
                ..Default::default()
            },
            max_num_sentences: 1,
            silence_scale: 0.2,
            ..Default::default()
        };
        let tts = OfflineTts::create(&config)
            .ok_or_else(|| anyhow!("克隆模型加载失败: {}", model_dir().display()))?;
        Ok(Self { tts, profile, steps: steps.clamp(1, 8) })
    }

    pub fn render(&self, text: &str) -> Result<(Vec<f32>, u32)> {
        let cfg = sherpa_onnx::GenerationConfig {
            speed: 1.0,
            reference_audio: Some(self.profile.samples.clone()),
            reference_sample_rate: self.profile.rate as i32,
            reference_text: Some(self.profile.text.clone()),
            num_steps: self.steps,
            ..Default::default()
        };
        let audio = self
            .tts
            .generate_with_config::<fn(&[f32], f32) -> bool>(text, &cfg, None)
            .ok_or_else(|| anyhow!("克隆音色合成失败"))?;
        let samples = audio.samples().to_vec();
        if samples.is_empty() {
            return Err(anyhow!("克隆音色合成结果为空"));
        }
        Ok((samples, audio.sample_rate() as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_could_escape_the_folder_are_rejected() {
        for bad in ["", "  ", "../x", "a/b", r"a\b", "c:", "a.b", &"长".repeat(33)] {
            assert!(validate_name(bad).is_err(), "{bad:?} 应该被拒绝");
        }
        assert_eq!(validate_name(" 我的 声音_1 ").unwrap(), "我的 声音_1");
    }

    #[test]
    fn text_language_and_steps() {
        assert_eq!(text_lang(PROMPT_ZH), "zh");
        assert_eq!(text_lang(PROMPT_EN), "en");
        assert_eq!(prompt_for("zh"), PROMPT_ZH);
        assert_eq!(prompt_for("ja"), PROMPT_EN);
        assert_eq!(steps_for(0, "zh"), 3);
        assert_eq!(steps_for(0, "en"), 2);
        assert_eq!(steps_for(99, "en"), 8);
    }

    #[test]
    fn prepare_trims_silence_and_checks_length() {
        let rate = 16_000;
        let tone = |secs: f32| -> Vec<f32> {
            (0..(rate as f32 * secs) as usize).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()
        };
        let mut s = vec![0.0; rate as usize * 2];
        s.extend(tone(5.0));
        s.extend(vec![0.0; rate as usize * 2]);
        let out = prepare(&s, rate).unwrap();
        let secs = out.len() as f32 / rate as f32;
        assert!((5.0..5.5).contains(&secs), "裁剪后 {secs} 秒");
        // 太短、太长、没声音都拒绝
        assert!(prepare(&tone(1.0), rate).is_err());
        assert!(prepare(&tone(25.0), rate).is_err());
        assert!(prepare(&vec![0.001; rate as usize * 5], rate).is_err());
    }
}
