//! Build class for managed worktrees.
//!
//! Compiler output for a managed worktree lives at
//! `AGENT_HOME/build/<repo-key>/<slug>`: outside the checkout, outside the
//! evidence tree, and reclaimed together with the worktree. `repo-key` mirrors
//! the `worktrees/` layout so equal slugs in different repositories never share
//! one build directory. `git-cli` cannot export into the caller's shell, so it
//! records the path and an evaluable `CARGO_TARGET_DIR` export for wrappers.

use super::{WorktreeLayout, display_path, is_managed_worktree};
use nils_common::shell::quote_posix_single;
use std::fs;
use std::path::{Path, PathBuf};

const BUILD_ROOT: &str = "build";

/// The build directory for the managed worktree `slug`.
pub(super) fn path(layout: &WorktreeLayout, slug: &str) -> PathBuf {
    layout
        .agent_home
        .join(BUILD_ROOT)
        .join(&layout.repo_key)
        .join(slug)
}

/// The build directory for an existing worktree path, or `None` when the path
/// is not a managed worktree and therefore has no build class.
pub(super) fn path_for_worktree(worktree: &Path, layout: &WorktreeLayout) -> Option<PathBuf> {
    if !is_managed_worktree(worktree, layout) {
        return None;
    }
    let slug = worktree.file_name()?.to_str()?;
    Some(path(layout, slug))
}

/// An evaluable export that points cargo at `dir`.
pub(super) fn export_command(dir: &Path) -> String {
    format!(
        "export CARGO_TARGET_DIR={}",
        quote_posix_single(&display_path(dir))
    )
}

/// Remove `dir` if present. A symlink is unlinked, never followed. A failure is
/// returned as a warning: the worktree itself is already gone by then.
pub(super) fn reclaim(dir: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to inspect {}: {err}", display_path(dir))),
    };
    let removed = if metadata.is_dir() {
        fs::remove_dir_all(dir)
    } else {
        fs::remove_file(dir)
    };
    removed
        .map(|()| true)
        .map_err(|err| format!("failed to remove build dir {}: {err}", display_path(dir)))
}
