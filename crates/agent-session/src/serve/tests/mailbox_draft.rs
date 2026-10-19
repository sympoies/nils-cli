use pretty_assertions::assert_eq;

use super::*;

struct WakeFixture {
    temp: tempfile::TempDir,
    state: Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
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
if command == 'load-buffer':
    (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
    if (root / 'draft-on-load').exists(): (root / 'composer').write_text('later operator draft')
elif command == 'paste-buffer':
    with (root / 'composer').open('a') as out: out.write((root / 'buffer').read_text())
    if (root / 'draft-after-paste').exists():
        with (root / 'composer').open('a') as out: out.write(' operator addition')
elif command == 'send-keys' and args[-1] == 'Enter':
    with (root / 'submissions').open('a') as out: out.write(json.dumps((root / 'composer').read_text()) + '\n')
    if (root / 'exit-after-next-enter').exists(): (root / 'exited-after-enter').touch()
    if (root / 'liveness-unknown-after-enter').exists(): (root / 'unknown-after-enter').touch()
    if (root / 'operator-after-first-enter').exists() and not (root / 'first-enter-seen').exists():
        (root / 'first-enter-seen').touch()
        with (root / 'composer').open('a') as out: out.write(' operator addition')
    elif (root / 'swallow-first-enter').exists(): (root / 'swallow-first-enter').unlink()
    elif (root / 'swallow-every-enter').exists(): pass
    else: (root / 'composer').write_text('')
elif command == 'capture-pane':
    if (root / 'capture-unavailable').exists() or ((root / 'capture-unavailable-after-enter').exists() and (root / 'submissions').exists()): sys.exit(1)
    text = (root / 'composer').read_text()
    if (root / 'provider').read_text() == 'claude':
        print('────────────────────\n❯\u00a0' + text + '\n────────────────────\n  status')
    else: print('› ' + text + '\n\n  status')
elif command != 'delete-buffer': sys.exit(1)
"#).expect("tmux");
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).expect("permissions");
        let state = state(temp.path(), Some(TOKEN), tmux);
        let alpha = load_session_record(&state.context, "alpha").expect("sender");
        let beta = load_session_record(&state.context, "beta").expect("recipient");
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
    assert!(
        matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Unknown)
        ),
        "changed composer must remain uncertain {agent:?}"
    );
    assert!(!fixture.temp.path().join("submissions").exists());
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        format!(
            "{} operator addition",
            crate::coordination::notification_prompt(&fixture.candidate)
        )
    );
    assert!(
        !fs::read_to_string(fixture.temp.path().join("calls"))
            .unwrap()
            .contains("send-keys")
    );
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
}

#[tokio::test]
async fn mailbox_wake_keeps_unreadable_pane_unverified_after_enter() {
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    fs::write(
        fixture.temp.path().join("capture-unavailable-after-enter"),
        "",
    )
    .unwrap();
    fs::write(fixture.temp.path().join("liveness-unknown-after-enter"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Unknown)
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
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
