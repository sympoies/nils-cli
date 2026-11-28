use super::*;
use pretty_assertions::assert_eq;

pub(super) fn bind_idle(fixture: &WakeFixture) {
    bind_transcript(fixture);
    let path = fixture.temp.path().join("sessions/beta/activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    activity["state"]["phase"] = json!("waiting");
    activity["state"]["current_turn"] = Value::Null;
    fs::write(path, serde_json::to_vec(&activity).unwrap()).unwrap();
}

#[tokio::test]
async fn claude_configuration_fallback_preserves_drafts_and_delivers_once() {
    use crate::send::claude_notice::attach::{self, Reason};
    let lock = GlobalStateLock::new();
    for reason in [Reason::HostRuleDisabled, Reason::ProviderVersionUnsupported, Reason::ProviderVersionUnreadable] {
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let fixture = WakeFixture::new(AgentKind::Claude).await;
        bind_idle(&fixture);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        attach::record(&fixture.state.context.state_dir, &record, reason, None);
        // A launch without the host rule stays unattached when daemon writers are enabled.
        let detail = match reason {
            Reason::HostRuleDisabled => "notice-receiver-not-attached",
            Reason::ProviderVersionUnsupported | Reason::ProviderVersionUnreadable => "notice-provider-unsupported",
        };
        let draft = "generic operator draft";
        fs::write(fixture.temp.path().join("composer"), draft).unwrap();
        assert_eq!(fixture.check_draft().unwrap_err().code(), "composer-not-empty");
        let observation_path = fixture.temp.path().join("sessions/beta/drafts/observation.json");
        let mut observation: Value = serde_json::from_slice(&fs::read(&observation_path).unwrap()).unwrap();
        observation["unchanged_since"] = json!(1);
        fs::write(observation_path, serde_json::to_vec(&observation).unwrap()).unwrap();
        drain_coordination_notifications(fixture.state.clone()).await;
        fixture.assert_no_input(draft);
        assert!(!fixture.temp.path().join("sessions/beta/drafts/saved").exists());
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "queued");
        assert_eq!(receipt["last_reason_detail"], detail, "{receipt}");
        fs::write(fixture.temp.path().join("composer"), "").unwrap();
        // A draft deferral has a retry deadline; make it due without changing the generation.
        let registry_path = fixture.temp.path().join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        for receipt in registry["notifications"].as_object_mut().unwrap().values_mut() {
            receipt["next_attempt_at_epoch"] = json!(0);
        }
        fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        drain_coordination_notifications(fixture.state.clone()).await;
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
        assert_eq!(receipt["last_reason_detail"], detail);
        assert_eq!(receipt["notified_generation"], 1);
        crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
        assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
    }
}

#[tokio::test]
async fn claude_configuration_fallback_with_daemon_writers_disabled() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
    crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn claude_configuration_fallback_unknown_submission_never_resends() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    fs::write(fixture.temp.path().join("capture-unavailable-after-enter"), "").unwrap();
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown", "{receipt}");
    assert_eq!(receipt["attempted_generation"], 1);
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
    crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn claude_configuration_fallback_preserves_a_draft_typed_during_load() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    fs::write(fixture.temp.path().join("draft-on-load"), "").unwrap();
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("later operator draft");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued", "{receipt}");
    assert_eq!(receipt["attempted_generation"], 0);
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
}

#[tokio::test]
async fn resumed_claude_wake_waits_for_a_late_paste() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    let tmux = &fixture.state.tmux_bin;
    let script = fs::read_to_string(tmux).unwrap().replace(
        "    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())",
        "    subprocess.Popen([sys.executable, '-c', \"import pathlib,sys,time; time.sleep(0.5); r=pathlib.Path(sys.argv[1]); t=(r/'buffer').read_text(); (r/'composer').write_text(t); (r/'cursor-x').write_text(str(2+len(t)))\", str(root)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
    );
    assert!(script.contains("time.sleep(0.5)"));
    fs::write(tmux, script).unwrap();

    drain_coordination_notifications(fixture.state.clone()).await;

    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted");
    assert!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap().is_empty(), "owned wake was not cleared");
    let submitted = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
    assert_eq!(submitted.lines().count(), 1);
    assert!(serde_json::from_str::<String>(submitted.trim()).unwrap() == fs::read_to_string(fixture.temp.path().join("buffer")).unwrap(), "submitted wake differs from the pasted input");
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);
}

