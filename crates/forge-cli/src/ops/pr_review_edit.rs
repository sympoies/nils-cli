//! Update a submitted review owned by the authenticated GitHub identity.
use super::{github_write, pr_review};
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, PrReviewEditArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, canonical_provider_host, detect};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

const OP: &str = "pr.review.edit";
#[derive(Serialize)]
struct ResultPayload {
    provider: &'static str,
    number: u64,
    review_id: u64,
    review_url: String,
    dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_diff: Option<BodyDiff>,
}
#[derive(Serialize)]
struct BodyDiff {
    before: String,
    after: String,
}

pub(super) fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    args: PrReviewEditArgs,
    format: OutputFormat,
    remote: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema(),
            "pr review edit supports GitHub submitted reviews only; GitLab has no equivalent native review",
            None,
        ));
    }
    let repo = ctx
        .repo
        .as_deref()
        .ok_or_else(|| invalid("repository_required", "a resolved repository is required"))?;
    let body = github_write::text_file(&args.body_file, OP)?;
    github_write::guard_text(&body, "review body")?;
    let target = BackendCall::new(
        BackendProgram::Gh,
        pr_review::github_native_review_lookup_argv(&ctx, args.id, args.review),
    );
    let review: serde_json::Value = serde_json::from_str(&runner.run(&target)?.stdout)
        .map_err(|_| invalid("review_invalid", "unable to read target review"))?;
    let expected_url = format!(
        "https://{}/{repo}/pull/{}#pullrequestreview-{}",
        canonical_provider_host(ctx.provider, &ctx.host),
        args.id,
        args.review
    );
    let url = review["html_url"]
        .as_str()
        .filter(|v| v.eq_ignore_ascii_case(&expected_url))
        .ok_or_else(|| {
            invalid(
                "review_invalid",
                "review does not belong to the selected pull request",
            )
        })?;
    if review["id"].as_u64() != Some(args.review)
        || review["state"].as_str().is_none_or(|v| v == "PENDING")
    {
        return Err(invalid(
            "review_invalid",
            "target must be a submitted review",
        ));
    }
    let before = review["body"]
        .as_str()
        .ok_or_else(|| invalid("review_invalid", "review body is missing"))?;
    let author = review
        .pointer("/user/login")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("review_invalid", "review author is missing"))?;
    let mut argv = vec![std::ffi::OsString::from("api")];
    ctx.push_github_api_hostname(&mut argv);
    argv.push("user".into());
    let actor: serde_json::Value = serde_json::from_str(
        &runner
            .run(&BackendCall::new(BackendProgram::Gh, argv))?
            .stdout,
    )
    .map_err(|_| invalid("identity_invalid", "unable to read current identity"))?;
    if !actor["login"]
        .as_str()
        .is_some_and(|v| !v.is_empty() && v.eq_ignore_ascii_case(author))
    {
        return Err(invalid(
            "review_author_mismatch",
            "review author is not the current identity",
        ));
    }
    let review_url = url.to_string();
    if !global.dry_run {
        let input = github_write::payload_file(
            serde_json::json!({"body": body}).to_string().as_bytes(),
            OP,
        )?;
        let mut argv = vec![std::ffi::OsString::from("api")];
        ctx.push_github_api_hostname(&mut argv);
        argv.extend([
            format!("repos/{repo}/pulls/{}/reviews/{}", args.id, args.review).into(),
            "--method".into(),
            "PUT".into(),
            "--input".into(),
            input.path().into(),
        ]);
        let response: serde_json::Value = serde_json::from_str(
            &runner
                .run(&BackendCall::new(BackendProgram::Gh, argv))?
                .stdout,
        )
        .map_err(|_| invalid("review_invalid", "unable to read updated review"))?;
        if response["id"].as_u64() != Some(args.review)
            || response["html_url"].as_str() != Some(url)
        {
            return Err(invalid(
                "review_invalid",
                "updated review response does not match target",
            ));
        }
    }
    let payload = ResultPayload {
        provider: ctx.provider.as_str(),
        number: args.id,
        review_id: args.review,
        review_url,
        dry_run: global.dry_run,
        body_diff: global.dry_run.then(|| BodyDiff {
            before: before.to_string(),
            after: body,
        }),
    };
    Ok(emit_success(schema(), payload, format, |p| {
        println!("review {}: {}", p.review_id, p.review_url);
        if let Some(diff) = &p.body_diff {
            println!("--- before\n{}\n+++ after\n{}", diff.before, diff.after);
        }
    }))
}
fn schema() -> String {
    schema_version_for(BINARY, OP, 1)
}
fn invalid(kind: &'static str, message: &str) -> ForgeError {
    ForgeError::validation(schema(), kind, message, None)
}
