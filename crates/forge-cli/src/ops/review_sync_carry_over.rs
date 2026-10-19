//! A designated PASS may survive a pure base-sync merge of its reviewed head.
//! Admission re-derives every fact from content-addressed local git objects in
//! an isolated repository; a recorded carry-over is audit evidence, never proof.

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Duration;

/// Ledger evidence that the designated PASS at `reviewed_head` admits `head`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncCarryOver {
    pub review_url: String,
    pub reviewed_head: String,
    pub head: String,
    pub base_commit: String,
    pub merge_tree: String,
}

pub(crate) fn validate(c: &SyncCarryOver) -> bool {
    [&c.reviewed_head, &c.head, &c.base_commit, &c.merge_tree]
        .iter()
        .all(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        && c.reviewed_head != c.head
        && c.review_url.starts_with("https://")
        && c.review_url.len() <= 512
}

#[cfg(test)]
mod tests {
    use super::{SyncCarryOver, validate};

    fn carry(sha: String) -> SyncCarryOver {
        SyncCarryOver {
            review_url: "https://example.invalid/review".to_string(),
            reviewed_head: sha.clone(),
            head: "b".repeat(40),
            base_commit: "c".repeat(40),
            merge_tree: "d".repeat(40),
        }
    }

    #[test]
    fn carry_over_sha_fields_require_exact_lowercase_hex() {
        let lower = "a".repeat(40);
        assert!(validate(&carry(lower.clone())));
        for invalid in [
            "A".repeat(40),
            format!("{}a", lower),
            format!("{} ", lower),
            format!(" {}", lower),
        ] {
            assert!(!validate(&carry(invalid)));
        }
    }
}

/// A verified `merge(reviewed_head, base_commit)` whose tree is `merge_tree`.
#[derive(Debug)]
pub(crate) struct SyncMerge {
    pub base_commit: String,
    pub merge_tree: String,
}

/// A throwaway bare repository that borrows the caller's object store. Only
/// content-addressed objects cross over: the caller's attributes, config, refs
/// and hooks never shape the recomputed merge.
pub(crate) struct Objects {
    dir: tempfile::TempDir,
    store: String,
}

impl Objects {
    pub(crate) fn open() -> Result<Self, &'static str> {
        let caller = output(Command::new("git").args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ]))
        .filter(|out| out.status.success())
        .ok_or("head-unavailable")?;
        let store = String::from_utf8(caller.stdout).map_err(|_| "head-unavailable")?;
        let store = store.trim_end_matches('\n').to_string();
        // The alternates variable is colon-separated; never let a path split.
        if store.is_empty() || store.contains([':', '\n', '"']) {
            return Err("head-unavailable");
        }
        let objects = Self {
            dir: tempfile::tempdir().map_err(|_| "merge-unavailable")?,
            store,
        };
        output(&mut objects.git(&["init", "--bare", "--quiet", "--template="]))
            .filter(|out| out.status.success())
            .ok_or("merge-unavailable")?;
        Ok(objects)
    }

    fn git(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_str().is_some_and(|key| key.starts_with("GIT_")) {
                command.env_remove(key);
            }
        }
        let home = self.dir.path();
        command
            .args(args)
            .current_dir(home)
            .env("GIT_DIR", home.join("repo"))
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.store)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("xdg"));
        command
    }

    /// Prove `head` is exactly the clean merge of `reviewed_head` (first
    /// parent) with one other commit. Errors are stable reason tokens; each
    /// means re-review.
    pub(crate) fn verify_merge(
        &self,
        reviewed_head: &str,
        head: &str,
    ) -> Result<SyncMerge, &'static str> {
        let raw = output(&mut self.git(&["cat-file", "commit", head]))
            .filter(|out| out.status.success())
            .ok_or("head-unavailable")?;
        let raw = String::from_utf8(raw.stdout).map_err(|_| "head-unavailable")?;
        let mut tree = None;
        let mut parents = Vec::new();
        for line in raw.lines().take_while(|line| !line.is_empty()) {
            if let Some(value) = line.strip_prefix("tree ") {
                tree = Some(value);
            } else if let Some(value) = line.strip_prefix("parent ") {
                parents.push(value);
            }
        }
        let (Some(tree), [first, base]) = (tree, parents.as_slice()) else {
            return Err("not-a-sync-merge");
        };
        if *first != reviewed_head
            || base.len() != 40
            || !base.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("not-a-sync-merge");
        }
        let merged = output(&mut self.git(&[
            "merge-tree",
            "--write-tree",
            "--no-messages",
            reviewed_head,
            base,
        ]))
        .ok_or("merge-unavailable")?;
        match merged.status.code() {
            Some(0) => {}
            Some(1) => return Err("merge-conflicted"),
            _ => return Err("merge-unavailable"),
        }
        let merge_tree = String::from_utf8_lossy(&merged.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        if merge_tree != tree {
            return Err("tree-mismatch");
        }
        Ok(SyncMerge {
            base_commit: (*base).to_string(),
            merge_tree,
        })
    }

    /// `commit` must already be history of the target branch tip.
    pub(crate) fn verify_on_target(
        &self,
        commit: &str,
        target_base: &str,
    ) -> Result<(), &'static str> {
        match output(&mut self.git(&["merge-base", "--is-ancestor", commit, target_base]))
            .map(|out| out.status.code())
        {
            Some(Some(0)) => Ok(()),
            Some(Some(1)) => Err("base-not-on-target"),
            _ => Err("base-unavailable"),
        }
    }
}

fn output(command: &mut Command) -> Option<std::process::Output> {
    crate::backend::output_with_limits(command, Some(Duration::from_secs(60)), 1024 * 1024).ok()
}
