//! `pr review-dismiss` — governed dismissal of one automated
//! CHANGES_REQUESTED review.
//!
//! An automated review App can keep blocking a head whose finding has already
//! been dispositioned. Dismissal is admitted only for a provider App account
//! that is explicitly configured in `[[review_convergence.bots]]`, on the
//! expected head, and only once the designated reviewer has published a PASS
//! at that head. It never replaces designated review: the designated
//! reviewer's own review cannot be dismissed, and the designated reviewer
//! session cannot dismiss. GitHub-only; GitLab has no review dismissal.

use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, PrReviewDismissArgs};
use crate::config::ReviewConvergenceBot;
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::github_write::text_file;
use crate::ops::{pr_merge, pr_review, pr_review_handoff, pr_reviews, pr_view};
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::validations::{no_agent_attribution, no_local_path};

const SCHEMA: &str = "pr.review-dismiss";
const SCHEMA_VERSION: u32 = 1;

/// Envelope payload for `cli.forge-cli.pr.review-dismiss.v1`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrReviewDismissPayload {
    pub provider: &'static str,
    pub number: u64,
    pub url: String,
    pub review_id: u64,
    pub review_url: String,
    pub author: String,
    pub head_sha: String,
    pub state: String,
    pub reason: String,
    pub actor: String,
    /// When forge-cli observed the provider confirm the dismissal.
    pub dismissed_at: String,
}

#[derive(Serialize)]
struct DismissDryRunPayload {
    #[serde(flatten)]
    mutation: DryRunPayload,
    number: u64,
    review_id: u64,
    review_url: String,
    author: String,
    head_sha: String,
    actor: String,
}

/// The target review as read back from the provider REST endpoint.
struct TargetReview {
    url: String,
    author: String,
    author_type: Option<String>,
    author_node_id: Option<String>,
    state: String,
    commit_sha: String,
}

