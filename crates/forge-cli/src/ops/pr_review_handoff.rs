//! Designated review ownership uses the existing trusted provider ledger.
//! Session selectors and machine addresses never enter provider-visible state.

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, PrReviewHandoffArgs, PrReviewHandoffCommand};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::{pr_review, pr_reviews, pr_view, review_state};
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;

const SCHEMA: &str = "pr.review-handoff";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewHandoff {
    pub coordinator_digest: String,
    pub reviewer_digest: String,
    pub review_author: String,
    pub base_sha: String,
    pub assigned_head: String,
    pub returned_reason: Option<String>,
}

#[derive(Serialize)]
struct HandoffPayload {
    provider: &'static str,
    number: u64,
    url: String,
    head_sha: String,
    state_tip_digest: Option<String>,
    handoff: Option<ReviewHandoff>,
    status: &'static str,
}

fn schema() -> String {
    schema_version_for(BINARY, SCHEMA, 1)
}
fn fail(kind: &'static str, message: &str) -> ForgeError {
    ForgeError::validation(
        schema(),
        kind,
        message,
        Some("recovery=return control to the coordinator; do not self-review".into()),
    )
}

/// Hash only the globally unique session id, never its machine suffix.
fn session_digest(selector: &str) -> Result<String, ForgeError> {
    let id = selector.split('@').next().unwrap_or("");
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(fail(
            "review_assignment_invalid",
            "a non-empty bounded reviewer session identity is required",
        ));
    }
    Ok(review_state::sha256_digest(id.as_bytes()))
}

fn actor_digest() -> Result<String, ForgeError> {
    session_digest(&std::env::var("AGENT_SESSION_ID").unwrap_or_default())
}

fn assignment_digest() -> Result<Option<String>, ForgeError> {
    std::env::var("AGENT_REVIEWER_SESSION")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| session_digest(&v))
        .transpose()
}

