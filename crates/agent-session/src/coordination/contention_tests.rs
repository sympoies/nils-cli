use super::*;
use pretty_assertions::assert_eq;

fn fixture_tmux(context: &CliContext) -> PathBuf {
    let path = context.state_dir.join("fixture-tmux");
    fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\n' \"can't find session: fixture\" >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn record(context: &CliContext, id: &str) -> SessionRecord {
    let created = crate::create_record(crate::RecordRequest {
        context,
        agent: crate::AgentKind::Codex,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some(id),
        cwd: &context.state_dir,
        prompt: None,
        log_file_name: None,
        provider_resume: None,
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .unwrap();
    let mut record = created.record.clone();
    crate::mark_tmux_runtime_never_launched(&mut record);
    crate::write_session_record(context, &record).unwrap();
    record
}

// Model concurrent older readers: shared flocks still block registry writers,
// even though current read-only snapshots no longer acquire the global lock.
fn readers(context: &CliContext, duration: Duration) -> thread::JoinHandle<()> {
    let root = coordination_root(context).unwrap();
    let held: Vec<_> = (0..4)
        .map(|_| {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(SECRET_FILE_MODE)
                .open(root.join(REGISTRY_LOCK))
                .unwrap();
            // SAFETY: each reader owns a valid descriptor for the fixture lock.
            assert_eq!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) },
                0
            );
            file
        })
        .collect();
    thread::spawn(move || {
        thread::sleep(duration);
        drop(held);
    })
}

#[test]
fn lifecycle_provision_survives_concurrent_registry_readers() {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    let record = record(&context, "start-contention");
    let held = readers(&context, Duration::from_millis(2300));
    let started = Instant::now();
    let result = broker::provision(&context, &record);
    held.join().unwrap();
    assert!(result.is_ok(), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(6));
    let registry = lock_registry_observational(&context).unwrap();
    assert_eq!(
        registry.registry.brokers[&record.id].incarnation,
        incarnation(&record).unwrap()
    );
}

#[test]
fn lifecycle_delete_and_close_survive_concurrent_registry_readers() {
    for close in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().join("state"),
            host: None,
        };
        let record = record(&context, "delete-contention");
        let held = readers(&context, Duration::from_millis(2300));
        let started = Instant::now();
        let result = if close {
            crate::finish_session_delete(
                &context,
                record.clone(),
                crate::session_dir(&context, &record.id),
                crate::SessionRegistryFence::from_record(&record),
                crate::board::CloseReason::Archived,
            )
        } else {
            crate::delete_session(&context, &record.id, fixture_tmux(&context))
        };
        held.join().unwrap();
        assert!(result.is_ok(), "close={close}: {result:?}");
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(!crate::session_dir(&context, &record.id).exists());
    }
}

#[test]
fn lifecycle_delete_and_archive_exhaustion_is_retryable_and_exact_retry_converges() {
    for (reason, action) in [
        (crate::board::CloseReason::Deleted, "retry-delete"),
        (crate::board::CloseReason::Archived, "retry-archive"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().join("state"),
            host: None,
        };
        let record = record(&context, "lifecycle-retry");
        let operation = || match reason {
            crate::board::CloseReason::Deleted => {
                crate::delete_session(&context, &record.id, fixture_tmux(&context))
            }
            crate::board::CloseReason::Archived => crate::finish_session_delete(
                &context,
                record.clone(),
                crate::session_dir(&context, &record.id),
                crate::SessionRegistryFence::from_record(&record),
                reason,
            ),
        };
        let held = readers(&context, Duration::from_millis(6500));
        let started = Instant::now();
        let error = operation().unwrap_err();
        let elapsed = started.elapsed();
        held.join().unwrap();
        assert_eq!(error.code(), "coordination-lock-timeout");
        assert_eq!(error.details().unwrap()["retryable"], true);
        assert_eq!(error.details().unwrap()["next_action"], action);
        assert_eq!(
            error.details().unwrap()["recovery"]["outcome"],
            "record_retained"
        );
        assert!(elapsed >= Duration::from_secs(6) && elapsed < Duration::from_secs(7));
        assert_eq!(
            crate::load_session_record(&context, &record.id)
                .unwrap()
                .runtime
                .unwrap()
                .launch_id,
            record.runtime.as_ref().unwrap().launch_id
        );
        assert!(
            !serde_json::to_string(error.details().unwrap())
                .unwrap()
                .contains(context.state_dir.to_str().unwrap())
        );
        assert_eq!(
            error.into_inner().exit_code,
            nils_common::cli_contract::exit::RUNTIME
        );
        operation().unwrap();
        assert!(!crate::session_dir(&context, &record.id).exists());
    }
}

#[test]
fn lifecycle_failed_provision_cleanup_has_a_definite_create_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    let created = crate::create_record(crate::RecordRequest {
        context: &context,
        agent: crate::AgentKind::Codex,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("create-retry"),
        cwd: &context.state_dir,
        prompt: None,
        log_file_name: None,
        provider_resume: None,
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .unwrap();
    let held = readers(&context, Duration::from_millis(6500));
    let error = broker::provision(&context, &created.record).unwrap_err();
    // The real failed-start branch uses this cleanup only after its exact
    // runtime is verified stopped; this fixture never starts a runtime.
    crate::cleanup_created_record(&context, &created);
    let error = crate::with_failed_launch_cleanup(error, crate::FailedLaunchCleanup::Completed);
    held.join().unwrap();
    assert_eq!(error.details().unwrap()["retryable"], true);
    assert_eq!(error.details().unwrap()["next_action"], "retry-create");
    assert_eq!(
        error.details().unwrap()["recovery"]["outcome"],
        "not_created"
    );
    assert!(!crate::session_dir(&context, &created.record.id).exists());
    assert!(
        !lock_registry_observational(&context)
            .unwrap()
            .registry
            .brokers
            .contains_key(&created.record.id)
    );
}

#[test]
fn tui_cancellation_survives_transient_session_state_contention() {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    let record = record(&context, "thread-contention");
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let held = thread::spawn(move || {
        thread::sleep(Duration::from_millis(1300));
        drop(lock);
    });
    let result = crate::auto_resume::cancel_for_manual_input_for_runtime_with_timeout(
        &context,
        &record.id,
        &incarnation(&record).unwrap(),
        "2030-01-01T00:00:00Z",
        false,
    )
    .unwrap();
    held.join().unwrap();
    assert_eq!(result, crate::auto_resume::ManualInputCancelOutcome::Ready);
}

#[test]
fn held_launcher_exit_contention_keeps_both_attempts_inside_graceful_budget() {
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    };
    let record = record(&context, "exit-contention");
    let held = lock_private_store(&context, REGISTRY_LOCK).unwrap();
    let started = Instant::now();
    // The held launcher executes a compatibility fallback after a failed exit
    // hook. Neither attempt may inherit the control-plane six-second wait.
    for _ in 0..2 {
        let attempt = Instant::now();
        let error = broker::revoke_after_runtime_exit(&context, &record).unwrap_err();
        assert_eq!(error.code(), "coordination-lock-timeout");
        assert!(attempt.elapsed() < Duration::from_secs(3));
    }
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(held);
    broker::revoke(&context, &record).unwrap();
}
