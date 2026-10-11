use super::*;

pub(super) async fn activity_completion_refresh_deadline(
    log: &ActivityEventLog,
    context: &CliContext,
) -> Option<tokio::time::Instant> {
    let ids: Vec<_> = log
        .state
        .lock()
        .expect("activity log lock")
        .latest_sessions
        .iter()
        .filter(|session| {
            session.turn_state.as_ref().is_some_and(|state| {
                state.current_turn.is_some() && state.source.provider.as_deref() == Some("claude")
            })
        })
        .map(|session| session.id.clone())
        .collect();
    if ids.is_empty() {
        return None;
    }
    let context = context.clone();
    let deadline = tokio::task::spawn_blocking(move || {
        ids.iter()
            .filter_map(|id| {
                let record = load_session_record(&context, id).ok()?;
                activity::completion_deadline_for_view(&context, &record)
            })
            .min()
    })
    .await
    .ok()
    .flatten()?;
    let delay = (deadline - jiff::Timestamp::now())
        .try_into()
        .unwrap_or(Duration::ZERO);
    Some(tokio::time::Instant::now() + delay)
}

pub(super) async fn activity_refresh_snapshot(
    log: Arc<ActivityEventLog>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    session_collector: SessionCollector,
    context: CliContext,
    tmux_bin: PathBuf,
) -> bool {
    match tokio::task::spawn_blocking(move || {
        collect_activity_stream_sessions(&session_collector, &context, &tmux_bin)
    })
    .await
    {
        Ok(Ok(sessions)) => {
            log.publish_snapshot(sessions);
            true
        }
        Ok(Err(_)) | Err(_) => {
            degrade_activity_broker(
                &lifecycle,
                "activity stream snapshot refresh failed; activity stream disabled",
            );
            false
        }
    }
}
