use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn gate_off_repeated_deferrals_do_not_rewrite_registry() {
    use std::os::unix::fs::MetadataExt;
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let registry = fixture.temp.path().join("coordination/registry.json");
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(), fixture.candidate.clone(),
    ).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["last_reason"], "provider-not-ready");
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
    let before = fs::metadata(&registry).unwrap().ino();
    for _ in 0..2 {
        crate::serve::terminal_notification::dispatch_coordination_notification(
            fixture.state.clone(), fixture.candidate.clone(),
        ).await;
    }
    assert_eq!(fs::metadata(registry).unwrap().ino(), before);
}

#[tokio::test]
async fn gate_off_deferral_records_the_detail_a_stored_reason_lacks_once() {
    use std::os::unix::fs::MetadataExt;
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let registry_path = fixture.temp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    let receipt = registry["notifications"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    receipt["last_reason"] = json!("provider-not-ready");
    receipt.as_object_mut().unwrap().remove("last_reason_detail");
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(), fixture.candidate.clone(),
    ).await;
    assert_eq!(
        notification_fixture(fixture.temp.path())["last_reason_detail"],
        "notice-writers-disabled"
    );
    let before = fs::metadata(&registry_path).unwrap().ino();
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(), fixture.candidate.clone(),
    ).await;
    assert_eq!(fs::metadata(registry_path).unwrap().ino(), before);
}

#[tokio::test]
async fn mailbox_notice_terminal_wakes_an_idle_recipient_holding_only_its_own_claim() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let registry_path = fixture.temp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    registry["claims"].as_array_mut().unwrap().push(json!({
        "schema_version": "agent-session.work-context.v1",
        "session_id": "beta",
        "session_incarnation": fixture.candidate.target_incarnation,
        "claim_id": "terminal-recipient-claim",
        "revision": 1,
        "state": "active",
        "intent": "implementation",
        "tier": "issue",
        "repositories": [],
        "worktrees": [],
        "provider_refs": [],
        "plan_refs": [],
        "scopes": [],
        "summary": "terminal recipient claim fixture",
        "updated_at": "2030-01-01T00:00:00Z",
        "expires_at": "9999-12-31T23:59:59Z",
        "expires_at_epoch": i64::MAX
    }));
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(), fixture.candidate.clone(),
    ).await;
    // The fixture pane never echoes the notice into a transcript, so the outcome
    // stays unknown; what matters is that the claim did not stop the attempt.
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["attempted_generation"], 1, "{receipt}");
}

fn bind_transcript(fixture: &WakeFixture) {
    let provider_root = fixture.temp.path().join("profile-config");
    let projects = provider_root.join("projects/project-27");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("provider-beta.jsonl"),
        format!(
            "{}\n",
            json!({"type":"user", "sessionId":"provider-beta", "cwd":"/repo"})
        ),
    )
    .unwrap();
    let path = session_dir(&fixture.state.context, "beta").join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["agent_profile_provider_config_dir"] = json!(provider_root);
    record["provider_resume"] = json!({"provider":"claude", "session_id":"provider-beta", "capture_method":"test", "captured_at":"2000-01-01T00:00:00Z", "resume_args":[]});
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("emit-transcript-after-enter"), "").unwrap();
}

#[tokio::test]
async fn mailbox_notice_unattached_claude_delivers_once() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    configuration_fallback::bind_idle(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["notified_generation"], 1);
    assert_eq!(receipt["last_reason_detail"], "notice-receiver-not-attached");
    drain_coordination_notifications(fixture.state.clone()).await;
    let submissions: Vec<String> = fs::read_to_string(fixture.temp.path().join("submissions"))
        .unwrap().lines().map(|row| serde_json::from_str(row).unwrap()).collect();
    assert_eq!(submissions.len(), 1);
    let wake = &submissions[0];
    assert!(!wake.contains('\n'), "{wake}");
    let (_, command) = wake.split_once("run ").expect("run command");
    assert!(command.starts_with('/'), "absolute path: {wake}");
    assert!(command.ends_with(" message inbox --session beta --format json"), "{wake}");
}

