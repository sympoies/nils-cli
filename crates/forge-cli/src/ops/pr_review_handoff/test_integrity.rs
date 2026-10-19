//! Conservative test-change admission for designated GitHub review.
//! The author declares findings; only the selected independent native report
//! can confirm the declaration for this immutable comparison.

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::error::ForgeError;
use crate::ops::review_state::sha256_digest;
use crate::provider::ProviderContext;
use serde::Deserialize;

mod diff;
#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Subject {
    head: String,
    base: String,
    author: String,
    body: String,
}

fn error(kind: &'static str, message: impl Into<String>, detail: Option<String>) -> ForgeError {
    let next_action = match kind {
        "test_changes_undeclared" => "declare_test_changes",
        "test_changes_unconfirmed" => "request_reviewer_confirmation",
        _ => "retry_complete_comparison",
    };
    ForgeError::validation_with_details(
        "cli.forge-cli.pr.review-handoff.v1",
        kind,
        message,
        serde_json::json!({"detail": detail, "next_action": next_action, "retryable": kind == "test_changes_incomplete"}),
    )
}

fn incomplete() -> ForgeError {
    error(
        "test_changes_incomplete",
        "cannot inspect the complete test-change comparison; retry with complete provider evidence",
        None,
    )
}

fn api(ctx: &ProviderContext, endpoint: String, extra: &[&str]) -> BackendCall {
    let mut argv = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(endpoint.into());
    argv.extend(extra.iter().map(Into::into));
    BackendCall::new(BackendProgram::Gh, argv)
}

/// Runs only after ordinary designated-review identity, interval and PASS checks.
/// Re-read mutable provider metadata after reading the immutable comparison.
pub(super) fn ensure<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    head: &str,
    review_author: &str,
    report: &str,
) -> Result<(), ForgeError> {
    let repo = ctx.repo.as_deref().ok_or_else(incomplete)?;
    let subject_call = api(
        ctx,
        format!("repos/{repo}/pulls/{number}"),
        &[
            "--jq",
            "{head: .head.sha, base: .base.sha, author: .user.login, body: (.body // \"\")}",
        ],
    );
    let read_subject = || -> Result<Subject, ForgeError> {
        serde_json::from_str(&runner.run(&subject_call)?.stdout).map_err(|_| incomplete())
    };
    let subject = read_subject()?;
    if subject.head != head || !super::valid_sha(&subject.base) || subject.author.is_empty() {
        return Err(incomplete());
    }
    let patch = runner
        .run(&api(
            ctx,
            format!("repos/{repo}/compare/{}...{head}", subject.base),
            &["--header", "Accept: application/vnd.github.diff"],
        ))?
        .stdout;
    let findings = diff::detect(&patch)?;
    if read_subject()? != subject {
        return Err(error(
            "test_changes_incomplete",
            "PR head, base, author or declaration changed during test-change admission",
            None,
        ));
    }
    if findings.is_empty() {
        return Ok(());
    }
    let declarations = declarations(&subject.body);
    let rows: Vec<_> = declarations
        .lines()
        .filter_map(|line| {
            let row = line.trim().strip_prefix("- ")?;
            let (path, rest) = row.split_once(" | ")?;
            let (kind, reason) = rest.split_once(" | ")?;
            (reason.trim().len() >= 12)
                .then_some((path.strip_prefix("allow ").unwrap_or(path), kind))
        })
        .collect();
    let missing: Vec<_> = findings
        .iter()
        .filter(|f| {
            !rows
                .iter()
                .any(|(path, kind)| *path == f.path && *kind == f.kind)
        })
        .collect();
    if rows.len() > 32 || !missing.is_empty() {
        let files = findings
            .iter()
            .map(|f| format!("- {} | {} | <reason and retained coverage>", f.path, f.kind))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(error(
            "test_changes_undeclared",
            "test changes require explicit PR-body declarations under ## Test changes",
            Some(format!(
                "Add the following rows to ## Test changes with a concrete reason (at least 12 characters). Intentional removals/false positives may use '- allow <path> | <kind> | <reason>'; at most 32 exact entries, no globs. All entries still need independent reviewer confirmation.\n{files}"
            )),
        ));
    }
    let digest =
        sha256_digest(format!("{}\n{}\n{patch}\n{declarations}", subject.base, head).as_bytes());
    let confirmation = format!("Test changes confirmed: {digest}");
    if subject.author.eq_ignore_ascii_case(review_author)
        || !report.lines().any(|line| line.trim() == confirmation)
    {
        return Err(error(
            "test_changes_unconfirmed",
            "the designated independent reviewer must confirm the declared test changes in the published passing report",
            Some(format!(
                "{confirmation}\nReview every declared/allowlisted change and publish that exact line in the designated review report. Confirmation expires when the head, base, diff or declaration changes."
            )),
        ));
    }
    Ok(())
}

fn declarations(body: &str) -> String {
    let mut active = false;
    let mut fence = None;
    let mut lines = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = &trimmed[..3];
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if line.starts_with("## ") {
            active = line.trim() == "## Test changes";
        } else if active {
            lines.push(line);
        }
    }
    lines.join("\n")
}
