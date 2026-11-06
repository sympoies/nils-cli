use super::*;
use notify::event::{AccessKind, AccessMode};

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
    // Linux inotify (notify arms `IN_OPEN`) reports every read as an access
    // event. A snapshot sweep reads these very files, so counting reads would
    // make each sweep schedule the next one. Only a completed write changes
    // state; creates, modifications, renames and removals arrive as their own
    // event kinds.
    if matches!(event.kind, EventKind::Access(access) if access != AccessKind::Close(AccessMode::Write))
    {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, ModifyKind};

    const SESSIONS: usize = 35;
    /// Loop ticks observed after the single triggering change.
    const TICKS: u32 = 8;
    /// State files a snapshot sweep opens per session when the session is idle.
    const PRESENT_FILES: [&str; 7] = [
        "session.json",
        "resume.json",
        "activity.json",
        "activity.replay.bin",
        "auth-incidents.json",
        "auto-resume.json",
        "coordination/heartbeat",
    ];
    const ABSENT_MARKER: &str = "activity.unhealthy.json";

    /// Runs the real change loop over idle fixture sessions whose collector
    /// counts its sweeps and file opens. Like a Linux inotify watch, it reports
    /// an `Access(Open)` event for every existing file it opens.
    struct SweepHarness {
        _tmp: tempfile::TempDir,
        sessions_root: PathBuf,
        broker: Arc<ActivityBroker>,
        change_tx: mpsc::Sender<ActivityChange>,
        sweeps: Arc<AtomicUsize>,
        opens: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl SweepHarness {
        fn start() -> Self {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let sessions_root = tmp.path().join("sessions");
            let session_dirs: Vec<PathBuf> = (0..SESSIONS)
                .map(|index| sessions_root.join(format!("idle-{index}")))
                .collect();
            for dir in &session_dirs {
                fs::create_dir_all(dir.join("coordination")).expect("session dir");
                for name in PRESENT_FILES {
                    fs::write(dir.join(name), b"{}").expect("state file");
                }
            }
            let broker = ActivityBroker::for_test("test-machine");
            let (change_tx, change_rx) = mpsc::channel(1);
            let sweeps = Arc::new(AtomicUsize::new(0));
            let opens = Arc::new(AtomicUsize::new(0));
            let collector: SessionCollector = {
                let (change_tx, lifecycle, root) = (
                    change_tx.clone(),
                    broker.lifecycle.clone(),
                    sessions_root.clone(),
                );
                let (sweeps, opens) = (sweeps.clone(), opens.clone());
                Arc::new(move |_, _| {
                    sweeps.fetch_add(1, Ordering::SeqCst);
                    for dir in &session_dirs {
                        for name in PRESENT_FILES.into_iter().chain([ABSENT_MARKER]) {
                            opens.fetch_add(1, Ordering::SeqCst);
                            let path = dir.join(name);
                            if fs::File::open(&path).is_ok() {
                                let open = NotifyEvent::new(EventKind::Access(AccessKind::Open(
                                    AccessMode::Any,
                                )))
                                .add_path(path);
                                activity_watch_callback(Ok(open), &change_tx, &lifecycle, &root);
                            }
                        }
                    }
                    Ok(Vec::new())
                })
            };
            let task = tokio::spawn(activity_change_loop_inner(
                broker.log.clone(),
                broker.lifecycle.clone(),
                collector,
                CliContext {
                    state_dir: tmp.path().to_path_buf(),
                    host: None,
                },
                PathBuf::from("tmux"),
                change_rx,
                ActivityChangeLoopControls::default(),
            ));
            Self {
                _tmp: tmp,
                sessions_root,
                broker,
                change_tx,
                sweeps,
                opens,
                task,
            }
        }

        /// Delivers one watcher event for `session/name`, then lets `TICKS`
        /// loop ticks pass on the paused clock.
        async fn change_and_settle(&self, kind: EventKind, name: &str) {
            let event =
                NotifyEvent::new(kind).add_path(self.sessions_root.join("idle-0").join(name));
            activity_watch_callback(
                Ok(event),
                &self.change_tx,
                &self.broker.lifecycle,
                &self.sessions_root,
            );
            tokio::time::sleep(ACTIVITY_STREAM_MAX_REFRESH_CADENCE * TICKS).await;
        }

        async fn stop(self) {
            self.task.abort();
            let _ = self.task.await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_sessions_do_not_retrigger_snapshot_sweeps_from_their_own_reads() {
        let harness = SweepHarness::start();
        harness
            .change_tx
            .send(ActivityChange::Refresh)
            .await
            .expect("trigger");
        tokio::time::sleep(ACTIVITY_STREAM_MAX_REFRESH_CADENCE * TICKS).await;

        assert_eq!(harness.sweeps.load(Ordering::SeqCst), 1);
        assert_eq!(
            harness.opens.load(Ordering::SeqCst),
            SESSIONS * (PRESENT_FILES.len() + 1)
        );
        harness.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn written_state_and_new_markers_still_refresh_the_snapshot() {
        let harness = SweepHarness::start();
        let changes = [
            (EventKind::Create(CreateKind::File), ABSENT_MARKER),
            (
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                "activity.json",
            ),
            (
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
                "session.json",
            ),
        ];
        for (index, (kind, name)) in changes.into_iter().enumerate() {
            harness.change_and_settle(kind, name).await;
            assert_eq!(harness.sweeps.load(Ordering::SeqCst), index + 1, "{name}");
        }
        harness.stop().await;
    }

    #[test]
    fn read_only_access_events_are_not_relevant() {
        for access in [
            AccessKind::Any,
            AccessKind::Read,
            AccessKind::Open(AccessMode::Any),
            AccessKind::Close(AccessMode::Read),
        ] {
            for name in [
                "activity.json",
                "session.json",
                ABSENT_MARKER,
                "activity.shadow.json",
            ] {
                let event = NotifyEvent::new(EventKind::Access(access))
                    .add_path(PathBuf::from("/sessions/s1").join(name));
                assert!(!activity_notify_event_relevant(&event), "{access:?} {name}");
            }
        }
    }
}