#[tokio::test]
async fn mailbox_notice_uncertain_claude_capability_does_not_fall_back() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let dir =
        crate::send::claude_notice::runtime::module_dir(&fixture.state.context.state_dir, &record)
            .unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued");
    assert_eq!(receipt["last_reason"], "provider-not-ready");
    assert_eq!(receipt["last_reason_detail"], "notice-runtime-unavailable");
}

mod configuration_fallback {
    include!("mailbox_notice_fallback.rs");
}

#[tokio::test]
async fn mailbox_notice_supported_terminal_preserves_operator_draft_before_claim() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    fs::write(
        fixture.temp.path().join("composer"),
        "operator partial draft",
    )
    .unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let mut candidate = fixture.candidate.clone();
    candidate
        .prepare_notice(&fixture.state.context, &record)
        .unwrap();
    assert!(candidate.is_notice());
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("operator partial draft");
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "queued");
}

#[tokio::test]
async fn wrapped_claude_draft_wake_defers_with_native_composer_reason() {
    use crate::send::claude_notice::{Launch, runtime, sha256};
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_transcript(&fixture);
    let activity_path = fixture.temp.path().join("sessions/beta/activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&activity_path).unwrap()).unwrap();
    activity["state"]["phase"] = json!("waiting");
    activity["state"]["current_turn"] = Value::Null;
    fs::write(activity_path, serde_json::to_vec(&activity).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("composer"), "previous draft").unwrap();
    assert_eq!(fixture.check_draft().unwrap_err().code(), "composer-not-empty");
    let observation = fixture.temp.path().join("sessions/beta/drafts/observation.json");
    assert!(observation.exists());
    let draft = "generic stale draft alpha beta gamma delta\n  epsilon z";
    fs::write(fixture.temp.path().join("composer"), draft).unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let dir = runtime::module_dir(&fixture.state.context.state_dir, &record).unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    for path in [&dir, &dir.join("hooks")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket_root = tempfile::tempdir_in("/tmp").unwrap();
    fs::set_permissions(socket_root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let socket_parent = socket_root.path().join("agent-session");
    fs::create_dir(&socket_parent).unwrap();
    fs::set_permissions(&socket_parent, fs::Permissions::from_mode(0o700)).unwrap();
    let namespace = sha256(format!("{}\0beta\0{}", fixture.state.context.state_dir.display(), fixture.candidate.target_incarnation).as_bytes());
    let socket = socket_parent.join(format!("cn-{}.sock", &namespace[..16]));
    let launch = Launch::from_record(&record).unwrap();
    let module = sha256(runtime::MODULE.as_bytes());
    let nonce = "a".repeat(64);
    let binding = json!({"launch":launch,"provider":"claude","provider_session_id":"provider-beta","provider_version":"2.1.296","module_sha256":module});
    for (name, value) in [
        ("bootstrap.json", json!({"schema_version":"agent-session.claude-notice-runtime.v1","launch":launch,"module_sha256":module,"provider_version":"2.1.296","socket_path":socket,"nonce":nonce})),
        ("ready.json", json!({"schema_version":"agent-session.claude-notice-ready.v1","binding":binding,"nonce":nonce,"terminal":true,"timer_started":true})),
        ("offer.json", json!({"schema_version":"agent-session.claude-notice-offer.v1","state":"idle"})),
    ] {
        crate::write_private_file(&dir.join(name), value.to_string().as_bytes()).unwrap();
    }
    crate::write_private_file(&dir.join("hooks/register.js"), runtime::MODULE.as_bytes()).unwrap();
    assert!(runtime::readiness(&fixture.state.context, &record).is_some());
    let receiver = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let deadline = Instant::now() + Duration::from_secs(10);
        let offer: Value = loop {
            let offer: Value = serde_json::from_slice(&fs::read(dir.join("offer.json")).unwrap()).unwrap();
            if offer["op"].is_string() { break offer; }
            assert!(Instant::now() < deadline, "receiver did not get an offer");
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let body = json!({"schema_version":"agent-session.claude-notice-authorize.v1",
            "op":offer["op"], "binding":binding, "nonce":nonce,
            "terminal":true, "box_empty":false, "cursor":0}).to_string();
        write!(stream, "POST /authorize HTTP/1.1\r\nHost: notice.invalid\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(!response.contains("HTTP/1.1 200 "), "a native draft must receive no notice text");
    });
    drain_coordination_notifications(fixture.state.clone()).await;
    receiver.join().unwrap();
    fixture.assert_no_input(draft);
    assert!(!fixture.temp.path().join("sessions/beta/drafts/saved").exists());
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued", "{receipt}");
    assert_eq!(receipt["last_reason"], "recipient-composer-not-empty", "{receipt}");
    assert_eq!(receipt["last_reason_detail"], Value::Null, "{receipt}");
    assert_eq!(receipt["attempted_generation"], 0);
    assert!(observation.exists(), "native admission must not run screen draft repair");
    let refusal = fixture.check_draft().unwrap_err();
    assert_eq!(refusal.code(), "composer-unobservable");
    assert_eq!(refusal.details().unwrap()["observation"], "region-not-single-row");
    assert!(refusal.message().contains("multi-row composer"));
}

async fn queued_claude_wake_remains_retryable_after_clear(stale: bool) {
    use crate::send::claude_notice::{Launch, runtime, sha256};
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_transcript(&fixture);
    let activity_path = fixture.temp.path().join("sessions/beta/activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&activity_path).unwrap()).unwrap();
    activity["state"]["phase"] = json!("waiting");
    activity["state"]["current_turn"] = Value::Null;
    fs::write(activity_path, serde_json::to_vec(&activity).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("composer"), "previous draft").unwrap();
    assert_eq!(fixture.check_draft().unwrap_err().code(), "composer-not-empty");
    let observation = fixture.temp.path().join("sessions/beta/drafts/observation.json");
    assert!(observation.exists());
    let draft = "generic one row draft";
    fs::write(fixture.temp.path().join("composer"), draft).unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let dir = runtime::module_dir(&fixture.state.context.state_dir, &record).unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    for path in [&dir, &dir.join("hooks")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket_root = tempfile::tempdir_in("/tmp").unwrap();
    fs::set_permissions(socket_root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let socket_parent = socket_root.path().join("agent-session");
    fs::create_dir(&socket_parent).unwrap();
    fs::set_permissions(&socket_parent, fs::Permissions::from_mode(0o700)).unwrap();
    let namespace = sha256(format!("{}\0beta\0{}", fixture.state.context.state_dir.display(), fixture.candidate.target_incarnation).as_bytes());
    let socket = socket_parent.join(format!("cn-{}.sock", &namespace[..16]));
    let launch = Launch::from_record(&record).unwrap();
    let module = sha256(runtime::MODULE.as_bytes());
    let nonce = "a".repeat(64);
    let binding = json!({"launch":launch,"provider":"claude","provider_session_id":"provider-beta","provider_version":"2.1.296","module_sha256":module});
    for (name, value) in [
        ("bootstrap.json", json!({"schema_version":"agent-session.claude-notice-runtime.v1","launch":launch,"module_sha256":module,"provider_version":"2.1.296","socket_path":socket,"nonce":nonce})),
        ("ready.json", json!({"schema_version":"agent-session.claude-notice-ready.v1","binding":binding,"nonce":nonce,"terminal":true,"timer_started":true})),
        ("offer.json", json!({"schema_version":"agent-session.claude-notice-offer.v1","state":"idle"})),
    ] {
        crate::write_private_file(&dir.join(name), value.to_string().as_bytes()).unwrap();
    }
    crate::write_private_file(&dir.join("hooks/register.js"), runtime::MODULE.as_bytes()).unwrap();
    assert!(runtime::readiness(&fixture.state.context, &record).is_some());
    if stale {
        crate::send_guard::drafts::observe_at(&fixture.state.context, &record, Some(draft), 1, 0).unwrap();
    }
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input(draft);
    assert!(!fixture.temp.path().join("sessions/beta/drafts/saved").exists());
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["attempted_generation"], 0);
    // Only the human clears this native draft; the notice never stashes or clears it.
    fs::write(fixture.temp.path().join("composer"), "").unwrap();
    assert_eq!(receipt["last_reason_detail"], "authorization-timeout", "unoffered wake must remain retryable: {receipt}");
    assert_eq!(receipt["state"], "queued", "unoffered wake must remain retryable: {receipt}");
    // Retry discovery honors the receipt backoff; direct redispatch must do so too.
    let retry_at = std::time::UNIX_EPOCH + Duration::from_secs(receipt["next_attempt_at_epoch"].as_u64().unwrap());
    tokio::time::sleep(retry_at.duration_since(std::time::SystemTime::now()).unwrap_or_default()).await;
    assert!(crate::coordination::pending_notifications(&fixture.state.context).unwrap().iter().any(|candidate| candidate.generation == fixture.candidate.generation), "retry candidate is not due; notification receipt: {receipt}");
    let transcript = fixture.temp.path().join("profile-config/projects/project-27/provider-beta.jsonl");
    let receiver = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let deadline = Instant::now() + Duration::from_secs(20);
        let offer: Value = loop {
            let offer: Value = serde_json::from_slice(&fs::read(dir.join("offer.json")).unwrap()).unwrap();
            if offer["op"].is_string() { break offer; }
            assert!(Instant::now() < deadline, "receiver did not get an offer");
            std::thread::sleep(Duration::from_millis(5));
        };
        let post = |path: &str, value: Value| -> Value {
            let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let body = value.to_string();
            write!(stream, "POST {path} HTTP/1.1\r\nHost: notice.invalid\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            let (head, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
            serde_json::from_str(body).unwrap()
        };
        let authorized = post("/authorize", json!({"schema_version":"agent-session.claude-notice-authorize.v1", "op":offer["op"], "binding":binding, "nonce":nonce, "terminal":true, "box_empty":true, "cursor":0}));
        let text = authorized["text"].as_str().unwrap_or_else(|| panic!("authorization response: {authorized}"));
        writeln!(fs::OpenOptions::new().append(true).open(transcript).unwrap(), "{}", json!({"type":"user", "sessionId":"provider-beta", "uuid":"11111111-1111-4111-8111-111111111111", "version":"2.1.296", "promptSource":"system", "isSidechain":false, "origin":{"kind":"plugin", "name":"agent-session-claude-notice", "asUser":true}, "message":{"role":"user", "content":text}})).unwrap();
        post("/result", json!({"schema_version":"agent-session.claude-notice-result.v1", "op":offer["op"], "binding":binding, "nonce":nonce, "outcome":"accepted", "after_empty":true, "returned_text":text, "origin":{"kind":"plugin", "name":"agent-session-claude-notice", "as_user":true}}));
    });
    crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert!(receiver.join().is_ok(), "receiver failed; notification receipt: {receipt}");
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["notified_generation"], 1);
    drain_coordination_notifications(fixture.state.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("profile-config/projects/project-27/provider-beta.jsonl")).unwrap().lines().count(), 2);
}

#[tokio::test]
async fn queued_claude_wake_remains_retryable_after_user_clear() {
    queued_claude_wake_remains_retryable_after_clear(false).await;
}

#[tokio::test]
async fn queued_claude_wake_preserves_stale_native_draft_until_user_clear() {
    queued_claude_wake_remains_retryable_after_clear(true).await;
}

#[tokio::test]
async fn mailbox_notice_collapsed_codex_paste_submits_once() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    fs::write(fixture.temp.path().join("codex-collapse-paste"), "").unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let mut candidate = fixture.candidate.clone();
    candidate.prepare_notice(&fixture.state.context, &record).unwrap();
    let prompt = crate::coordination::notification_prompt(&candidate);
    assert!(prompt.chars().count() > 1000);
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(), candidate,
    ).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["notified_generation"], 1);
    let submissions: Vec<String> = fs::read_to_string(fixture.temp.path().join("submissions"))
        .unwrap().lines().map(|row| serde_json::from_str(row).unwrap()).collect();
    assert_eq!(submissions, [prompt]);
    assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "");
    drain_coordination_notifications(fixture.state.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn stale_codex_wake_saves_draft_and_delivers_once() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let draft = "  café owner draft  ";
    fs::write(fixture.temp.path().join("composer"), draft).unwrap();
    fs::write(
        fixture.temp.path().join("cursor-x"),
        (2 + draft.chars().count()).to_string(),
    )
    .unwrap();
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(),
        fixture.candidate.clone(),
    )
    .await;
    let observed = fixture
        .temp
        .path()
        .join("sessions/beta/drafts/observation.json");
    let mut observation: Value = serde_json::from_slice(&fs::read(&observed).unwrap()).unwrap();
    observation["unchanged_since"] = json!(1);
    fs::write(observed, serde_json::to_vec(&observation).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("codex-collapse-paste"), "").unwrap();
    let receipt = notification_fixture(fixture.temp.path());
    // Match retry discovery rather than relying on composer guard latency.
    let retry_at = std::time::UNIX_EPOCH + Duration::from_secs(receipt["next_attempt_at_epoch"].as_u64().unwrap());
    tokio::time::sleep(retry_at.duration_since(std::time::SystemTime::now()).unwrap_or_default()).await;
    assert!(crate::coordination::pending_notifications(&fixture.state.context).unwrap().iter().any(|candidate| candidate.generation == fixture.candidate.generation), "retry candidate is not due; notification receipt: {receipt}");
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(),
        fixture.candidate.clone(),
    )
    .await;
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let saved = crate::send_guard::drafts::list(&fixture.state.context, &record).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved.into_iter().next().unwrap().text().as_bytes(),
        draft.as_bytes()
    );
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["notified_generation"], 1);
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    drain_coordination_notifications(fixture.state.clone()).await;
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn mailbox_notice_unbound_terminal_codex_settles_without_unbounded_retry() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    assert!(
        record.provider_resume.is_none(),
        "fixture models a terminal-started record with no provider binding"
    );
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_ne!(receipt["state"], "queued", "{receipt}");
    assert_ne!(
        receipt["last_reason"], "provider-observation-unavailable",
        "{receipt}"
    );
    assert_eq!(receipt["attempted_generation"], 1, "{receipt}");
    drain_coordination_notifications(fixture.state.clone()).await;
    assert_eq!(
        notification_fixture(fixture.temp.path()),
        receipt,
        "a settled wake is not retried"
    );
}

