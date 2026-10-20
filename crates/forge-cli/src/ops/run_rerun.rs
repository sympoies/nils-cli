//! One non-replayed GitHub rerun write, fenced by the run and current PR heads.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, RunCommand, RunRerunArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::time::{Duration, Instant};

const OP: &str = "run.rerun";
const READ_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    head_sha: String,
    run_attempt: u64,
    html_url: String,
    event: String,
    status: String,
    pull_requests: Vec<PullRequest>,
}
#[derive(Deserialize)]
struct PullRequest {
    number: u64,
}
#[derive(Serialize, Deserialize)]
struct JobReceipt {
    id: u64,
    name: String,
    html_url: String,
}
#[derive(Serialize)]
struct Receipt {
    provider: &'static str,
    repository: String,
    run_id: u64,
    head_sha: String,
    actor: String,
    selection: &'static str,
    selected_job: Option<u64>,
    previous_attempt: u64,
    rerun_requested: bool,
    receipt_status: &'static str,
    receipt_error: Option<String>,
    attempt: Option<u64>,
    url: String,
    attempt_url: Option<String>,
    jobs: Vec<JobReceipt>,
}

fn call(ctx: &ProviderContext, path: &str, post: bool) -> BackendCall {
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(path.into());
    argv.extend(["--method".into(), if post { "POST" } else { "GET" }.into()]);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn read<T: serde::de::DeserializeOwned>(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    path: &str,
    timeout: Duration,
) -> Result<T, ForgeError> {
    let output = runner.run_with_timeout(&call(ctx, path, false), Some(timeout))?;
    serde_json::from_str(&output.stdout).map_err(|_| {
        invalid(
            OP,
            "rerun_response_invalid",
            "provider returned an invalid rerun response",
        )
    })
}

fn fence(run: &WorkflowRun, args: &RunRerunArgs) -> Result<(), ForgeError> {
    if run.id != args.run_id || !run.head_sha.eq_ignore_ascii_case(&args.expected_head) {
        return Err(invalid(
            OP,
            "run_head_mismatch",
            "run does not match the expected id and head",
        ));
    }
    if run.run_attempt == 0 || run.status != "completed" {
        return Err(invalid(
            OP,
            "run_not_completed",
            "only a completed workflow attempt can be rerun",
        ));
    }
    Ok(())
}

fn pr_fence(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    run: &WorkflowRun,
    repo: &str,
) -> Result<(), ForgeError> {
    let discovered: Vec<PullRequest>;
    let commit_associations =
        run.pull_requests.is_empty() && !run.event.starts_with("pull_request");
    let prs = if commit_associations {
        // Push-triggered checks can also belong to a PR. Discover associations
        // rather than bypass current-head fencing for those workflow runs.
        let mut request = call(
            ctx,
            &format!("repos/{repo}/commits/{}/pulls?per_page=100", run.head_sha),
            false,
        );
        request.argv.extend(["--paginate".into(), "--slurp".into()]);
        let output = runner.run_with_timeout(&request, Some(READ_TIMEOUT))?;
        let pages: Vec<Vec<PullRequest>> = serde_json::from_str(&output.stdout).map_err(|_| {
            invalid(
                OP,
                "rerun_response_invalid",
                "invalid commit PR associations",
            )
        })?;
        discovered = pages.into_iter().flatten().collect();
        &discovered
    } else {
        &run.pull_requests
    };
    // GitHub can omit this association for fork PRs. Refuse rather than treat
    // a PR-triggered run with missing metadata as an unassociated push run.
    if run.event.starts_with("pull_request") && run.pull_requests.is_empty() {
        return Err(invalid(
            OP,
            "run_pr_head_unavailable",
            "current pull request head cannot be established",
        ));
    }
    for pr in prs {
        if pr.number == 0 {
            return Err(invalid(
                OP,
                "rerun_response_invalid",
                "provider returned an invalid PR association",
            ));
        }
        let current: Value = read(
            runner,
            ctx,
            &format!("repos/{repo}/pulls/{}", pr.number),
            READ_TIMEOUT,
        )?;
        // Commit lookup also returns the merged PR that introduced a default-
        // branch commit. Its source head need not equal the merge/squash head.
        // Only these confirmed historical associations bypass current-head fencing.
        let historical_merged = commit_associations
            && current["state"].as_str() == Some("closed")
            && current["merged_at"].as_str().is_some_and(|s| !s.is_empty());
        if current["number"].as_u64() != Some(pr.number)
            || (!historical_merged
                && !current["head"]["sha"]
                    .as_str()
                    .is_some_and(|sha| sha.eq_ignore_ascii_case(&run.head_sha)))
        {
            return Err(invalid(
                OP,
                "pr_head_mismatch",
                "run head is no longer the current pull request head",
            ));
        }
    }
    Ok(())
}

pub fn run(
    global: &GlobalFlags,
    command: RunCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let RunCommand::Rerun(args) = command;
    let ctx = write::target(global, OP)?;
    if args.run_id == 0 || args.job == Some(0) {
        return Err(invalid(
            OP,
            "run_id_invalid",
            "run and job ids must be positive integers",
        ));
    }
    if !matches!(args.expected_head.len(), 40 | 64)
        || !args.expected_head.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid(
            OP,
            "expected_head_invalid",
            "expected head must be a full hexadecimal commit SHA",
        ));
    }
    let repo = ctx.repo.as_deref().expect("target repository");
    let runner = crate::rate_limit::default_runner();
    let run_path = format!("repos/{repo}/actions/runs/{}", args.run_id);
    let before: WorkflowRun = read(&runner, &ctx, &run_path, READ_TIMEOUT)?;
    fence(&before, &args)?;
    if let Some(id) = args.job {
        let job: Value = read(
            &runner,
            &ctx,
            &format!("repos/{repo}/actions/jobs/{id}"),
            READ_TIMEOUT,
        )?;
        if job["id"].as_u64() != Some(id)
            || job["run_id"].as_u64() != Some(args.run_id)
            || job["run_attempt"].as_u64() != Some(before.run_attempt)
            || !job["head_sha"]
                .as_str()
                .is_some_and(|sha| sha.eq_ignore_ascii_case(&before.head_sha))
        {
            return Err(invalid(
                OP,
                "run_job_mismatch",
                "selected job does not belong to this run, head, and attempt",
            ));
        }
    }
    let actor: Value = read(&runner, &ctx, "user", READ_TIMEOUT)?;
    let actor = actor["login"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            invalid(
                OP,
                "rerun_actor_unavailable",
                "authenticated actor is unavailable",
            )
        })?
        .to_owned();
    // Re-read immediately before the write; a concurrent rerun must not make
    // a previously selected job or attempt silently stale.
    let latest: WorkflowRun = read(&runner, &ctx, &run_path, READ_TIMEOUT)?;
    fence(&latest, &args)?;
    if latest.run_attempt != before.run_attempt {
        return Err(invalid(
            OP,
            "run_attempt_changed",
            "workflow attempt changed during preflight",
        ));
    }
    pr_fence(&runner, &ctx, &latest, repo)?;
    let (path, selection) = match args.job {
        Some(job) => (format!("repos/{repo}/actions/jobs/{job}/rerun"), "job"),
        None if args.failed_only => (format!("{run_path}/rerun-failed-jobs"), "failed"),
        None => (format!("{run_path}/rerun"), "all"),
    };
    let mutation = call(&ctx, &path, true);
    let schema = schema_version_for(BINARY, OP, 1);
    if global.dry_run {
        return Ok(emit_success(
            schema,
            DryRunPayload::new(ctx.provider, &mutation),
            format,
            |p| println!("would run: {}", p.plan.join(" ")),
        ));
    }
    // REST writes are not replayed by the GraphQL rate-limit decorator. Even
    // ambiguous provider failures are returned without a second write.
    runner.run_with_timeout(&mutation, Some(READ_TIMEOUT))?;
    let mut receipt = Receipt {
        provider: "github",
        repository: repo.to_owned(),
        run_id: args.run_id,
        head_sha: before.head_sha,
        actor,
        selection,
        selected_job: args.job,
        previous_attempt: before.run_attempt,
        rerun_requested: true,
        receipt_status: "pending",
        receipt_error: None,
        attempt: None,
        url: before.html_url,
        attempt_url: None,
        jobs: Vec::new(),
    };
    observe(&runner, &ctx, &run_path, &args, &mut receipt);
    Ok(emit_success(schema, receipt, format, |p| {
        println!(
            "run {}: rerun requested by {} ({})",
            p.run_id, p.actor, p.receipt_status
        );
        if let Some(attempt) = p.attempt {
            println!("attempt: {attempt}");
        }
        println!("{}", p.attempt_url.as_deref().unwrap_or(&p.url));
        for job in &p.jobs {
            println!("job {}: {} {}", job.id, job.name, job.html_url);
        }
    }))
}

