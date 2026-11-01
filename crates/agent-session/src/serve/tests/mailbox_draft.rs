use pretty_assertions::assert_eq;

use super::*;
use crate::serve::terminal_notification::HoldBudgetGuard;

struct WakeFixture {
    temp: tempfile::TempDir,
    state: Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
    capability: String,
    agent: AgentKind,
}

impl WakeFixture {
    async fn new(agent: AgentKind) -> Self {
        let temp = tempfile::TempDir::new().expect("fixture");
        seed_session_with_runtime(temp.path(), "alpha", "codex", "modeled-alpha");
        seed_session_with_runtime(temp.path(), "beta", agent.as_str(), "modeled-beta");
        fs::write(temp.path().join("composer"), "").expect("composer");
        fs::write(temp.path().join("calls"), "").expect("calls");
        fs::write(temp.path().join("provider"), agent.as_str()).expect("provider");
        fs::write(temp.path().join("transcript"), "").expect("transcript");
        let tmux = temp.path().join("tmux");
        fs::write(&tmux, r#"#!/usr/bin/env python3
import json, pathlib, sys
import subprocess
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'calls').open('a') as out: out.write(json.dumps(args) + '\n')
command = args[0]
if command == 'has-session':
    if (root / 'exited-after-enter').exists():
        sys.stderr.write("can't find session: beta\n")
        sys.exit(1)
    if (root / 'unknown-after-enter').exists():
        sys.stderr.write("status probe failed\n")
        sys.exit(1)
    sys.exit(0)
if command == 'display-message': print('0'); sys.exit(0)
if command == 'show-environment':
    identities = json.loads((root / 'identities.json').read_text())
    print('\n'.join(f'{key}={identities[key]}' for key in args if key in identities))
    sys.exit(0)
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
    if (root / 'draft-on-load').exists(): (root / 'composer').write_text('later operator draft')
elif command == 'paste-buffer':
    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
    (root / 'cursor-x').write_text(str(2 + len((root / 'composer').read_text())))
    if (root / 'working-after-paste').exists():
        subprocess.Popen([sys.executable, str(root / 'hold-ingress.py')], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if (root / 'draft-after-paste').exists():
        with (root / 'composer').open('a') as out: out.write(' operator addition')
elif command == 'send-keys' and args[-1] == 'Enter':
    submitted = (root / 'composer').read_text()
    with (root / 'submissions').open('a') as out: out.write(json.dumps(submitted) + '\n')
    if (root / 'emit-transcript-after-enter').exists():
        count_file = root / 'enter-count'
        count = int(count_file.read_text()) if count_file.exists() else 0
        count_file.write_text(str(count + 1))
        with (root / 'profile-config/projects/project-27/provider-beta.jsonl').open('a') as out:
            out.write(json.dumps({"type":"user", "sessionId":"provider-beta", "isSidechain":False, "isMeta":False, "promptSource":"typed", "message":{"role":"user", "content":submitted}}) + '\n')
    if (root / 'exit-after-next-enter').exists(): (root / 'exited-after-enter').touch()
    if (root / 'liveness-unknown-after-enter').exists(): (root / 'unknown-after-enter').touch()
    if (root / 'operator-after-first-enter').exists() and not (root / 'first-enter-seen').exists():
        (root / 'first-enter-seen').touch()
        with (root / 'composer').open('a') as out: out.write(' operator addition')
    elif (root / 'swallow-first-enter').exists(): (root / 'swallow-first-enter').unlink()
    elif (root / 'swallow-every-enter').exists(): pass
    elif (root / 'keep-after-enter').exists(): pass
    else: (root / 'composer').write_text('')
elif command == 'send-keys' and args[-1] == 'Delete':
    text = (root / 'composer').read_text()
    count = int(args[-2])
    prompt = (root / 'buffer').read_text()
    if text.startswith(prompt): (root / 'composer').write_text(text[len(prompt):])
elif command == 'send-keys' and args[-1] == 'C-a': (root / 'cursor-x').write_text('2')
elif command == 'send-keys' and args[-1] == 'Right': (root / 'cursor-x').write_text(str(int((root / 'cursor-x').read_text()) + int(args[-2])))
elif command == 'send-keys' and args[-1] == 'BSpace':
    text = (root / 'composer').read_text()
    count = int(args[-2])
    cursor = int((root / 'cursor-x').read_text()) - 2
    (root / 'composer').write_text(text[:max(0, cursor-count)] + text[cursor:])
elif command == 'capture-pane':
    if (root / 'capture-unavailable').exists() or ((root / 'capture-unavailable-after-enter').exists() and (root / 'submissions').exists()): sys.exit(1)
    text = (root / 'composer').read_text()
    if (root / 'unknown-layout-after-enter').exists() and (root / 'submissions').exists():
        count_file = root / 'unknown-layout-captures'
        count = int(count_file.read_text()) if count_file.exists() else 0
        if count < 2:
            count_file.write_text(str(count + 1))
            print('unknown pane layout')
            sys.exit(0)
        (root / 'unknown-layout-after-enter').unlink()
    if (root / 'provider').read_text() == 'claude':
        print('────────────────────\n❯\u00a0' + text + '\n────────────────────\n  status')
    else: print('› ' + text + '\n\n  status')
    if 'display-message' in args:
        x = int((root / 'cursor-x').read_text()) if (root / 'cursor-x').exists() else 2
        print(f'{x} 1')
elif command != 'delete-buffer': sys.exit(1)
"#).expect("tmux");
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).expect("permissions");
        let state = state(temp.path(), Some(TOKEN), tmux);
        let alpha = load_session_record(&state.context, "alpha").expect("sender");
        let beta = load_session_record(&state.context, "beta").expect("recipient");
        fs::write(temp.path().join("cursor-x"), "2").expect("cursor");
        fs::write(
            temp.path().join("identities.json"),
            serde_json::to_vec(&json!({
                "AGENT_SESSION_ID": beta.id,
                "AGENT_SESSION_STATE_DIR": crate::display_path(&state.context.state_dir),
                "AGENT_SESSION_RUNTIME_ID": beta.runtime.as_ref().expect("runtime").launch_id,
            }))
            .expect("identity json"),
        )
        .expect("write identities");
        let capability = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &beta);
        crate::activity::activate_runtime(&state.context, &beta).expect("activate");
        let runtime = &beta.runtime.as_ref().expect("runtime").launch_id;
        let kinds: &[&str] = if agent == AgentKind::Claude {
            &["stop_observed"]
        } else {
            &["turn_started", "turn_completed"]
        };
        for kind in kinds {
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": format!("draft-fixture-{kind}"),
                "runtime_id": runtime, "provider": agent.as_str(),
                "provider_session_id": "provider-beta", "provider_turn_id": "fixture-turn",
                "kind": kind, "confidence": "authoritative"
            }))
            .expect("event");
            crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
        }
        let path = session_dir(&state.context, &beta.id).join("activity.json");
        let mut activity: Value =
            serde_json::from_slice(&fs::read(&path).expect("activity")).expect("json");
        activity["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
        fs::write(path, serde_json::to_vec(&activity).expect("json")).expect("activity");
        send_notification_fixture(&state, capability.trim(), "draft-wake-fixture").await;
        let candidate = crate::coordination::pending_notifications(&state.context)
            .expect("notifications")
            .pop()
            .expect("candidate");
        Self {
            temp,
            state,
            candidate,
            capability: capability.trim().to_string(),
            agent,
        }
    }

    fn start(&self) -> Result<ProviderPromptTail, TerminalNotificationStartError> {
        let kind = if self.agent == AgentKind::Claude {
            ProviderKind::Claude
        } else {
            ProviderKind::Codex
        };
        let tail = ProviderPromptTail::open_path(
            kind,
            "provider-beta",
            self.temp.path().join("transcript"),
            Duration::ZERO,
        )
        .expect("tail");
        start_terminal_coordination_notification(
            &self.state.context,
            &self.state.tmux_bin,
            &self.candidate,
            &crate::coordination::notification_prompt(&self.candidate),
            tail,
            self.agent,
        )
    }

    fn assert_no_input(&self, draft: &str) {
        assert_eq!(
            fs::read_to_string(self.temp.path().join("composer")).unwrap(),
            draft
        );
        assert!(
            !self.temp.path().join("submissions").exists(),
            "operator input must never be submitted"
        );
        let calls = fs::read_to_string(self.temp.path().join("calls")).unwrap();
        assert!(
            !calls.contains("paste-buffer"),
            "deferred wake must not paste"
        );
        assert!(
            !calls.contains("send-keys"),
            "deferred wake must not press a key"
        );
    }
}

