use super::*;

pub(crate) fn completion_deadline_for_view(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<jiff::Timestamp> {
    if crate::dsh_external::is_external_record(record) {
        return None;
    }
    let dir = session_dir(context, &record.id);
    if let Some(runtime) = &record.runtime
        && !matches!(
            runtime_unhealthy_marker(&dir, &runtime.launch_id, runtime.generation),
            RuntimeUnhealthyStatus::Absent
        )
    {
        return None;
    }
    let document = read_document(&dir.join(ACTIVITY_FILE)).ok()?;
    if !activity_matches_runtime(&document, record) || !replay_matches_document(&dir, &document) {
        return None;
    }
    claude_completion_deadline(&document)
}

fn claude_completion_deadline(document: &ActivityDocument) -> Option<jiff::Timestamp> {
    document
        .pending_completion
        .as_ref()?
        .received_at
        .parse::<jiff::Timestamp>()
        .ok()?
        .checked_add(jiff::SignedDuration::from_secs(1))
        .ok()
}

/// Completion is a deterministic projection of the latest qualifying Stop.
/// Reads never write activity; the next accepted event materializes the same
/// settled state before reduction. Any event inside the quiet window cancels it.
pub(super) fn settle_claude_completion(document: &mut ActivityDocument, at: &str) {
    let Some(pending) = document.pending_completion.as_ref() else {
        return;
    };
    let Some(deadline) = claude_completion_deadline(document) else {
        return;
    };
    if !at
        .parse::<jiff::Timestamp>()
        .is_ok_and(|time| time >= deadline)
    {
        return;
    }
    let mut completion = pending.event.clone();
    completion.kind = TurnEventKind::TurnCompleted;
    completion.completion_candidate = false;
    reduce(document, &completion, &deadline.to_string());
    document.state.diagnostic = None;
}
