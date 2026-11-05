//! Reconcile exact attempt bytes without turning an unknown format into absence.
use super::{NotificationCandidate, receipt_key};
use crate::provider_prompt::{ProviderPromptTail, prompt_observed_after};
use crate::{CliContext, load_session_record};

pub(crate) fn observe_attempt(
    context: &CliContext,
    candidate: &NotificationCandidate,
) -> Option<bool> {
    let record = load_session_record(context, &candidate.target_session_id).ok()?;
    if record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
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
        if receipt.attempted_generation != candidate.generation
            || receipt.attempted_at != candidate.attempted_at
            || receipt.attempted_at_epoch != candidate.attempted_at_epoch
            || receipt.attempted_queued_at_epoch != candidate.queued_at_epoch
        {
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
    prompt_observed_after(&source, current, &attempted_at).filter(|observed| *observed)
}
