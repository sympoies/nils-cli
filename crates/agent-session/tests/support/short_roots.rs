//! Cleanup for short roots owned by a disposable agent-home fixture.
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub fn remove_owned_short_roots(fixture_root: &Path, agent_home: &Path) -> io::Result<()> {
    let boundary = fs::canonicalize(fixture_root)?;
    match fs::symlink_metadata(agent_home) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture home is not a directory",
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    }
    let home = match fs::canonicalize(agent_home) {
        Ok(path) => path,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if home == boundary || !home.starts_with(&boundary) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture home is outside its root",
        ));
    }
    let entries = match fs::read_dir(home.join("tmp")) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let dir = entry?.path();
        let metadata = fs::symlink_metadata(&dir)?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            continue;
        }
        let marker = dir.join(".agent-session-short");
        let marker_meta = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if !marker_meta.is_file()
            || marker_meta.uid() != unsafe { libc::geteuid() }
            || marker_meta.mode() & 0o777 != 0o600
        {
            continue;
        }
        let token = match fs::read_to_string(marker) {
            Ok(token) => token,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if token.len() != 32
            || !token
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            continue;
        }
        let root = Path::new("/tmp").join(format!("ark-{token}"));
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            continue;
        }
        let link = root.join("t");
        let link_meta = match fs::symlink_metadata(&link) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if !link_meta.file_type().is_symlink() || link_meta.uid() != unsafe { libc::geteuid() } {
            continue;
        }
        let target = fs::canonicalize(&link)?;
        let backing = fs::canonicalize(&dir)?;
        if !target.starts_with(&home) || target != backing {
            continue;
        }
        fs::remove_file(link)?;
        fs::remove_dir(root)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub struct ShortRootsGuard(PathBuf, PathBuf);

#[allow(dead_code)]
impl ShortRootsGuard {
    pub fn new(fixture_root: PathBuf, agent_home: PathBuf) -> Self {
        Self(fixture_root, agent_home)
    }
}

impl Drop for ShortRootsGuard {
    fn drop(&mut self) {
        if let Err(err) = remove_owned_short_roots(&self.0, &self.1) {
            if std::thread::panicking() {
                eprintln!("fixture short-root cleanup failed: {err}");
            } else {
                panic!("fixture short-root cleanup failed: {err}");
            }
        }
    }
}
