// Controller and authenticated hook behavior; transport acceptance is separate.
use super::*;
use pretty_assertions::assert_eq;

fn setup(root: &Path) -> (Arc<ServeState>, String, String, String) {
    seed_session_with_runtime(root, "alpha", "codex", "hs-alpha");
    let launch = seed_codex_app_server_session(root, "beta");
    let state = state(root, Some(TOKEN), minimal_tmux(root));
    let alpha = load_session_record(&state.context, "alpha").unwrap();
    let beta = load_session_record(&state.context, "beta").unwrap();
    let sender = provision_ready_coordination_fixture(&state.context, &alpha);
    let recipient = provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).unwrap();
    nils_common::fs::write_atomic(
        &root.join("coordination/notification-policy.json"),
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.notification-policy.v1",
            "recipients": [{"session_id": "beta", "routine_interval_seconds": 600, "suppress_categories": []}]
        })).unwrap(),
        0o600,
    ).unwrap();
    (state, launch, sender, recipient)
}

fn turn(state: &ServeState, launch: &str, working: bool) {
    for kind in if working {
        vec!["turn_started"]
    } else {
        vec!["turn_started", "turn_completed"]
    } {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": format!("notice-{kind}"), "runtime_id": launch,
            "provider": "codex", "provider_turn_id": "notice-turn",
            "kind": kind, "confidence": "authoritative"
        }))
        .unwrap();
        crate::activity::ingest_event(&state.context, "beta", event).unwrap();
    }
}

