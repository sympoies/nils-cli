//! Cross-process checkout lifecycle exclusion, independent of coordination mode.
//!
//! Launch and removal share a canonical checkout key under the same state root.
//! Lock files are persistent: unlinking them would split the exclusion domain.
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum Error {
    #[error("checkout lifecycle is busy")]
    Busy,
    #[error("checkout lifecycle proof is unavailable")]
    Unavailable,
    #[error("checkout lifecycle identity changed")]
    Changed,
}

/// Resolve nested cwd to its physical Git checkout, without inherited Git
/// retargeting. Ordinary non-repository directories have no checkout fence.
pub fn checkout_root(state: &Path, cwd: &Path) -> Result<Option<PathBuf>, Error> {
    let cwd = fs::canonicalize(cwd).map_err(|_| Error::Unavailable)?;
    if !cwd.ancestors().any(|path| path.join(".git").exists()) {
        // Git may already have removed .git while deleting a checkout. Its
        // remover created the persistent root lock first. Preserve that key
        // instead of allowing startup to misclassify the directory as non-Git.
        for ancestor in cwd.ancestors() {
            let lock = state
                .join("coordination/worktree-lifecycle")
                .join(format!("{}.lock", checkout_key(ancestor)));
            match fs::symlink_metadata(lock) {
                Ok(_) => return Ok(Some(ancestor.to_path_buf())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Unavailable),
            }
        }
        return Ok(None);
    }
    // This identity probe cannot use a PATH shim that reports a different
    // checkout key from the one removal uses.
    let mut command = Command::new("/usr/bin/git");
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    let mut child = command.spawn().map_err(|_| Error::Unavailable)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Unavailable);
            }
        }
    };
    if !status.success() {
        return Err(Error::Unavailable);
    }
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or(Error::Unavailable)?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    if bytes.len() > 65_536 || bytes.pop() != Some(b'\n') || bytes.is_empty() {
        return Err(Error::Unavailable);
    }
    let root = fs::canonicalize(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        .map_err(|_| Error::Unavailable)?;
    if !cwd.starts_with(&root) {
        return Err(Error::Unavailable);
    }
    Ok(Some(root))
}

fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn checkout_key(checkout: &Path) -> String {
    Sha256::digest(checkout.as_os_str().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn private_directory(path: &Path) -> Result<(), Error> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Error::Unavailable),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::Unavailable)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}

#[derive(Debug)]
pub struct Guard {
    lock: File,
    lock_path: PathBuf,
    root: PathBuf,
    root_identity: (u64, u64),
    directories: Vec<(PathBuf, (u64, u64))>,
}

impl Guard {
    /// Immediate, bounded admission. In particular, resume callers may hold a
    /// session-record lock also used by inventory: never wait and invert it.
    pub fn acquire(state: &Path, checkout: &Path) -> Result<Self, Error> {
        let root = fs::canonicalize(checkout).map_err(|_| Error::Unavailable)?;
        let root_identity = identity(&fs::metadata(&root).map_err(|_| Error::Unavailable)?);
        // State-root creation has the same private mode as the session owner.
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(state)
            .map_err(|_| Error::Unavailable)?;
        private_directory(state)?;
        let mut directories = vec![(
            state.to_path_buf(),
            identity(&fs::symlink_metadata(state).map_err(|_| Error::Unavailable)?),
        )];
        let directory = state.join("coordination");
        private_directory(&directory)?;
        directories.push((
            directory.clone(),
            identity(&fs::symlink_metadata(&directory).map_err(|_| Error::Unavailable)?),
        ));
        let directory = directory.join("worktree-lifecycle");
        private_directory(&directory)?;
        directories.push((
            directory.clone(),
            identity(&fs::symlink_metadata(&directory).map_err(|_| Error::Unavailable)?),
        ));
        let key = checkout_key(&root);
        let lock_path = directory.join(format!("{key}.lock"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|_| Error::Unavailable)?;
        let metadata = lock.metadata().map_err(|_| Error::Unavailable)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(Error::Unavailable);
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(
                if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                    Error::Busy
                } else {
                    Error::Unavailable
                },
            );
        }
        let guard = Self {
            lock,
            lock_path,
            root,
            root_identity,
            directories,
        };
        guard.verify()?;
        Ok(guard)
    }

    pub fn verify(&self) -> Result<(), Error> {
        for (directory, expected) in &self.directories {
            let metadata = fs::symlink_metadata(directory).map_err(|_| Error::Changed)?;
            if !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || identity(&metadata) != *expected
            {
                return Err(Error::Changed);
            }
        }
        let root = fs::symlink_metadata(&self.root).map_err(|_| Error::Changed)?;
        let path = fs::symlink_metadata(&self.lock_path).map_err(|_| Error::Changed)?;
        let lock = self.lock.metadata().map_err(|_| Error::Changed)?;
        if !root.is_dir()
            || identity(&root) != self.root_identity
            || !path.is_file()
            || path.nlink() != 1
            || path.uid() != unsafe { libc::geteuid() }
            || path.mode() & 0o777 != 0o600
            || identity(&path) != identity(&lock)
        {
            return Err(Error::Changed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_checkout_identity_survives_git_marker_deletion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("checkout");
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("/usr/bin/git")
                .current_dir(&root)
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let state = tmp.path().join("state");
        let physical = fs::canonicalize(&root).unwrap();
        pretty_assertions::assert_eq!(
            checkout_root(&state, &nested).unwrap(),
            Some(physical.clone())
        );
        let _guard = Guard::acquire(&state, &root).unwrap();
        fs::remove_dir_all(root.join(".git")).unwrap();
        pretty_assertions::assert_eq!(checkout_root(&state, &nested).unwrap(), Some(physical));
    }

    #[test]
    fn lifecycle_excludes_both_directions_and_retains_lock_inode() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("checkout");
        fs::create_dir(&root).unwrap();
        let state = tmp.path().join("state");
        let guard = Guard::acquire(&state, &root).unwrap();
        let inode = identity(&guard.lock.metadata().unwrap());
        assert!(matches!(Guard::acquire(&state, &root), Err(Error::Busy)));
        drop(guard);
        let guard = Guard::acquire(&state, &root).unwrap();
        pretty_assertions::assert_eq!(identity(&guard.lock.metadata().unwrap()), inode);
        fs::remove_dir(&root).unwrap();
        assert!(matches!(guard.verify(), Err(Error::Changed)));
    }

    #[test]
    fn lifecycle_rejects_symlinked_directory_and_lock_replacement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("checkout");
        fs::create_dir(&root).unwrap();
        let state = tmp.path().join("state");
        let guard = Guard::acquire(&state, &root).unwrap();
        fs::remove_file(&guard.lock_path).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("absent"), &guard.lock_path).unwrap();
        assert!(matches!(guard.verify(), Err(Error::Changed)));
        assert!(matches!(
            Guard::acquire(&state, &root),
            Err(Error::Unavailable)
        ));
        let other = tmp.path().join("other-state");
        std::os::unix::fs::symlink(&state, &other).unwrap();
        assert!(matches!(
            Guard::acquire(&other, &root),
            Err(Error::Unavailable)
        ));
    }
}
