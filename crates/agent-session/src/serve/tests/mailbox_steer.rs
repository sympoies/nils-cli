use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[tokio::test]
async fn coordination_notification_working_codex_uses_safe_in_turn_checkpoint() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2030/01/active.jsonl");
    fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("transcript dir");
    let provider_id = "notification-active-provider";
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let record_path = tmp.path().join("sessions/beta/session.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&record_path).expect("beta record")).expect("beta json");
    record["provider_resume"] = json!({
        "provider": "codex",
        "session_id": provider_id,
        "captured_at": "2030-01-01T00:00:00Z",
        "capture_method": "fixture",
        "resume_args": []
    });
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("beta record json"),
    )
    .expect("write beta provider resume");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = fixture::bind(&state.context, "beta", tmp.path(), provider_id);
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let turn = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "notification-active-turn-start",
        "runtime_id": launch_id,
        "provider": "codex",
        "provider_session_id": provider_id,
        "provider_turn_id": "notification-active-turn",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .expect("activity event");
    crate::activity::ingest_event(&state.context, &beta.id, turn).expect("ingest");
    let projected_turn_id = crate::activity::state_for_view(&state.context, &beta)
        .and_then(|turn| turn.current_turn)
        .and_then(|turn| turn.provider_turn_id)
        .expect("projected active turn id");
    assert_ne!(projected_turn_id, "notification-active-turn");
    crate::codex_account::queue_next_account_with_unbound(
        &state.context,
        &beta.id,
        &launch_id,
        "omega",
    )
    .expect("queue next account during the active turn");

    let canary = "ACTIVE-TURN-MAILBOX-BODY-MUST-NOT-REACH-CHECKPOINT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "notification-active-turn-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let registry_path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"]
            .as_array_mut()
            .expect("operations")
            .push(json!({
                "schema_version": "agent-session.operation-lease.v1",
                "lease_id": "active-turn-checkpoint-operation",
                "session_id": "beta",
                "session_incarnation": launch_id,
                "claim_id": "active-turn-checkpoint-claim",
                "claim_revision": 1,
                "operation": "atomic mutation",
                "targets": [],
                "provider_targets": [],
                "state": "active",
                "revision": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "expires_at": "9999-12-31T23:59:59Z",
                "expires_at_epoch": i64::MAX,
                "terminal_at_epoch": null,
                "execution_token_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activity_revision": 1,
                "activity_identity_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "runtime_identity_digest": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "descendant": null,
                "reconcile_observed_at_epoch": null,
                "outcome": null
            }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write active operation");
    drain_coordination_notifications(state.clone()).await;
    assert!(
        commands.try_recv().is_err(),
        "an active atomic operation must defer the in-turn checkpoint"
    );
    assert_eq!(
        notification_fixture(tmp.path())["last_reason"],
        "recipient-coordination-not-quiescent"
    );

    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["operations"] = json!([]);
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("clear active operation");
    let lock_probe_context = state.context.clone();
    let lifecycle_context = state.context.clone();
    let lifecycle_tmux = state.tmux_bin.clone();
    let maintenance_context = state.context.clone();
    let maintenance_tmux = state.tmux_bin.clone();
    let maintenance_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Delete,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("maintenance preview before notification submission");
    let inspect_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Inspect,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("inspect preview before notification submission");
    let attach_preview = crate::maintenance::preview(
        &maintenance_context,
        "beta",
        &maintenance_tmux,
        crate::maintenance::MaintenanceOperation::Attach,
        crate::maintenance::MaintenanceContract::V1,
    )
    .expect("attach preview before notification submission");
    let expected_prompt = fixture_notification_prompt(tmp.path());
    let notice_path = fixture::path(tmp.path(), "beta");
    let responder = tokio::spawn(async move {
        let command = tokio::time::timeout(Duration::from_secs(60), commands.recv())
            .await
            .expect("working Codex notification must reach an in-turn checkpoint")
            .expect("Codex control command");
        let codex_app_server::ControlCommand::Steer {
            message,
            expected_turn_id,
            client_message_id,
            response,
            ..
        } = command
        else {
            panic!("working Codex notification must use turn/steer");
        };
        assert_eq!(expected_turn_id, projected_turn_id);
        assert_eq!(message, expected_prompt);
        assert!(!message.contains(canary));
        let lock_probe = tokio::task::spawn_blocking(move || {
            let _record_lock = crate::acquire_session_record_lock(&lock_probe_context, "beta")
                .expect("checkpoint record lock must be reacquirable");
        });
        tokio::time::timeout(Duration::from_secs(60), lock_probe)
            .await
            .expect("checkpoint must release the record lock before provider I/O")
            .expect("record-lock probe worker");
        let lifecycle = tokio::task::spawn_blocking(move || {
            crate::resume_session_by_id(&lifecycle_context, "beta", &lifecycle_tmux)
        });
        let error = tokio::time::timeout(Duration::from_secs(60), lifecycle)
            .await
            .expect("lifecycle admission must remain bounded during checkpoint provider I/O")
            .expect("lifecycle probe worker")
            .expect_err("lifecycle mutation must be fenced while checkpoint steer is pending");
        assert_eq!(
            error.code(),
            "coordination-notification-submission-in-progress"
        );
        let maintenance = tokio::task::spawn_blocking(move || {
            let inspect = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Inspect,
                    action: crate::maintenance::MaintenanceActionId::Inspect,
                    expected_session_incarnation: inspect_preview.session_incarnation.clone(),
                    expected_session_generation: inspect_preview.session_generation,
                    expected_preview_digest: inspect_preview.preview_digest.clone(),
                    confirmed: false,
                },
                |_| Ok(()),
            );
            let attach = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Attach,
                    action: crate::maintenance::MaintenanceActionId::RetryAttach,
                    expected_session_incarnation: attach_preview.session_incarnation.clone(),
                    expected_session_generation: attach_preview.session_generation,
                    expected_preview_digest: attach_preview.preview_digest.clone(),
                    confirmed: false,
                },
                |_| Ok(()),
            );
            let delete = crate::maintenance::execute_with_resume_guard(
                &maintenance_context,
                "beta",
                &maintenance_tmux,
                crate::maintenance::MaintenanceActionRequest {
                    schema_version: crate::maintenance::MaintenanceContract::V1,
                    operation: crate::maintenance::MaintenanceOperation::Delete,
                    action: crate::maintenance::MaintenanceActionId::RetryDelete,
                    expected_session_incarnation: maintenance_preview.session_incarnation.clone(),
                    expected_session_generation: maintenance_preview.session_generation,
                    expected_preview_digest: maintenance_preview.preview_digest.clone(),
                    confirmed: true,
                },
                |_| Ok(()),
            );
            (inspect, attach, delete)
        });
        let (inspect, attach, delete) = tokio::time::timeout(Duration::from_secs(60), maintenance)
            .await
            .expect("maintenance admission must remain bounded during checkpoint provider I/O")
            .expect("maintenance probe worker");
        let inspect = serde_json::to_value(
            inspect.expect("read-only inspect must remain available during checkpoint steer"),
        )
        .expect("inspect result json");
        assert_eq!(inspect["outcome"], "inspected");
        let attach = serde_json::to_value(
            attach.expect("read-only attach must remain available during checkpoint steer"),
        )
        .expect("attach result json");
        assert_eq!(attach["outcome"], "inspected");
        let maintenance_error =
            delete.expect_err("maintenance mutation must preserve the checkpoint retry contract");
        assert_eq!(
            maintenance_error.code(),
            "coordination-notification-submission-in-progress"
        );
        let maintenance_details = maintenance_error
            .0
            .details
            .expect("notification submission retry details");
        assert_eq!(maintenance_details["retryable"], true);
        assert_eq!(
            maintenance_details["next_action"],
            "wait-for-notification-outcome"
        );
        fixture::append(
            &notice_path,
            &message,
            client_message_id.as_deref().unwrap(),
            "notification-active-turn",
        );
        response
            .send(Ok("notification-active-turn".into()))
            .expect("acknowledge checkpoint");
    });

    drain_coordination_notifications(state.clone()).await;
    responder.await.expect("checkpoint responder");

    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["notified_generation"], 1);
    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    assert_eq!(registry["messages"][0]["state"], "unread");

    send_notification_fixture(
        &state,
        &alpha_capability,
        "notification-active-turn-unknown-0002",
    )
    .await;
    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );

    let drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    let command = tokio::time::timeout(Duration::from_secs(60), commands.recv())
        .await
        .expect("unknown steer attempt")
        .expect("steer command");
    let codex_app_server::ControlCommand::Steer {
        expected_turn_id,
        response,
        ..
    } = command
    else {
        panic!("active notification retry must use turn/steer");
    };
    assert_eq!(
        expected_turn_id,
        crate::activity::state_for_view(&state.context, &beta)
            .and_then(|turn| turn.current_turn)
            .and_then(|turn| turn.provider_turn_id)
            .expect("same projected active turn")
    );
    response
        .send(Err("provider rejected stale turn".to_string()))
        .expect("reject steer");
    drain.await.expect("unknown steer drain");
    assert_eq!(notification_fixture(tmp.path())["state"], "attempt_unknown");

    assert!(
        crate::coordination::pending_notifications(&state.context)
            .expect("pending notifications")
            .is_empty(),
        "an unresolved steer must not become a pending retry"
    );
    let duplicate_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    // A drain that submitted would block on its response, so finishing
    // first proves nothing was sent.
    tokio::select! {
        biased;
        joined = duplicate_drain => joined.expect("empty notification drain"),
        command = commands.recv() => match command {
            Some(codex_app_server::ControlCommand::Steer { response, .. }) => {
                let _ = response.send(Err("duplicate test steer".to_string()));
                panic!("an unresolved steer must not be submitted twice");
            }
            Some(_) => panic!("an unresolved steer emitted another control command"),
            None => panic!("the active control channel closed unexpectedly"),
        },
    }
    write_codex_notification_transcript(&transcript, provider_id, &[]);
    reconcile_coordination_notifications(state.clone()).await;
    assert_eq!(
        notification_fixture(tmp.path())["state"],
        "attempt_unknown",
        "absence never authorizes notice replay"
    );

    write_codex_notification_transcript(
        &transcript,
        provider_id,
        &[(
            "9999-01-01T00:00:00Z",
            fixture_notification_prompt(tmp.path()),
        )],
    );
    reconcile_coordination_notifications(state.clone()).await;
    let notification = notification_fixture(tmp.path());
    assert_eq!(
        notification["state"], "attempt_unknown",
        "matching text without a client-row attestation cannot confirm a native attempt"
    );
    assert_eq!(notification["notified_generation"], 1);
}
