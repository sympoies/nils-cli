//! Opt-in local proofs for callers that intend eligibility-based cleanup.
use super::{
    CliError, Fence, git_error_reason, git_state, git_stdout, local_git_command, refused, run_probe,
};
use serde_json::json;
use std::{path::Path, process::Output};

#[derive(Debug, Default)]
pub(in crate::worktree) struct Predicates {
    expected_head: Option<String>,
    ancestor_of: Option<String>,
    pub(in crate::worktree) require_clean: bool,
}

impl Predicates {
    pub(in crate::worktree) fn active(&self) -> bool {
        self.expected_head.is_some() || self.ancestor_of.is_some() || self.require_clean
    }

    pub(in crate::worktree) fn set(&mut self, flag: &str, value: String) -> Result<(), CliError> {
        let slot = match flag {
            "--expect-head" if object_id(&value) => &mut self.expected_head,
            "--ancestor-of" if value.starts_with("refs/") => &mut self.ancestor_of,
            "--expect-head" => {
                return Err(CliError::usage(
                    "invalid-expected-head",
                    "--expect-head requires a full Git object ID",
                ));
            }
            _ => {
                return Err(CliError::usage(
                    "invalid-ancestor-ref",
                    "--ancestor-of requires a fully qualified local ref under refs/",
                ));
            }
        };
        if slot.replace(value).is_some() {
            return Err(CliError::usage(
                "duplicate-argument",
                format!("{flag} can be given only once"),
            ));
        }
        Ok(())
    }

    pub(super) fn verify(&self, target: &Path, fence: &mut Fence) -> Result<(), CliError> {
        if !self.active() {
            return Ok(());
        }
        let unavailable = || refused("removal-proof-unavailable", "removal fence is unavailable");
        fence
            .lifecycle
            .as_ref()
            .ok_or_else(unavailable)?
            .verify()
            .map_err(|_| refused("removal-target-changed", "lifecycle identity changed"))?;
        fence
            .checkout
            .as_ref()
            .ok_or_else(unavailable)?
            .verify_target()
            .map_err(|_| refused("removal-target-changed", "checkout identity changed"))?;
        fence.operations = git_state(target, fence.removed_branch.as_deref(), &mut fence.warnings)?;
        fence.warnings.sort();
        fence.warnings.dedup();

        let head = git_stdout(local_probe(
            target,
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )?)?;
        if !object_id(&head) {
            return Err(refused(
                "removal-proof-unavailable",
                "Git returned an invalid HEAD object ID",
            ));
        }
        if let Some(expected) = &self.expected_head
            && !head.eq_ignore_ascii_case(expected)
        {
            return Err(refused(
                "removal-head-mismatch",
                "target HEAD differs from the expected object ID",
            )
            .with_details(json!({"expected_head": expected, "actual_head": head})));
        }
        if head != fence.removed_head {
            return Err(refused(
                "removal-target-changed",
                "target HEAD changed during removal",
            ));
        }
        if let Some(reference) = &self.ancestor_of {
            verify_ancestry(target, &head, reference)?;
        }
        if self.require_clean {
            verify_clean(target, &fence.operations)?;
        }
        Ok(())
    }
}

fn object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn local_probe(target: &Path, args: &[&str]) -> Result<Output, CliError> {
    run_probe(
        local_git_command(target)
            .env("GIT_GRAFT_FILE", "/dev/null")
            .args([
                "--no-replace-objects",
                "--no-optional-locks",
                "-c",
                "advice.graftFileDeprecated=false",
            ])
            .args(args),
    )
}

fn verify_ancestry(target: &Path, head: &str, reference: &str) -> Result<(), CliError> {
    // Use an exact ref lookup so a revision expression cannot select another commit.
    let unavailable = || {
        refused(
            "removal-ref-unavailable",
            "selected local ref does not resolve to an available commit",
        )
        .with_details(json!({"ref": reference}))
    };
    let oid = git_stdout(local_probe(
        target,
        &["show-ref", "--verify", "--hash", reference],
    )?)
    .map_err(|_| unavailable())?;
    if !object_id(&oid) {
        return Err(unavailable());
    }
    let tip = git_stdout(local_probe(
        target,
        &["rev-parse", "--verify", &format!("{oid}^{{commit}}")],
    )?)
    .map_err(|_| unavailable())?;
    if !object_id(&tip) {
        return Err(unavailable());
    }
    let result = local_probe(
        target,
        &[
            "-c",
            "core.commitGraph=false",
            "merge-base",
            "--is-ancestor",
            head,
            &tip,
        ],
    )?;
    match result.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(refused(
            "removal-not-ancestor",
            "target HEAD is not an ancestor of the selected local ref",
        )
        .with_details(json!({"head": head, "ref": reference, "ref_head": tip}))),
        _ => Err(
            refused("removal-proof-unavailable", "local ancestry check failed")
                .with_details(json!({"reason": git_error_reason(&result)})),
        ),
    }
}

fn verify_clean(target: &Path, operations: &[String]) -> Result<(), CliError> {
    let dirty = || {
        refused(
            "removal-dirty",
            "target has staged, unstaged, or non-ignored untracked changes, or a Git operation in progress",
        )
    };
    if !operations.is_empty() {
        return Err(dirty());
    }
    let status = local_probe(
        target,
        &[
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.trustctime=true",
            "-c",
            "core.checkStat=default",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    if !status.status.success() || !status.stderr.is_empty() {
        return Err(refused(
            "removal-proof-unavailable",
            "clean state could not be verified",
        )
        .with_details(json!({"reason": git_error_reason(&status)})));
    }
    if !status.stdout.is_empty() {
        return Err(dirty());
    }
    let flags = local_probe(target, &["ls-files", "-v", "-z"])?;
    // Lowercase flags (assume-unchanged) and S (skip-worktree) can hide edits.
    if !flags.status.success()
        || !flags.stderr.is_empty()
        || flags
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .any(|record| !record.starts_with(b"H "))
    {
        return Err(refused(
            "removal-proof-unavailable",
            "index flags prevent a clean state proof",
        ));
    }
    Ok(())
}
