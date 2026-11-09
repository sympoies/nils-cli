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
async fn mailbox_notice_unsupported_claude_sends_nothing() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_transcript(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("");
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "queued");
    assert_eq!(notification_fixture(fixture.temp.path())["notified_generation"], 0);
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
