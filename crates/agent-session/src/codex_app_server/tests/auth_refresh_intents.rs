use super::*;
use pretty_assertions::assert_eq;

async fn refresh_with_intent(state: &str, allowed: bool) {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$1"
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"intent-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker, calls.clone()]).unwrap(),
    );
    let (context, expected) = recovery_fixture(&tmp);
    let mut record = crate::load_session_record(&context, &expected.id).unwrap();
    let intent = json!({
        "schema_version": crate::codex_account::NEXT_SCHEMA_VERSION,
        "account": "acct2",
        "revision": 1,
        "intent_id": "fixture-intent",
        "state": state,
        "applying_runtime_id": record.runtime.as_ref().unwrap().launch_id,
        "updated_at": Timestamp::now().to_string()
    });
    // An unknown state is an Invalid intent, which cannot be applied.
    record
        .extra
        .insert("codex_account_next".into(), intent.clone());
    crate::write_session_record(&context, &record).unwrap();
    let before = fs::read(crate::session_dir(&context, &record.id).join("session.json")).unwrap();
    let mut sink = RecordingMessageSink::default();
    let result = respond_to_external_auth_refresh(
        &mut sink,
        &json!({
            "id": "intent-refresh",
            "method": "account/chatgptAuthTokens/refresh",
            "params": {"reason": "unauthorized"}
        }),
        // The control loop's snapshot predates the intent; admission must
        // inspect the durable record under the session lock.
        Some((&context, &expected, "acct1", "fixture-connection")),
    )
    .await;
    let receipt_path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    if allowed {
        assert_eq!(result, Ok(true), "intent state: {state}");
        assert_eq!(sink.messages.len(), 1);
        let Message::Text(text) = &sink.messages[0] else {
            panic!("expected credential response");
        };
        let response: Value = serde_json::from_str(text).unwrap();
        assert_eq!(response["id"], "intent-refresh");
        assert_eq!(response["result"]["accessToken"], "intent-fixture-token");
        assert!(response.get("method").is_none());
        let calls = fs::read_to_string(&calls).unwrap();
        assert_eq!(calls.lines().count(), 1);
        assert!(calls.ends_with("resolve --account acct1 --force-refresh --format json\n"));
        let receipt: Value = serde_json::from_slice(&fs::read(receipt_path).unwrap()).unwrap();
        assert_eq!(receipt["operations"][0]["state"], "credentials_supplied");
        assert_eq!(receipt["operations"][0]["replayed"], false);
    } else {
        assert_eq!(result, Err("Codex refresh account-switch-pending".into()));
        assert!(sink.messages.is_empty());
        assert!(!calls.exists());
        assert!(!receipt_path.exists());
        assert!(crate::auth_incident::view(&context, &record).is_none());
        assert_eq!(
            fs::read(crate::session_dir(&context, &record.id).join("session.json")).unwrap(),
            before
        );
    }
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(current.extra["codex_account_next"], intent);
    let binding = crate::codex_account::view_for_record(&current);
    assert_eq!(binding.state, "bound");
    assert_eq!(binding.selected_account.as_deref(), Some("acct1"));
    assert_eq!(binding.revision, if allowed { 2 } else { 1 });
    assert_eq!(
        serde_json::to_value(&current.runtime).unwrap(),
        serde_json::to_value(&record.runtime).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&current.provider_resume).unwrap(),
        serde_json::to_value(&record.provider_resume).unwrap()
    );
    // Refresh does not apply or cancel the intent or authorize a new prompt.
    assert!(crate::codex_account::ensure_input_allowed(&current).is_err());
}

#[tokio::test]
async fn external_auth_refresh_allows_failed_intent() {
    refresh_with_intent("failed", true).await;
}

#[tokio::test]
async fn external_auth_refresh_allows_invalid_intent() {
    refresh_with_intent("unknown", true).await;
}

#[tokio::test]
async fn external_auth_refresh_refuses_queued_and_applying_intents() {
    for state in ["queued", "applying"] {
        refresh_with_intent(state, false).await;
    }
}