async fn dispatch_wake(fixture: &WakeFixture) {
    crate::serve::terminal_notification::dispatch_coordination_notification(
        fixture.state.clone(),
        fixture.candidate.clone(),
    )
    .await;
}

fn bind_codex_provider(fixture: &WakeFixture) {
    let path = session_dir(&fixture.state.context, "beta").join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["provider_resume"] = json!({"provider":"codex", "session_id":"provider-beta", "capture_method":"test", "captured_at":"2000-01-01T00:00:00Z", "resume_args":[]});
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

#[tokio::test]
async fn mailbox_notice_unbound_terminal_codex_unobservable_composer_settles_undeliverable() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    assert!(record.provider_resume.is_none());
    fs::write(fixture.temp.path().join("capture-unavailable"), "").unwrap();
    // The fifth consecutive refusal settles the wake; earlier ones retry.
    for refusal in 1..=4 {
        dispatch_wake(&fixture).await;
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "queued", "refusal {refusal}: {receipt}");
        assert_eq!(
            receipt["last_reason"], "recipient-composer-unobservable",
            "refusal {refusal}: {receipt}"
        );
    }
    dispatch_wake(&fixture).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "undeliverable", "{receipt}");
    assert_eq!(receipt["last_reason"], "recipient-composer-unobservable", "{receipt}");
    fixture.assert_no_input("");
    dispatch_wake(&fixture).await;
    assert_eq!(
        notification_fixture(fixture.temp.path()),
        receipt,
        "a settled wake is not retried"
    );
}

#[tokio::test]
async fn mailbox_notice_bound_terminal_codex_unobservable_composer_keeps_retrying() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    bind_codex_provider(&fixture);
    fs::write(fixture.temp.path().join("capture-unavailable"), "").unwrap();
    for _ in 0..6 {
        dispatch_wake(&fixture).await;
    }
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued", "{receipt}");
    fixture.assert_no_input("");
}

#[tokio::test]
async fn mailbox_notice_unbound_terminal_codex_draft_refusal_keeps_retrying() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    assert!(record.provider_resume.is_none());
    fs::write(fixture.temp.path().join("composer"), "operator partial draft").unwrap();
    for _ in 0..6 {
        dispatch_wake(&fixture).await;
    }
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued", "{receipt}");
    assert_eq!(receipt["last_reason"], "provider-not-ready", "{receipt}");
    fixture.assert_no_input("operator partial draft");
}
