use super::*;
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn native_idle_notice_failure_is_persisted_before_attempt_and_stays_retryable() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let context = &fixture.state.context;
    let mut record = load_session_record(context, "beta").unwrap();
    let runtime = record.runtime.as_mut().unwrap();
    runtime.kind = codex_app_server::RUNTIME_KIND.into();
    runtime.extra.insert(
        codex_app_server::PROTOCOL_KEY.into(),
        json!(codex_app_server::PROTOCOL_VERSION),
    );
    for (key, suffix) in [
        (codex_app_server::SOCKET_KEY, "unavailable.sock"),
        (codex_app_server::PROXY_KEY, "proxy"),
        (codex_app_server::THREAD_HANDOFF_KEY, "thread"),
        (codex_app_server::THREAD_ATTACHED_KEY, "attached"),
    ] {
        runtime
            .extra
            .insert(key.into(), json!(fixture.temp.path().join(suffix)));
    }
    // An otherwise bound, waiting synthetic session has no app-server socket.
    // Admission must fail before capability, account or composer mutation.
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"codex", "session_id":"provider-beta", "captured_at":"2000-01-01T00:00:00Z",
            "capture_method":"fixture", "resume_args":[]
        }))
        .unwrap(),
    );
    let digest = Sha256::digest(b"agent-session-codex-thread-v1\0provider-beta");
    fs::write(
        fixture.temp.path().join("attached"),
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
    .unwrap();
    crate::write_session_record(context, &record).unwrap();
    let mut candidate = fixture.candidate.clone();
    candidate.prepare_notice(context, &record).unwrap();
    crate::serve::terminal_notification::dispatch_native_notice_for_test(
        fixture.state.clone(),
        record,
        candidate,
    )
    .await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued");
    assert_eq!(receipt["attempted_generation"], 0);
    assert_eq!(receipt["last_reason"], "provider-not-ready");
    assert_eq!(
        receipt["last_reason_detail"],
        "send-idle-unobservable:connect_failed"
    );
    assert!(!receipt.to_string().contains("unavailable.sock"));
    assert!(!fixture.temp.path().join("submissions").exists());
    assert!(!fixture.temp.path().join("buffer").exists());
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        ""
    );
    // The generation remains eligible after the normal retry delay.
    let registry_path = context.state_dir.join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    for notification in registry["notifications"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        notification["next_attempt_at_epoch"] = json!(0);
    }
    fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert!(
        crate::coordination::pending_notifications(context)
            .unwrap()
            .iter()
            .any(|pending| pending.generation == fixture.candidate.generation)
    );
}
