//! Fail-closed managed cleanup. Locks survive until the lifecycle call returns.
use super::{CliError, WorktreeLayout, dirty_checkout_adoption as lease, is_managed_worktree};
use nils_common::coordination_projection;
use nils_common::worktree_lifecycle;
use serde::Serialize;
use serde_json::Value;
use std::os::unix::fs::MetadataExt;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub(super) struct Fence {
    _registry: lease::RemovalLeaseGuard,
    _checkout: lease::RemovalLeaseGuard,
    _lifecycle: worktree_lifecycle::Guard,
    pub(super) removed_branch: Option<String>,
    pub(super) removed_head: String,
    pub(super) delivery_proof: DeliveryProof,
}

#[derive(Debug, Serialize)]
pub(super) struct DeliveryProof {
    basis: &'static str,
    default_branch: String,
    default_head: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_number: Option<u64>,
}

fn refused(code: &'static str, message: &str) -> CliError {
    CliError::data(code, message)
        .with_hint("Retain the target until the failed proof is resolved; do not force removal")
}

fn probe(program: &str, args: &[&str], cwd: &Path) -> Result<Output, CliError> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if program == "git" {
        lease::sanitize_git_environment(&mut command);
    }
    lease::removal_probe(&mut command).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "a bounded removal proof could not be completed",
        )
    })
}

fn git(target: &Path, args: &[&str]) -> Result<String, CliError> {
    let output = probe("git", args, target)?;
    if !output.status.success() {
        return Err(refused(
            "removal-proof-unavailable",
            "Git could not establish removal proof",
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .map_err(|_| {
            refused(
                "removal-proof-unavailable",
                "Git returned an unreadable proof",
            )
        })
}

fn json(program: &str, args: &[&str], cwd: &Path) -> Result<Value, CliError> {
    let output = probe(program, args, cwd)?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "removal proof response is malformed",
        )
    })?;
    if !output.status.success() || value["ok"] != true {
        return Err(refused(
            "removal-proof-unavailable",
            "removal proof response is unsuccessful",
        ));
    }
    Ok(value["data"].clone())
}

fn session_root() -> Result<PathBuf, CliError> {
    if let Some(path) = env::var_os("AGENT_SESSION_STATE_DIR").filter(|value| !value.is_empty()) {
        return Ok(path.into());
    }
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "session state root is unavailable",
            )
        })?;
    Ok(base.join("agent-session"))
}

fn sessions_idle(target: &Path, sessions: &Value) -> Result<(), CliError> {
    let rows = sessions.as_array().ok_or_else(|| {
        refused(
            "removal-proof-unavailable",
            "session inventory is malformed",
        )
    })?;
    for row in rows {
        let status = row["status"]
            .as_str()
            .ok_or_else(|| refused("removal-proof-unavailable", "session status is unknown"))?;
        if status == "stopped" {
            continue;
        }
        if status != "running" {
            return Err(refused(
                "removal-proof-unavailable",
                "session liveness is unknown",
            ));
        }
        let cwd = row["cwd"].as_str().ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "session checkout binding is unknown",
            )
        })?;
        let cwd = fs::canonicalize(cwd).map_err(|_| {
            refused(
                "removal-proof-unavailable",
                "live session checkout cannot be resolved",
            )
        })?;
        if cwd.starts_with(target) {
            return Err(refused(
                "removal-session-active",
                "a live session is bound to the removal target",
            ));
        }
    }
    Ok(())
}

fn processes_idle(target: &Path) -> Result<(), CliError> {
    let path = target.to_str().ok_or_else(|| {
        refused(
            "removal-proof-unavailable",
            "process probe target is unreadable",
        )
    })?;
    // cwd, directory descriptors, mapped files, and regular open files are all
    // selected. Any warning means the inventory may be incomplete.
    let output = probe(
        "lsof",
        &["-nP", "-Fpf", "+D", path],
        target.parent().unwrap(),
    )?;
    if !output.stdout.is_empty() {
        return Err(refused(
            "removal-process-active",
            "a live process has its cwd or an open file in the removal target",
        ));
    }
    if output.status.code() != Some(1) || !output.stderr.is_empty() {
        return Err(refused(
            "removal-proof-unavailable",
            "complete process cwd/open-file visibility is unavailable",
        ));
    }
    Ok(())
}