async fn mailbox_wake_draft_defers_then_delivers_after_operator_clears(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(
        fixture.temp.path().join("composer"),
        "operator partial draft",
    )
    .unwrap();
    assert!(
        matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-not-empty"
            ))
        ),
        "draft must defer {agent:?}"
    );
    fixture.assert_no_input("operator partial draft");
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "queued");
    fs::write(fixture.temp.path().join("composer"), "").unwrap();
    fixture
        .start()
        .expect("empty composer accepts pending wake");
    let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
    assert_eq!(submissions.lines().count(), 1);
    let submitted: String = serde_json::from_str(submissions.lines().next().unwrap()).unwrap();
    assert_eq!(
        submitted,
        crate::coordination::notification_prompt(&fixture.candidate)
    );
    assert!(!submitted.contains("operator partial draft"));
}

async fn mailbox_wake_draft_during_buffer_load_refuses_without_paste(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("draft-on-load"), "").unwrap();
    assert!(
        matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-not-empty"
            ))
        ),
        "late draft must defer {agent:?}"
    );
    fixture.assert_no_input("later operator draft");
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "queued");
}

async fn mailbox_wake_operator_addition_after_paste_never_presses_enter(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("draft-after-paste"), "").unwrap();
    let result = fixture.start();
    if agent == AgentKind::Claude {
        assert!(matches!(
            result,
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-not-empty"
            ))
        ));
    } else {
        assert!(matches!(
            result,
            Err(TerminalNotificationStartError::UnknownReason(
                "recipient-composer-not-empty"
            ))
        ));
        fixture
            .candidate
            .mark_unknown_after_paste(
                &fixture.state.context,
                "recipient-composer-not-empty",
            )
        .expect("record mixed-composer unknown state");
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "attempt_unknown");
        assert_eq!(receipt["last_reason"], "recipient-composer-not-empty");
    }
    assert!(!fixture.temp.path().join("submissions").exists());
    let remaining = fs::read_to_string(fixture.temp.path().join("composer")).unwrap();
    let expected = if agent == AgentKind::Claude {
        " operator addition".to_string()
    } else {
        format!(
            "{} operator addition",
            crate::coordination::notification_prompt(&fixture.candidate)
        )
    };
    assert_eq!(remaining, expected);
    assert!(fs::read_to_string(fixture.temp.path().join("calls"))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Vec<String>>(line).ok())
        .all(|args| args.first().map(String::as_str) != Some("send-keys") || args.last().map(String::as_str) != Some("Enter")));
}

