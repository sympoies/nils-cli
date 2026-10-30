//! The destructive removal step and bounded restoration when it does not finish.
//!
//! Every proof and the backup snapshot have passed before this runs, so the
//! only way to lose tracked content is the destructive child itself being cut
//! off. Content that is missing afterwards is rewritten from the snapshot (or
//! from HEAD when no snapshot was needed) without touching existing files or
//! the real index.
use super::{
    CliError, Fence, backup::Receipt, git, git_error_reason, in_phase, lease, probe, refused,
};
use serde_json::json;
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
};

pub(in crate::worktree) struct Plan {
    git_dir: PathBuf,
    source: String,
    backup_ref: Option<String>,
}

enum Recovery {
    /// Nothing tracked was missing.
    NoneNeeded,
    Restored(usize),
    /// Recovery could not run or finish; content may be missing.
    Failed,
}

pub(in crate::worktree) fn plan(
    target: &Path,
    fence: &Fence,
    backup: &Receipt,
) -> Result<Plan, CliError> {
    let git_dir = git(target, &["rev-parse", "--absolute-git-dir"])
        .map_err(|error| in_phase(error, "remove", false))?;
    Ok(Plan {
        git_dir: git_dir.into(),
        source: backup
            .reference
            .clone()
            .unwrap_or_else(|| fence.removed_head.clone()),
        backup_ref: backup.reference.clone(),
    })
}

pub(in crate::worktree) fn remove(
    target: &Path,
    repo_root: &Path,
    plan: &Plan,
) -> Result<(), CliError> {
    let path = target.to_str().ok_or_else(|| {
        in_phase(
            refused("removal-proof-unavailable", "target path unreadable"),
            "remove",
            false,
        )
    })?;
    match probe("git", &["worktree", "remove", "--force", path], repo_root) {
        Ok(output) if output.status.success() => {}
        Ok(output) => return Err(incomplete(target, plan, git_error_reason(&output))),
        Err(error) => {
            let reason = error
                .details
                .as_deref()
                .and_then(|details| details["reason"].as_str())
                .unwrap_or("bounded local probe could not complete")
                .to_owned();
            return Err(incomplete(target, plan, reason));
        }
    }
    git(repo_root, &["worktree", "prune"]).map_err(|error| in_phase(error, "prune", true))?;
    Ok(())
}

fn incomplete(target: &Path, plan: &Plan, reason: String) -> CliError {
    let recovery = recover(target, plan);
    let (message, hint, mutated, state, restored) = match recovery {
        Recovery::NoneNeeded => (
            "worktree removal did not complete; no tracked content was removed and the target is retained",
            "Retry the same command".to_owned(),
            false,
            "none-needed",
            0,
        ),
        Recovery::Restored(count) => (
            "worktree removal did not complete; missing tracked content was restored and the target is retained",
            "Retry the same command".to_owned(),
            true,
            "restored",
            count,
        ),
        Recovery::Failed => (
            "worktree removal did not complete and tracked content may be missing; restoration failed",
            plan.backup_ref.as_ref().map_or_else(
                || {
                    format!(
                        "Recreate the worktree from {} with git-cli worktree add, or run git checkout -- . inside the target if it still exists",
                        plan.source
                    )
                },
                |backup| format!("Run git-cli worktree restore {backup} or git checkout -- . inside the target"),
            ),
            true,
            "failed",
            0,
        ),
    };
    let error = refused("removal-remove-incomplete", message)
        .with_hint(hint)
        .with_details(json!({
            "reason": reason,
            "recovery": state,
            "restored_paths": restored,
        }));
    in_phase(error, "remove", mutated)
}

fn recover(target: &Path, plan: &Plan) -> Recovery {
    // The administrative directory is deleted last; without it neither the
    // index nor registration remain to restore into.
    if !plan.git_dir.is_dir() {
        return Recovery::Failed;
    }
    match restore_missing(target, plan) {
        Ok(0) => Recovery::NoneNeeded,
        Ok(count) => Recovery::Restored(count),
        Err(_) => Recovery::Failed,
    }
}

fn restore_missing(target: &Path, plan: &Plan) -> anyhow::Result<usize> {
    let mut restored = 0;
    let pointer = target.join(".git");
    if fs::symlink_metadata(&pointer).is_err() {
        fs::write(&pointer, format!("gitdir: {}\n", plan.git_dir.display()))?;
        restored += 1;
    }
    let tree = bounded(
        target,
        plan,
        None,
        &["ls-tree", "-r", "-z", "--full-tree", &plan.source],
    )?;
    let mut missing = Vec::new();
    for entry in tree.split(|byte| *byte == 0).filter(|e| !e.is_empty()) {
        // "<mode> SP <type> SP <oid> TAB <path>"; gitlinks are never snapshot content.
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        if entry.starts_with(b"160000 ") {
            continue;
        }
        let path = &entry[tab + 1..];
        let missing_path =
            match fs::symlink_metadata(target.join(OsString::from_vec(path.to_vec()))) {
                Ok(_) => false,
                Err(error) => error.kind() == std::io::ErrorKind::NotFound,
            };
        if missing_path {
            missing.push(path);
        }
    }
    if missing.is_empty() {
        return Ok(restored);
    }
    let temporary = tempfile::tempdir()?;
    let pathspecs = temporary.path().join("pathspecs");
    let mut file = fs::File::create(&pathspecs)?;
    for path in &missing {
        file.write_all(OsString::from_vec(path.to_vec()).as_bytes())?;
        file.write_all(&[0])?;
    }
    file.flush()?;
    let pathspec_arg = format!("--pathspec-from-file={}", pathspecs.display());
    // A scratch index keeps `restore` from rewriting the real index.
    bounded(
        target,
        plan,
        Some(&temporary.path().join("index")),
        &[
            "restore",
            "--worktree",
            &format!("--source={}", plan.source),
            &pathspec_arg,
            "--pathspec-file-nul",
        ],
    )?;
    Ok(restored + missing.len())
}

fn bounded(
    target: &Path,
    plan: &Plan,
    index: Option<&Path>,
    args: &[&str],
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .arg(format!("--git-dir={}", plan.git_dir.display()))
        .arg(format!("--work-tree={}", target.display()))
        .args(args)
        .current_dir(&plan.git_dir);
    lease::sanitize_git_environment(&mut command);
    command.env("GIT_LITERAL_PATHSPECS", "1");
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let output = lease::removal_probe(&mut command)?;
    anyhow::ensure!(
        output.status.success(),
        "recovery Git operation failed: {}",
        git_error_reason(&output)
    );
    Ok(output.stdout)
}