#[tokio::test]
async fn resumed_claude_wake_capture_loss_is_recoverable_without_a_second_paste() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    let tmux = &fixture.state.tmux_bin;
    let script = fs::read_to_string(tmux).unwrap().replace("import subprocess", "import subprocess, time").replace(
        "elif command == 'paste-buffer':",
        "elif command == 'paste-buffer':\n    (root / 'paste-time').write_text(str(time.monotonic()))",
    ).replace(
        "    text = (root / 'composer').read_text()\n    if (root / 'unknown-layout-after-enter').exists()",
        "    text = (root / 'composer').read_text()\n    if text and (root / 'paste-time').exists() and time.monotonic() - float((root / 'paste-time').read_text()) < 1.0: sys.exit(1)\n    if (root / 'unknown-layout-after-enter').exists()",
    );
    assert!(script.contains("time.monotonic() - float"));
    fs::write(tmux, script).unwrap();

    drain_coordination_notifications(fixture.state.clone()).await;

    let receipt = notification_fixture(fixture.temp.path());
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);
    if receipt["state"] != "prompt_submitted" {
        assert!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap() == fs::read_to_string(fixture.temp.path().join("buffer")).unwrap(), "wake input was changed");
        assert!(!fixture.temp.path().join("submissions").exists());
    }
    assert_eq!(receipt["state"], "prompt_submitted");
    assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "");
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn resumed_claude_wake_with_unobservable_composer_is_never_replayed() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    let tmux = &fixture.state.tmux_bin;
    let script = fs::read_to_string(tmux).unwrap().replace(
        "    text = (root / 'composer').read_text()\n    if (root / 'unknown-layout-after-enter').exists()",
        "    text = (root / 'composer').read_text()\n    if text and not (root / 'submissions').exists(): sys.exit(1)\n    if (root / 'unknown-layout-after-enter').exists()",
    );
    assert!(script.contains("if text and not (root / 'submissions').exists(): sys.exit(1)"));
    fs::write(tmux, script).unwrap();

    let mut candidate = fixture.candidate.clone();
    candidate.prompt = candidate.text_wake_prompt();
    let mut args = crate::send::unguarded_args(
        "beta".into(), candidate.prompt.clone(), vec![SpecialKey::Enter], false,
        fixture.state.tmux_bin.clone(),
    );
    args.guards.require_idle = true;
    args.guards.require_empty_composer = true;
    args.guards.expect_runtime = Some(candidate.target_incarnation.clone());
    let error = crate::send::send_mailbox_notice(&fixture.state.context, args, &candidate).unwrap_err();
    let details = error.details().unwrap();
    assert_eq!(details["pasted"], true);
    assert_eq!(details["submitted"], false);
    assert_eq!(details["cleared"], false);
    assert_eq!(details["remaining_input"], "unobservable");
    crate::coordination::mark_notification_unknown(&fixture.state.context, &candidate, "submission-outcome-unknown").unwrap();
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown");
    assert!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap() == fs::read_to_string(fixture.temp.path().join("buffer")).unwrap(), "wake input was changed");
    assert!(!fixture.temp.path().join("submissions").exists());
    dispatch_wake(&fixture).await;
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);
}