async fn mail(state: &Arc<ServeState>, sender: &str) {
    for (i, category) in ["handoff", "report", "report"].into_iter().enumerate() {
        let (status, result) = call(
            router(state.clone()),
            post_coordination(
                "/sessions/beta/messages/v1",
                sender.trim(),
                json!({
                    "body": "PRIVATE-MESSAGE-CANARY", "idempotency_key": format!("notice-mail-{i}"),
                    "category": category
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
    }
}

fn assert_notice(text: &str) {
    let parsed = crate::notice::parse_notice(text);
    assert!(
        parsed.is_some(),
        "notice-capable recipient received plain prompt: {text:?}"
    );
    let parsed = parsed.unwrap();
    assert_eq!(parsed.metadata.kind(), "mailbox.wake");
    assert_eq!(parsed.metadata.source(), "agent-session");
    assert_eq!(parsed.body, "Mailbox: h=1 r=2");
}

#[derive(Clone, Copy)]
enum Outcome {
    Accepted,
    Rejected,
    Dropped,
}

async fn controller(working: bool, writers: bool, outcome: Outcome) {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(
        &lock,
        crate::notice::WRITERS_ENV,
        if writers { "1" } else { "0" },
    );
    let tmp = tempfile::tempdir().unwrap();
    let (state, launch, sender, _) = setup(tmp.path());
    turn(&state, &launch, working);
    mail(&state, &sender).await;
    let (handle, mut commands) = codex_app_server::control_channel();
    state.codex_controls.lock().unwrap().insert(
        "beta".into(),
        CodexControlEntry {
            launch_id: launch,
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let command = commands.recv().await.unwrap();
        let message = match command {
            codex_app_server::ControlCommand::Prompt { message, response } => {
                assert!(!working);
                match outcome {
                    Outcome::Accepted => response.send(Ok("notice-accepted".into())).unwrap(),
                    Outcome::Rejected => response
                        .send(Err(codex_app_server::prompt_submit::not_confirmed(
                            "provider rejected notice",
                        )))
                        .unwrap(),
                    Outcome::Dropped => drop(response),
                }
                message
            }
            codex_app_server::ControlCommand::Steer {
                message,
                expected_turn_id,
                response,
            } => {
                assert!(working);
                match outcome {
                    Outcome::Accepted => response.send(Ok(expected_turn_id)).unwrap(),
                    Outcome::Rejected => response
                        .send(Err("provider rejected notice".into()))
                        .unwrap(),
                    Outcome::Dropped => drop(response),
                }
                message
            }
            _ => panic!("unexpected controller command"),
        };
        (message, commands)
    });
    tokio::join!(
        drain_coordination_notifications(state.clone()),
        drain_coordination_notifications(state.clone())
    );
    let (message, mut commands) = responder.await.unwrap();
    drain_coordination_notifications(state.clone()).await;
    assert!(
        commands.try_recv().is_err(),
        "one generation must be delivered once"
    );
    assert_eq!(
        notification_fixture(tmp.path())["notified_generation"],
        if matches!(outcome, Outcome::Accepted) {
            3
        } else {
            0
        }
    );
    assert!(
        claim(&state)["reminder"].is_null(),
        "controller attempt must exclude hook delivery"
    );
    if writers {
        assert_notice(&message);
    } else {
        assert_eq!(message, "Mailbox: h=1 r=2");
    }
    if !matches!(outcome, Outcome::Accepted) {
        assert_eq!(notification_fixture(tmp.path())["state"], "attempt_unknown");
    }
}

#[tokio::test]
async fn mailbox_notice_idle_controller_delivers_typed_counts() {
    controller(false, true, Outcome::Accepted).await;
}

#[tokio::test]
async fn mailbox_notice_working_controller_delivers_typed_counts() {
    controller(true, true, Outcome::Accepted).await;
}

#[tokio::test]
async fn mailbox_notice_hook_claim_delivers_typed_counts_once() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::tempdir().unwrap();
    let (state, _, sender, _) = setup(tmp.path());
    mail(&state, &sender).await;
    let record = load_session_record(&state.context, "beta").unwrap();
    let capability = crate::coordination::capability_path(
        &state.context,
        "beta",
        &crate::coordination::incarnation(&record).unwrap(),
    );
    let claim = || {
        crate::coordination::mailbox::reminder(
            &state.context,
            crate::cli::MessageReminderArgs {
                session: "beta".into(),
                capability_file: Some(capability.clone()),
                format: nils_common::cli_contract::OutputFormat::Json,
            },
        )
        .unwrap()
    };
    let first = claim();
    assert!(claim()["reminder"].is_null());
    drain_coordination_notifications(state.clone()).await;
    assert_eq!(notification_fixture(tmp.path())["notified_generation"], 3);
    assert_notice(first["reminder"].as_str().unwrap());
}

fn claim(state: &ServeState) -> Value {
    let record = load_session_record(&state.context, "beta").unwrap();
    let capability = crate::coordination::capability_path(
        &state.context,
        "beta",
        &crate::coordination::incarnation(&record).unwrap(),
    );
    crate::coordination::mailbox::reminder(
        &state.context,
        crate::cli::MessageReminderArgs {
            session: "beta".into(),
            capability_file: Some(capability),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn mailbox_notice_rollout_off_preserves_explicit_prompt_route() {
    for working in [false, true] {
        controller(working, false, Outcome::Accepted).await;
    }
}

#[tokio::test]
async fn mailbox_notice_rejected_and_unknown_never_fall_back_or_repeat() {
    for working in [false, true] {
        for outcome in [Outcome::Rejected, Outcome::Dropped] {
            controller(working, true, outcome).await;
        }
    }
}

#[tokio::test]
async fn mailbox_notice_stale_generation_cannot_win_against_hook_claim() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::tempdir().unwrap();
    let (state, _, sender, _) = setup(tmp.path());
    send_notification_fixture(&state, &sender, "notice-old-generation").await;
    let stale = crate::coordination::pending_notifications(&state.context)
        .unwrap()
        .pop()
        .unwrap();
    mail(&state, &sender).await;
    let mut guard = crate::coordination::lock_session_quiescence(
        &state.context,
        "beta",
        &stale.target_incarnation,
    )
    .unwrap();
    assert!(!guard.begin_notification_attempt(&stale).unwrap());
    drop(guard);
    let first = claim(&state);
    let parsed = crate::notice::parse_notice(first["reminder"].as_str().unwrap()).unwrap();
    assert_eq!(parsed.metadata.kind(), "mailbox.wake");
    assert_eq!(parsed.body, "Mailbox: h=1 r=2 u=1");
    assert!(claim(&state)["reminder"].is_null());
}

#[tokio::test]
async fn mailbox_notice_hook_counts_only_live_unread_and_counts_unconfigured_mail() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::tempdir().unwrap();
    let (state, _, sender, _) = setup(tmp.path());
    mail(&state, &sender).await;
    let path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["messages"][1]["state"] = json!("read");
    registry["messages"][2]["expires_at_epoch"] = json!(0);
    fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let result = claim(&state);
    let notice = crate::notice::parse_notice(result["reminder"].as_str().unwrap()).unwrap();
    assert_eq!(notice.body, "Mailbox: h=1");

    let other = tempfile::tempdir().unwrap();
    let (state, _, sender, _) = setup(other.path());
    fs::remove_file(other.path().join("coordination/notification-policy.json")).unwrap();
    send_notification_fixture(&state, &sender, "notice-unconfigured").await;
    let result = claim(&state);
    let notice = crate::notice::parse_notice(result["reminder"].as_str().unwrap()).unwrap();
    assert_eq!(notice.body, "Mailbox: u=1");
}
