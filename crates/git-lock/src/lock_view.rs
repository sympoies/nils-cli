use anyhow::Result;
use std::path::Path;

use crate::fs as lock_fs;
use crate::git::GitBackend;
use crate::store::LockStore;

#[derive(Debug, Clone)]
pub struct LockDetails {
    pub label: String,
    pub lock: lock_fs::LockFile,
    pub subject: Option<String>,
    pub epoch: i64,
}

impl LockDetails {
    pub fn load_from_path(
        store: &LockStore,
        label: &str,
        path: &Path,
        git: &dyn GitBackend,
    ) -> Result<Self> {
        let lock = store.read_lock_at_path(path)?;
        let subject = git
            .log_subject(&lock.hash)?
            .filter(|value| !value.is_empty());
        let epoch = lock
            .timestamp
            .as_deref()
            .map(lock_fs::timestamp_epoch)
            .unwrap_or(0);

        Ok(Self {
            label: label.to_string(),
            lock,
            subject,
            epoch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::LockDetails;
    use crate::git::GitBackend;
    use crate::store::LockStore;
    use pretty_assertions::assert_eq;
    use std::cell::Cell;
    use std::fs;

    #[derive(Default)]
    struct StubGit {
        calls: Cell<u32>,
    }

    impl GitBackend for StubGit {
        fn log_subject(&self, _hash: &str) -> anyhow::Result<Option<String>> {
            self.calls.set(self.calls.get() + 1);
            Ok(Some("subject".to_string()))
        }
    }

    #[test]
    fn lock_details_uses_git_backend_once() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let lock_dir = temp.path().join("git-locks");
        fs::create_dir_all(&lock_dir).expect("create lock dir");
        let store = LockStore::new("repo", lock_dir.clone());

        let path = store.lock_path("sample");
        fs::write(&path, "abc123 # note\n").expect("write lock");

        let git = StubGit::default();
        let details =
            LockDetails::load_from_path(&store, "sample", &path, &git).expect("load details");

        assert_eq!(details.subject.as_deref(), Some("subject"));
        assert_eq!(git.calls.get(), 1);
        assert_eq!(details.lock.hash, "abc123");
    }

    #[test]
    fn lock_details_filters_empty_subject() {
        struct EmptyGit;
        impl GitBackend for EmptyGit {
            fn log_subject(&self, _hash: &str) -> anyhow::Result<Option<String>> {
                Ok(Some(String::new()))
            }
        }

        let temp = tempfile::TempDir::new().expect("tempdir");
        let lock_dir = temp.path().join("git-locks");
        fs::create_dir_all(&lock_dir).expect("create lock dir");
        let store = LockStore::new("repo", lock_dir.clone());
        let path = store.lock_path("empty");
        fs::write(&path, "abc123\n").expect("write lock");

        let details = LockDetails::load_from_path(&store, "empty", &path, &EmptyGit).expect("load");
        assert_eq!(details.subject, None);
    }
}
