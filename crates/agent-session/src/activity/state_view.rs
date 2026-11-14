use super::*;
#[path = "completion_uncertainty.rs"]
mod completion_uncertainty;
#[path = "runtime_stopped.rs"]
mod runtime_stopped;
#[path = "session_ready.rs"]
mod session_ready;
pub(crate) use session_ready::settle_starting;

pub fn state_for_view(context: &CliContext, record: &SessionRecord) -> Option<TurnState> {
    read_state(context, record, true)
}

/// Quiet presentation uncertainty must not deny active tools or expire leases.
pub(crate) fn controller_state_for_record(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<TurnState> {
    read_state(context, record, false)
}

fn read_state(context: &CliContext, record: &SessionRecord, quiet: bool) -> Option<TurnState> {
    let dir = session_dir(context, &record.id);
    if let Some(runtime) = record.runtime.as_ref() {
        match runtime_unhealthy_marker(&dir, &runtime.launch_id, runtime.generation) {
            RuntimeUnhealthyStatus::Matching(state) => return Some(*state),
            RuntimeUnhealthyStatus::Pending(marked_at) => {
                return Some(marker_state_from_snapshot(
                    &dir,
                    record,
                    &runtime.launch_id,
                    runtime.generation,
                    &marked_at,
                ));
            }
            RuntimeUnhealthyStatus::Invalid => {
                return Some(marker_state_from_snapshot(
                    &dir,
                    record,
                    &runtime.launch_id,
                    runtime.generation,
                    &runtime.started_at,
                ));
            }
            RuntimeUnhealthyStatus::Absent => {}
        }
    }
    let path = dir.join(ACTIVITY_FILE);
    if !path.is_file() {
        return None;
    }
    match read_document(&path) {
        Ok(mut document)
            if activity_matches_runtime(&document, record)
                && replay_matches_document(&dir, &document) =>
        {
            super::claude_completion::settle_claude_completion(context, record, &mut document);
            super::codex_completion::recover_completed_snapshot(
                &dir.join(ACTIVITY_JOURNAL_FILE),
                &mut document,
            );
            if quiet {
                session_ready::project_unverified(record, &mut document.state, &now());
                completion_uncertainty::project(&mut document.state, &now());
                super::codex_quiet::project(record, &mut document.state, &now());
                crate::send_guard::response::project(context, record, &mut document.state);
                super::shadow::project_claude_observation(context, record, &mut document.state);
            }
            runtime_stopped::project(context, record, &mut document.state);
            Some(document.state)
        }
        Ok(_) | Err(_) => Some(unknown_state_with_reason(
            record,
            "activity_state_unavailable",
        )),
    }
}
