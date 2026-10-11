use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub mod bin;
pub mod cmd;
pub mod fixtures;
pub mod fs;
pub mod git;
pub mod help;
pub mod http;
pub mod stubs;
pub mod tempdir;

static GLOBAL_STATE_LOCK: Mutex<()> = Mutex::new(());

pub struct GlobalStateLock {
    _guard: MutexGuard<'static, ()>,
}

impl GlobalStateLock {
    pub fn new() -> Self {
        let guard = match GLOBAL_STATE_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        Self { _guard: guard }
    }
}

impl Default for GlobalStateLock {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EnvGuard {
    key: String,
    original: Option<String>,
}

impl EnvGuard {
    /// Requires holding `GlobalStateLock` to avoid concurrent global mutations.
    pub fn set(lock: &GlobalStateLock, key: &str, value: &str) -> Self {
        let _ = lock;
        let original = env::var(key).ok();
        // SAFETY: tests mutate process environment only while holding GlobalStateLock.
        unsafe { env::set_var(key, value) };
        Self {
            key: key.to_string(),
            original,
        }
    }

    /// Requires holding `GlobalStateLock` to avoid concurrent global mutations.
    pub fn remove(lock: &GlobalStateLock, key: &str) -> Self {
        let _ = lock;
        let original = env::var(key).ok();
        // SAFETY: tests mutate process environment only while holding GlobalStateLock.
        unsafe { env::remove_var(key) };
        Self {
            key: key.to_string(),
            original,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => {
                // SAFETY: tests mutate process environment only while holding GlobalStateLock.
                unsafe { env::set_var(&self.key, value) };
            }
            None => {
                // SAFETY: tests mutate process environment only while holding GlobalStateLock.
                unsafe { env::remove_var(&self.key) };
            }
        }
    }
}

/// Point forge identity and config lookups at empty directories under `root` and
/// clear the forge principal, for an in-process test. Hold the returned guards for
/// the test's duration; they restore the previous values on drop.
pub fn isolate_forge_identity_env(lock: &GlobalStateLock, root: &Path) -> Vec<EnvGuard> {
    vec![
        EnvGuard::set(
            lock,
            "XDG_CONFIG_HOME",
            &root.join("xdg-config").to_string_lossy(),
        ),
        EnvGuard::set(
            lock,
            "XDG_STATE_HOME",
            &root.join("xdg-state").to_string_lossy(),
        ),
        EnvGuard::remove(lock, "FORGE_IDENTITY_PRINCIPAL"),
    ]
}

/// In-process forge identity isolation for one test: holds the global lock, a
/// temp config root, and the env guards. Fields drop in declaration order, so the
/// environment is restored before the lock is released.
pub struct ForgeIdentityIsolation {
    _env: Vec<EnvGuard>,
    _root: tempfile::TempDir,
    _lock: GlobalStateLock,
}

/// Start in-process forge identity isolation with a fresh temp root.
pub fn isolate_forge_identity() -> ForgeIdentityIsolation {
    let lock = GlobalStateLock::new();
    let root = tempfile::TempDir::new().expect("isolated config dir");
    let env = isolate_forge_identity_env(&lock, root.path());
    ForgeIdentityIsolation {
        _env: env,
        _root: root,
        _lock: lock,
    }
}

/// Mark a freshly written script executable and run it once. macOS checks a newly
/// created executable on its first run, which can take far longer than a short
/// timeout budget. The warm-up keeps that cost out of the timed call.
#[cfg(unix)]
pub fn make_executable_and_warm(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).expect("chmod script");
    let _ = std::process::Command::new(path)
        .arg("warm")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    /// Requires holding `GlobalStateLock` to avoid concurrent global mutations.
    pub fn set(lock: &GlobalStateLock, path: &Path) -> io::Result<Self> {
        let _ = lock;
        let original = env::current_dir()?;
        env::set_current_dir(path)?;
        Ok(Self { original })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
    }
}

pub struct StubBinDir {
    dir: tempfile::TempDir,
}

impl StubBinDir {
    pub fn new() -> Self {
        Self {
            dir: tempfile::TempDir::new().expect("tempdir"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn path_str(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    pub fn write_exe(&self, name: &str, content: &str) {
        write_exe(self.path(), name, content);
    }
}

impl Default for StubBinDir {
    fn default() -> Self {
        Self::new()
    }
}

pub fn write_exe(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::write(&path, content).expect("write stub");
    let mut perms = std::fs::metadata(&path).expect("meta").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    std::fs::set_permissions(&path, perms).expect("chmod stub");
}

/// Requires holding `GlobalStateLock` to avoid concurrent global mutations.
pub fn prepend_path(lock: &GlobalStateLock, dir: &Path) -> EnvGuard {
    let _ = lock;
    let mut paths: Vec<PathBuf> =
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
    paths.insert(0, dir.to_path_buf());
    let joined = env::join_paths(paths).expect("join paths");
    let joined = joined.to_string_lossy().to_string();
    EnvGuard::set(lock, "PATH", &joined)
}

#[cfg(test)]
mod tests {
    use super::{GLOBAL_STATE_LOCK, GlobalStateLock};

    #[test]
    fn global_state_lock_recovers_after_poison() {
        let _ = std::panic::catch_unwind(|| {
            let _guard = GLOBAL_STATE_LOCK.lock().expect("lock should be acquired");
            panic!("intentional poison for recovery test");
        });

        let _lock = GlobalStateLock::new();
    }
}
