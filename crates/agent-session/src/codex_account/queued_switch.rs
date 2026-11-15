//! Terminal usage-limit evidence for queued account adoption.

use super::*;

/// Called again under both the record lock and account gate before consuming
/// the exact intent. Explicit queueing cancels auto-resume, so its evidence
/// must be checked independently of the automatic continuation policy.
pub(crate) fn next_apply_has_authoritative_idle(
    context: &CliContext,
    record: &SessionRecord,
) -> bool {
    let DecodedNext::Valid(next) = decode_next(record) else {
        return false;
    };
    if next.state != "queued" {
        return false;
    }
    if next.selection_source.as_deref() == Some("auto_failover") {
        return crate::auto_resume::has_authoritative_usage_exhaustion_idle(context, record);
    }
    if next.selection_source.as_deref() != Some("explicit") {
        return false;
    }
    let BindingSnapshot::Bound { account, revision } = binding_snapshot(record) else {
        return false;
    };
    let Ok(Some(fence)) = input_fence(record) else {
        return false;
    };
    if record
        .runtime
        .as_ref()
        .is_none_or(|runtime| runtime.launch_id != fence.launch_id)
        || fence.binding_account.as_deref() != Some(account.as_str())
        || fence.binding_revision != Some(revision)
    {
        return false;
    }
    crate::activity::state_for_view(context, record).is_some_and(|activity| {
        activity.phase == crate::activity::TurnPhase::Waiting
            && activity.current_turn.is_none()
            && activity.revision > fence.activity_revision
            && activity.source.kind == crate::activity::SourceKind::ProviderHook
            && activity.source.confidence == crate::activity::Confidence::Authoritative
            && activity.source.provider.as_deref() == Some("codex")
            && activity.last_turn.as_ref().is_some_and(|turn| {
                turn.outcome == "failed" && turn.provider_failure_kind() == Some("usage_exhausted")
            })
    })
}
