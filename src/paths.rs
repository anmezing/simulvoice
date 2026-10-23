use std::path::{Path, PathBuf};

/// Location marker written next to the executable (falls back to %APPDATA%).
fn marker_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("simulvoice.data-dir");
        }
    }
    legacy_dir().join("data-dir.txt")
}

fn legacy_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("SimulVoice")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".simulvoice")
    }
}

/// Picks a large-data location: first non-C fixed drive, else beside the exe.
fn choose_default() -> PathBuf {
    for letter in b'D'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if Path::new(&root).exists() {
            return PathBuf::from(root).join("SimulVoiceData");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("SimulVoiceData");
        }
    }
    legacy_dir()
}

pub fn data_dir() -> PathBuf {
    if let Ok(v) = std::env::var("SIMULVOICE_DATA_DIR") {
        if !v.trim().is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(text) = std::fs::read_to_string(marker_path()) {
        let p = text.trim();
        if !p.is_empty() {
            let pb = PathBuf::from(p);
            if pb.is_absolute() {
                return pb;
            }
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    return dir.join(pb);
                }
            }
            return pb;
        }
    }
    choose_default()
}

pub fn models_dir() -> PathBuf {
    let current = data_dir().join("models");
    if current.exists() {
        return current;
    }
    if let Some(home) = dirs::home_dir() {
        let legacy = home.join("simulvoice").join("models");
        if legacy.exists() {
            return legacy;
        }
    }
    current
}

pub fn default_config_path() -> PathBuf {
    data_dir().join("simulvoice.toml")
}

pub fn keys_path() -> PathBuf {
    data_dir().join("keys.toml")
}

pub fn ensure_layout() {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let marker = marker_path();
    if !marker.exists() {
        let _ = std::fs::create_dir_all(marker.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&marker, dir.display().to_string());
    }
    let cfg = default_config_path();
    if !cfg.exists() {
        let _ = std::fs::write(&cfg, crate::config::TEMPLATE);
    }
    let keys = keys_path();
    if !keys.exists() {
        let mut candidates = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join("keys.toml"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("keys.toml"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("simulvoice").join("keys.toml"));
        }
        for c in candidates {
            if c.exists() {
                let _ = std::fs::copy(&c, &keys);
                break;
            }
        }
    }
}

pub fn set_data_dir(path: &str) -> anyhow::Result<()> {
    let p = PathBuf::from(path);
    if p.as_os_str().is_empty() {
        anyhow::bail!("数据目录不能为空");
    }
    std::fs::create_dir_all(&p)?;
    let marker = marker_path();
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&marker, p.display().to_string())?;
    Ok(())
}

/// Moves models/keys/config/recordings from `old` to `new` using PowerShell.
pub fn move_data(old: &Path, new: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(new)?;
    let items = ["models", "rec", "keys.toml", "simulvoice.toml", "transcript.txt"];
    for item in items {
        let src = old.join(item);
        if !src.exists() {
            continue;
        }
        let dst = new.join(item);
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Test-Path -LiteralPath '{}') {{ Move-Item -LiteralPath '{}' -Destination '{}' -Force }}",
                    src.display(),
                    src.display(),
                    dst.display()
                ),
            ])
            .status()?;
        if !status.success() {
            eprintln!("[data] 迁移失败: {}", src.display());
        }
    }
    Ok(())
}