#[tokio::test]
async fn resumed_claude_wake_never_submits_after_runtime_identity_changes() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    let tmux = &fixture.state.tmux_bin;
    let script = fs::read_to_string(tmux).unwrap().replace(
        "elif command == 'paste-buffer':",
        "elif command == 'paste-buffer':\n    subprocess.Popen([sys.executable, '-c', \"import json,pathlib,sys,time; time.sleep(0.15); p=pathlib.Path(sys.argv[1])/'identities.json'; d=json.loads(p.read_text()); d['AGENT_SESSION_RUNTIME_ID']='replacement'; p.write_text(json.dumps(d))\", str(root)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
    ).replace(
        "    text = (root / 'composer').read_text()\n    if (root / 'unknown-layout-after-enter').exists()",
        "    text = (root / 'composer').read_text()\n    if text and not (root / 'submissions').exists(): sys.exit(1)\n    if (root / 'unknown-layout-after-enter').exists()",
    );
    assert!(script.contains("AGENT_SESSION_RUNTIME_ID']='replacement'"));
    fs::write(tmux, script).unwrap();

    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown");
    assert!(!fixture.temp.path().join("submissions").exists());
    assert!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap() == fs::read_to_string(fixture.temp.path().join("buffer")).unwrap(), "wake input was changed");
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);
}

#[tokio::test]
async fn resumed_claude_wake_never_submits_a_changed_composer() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    let tmux = &fixture.state.tmux_bin;
    let script = fs::read_to_string(tmux).unwrap().replace("import subprocess", "import subprocess, time").replace(
        "elif command == 'paste-buffer':",
        "elif command == 'paste-buffer':\n    (root / 'paste-time').write_text(str(time.monotonic()))\n    subprocess.Popen([sys.executable, '-c', \"import pathlib,sys,time; time.sleep(0.15); (pathlib.Path(sys.argv[1])/'composer').write_text('owner draft')\", str(root)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
    ).replace(
        "    text = (root / 'composer').read_text()\n    if (root / 'unknown-layout-after-enter').exists()",
        "    text = (root / 'composer').read_text()\n    if text and (root / 'paste-time').exists() and time.monotonic() - float((root / 'paste-time').read_text()) < 1.0: sys.exit(1)\n    if (root / 'unknown-layout-after-enter').exists()",
    );
    assert!(script.contains("write_text('owner draft')"));
    fs::write(tmux, script).unwrap();

    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown");
    assert!(!fixture.temp.path().join("submissions").exists());
    assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "owner draft");
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);
}

#[tokio::test]
async fn resumed_claude_wake_cleans_owned_input_after_swallowed_enters() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();

    let mut candidate = fixture.candidate.clone();
    candidate.prompt = candidate.text_wake_prompt();
    let mut args = crate::send::unguarded_args(
        "beta".into(), candidate.prompt.clone(), vec![SpecialKey::Enter], false,
        fixture.state.tmux_bin.clone(),
    );
    args.guards.require_idle = true;
    args.guards.require_empty_composer = true;
    args.guards.expect_runtime = Some(candidate.target_incarnation.clone());
    let error = crate::send::send_mailbox_notice(&fixture.state.context, args, &candidate).unwrap_err();
    assert_eq!(error.code(), "send-submit-stuck");
    let details = error.details().unwrap();
    assert_eq!(details["cleared"], true);
    assert_eq!(details["remaining_input"], "empty");
    crate::coordination::mark_notification_unknown(&fixture.state.context, &candidate, "submission-outcome-unknown").unwrap();
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown");
    assert!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap().is_empty(), "owned wake was not cleared");
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 2);
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|line| line.contains("paste-buffer")).count(), 1);

    fs::remove_file(fixture.temp.path().join("swallow-every-enter")).unwrap();
    let capability = fs::read_to_string(crate::coordination::capability_path(
        &fixture.state.context, "alpha", "launch-alpha",
    )).unwrap();
    send_notification_fixture(&fixture.state, &capability, "next-wake-after-cleanup").await;
    bind_idle(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted");
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 3);
}

fn activate_restarted_claude(fixture: &WakeFixture) -> SessionRecord {
    bind_transcript(fixture);
    let dir = session_dir(&fixture.state.context, "beta");
    let path = dir.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["generation"] = json!(2);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let state = crate::activity::activate_runtime(&fixture.state.context, &record).unwrap();
    assert_eq!(state.phase, crate::activity::TurnPhase::Starting);
    assert!(state.current_turn.is_none());
    record
}

