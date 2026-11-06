//! 检查更新：查 GitHub Releases 上的最新正式版（草稿、预发布不算）。
//!
//! 只负责"有没有新版、去哪下载"，不自动下载安装：下载链接交给浏览器，
//! 用户自己运行安装包（安装包会提示关闭正在运行的 SimulVoice）。

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;

/// 仓库地址只在 Cargo.toml 的 repository 里写一份
const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// 自动检查的最短间隔
const AUTO_INTERVAL_SECS: u64 = 24 * 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// 发布页（更新说明）
    pub page: String,
    /// 直接下载：安装版用户给安装包，便携版用户给 zip；找不到对应文件时就是发布页
    pub download: String,
}

fn api_url() -> String {
    let path = REPO_URL.trim_end_matches('/').trim_start_matches("https://github.com/");
    format!("https://api.github.com/repos/{path}/releases/latest")
}

/// 查最新版本。Ok(None) = 已是最新（或还没发布过任何版本）
pub fn check() -> Result<Option<Release>> {
    // 开发用：SIMULVOICE_UPDATE_JSON=<文件> 用本地 JSON 代替网络请求
    let body = if let Ok(path) = std::env::var("SIMULVOICE_UPDATE_JSON") {
        std::fs::read_to_string(path)?
    } else {
        let resp = crate::http::agent()
            .get(&api_url())
            .set("User-Agent", &format!("SimulVoice/{CURRENT}"))
            .set("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(10))
            .call();
        match resp {
            Ok(r) => r.into_string()?,
            // 仓库还没有任何正式发布
            Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(ureq::Error::Status(403 | 429, _)) => {
                return Err(anyhow!("GitHub 查询太频繁，稍后再试"))
            }
            Err(ureq::Error::Status(code, _)) => return Err(anyhow!("GitHub 返回 HTTP {code}")),
            Err(ureq::Error::Transport(_)) => return Err(anyhow!("连不上 GitHub，请检查网络")),
        }
    };
    let json: Value = serde_json::from_str(&body)?;
    Ok(parse_release(&json, CURRENT, installed()))
}

/// 从 GitHub API 的 JSON 里挑出比 `current` 新的版本；解析不了的一律当"不是新版"
fn parse_release(json: &Value, current: &str, installed: bool) -> Option<Release> {
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = json["tag_name"].as_str()?;
    let remote = parse_version(tag)?;
    let local = parse_version(current)?;
    if remote <= local {
        return None;
    }
    let page = json["html_url"].as_str().unwrap_or(REPO_URL).to_string();
    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let pick = |want: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?.to_ascii_lowercase();
            want(&name).then(|| a["browser_download_url"].as_str().map(str::to_string)).flatten()
        })
    };
    let download = if installed {
        pick(&|n| n.ends_with(".exe") && n.contains("setup"))
    } else {
        pick(&|n| n.ends_with("-portable.zip") && !n.contains("directml"))
    }
    .unwrap_or_else(|| page.clone());
    Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        page,
        download,
    })
}

/// "v0.45.0" / "0.45" → (0, 45, 0)
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split(['-', '+']).next()?;
    let mut it = s.split('.').map(|p| p.parse::<u32>());
    let major = it.next()?.ok()?;
    let minor = it.next().unwrap_or(Ok(0)).ok()?;
    let patch = it.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// 安装版（exe 旁边有卸载程序）还是便携版
fn installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("unins000.exe").exists()))
        .unwrap_or(false)
}

fn stamp_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("update-check.txt")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 距上次自动检查是否已满一天
pub fn auto_due() -> bool {
    let last = std::fs::read_to_string(stamp_path())
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    now_secs().saturating_sub(last) >= AUTO_INTERVAL_SECS
}

/// 记下这次检查的时间（失败也记，免得每次启动都去连）
pub fn mark_checked() {
    if let Err(e) = std::fs::write(stamp_path(), now_secs().to_string()) {
        eprintln!("[更新] 记录检查时间失败: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tag: &str) -> Value {
        serde_json::json!({
            "tag_name": tag,
            "html_url": "https://github.com/x/y/releases/tag/v0.46.0",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"name": "simulvoice-0.46.0-directml-portable.zip", "browser_download_url": "https://dl/dml.zip"},
                {"name": "simulvoice-0.46.0-portable.zip", "browser_download_url": "https://dl/portable.zip"},
                {"name": "SimulVoiceSetup-0.46.0.exe", "browser_download_url": "https://dl/setup.exe"}
            ]
        })
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.45.0"), Some((0, 45, 0)));
        assert_eq!(parse_version("0.46"), Some((0, 46, 0)));
        assert_eq!(parse_version("1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("latest"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn picks_asset_by_install_kind() {
        let r = parse_release(&sample("v0.46.0"), "0.45.0", true).unwrap();
        assert_eq!(r.version, "0.46.0");
        assert_eq!(r.download, "https://dl/setup.exe");
        let r = parse_release(&sample("v0.46.0"), "0.45.0", false).unwrap();
        assert_eq!(r.download, "https://dl/portable.zip");
    }

    #[test]
    fn same_older_or_odd_is_not_newer() {
        assert!(parse_release(&sample("v0.45.0"), "0.45.0", true).is_none());
        assert!(parse_release(&sample("v0.44.9"), "0.45.0", true).is_none());
        assert!(parse_release(&sample("nightly"), "0.45.0", true).is_none());
        let mut pre = sample("v0.50.0");
        pre["prerelease"] = serde_json::json!(true);
        assert!(parse_release(&pre, "0.45.0", true).is_none());
    }
}
