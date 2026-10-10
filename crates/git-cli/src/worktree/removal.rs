//! Collision fencing and automatic preservation for managed cleanup.
use super::{CliError, WorktreeLayout, dirty_checkout_adoption as lease, is_managed_worktree};
use nils_common::{coordination_projection, worktree_lifecycle};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, SystemTime},
};

pub(super) mod backup;
#[cfg(target_os = "linux")]
mod procfs;

pub(super) struct Fence {
    _registry: Option<lease::RemovalLeaseGuard>,
    checkout: Option<lease::RemovalLeaseGuard>,
    _lifecycle: Option<worktree_lifecycle::Guard>,
    pub(super) removed_branch: Option<String>,
    pub(super) removed_head: String,
    pub(super) delivered: bool,
    pub(super) delivery_proof: Option<DeliveryProof>,
    pub(super) warnings: Vec<String>,
    pub(super) operations: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct DeliveryProof {
    basis: &'static str,
    default_branch: String,
    default_head: String,
}

fn cached_delivery(target: &Path, head: &str) -> Option<DeliveryProof> {
    let reference = git(
        target,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )
    .ok()?;
    let branch = reference.strip_prefix("refs/remotes/origin/")?;
    let default_head = git(
        target,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
    .ok()?;
    if !probe(
        "git",
        &[
            "--no-replace-objects",
            "merge-base",
            "--is-ancestor",
            head,
            &default_head,
        ],
        target,
    )
    .ok()?
    .status
    .success()
    {
        return None;
    }
    Some(DeliveryProof {
        basis: "cached-origin-default-ancestry",
        default_branch: branch.to_owned(),
        default_head,
    })
}

impl Fence {
    pub(super) fn release_own_lease(&self) -> Result<(), CliError> {
        if let Some(checkout) = &self.checkout {
            checkout.release_own_lease().map_err(|_| {
                refused(
                    "removal-backup-failed",
                    "caller lease release failed; target retained",
                )
            })?;
        }
        Ok(())
    }
}

pub(super) fn refused(code: &'static str, message: &str) -> CliError {
    CliError::data(code, message)
}

pub(super) fn probe_error_reason(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<lease::DirtyCheckoutError>() {
        return error.code().to_owned();
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return format!("local process I/O error: {:?}", error.kind());
    }
    "local process supervision failed".into()
}

pub(super) fn git_error_reason(output: &Output) -> String {
    // Never project arbitrary stderr: Git filters/signers can include paths,
    // identities or credentials. Only recognized causes get a fixed summary.
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let cause = if [
        "identity unknown",
        "empty ident name",
        "unable to auto-detect email",
        "no email was given",
        "no name was given",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern))
    {
        "author or committer identity is missing or invalid"
    } else if stderr.contains("failed to sign") || stderr.contains("signing failed") {
        "commit signing failed"
    } else if stderr.contains("permission denied") || stderr.contains("read-only file system") {
        "repository access denied or read-only"
    } else if stderr.contains("no space left on device") {
        "repository storage is full"
    } else if stderr.contains("cannot lock ref") || stderr.contains("file exists") {
        "repository lock or reference conflict"
    } else {
        "unrecognized Git diagnostic withheld"
    };
    format!("{cause} ({})", output.status)
}

pub(super) fn probe(program: &str, args: &[&str], cwd: &Path) -> Result<Output, CliError> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if program == "git" {
        lease::sanitize_git_environment(&mut command);
    }
    lease::removal_probe(&mut command).map_err(|error| {
        refused(
            "removal-proof-unavailable",
            "bounded local probe could not complete",
        )
        .with_details(json!({"reason": probe_error_reason(&error)}))
    })
}

pub(super) fn git(target: &Path, args: &[&str]) -> Result<String, CliError> {
    let output = probe("git", args, target)?;
    if !output.status.success() {
        return Err(
            refused("removal-proof-unavailable", "local Git operation failed")
                .with_details(json!({"reason": git_error_reason(&output)})),
        );
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .map_err(|_| {
            refused(
                "removal-proof-unavailable",
                "Git returned unreadable output",
            )
        })
}

fn caller() -> Option<String> {
    env::var("AGENT_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn session_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os("AGENT_SESSION_STATE_DIR").filter(|value| !value.is_empty()) {
        return Some(path.into());
    }
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|base| base.join("agent-session"))
}

