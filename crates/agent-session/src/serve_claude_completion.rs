use super::*;

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