fn delivered(target: &Path, head: &str) -> Result<DeliveryProof, CliError> {
    // Query the remote now: cached refs cannot prove that the head was pushed.
    let advertised = git(target, &["ls-remote", "--symref", "origin", "HEAD"])?;
    let default = advertised
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|line| line.strip_suffix("\tHEAD"))
        })
        .filter(|name| !name.is_empty() && !name.starts_with('-'))
        .ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "remote default branch could not be resolved",
            )
        })?;
    let advertised_head = advertised
        .lines()
        .filter_map(|line| line.strip_suffix("\tHEAD"))
        .find(|oid| {
            matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "remote default HEAD is malformed",
            )
        })?;
    // Fetch into FETCH_HEAD rather than trusting or modifying the local base.
    git(
        target,
        &[
            "fetch",
            "--no-tags",
            "origin",
            &format!("refs/heads/{default}"),
        ],
    )?;
    let default_head = git(target, &["rev-parse", "--verify", "FETCH_HEAD"])?;
    if default_head != advertised_head {
        return Err(refused(
            "removal-proof-unavailable",
            "remote default HEAD changed during proof",
        ));
    }
    if probe(
        "git",
        &[
            "--no-replace-objects",
            "merge-base",
            "--is-ancestor",
            head,
            &default_head,
        ],
        target,
    )?
    .status
    .success()
    {
        return Ok(DeliveryProof {
            basis: "remote-default-ancestry",
            default_branch: default.to_string(),
            default_head,
            pr_number: None,
        });
    }
    // Squash/rebase merge proof is provider-bound to the exact published head.
    let branch = git(target, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let listed = json(
        "forge-cli",
        &[
            "pr", "list", "--state", "merged", "--head", &branch, "--base", default, "--format",
            "json",
        ],
        target,
    )?;
    let rows = listed["items"].as_array().ok_or_else(|| {
        refused(
            "removal-proof-unavailable",
            "merged pull request inventory is malformed",
        )
    })?;
    for row in rows {
        let number = row["number"].as_u64().ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "merged pull request identity is malformed",
            )
        })?;
        let pr = json(
            "forge-cli",
            &["pr", "view", &number.to_string(), "--format", "json"],
            target,
        )?;
        if pr["state"] == "merged"
            && pr["head"] == branch
            && pr["base"] == default
            && pr["head_sha"] == head
            && pr["merged_at"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        {
            return Ok(DeliveryProof {
                basis: "provider-exact-head-merge",
                default_branch: default.to_string(),
                default_head,
                pr_number: Some(number),
            });
        }
    }
    Err(refused(
        "removal-head-undelivered",
        "target HEAD has no current pushed-and-merged proof",
    ))
}

