use anyhow::{anyhow, Result};
use std::time::Duration;


use crate::config::Config;
use crate::palabra::{PalabraConfig, PalabraEvent, PalabraSession};

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
        .ok_or_else(|| anyhow!("用法: palabratest <音频文件> [--config <文件>]"))?;
    let cfg = Config::load(flag(args, "--config").as_deref())?;
    let key = std::env::var("PALABRA_API_KEY")
        .map_err(|_| anyhow!("PALABRA_API_KEY 未设置（keys.toml 或环境变量）"))?;

    let mut pcm = decode_16k(&path)?;
    pcm.extend(std::iter::repeat(0i16).take(16_000 * 2));

    let pcfg = PalabraConfig {
        api_key: key,
        source: cfg.stt.language.clone(),
        target: cfg.translate.target.clone(),
        voice: "default_low".into(),
        speech: true,
    };
    println!(
        "[palabra] {} → {} / {} 采样",
        pcfg.source,
        pcfg.target,
        pcm.len()
    );
    let mut session = PalabraSession::connect(&pcfg, None)?;

    let mut source = String::new();
    let mut target = String::new();
    let mut audio_chunks = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut closed = false;

    let mut drain = |session: &mut PalabraSession,
                     source: &mut String,
                     target: &mut String,
                     audio_chunks: &mut usize,
                     errors: &mut Vec<String>,
                     closed: &mut bool| {
        for ev in session.poll() {
            match ev {
                PalabraEvent::Task(s) => eprintln!("[palabra] task={s}"),
                PalabraEvent::SourcePartial(_) => {}
                PalabraEvent::SourceFinal(t) => {
                    println!("[palabra] 原声: {}", t.trim());
                    source.push_str(&t);
                }
                PalabraEvent::TargetPartial(_) => {}
                PalabraEvent::TargetFinal(t) => {
                    println!("[palabra] 译文: {}", t.trim());
                    target.push_str(&t);
                }
                PalabraEvent::Audio => *audio_chunks += 1,
                PalabraEvent::Error(e) => errors.push(e),
                PalabraEvent::Closed => *closed = true,
            }
        }
    };

    for chunk in pcm.chunks(3200) {
        if closed {
            break;
        }
        session.send_audio(chunk)?;
        std::thread::sleep(Duration::from_millis(200));
        drain(
            &mut session,
            &mut source,
            &mut target,
            &mut audio_chunks,
            &mut errors,
            &mut closed,
        );
    }
    let _ = session.end();
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        drain(
            &mut session,
            &mut source,
            &mut target,
            &mut audio_chunks,
            &mut errors,
            &mut closed,
        );
    }

    println!("[palabra] 音频块={audio_chunks} closed={closed}");
    println!("[palabra] 原声汇总: {}", source.trim());
    println!("[palabra] 译文汇总: {}", target.trim());
    for e in errors {
        eprintln!("[palabra] 错误: {e}");
    }
    Ok(())
}



