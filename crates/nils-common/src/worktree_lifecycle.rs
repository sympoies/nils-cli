//! Cross-process checkout lifecycle exclusion, independent of coordination mode.
//!
//! Launch and removal share a canonical checkout key under the same state root.
//! Lock files are persistent: unlinking them would split the exclusion domain.
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum Error {
    #[error("checkout lifecycle is busy")]
    Busy,
    #[error("checkout lifecycle proof is unavailable")]
    Unavailable,
    #[error("checkout lifecycle identity changed")]
    Changed,
    #[error("checkout session state root differs from its persistent binding")]
    StateRootMismatch,
}

/// The physical checkout namespace is independent of session inventory overrides.
/// Keep this resolver shared with the checkout-lease owner.
pub fn state_home() -> Result<PathBuf, Error> {
    let nonempty = |key| std::env::var_os(key).filter(|value| !value.is_empty());
    if let Some(value) = nonempty("AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME") {
        return Ok(PathBuf::from(value));
    }
    if let Some(value) = nonempty("AGENT_RUNTIME_STATE_HOME") {
        return Ok(PathBuf::from(value).join("checkout-leases"));
    }
    if let Some(value) = nonempty("XDG_STATE_HOME") {
        return Ok(PathBuf::from(value).join("agent-runtime-kit/checkout-leases"));
    }
    nonempty("HOME")
        .map(|home| PathBuf::from(home).join(".local/state/agent-runtime-kit/checkout-leases"))
        .ok_or(Error::Unavailable)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    schema_version: String,
    session_state_root: Vec<u8>,
}

/// Find a prior canonical checkout key for a nested cwd. This retains identity
/// while a remover deletes the Git marker before deleting the directory.
/// The returned candidate still requires Guard::acquire and its private proof.
pub fn prior_checkout_root(state: &Path, cwd: &Path) -> Result<Option<PathBuf>, Error> {
    let cwd = fs::canonicalize(cwd).map_err(|_| Error::Unavailable)?;
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
    Ok(None)
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
    binding: Option<(PathBuf, File, Vec<u8>)>,
}

impl Guard {
    /// Immediate, bounded admission. In particular, resume callers may hold a
    /// session-record lock also used by inventory: never wait and invert it.
    pub fn acquire(state: &Path, checkout: &Path) -> Result<Self, Error> {
        Self::acquire_with_lock(state, checkout, libc::LOCK_EX)
    }

    /// Concurrent launches are readers; deletion must exclude every reader.
    pub fn acquire_startup(state: &Path, checkout: &Path) -> Result<Self, Error> {
        Self::acquire_with_lock(state, checkout, libc::LOCK_SH)
    }

    fn acquire_with_lock(
        state: &Path,
        checkout: &Path,
        operation: libc::c_int,
    ) -> Result<Self, Error> {
        if !state.is_absolute() {
            return Err(Error::Unavailable);
        }
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
        if unsafe { libc::flock(lock.as_raw_fd(), operation | libc::LOCK_NB) } != 0 {
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
            binding: None,
        };
        guard.verify()?;
        Ok(guard)
    }

