use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::fs as lock_fs;

#[derive(Debug, Clone)]
pub struct LockStore {
    repo_id: String,
    lock_dir: PathBuf,
}

impl LockStore {
    pub fn open() -> Result<Self> {
        let repo_id = repo_id()?;
        let lock_dir = lock_dir_from_env();
        Ok(Self { repo_id, lock_dir })
    }

    #[cfg(test)]
    pub fn new(repo_id: impl Into<String>, lock_dir: PathBuf) -> Self {
        Self {
            repo_id: repo_id.into(),
            lock_dir,
        }
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub fn lock_dir(&self) -> &Path {
        &self.lock_dir
    }

    pub fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.lock_dir)
            .with_context(|| format!("create lock dir {:?}", self.lock_dir))?;
        Ok(())
    }

    pub fn lock_path(&self, label: &str) -> PathBuf {
        self.lock_dir
            .join(format!("{}-{}.lock", self.repo_id, label))
    }

    pub fn latest_path(&self) -> PathBuf {
        self.lock_dir.join(format!("{}-latest", self.repo_id))
    }

    pub fn read_latest_label(&self) -> Result<Option<String>> {
        let path = self.latest_path();
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

    pub fn write_latest_label(&self, label: &str) -> Result<()> {
        fs::write(self.latest_path(), format!("{label}\n"))?;
        Ok(())
    }

    pub fn remove_latest_if_matches(&self, label: &str) -> Result<bool> {
        let path = self.latest_path();
        if !path.exists() {
            return Ok(false);
        }
        let latest_label = fs::read_to_string(&path).unwrap_or_default();
        if latest_label.trim() == label {
            fs::remove_file(&path)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn resolve_label(&self, input: Option<&str>) -> Result<Option<String>> {
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

        self.read_latest_label()
    }

    pub fn read_lock_at_path(&self, path: &Path) -> Result<lock_fs::LockFile> {
        let content =
            fs::read_to_string(path).with_context(|| format!("read {:?}", path.display()))?;
        Ok(lock_fs::parse_lock_file(&content))
    }

    pub fn write_lock_content(&self, label: &str, content: &str) -> Result<()> {
        let path = self.lock_path(label);
        fs::write(path, content)?;
        Ok(())
    }
}

pub fn lock_dir_from_env() -> PathBuf {
    let base = std::env::var("ZSH_CACHE_DIR").unwrap_or_default();
    if base.is_empty() {
        PathBuf::from("/git-locks")
    } else {
        PathBuf::from(base).join("git-locks")
    }
}

fn repo_id() -> Result<String> {
    const SHOW_TOPLEVEL_ARGS: [&str; 2] = ["rev-parse", "--show-toplevel"];
    let output = nils_common::git::run_output(&SHOW_TOPLEVEL_ARGS)
        .with_context(|| format!("git {SHOW_TOPLEVEL_ARGS:?}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {SHOW_TOPLEVEL_ARGS:?} failed: {stderr}");
    }

    let toplevel = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let path = Path::new(&toplevel);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::{lock_dir_from_env, LockStore};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn lock_dir_from_env_prefers_zsh_cache_dir() {
        let lock = GlobalStateLock::new();
        let cache = tempfile::TempDir::new().expect("cache");
        let _guard = EnvGuard::set(&lock, "ZSH_CACHE_DIR", cache.path().to_str().unwrap());

        let dir = lock_dir_from_env();
        assert_eq!(dir, cache.path().join("git-locks"));
    }

    #[test]
    fn lock_dir_from_env_defaults_when_unset() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "ZSH_CACHE_DIR");

        let dir = lock_dir_from_env();
        assert_eq!(dir, PathBuf::from("/git-locks"));
    }

    #[test]
    fn read_latest_label_ignores_empty_file() {
        let cache = tempfile::TempDir::new().expect("cache");
        let lock_dir = cache.path().join("git-locks");
        fs::create_dir_all(&lock_dir).expect("create lock dir");
        let store = LockStore::new("repo", lock_dir.clone());
        fs::write(store.latest_path(), "\n").expect("write latest");

        let latest = store.read_latest_label().expect("read latest");
        assert_eq!(latest, None);
    }

    #[test]
    fn resolve_label_prefers_explicit() {
        let store = LockStore::new("repo", PathBuf::from("/tmp"));
        let label = store.resolve_label(Some("  wip  ")).expect("resolve label");
        assert_eq!(label, Some("wip".to_string()));
    }
}
