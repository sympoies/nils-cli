//! The destructive removal step and bounded restoration when it does not finish.
//!
//! Every proof and the backup snapshot have passed before this runs, so the
//! only way to lose tracked content is the destructive child itself being cut
//! off. Content the snapshot (or HEAD, when no snapshot was needed) still
//! represents and that is now missing is rewritten, but only into the original
//! verified target and registration, and never over an existing entry or the
//! real index. Anything unproven fails closed with the original failure.
use super::{
    CliError, Fence,
    backup::{Omission, Receipt},
    git, git_error_reason, in_phase, lease, probe, refused,
};
use serde_json::json;
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    process::Command,
};

mod install;

pub(in crate::worktree) struct Plan {
    git_dir: PathBuf,
    git_dir_identity: (u64, u64),
    target_identity: (u64, u64),
    source: String,
    backup_ref: Option<String>,
    omissions: Vec<Omission>,
}

enum Recovery {
    /// Nothing the snapshot represents was missing.
    NoneNeeded,
    Restored(usize),
    /// Restored, but omitted paths are missing and cannot be rewritten.
    Partial(usize),
    /// Recovery could not run or finish; `started` is true when deletion is proven.
    Failed {
        started: bool,
    },
}

fn identity(path: &Path) -> std::io::Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    Ok((metadata.dev(), metadata.ino()))
}

pub(in crate::worktree) fn plan(
    target: &Path,
    fence: &Fence,
    backup: &Receipt,
) -> Result<Plan, CliError> {
    let git_dir = PathBuf::from(
        git(target, &["rev-parse", "--absolute-git-dir"])
            .map_err(|error| in_phase(error, "remove", "none"))?,
    );
    let unproven = |_| {
        in_phase(
            refused("removal-target-changed", "target identity unavailable"),
            "remove",
            "none",
        )
    };
    Ok(Plan {
        git_dir_identity: identity(&git_dir).map_err(unproven)?,
        target_identity: identity(target).map_err(unproven)?,
        git_dir,
        source: backup
            .reference
            .clone()
            .unwrap_or_else(|| fence.removed_head.clone()),
        backup_ref: backup.reference.clone(),
        omissions: backup.omissions.clone(),
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
            "none",
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
    git(repo_root, &["worktree", "prune"]).map_err(|error| in_phase(error, "prune", "removed"))?;
    Ok(())
}

fn incomplete(target: &Path, plan: &Plan, reason: String) -> CliError {
    // The child's effects cannot be fully verified (ignored files, empty
    // directories), so a clean tracked-content check is only "unknown".
    let (message, hint, mutation, state, restored) = match recover(target, plan) {
        Recovery::NoneNeeded => (
            "worktree removal did not complete; tracked content is intact but other effects cannot be verified, and the target is retained",
            "Retry the same command".to_owned(),
            "unknown",
            "none-needed",
            0,
        ),
        Recovery::Restored(count) => (
            "worktree removal did not complete; deletion had started and missing tracked content was restored; the target is retained",
            "Retry the same command".to_owned(),
            "started",
            "restored",
            count,
        ),
        Recovery::Partial(count) => (
            "worktree removal did not complete; deletion had started and omitted backup paths could not be restored; the target is retained",
            manual_route(plan),
            "started",
            "partial",
            count,
        ),
        Recovery::Failed { started } => (
            "worktree removal did not complete and restoration was not possible; tracked content may be missing",
            manual_route(plan),
            if started { "started" } else { "unknown" },
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
            "backup_ref": plan.backup_ref,
            "backup_omissions": plan.omissions,
        }));
    in_phase(error, "remove", mutation)
}

fn manual_route(plan: &Plan) -> String {
    plan.backup_ref.as_ref().map_or_else(
        || {
            format!(
                "Recreate the worktree from {} with git-cli worktree add",
                plan.source
            )
        },
        |backup| {
            format!(
                "Run git-cli worktree restore {backup}, or git checkout -- . inside the target if it remains"
            )
        },
    )
}

fn recover(target: &Path, plan: &Plan) -> Recovery {
    // The administrative directory is deleted last; without it the index and
    // registration no longer exist to restore into.
    if !plan.git_dir.is_dir() {
        return Recovery::Failed { started: true };
    }
    let registered = fs::canonicalize(target).is_ok_and(|real| {
        fs::read(plan.git_dir.join("gitdir"))
            .is_ok_and(|bytes| bytes.trim_ascii_end() == real.join(".git").as_os_str().as_bytes())
    });
    if !registered
        || identity(target).ok() != Some(plan.target_identity)
        || identity(&plan.git_dir).ok() != Some(plan.git_dir_identity)
    {
        return Recovery::Failed { started: false };
    }
    match restore_missing(target, plan) {
        Ok((0, 0)) => Recovery::NoneNeeded,
        Ok((count, 0)) => Recovery::Restored(count),
        Ok((count, _)) => Recovery::Partial(count),
        Err(_) => Recovery::Failed { started: true },
    }
}

fn absent(target: &Path, relative: &[u8]) -> bool {
    matches!(
        fs::symlink_metadata(target.join(OsString::from_vec(relative.to_vec()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Returns (restored entries, omitted entries that are missing and left alone).
fn restore_missing(target: &Path, plan: &Plan) -> anyhow::Result<(usize, usize)> {
    let root = install::open_root(target, plan.target_identity)?;
    let mut restored = 0;
    if let Some(mut pointer) = install::dot_git(&root)? {
        pointer.write_all(format!("gitdir: {}\n", plan.git_dir.display()).as_bytes())?;
        restored += 1;
    }
    let tree = bounded(
        target,
        plan,
        None,
        &["ls-tree", "-r", "-z", "--full-tree", &plan.source],
    )?;
    let omitted: HashSet<&[u8]> = plan
        .omissions
        .iter()
        .map(|omission| omission.path.as_bytes())
        .collect();
    let mut missing = Vec::new();
    for entry in tree.split(|byte| *byte == 0).filter(|e| !e.is_empty()) {
        // "<mode> SP <type> SP <oid> TAB <path>"; gitlinks are never snapshot content.
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        let path = &entry[tab + 1..];
        if !entry.starts_with(b"160000 ") && !omitted.contains(path) && absent(target, path) {
            missing.push(path);
        }
    }
    // Omissions never enter the snapshot (untracked ones are not in the tree),
    // so their absence is counted from the recorded list and never restored.
    let omitted_missing = omitted.iter().filter(|path| absent(target, path)).count();
    if missing.is_empty() {
        return Ok((restored, omitted_missing));
    }
    // Materialize outside the target, then install only still-missing entries
    // create-new and no-follow under the verified root.
    let scratch = tempfile::tempdir()?;
    let work = scratch.path().join("tree");
    fs::create_dir(&work)?;
    let pathspecs = scratch.path().join("pathspecs");
    let mut file = fs::File::create(&pathspecs)?;
    for path in &missing {
        file.write_all(path)?;
        file.write_all(&[0])?;
    }
    file.flush()?;
    let pathspec_arg = format!("--pathspec-from-file={}", pathspecs.display());
    // A scratch index keeps `restore` from rewriting the real index.
    bounded(
        &work,
        plan,
        Some(&scratch.path().join("index")),
        &[
            "restore",
            "--worktree",
            &format!("--source={}", plan.source),
            &pathspec_arg,
            "--pathspec-file-nul",
        ],
    )?;
    for path in missing {
        if install::install(&root, &work, path)? {
            restored += 1;
        }
    }
    Ok((restored, omitted_missing))
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
