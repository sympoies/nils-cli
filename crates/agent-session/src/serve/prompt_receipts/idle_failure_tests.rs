use super::*;
use codex_app_server::guarded::IdleReason;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn native_idle_reason_survives_durable_refusal_and_lookup() {
    let temp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    fs::create_dir_all(session_dir(&context, "fixture-session")).unwrap();
    let receipt = Receipt::unknown(
        "fixture-session".into(),
        "runtime-one".into(),
        "attempt-one".into(),
    );
    let store::ClaimResult::New(claim) = store::claim(
        &context,
        receipt.clone(),
        &store::fingerprint("synthetic notice", true),
    )
    .unwrap() else {
        panic!("new claim")
    };
    let error = IdleReason::ThreadReadRpcFailed.annotate(CliError::runtime(
        "send-idle-unobservable",
        "native idle unavailable",
        Some(json!({"stage":"admission", "pasted":false, "submitted":false})),
    ));
    let result = classify(terminal::failure(error), receipt.clone()).await;
    assert_eq!(result.receipt.outcome, Outcome::Refused);
    store::finish(claim, &result).unwrap();
    let loaded = store::lookup(&context, &receipt).unwrap().unwrap();
    let response = loaded.response("fixture-machine");
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["error"]["details"]["receipt"]["native_idle_reason"],
        "thread_read_rpc_failed"
    );
    assert_eq!(
        value["error"]["hint"],
        IdleReason::ThreadReadRpcFailed.hint()
    );
    assert_eq!(
        loaded.receipt.code.as_deref(),
        Some("send-idle-unobservable")
    );
}

#[tokio::test]
async fn native_idle_refusal_record_drops_unrecognized_reason_content() {
    let receipt = Receipt::unknown(
        "fixture-session".into(),
        "runtime-one".into(),
        "attempt-one".into(),
    );
    let response = terminal::failure(CliError::runtime(
        "send-idle-unobservable",
        "idle unavailable",
        Some(
            json!({"stage":"admission", "pasted":false, "submitted":false,
            "native_idle_reason":"private-provider-status"}),
        ),
    ));
    let result = classify(response, receipt).await;
    let bytes = serde_json::to_string(&result).unwrap();
    assert!(!bytes.contains("private-provider-status"));
    assert!(result.receipt.native_idle_reason.is_none());
}
