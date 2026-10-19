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
        "test_changes_limit_exceeded" => "split_comparison_or_rename_paths",
        _ => "retry_complete_comparison",
    };
    ForgeError::validation_with_details(
        "cli.forge-cli.pr.review-handoff.v1",
        kind,
        message,
        serde_json::json!({"detail": detail, "next_action": next_action, "retryable": kind == "test_changes_incomplete"}),
    )
}

fn limit(name: &str, message: &str) -> ForgeError {
    error(
        "test_changes_limit_exceeded",
        message,
        Some(format!(
            "Limit: {name}. Recovery: split the PR into smaller comparisons; for path_bytes, rename affected paths to unambiguous printable ASCII without whitespace, quotes or pipes. See crates/forge-cli/docs/specs/test-change-admission-v1.md#limits-and-recovery. No bypass is available."
        )),
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

/// Re-read mutable provider metadata after reading the immutable comparison.
fn comparison<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    head: &str,
) -> Result<(Subject, String, Vec<diff::Finding>), ForgeError> {
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
    if subject.head != head
        || !super::valid_sha(head)
        || !super::valid_sha(&subject.base)
        || subject.author.is_empty()
    {
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
    Ok((subject, patch, findings))
}

fn digest(subject: &Subject, patch: &str) -> String {
    sha256_digest(
        format!(
            "{}\n{}\n{patch}\n{}",
            subject.base,
            subject.head,
            declarations(&subject.body)
        )
        .as_bytes(),
    )
}

/// Read-only author preview: no assignment, ledger or review is required.
pub(super) fn preview<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    head: &str,
    format: nils_common::cli_contract::OutputFormat,
) -> Result<i32, ForgeError> {
    let (subject, patch, findings) = comparison(runner, ctx, number, head)?;
    let digest = digest(&subject, &patch);
    let confirmation = format!("Test changes confirmed: {digest}");
    let payload = serde_json::json!({"number": number, "head_sha": head, "base_sha": subject.base, "findings": findings, "digest": digest, "confirmation": confirmation});
    Ok(crate::envelope::emit_success(
        super::schema(),
        payload,
        format,
        |p| {
            for finding in p["findings"].as_array().unwrap() {
                println!(
                    "- {} | {} | <reason and retained coverage>",
                    finding["path"].as_str().unwrap(),
                    finding["kind"].as_str().unwrap()
                );
            }
            println!("{}", p["confirmation"].as_str().unwrap());
        },
    ))
}

/// Runs only after ordinary designated-review identity, interval and PASS checks.
pub(super) fn ensure<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    head: &str,
    review_author: &str,
    report: &str,
) -> Result<(), ForgeError> {
    let (subject, patch, findings) = comparison(runner, ctx, number, head)?;
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
    if rows.len() > 32 {
        return Err(limit(
            "declaration_pairs",
            "declaration contains more than 32 file/kind rows",
        ));
    }
    if !missing.is_empty() {
        let files = findings
            .iter()
            .map(|f| format!("- {} | {} | <reason and retained coverage>", f.path, f.kind))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(error(
            "test_changes_undeclared",
            "test changes require explicit PR-body declarations under ## Test changes",
            Some(format!(
                "Before requesting review, run `forge-cli pr review-handoff test-changes {number} --expected-head {head}` to print findings and the digest without an existing review. Add the following rows to ## Test changes with a concrete reason (at least 12 characters), then rerun that preview for the updated digest. Intentional removals/false positives may use '- allow <path> | <kind> | <reason>'; at most 32 exact entries, no globs. All entries still need independent reviewer confirmation.\n{files}"
            )),
        ));
    }
    let digest = digest(&subject, &patch);
    let confirmation = format!("Test changes confirmed: {digest}");
    if subject.author.eq_ignore_ascii_case(review_author)
        || !report.lines().any(|line| line.trim() == confirmation)
    {
        return Err(error(
            "test_changes_unconfirmed",
            "the designated independent reviewer must confirm the declared test changes in the published passing report",
            Some(format!(
                "{confirmation}\nRead-only pre-review preview: `forge-cli pr review-handoff test-changes {number} --expected-head {head}`. Review every declared/allowlisted change and publish that exact line in the designated review report. Confirmation expires when the head, base, diff or declaration changes."
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
