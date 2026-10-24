use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use std::io::Cursor;

pub fn list_output_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for dev in host.output_devices()? {
        names.push(dev.name()?);
    }
    Ok(names)
}

pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for dev in host.input_devices()? {
        names.push(dev.name()?);
    }
    Ok(names)
}

fn find_device(name: &str) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let needle = name.to_lowercase();
    for dev in host.output_devices()? {
        if let Ok(n) = dev.name() {
            if n.to_lowercase().contains(&needle) {
                return Ok(dev);
            }
        }
    }
    Err(anyhow!("找不到输出设备包含 '{name}'"))
}

/// Plays MP3 on the default device, or the first device whose name contains `device`.
pub fn play_mp3(mp3: Vec<u8>, device: Option<&str>) -> Result<()> {
    let (_stream, handle) = match device {
        Some(name) if !name.trim().is_empty() => {
            let dev = find_device(name)?;
            rodio::OutputStream::try_from_device(&dev)?
        }
        _ => rodio::OutputStream::try_default()?,
    };
    let sink = rodio::Sink::try_new(&handle)?;
    let src = rodio::Decoder::new(Cursor::new(mp3))?;
    sink.append(src);
    sink.sleep_until_end();
    Ok(())
}


pub fn play_pcm(samples: &[f32], rate: u32, device: Option<&str>) -> Result<()> {
    let (_stream, handle) = match device {
        Some(name) if !name.trim().is_empty() => {
            let dev = find_device(name)?;
            rodio::OutputStream::try_from_device(&dev)?
        }
        _ => rodio::OutputStream::try_default()?,
    };
    let sink = rodio::Sink::try_new(&handle)?;
    let buf = rodio::buffer::SamplesBuffer::new(1, rate, samples.to_vec());
    sink.append(buf);
    sink.sleep_until_end();
    Ok(())
}

pub fn open_output(device: Option<&str>) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle)> {
    match device {
        Some(name) if !name.trim().is_empty() => {
            let dev = find_device(name)?;
            Ok(rodio::OutputStream::try_from_device(&dev)?)
        }
        _ => Ok(rodio::OutputStream::try_default()?),
    }
}

pub fn default_output_name() -> Option<String> {
    let host = cpal::default_host();
    host.default_output_device()?.name().ok()
}





/// Picks sensible devices for the Teams preset:
/// virtual mic = first CABLE device, listening = first non-default, non-virtual output.
pub fn auto_pick_teams(output_device: &mut String, remote_output_device: &mut String) -> (bool, bool) {
    let outputs = list_output_devices().unwrap_or_default();
    let default_name = default_output_name().unwrap_or_default().to_lowercase();

    let mut cable_found = false;
    if let Some(cable) = outputs.iter().find(|n| {
        let l = n.to_lowercase();
        l.contains("cable") || l.contains("vb-audio") || l.contains("voicemeeter")
    }) {
        *remote_output_device = cable.clone();
        cable_found = true;
    }

    let preferred = outputs.iter().find(|n| {
        let l = n.to_lowercase();
        l.contains("耳机") || l.contains("headphone") || l.contains("headset") || l.contains("earphone")
    });
    let fallback = outputs.iter().find(|n| {
        let l = n.to_lowercase();
        l != default_name && !l.contains("cable") && !l.contains("vb-audio")
    });
    let mut output_found = false;
    if let Some(dev) = preferred.or(fallback) {
        *output_device = dev.clone();
        output_found = true;
    }
    (cable_found, output_found)
}