fn observe(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    path: &str,
    args: &RunRerunArgs,
    receipt: &mut Receipt,
) {
    let deadline = Instant::now()
        + Duration::from_secs(if args.receipt_timeout_secs == 0 {
            15
        } else {
            args.receipt_timeout_secs
        });
    loop {
        let result = observe_once(runner, ctx, path, args, receipt, deadline);
        if let Err(error) = result {
            // The write already succeeded. Preserve that fact and never turn a
            // receipt read failure into an invitation to submit another rerun.
            receipt.receipt_error = Some(error.kind().to_owned());
            return;
        }
        if receipt.receipt_status != "pending"
            || args.receipt_timeout_secs == 0
            || Instant::now() >= deadline
        {
            return;
        }
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn observe_once(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    path: &str,
    args: &RunRerunArgs,
    receipt: &mut Receipt,
    deadline: Instant,
) -> Result<(), ForgeError> {
    let remaining = || {
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1))
            .min(READ_TIMEOUT)
    };
    let after: WorkflowRun = read(runner, ctx, path, remaining())?;
    if after.id != receipt.run_id || after.head_sha != receipt.head_sha {
        return Err(invalid(
            OP,
            "rerun_receipt_mismatch",
            "receipt does not match the requested run",
        ));
    }
    if after.run_attempt <= receipt.previous_attempt {
        return Ok(());
    }
    if Some(after.run_attempt) != receipt.previous_attempt.checked_add(1) {
        receipt.receipt_status = "superseded";
        return Ok(());
    }
    let jobs_path = format!("{path}/attempts/{}/jobs?per_page=100", after.run_attempt);
    let mut jobs_call = call(ctx, &jobs_path, false);
    jobs_call
        .argv
        .extend(["--paginate".into(), "--slurp".into()]);
    let output = runner.run_with_timeout(&jobs_call, Some(remaining()))?;
    let pages: Vec<Value> = serde_json::from_str(&output.stdout).map_err(|_| {
        invalid(
            OP,
            "rerun_response_invalid",
            "invalid attempt jobs response",
        )
    })?;
    let mut jobs = Vec::new();
    for page in pages {
        let rows = page["jobs"]
            .as_array()
            .ok_or_else(|| invalid(OP, "rerun_response_invalid", "invalid attempt jobs page"))?;
        for row in rows {
            if row["run_id"].as_u64() != Some(args.run_id)
                || row["run_attempt"].as_u64() != Some(after.run_attempt)
            {
                return Err(invalid(
                    OP,
                    "rerun_receipt_mismatch",
                    "job receipt does not match the new attempt",
                ));
            }
            jobs.push(
                serde_json::from_value::<JobReceipt>(row.clone())
                    .map_err(|_| invalid(OP, "rerun_response_invalid", "invalid job receipt"))?,
            );
        }
    }
    if jobs.is_empty() {
        return Ok(());
    }
    receipt.attempt = Some(after.run_attempt);
    receipt.attempt_url = Some(format!(
        "{}/attempts/{}",
        receipt.url.trim_end_matches('/'),
        after.run_attempt
    ));
    receipt.jobs = jobs;
    receipt.receipt_status = "observed";
    Ok(())
}
