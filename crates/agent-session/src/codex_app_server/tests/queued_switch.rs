use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn manual_switch_applies_after_usage_limit_without_live_idle_probe() {
    after_usage_limit(false).await;
}

#[tokio::test]
async fn automatic_failover_applies_after_structured_failure_without_live_idle_probe() {
    after_usage_limit(true).await;
}

async fn after_usage_limit(automatic: bool) {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let broker_calls = tmp.path().join("broker-calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
calls=$1
shift
account=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --account)
      account="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
case "$account" in
  acct1|omega) ;;
  *) exit 2 ;;
esac
printf '%s\n' "$account" >> "$calls"
printf '%s\n' "{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"$account\",\"access_token\":\"token-$account\",\"chatgpt_account_id\":\"workspace-$account\",\"plan\":\"team\"}"
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![
            broker.to_string_lossy().into_owned(),
            broker_calls.to_string_lossy().into_owned(),
        ])
        .unwrap(),
    );
    let (_dir, socket_path) = nils_test_support::short_socket_path("manual-switch.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("manual-switch", &socket_path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-manual-switch",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    record = crate::load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
    if automatic {
        crate::auto_resume::set_enabled_with_policy(
            &context,
            &record.id,
            true,
            crate::auto_resume::NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
    }
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        "runtime-manual-switch",
        "thread-manual-switch",
        "turn-manual-switch",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    if automatic {
        crate::codex_account::queue_auto_failover_locked(&context, &mut record, "omega").unwrap();
    } else {
        crate::codex_account::queue_next_account(
            &context,
            &record.id,
            "runtime-manual-switch",
            "omega",
        )
        .unwrap();
    }
    record = crate::codex_account::prepare_control_reconnect(
        &context,
        &record.id,
        "runtime-manual-switch",
    )
    .unwrap();
    bind_thread(&record, "thread-manual-switch").unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let first_login = receive_json(&mut socket).await;
        assert_eq!(first_login["method"], "account/login/start");
        respond(
            &mut socket,
            &first_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["thread-manual-switch"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;

        let failover_login = receive_json(&mut socket).await;
        if failover_login["method"] == "thread/turns/list" {
            send_json(&mut socket, json!({"id": failover_login["id"], "error": {"code": -32000, "message": "fixture exhausted"}})).await.unwrap();
        }
        assert_eq!(
            failover_login["method"], "account/login/start",
            "an authoritative structured terminal failure must not depend on a live idle probe"
        );
        assert_eq!(failover_login["params"]["accessToken"], "token-omega");
        respond(
            &mut socket,
            &failover_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
    });

    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));
    handle.apply_next().await.unwrap();
    server.await.unwrap();
    assert!(
        ensure_turn_start_account_ready(&context, &record).await,
        "the next prompt must become admissible after the queued switch binds"
    );
    let mut bootstrap = FreshBootstrap::Closed;
    let authorization = cancel_before_tui_mutation_detailed(
        &context, &record, &mut bootstrap,
        &json!({"id": 42, "method": "turn/start", "params": {"threadId": "thread-manual-switch", "input": []}}),
    ).await;
    assert!(
        authorization.is_ok(),
        "the next prompt must pass admission after binding"
    );
    drop(authorization);
    control.abort();
    let _ = control.await;

    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.selected_account.as_deref(), Some("omega"));
    assert!(view.next.is_none());
}

#[test]
fn pending_account_admission_is_retryable_with_recovery() {
    let response = tui_busy_response(&json!(42), TuiMutationRejection::AccountNotReady);
    let Message::Text(response) = response else {
        panic!("expected JSON response")
    };
    let value: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(value["error"]["data"]["retryable"], true);
    assert_eq!(
        value["error"]["data"]["next_action"],
        "inspect-account-switch"
    );
    assert_eq!(
        value["error"]["data"]["recovery"]["outcome"],
        "not_forwarded"
    );
}

#[test]
fn pending_switch_projection_explains_why_input_is_fenced() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let mut record = crate::codex_account::record_with_binding_value(json!({
        "schema_version": crate::codex_account::BINDING_SCHEMA_VERSION,
        "selected_account": "acct1", "revision": 1, "state": "bound",
        "applied_runtime_id": "runtime-binding-fixture", "updated_at": "2030-01-01T00:00:00Z"
    }));
    record.extra.insert(
        "codex_account_next".into(),
        json!({
            "schema_version": crate::codex_account::NEXT_SCHEMA_VERSION,
            "account": "omega", "revision": 1, "intent_id": "fixture-intent",
            "selection_source": "explicit", "state": "queued", "updated_at": "2030-01-01T00:00:00Z"
        }),
    );
    let value = serde_json::to_value(crate::codex_account::view_for_record(&record)).unwrap();
    assert_eq!(value["selected_account"], "acct1");
    assert_eq!(value["next"]["account"], "omega");
    assert_eq!(
        value["next"]["blocking_reason"],
        "awaiting_idle_confirmation"
    );
}