    /// Bind a linked checkout to one canonical inventory root. Publish complete
    /// bytes atomically even while several startup readers hold the fence.
    /// A mismatch retains the checkout; bindings are never silently rebound.
    pub fn bind_session_state(&mut self, state: &Path) -> Result<PathBuf, Error> {
        self.verify()?;
        if !state.is_absolute() {
            return Err(Error::Unavailable);
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(state)
            .map_err(|_| Error::Unavailable)?;
        let state = fs::canonicalize(state).map_err(|_| Error::Unavailable)?;
        private_directory(&state)?;
        let path = self.lock_path.with_extension("binding.json");
        let expected = Binding {
            schema_version: "agent-runtime.worktree-lifecycle-binding.v1".into(),
            session_state_root: state.as_os_str().as_bytes().to_vec(),
        };
        let bytes = serde_json::to_vec(&expected).map_err(|_| Error::Unavailable)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().ok_or(Error::Unavailable)?)
                .map_err(|_| Error::Unavailable)?;
        temporary
            .write_all(&bytes)
            .map_err(|_| Error::Unavailable)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| Error::Unavailable)?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error::Unavailable),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| Error::Unavailable)?;
        let metadata = file.metadata().map_err(|_| Error::Unavailable)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(Error::Unavailable);
        }
        let mut contents = Vec::new();
        (&mut file)
            .take(16_385)
            .read_to_end(&mut contents)
            .map_err(|_| Error::Unavailable)?;
        if contents.len() > 16_384 {
            return Err(Error::Unavailable);
        }
        let binding: Binding = serde_json::from_slice(&contents).map_err(|_| Error::Unavailable)?;
        if binding.schema_version != expected.schema_version {
            return Err(Error::Unavailable);
        }
        if binding.session_state_root != expected.session_state_root {
            return Err(Error::StateRootMismatch);
        }
        self.directories.push((
            state.clone(),
            identity(&fs::metadata(&state).map_err(|_| Error::Unavailable)?),
        ));
        self.binding = Some((path, file, contents));
        self.verify()?;
        Ok(state)
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
        if let Some((path, file, expected)) = &self.binding {
            let metadata = fs::symlink_metadata(path).map_err(|_| Error::Changed)?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o777 != 0o600
                || metadata.nlink() != 1
                || identity(&metadata) != identity(&file.metadata().map_err(|_| Error::Changed)?)
            {
                return Err(Error::Changed);
            }
            let mut reader = file.try_clone().map_err(|_| Error::Changed)?;
            reader.rewind().map_err(|_| Error::Changed)?;
            let mut actual = Vec::new();
            reader
                .take(16_385)
                .read_to_end(&mut actual)
                .map_err(|_| Error::Changed)?;
            if &actual != expected {
                return Err(Error::Changed);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_readers_share_the_fence_but_removal_is_exclusive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let checkout = tmp.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let namespace = tmp.path().join("lease-state");
        let first = Guard::acquire_startup(&namespace, &checkout).unwrap();
        let second = Guard::acquire_startup(&namespace, &checkout).unwrap();
        assert!(matches!(
            Guard::acquire(&namespace, &checkout),
            Err(Error::Busy)
        ));
        drop(first);
        assert!(matches!(
            Guard::acquire(&namespace, &checkout),
            Err(Error::Busy)
        ));
        drop(second);
        let removal = Guard::acquire(&namespace, &checkout).unwrap();
        assert!(matches!(
            Guard::acquire_startup(&namespace, &checkout),
            Err(Error::Busy)
        ));
        drop(removal);
        assert!(Guard::acquire_startup(&namespace, &checkout).is_ok());
    }

    #[test]
    fn session_inventory_binding_persists_and_rejects_root_mismatch_and_replacement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let checkout = tmp.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let namespace = tmp.path().join("lease-state");
        let state = tmp.path().join("sessions");
        let mut first = Guard::acquire_startup(&namespace, &checkout).unwrap();
        let canonical = first.bind_session_state(&state).unwrap();
        let mut second = Guard::acquire_startup(&namespace, &checkout).unwrap();
        pretty_assertions::assert_eq!(second.bind_session_state(&state).unwrap(), canonical);
        assert!(matches!(
            second.bind_session_state(&tmp.path().join("other")),
            Err(Error::StateRootMismatch)
        ));
        drop(first);
        drop(second);
        let mut removal = Guard::acquire(&namespace, &checkout).unwrap();
        removal.bind_session_state(&state).unwrap();
        let path = removal.binding.as_ref().unwrap().0.clone();
        fs::write(&path, b"unknown binding").unwrap();
        assert!(matches!(removal.verify(), Err(Error::Changed)));
        drop(removal);
        let mut guard = Guard::acquire(&namespace, &checkout).unwrap();
        assert!(matches!(
            guard.bind_session_state(&state),
            Err(Error::Unavailable)
        ));
    }

    #[test]
    fn lifecycle_checkout_identity_survives_git_marker_deletion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("checkout");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let state = tmp.path().join("state");
        let physical = fs::canonicalize(&root).unwrap();
        pretty_assertions::assert_eq!(prior_checkout_root(&state, &nested).unwrap(), None);
        let _guard = Guard::acquire(&state, &root).unwrap();
        pretty_assertions::assert_eq!(
            prior_checkout_root(&state, &nested).unwrap(),
            Some(physical.clone())
        );
        fs::remove_dir_all(root.join(".git")).unwrap();
        pretty_assertions::assert_eq!(
            prior_checkout_root(&state, &nested).unwrap(),
            Some(physical)
        );
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
