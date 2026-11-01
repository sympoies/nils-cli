use super::{git, options::Options};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) struct State {
    pub root: PathBuf,
    pub branch: String,
    pub upstream: Option<String>,
}

fn file(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "rebase_state_unavailable")?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("rebase_state_unsupported".into());
    }
    fs::read_to_string(path).map_err(|_| "rebase_state_unreadable".into())
}

pub(super) fn preflight(options: &Options) -> Result<State, String> {
    if [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
    {
        return Err("identity_git_override".into());
    }
    let start = options
        .repo
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .map_err(|_| "rebase_repository_unavailable")?;
    let root = fs::canonicalize(git::read(&start, &["rev-parse", "--show-toplevel"])?)
        .map_err(|_| "rebase_repository_unavailable")?;
    let admin = PathBuf::from(git::read(&root, &["rev-parse", "--absolute-git-dir"])?);
    let common = PathBuf::from(git::read(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    let agent_home = std::env::var_os("AGENT_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME").map(|p| PathBuf::from(p).join("agent-runtime-kit"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|p| PathBuf::from(p).join(".local/state/agent-runtime-kit"))
        })
        .ok_or("rebase_managed_worktree_required")?;
    let managed = fs::canonicalize(agent_home.join("worktrees"))
        .map_err(|_| "rebase_managed_worktree_required")?;
    if admin == common
        || !root.starts_with(&managed)
        || root
            .strip_prefix(&managed)
            .map_err(|_| "rebase_managed_worktree_required")?
            .components()
            .count()
            != 2
    {
        return Err("rebase_managed_worktree_required".into());
    }
    let state = admin.join("rebase-merge");
    let branch = if options.upstream.is_some() {
        if [
            "rebase-merge",
            "rebase-apply",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ]
        .iter()
        .any(|path| admin.join(path).exists())
        {
            return Err("rebase_operation_in_progress_use_continue".into());
        }
        if !git::read(&root, &["status", "--porcelain=v1"])?.is_empty() {
            return Err("rebase_clean_worktree_required".into());
        }
        git::read(&root, &["symbolic-ref", "--quiet", "HEAD"])?
    } else {
        if !state.is_dir() || admin.join("rebase-apply").exists() {
            return Err("rebase_merge_state_required".into());
        }
        // The identity owner validates the saved replay program and rechecks
        // this branch against orig-head before and after authentication.
        file(&state.join("head-name"))?.trim().to_owned()
    };
    let branch = branch
        .strip_prefix("refs/heads/")
        .filter(|branch| !branch.is_empty())
        .ok_or("rebase_attached_branch_required")?
        .to_owned();
    git::read(&root, &["check-ref-format", "--branch", &branch])?;
    // The dedicated identity owner checks the selected original-branch remote's
    // cached default before authorization. Do not guess a remote while detached.
    let upstream = options
        .upstream
        .as_ref()
        .map(|reference| {
            git::read(
                &root,
                &[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{reference}^{{commit}}"),
                ],
            )
        })
        .transpose()?;
    if let Some(upstream) = &upstream
        && !git::read(
            &root,
            &["rev-list", "--merges", &format!("{upstream}..HEAD")],
        )?
        .is_empty()
    {
        return Err("rebase_linear_history_required".into());
    }
    Ok(State {
        root,
        branch,
        upstream,
    })
}