pub fn run(
    global: &GlobalFlags,
    args: PrReviewDismissArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    run_with(&runner, global, args, format, &workdir, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    args: PrReviewDismissArgs,
    format: OutputFormat,
    workdir: &std::path::Path,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema(),
            format!(
                "pr review-dismiss is GitHub-only (provider: {}); GitLab has no review dismissal",
                provider_label(ctx.provider)
            ),
            None,
        ));
    }
    let reason = text_file(&args.reason_file, SCHEMA)?.trim().to_string();
    if reason.is_empty() {
        return Err(refuse(
            "review_dismiss_reason_missing",
            "a non-empty dismissal reason is required (--reason-file)",
        ));
    }
    no_local_path(&reason, "dismissal reason")?;
    no_agent_attribution(&reason, "dismissal reason")?;
    let bots = pr_merge::resolve_review_convergence_for_workdir(workdir, None, ctx.provider)?.bots;

    let view = pr_view::compute(runner, &ctx, args.id)?;
    let head = args.expected_head.as_str();
    if view.head_sha.as_deref() != Some(head) {
        return Err(refuse(
            "review_dismiss_head_mismatch",
            "provider head differs from --expected-head",
        ));
    }
    let target = read_target(runner, &ctx, args.id, args.review_id)?;
    if target.state != "CHANGES_REQUESTED" {
        return Err(refuse(
            "review_dismiss_state_invalid",
            &format!(
                "only a CHANGES_REQUESTED review can be dismissed (state: {})",
                target.state
            ),
        ));
    }
    if target.commit_sha != head {
        return Err(refuse(
            "review_dismiss_head_mismatch",
            "the review is not on --expected-head",
        ));
    }
    if !is_configured_automated(target.author_type.as_deref(), &target.author, &bots) {
        return Err(refuse(
            "review_dismiss_author_not_automated",
            "the review author is not a configured automated-review App account",
        ));
    }

    let repository = super::pr_comments::github_repo_slug_from_url(&view.url)
        .ok_or_else(|| refuse("repo_required", "review dismissal requires a repository"))?;
    let state = pr_review::read_review_loop_state_view(runner, &ctx, &repository, args.id)?;
    let Some(handoff) = pr_review_handoff::latest(&state.chain) else {
        return Err(refuse(
            "awaiting_designated_review",
            "review dismissal requires a designated-review PASS at --expected-head",
        ));
    };
    if pr_review_handoff::matches_designated_author(&target.summary(), &handoff.review_author) {
        return Err(refuse(
            "review_dismiss_designated_reviewer",
            "the designated reviewer's own review cannot be dismissed",
        ));
    }
    let actor_session = std::env::var("AGENT_SESSION_ID").unwrap_or_default();
    if !actor_session.is_empty()
        && pr_review_handoff::session_digest(&actor_session)
            .is_ok_and(|digest| digest == handoff.reviewer_digest)
    {
        return Err(refuse(
            "review_dismiss_self_review",
            "the designated reviewer session cannot dismiss reviews on the head it reviews",
        ));
    }
    pr_review_handoff::ensure_published(
        runner,
        &ctx,
        args.id,
        &view.url,
        head,
        &state.chain,
        state.handoff_created_at.as_deref(),
    )?;

    let actor = read_actor(runner, &ctx)?;
    let message = format!(
        "{reason}\n\nDismissed with forge-cli pr review-dismiss at head {head} by {actor}."
    );
    let call = build_dismiss_call(&ctx, &repository, args.id, args.review_id, &message);
    if global.dry_run {
        let payload = DismissDryRunPayload {
            mutation: DryRunPayload::new(ctx.provider, &call),
            number: args.id,
            review_id: args.review_id,
            review_url: target.url,
            author: target.author,
            head_sha: head.to_string(),
            actor,
        };
        return Ok(emit_success(schema(), payload, format, |p| {
            println!("would run: {}", p.mutation.plan.join(" "))
        }));
    }

    let output = runner.run(&call)?;
    let confirmed = serde_json::from_str::<serde_json::Value>(&output.stdout)
        .ok()
        .filter(|v| v["id"].as_u64() == Some(args.review_id) && v["state"] == "DISMISSED");
    if confirmed.is_none() {
        return Err(ForgeError::unavailable(
            schema(),
            "review_dismiss_unconfirmed",
            "the provider did not confirm the dismissal; inspect the review before retrying",
            Some(format!("review_id={}", args.review_id)),
        ));
    }
    let payload = PrReviewDismissPayload {
        provider: ctx.provider.as_str(),
        number: args.id,
        url: view.url,
        review_id: args.review_id,
        review_url: target.url,
        author: target.author,
        head_sha: head.to_string(),
        state: "DISMISSED".to_string(),
        reason,
        actor,
        dismissed_at: jiff::Timestamp::now().to_string(),
    };
    Ok(emit_success(schema(), payload, format, |p| {
        println!(
            "dismissed review {} by {} at {} ({})",
            p.review_id, p.author, p.head_sha, p.review_url
        )
    }))
}

/// Configuration is the only authority: a provider App account is automated
/// only when its login is listed, and no login is automated by its spelling.
fn is_configured_automated(
    author_type: Option<&str>,
    author: &str,
    bots: &[ReviewConvergenceBot],
) -> bool {
    fn app_name(login: &str) -> String {
        let login = login.trim().to_ascii_lowercase();
        login.strip_suffix("[bot]").unwrap_or(&login).to_string()
    }
    author_type == Some("Bot")
        && bots
            .iter()
            .any(|bot| app_name(&bot.login) == app_name(author))
}

fn read_target<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    review_id: u64,
) -> Result<TargetReview, ForgeError> {
    let call = BackendCall::new(
        BackendProgram::Gh,
        pr_review::github_native_review_lookup_argv(ctx, number, review_id),
    );
    let output = runner.run(&call)?;
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).map_err(|_| incomplete())?;
    let text = |pointer: &str| value.pointer(pointer).and_then(|v| v.as_str());
    if value["id"].as_u64() != Some(review_id) {
        return Err(incomplete());
    }
    Ok(TargetReview {
        url: text("/html_url").ok_or_else(incomplete)?.to_string(),
        author: text("/user/login").ok_or_else(incomplete)?.to_string(),
        author_type: text("/user/type").map(str::to_string),
        author_node_id: text("/user/node_id").map(str::to_string),
        state: text("/state").ok_or_else(incomplete)?.to_string(),
        commit_sha: text("/commit_id").ok_or_else(incomplete)?.to_string(),
    })
}

