use crate::CliError;

use super::{
    RetitleReceipt, account_readiness, api_key_missing, context_unavailable, provider_malformed,
    provider_quota_exceeded, provider_rate_limited, provider_timeout, provider_unavailable,
    retitle_error, turn_conflict,
};

pub(super) fn replayed_error(receipt: &RetitleReceipt) -> CliError {
    match receipt.diagnostic_code.as_str() {
        "retitle-queue-saturated" => retitle_error(
            "retitle-queue-saturated",
            "title provider queue is saturated",
            true,
            "wait_and_retry",
            "bounded_backoff",
        ),
        "retitle-context-unavailable" => context_unavailable(),
        "retitle-account-missing" => retitle_error(
            "retitle-account-missing",
            "configured title account is unavailable",
            true,
            "select_account",
            "refresh_account",
        ),
        "retitle-provider-timeout" => provider_timeout(),
        "retitle-provider-unavailable" => {
            let terminal_provider = receipt
                .observation
                .as_ref()
                .and_then(|observation| observation.attempts.last())
                .and_then(|attempt| attempt.providers.last());
            if terminal_provider.is_some_and(|provider| {
                provider.failure_class.as_deref()
                    == Some(account_readiness::BROKER_UNAVAILABLE_REASON)
            }) {
                account_readiness::broker_unavailable()
            } else {
                provider_unavailable()
            }
        }
        "retitle-provider-rate-limited" => provider_rate_limited(),
        "retitle-provider-quota-exceeded" => provider_quota_exceeded(),
        "retitle-provider-malformed-response" => provider_malformed(),
        "retitle-worker-failed" => retitle_error(
            "retitle-worker-failed",
            "title worker failed",
            true,
            "retry",
            "retry_request",
        ),
        "retitle-api-key-missing" => api_key_missing(),
        "retitle-state-conflict" => retitle_error(
            "retitle-state-conflict",
            "retitle state changed before completion",
            false,
            "refresh_session",
            "refresh_session",
        ),
        "retitle-turn-conflict" => turn_conflict(),
        _ => provider_unavailable(),
    }
}