pub(super) fn fence(target: &Path, layout: &WorktreeLayout) -> Result<Fence, CliError> {
    if !is_managed_worktree(target, layout) {
        return Err(refused(
            "removal-unmanaged",
            "removal target is outside the managed worktree tree",
        ));
    }
    let target = fs::canonicalize(target)
        .map_err(|_| refused("removal-proof-unavailable", "removal target is missing"))?;
    let state = session_root()?;
    // Lock order is lifecycle -> checkout lease -> registry. The lifecycle
    // barrier covers launch before session registration and survives deletion.
    let lifecycle = worktree_lifecycle::Guard::acquire(&state, &target).map_err(|error| {
        let code = match error {
            worktree_lifecycle::Error::Busy => "removal-lifecycle-busy",
            worktree_lifecycle::Error::Changed => "removal-target-changed",
            worktree_lifecycle::Error::Unavailable => "removal-proof-unavailable",
        };
        refused(code, "checkout lifecycle fencing is busy or unavailable")
    })?;
    let identity = fs::metadata(&target).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "target identity is unavailable",
        )
    })?;
    if !git(
        &target,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err(refused(
            "removal-dirty",
            "dirty removal target must be retained",
        ));
    }
    let checkout = lease::fence_removal(&target).map_err(|_| {
        refused(
            "removal-lease-active-or-unavailable",
            "checkout lease or Git operation proof failed",
        )
    })?;
    let head = git(&target, &["rev-parse", "--verify", "HEAD"])?;
    let branch = probe(
        "git",
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        &target,
    )?;
    let removed_branch = if branch.status.success() {
        Some(
            String::from_utf8(branch.stdout)
                .map_err(|_| refused("removal-proof-unavailable", "target branch is unreadable"))?
                .trim()
                .to_string(),
        )
    } else if branch.status.code() == Some(1) {
        None
    } else {
        return Err(refused(
            "removal-proof-unavailable",
            "target branch is unavailable",
        ));
    };
    let delivery_proof = delivered(&target, &head)?;
    // `list` is the session owner's public liveness projection, not raw state.
    let sessions = json(
        "agent-session",
        &["list", "--format", "json"],
        &layout.repo_root,
    )?;
    sessions_idle(&target, &sessions)?;
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(state.join("coordination"))
        .map_err(|_| {
            refused(
                "removal-proof-unavailable",
                "session registry directory is unavailable",
            )
        })?;
    let registry = lease::removal_registry_lock(&state.join("coordination")).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "session registry fencing is unavailable",
        )
    })?;
    if let Some(projection) = coordination_projection::load(&state).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "session ownership projection is unavailable",
        )
    })? {
        let fingerprint = coordination_projection::worktree_fingerprint(
            projection.fingerprint_epoch,
            &projection.fingerprint_key,
            &target,
        )
        .ok_or_else(|| {
            refused(
                "removal-proof-unavailable",
                "checkout binding fingerprint is unavailable",
            )
        })?;
        if projection.claims.iter().any(|claim| {
            !matches!(
                claim.state.as_str(),
                "active" | "released" | "expired" | "stale"
            )
        }) {
            return Err(refused(
                "removal-proof-unavailable",
                "session claim state is unknown",
            ));
        }
        if projection.operations.iter().any(|operation| {
            operation.schema_version != "agent-session.operation-lease.v1"
                || !matches!(
                    operation.state.as_str(),
                    "active"
                        | "completing"
                        | "reconcile_pending"
                        | "completed"
                        | "failed"
                        | "abandoned"
                )
        }) {
            return Err(refused(
                "removal-proof-unavailable",
                "session operation schema or state is unknown",
            ));
        }
        if projection
            .claims
            .iter()
            .any(|claim| claim.state == "active" && claim.worktrees.contains(&fingerprint))
        {
            return Err(refused(
                "removal-session-active",
                "an active session claim is bound to the removal target",
            ));
        }
        for operation in &projection.operations {
            if matches!(
                operation.state.as_str(),
                "completed" | "failed" | "abandoned"
            ) {
                continue;
            }
            let claim = projection
                .claims
                .iter()
                .find(|claim| !claim.claim_id.is_empty() && claim.claim_id == operation.claim_id)
                .ok_or_else(|| {
                    refused(
                        "removal-proof-unavailable",
                        "operation checkout binding is unavailable",
                    )
                })?;
            if claim.worktrees.contains(&fingerprint) {
                return Err(refused(
                    "removal-session-active",
                    "a nonterminal session operation is bound to the removal target",
                ));
            }
        }
    }
    processes_idle(&target)?;
    lifecycle.verify().map_err(|_| {
        refused(
            "removal-target-changed",
            "checkout lifecycle identity changed during proof",
        )
    })?;
    checkout.verify_target().map_err(|_| {
        refused(
            "removal-target-changed",
            "checkout identity changed during proof",
        )
    })?;
    let after = fs::metadata(&target).map_err(|_| {
        refused(
            "removal-proof-unavailable",
            "target disappeared during proof",
        )
    })?;
    if (identity.dev(), identity.ino()) != (after.dev(), after.ino())
        || git(&target, &["rev-parse", "--verify", "HEAD"])? != head
        || !git(
            &target,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?
        .is_empty()
    {
        return Err(refused(
            "removal-target-changed",
            "target changed during removal proof",
        ));
    }
    Ok(Fence {
        _registry: registry,
        _checkout: checkout,
        _lifecycle: lifecycle,
        removed_branch,
        removed_head: head,
        delivery_proof,
    })
}

pub(super) fn remove(target: &Path, repo_root: &Path) -> Result<(), CliError> {
    let path = target
        .to_str()
        .ok_or_else(|| refused("removal-proof-unavailable", "target path is unreadable"))?;
    git(repo_root, &["worktree", "remove", path])?;
    Ok(())
}