pub(crate) fn validate_handoff(h: &ReviewHandoff) -> Result<(), ForgeError> {
    fn digest_valid(s: &str) -> bool {
        s.strip_prefix("sha256:")
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|c| c.is_ascii_hexdigit()))
    }
    if !digest_valid(&h.coordinator_digest)
        || !digest_valid(&h.reviewer_digest)
        || h.coordinator_digest == h.reviewer_digest
        || !valid_sha(&h.base_sha)
        || !valid_sha(&h.assigned_head)
        || h.review_author.is_empty()
        || h.review_author.len() > 128
        || !h
            .review_author
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_[].".contains(&c))
        || h.returned_reason.as_deref().is_some_and(|r| {
            !matches!(
                r,
                "reviewer-unreachable"
                    | "reviewer-closed"
                    | "reviewer-declined"
                    | "reviewer-timeout"
            )
        })
    {
        return Err(fail(
            "review_assignment_invalid",
            "persisted reviewer handoff is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_provider(ctx: &ProviderContext) -> Result<(), ForgeError> {
    if assignment_digest()?.is_some() && ctx.provider != Provider::GitHub {
        return Err(fail(
            "designated_review_provider_unsupported",
            "designated review requires the GitHub provider ledger",
        ));
    }
    Ok(())
}

pub(crate) fn latest(chain: &review_state::ReviewStateChain) -> Option<&ReviewHandoff> {
    chain
        .records
        .iter()
        .rev()
        .find_map(|record| match &record.payload {
            review_state::ReviewStatePayload::ReviewHandoff { handoff } => Some(handoff),
            _ => None,
        })
}

pub(crate) fn is_assigned(chain: &review_state::ReviewStateChain) -> bool {
    latest(chain).is_some() || std::env::var("AGENT_REVIEWER_SESSION").is_ok_and(|v| !v.is_empty())
}

fn assigned(chain: &review_state::ReviewStateChain) -> Result<Option<&ReviewHandoff>, ForgeError> {
    let requested = assignment_digest()?;
    let Some(handoff) = latest(chain) else {
        return if requested.is_some() {
            Err(fail(
                "awaiting_designated_review",
                "awaiting designated review: explicit handoff is missing",
            ))
        } else {
            Ok(None)
        };
    };
    if requested
        .as_ref()
        .is_some_and(|v| v != &handoff.reviewer_digest)
    {
        return Err(fail(
            "review_assignment_conflict",
            "reviewer assignment differs from the persisted handoff",
        ));
    }
    if handoff.returned_reason.is_some() {
        return Err(ForgeError::unavailable(
            schema(),
            "designated_reviewer_unavailable",
            "designated reviewer unavailable; control returned to coordinator",
            None,
        ));
    }
    Ok(Some(handoff))
}

fn owned_state(chain: &review_state::ReviewStateChain) -> Option<&review_state::ReviewLoopState> {
    chain
        .records
        .iter()
        .rev()
        .take_while(|r| {
            !matches!(
                r.payload,
                review_state::ReviewStatePayload::ReviewHandoff { .. }
            )
        })
        .find_map(|r| match &r.payload {
            review_state::ReviewStatePayload::ReviewLoop { state } => Some(state),
            _ => None,
        })
}

pub(crate) fn ensure_observation(
    chain: &review_state::ReviewStateChain,
    head: &str,
) -> Result<(), ForgeError> {
    ensure_writer(chain)?;
    if let Some(h) = assigned(chain)?
        && owned_state(chain).is_none()
        && h.assigned_head != head
    {
        return Err(fail(
            "review_repair_unobserved",
            "a repair was pushed before the designated reviewer observed findings at the handed-off head",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_writer(chain: &review_state::ReviewStateChain) -> Result<(), ForgeError> {
    let Some(handoff) = assigned(chain)? else {
        return Ok(());
    };
    if actor_digest()? != handoff.reviewer_digest {
        return Err(fail(
            "review_writer_conflict",
            "only the designated reviewer session may append or publish this review",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_published<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    url: &str,
    head: &str,
    chain: &review_state::ReviewStateChain,
    handoff_created_at: Option<&str>,
) -> Result<(), ForgeError> {
    let Some(handoff) = assigned(chain)? else {
        return Ok(());
    };
    let state = owned_state(chain).ok_or_else(|| {
        fail(
            "awaiting_designated_review",
            "awaiting designated review: reviewed-head ledger observation is missing",
        )
    })?;
    if state.head_sha != head
        || state.hard_stop.is_some()
        || state
            .findings
            .values()
            .any(|f| f.blocking && f.status == review_state::ReviewFindingStatus::Open)
    {
        return Err(fail(
            "awaiting_designated_review",
            "awaiting designated review: ledger is stale or has unresolved findings",
        ));
    }
    let reviews = pr_reviews::compute_for_pr(runner, ctx, number, url)?;
    if reviews.head_sha != head {
        return Err(fail(
            "review_convergence_head_changed",
            "provider head changed during designated review admission",
        ));
    }
    let after = handoff_created_at
        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        .ok_or_else(|| {
            fail(
                "review_snapshot_incomplete",
                "handoff publication timestamp is missing or invalid",
            )
        })?;
    let review = reviews
        .current_head_reviews
        .iter()
        .filter(|r| r.author == handoff.review_author && r.commit_sha == head)
        .max_by_key(|r| (&r.submitted_at, r.database_id));
    let passing = if let Some(r) = review {
        let body = if r.summary_truncated {
            read_complete_report(runner, ctx, number, r)?
        } else {
            r.summary.clone()
        };
        pr_review::validate_specialist_review_report(&body).is_ok()
            && matches!(r.state.as_str(), "APPROVED" | "COMMENTED")
            && r.submitted_at
                .parse::<jiff::Timestamp>()
                .is_ok_and(|t| t > after)
            && body.lines().any(|l| {
                matches!(
                    l.trim(),
                    "- Lens verdict: pass" | "- Lens verdict: follow-up-pass"
                )
            })
    } else {
        false
    };
    if !passing {
        return Err(fail(
            "awaiting_designated_review",
            "awaiting designated review: published passing review for the current head is missing",
        ));
    }
    Ok(())
}

fn read_complete_report<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    selected: &pr_reviews::NativeReviewSummary,
) -> Result<String, ForgeError> {
    let id = selected
        .database_id
        .ok_or_else(|| fail("review_snapshot_incomplete", "native review id is missing"))?;
    let call = BackendCall::new(
        BackendProgram::Gh,
        pr_review::github_native_review_lookup_argv(ctx, number, id),
    );
    let output = runner.run(&call)?;
    let value: serde_json::Value = serde_json::from_str(&output.stdout).map_err(|_| {
        fail(
            "review_snapshot_incomplete",
            "native report read-back is invalid",
        )
    })?;
    if value["id"].as_u64() != Some(id)
        || value["html_url"].as_str() != Some(selected.url.as_str())
        || value["commit_id"].as_str() != Some(selected.commit_sha.as_str())
        || value["user"]["login"].as_str() != Some(selected.author.as_str())
        || value["state"].as_str() != Some(selected.state.as_str())
    {
        return Err(fail(
            "review_snapshot_incomplete",
            "native report changed during read-back",
        ));
    }
    let body = value["body"]
        .as_str()
        .filter(|v| v.len() <= 64 * 1024)
        .ok_or_else(|| {
            fail(
                "review_snapshot_incomplete",
                "native report body is missing or exceeds its bound",
            )
        })?;
    Ok(body.to_string())
}

pub fn run(
    global: &GlobalFlags,
    args: PrReviewHandoffArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(fail(
            "designated_review_provider_unsupported",
            "designated review requires the GitHub provider ledger",
        ));
    }
    let id = match &args.command {
        PrReviewHandoffCommand::Assign(a) => a.id,
        PrReviewHandoffCommand::Inspect(a) => a.id,
        PrReviewHandoffCommand::Check(a) => a.id,
        PrReviewHandoffCommand::Return(a) => a.id,
    };
    let view = pr_view::compute(&runner, &ctx, id)?;
    let repository = super::pr_comments::github_repo_slug_from_url(&view.url).ok_or_else(|| {
        fail(
            "review_assignment_invalid",
            "reviewable repository is missing",
        )
    })?;
    let head = view
        .head_sha
        .as_deref()
        .ok_or_else(|| fail("review_assignment_invalid", "provider head is missing"))?;
    let state = pr_review::read_review_loop_state_view(&runner, &ctx, &repository, id)?;
    let mut chain = state.chain;
    let mut status = match latest(&chain) {
        Some(h) if h.returned_reason.is_some() => "returned-to-coordinator",
        Some(_) => "awaiting-designated-review",
        None => "unassigned",
    };
    let mut proposed = None;
    match &args.command {
        PrReviewHandoffCommand::Inspect(_) => (),
        PrReviewHandoffCommand::Check(a) => {
            require_head(head, &a.expected_head)?;
            if latest(&chain).is_none() && assignment_digest()?.is_none() {
                return Err(fail(
                    "review_assignment_missing",
                    "no designated reviewer handoff exists",
                ));
            }
            ensure_published(
                &runner,
                &ctx,
                id,
                &view.url,
                head,
                &chain,
                state.handoff_created_at.as_deref(),
            )?;
            status = "reviewed";
        }
        PrReviewHandoffCommand::Assign(a) => {
            require_head(head, &a.expected_head)?;
            require_tip(chain.tip_digest.as_deref(), &a.expected_state)?;
            let coordinator = actor_digest()?;
            if latest(&chain).is_some_and(|h| h.coordinator_digest != coordinator) {
                return Err(fail(
                    "review_writer_conflict",
                    "only the handoff coordinator may reassign review ownership",
                ));
            }
            if !valid_sha(&a.base_sha)
                || a.review_author.is_empty()
                || a.review_author.len() > 128
                || !a
                    .review_author
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_[].".contains(&c))
            {
                return Err(fail(
                    "review_assignment_invalid",
                    "base SHA or public review author is invalid",
                ));
            }
            let reviewer = session_digest(&a.reviewer_session)?;
            if reviewer == coordinator {
                return Err(fail(
                    "review_assignment_invalid",
                    "designated reviewer must be a different session from the worker",
                ));
            }
            proposed = Some(ReviewHandoff {
                coordinator_digest: coordinator,
                reviewer_digest: reviewer,
                review_author: a.review_author.clone(),
                base_sha: a.base_sha.clone(),
                assigned_head: head.to_string(),
                returned_reason: None,
            });
        }
        PrReviewHandoffCommand::Return(a) => {
            require_head(head, &a.expected_head)?;
            require_tip(chain.tip_digest.as_deref(), &a.expected_state)?;
            let mut h = latest(&chain)
                .cloned()
                .ok_or_else(|| fail("review_assignment_missing", "no handoff to return"))?;
            if actor_digest()? != h.coordinator_digest {
                return Err(fail(
                    "review_writer_conflict",
                    "only the handoff coordinator may return review control",
                ));
            }
            h.returned_reason = Some(a.reason.clone());
            proposed = Some(h);
            status = "returned-to-coordinator";
        }
    }
    if let Some(handoff) = proposed {
        validate_handoff(&handoff)?;
        status = if handoff.returned_reason.is_some() {
            "returned-to-coordinator"
        } else {
            "awaiting-designated-review"
        };
        if !global.dry_run {
            chain = pr_review::append_review_state_payload(
                &runner,
                &ctx,
                pr_review::ReviewStateAppend {
                    repository: &repository,
                    number: id,
                    expected_head: head,
                    expected_tip: chain.tip_digest.as_deref(),
                    payload: review_state::ReviewStatePayload::ReviewHandoff { handoff },
                    visible_outcome: None,
                },
            )?
            .chain;
        } else {
            // Read-only previews report the prospective handoff with the real tip.
            let record = review_state::ReviewStateRecord::new(
                &repository,
                id,
                head,
                chain.records.len() as u64,
                chain.tip_digest.clone(),
                review_state::ReviewStatePayload::ReviewHandoff { handoff },
            )?;
            chain.records.push(record);
        }
    }
    let payload = HandoffPayload {
        provider: ctx.provider.as_str(),
        number: id,
        url: view.url,
        head_sha: head.to_string(),
        state_tip_digest: chain.tip_digest.clone(),
        handoff: latest(&chain).cloned(),
        status,
    };
    Ok(emit_success(schema(), payload, format, |p| {
        println!("{}: {}", p.status, p.url)
    }))
}

fn valid_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn require_head(actual: &str, expected: &str) -> Result<(), ForgeError> {
    if !valid_sha(expected) || actual != expected {
        return Err(fail(
            "review_state_conflict",
            "provider head differs from the handoff head",
        ));
    }
    Ok(())
}
fn require_tip(actual: Option<&str>, expected: &str) -> Result<(), ForgeError> {
    if actual
        != if expected == "none" {
            None
        } else {
            Some(expected)
        }
    {
        return Err(fail(
            "review_state_conflict",
            "ledger tip differs from the explicit handoff tip",
        ));
    }
    Ok(())
}
