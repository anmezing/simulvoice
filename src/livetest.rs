use anyhow::{anyhow, Result};
use rodio::Source;
use std::time::Duration;


use crate::config::Config;
use crate::gemini_live::{GeminiLive, LiveEvent};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn decode_16k(path: &str) -> Result<Vec<i16>> {
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
        .ok_or_else(|| anyhow!("用法: livetest <音频文件> [--config <文件>]"))?;
    let cfg = Config::load(flag(args, "--config").as_deref())?;
    let key = std::env::var("GEMINI_API_KEY")
        .map_err(|_| anyhow!("GEMINI_API_KEY 未设置（keys.toml 或环境变量）"))?;
    let instruction = format!(
        "You are a simultaneous interpreter. Translate everything you hear into {}. \
         Speak only the translation, naturally and concisely.",
        cfg.translate.target
    );
    let mut live = GeminiLive::connect(
        &key,
        &cfg.realtime.model,
        &instruction,
        &cfg.realtime.voice,
        &cfg.realtime.output_device,
    )?;

    let mut pcm = decode_16k(&path)?;
    pcm.extend(std::iter::repeat(0i16).take(16_000 * 2));
    println!("[live] 模型 {} / 音频 {} 采样", cfg.realtime.model, pcm.len());

    let mut audio_chunks = 0usize;
    let mut input_text = String::new();
    let mut output_text = String::new();
    let mut ready = false;
    let mut errors: Vec<String> = Vec::new();

    for chunk in pcm.chunks(1600) {
        live.feed(chunk)?;
        std::thread::sleep(Duration::from_millis(100));
        for ev in live.poll() {
            match ev {
                LiveEvent::Ready => ready = true,
                LiveEvent::Audio => audio_chunks += 1,
                LiveEvent::Input(t) => input_text.push_str(&t),
                LiveEvent::Output(t) => output_text.push_str(&t),
                LiveEvent::Error(e) => errors.push(e),
            }
        }
    }
    live.end_input();
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        for ev in live.poll() {
            match ev {
                LiveEvent::Ready => ready = true,
                LiveEvent::Audio => audio_chunks += 1,
                LiveEvent::Input(t) => input_text.push_str(&t),
                LiveEvent::Output(t) => output_text.push_str(&t),
                LiveEvent::Error(e) => errors.push(e),
            }
        }
    }

    println!("[live] setup={ready}, 音频块={audio_chunks}");
    println!("[live] 输入转写: {}", input_text.trim());
    println!("[live] 输出转写: {}", output_text.trim());
    for e in errors {
        eprintln!("[live] 错误: {e}");
    }
    Ok(())
}