fn sessions_idle(
    target: &Path,
    sessions: &Value,
    own: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<(), CliError> {
    let Some(rows) = sessions.as_array() else {
        warnings.push("session inventory is malformed".into());
        return Ok(());
    };
    for row in rows {
        if row
            .get("schema_version")
            .is_some_and(|schema| schema != "agent-session.session.v1")
        {
            warnings.push("session record schema is unknown".into());
            continue;
        }
        let id = row["session_id"].as_str().or_else(|| row["id"].as_str());
        if id.is_none() {
            warnings.push("session record identity is unknown".into());
            continue;
        }
        if own.is_some() && id == own {
            continue;
        }
        match row["status"].as_str() {
            Some("stopped") => continue,
            Some("running") => {}
            _ => {
                warnings.push("session status is unknown".into());
                continue;
            }
        }
        let cwd = row["cwd"]
            .as_str()
            .and_then(|cwd| fs::canonicalize(cwd).ok());
        match cwd {
            Some(cwd) if cwd.starts_with(target) => {
                return Err(refused(
                    "removal-session-active",
                    "a foreign running session is bound to the target",
                )
                .with_details(json!({"session_id":id,"cwd":cwd,"evidence":"running-session"})));
            }
            Some(_) => {}
            None => warnings.push("running session cwd is unavailable".into()),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn ancestor_pids() -> Vec<u32> {
    let mut pids = vec![std::process::id()];
    for _ in 0..128 {
        let pid = *pids.last().unwrap();
        if pid <= 1 {
            break;
        }
        let output = Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output();
        let Some(parent) = output
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
        else {
            break;
        };
        if parent == 0 || pids.contains(&parent) {
            break;
        }
        pids.push(parent);
    }
    pids
}

fn processes_idle(target: &Path, warnings: &mut Vec<String>) -> Result<(), CliError> {
    #[cfg(target_os = "linux")]
    {
        procfs::processes_idle(target, Path::new("/proc"), warnings)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let excluded = ancestor_pids();
        let output = match probe(
            "lsof",
            &["-nP", "-Fpcf", "+D", target.to_str().unwrap()],
            target.parent().unwrap(),
        ) {
            Ok(output) => output,
            Err(_) => {
                warnings.push("lsof visibility unavailable".into());
                return Ok(());
            }
        };
        if !output.stderr.is_empty() {
            warnings.push(format!(
                "lsof: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let mut pid = None;
        let mut command = String::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(value) = line.strip_prefix('p') {
                pid = value.parse::<u32>().ok();
                command.clear();
            }
            if let Some(value) = line.strip_prefix('c') {
                command = value.to_owned();
            }
            if line.starts_with('f')
                && let Some(pid) = pid
                && !excluded.contains(&pid)
            {
                return Err(refused(
                    "removal-process-active",
                    "a readable process cwd or file is inside the target",
                )
                .with_details(json!({"pid":pid,"command":command,"backend":"lsof"})));
            }
        }
        if !matches!(output.status.code(), Some(0 | 1)) {
            warnings.push("lsof inventory incomplete".into());
        }
        Ok(())
    }
}

fn git_state(
    target: &Path,
    branch: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, CliError> {
    let directory = PathBuf::from(git(target, &["rev-parse", "--absolute-git-dir"])?);
    let mut locks = vec![directory.join("index.lock"), directory.join("HEAD.lock")];
    let mut dirs = vec![directory.join("refs")];
    while let Some(dir) = dirs.pop() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "lock") {
                    locks.push(path);
                }
            }
        }
    }
    if let Some(branch) = branch {
        let common = git(
            target,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        locks.push(Path::new(&common).join(format!("refs/heads/{branch}.lock")));
    }
    for lock in locks {
        match fs::symlink_metadata(&lock) {
            Ok(metadata) => match metadata.modified() {
                Ok(modified)
                    if SystemTime::now()
                        .duration_since(modified)
                        .unwrap_or_default()
                        < Duration::from_secs(3600) =>
                {
                    return Err(refused("removal-git-busy", "a fresh Git lock is present")
                        .with_details(json!({"lock_path":lock,"evidence":"fresh-git-lock"})));
                }
                Ok(_) => warnings.push(format!("stale Git lock: {}", lock.display())),
                Err(_) => warnings.push(format!("Git lock age unavailable: {}", lock.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => warnings.push(format!("Git lock unreadable: {}", lock.display())),
        }
    }
    Ok([
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "BISECT_LOG",
    ]
    .into_iter()
    .filter(|marker| directory.join(marker).exists())
    .map(str::to_owned)
    .collect())
}

fn registry_idle(
    state: &Path,
    target: &Path,
    own: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<(), CliError> {
    let projection = match coordination_projection::load(state) {
        Ok(Some(projection)) => projection,
        Ok(None) => return Ok(()),
        Err(error) => {
            warnings.push(format!(
                "session ownership projection unavailable: {error:?}"
            ));
            return Ok(());
        }
    };
    let Some(fingerprint) = coordination_projection::worktree_fingerprint(
        projection.fingerprint_epoch,
        &projection.fingerprint_key,
        target,
    ) else {
        warnings.push("session checkout fingerprint unavailable".into());
        return Ok(());
    };
    for claim in &projection.claims {
        if own.is_some_and(|id| id == claim.session_id) {
            continue;
        }
        if !matches!(
            claim.state.as_str(),
            "active" | "released" | "expired" | "stale"
        ) {
            warnings.push("session claim state unknown".into());
            continue;
        }
        if claim.state == "active" && claim.worktrees.contains(&fingerprint) {
            return Err(refused("removal-session-active", "a foreign active claim is bound to the target")
                .with_details(json!({"session_id":claim.session_id,"claim_id":claim.claim_id,"evidence":"active-claim"})));
        }
    }
    for operation in &projection.operations {
        if operation.schema_version != "agent-session.operation-lease.v1"
            || !matches!(
                operation.state.as_str(),
                "active"
                    | "completing"
                    | "reconcile_pending"
                    | "completed"
                    | "failed"
                    | "abandoned"
            )
        {
            warnings.push("session operation schema or state unknown".into());
            continue;
        }
        if matches!(
            operation.state.as_str(),
            "completed" | "failed" | "abandoned"
        ) {
            continue;
        }
        let Some(claim) = projection
            .claims
            .iter()
            .find(|claim| !claim.claim_id.is_empty() && claim.claim_id == operation.claim_id)
        else {
            warnings.push("session operation binding unavailable".into());
            continue;
        };
        if own.is_some_and(|id| id == claim.session_id) {
            continue;
        }
        if claim.worktrees.contains(&fingerprint) {
            return Err(refused("removal-session-active", "a foreign nonterminal operation is bound to the target")
                .with_details(json!({"session_id":claim.session_id,"claim_id":claim.claim_id,"operation_state":operation.state,"evidence":"nonterminal-operation"})));
        }
    }
    Ok(())
}

pub(super) fn fence(target: &Path, layout: &WorktreeLayout) -> Result<Fence, CliError> {
    if !is_managed_worktree(target, layout) {
        return Err(refused(
            "removal-unmanaged",
            "target is outside the managed worktree tree",
        ));
    }
    let target =
        fs::canonicalize(target).map_err(|_| refused("worktree-not-found", "target is missing"))?;
    let identity = fs::metadata(&target)
        .map_err(|_| refused("removal-target-changed", "target identity unavailable"))?;
    let mut warnings = Vec::new();
    let own = caller();
    let key = own.as_ref().map(|id| {
        Sha256::digest(id.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    });
    let mut state = session_root();
    let mut lifecycle = match worktree_lifecycle::state_home()
        .and_then(|namespace| worktree_lifecycle::Guard::acquire(&namespace, &target))
    {
        Ok(guard) => Some(guard),
        Err(worktree_lifecycle::Error::Busy) => {
            return Err(
                refused("removal-lifecycle-busy", "target lifecycle lock is held")
                    .with_details(json!({"target":target,"evidence":"held-lifecycle-lock"})),
            );
        }
        Err(error) => {
            warnings.push(format!("lifecycle inventory unavailable: {error}"));
            None
        }
    };
    if let (Some(guard), Some(root)) = (&mut lifecycle, &state) {
        match guard.bind_session_state(root) {
            Ok(bound) => state = Some(bound),
            Err(error) => warnings.push(format!("session inventory binding unavailable: {error}")),
        }
    }
    let checkout =
        match lease::fence_removal(&target, key.as_deref()) {
            Ok((_guard, Some(holder), _)) => return Err(refused(
                "removal-lease-active",
                "a foreign unexpired checkout lease is held",
            )
            .with_details(
                json!({"session_key":holder,"target":target,"evidence":"unexpired-checkout-lease"}),
            )),
            Ok((guard, None, notes)) => {
                warnings.extend(notes);
                guard
            }
            Err(error) => {
                warnings.push(format!("checkout lease inventory unavailable: {error}"));
                None
            }
        };
    let head = git(&target, &["rev-parse", "--verify", "HEAD"])?;
    let branch = probe(
        "git",
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        &target,
    )?;
    let removed_branch = branch
        .status
        .success()
        .then(|| String::from_utf8_lossy(&branch.stdout).trim().to_owned());
    let operations = git_state(&target, removed_branch.as_deref(), &mut warnings)?;
    if !operations.is_empty() {
        warnings.push(format!(
            "Git operation state preserved: {}",
            operations.join(", ")
        ));
    }
    if removed_branch.is_none() {
        warnings.push("detached HEAD".into());
    }
    let mut registry = None;
    if let Some(state) = &state {
        use std::os::unix::fs::DirBuilderExt;
        let directory = state.join("coordination");
        let lock = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(anyhow::Error::from)
            .and_then(|_| lease::removal_registry_lock(&directory));
        match lock {
            Ok(guard) => registry = Some(guard),
            Err(error) => warnings.push(format!("session registry fencing unavailable: {error}")),
        }
        match probe(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap_or(""),
                "list",
                "--format",
                "json",
            ],
            &layout.repo_root,
        ) {
            Ok(output) => match serde_json::from_slice::<Value>(&output.stdout) {
                Ok(value) if output.status.success() && value["ok"] == true => {
                    sessions_idle(&target, &value["data"], own.as_deref(), &mut warnings)?
                }
                _ => warnings.push("session inventory unavailable or malformed".into()),
            },
            Err(_) => warnings.push("session inventory unavailable".into()),
        }
        registry_idle(state, &target, own.as_deref(), &mut warnings)?;
    } else {
        warnings.push("session state root unavailable".into());
    }
    processes_idle(&target, &mut warnings)?;
    if let Some(guard) = &lifecycle {
        guard
            .verify()
            .map_err(|_| refused("removal-target-changed", "lifecycle identity changed"))?;
    }
    if let Some(guard) = &checkout {
        guard
            .verify_target()
            .map_err(|_| refused("removal-target-changed", "checkout identity changed"))?;
    }
    let after = fs::metadata(&target)
        .map_err(|_| refused("removal-target-changed", "target disappeared"))?;
    if (identity.dev(), identity.ino()) != (after.dev(), after.ino())
        || git(&target, &["rev-parse", "--verify", "HEAD"])? != head
    {
        return Err(refused(
            "removal-target-changed",
            "target identity or HEAD changed",
        ));
    }
    let delivery_proof = cached_delivery(&target, &head);
    let delivered = delivery_proof.is_some();
    if !delivered {
        warnings.push(
            "HEAD is not in the cached origin default branch; no network proof requested".into(),
        );
    }
    warnings.sort();
    warnings.dedup();
    Ok(Fence {
        _registry: registry,
        checkout,
        _lifecycle: lifecycle,
        removed_branch,
        removed_head: head,
        delivered,
        delivery_proof,
        warnings,
        operations,
    })
}

pub(super) fn remove(target: &Path, repo_root: &Path) -> Result<(), CliError> {
    git(
        repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            target
                .to_str()
                .ok_or_else(|| refused("removal-proof-unavailable", "target path unreadable"))?,
        ],
    )?;
    git(repo_root, &["worktree", "prune"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn git_failure_details_keep_cause_without_projecting_stderr() {
        for (stderr, expected) in [
            (
                "Author identity unknown\nfatal: empty ident name (for <fixture@example.invalid>) not allowed",
                "author or committer identity is missing or invalid",
            ),
            (
                "gpg failed to sign the data: fixture-signing-key",
                "commit signing failed",
            ),
            (
                "fatal: /fixture/repository: Permission denied",
                "repository access denied or read-only",
            ),
            (
                "filter diagnostic token=fixture-secret-value",
                "unrecognized Git diagnostic withheld",
            ),
        ] {
            let output = Output {
                status: std::process::ExitStatus::from_raw(128 << 8),
                stdout: Vec::new(),
                stderr: stderr.as_bytes().to_vec(),
            };
            let reason = git_error_reason(&output);
            assert_eq!(reason, format!("{expected} ({})", output.status));
            assert!(reason.len() <= 256);
            assert!(!reason.contains("fixture"));
        }
    }

    #[test]
    fn probe_failure_details_keep_io_kind_without_projecting_context() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "/fixture/repository token=fixture-secret-value",
        ))
        .context("fixture process could not start");
        assert_eq!(
            probe_error_reason(&error),
            "local process I/O error: PermissionDenied"
        );
    }
}
