//! Governed approval of a GitHub Actions workflow run.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, RunApproveArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

const OP: &str = "run.approve";
#[derive(Deserialize)]
struct Run {
    id: u64,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    event: String,
    head_repository: Repo,
    pull_requests: Vec<Pr>,
}
#[derive(Deserialize)]
struct Repo {
    full_name: String,
}
#[derive(Deserialize)]
struct Pr {
    number: u64,
}
#[derive(Serialize)]
struct Receipt {
    provider: &'static str,
    repository: String,
    run_id: u64,
    head_sha: String,
    pr: u64,
    reason: String,
    prior_status: String,
    new_status: String,
    outcome: &'static str,
}

pub fn run(
    global: &GlobalFlags,
    args: RunApproveArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let ctx = write::target(global, OP)?;
    if args.run_id == 0
        || args.pr == 0
        || !matches!(args.expected_head.len(), 40 | 64)
        || !args.expected_head.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid(
            OP,
            "approval_arguments_invalid",
            "run and PR ids must be positive and expected head must be a full hexadecimal SHA",
        ));
    }
    let mut reason = String::new();
    std::fs::File::open(&args.reason_file)
        .map_err(|_| invalid(OP, "reason_file_unreadable", "unable to read reason file"))?
        .take(2001)
        .read_to_string(&mut reason)
        .map_err(|_| invalid(OP, "reason_file_unreadable", "unable to read reason file"))?;
    if reason.len() > 2000 {
        return Err(invalid(OP, "reason_invalid", "reason exceeds 2000 bytes"));
    }
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(invalid(
            OP,
            "reason_invalid",
            "reason must contain 1 to 2000 bytes",
        ));
    }
    if crate::backend::redact_tokens(&reason) != reason {
        return Err(invalid(
            OP,
            "reason_invalid",
            "reason must not contain credentials",
        ));
    }
    write::guard_text(&reason, "reason")?;
    let repo = ctx.repo.as_deref().expect("target repository");
    let path = format!("repos/{repo}/actions/runs/{}", args.run_id);
    let runner = crate::rate_limit::default_runner();
    let read = || -> Result<Run, ForgeError> {
        super::run_rerun::read(&runner, &ctx, &path, Duration::from_secs(15), OP)
    };
    let pr: serde_json::Value = super::run_rerun::read(
        &runner,
        &ctx,
        &format!("repos/{repo}/pulls/{}", args.pr),
        Duration::from_secs(15),
        OP,
    )?;
    let pr_head = pr["head"]["sha"].as_str().unwrap_or("");
    let pr_repo = pr["head"]["repo"]["full_name"].as_str().unwrap_or("");
    let fence = |run: &Run| -> Result<(), ForgeError> {
        if run.id != args.run_id || !run.head_sha.eq_ignore_ascii_case(&args.expected_head) {
            return Err(invalid(
                OP,
                "run_head_mismatch",
                "run does not match expected head",
            ));
        }
        if !pr_head.eq_ignore_ascii_case(&args.expected_head)
            || pr["number"].as_u64() != Some(args.pr)
            || pr_repo.is_empty()
            || !run.head_repository.full_name.eq_ignore_ascii_case(pr_repo)
            || !matches!(run.event.as_str(), "pull_request" | "pull_request_target")
            || (!run.pull_requests.is_empty()
                && !run.pull_requests.iter().any(|p| p.number == args.pr))
        {
            return Err(invalid(
                OP,
                "run-pr-mismatch",
                "run does not belong to the requested PR",
            ));
        }
        let awaiting = run.status == "action_required"
            || (run.status == "completed" && run.conclusion.as_deref() == Some("action_required"));
        if !awaiting {
            return Err(ForgeError::validation_with_details(
                schema_version_for(BINARY, OP, 1),
                "run-not-awaiting-approval",
                "run is not awaiting approval",
                serde_json::json!({"status": run.status, "head": run.head_sha, "pr": args.pr}),
            ));
        }
        Ok(())
    };
    let before = read()?;
    fence(&before)?;
    let mutation = super::run_rerun::call(&ctx, &format!("{path}/approve"), true);
    let schema = schema_version_for(BINARY, OP, 1);
    if global.dry_run {
        return Ok(emit_success(
            schema,
            DryRunPayload::new(ctx.provider, &mutation),
            format,
            |p| println!("would run: {}", p.plan.join(" ")),
        ));
    }
    if let Err(error) = runner.run_with_timeout(&mutation, Some(Duration::from_secs(15))) {
        if let Ok(latest) = read()
            && latest.status != "action_required"
        {
            fence(&latest)?;
        }
        return Err(error);
    }
    let after = read()?;
    if after.id != before.id || after.head_sha != before.head_sha {
        return Err(invalid(
            OP,
            "approval_receipt_mismatch",
            "run changed after approval",
        ));
    }
    let receipt = Receipt {
        provider: "github",
        repository: repo.to_owned(),
        run_id: args.run_id,
        head_sha: before.head_sha,
        pr: args.pr,
        reason,
        prior_status: before.status,
        new_status: after.status,
        outcome: "approved",
    };
    Ok(emit_success(schema, receipt, format, |p| {
        println!("run {}: {} ({})", p.run_id, p.outcome, p.new_status)
    }))
}
