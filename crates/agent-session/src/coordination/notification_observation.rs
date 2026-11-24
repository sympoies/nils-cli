//! Reconcile exact notification attempts against positive provider evidence.
use super::{NotificationCandidate, matches_observed_attempt, receipt_key};
use crate::coordination::now_epoch;
use crate::provider_prompt::{ProviderPromptTail, notice_observed_after};
use crate::{CliContext, load_session_record};

/// Give submission and transcript observation time to settle before releasing
/// a lifecycle fence. An expired attempt remains consumed.
const ATTEMPT_DEADLINE_SECONDS: i64 = 120;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotificationOutcome {
    Observed,
    Expired,
}

pub(crate) fn notification_outcome(
    context: &CliContext,
    candidate: &NotificationCandidate,
    attempting: bool,
) -> Option<NotificationOutcome> {
    if observe_attempt(context, candidate).is_some() {
        Some(NotificationOutcome::Observed)
    } else if attempting
        && now_epoch().saturating_sub(candidate.attempted_at_epoch) >= ATTEMPT_DEADLINE_SECONDS
    {
        Some(NotificationOutcome::Expired)
    } else {
        None
    }
}

pub(in crate::coordination) fn observe_attempt(
    context: &CliContext,
    candidate: &NotificationCandidate,
) -> Option<()> {
    let record = load_session_record(context, &candidate.target_session_id).ok()?;
    if record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
        return None;
    }
    // A timestamp/text match cannot supply the native client/physical-row proof.
    // An unresolved app-server notice remains consumed even if it completes late.
    if candidate.is_notice() && crate::codex_app_server::runtime_is_supported(&record) {
        return None;
    }
    let attempted_at = candidate
        .attempted_at
        .as_deref()?
        .parse::<jiff::Timestamp>()
        .ok()?;
    let source = ProviderPromptTail::resolve_source(&record)?;
    let format = {
        let locked = super::super::lock_registry_observational(context).ok()?;
        let receipt = locked.registry.notifications.get(&receipt_key(
            &candidate.target_session_id,
            &candidate.target_incarnation,
        ))?;
        if !matches_observed_attempt(receipt, candidate) {
            return None;
        }
        receipt.attempted_prompt_format.clone()
    };
    if format.as_deref() != Some(super::notice::FORMAT) {
        return None;
    }
    let current = candidate.prompt.as_deref()?;
    if !super::is_mailbox_reminder_prompt(current) {
        return None;
    }
    // A native call can complete late. Only exact positive observation resolves
    // it; absence never authorizes replay or a plain-prompt fallback.
    notice_observed_after(&source, current, &attempted_at).then_some(())
}
