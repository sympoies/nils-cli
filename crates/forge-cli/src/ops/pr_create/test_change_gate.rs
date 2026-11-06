//! Apply the bug evidence policy to test changes flagged by the review detector.
use std::path::Path;
use std::process::Command;

use crate::config::TestFirstPolicy;
use crate::error::ForgeError;
use crate::ops::pr_review_handoff::test_integrity::diff;
use crate::validations::PrKind;

pub(crate) fn policy(
    policy: &TestFirstPolicy,
    kind: PrKind,
    workdir: &Path,
    remote: &str,
    base: &str,
    head: &str,
) -> Result<TestFirstPolicy, ForgeError> {
    let mut effective = policy.clone();
    if policy.invalid_require_kinds
        || kind != PrKind::Test
        || !policy.required_for(PrKind::Bug)
        || policy.required_for(kind)
    {
        return Ok(effective);
    }
    let base_ref = if base == "HEAD" {
        "HEAD".to_owned()
    } else {
        format!("refs/heads/{base}")
    };
    let fetch = Command::new("git")
        .current_dir(workdir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            remote,
            &base_ref,
        ])
        .output()
        .map_err(|e| {
            ForgeError::software(
                super::schema_err(),
                "git fetch failed to spawn",
                Some(e.to_string()),
            )
        })?;
    if !fetch.status.success() {
        return Err(ForgeError::validation(
            super::schema_err(),
            "test_changes_fetch_failed",
            "cannot fetch the base for test-change inspection; check the remote and base, then retry",
            None,
        ));
    }
    let resolved = Command::new("git")
        .current_dir(workdir)
        .args(["rev-parse", "--verify", "FETCH_HEAD^{commit}"])
        .output()
        .map_err(|e| {
            ForgeError::software(
                super::schema_err(),
                "git rev-parse failed to spawn",
                Some(e.to_string()),
            )
        })?;
    if !resolved.status.success() {
        return Err(ForgeError::validation(
            super::schema_err(),
            "test_changes_incomplete",
            "cannot resolve the fetched test-change base",
            None,
        ));
    }
    let fetched = std::str::from_utf8(&resolved.stdout)
        .map_err(|_| {
            ForgeError::validation(
                super::schema_err(),
                "test_changes_incomplete",
                "cannot resolve the fetched test-change base",
                None,
            )
        })?
        .trim();
    let comparison = format!("{fetched}...{head}");
    let mut command = Command::new("git");
    command.current_dir(workdir).args([
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        &comparison,
        "--",
    ]);
    for extension in diff::SOURCE_EXTENSIONS {
        command.arg(format!(":(glob)**/*.{extension}"));
    }
    let output = command.output().map_err(|e| {
        ForgeError::software(
            super::schema_err(),
            "git diff failed to spawn",
            Some(e.to_string()),
        )
    })?;
    if !output.status.success() {
        return Err(ForgeError::validation(
            super::schema_err(),
            "test_changes_incomplete",
            "cannot inspect the complete test-change comparison",
            None,
        ));
    }
    let patch = std::str::from_utf8(&output.stdout).map_err(|_| {
        ForgeError::validation(
            super::schema_err(),
            "test_changes_incomplete",
            "test-change comparison is not UTF-8",
            None,
        )
    })?;
    if !diff::detect(patch)?.is_empty() {
        effective.require_kinds.push(PrKind::Test);
    }
    Ok(effective)
}