#[tokio::test]
async fn terminal_usage_switch_rechecks_intent_and_new_input_before_adoption() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let (_dir, socket) = nils_test_support::short_socket_path("queued-fences.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("queued-fences", &socket);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let runtime = record.runtime.as_ref().unwrap().launch_id.clone();
    crate::codex_account::finish_binding(&context, &record.id, &runtime, "acct1", 1, Ok(()))
        .unwrap();
    record = crate::load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        &runtime,
        "fixture-thread",
        "fixture-turn",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    crate::codex_account::queue_next_account(&context, &record.id, &runtime, "omega").unwrap();
    let expected = queued_switch_has_authoritative_idle(&context, &record)
        .await
        .unwrap();

    // A later manual selection cannot inherit the earlier intent's proof.
    crate::codex_account::queue_next_account(&context, &record.id, &runtime, "beta").unwrap();
    assert!(
        crate::codex_account::begin_next_apply_if_unchanged(
            &context, &record.id, &runtime, &expected,
        )
        .unwrap()
        .is_none()
    );
    let expected = queued_switch_has_authoritative_idle(&context, &record)
        .await
        .unwrap();

    // Terminal input can be accepted before its activity event is observed.
    // Its fence must win over the old usage-limit completion.
    let _lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let mut current = crate::load_session_record(&context, &record.id).unwrap();
    crate::codex_account::authorize_terminal_input_locked(&context, &mut current).unwrap();
    drop(_lock);
    assert!(
        crate::codex_account::begin_next_apply_if_unchanged(
            &context, &record.id, &runtime, &expected,
        )
        .unwrap()
        .is_none()
    );
    let current = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&current);
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    assert_eq!(view.next.unwrap().state, "queued");
}

fn queued_terminal_fixture(
    tmp: &tempfile::TempDir,
    failure: StructuredFailureKind,
) -> (CliContext, SessionRecord) {
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("terminal-guards", &tmp.path().join("fixture.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let runtime = record.runtime.as_ref().unwrap().launch_id.clone();
    crate::codex_account::finish_binding(&context, &record.id, &runtime, "acct1", 1, Ok(()))
        .unwrap();
    record = crate::load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        &runtime,
        "fixture-thread",
        "fixture-turn",
        failure,
    )
    .unwrap();
    crate::codex_account::queue_next_account(&context, &record.id, &runtime, "omega").unwrap();
    (context, record)
}

#[tokio::test]
async fn terminal_usage_switch_rejects_other_failures_and_success() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    for failure in [
        StructuredFailureKind::Authentication,
        StructuredFailureKind::ProviderCapacity,
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = queued_terminal_fixture(&tmp, failure);
        assert!(
            queued_switch_has_authoritative_idle(&context, &record)
                .await
                .is_none()
        );
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let (context, record) = queued_terminal_fixture(&tmp, StructuredFailureKind::UsageExhausted);
    // A later successful turn replaces the exhaustion proof.
    for kind in ["turn_started", "turn_completed"] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": format!("successful-{kind}"),
            "runtime_id": record.runtime.as_ref().unwrap().launch_id,
            "provider": "codex",
            "provider_session_id": "fixture-thread",
            "provider_turn_id": "successful-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .unwrap();
        crate::activity::ingest_event(&context, &record.id, event).unwrap();
    }
    let activity = crate::activity::state_for_view(&context, &record).unwrap();
    assert_eq!(activity.last_turn.unwrap().outcome, "completed");
    assert!(
        queued_switch_has_authoritative_idle(&context, &record)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn terminal_usage_switch_rejects_refreshed_binding() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, record) = queued_terminal_fixture(&tmp, StructuredFailureKind::UsageExhausted);
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let expected = queued_switch_has_authoritative_idle(&context, &record)
        .await
        .unwrap();
    let attempt =
        crate::codex_account::begin_refresh_binding(&context, &record.id, runtime, "acct1")
            .unwrap();
    let view = crate::codex_account::restore_binding_after_refresh_failure(
        &context, &record.id, runtime, "acct1", attempt,
    )
    .unwrap();
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    assert_eq!(view.revision, 2);
    // Refresh advances the revision even when the account name is unchanged.
    // An account change also advances it, so the revision fences both cases.
    assert!(
        crate::codex_account::begin_next_apply_if_unchanged(
            &context, &record.id, runtime, &expected,
        )
        .unwrap()
        .is_none()
    );
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::view_for_record(&current)
            .next
            .unwrap()
            .state,
        "queued"
    );
}

#[tokio::test]
async fn terminal_usage_switch_rejects_in_flight_turn() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, record) = queued_terminal_fixture(&tmp, StructuredFailureKind::UsageExhausted);
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let expected = queued_switch_has_authoritative_idle(&context, &record)
        .await
        .unwrap();
    let event = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "new-turn-started",
        "runtime_id": runtime,
        "provider": "codex",
        "provider_session_id": "fixture-thread",
        "provider_turn_id": "new-turn",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&context, &record.id, event).unwrap();
    let activity = crate::activity::state_for_view(&context, &record).unwrap();
    assert!(activity.current_turn.is_some());
    assert!(
        queued_switch_has_authoritative_idle(&context, &record)
            .await
            .is_none()
    );
    assert!(
        crate::codex_account::begin_next_apply_if_unchanged(
            &context, &record.id, runtime, &expected,
        )
        .unwrap()
        .is_none()
    );
}
