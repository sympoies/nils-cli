//! `pr create --supersedes <old-pr>`.
//!
//! A replacement PR normally needs a fresh test-first record bound to its own
//! head. When the new head differs from the superseded PR's reviewed head only
//! by merges of the default branch, the old PR's verified record still covers
//! the source, so the gate verifies that record against the old head and then
//! proves the new head adds nothing beyond default-branch merges.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use nils_common::cli_contract::schema_version_for;

use crate::backend::BackendRunner;
use crate::cli::BINARY;
use crate::error::ForgeError;
use crate::ops::pr_view;
use crate::provider::ProviderContext;

use super::VerifiedTestFirstSubject;

/// Verified subject for the new head and the superseded PR number, if any.
pub(super) type Outcome = (Option<VerifiedTestFirstSubject>, Option<u64>);

/// Run the test-first gate. Without `--supersedes` this is `verify_record`
/// against the branch being delivered. With it, `verify_record` runs against the
/// superseded PR's head and the new head must differ from that head only by
/// default-branch merges.
#[allow(clippy::too_many_arguments)]
pub(super) fn gate<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    supersedes: Option<u64>,
    gate_applies: bool,
    workdir: &Path,
    remote: &str,
    base: &str,
    branch: &str,
    verify_record: impl FnOnce(&str) -> Result<Option<VerifiedTestFirstSubject>, ForgeError>,
) -> Result<Outcome, ForgeError> {
    let Some(old_pr) = supersedes else {
        return Ok((verify_record(branch)?, None));
    };
    if !gate_applies {
        return Err(refuse(
            "supersedes_not_applicable",
            "--supersedes only applies to kinds covered by the test-first gate",
            None,
        ));
    }
    let old = pr_view::compute(runner, ctx, old_pr)?;
    let old_head = match old.head_sha.as_deref() {
        Some(head) if old.state != "merged" => head.to_string(),
        _ => {
            return Err(refuse(
                "supersede_old_pr_unusable",
                format!("PR #{old_pr} is merged or exposes no head commit to supersede"),
                None,
            ));
        }
    };
    let subject = verify_record(&old_head)?;
    let new_head = git_text(
        workdir,
        &["rev-parse", "--verify", &format!("{branch}^{{commit}}")],
    )?;
    source_unchanged(workdir, remote, base, &old_head, &new_head)?;
    let subject = subject.map(|subject| VerifiedTestFirstSubject {
        head: new_head,
        ..subject
    });
    Ok((subject, Some(old_pr)))
}

/// Record the supersession link in the new PR body.
pub(super) fn append_link(body: String, supersedes: Option<u64>) -> String {
    match supersedes {
        Some(old_pr) => format!("{body}\n\nSupersedes #{old_pr}.\n"),
        None => body,
    }
}

/// The new head contains the old head, adds only merge commits or commits
/// already on the default branch, and leaves the branch's own change (the
/// patch against the default branch) identical.
fn source_unchanged(
    workdir: &Path,
    remote: &str,
    base: &str,
    old_head: &str,
    new_head: &str,
) -> Result<(), ForgeError> {
    let base_ref = format!("refs/remotes/{remote}/{base}");
    let contains_old = run_git(
        workdir,
        &["merge-base", "--is-ancestor", old_head, new_head],
        None,
    )?;
    let unchanged = contains_old.status.success()
        && git_text(
            workdir,
            &[
                "rev-list",
                "--no-merges",
                &format!("{old_head}..{new_head}"),
                "--not",
                &base_ref,
            ],
        )?
        .is_empty()
        && branch_patch_id(workdir, &base_ref, old_head)?
            == branch_patch_id(workdir, &base_ref, new_head)?;
    if unchanged {
        return Ok(());
    }
    Err(refuse(
        "supersede_source_changed",
        format!(
            "the new head differs from the superseded head {} by more than merges of \
             {base_ref}; deliver it with its own test-first record",
            old_head.get(..12).unwrap_or(old_head)
        ),
        None,
    ))
}

/// Stable patch id of the branch's change against its merge base with the
/// default branch, independent of where the default branch has advanced to.
fn branch_patch_id(workdir: &Path, base_ref: &str, head: &str) -> Result<String, ForgeError> {
    let merge_base = git_text(workdir, &["merge-base", base_ref, head])?;
    let diff = run_git(workdir, &["diff", "--binary", &merge_base, head], None)?;
    let id = run_git(workdir, &["patch-id", "--stable"], Some(&diff.stdout))?;
    Ok(String::from_utf8_lossy(&id.stdout).into_owned())
}

fn git_text(workdir: &Path, args: &[&str]) -> Result<String, ForgeError> {
    let output = run_git(workdir, args, None)?;
    if !output.status.success() {
        return Err(refuse(
            "supersede_unverifiable",
            format!("git {} failed", args.join(" ")),
            Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git(workdir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, ForgeError> {
    let spawn_failed = |error: std::io::Error| {
        ForgeError::software(
            schema_err(),
            format!("git {} failed to run", args.join(" ")),
            Some(error.to_string()),
        )
    };
    let mut child = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_failed)?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A closed pipe surfaces as git's own exit status below.
        let _ = pipe.write_all(input);
    }
    child.wait_with_output().map_err(spawn_failed)
}

fn refuse(code: &'static str, message: impl Into<String>, detail: Option<String>) -> ForgeError {
    ForgeError::validation(schema_err(), code, message, detail)
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}
