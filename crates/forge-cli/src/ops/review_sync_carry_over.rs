//! A designated PASS may survive a pure base-sync merge of its reviewed head.
//! Admission re-derives every fact from content-addressed local git objects;
//! a recorded carry-over is audit evidence, never proof.

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
        .all(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        && c.reviewed_head != c.head
        && c.review_url.starts_with("https://")
        && c.review_url.len() <= 512
}

/// A verified `merge(reviewed_head, base_commit)` whose tree is `merge_tree`.
#[derive(Debug)]
pub(crate) struct SyncMerge {
    pub base_commit: String,
    pub merge_tree: String,
}

/// Prove `head` is exactly the clean merge of `reviewed_head` (first parent)
/// with one other commit. Errors are stable reason tokens; each means re-review.
pub(crate) fn verify_merge(reviewed_head: &str, head: &str) -> Result<SyncMerge, &'static str> {
    let raw = git(&["cat-file", "commit", head])
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
    if *first != reviewed_head {
        return Err("not-a-sync-merge");
    }
    let merged = git(&[
        "merge-tree",
        "--write-tree",
        "--no-messages",
        reviewed_head,
        base,
    ])
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

/// The merged base commit must already be target-branch history.
pub(crate) fn verify_on_target(base_commit: &str, target_base: &str) -> Result<(), &'static str> {
    match git(&["merge-base", "--is-ancestor", base_commit, target_base])
        .map(|out| out.status.code())
    {
        Some(Some(0)) => Ok(()),
        Some(Some(1)) => Err("base-not-on-target"),
        _ => Err("base-unavailable"),
    }
}

/// Replacement refs could rewrite parents or trees locally; read only the
/// objects the SHAs actually name.
fn git(args: &[&str]) -> Option<std::process::Output> {
    let mut command = Command::new("git");
    command.args(args).env("GIT_NO_REPLACE_OBJECTS", "1");
    crate::backend::output_with_limits(&mut command, Some(Duration::from_secs(60)), 1024 * 1024)
        .ok()
}
