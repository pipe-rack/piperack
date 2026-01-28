//! Update check helpers for Piperack.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const UPDATE_URL: &str = "https://api.github.com/repos/pipe-rack/piperack/releases/latest";
const UPDATE_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const UPDATE_CACHE_FILE: &str = "update.json";
const NO_UPDATE_ENV: &str = "PIPERACK_NO_UPDATE_CHECK";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    checked_at: u64,
    latest: String,
}

pub async fn check_for_update() -> Option<UpdateInfo> {
    if update_check_disabled() {
        return None;
    }

    let current = env!("CARGO_PKG_VERSION").to_string();
    let current_version = version_tuple(&current)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    let cache_path = cache_path();
    let mut cached_latest = None;
    let mut cache_is_fresh = false;
    if let Some(path) = cache_path.as_ref() {
        if let Some(cache) = read_cache(path) {
            cached_latest = Some(cache.latest);
            cache_is_fresh = now.saturating_sub(cache.checked_at) < UPDATE_TTL.as_secs();
        }
    }

    let latest = if cache_is_fresh {
        cached_latest
    } else {
        match fetch_latest_version().await {
            Some(latest) => {
                if let Some(path) = cache_path.as_ref() {
                    write_cache(path, &latest, now);
                }
                Some(latest)
            }
            None => cached_latest,
        }
    }?;

    let latest_version = version_tuple(&latest)?;
    if latest_version > current_version {
        Some(UpdateInfo {
            current: normalize_version(&current)?,
            latest: normalize_version(&latest)?,
        })
    } else {
        None
    }
}

async fn fetch_latest_version() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("piperack/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(UPDATE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let payload: ReleaseResponse = response.json().await.ok()?;
    Some(payload.tag_name)
}

fn update_check_disabled() -> bool {
    env::var(NO_UPDATE_ENV)
        .ok()
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn cache_path() -> Option<PathBuf> {
    cache_dir().map(|dir| dir.join("piperack").join(UPDATE_CACHE_FILE))
}

fn cache_dir() -> Option<PathBuf> {
    if let Ok(path) = env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(path));
    }
    if cfg!(windows) {
        return env::var("LOCALAPPDATA").ok().map(PathBuf::from);
    }
    env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".cache"))
}

fn read_cache(path: &Path) -> Option<UpdateCache> {
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_cache(path: &Path, latest: &str, checked_at: u64) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let cache = UpdateCache {
        checked_at,
        latest: latest.to_string(),
    };
    if let Ok(serialized) = serde_json::to_string(&cache) {
        let _ = fs::write(path, serialized);
    }
}

fn normalize_version(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('v');
    let no_build = trimmed.split('+').next().unwrap_or(trimmed);
    let no_pre = no_build.split('-').next().unwrap_or(no_build);
    if no_pre.is_empty() {
        None
    } else {
        Some(no_pre.to_string())
    }
}

fn version_tuple(raw: &str) -> Option<(u64, u64, u64)> {
    let normalized = normalize_version(raw)?;
    let mut parts = normalized.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{normalize_version, version_tuple};

    #[test]
    fn normalize_version_strips_prefixes() {
        assert_eq!(normalize_version("v1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(normalize_version("1.2.3-beta.1"), Some("1.2.3".to_string()));
        assert_eq!(normalize_version("1.2.3+build"), Some("1.2.3".to_string()));
    }

    #[test]
    fn version_tuple_parses_semver() {
        assert_eq!(version_tuple("0.2.3"), Some((0, 2, 3)));
        assert_eq!(version_tuple("v10.4.1"), Some((10, 4, 1)));
    }
}
