use super::*;

pub fn state_for_view(context: &CliContext, record: &SessionRecord) -> Option<TurnState> {
    // A plugin-owned dsh lane reports its turn through the liveness sidecar
    // instead of this store's activity document, and its unhealthy markers
    // belong to a tmux runtime it never had. Projecting it here keeps every
    // activity consumer — claim gates, views, prompt baselines — reading the
    // same evidence as `main-agent worker diagnose`.
    if crate::dsh_external::is_external_record(record) {
        return crate::dsh_external::external_turn_state(record);
    }
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
            super::claude_completion::settle_claude_completion(&mut document, &now());
            Some(document.state)
        }
        Ok(_) | Err(_) => Some(unknown_state_with_reason(
            record,
            "activity_state_unavailable",
        )),
    }
}