#[tokio::test]
async fn restarted_claude_idle_with_no_turn_evidence_wakes_once() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let record = activate_restarted_claude(&fixture);
    let module_dir =
        crate::send::claude_notice::runtime::module_dir(&fixture.state.context.state_dir, &record)
            .unwrap();
    fs::create_dir_all(module_dir).unwrap();
    let provider = fixture.temp.path().join("fake-claude");
    fs::write(
        &provider,
        "#!/bin/sh\n[ \"$*\" = 'agents --json' ] || exit 42\nexec cat \"$(dirname \"$0\")/native-state.json\"\n",
    )
    .unwrap();
    fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
    let _provider = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", provider.to_str().unwrap());
    let set_status = |status| {
        fs::write(
            fixture.temp.path().join("native-state.json"),
            json!([{"sessionId": "provider-beta", "status": status}]).to_string(),
        )
        .unwrap();
    };

    set_status("busy");
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["last_reason_detail"], "send-session-not-idle");
    assert_eq!(receipt["attempted_generation"], 0);

    set_status("idle");
    let composer = fixture.temp.path().join("composer");
    fs::write(&composer, "operator draft").unwrap();
    allow_notification_retry(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("operator draft");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["last_reason"], "recipient-composer-not-empty");
    assert_eq!(receipt["attempted_generation"], 0);

    fs::write(composer, "").unwrap();
    allow_notification_retry(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;

    assert_single_restart_wake(&fixture).await;
}

#[tokio::test]
async fn restarted_claude_uses_resume_hook_when_native_listing_is_unavailable() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let record = activate_restarted_claude(&fixture);
    let runtime = record.runtime.as_ref().unwrap();
    let payload = serde_json::to_vec(&json!({
        "hook_event_name": "SessionStart", "source": "resume", "session_id": "provider-beta"
    }))
    .unwrap();
    crate::activity::ingest_provider_hook_input(
        &fixture.state.context,
        AgentKind::Claude,
        None,
        crate::activity::ProviderHookInput {
            id: &record.id,
            runtime_id: &runtime.launch_id,
            payload: &payload,
            attention_authority: None,
        },
    )
    .unwrap();
    let state = crate::activity::bound_claude_state(&fixture.state.context, &record).unwrap();
    assert_eq!(state.phase, crate::activity::TurnPhase::Waiting);

    let shadow = json!({
        "schema_version": "agent-session.activity-shadow.v1",
        "runtime_id": runtime.launch_id, "runtime_generation": runtime.generation,
        "activity_revision": state.revision,
        "observation": {"observer_version": "claude-agents.v1",
            "rule_id": "claude_agents_unavailable", "observed_at": jiff::Timestamp::now().to_string(),
            "projection": "unknown", "disagrees": true}
    });
    fs::write(
        session_dir(&fixture.state.context, &record.id).join("activity.shadow.json"),
        serde_json::to_vec(&shadow).unwrap(),
    )
    .unwrap();

    assert_eq!(
        crate::activity::activity_status_for_record(&fixture.state.context, &record)
            .unwrap()
            .turn_state
            .phase,
        crate::activity::TurnPhase::Waiting,
    );
    drain_coordination_notifications(fixture.state.clone()).await;

    assert_single_restart_wake(&fixture).await;
}

fn allow_notification_retry(fixture: &WakeFixture) {
    let path = fixture.temp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for receipt in registry["notifications"].as_object_mut().unwrap().values_mut() {
        receipt["next_attempt_at_epoch"] = json!(0);
    }
    fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
}

async fn assert_single_restart_wake(fixture: &WakeFixture) {
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    drain_coordination_notifications(fixture.state.clone()).await;
    let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
    let lines: Vec<_> = submissions.lines().collect();
    assert_eq!(lines.len(), 1);
    let wake: String = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(wake.lines().count(), 1);
    assert!(wake.ends_with(" message inbox --session beta --format json"));
}