impl TargetReview {
    fn summary(&self) -> pr_reviews::NativeReviewSummary {
        pr_reviews::NativeReviewSummary {
            id: String::new(),
            database_id: None,
            url: self.url.clone(),
            author: self.author.clone(),
            author_node_id: self.author_node_id.clone(),
            author_type: self.author_type.clone(),
            state: self.state.clone(),
            commit_sha: self.commit_sha.clone(),
            submitted_at: String::new(),
            summary: String::new(),
            summary_truncated: false,
        }
    }
}

fn read_actor<R: BackendRunner>(runner: &R, ctx: &ProviderContext) -> Result<String, ForgeError> {
    let mut argv = vec![OsString::from("api")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend(["user", "--jq", ".login"].map(OsString::from));
    let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    let login = output.stdout.trim();
    if login.is_empty() {
        return Err(ForgeError::unavailable(
            schema(),
            "review_dismiss_actor_unknown",
            "the authenticated provider actor could not be read",
            None,
        ));
    }
    Ok(login.to_string())
}

fn build_dismiss_call(
    ctx: &ProviderContext,
    repository: &str,
    number: u64,
    review_id: u64,
    message: &str,
) -> BackendCall {
    let mut argv = vec![OsString::from("api")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        OsString::from("--method"),
        OsString::from("PUT"),
        OsString::from(format!(
            "repos/{repository}/pulls/{number}/reviews/{review_id}/dismissals"
        )),
        OsString::from("-f"),
        OsString::from(format!("message={message}")),
        OsString::from("-f"),
        OsString::from("event=DISMISS"),
    ]);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::GitHub => "GitHub",
        Provider::GitLab => "GitLab",
        Provider::Local => "local",
    }
}

fn schema() -> String {
    schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION)
}

fn refuse(kind: &'static str, message: &str) -> ForgeError {
    ForgeError::validation(schema(), kind, message, None)
}

fn incomplete() -> ForgeError {
    ForgeError::unavailable(
        schema(),
        "review_snapshot_incomplete",
        "the review read-back is missing or invalid",
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReviewBotMode;
    use pretty_assertions::assert_eq;

    fn bots(logins: &[&str]) -> Vec<ReviewConvergenceBot> {
        logins
            .iter()
            .map(|login| ReviewConvergenceBot {
                login: (*login).to_string(),
                mode: ReviewBotMode::Observed,
            })
            .collect()
    }

    #[test]
    fn automation_requires_both_an_app_account_and_configuration() {
        let configured = bots(&["lint-app"]);
        assert_eq!(
            is_configured_automated(Some("Bot"), "lint-app[bot]", &configured),
            true
        );
        assert_eq!(
            is_configured_automated(Some("Bot"), "LINT-APP", &configured),
            true
        );
        assert_eq!(
            is_configured_automated(Some("User"), "lint-app", &configured),
            false
        );
        assert_eq!(
            is_configured_automated(None, "lint-app", &configured),
            false
        );
        assert_eq!(
            is_configured_automated(Some("Bot"), "other-app[bot]", &configured),
            false
        );
        assert_eq!(
            is_configured_automated(Some("Bot"), "lint-app[bot]", &[]),
            false
        );
    }

    #[test]
    fn dismissal_call_targets_the_exact_review_with_the_message() {
        let ctx = ProviderContext {
            provider: Provider::GitHub,
            host: "github.com".into(),
            source: crate::provider::DetectionSource::Remote,
            repo: Some("acme/widgets".into()),
        };
        let plan = build_dismiss_call(&ctx, "acme/widgets", 7, 5, "why").plan_argv();
        assert_eq!(
            plan[1..],
            [
                "api",
                "--method",
                "PUT",
                "repos/acme/widgets/pulls/7/reviews/5/dismissals",
                "-f",
                "message=why",
                "-f",
                "event=DISMISS",
            ]
        );
    }
}
