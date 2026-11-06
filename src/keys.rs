use std::path::PathBuf;

/// Values that are obviously left-over template text rather than a real credential.
/// A placeholder is worse than a missing key: it makes `doctor` report the engine as
/// configured and then fails with an opaque 401 on the first utterance.
fn is_placeholder(v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() {
        return true;
    }
    if t.ends_with("...") || t.ends_with("…") {
        return true;
    }
    matches!(
        t.to_ascii_lowercase().as_str(),
        "your-key" | "your_api_key" | "your-api-key" | "changeme" | "change-me" | "xxx" | "todo"
    )
}

/// Loads local API keys into the environment without logging their values.
pub fn load_env() {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(crate::paths::keys_path());
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("keys.toml"));
        }
    }
    candidates.push(PathBuf::from("keys.toml"));
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("simulvoice").join("keys.toml"));
    }
    for path in candidates {
        if !path.exists() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        let mut set = |env: &str, table: &str, key: &str| {
            if std::env::var(env).is_ok() {
                return;
            }
            if let Some(s) = v.get(table).and_then(|t| t.get(key)).and_then(|x| x.as_str()) {
                if !s.is_empty() {
                    let value = if let Some(enc) = s.strip_prefix("dpapi:") {
                        unprotect(enc).unwrap_or_default()
                    } else {
                        s.to_string()
                    };
                    if is_placeholder(&value) {
                        eprintln!(
                            "[keys] {table}.{key} 是占位符（{value}），按未设置处理；请填真实密钥"
                        );
                        return;
                    }
                    if !value.is_empty() {
                        std::env::set_var(env, value);
                    }
                }
            }
        };
        set("OPENAI_API_KEY", "openai", "api_key");
        set("GEMINI_API_KEY", "gemini", "api_key");
        set("SONIOX_API_KEY", "soniox", "api_key");
        set("PALABRA_API_KEY", "palabra", "api_key");
        set("VOLC_APP_ID", "volcengine", "app_id");
        set("VOLC_ACCESS_TOKEN", "volcengine", "access_token");
        set("VOLC_RESOURCE_ID", "volcengine", "resource_id");
        return;
    }
}


#[cfg(windows)]
fn protect(plain: &str) -> Option<String> {
    use base64::Engine;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    use windows_sys::Win32::Foundation::LocalFree;
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = CryptProtectData(
            &mut input,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut out,
        );
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(out.pbData as *mut core::ffi::c_void);
        Some(format!(
            "dpapi:{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

#[cfg(windows)]
fn unprotect(encoded: &str) -> Option<String> {
    use base64::Engine;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    use windows_sys::Win32::Foundation::LocalFree;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut out,
        );
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let plain = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(out.pbData as *mut core::ffi::c_void);
        String::from_utf8(plain).ok()
    }
}

#[cfg(not(windows))]
fn protect(plain: &str) -> Option<String> {
    Some(plain.to_string())
}

#[cfg(not(windows))]
fn unprotect(encoded: &str) -> Option<String> {
    Some(encoded.to_string())
}

pub fn encrypt_file(path: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let mut value: toml::Value = toml::from_str(&text)?;
    let targets = [
        ("openai", "api_key"),
        ("gemini", "api_key"),
        ("palabra", "api_key"),
        ("volcengine", "app_id"),
        ("volcengine", "access_token"),
        ("volcengine", "resource_id"),
    ];
    let mut changed = 0;
    for (table, key) in targets {
        let current = value
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        if let Some(v) = current {
            if !v.is_empty() && !v.starts_with("dpapi:") {
                if let Some(enc) = protect(&v) {
                    if let Some(t) = value.get_mut(table).and_then(|t| t.as_table_mut()) {
                        t.insert(key.to_string(), toml::Value::String(enc));
                        changed += 1;
                    }
                }
            }
        }
    }
    std::fs::write(format!("{path}.bak"), &text)?;
    std::fs::write(path, toml::to_string_pretty(&value)?)?;
    println!("已加密 {changed} 项（备份: {path}.bak）");
    Ok(())
}

#[cfg(windows)]
pub fn roundtrip_test() -> bool {
    match protect("simulvoice-test") {
        Some(enc) => unprotect(enc.trim_start_matches("dpapi:")).as_deref() == Some("simulvoice-test"),
        None => false,
    }
}

#[cfg(not(windows))]
pub fn roundtrip_test() -> bool {
    true
}





/// Writes one key into the data-dir keys.toml and sets it for this process.
pub fn set_key(table: &str, key: &str, value: &str) -> anyhow::Result<()> {
    let path = crate::paths::keys_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut root: toml::Value = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).unwrap_or_else(|_| toml::Value::Table(Default::default()))
    } else {
        toml::Value::Table(Default::default())
    };
    let table_value = root
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("keys.toml 根节点不是表"))?
        .entry(table)
        .or_insert_with(|| toml::Value::Table(Default::default()));
    table_value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("keys.toml 表结构错误"))?
        .insert(key.to_string(), toml::Value::String(value.to_string()));
    std::fs::write(&path, toml::to_string_pretty(&root)?)?;
    Ok(())
}

