use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const SECRET_FILE_MODE: u32 = 0o600;

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("failed to open for sha256: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    Ok(out)
}

pub fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir: {}", parent.display()))?;
    }

    let mut attempt = 0u32;
    loop {
        let tmp_path = temp_path(path, attempt);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(mut file) => {
                file.write_all(contents)
                    .with_context(|| format!("failed to write temp file: {}", tmp_path.display()))?;
                file.flush().ok();

                set_permissions(&tmp_path, mode)?;
                drop(file);

                fs::rename(&tmp_path, path)
                    .with_context(|| format!("failed to rename {} -> {}", tmp_path.display(), path.display()))?;
                set_permissions(path, mode)?;
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                attempt += 1;
                if attempt > 10 {
                    return Err(err).context("failed to create unique temp file");
                }
            }
            Err(err) => return Err(err).context("failed to create temp file"),
        }
    }
}

pub fn write_timestamp(path: &Path, iso: Option<&str>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir: {}", parent.display()))?;
    }

    if let Some(raw) = iso {
        let trimmed = raw.split(&['\n', '\r'][..]).next().unwrap_or("");
        if !trimmed.is_empty() {
            fs::write(path, trimmed)
                .with_context(|| format!("failed to write timestamp: {}", path.display()))?;
            return Ok(());
        }
    }

    let _ = fs::remove_file(path);
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<()> {
    let perm = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, perm)
        .with_context(|| format!("failed to set permissions: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn temp_path(path: &Path, attempt: u32) -> PathBuf {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tmp");
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{filename}.tmp-{pid}-{nanos}-{attempt}");
    path.with_file_name(tmp_name)
}
