use anyhow::{anyhow, Result};
use std::time::Duration;


use crate::config::Config;
use crate::volc::{self, VolcSession};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn decode_16k(path: &str) -> Result<Vec<i16>> {
    use rodio::Source;
    let file = std::fs::File::open(path)?;
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
    Ok(crate::dsp::resample_all_16k(&mono, rate))
}

pub fn run(args: &[String]) -> Result<()> {
    let path = args
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow!("用法: volctest <音频文件> [--config <文件>]"))?;
    let cfg = Config::load(flag(args, "--config").as_deref())?;
    let app_key = std::env::var("VOLC_APP_ID").map_err(|_| anyhow!("VOLC_APP_ID 未设置"))?;
    let access = std::env::var("VOLC_ACCESS_TOKEN")
        .map_err(|_| anyhow!("VOLC_ACCESS_TOKEN 未设置"))?;
    let resource = std::env::var("VOLC_RESOURCE_ID")
        .unwrap_or_else(|_| "volc.service_type.10053".into());

    let mut pcm = decode_16k(&path)?;
    pcm.extend(std::iter::repeat(0i16).take(16_000 * 2));

    let source = if cfg.stt.language == "auto" {
        "zh".to_string()
    } else {
        crate::palabra::lang_code(&cfg.stt.language).to_string()
    };
    let target = crate::palabra::lang_code(&cfg.translate.target).to_string();
    println!("[volc] {} → {} / {} 采样", source, target, pcm.len());

    let mut session = VolcSession::connect(&app_key, &access, &resource, &source, &target)?;
    let mut src_text = String::new();
    let mut dst_text = String::new();
    let mut errors: Vec<String> = Vec::new();
    let mut failed = false;

    for chunk in pcm.chunks(3200) {
        session.send_audio(chunk)?;
        std::thread::sleep(Duration::from_millis(200));
        for r in session.poll() {
            match r.event {
                volc::EVENT_SESSION_FAILED => {
                    failed = true;
                    errors.push(format!("SessionFailed: {}", r.message));
                }
                volc::EVENT_SOURCE_RESPONSE => {
                    if !r.text.trim().is_empty() {
                        println!("[volc] 原声: {}", r.text.trim());
                        src_text.push_str(&r.text);
                    }
                }
                volc::EVENT_TRANSLATION_RESPONSE => {
                    if !r.text.trim().is_empty() {
                        println!("[volc] 译文: {}", r.text.trim());
                        dst_text.push_str(&r.text);
                    }
                }
                other => {
                    if std::env::var("SIMULVOICE_DEBUG").map(|v| !v.is_empty()).unwrap_or(false) {
                        eprintln!("[volc] event={} status={} msg={} text={}", other, r.status, r.message, r.text);
                    }
                    if r.status != 20_000_000 && !r.message.is_empty() && r.message != "OK" {
                        errors.push(r.message);
                    }
                }
            }
        }
        if failed {
            break;
        }
    }
    session.finish();
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        for r in session.poll() {
            match r.event {
                volc::EVENT_TRANSLATION_RESPONSE => {
                    if !r.text.trim().is_empty() {
                        println!("[volc] 译文: {}", r.text.trim());
                        dst_text.push_str(&r.text);
                    }
                }
                volc::EVENT_SOURCE_RESPONSE => {
                    if !r.text.trim().is_empty() {
                        src_text.push_str(&r.text);
                    }
                }
                _ => {}
            }
        }
    }

    println!("[volc] 原声汇总: {}", src_text.trim());
    println!("[volc] 译文汇总: {}", dst_text.trim());
    for e in errors {
        eprintln!("[volc] 错误: {e}");
    }
    Ok(())
}




