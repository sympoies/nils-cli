use anyhow::{Context, Result};
use chrono::{Local, NaiveDateTime, TimeZone};
use std::fs;
use std::path::{Path, PathBuf};

use crate::git;

pub const TIMESTAMP_PREFIX: &str = "timestamp=";

#[derive(Debug, Clone)]
pub struct LockFile {
    pub hash: String,
    pub note: String,
    pub timestamp: Option<String>,
}

pub fn repo_id() -> Result<String> {
    let toplevel = git::run_capture(&["rev-parse", "--show-toplevel"])?
        .trim()
        .to_string();
    let path = Path::new(&toplevel);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    Ok(name)
}

pub fn lock_dir_path() -> PathBuf {
    let base = std::env::var("ZSH_CACHE_DIR").unwrap_or_default();
    if base.is_empty() {
        PathBuf::from("/git-locks")
    } else {
        PathBuf::from(base).join("git-locks")
    }
}

pub fn ensure_lock_dir() -> Result<PathBuf> {
    let dir = lock_dir_path();
    fs::create_dir_all(&dir).with_context(|| format!("create lock dir {dir:?}"))?;
    Ok(dir)
}

pub fn lock_file(repo_id: &str, label: &str) -> PathBuf {
    lock_dir_path().join(format!("{repo_id}-{label}.lock"))
}

pub fn latest_file(repo_id: &str) -> PathBuf {
    lock_dir_path().join(format!("{repo_id}-latest"))
}

pub fn read_latest_label(repo_id: &str) -> Result<Option<String>> {
    let path = latest_file(repo_id);
    if !path.exists() {
        return Ok(None);
    }
    let label = fs::read_to_string(&path).unwrap_or_default();
    let label = label.trim().to_string();
    if label.is_empty() {
        return Ok(None);
    }
    Ok(Some(label))
}

pub fn resolve_label(repo_id: &str, input: Option<&str>) -> Result<Option<String>> {
    if let Some(label) = input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }) {
        return Ok(Some(label));
    }

    read_latest_label(repo_id)
}

pub fn read_lock_file(path: &Path) -> Result<LockFile> {
    let content = fs::read_to_string(path).with_context(|| format!("read {path:?}"))?;
    let mut lines = content.lines();
    let line1 = lines.next().unwrap_or("");
    let (hash, note) = parse_lock_line(line1);

    let mut timestamp = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(TIMESTAMP_PREFIX) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                timestamp = Some(trimmed.to_string());
            }
            break;
        }
    }

    Ok(LockFile {
        hash,
        note,
        timestamp,
    })
}

pub fn parse_lock_line(line: &str) -> (String, String) {
    let mut parts = line.splitn(2, '#');
    let hash = parts.next().unwrap_or("").trim().to_string();
    let note = parts.next().unwrap_or("").trim().to_string();
    (hash, note)
}

pub fn timestamp_epoch(timestamp: &str) -> i64 {
    if timestamp.trim().is_empty() {
        return 0;
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(timestamp.trim(), "%Y-%m-%d %H:%M:%S") {
        if let Some(local) = Local.from_local_datetime(&parsed).single() {
            return local.timestamp();
        }
    }

    0
}
