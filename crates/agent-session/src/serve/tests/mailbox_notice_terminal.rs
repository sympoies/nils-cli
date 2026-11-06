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
    assert_eq!(notification_fixture(fixture.temp.path())["last_reason"], "provider-not-ready");
    let before = fs::metadata(&registry).unwrap().ino();
    for _ in 0..2 {
        crate::serve::terminal_notification::dispatch_coordination_notification(
            fixture.state.clone(), fixture.candidate.clone(),
        ).await;
    }
    assert_eq!(fs::metadata(registry).unwrap().ino(), before);
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
