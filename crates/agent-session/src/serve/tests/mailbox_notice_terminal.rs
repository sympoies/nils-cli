use super::*;
use pretty_assertions::assert_eq;

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
async fn mailbox_notice_unsupported_claude_keeps_existing_terminal_fallback() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_transcript(&fixture);
    let _budget = HoldBudgetGuard::widen(Duration::from_secs(30));
    drain_coordination_notifications(fixture.state.clone()).await;
    let submitted: Vec<String> = fs::read_to_string(fixture.temp.path().join("submissions"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(submitted, ["Mailbox: agent-session message inbox"]);
    assert_eq!(
        notification_fixture(fixture.temp.path())["state"],
        "prompt_submitted"
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
