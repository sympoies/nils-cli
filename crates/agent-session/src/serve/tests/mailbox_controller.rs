use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn coordination_notification_startup_catchup_uses_one_acknowledged_codex_submission() {
    let _lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    for (event_id, kind) in [
        ("notification-turn-start", "turn_started"),
        ("notification-turn-complete", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "notification-idle-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
    }

    let canary = "CODEX-MAILBOX-BODY-MUST-NOT-REACH-PROMPT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "notification-catchup-0001",
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
    registry["claims"]
        .as_array_mut()
        .expect("claims")
        .push(json!({
            "schema_version": "agent-session.work-context.v1",
            "session_id": "beta",
            "session_incarnation": launch_id,
            "claim_id": "app-server-notification-race-claim",
            "revision": 1,
            "state": "active",
            "intent": "implementation",
            "tier": "direct",
            "repositories": [],
            "worktrees": [],
            "provider_refs": [],
            "plan_refs": [],
            "scopes": [],
            "summary": "structured notification race fixture",
            "updated_at": "2030-01-01T00:00:00Z",
            "expires_at": "9999-12-31T23:59:59Z",
            "expires_at_epoch": i64::MAX
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write active claim");
    let claimed_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    tokio::select! {
        biased;
        joined = claimed_drain => joined.expect("claimed drain"),
        command = commands.recv() => {
            if let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = command {
                let _ = response.send(Ok("turn-claimed-notification".to_string()));
            }
            panic!("active claim must prevent structured notification submission");
        }
    }

    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry json");
    registry["claims"] = json!([]);
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    registry["operations"]
            .as_array_mut()
            .expect("operations")
            .push(json!({
                "schema_version": "agent-session.operation-lease.v1",
                "lease_id": "app-server-notification-race-operation",
                "session_id": "beta",
                "session_incarnation": launch_id,
                "claim_id": "app-server-notification-race-claim",
                "claim_revision": 1,
                "operation": "test mutation",
                "targets": [],
                "provider_targets": [],
                "state": "reconcile_pending",
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
    .expect("write uncertain operation");
    let uncertain_drain = tokio::spawn(drain_coordination_notifications(state.clone()));
    tokio::select! {
        biased;
        joined = uncertain_drain => joined.expect("uncertain drain"),
        command = commands.recv() => {
            if let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = command {
                let _ = response.send(Ok("turn-uncertain-notification".to_string()));
            }
            panic!("uncertain operation must prevent structured notification submission");
        }
    }

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
    .expect("clear uncertain operation");

    let expected_prompt = fixture_notification_prompt(tmp.path());
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("notification did not reach the Codex control plane");
        };
        assert_eq!(message, expected_prompt);
        assert!(!message.contains(canary));
        response
            .send(Ok("turn-notification-catchup".to_string()))
            .expect("acknowledge prompt");
        commands
    });

    let first = tokio::spawn(drain_coordination_notifications(state.clone()));
    let second = tokio::spawn(drain_coordination_notifications(state.clone()));
    first.await.expect("first wakeup");
    second.await.expect("second wakeup");
    let mut commands = responder.await.expect("responder");
    assert!(
        commands.try_recv().is_err(),
        "racing wakeups must not submit a duplicate generation"
    );

    let registry: Value = serde_json::from_slice(
        &fs::read(tmp.path().join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notification = registry["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .next()
        .expect("notification generation");
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["notified_generation"], 1);
    assert_eq!(registry["messages"][0]["state"], "unread");
    assert!(!notification.to_string().contains(canary));
}

#[tokio::test]
async fn coordination_notification_skips_a_mailbox_the_recipient_already_drained() {
    let _lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    let launch_id = seed_codex_app_server_session(tmp.path(), "beta");
    let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    for (event_id, kind) in [
        ("drained-turn-start", "turn_started"),
        ("drained-turn-complete", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "drained-idle-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
    }
    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        beta.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let registry_path = tmp.path().join("coordination/registry.json");
    let drain_recipient_inbox = || {
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
                .expect("registry json");
        for message in registry["messages"].as_array_mut().expect("messages") {
            message["state"] = json!("acknowledged");
        }
        fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry).expect("registry json"),
        )
        .expect("drain recipient inbox");
    };

    send_notification_fixture(&state, &alpha_capability, "drained-mailbox-0001").await;
    let raced = crate::coordination::pending_notifications(&state.context)
        .expect("pending notification")
        .pop()
        .expect("notification candidate");
    // The recipient reads its mail at its own safe boundary before the
    // reminder is submitted.
    drain_recipient_inbox();
    assert!(
        crate::coordination::pending_notifications(&state.context)
            .expect("pending notifications")
            .is_empty(),
        "a drained mailbox must not be offered for delivery"
    );
    assert!(
        !crate::coordination::lock_session_quiescence(&state.context, "beta", &launch_id)
            .expect("quiescence")
            .begin_notification_attempt(&raced)
            .expect("attempt"),
        "the locked attempt must recheck unread mail discovered earlier"
    );
    drain_coordination_notifications(state.clone()).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), commands.recv())
            .await
            .is_err(),
        "no stale reminder may reach the recipient"
    );
    assert_eq!(notification_fixture(tmp.path())["state"], "queued");
    assert_eq!(notification_fixture(tmp.path())["notified_generation"], 0);

    send_notification_fixture(&state, &alpha_capability, "drained-mailbox-0002").await;
    let expected_prompt = fixture_notification_prompt(tmp.path());
    assert_eq!(expected_prompt, "Mailbox: agent-session message inbox");
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("new mail must be announced");
        };
        assert_eq!(message, expected_prompt);
        response
            .send(Ok("turn-drained-mailbox".to_string()))
            .expect("acknowledge prompt");
    });
    drain_coordination_notifications(state.clone()).await;
    responder.await.expect("responder");
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["notified_generation"], 2);
}
