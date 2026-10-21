//! Complete review bodies bound to one repository, PR and native database ID.
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::BINARY;
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{ProviderContext, canonical_provider_host};

use super::{pr_review, pr_reviews, pr_view};

const MAX_COMPLETE_BODY_BYTES: usize = 64 * 1024;

#[derive(Serialize)]
struct CompleteReview {
    provider: &'static str,
    repository: String,
    number: u64,
    pr_url: String,
    head_sha: String,
    viewer_login: String,
    id: String,
    database_id: u64,
    url: String,
    author: String,
    commit_sha: Option<String>,
    state: String,
    submitted_at: Option<String>,
    body: String,
    body_bytes: usize,
    body_truncated: bool,
    complete: bool,
    observed_at: String,
}

struct ReviewBinding<'a> {
    id: &'a str,
    database_id: u64,
    url: &'a str,
    author: &'a str,
    author_node_id: Option<&'a str>,
    author_type: Option<&'a str>,
    commit_sha: Option<&'a str>,
    state: &'a str,
    submitted_at: Option<&'a str>,
}

pub(crate) fn run_with<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repo: &str,
    number: u64,
    review_id: u64,
    format: OutputFormat,
    dry_run: bool,
) -> Result<i32, ForgeError> {
    let mut ctx = ctx.clone();
    ctx.repo = Some(repo.to_string());
    let call = BackendCall::new(
        BackendProgram::Gh,
        pr_review::github_native_review_lookup_argv(&ctx, number, review_id),
    );
    if dry_run {
        return Ok(emit_success(
            schema(),
            DryRunPayload::new(ctx.provider, &call),
            format,
            |p| {
                println!("would run: {}", p.plan.join(" "));
            },
        ));
    }

    let view = pr_view::parse_view_output(
        &ctx,
        &runner.run(&pr_view::build_view_call(&ctx, &number.to_string()))?,
    )?;
    let expected_url = format!(
        "https://{}/{repo}/pull/{number}",
        canonical_provider_host(ctx.provider, &ctx.host)
    );
    if view.number != number || !view.url.eq_ignore_ascii_case(&expected_url) {
        return Err(incomplete(
            "native review repository or pull request binding changed",
        ));
    }
    let snapshot = pr_reviews::compute_for_pr(runner, &ctx, number, &view.url)?;
    if view.head_sha.as_deref() != Some(snapshot.head_sha.as_str()) {
        return Err(incomplete(
            "the pull request head changed during the review read",
        ));
    }
    let selected = select_review(&snapshot, review_id)?;
    let expected_review_url = format!("{}#pullrequestreview-{review_id}", view.url);
    if !selected.url.eq_ignore_ascii_case(&expected_review_url) {
        return Err(incomplete(
            "native review URL does not belong to the selected pull request",
        ));
    }
    let body = read_bound_body(runner, &ctx, number, &selected, &incomplete)?;
    let payload = CompleteReview {
        provider: ctx.provider.as_str(),
        repository: repo.to_string(),
        number,
        pr_url: view.url,
        head_sha: snapshot.head_sha.clone(),
        viewer_login: snapshot.viewer_login.clone(),
        id: selected.id.to_string(),
        database_id: review_id,
        url: selected.url.to_string(),
        author: selected.author.to_string(),
        commit_sha: selected.commit_sha.map(str::to_string),
        state: selected.state.to_string(),
        submitted_at: selected.submitted_at.map(str::to_string),
        body_bytes: body.len(),
        body,
        body_truncated: false,
        complete: true,
        observed_at: jiff::Timestamp::now().to_string(),
    };
    Ok(emit_success(schema(), payload, format, |p| {
        println!(
            "review {} by {} ({})\n{}\n\n{}",
            p.database_id, p.author, p.state, p.url, p.body
        );
    }))
}

fn select_review(
    snapshot: &pr_reviews::PrReviewsPayload,
    id: u64,
) -> Result<ReviewBinding<'_>, ForgeError> {
    if let Some(review) = snapshot
        .current_head_reviews
        .iter()
        .chain(&snapshot.stale_reviews)
        .find(|review| review.database_id == Some(id))
    {
        return Ok(submitted_binding(review, id, &review.author));
    }
    if let Some(review) = snapshot
        .pending_reviews
        .iter()
        .find(|review| review.database_id == Some(id))
    {
        return Ok(ReviewBinding {
            id: &review.id,
            database_id: id,
            url: &review.url,
            author: &review.author,
            author_node_id: None,
            author_type: None,
            commit_sha: review.commit_sha.as_deref(),
            state: &review.state,
            submitted_at: None,
        });
    }
    Err(ForgeError::validation(
        schema(),
        "review_not_found",
        "native review database ID was not found on the selected pull request",
        None,
    ))
}

fn submitted_binding<'a>(
    review: &'a pr_reviews::NativeReviewSummary,
    id: u64,
    author: &'a str,
) -> ReviewBinding<'a> {
    ReviewBinding {
        id: &review.id,
        database_id: id,
        url: &review.url,
        author,
        author_node_id: review.author_node_id.as_deref(),
        author_type: review.author_type.as_deref(),
        commit_sha: Some(&review.commit_sha),
        state: &review.state,
        submitted_at: Some(&review.submitted_at),
    }
}

/// Shared with designated handoff admission; its caller retains its own error/recovery contract.
pub(crate) fn read_complete_report<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    selected: &pr_reviews::NativeReviewSummary,
    expected_author: &str,
    failure: impl Fn(&str) -> ForgeError,
) -> Result<String, ForgeError> {
    let id = selected
        .database_id
        .ok_or_else(|| failure("native review id is missing"))?;
    read_bound_body(
        runner,
        ctx,
        number,
        &submitted_binding(selected, id, expected_author),
        &failure,
    )
}

fn read_bound_body<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    selected: &ReviewBinding<'_>,
    failure: &impl Fn(&str) -> ForgeError,
) -> Result<String, ForgeError> {
    let call = BackendCall::new(
        BackendProgram::Gh,
        pr_review::github_native_review_lookup_argv(ctx, number, selected.database_id),
    );
    let output = runner.run(&call)?;
    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|_| failure("native report read-back is invalid"))?;
    let rest_author = &value["user"];
    let same_node = selected
        .author_node_id
        .is_some_and(|node| rest_author["node_id"].as_str() == Some(node));
    let identity_matches = if selected.author_type == Some("Bot") {
        rest_author["type"].as_str() == Some("Bot") && same_node
    } else {
        selected.author_node_id.is_none() || same_node
    };
    if value["id"].as_u64() != Some(selected.database_id)
        || value["html_url"].as_str() != Some(selected.url)
        || value["commit_id"].as_str() != selected.commit_sha
        || !rest_author["login"]
            .as_str()
            .is_some_and(|login| login.eq_ignore_ascii_case(selected.author))
        || !identity_matches
        || selected.author_type.is_some_and(|kind| {
            rest_author["type"]
                .as_str()
                .is_some_and(|rest| rest != kind)
        })
        || value["state"].as_str() != Some(selected.state)
    {
        return Err(failure("native report changed during read-back"));
    }
    value["body"]
        .as_str()
        .filter(|body| body.len() <= MAX_COMPLETE_BODY_BYTES)
        .map(str::to_string)
        .ok_or_else(|| failure("native report body is missing or exceeds its bound"))
}

fn schema() -> String {
    schema_version_for(BINARY, "pr.review-get", 1)
}

fn incomplete(message: &str) -> ForgeError {
    ForgeError::validation(schema(), "review_snapshot_incomplete", message, None)
}
