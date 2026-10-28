//! Restart admission against a native idle thread, without a model turn.
use super::*;
use futures_util::{SinkExt, StreamExt};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt};

fn fixture(root: &Path) -> (CliContext, SessionRecord, PathBuf) {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let mut record = crate::codex_account::record_with_binding_value(json!({
        "schema_version": crate::codex_account::BINDING_SCHEMA_VERSION,
        "selected_account": "alpha", "revision": 3, "state": "failed",
        "failure_reason": "apply_failed", "updated_at": "2030-01-01T00:00:00Z"
    }));
    record.cwd = root.to_string_lossy().into_owned();
    let runtime = record.runtime.as_mut().unwrap();
    runtime.extra.insert(
        crate::codex_app_server::SOCKET_KEY.into(),
        json!(root.join("s")),
    );
    runtime.extra.insert(
        crate::codex_app_server::THREAD_ATTACHED_KEY.into(),
        json!(root.join("attached")),
    );
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"codex", "session_id":"fixture-thread",
            "captured_at":"2030-01-01T00:00:00Z", "capture_method":"fixture",
            "resume_args":["resume", "fixture-thread"]
        }))
        .unwrap(),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_app_server::replace_thread_binding(&record, "fixture-thread").unwrap();
    crate::codex_account::queue_next_account_with_unbound(
        &context,
        &record.id,
        "runtime-binding-fixture",
        "beta",
    )
    .unwrap();
    let record = load_session_record(&context, &record.id).unwrap();
    let tmux = root.join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
case "$1" in
show-environment)
  case "$4" in
  AGENT_SESSION_ID) echo 'AGENT_SESSION_ID=binding-fixture' ;;
  AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state} ;;
  AGENT_SESSION_RUNTIME_ID) echo 'AGENT_SESSION_RUNTIME_ID=runtime-binding-fixture' ;;
  esac ;;
capture-pane) printf '\n› \n\n  fixture-model · 100%% left\n' ;;
load-buffer) cat "$4" >> {input} ;;
esac
"#,
            state = shell_words::quote(&context.state_dir.to_string_lossy()),
            input = shell_words::quote(&root.join("input").to_string_lossy())
        ),
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    (context, record, tmux)
}

