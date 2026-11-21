//! Admit additive coverage and apply the evidence gate to other test-kind changes.
use std::path::Path;
use std::process::Command;

use crate::config::TestFirstPolicy;
use crate::error::ForgeError;
use crate::ops::pr_review_handoff::test_integrity::{declarations, diff, validate_declarations};
use crate::provider::{Provider, ProviderContext};
use crate::validations::PrKind;

use super::VerifiedDeliverySubject;

pub(crate) fn policy(
    policy: &TestFirstPolicy,
    ctx: &ProviderContext,
    kind: PrKind,
    workdir: &Path,
    remote: &str,
    base: &str,
    head: &str,
) -> Result<(TestFirstPolicy, Option<VerifiedDeliverySubject>), ForgeError> {
    let mut effective = policy.clone();
    if policy.invalid_require_kinds
        || kind != PrKind::Test
        || !(policy.required_for(PrKind::Bug) || policy.required_for(kind))
    {
        return Ok((effective, None));
    }
    let base_ref = if base == "HEAD" {
        "HEAD".to_owned()
    } else {
        format!("refs/heads/{base}")
    };
    let fetch_target = target_remote(ctx, workdir, remote)?;
    let fetch = Command::new("git")
        .current_dir(workdir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            &fetch_target,
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
    let fetched = comparison_git(workdir, &["rev-parse", "--verify", "FETCH_HEAD^{commit}"])?;
    let head_ref = format!("refs/heads/{head}^{{commit}}");
    let head_oid = comparison_git(workdir, &["rev-parse", "--verify", &head_ref])?;
    let comparison = format!("{}...{}", fetched.trim(), head_oid.trim());
    let paths = comparison_diff(workdir, &comparison, &["--name-only", "-z"])?;
    let changed_paths = paths.split_terminator('\0');
    let test_sources_only = changed_paths.clone().all(|path| {
        dedicated_test_path(path)
            && diff::source(path)
            && !path
                .split('/')
                .any(|part| matches!(part, "fixture" | "fixtures" | "__fixtures__"))
    });
    // Python permits redefining a test name. Compare appended definitions with
    // the whole old file to distinguish a new test from a replacement.
    let context = if changed_paths.clone().any(|path| path.ends_with(".py")) {
        "--unified=2147483647"
    } else {
        "--unified=3"
    };
    // An empty comparison stays unselected unless test was explicitly selected.
    // Bind its head too, so a stale tracking ref cannot admit a different PR diff.
    if (paths.is_empty() && !policy.required_for(kind))
        || (test_sources_only
            && diff::additive_coverage(&comparison_diff(
                workdir,
                &comparison,
                &["--patch", context],
            )?)?)
    {
        effective
            .require_kinds
            .retain(|selected| *selected != PrKind::Test);
        return Ok((
            effective,
            Some(VerifiedDeliverySubject {
                head: head_oid.trim().to_owned(),
                validation_summary: String::new(),
            }),
        ));
    }
    if !policy.required_for(kind) {
        effective.require_kinds.push(kind);
    }
    Ok((effective, None))
}

fn dedicated_test_path(path: &str) -> bool {
    let mut components = path.split('/');
    match components.next() {
        Some("test" | "tests" | "__tests__") => true,
        Some("crates") => components.nth(1) == Some("tests"),
        _ => false,
    }
}

pub(crate) fn validate_body(kind: PrKind, body: &str) -> Result<(), ForgeError> {
    if kind != PrKind::Test {
        return Ok(());
    }
    if declarations(body).trim().is_empty() {
        return Err(ForgeError::validation(
            super::schema_err(),
            "test_changes_undeclared",
            "a test PR requires a non-empty ## Test changes section; use None. when the detector has no findings",
            None,
        ));
    }
    validate_declarations(body)
}

fn comparison_diff(
    workdir: &Path,
    comparison: &str,
    options: &[&str],
) -> Result<String, ForgeError> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--no-relative",
        "--ignore-submodules=none",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    args.extend(options);
    args.extend([comparison, "--"]);
    comparison_git(workdir, &args)
}

fn comparison_git(workdir: &Path, args: &[&str]) -> Result<String, ForgeError> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .map_err(|e| {
            ForgeError::software(
                super::schema_err(),
                "git test-change comparison failed to spawn",
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
    String::from_utf8(output.stdout).map_err(|_| {
        ForgeError::validation(
            super::schema_err(),
            "test_changes_incomplete",
            "test-change comparison is not UTF-8",
            None,
        )
    })
}

/// Use the configured remote only when it identifies the selected target.
fn target_remote(
    ctx: &ProviderContext,
    workdir: &Path,
    remote: &str,
) -> Result<String, ForgeError> {
    if ctx.provider == Provider::Local {
        return Ok(remote.to_owned());
    }
    let repo = ctx.repo.as_deref().ok_or_else(|| {
        ForgeError::validation(
            super::schema_err(),
            "test_changes_fetch_failed",
            "cannot identify the target repository for test-change inspection",
            None,
        )
    })?;
    let identity = nils_common::git::canonical_repository_identity(&ctx.host, repo);
    let urls = Command::new("git")
        .current_dir(workdir)
        .args(["remote", "get-url", "--all", "--", remote])
        .output();
    if let Ok(output) = urls
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let urls: Vec<_> = text.lines().filter(|url| !url.is_empty()).collect();
        if !urls.is_empty()
            && urls.iter().all(|url| {
                nils_common::git::repository_identity_from_remote_url(url).as_deref()
                    == Some(identity.as_str())
            })
        {
            return Ok(remote.to_owned());
        }
    }
    let mut target = url::Url::parse(&format!("https://{}/", ctx.host)).map_err(|_| {
        ForgeError::validation(
            super::schema_err(),
            "test_changes_fetch_failed",
            "cannot form the target repository address for test-change inspection",
            None,
        )
    })?;
    {
        let mut segments = target.path_segments_mut().map_err(|_| {
            ForgeError::validation(
                super::schema_err(),
                "test_changes_fetch_failed",
                "cannot form the target repository address for test-change inspection",
                None,
            )
        })?;
        segments.pop_if_empty();
        let mut parts = repo.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                segments.push(&format!("{part}.git"));
            } else {
                segments.push(part);
            }
        }
    }
    Ok(target.into())
}
