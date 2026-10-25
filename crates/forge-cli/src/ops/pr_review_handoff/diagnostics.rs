//! Retired-coordinator takeover refusals. Each names the exact missing
//! prerequisite, whether a retry can help, and the supported remedy. These
//! constructors only describe a refusal; the guards that raise it stay in
//! `policy` and `prove_coordinator_retired`.
use super::schema;
use crate::error::ForgeError;

/// `review_coordinator_unproven` with a stable `details.prerequisite`.
pub(super) fn unproven(
    prerequisite: &'static str,
    retryable: bool,
    next_action: &'static str,
    message: impl Into<String>,
) -> ForgeError {
    ForgeError::validation_with_details(
        schema(),
        "review_coordinator_unproven",
        message,
        serde_json::json!({
            "retryable": retryable,
            "next_action": next_action,
            "prerequisite": prerequisite,
            "recovery": {
                "reason": "coordinator-retired",
                "route": "forge-cli pr review-handoff recover",
                "requires_repository_supervisor": true,
                "requires_stale_terminal_activity": true,
                "requires_exact_head_base_tip": true
            }
        }),
    )
}

pub(super) fn board_unavailable() -> ForgeError {
    unproven(
        "session_board_unavailable",
        true,
        "refresh_session_evidence",
        "the session board could not be read (agent-session board failed, timed out or returned an \
         unrecognized response); check that agent-session is installed (FORGE_CLI_AGENT_SESSION_BIN) \
         and retry",
    )
}

pub(super) fn board_incomplete() -> ForgeError {
    unproven(
        "session_board_incomplete",
        true,
        "refresh_session_evidence",
        "the session board snapshot is truncated or older than 60 seconds; retry for a complete, \
         fresh board",
    )
}

pub(super) fn coordinator_not_on_board() -> ForgeError {
    unproven(
        "coordinator_not_on_board",
        false,
        "request_lifecycle_evidence",
        "the recorded coordinator has no row on the session board, so its retirement cannot be \
         proven; `agent-session board --state all --format json` must return a retained stopped \
         or closed row for it, which only the session-board operator can supply",
    )
}

pub(super) fn coordinator_source_ambiguous() -> ForgeError {
    unproven(
        "coordinator_source_ambiguous",
        false,
        "request_lifecycle_evidence",
        "the recorded coordinator does not resolve to exactly one selected source machine on the \
         session board; the session-board operator must remove the ambiguity",
    )
}

pub(super) fn lifecycle_source_unavailable() -> ForgeError {
    unproven(
        "lifecycle_source_unavailable",
        true,
        "refresh_session_evidence",
        "the source machine that hosts the recorded coordinator is offline or was not seen \
         recently; retry once it reports to the session board",
    )
}

pub(super) fn coordinator_not_terminal() -> ForgeError {
    unproven(
        "coordinator_not_terminal",
        false,
        "return_to_coordinator",
        "the recorded coordinator is neither stopped nor closed as deleted or archived; only a \
         retired coordinator can be taken over",
    )
}

pub(super) fn terminal_activity_missing() -> ForgeError {
    unproven(
        "terminal_activity_missing",
        false,
        "request_lifecycle_evidence",
        "the coordinator's retained board row has no last_terminal_activity_at, so staleness \
         cannot be proven; waiting cannot create it and no substitute or caller-supplied timestamp \
         is accepted. `agent-session board --state all --format json` must return that field for \
         this row, which only the session-board operator can supply",
    )
}

pub(super) fn coordinator_not_stale(stale_after_seconds: u64) -> ForgeError {
    unproven(
        "coordinator_not_stale",
        true,
        "refresh_session_evidence",
        format!(
            "the recorded coordinator's last terminal activity is more recent than the configured \
             stale_after ({stale_after_seconds}s); retry after that interval has elapsed"
        ),
    )
}

pub(super) fn checkout_unavailable() -> ForgeError {
    unproven(
        "checkout_unavailable",
        false,
        "run_from_repository_checkout",
        "the handoff policy cannot be read: run the command from a checkout of this repository",
    )
}

pub(super) fn remote_unavailable(remote: &str) -> ForgeError {
    unproven(
        "remote_unavailable",
        false,
        "select_matching_remote",
        format!(
            "the handoff policy cannot be read: git remote {remote} is not configured or has an \
             unreadable URL; pass --remote naming this repository's remote"
        ),
    )
}

pub(super) fn remote_repository_mismatch(remote: &str) -> ForgeError {
    unproven(
        "remote_repository_mismatch",
        false,
        "select_matching_remote",
        format!(
            "git remote {remote} of the current checkout does not point at this pull request's \
             repository and host; run from a checkout of this repository or pass --remote naming \
             its remote"
        ),
    )
}

pub(super) fn base_ref_missing(remote: &str, base_ref: &str) -> ForgeError {
    unproven(
        "base_ref_missing",
        true,
        "fetch_base_ref",
        format!(
            "the provider base branch is not cached as refs/remotes/{remote}/{base_ref}; run `git \
             fetch {remote} {base_ref}` and retry"
        ),
    )
}

pub(super) fn base_ref_stale(remote: &str, base_ref: &str, provider_base: &str) -> ForgeError {
    unproven(
        "base_ref_stale",
        true,
        "fetch_base_ref",
        format!(
            "cached refs/remotes/{remote}/{base_ref} does not match the provider base {}; run \
             `git fetch {remote} {base_ref}` and retry",
            short(provider_base)
        ),
    )
}

pub(super) fn provider_base_behind_cache(
    remote: &str,
    base_ref: &str,
    provider_base: &str,
) -> ForgeError {
    unproven(
        "provider_base_behind_cache",
        true,
        "update_pr_branch",
        format!(
            "the provider reports base {}, which is older than the cached tip of \
             refs/remotes/{remote}/{base_ref}; update the pull request branch onto the current \
             base so the provider base advances, then retry",
            short(provider_base)
        ),
    )
}

pub(super) fn policy_config_invalid() -> ForgeError {
    unproven(
        "policy_config_invalid",
        false,
        "fix_handoff_policy_config",
        "[review_handoff].stale_after is invalid at the cached base ref; correct it on the base \
         branch and retry",
    )
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}
