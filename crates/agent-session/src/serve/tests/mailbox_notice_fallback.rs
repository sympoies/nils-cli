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
    for reason in [Reason::WritersDisabled, Reason::ProviderVersionUnsupported, Reason::ProviderVersionUnreadable] {
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let fixture = WakeFixture::new(AgentKind::Claude).await;
        bind_idle(&fixture);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        attach::record(&fixture.state.context.state_dir, &record, reason, None);
        // A launch without writers stays unattached even if the daemon now enables them.
        let detail = match reason {
            Reason::WritersDisabled => "notice-writers-disabled",
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
    assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "");
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

    drain_coordination_notifications(fixture.state.clone()).await;
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
