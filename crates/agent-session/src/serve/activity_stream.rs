use super::*;

impl ActivityEventLog {
    pub(super) fn publish_snapshot(&self, sessions: Vec<ActivityStreamSession>) {
        let mut state = self.state.lock().expect("activity event state lock");
        let unchanged = state.latest_sessions == sessions;
        if unchanged {
            state.latest_sessions = sessions;
            state.latest_observed_at = activity_observed_at();
            if !state.latest_snapshot_oversized {
                state.cached_snapshot = None;
                state.cached_reset = None;
            }
            return;
        }
        let sequence = self.sequence.load(Ordering::SeqCst) + 1;
        let observed_at = activity_observed_at();
        let event = self.encoder.encode(ActivityStreamEvent {
            schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
            kind: "snapshot",
            stream_id: self.stream_id.clone(),
            sequence,
            machine: self.machine.clone(),
            observed_at: observed_at.clone(),
            reason: None,
            sessions: Some(sessions.clone()),
        });
        let oversized = event.replay_bytes() > ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY;
        let was_oversized = state.latest_snapshot_oversized;
        state.latest_sessions = sessions;
        state.latest_observed_at = observed_at;
        state.latest_snapshot_oversized = oversized;
        if oversized && was_oversized {
            return;
        }
        self.sequence.store(sequence, Ordering::SeqCst);
        state.cached_snapshot = None;
        state.cached_reset = None;
        let event = if oversized {
            let reset = self.content_free_reset_frame(sequence, &state.latest_observed_at);
            state.cached_snapshot = Some(reset.clone());
            state.cached_reset = Some(reset.clone());
            reset
        } else {
            if event.kind == "snapshot" {
                state.cached_snapshot = Some(event.clone());
            }
            event
        };
        self.retain_and_broadcast_locked(&mut state, event);
    }
}

pub(super) fn activity_notify_event_relevant(event: &NotifyEvent) -> bool {
    let temporary_shadow_only = !event.paths.is_empty()
        && event.paths.iter().all(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".activity.shadow.json.tmp-"))
        });
    if temporary_shadow_only {
        return false;
    }
    let known_snapshot_path = event.paths.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name,
                    "activity.json"
                        | "activity.unhealthy.json"
                        | "session.json"
                        | "activity.shadow.json"
                )
            })
    });
    // Linux inotify normally names the removed file, while macOS FSEvents can
    // coalesce a rename/removal to its watched directory. All paths delivered
    // here are already scoped beneath the sessions root, so treat rename and
    // remove kinds as lifecycle invalidations without broadening modify noise
    // from terminal FIFOs or attachment writes.
    known_snapshot_path
        || matches!(
            event.kind,
            EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
        )
}

pub(super) fn shadow_refresh_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}