async fn mailbox_wake_unobservable_composer_defers_without_input(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("capture-unavailable"), "").unwrap();
    assert!(
        matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-unobservable"
            ))
        ),
        "unknown composer must defer {agent:?}"
    );
    fixture.assert_no_input("");
}

#[tokio::test]
async fn mailbox_wake_draft_defers_then_delivers_after_operator_clears_claude() {
    mailbox_wake_draft_defers_then_delivers_after_operator_clears(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_draft_defers_then_delivers_after_operator_clears_codex() {
    mailbox_wake_draft_defers_then_delivers_after_operator_clears(AgentKind::Codex).await;
}

#[tokio::test]
async fn mailbox_wake_draft_during_buffer_load_refuses_without_paste_claude() {
    mailbox_wake_draft_during_buffer_load_refuses_without_paste(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_draft_during_buffer_load_refuses_without_paste_codex() {
    mailbox_wake_draft_during_buffer_load_refuses_without_paste(AgentKind::Codex).await;
}

#[tokio::test]
async fn mailbox_wake_operator_addition_after_paste_never_presses_enter_claude() {
    mailbox_wake_operator_addition_after_paste_never_presses_enter(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_operator_addition_after_paste_never_presses_enter_codex() {
    mailbox_wake_operator_addition_after_paste_never_presses_enter(AgentKind::Codex).await;
}

#[tokio::test]
async fn mailbox_wake_unobservable_composer_defers_without_input_claude() {
    mailbox_wake_unobservable_composer_defers_without_input(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_unobservable_composer_defers_without_input_codex() {
    mailbox_wake_unobservable_composer_defers_without_input(AgentKind::Codex).await;
}

#[tokio::test]
async fn mailbox_wake_empty_composer_control_submits_once() {
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = WakeFixture::new(agent).await;
        fixture.start().expect("empty composer control");
        let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
        assert_eq!(submissions.lines().count(), 1);
        let submitted: String = serde_json::from_str(submissions.lines().next().unwrap()).unwrap();
        assert_eq!(
            submitted,
            crate::coordination::notification_prompt(&fixture.candidate)
        );
    }
}

#[tokio::test]
async fn mailbox_wake_retries_a_swallowed_enter_and_confirms_submission() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let _budget = HoldBudgetGuard::widen(Duration::from_secs(30));
    fs::write(fixture.temp.path().join("swallow-first-enter"), "").unwrap();
    fixture.start().expect("swallowed Enter is retried");
    let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
    assert_eq!(submissions.lines().count(), 2);
    assert!(submissions.lines().all(|line| {
        serde_json::from_str::<String>(line).unwrap()
            == crate::coordination::notification_prompt(&fixture.candidate)
    }));
}

#[tokio::test]
async fn mailbox_wake_does_not_retry_after_operator_edits_composer() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    fs::write(fixture.temp.path().join("operator-after-first-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known(
            "submission-not-confirmed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(
        fs::read_to_string(fixture.temp.path().join("composer"))
            .unwrap()
            .ends_with(" operator addition")
    );
}

#[tokio::test]
async fn mailbox_wake_reports_known_failure_after_two_swallowed_enters() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let _budget = HoldBudgetGuard::widen(Duration::from_secs(30));
    fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known(
            "submission-not-confirmed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "queued");
    assert!(
        fs::read_to_string(fixture.temp.path().join("composer"))
            .unwrap()
            .is_empty(),
        "a refused wake must not remain in the composer"
    );
}

#[tokio::test]
async fn mailbox_wake_stuck_submit_does_not_leave_notification_in_claude_composer() {
    mailbox_wake_stuck_submit_does_not_leave_notification_in_composer(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_stuck_submit_does_not_leave_notification_in_codex_composer() {
    mailbox_wake_stuck_submit_does_not_leave_notification_in_composer(AgentKind::Codex).await;
}

async fn mailbox_wake_stuck_submit_does_not_leave_notification_in_composer(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known("submission-not-confirmed"))
    ));
    assert!(
        fs::read_to_string(fixture.temp.path().join("composer"))
            .unwrap()
            .is_empty(),
        "a refused wake must not remain in the {agent:?} composer"
    );
}

#[tokio::test]
async fn mailbox_wake_working_after_paste_clears_owned_prompt_claude() {
    mailbox_wake_working_after_paste_clears_owned_prompt(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_working_after_paste_clears_owned_prompt_codex() {
    mailbox_wake_working_after_paste_clears_owned_prompt(AgentKind::Codex).await;
}

async fn mailbox_wake_working_after_paste_clears_owned_prompt(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    let hold_ingress = format!(
        "import fcntl, time\nf = open({}, 'a')\nfcntl.flock(f, fcntl.LOCK_SH)\ntime.sleep(5)\n",
        serde_json::to_string(
            &fixture
                .temp
                .path()
                .join("sessions/beta/.activity-ingress.lock")
                .to_string_lossy()
                .to_string()
        )
        .unwrap()
    );
    fs::write(fixture.temp.path().join("hold-ingress.py"), hold_ingress).unwrap();
    fs::write(fixture.temp.path().join("working-after-paste"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known("recipient-working"))
    ));
    assert!(
        fs::read_to_string(fixture.temp.path().join("composer"))
            .unwrap()
            .is_empty(),
        "post-paste deferral must clear its exact notification for {agent:?}"
    );
    assert!(fs::read_to_string(fixture.temp.path().join("calls"))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Vec<String>>(line).ok())
        .all(|args| args.first().map(String::as_str) != Some("send-keys") || args.last().map(String::as_str) != Some("Enter")));
}

#[tokio::test]
async fn mailbox_wake_keeps_unreadable_pane_unverified_after_enter() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    fs::write(
        fixture.temp.path().join("capture-unavailable-after-enter"),
        "",
    )
    .unwrap();
    fs::write(fixture.temp.path().join("keep-after-enter"), "").unwrap();
    fs::write(fixture.temp.path().join("liveness-unknown-after-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::UnknownReason(
            "submission-not-confirmed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        crate::coordination::notification_prompt(&fixture.candidate)
    );
    fixture
        .candidate
        .mark_unknown_after_paste(
            &fixture.state.context,
            "submission-not-confirmed",
        )
    .expect("record visible unknown state");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown");
    assert_eq!(receipt["last_reason"], "submission-not-confirmed");
}

#[tokio::test]
async fn mailbox_wake_retry_clears_only_prior_unknown_notification_text() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let prior_prompt = crate::coordination::notification_prompt(&fixture.candidate);
    fs::write(
        fixture.temp.path().join("capture-unavailable-after-enter"),
        "",
    )
    .unwrap();
    fs::write(fixture.temp.path().join("keep-after-enter"), "").unwrap();
    fs::write(fixture.temp.path().join("liveness-unknown-after-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::UnknownReason(
            "submission-not-confirmed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        prior_prompt,
        "an unverified notification remains visible until a later readable retry"
    );
    fixture
        .candidate
        .mark_unknown_after_paste(
            &fixture.state.context,
            "submission-not-confirmed",
        )
    .expect("record unknown outcome");
    fs::remove_file(fixture.temp.path().join("capture-unavailable-after-enter")).unwrap();
    fs::remove_file(fixture.temp.path().join("liveness-unknown-after-enter")).unwrap();
    fs::remove_file(fixture.temp.path().join("unknown-after-enter")).unwrap();
    fs::remove_file(fixture.temp.path().join("keep-after-enter")).unwrap();

    let provider_root = fixture.temp.path().join("profile-config");
    let provider_projects = provider_root.join("projects/project-27");
    fs::create_dir_all(&provider_projects).unwrap();
    fs::write(
        provider_projects.join("provider-beta.jsonl"),
        format!("{}\n", json!({"type":"user", "sessionId":"provider-beta", "cwd":"/repo"})),
    )
    .unwrap();
    let record_path = session_dir(&fixture.state.context, &fixture.candidate.target_session_id)
        .join("session.json");
    let mut persisted: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    persisted["runtime"]["agent_profile_provider_config_dir"] = json!(provider_root);
    persisted["provider_resume"] = json!({
        "provider":"claude", "session_id":"provider-beta", "capture_method":"test",
        "captured_at":"2000-01-01T00:00:00Z", "resume_args":[]
    });
    fs::write(&record_path, serde_json::to_vec_pretty(&persisted).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("emit-transcript-after-enter"), "").unwrap();
    // The first wake above waited out the real budget; the recovery wake needs
    // room for its cleanup, paste and Enter on a loaded host.
    let _budget = HoldBudgetGuard::widen(Duration::from_secs(30));

    for id in ["alpha", "beta"] {
        let record = load_session_record(&fixture.state.context, id).expect("fixture session");
        let incarnation = record
            .runtime
            .as_ref()
            .expect("fixture runtime")
            .launch_id
            .as_str();
        let heartbeat = crate::coordination::heartbeat_path(
            &fixture.state.context.state_dir,
            &record.id,
        );
        fs::write(
            heartbeat,
            format!("{incarnation}:{}\n", crate::coordination::now_epoch()),
        )
        .expect("refresh fixture heartbeat");
    }

    send_notification_fixture(
        &fixture.state,
        &fixture.capability,
        "draft-wake-recovery-fixture",
    )
    .await;
    let candidate = crate::coordination::pending_notifications(&fixture.state.context)
        .expect("pending recovery generation")
        .pop()
        .expect("recovery generation");
    let current_prompt = crate::coordination::notification_prompt(&candidate);
    let recipient = load_session_record(&fixture.state.context, "beta").expect("recipient");
    dispatch_terminal_coordination_notification(
        fixture.state.clone(),
        recipient,
        candidate.clone(),
        AgentKind::Claude,
    )
    .await;
    assert!(fs::read_to_string(fixture.temp.path().join("composer"))
        .unwrap()
        .is_empty());
    let notification = notification_fixture(fixture.temp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], candidate.generation);
    let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
    let submissions = submissions
        .lines()
        .map(|line| serde_json::from_str::<String>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(submissions, [prior_prompt, current_prompt]);
}

#[tokio::test]
async fn mailbox_wake_retry_preserves_mixed_codex_composer_as_unknown() {
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    fs::write(fixture.temp.path().join("draft-after-paste"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::UnknownReason(
            "recipient-composer-not-empty"
        ))
    ));
    fixture
        .candidate
        .mark_unknown_after_paste(
            &fixture.state.context,
            "recipient-composer-not-empty",
        )
    .expect("record mixed composer outcome");
    let old_prompt = crate::coordination::notification_prompt(&fixture.candidate);
    let expected_composer = format!("{old_prompt} operator addition");
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        expected_composer
    );

    fs::remove_file(fixture.temp.path().join("draft-after-paste")).unwrap();
    send_notification_fixture(
        &fixture.state,
        &fixture.capability,
        "draft-wake-mixed-recovery-fixture",
    )
    .await;
    let candidate = crate::coordination::pending_notifications(&fixture.state.context)
        .expect("pending recovery generation")
        .pop()
        .expect("recovery generation");
    let tail = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        "provider-beta",
        fixture.temp.path().join("transcript"),
        Duration::ZERO,
    )
    .expect("tail");
    assert!(matches!(
        start_terminal_coordination_notification(
            &fixture.state.context,
            &fixture.state.tmux_bin,
            &candidate,
            &crate::coordination::notification_prompt(&candidate),
            tail,
            AgentKind::Codex,
        ),
        Err(TerminalNotificationStartError::UnknownReason(
            "notification-residual-unobservable-or-changed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        expected_composer,
        "Codex mixed composer content remains untouched"
    );
    assert!(fs::read_to_string(fixture.temp.path().join("calls"))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Vec<String>>(line).ok())
        .all(|args| args.first().map(String::as_str) != Some("send-keys")
            || args.last().map(String::as_str) != Some("Enter")));
}

#[tokio::test]
async fn mailbox_wake_recovers_unverified_composer_and_clears_claude_prompt() {
    mailbox_wake_recovers_unverified_composer_and_clears_prompt(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_recovers_unverified_composer_and_clears_codex_prompt() {
    mailbox_wake_recovers_unverified_composer_and_clears_prompt(AgentKind::Codex).await;
}

async fn mailbox_wake_recovers_unverified_composer_and_clears_prompt(agent: AgentKind) {
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("keep-after-enter"), "").unwrap();
    fs::write(fixture.temp.path().join("unknown-layout-after-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known(
            "submission-not-confirmed"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        "",
        "a recovered composer must be safely cleared for {agent:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1,
        "unverified submission must not be retried with Enter"
    );
}

#[tokio::test]
async fn mailbox_wake_unverified_but_submitted_stays_unknown_claude() {
    mailbox_wake_unverified_but_submitted_stays_unknown(AgentKind::Claude).await;
}

#[tokio::test]
async fn mailbox_wake_unverified_but_submitted_stays_unknown_codex() {
    mailbox_wake_unverified_but_submitted_stays_unknown(AgentKind::Codex).await;
}

async fn mailbox_wake_unverified_but_submitted_stays_unknown(agent: AgentKind) {
    // Enter succeeds and empties the composer, but the pane layout is unreadable
    // while the submission is confirmed. An empty composer after an unverified
    // submission is not evidence of failure, so the outcome must stay Unknown
    // for transcript reconciliation instead of becoming a retryable refusal.
    let fixture = WakeFixture::new(agent).await;
    fs::write(fixture.temp.path().join("unknown-layout-after-enter"), "").unwrap();
    assert!(
        matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Unknown)
        ),
        "a submitted wake with an empty composer must stay Unknown for {agent:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        ""
    );
    assert_eq!(notification_fixture(fixture.temp.path())["state"], "attempting");
}

#[tokio::test]
async fn mailbox_wake_accepts_verified_runtime_exit_after_enter() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    fs::write(fixture.temp.path().join("exit-after-next-enter"), "").unwrap();
    fixture
        .start()
        .expect("verified runtime exit confirms submission");
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

mod notices {
    include!("mailbox_notice_terminal.rs");
}