/// Serve only observation RPCs. Any attempt to start a model turn fails.
async fn idle_server(listener: tokio::net::UnixListener, status: &'static str) {
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            if !frame.is_text() {
                continue;
            }
            let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "initialize" => json!({}),
                "initialized" => continue,
                "thread/loaded/list" => json!({"data":["fixture-thread"],"nextCursor":null}),
                "thread/read" => json!({"thread":{"id":"fixture-thread","status":{"type":status}}}),
                method => panic!("unexpected mutation: {method}"),
            };
            if socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    json!({"id":request["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

fn seed_idle(context: &CliContext, record: &SessionRecord) {
    // Same normalized events as the documented disposable-session CLI recipe.
    for kind in ["turn_started", "turn_completed"] {
        crate::activity::ingest_event(
            context,
            &record.id,
            serde_json::from_value(json!({
                "schema_version":"agent-session.turn-event.v1", "event_id":kind,
                "runtime_id":"runtime-binding-fixture", "provider":"codex", "provider_session_id":"fixture-thread",
                "provider_turn_id":"acceptance-idle", "kind":kind, "confidence":"observed",
                "source_kind":"console_observation"
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

async fn request_exit(
    context: CliContext,
    record: SessionRecord,
    tmux: PathBuf,
    status: &'static str,
) -> Result<bool, CliError> {
    let server = tokio::spawn(idle_server(
        tokio::net::UnixListener::bind(crate::codex_app_server::socket_path(&record).unwrap())
            .unwrap(),
        status,
    ));
    let result = tokio::task::spawn_blocking(move || {
        let result = request_provider_exit(
            &context,
            &record,
            AgentKind::Codex,
            "runtime-binding-fixture",
            &tmux,
            false,
        );
        let sent = result.map(|(sent, _fence)| sent);
        // Restart admission must never authorize ordinary terminal or turn input.
        let current = load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            crate::codex_account::ensure_terminal_input_allowed(&current)
                .unwrap_err()
                .code(),
            "codex-account-not-bound"
        );
        assert!(crate::codex_account::ensure_input_allowed(&current).is_err());
        if sent.is_ok()
            && crate::activity::state_for_view(&context, &current)
                .unwrap()
                .phase
                == crate::activity::TurnPhase::Waiting
        {
            assert_eq!(
                crate::codex_account::begin_switch_binding(
                    &context,
                    &current.id,
                    "runtime-binding-fixture",
                    "gamma"
                )
                .unwrap_err()
                .code(),
                "codex-account-session-busy"
            );
        }
        sent
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    result
}

async fn assert_restart_repairs(phase: crate::activity::TurnPhase) {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::tempdir().unwrap();
    let (context, record, tmux) = fixture(tmp.path());
    if phase == crate::activity::TurnPhase::Waiting {
        crate::auth_incident::observe(
            &context,
            &record,
            "fixture-auth-failure",
            crate::auth_incident::AuthSource::CodexAppServer,
            crate::activity::Confidence::Authoritative,
            &now(),
        )
        .unwrap();
        assert!(crate::auth_incident::recovery_pending(&context, &record));
        seed_idle(&context, &record);
        // Synthetic activity is not evidence of successful authentication.
        assert!(crate::auth_incident::recovery_pending(&context, &record));
        assert_eq!(
            crate::activity::state_for_view(&context, &record)
                .unwrap()
                .source
                .kind,
            crate::activity::SourceKind::ConsoleObservation
        );
    }
    assert_eq!(
        crate::activity::state_for_view(&context, &record)
            .unwrap()
            .phase,
        phase
    );
    let result = request_exit(context, record, tmux, "idle").await;
    assert!(
        result.unwrap(),
        "normal restart must request the sealed provider exit"
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("input")).unwrap(),
        "/quit"
    );
}

#[tokio::test]
async fn native_idle_starting_restart_honours_queued_account_repair() {
    assert_restart_repairs(crate::activity::TurnPhase::Starting).await;
}

#[tokio::test]
async fn synthetic_idle_fixture_allows_restart_repair_without_a_model_turn() {
    assert_restart_repairs(crate::activity::TurnPhase::Waiting).await;
}

#[tokio::test]
async fn restart_repair_keeps_native_idle_and_explicit_intent_guards() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    for case in [
        "active",
        "attention",
        "absent",
        "malformed",
        "applying",
        "automatic",
        "pending_binding",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (context, mut record, tmux) = fixture(tmp.path());
        match case {
            "pending_binding" => {
                record.extra.get_mut("codex_account_binding").unwrap()["state"] = json!("pending");
            }
            "absent" => {
                record.extra.remove("codex_account_next");
            }
            "malformed" => {
                record
                    .extra
                    .insert("codex_account_next".into(), json!({"state":"queued"}));
            }
            "applying" => {
                record.extra.get_mut("codex_account_next").unwrap()["state"] = json!("applying");
            }
            "automatic" => {
                record.extra.get_mut("codex_account_next").unwrap()["selection_source"] =
                    json!("auto_failover");
            }
            "attention" => {
                crate::activity::ingest_event(
                    &context,
                    &record.id,
                    serde_json::from_value(json!({
                        "schema_version":"agent-session.turn-event.v1", "event_id":"attention",
                        "runtime_id":"runtime-binding-fixture", "provider":"codex", "provider_session_id":"fixture-thread",
                        "kind":"attention_requested", "attention_id":"clarification",
                        "attention_kind":"clarification", "confidence":"observed"
                    }))
                    .unwrap(),
                )
                .unwrap();
            }
            _ => {}
        }
        crate::write_session_record(&context, &record).unwrap();
        let status = if case == "active" { "active" } else { "idle" };
        let error = request_exit(context, record, tmux, status)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            if matches!(case, "active" | "attention") {
                "restart-session-not-idle"
            } else {
                "codex-account-not-bound"
            },
            "{case}"
        );
        assert!(
            !tmp.path().join("input").exists(),
            "{case} must not type an exit"
        );
    }
}
