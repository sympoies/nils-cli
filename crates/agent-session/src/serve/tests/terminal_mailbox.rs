use pretty_assertions::assert_eq;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coordination_notification_reenters_detached_terminal_codex_once() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("transcript dir");
    let provider_id = "terminal-notification-reentry";
    write_codex_notification_transcript(&transcript, provider_id, &[]);
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());

    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_fresh_provider_session(
        tmp.path(),
        "beta",
        "codex",
        "hs-codex-beta",
        tmp.path(),
        Some((provider_id, "codex-explicit-session-id")),
    );
    let command_log = tmp.path().join("tmux-commands.log");
    let captured_prompt = tmp.path().join("captured-prompt");
    let pasted = tmp.path().join("pasted");
    let runtime_race = tmp.path().join("runtime-race");
    let coordination_registry = tmp.path().join("coordination/registry.json");
    let tmux = tmp.path().join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$1" in
  has-session)
if [ -f {} ] && grep -q '"state"[[:space:]]*:[[:space:]]*"attempting"' {}; then exit 1; fi
exit 0
;;
  display-message) printf '0\n'; exit 0 ;;
  load-buffer)
for value in "$@"; do last="$value"; done
cp "$last" {}
exit 0
;;
  paste-buffer) printf '1' > {}; exit 0 ;;
  send-keys) : > {}; exit 0 ;;
  delete-buffer) exit 0 ;;
  capture-pane)
    value=""
    if [ -s {} ]; then value=$(cat {}); fi
    printf '› %s\n' "$value" | fold -w 51
    printf '\n  status\n'
    exit 0 ;;
  *) exit 1 ;;
esac
"#,
            shell_words::quote(&command_log.to_string_lossy()),
            shell_words::quote(&runtime_race.to_string_lossy()),
            shell_words::quote(&coordination_registry.to_string_lossy()),
            shell_words::quote(&captured_prompt.to_string_lossy()),
            shell_words::quote(&pasted.to_string_lossy()),
            shell_words::quote(&pasted.to_string_lossy()),
            shell_words::quote(&pasted.to_string_lossy()),
            shell_words::quote(&captured_prompt.to_string_lossy()),
        ),
    )
    .expect("tmux script");
    let mut permissions = fs::metadata(&tmux).expect("tmux metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&tmux, permissions).expect("tmux permissions");

    let state = state(tmp.path(), Some(TOKEN), tmux);
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let launch_id = beta
        .runtime
        .as_ref()
        .expect("beta runtime")
        .launch_id
        .clone();
    for (event_id, kind) in [
        ("terminal-notification-turn-start", "turn_started"),
        ("terminal-notification-turn-complete", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_session_id": provider_id,
            "provider_turn_id": "terminal-notification-idle-turn",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .expect("activity event");
        crate::activity::ingest_event(&state.context, &beta.id, event).expect("ingest");
    }

    let canary = "TERMINAL-CODEX-MAILBOX-BODY-MUST-NOT-REACH-PROMPT";
    let (status, sent) = call(
        router(state.clone()),
        post_coordination(
            "/sessions/beta/messages/v1",
            alpha_capability.trim(),
            json!({
                "body": canary,
                "idempotency_key": "terminal-notification-reentry-0001",
                "reply_to": null,
                "expires_in": null
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    let candidate = crate::coordination::pending_notifications(&state.context)
        .expect("pending notification")
        .pop()
        .expect("notification candidate");
    fs::write(&runtime_race, b"stop after attempt\n").expect("runtime race");
    let prompt = crate::coordination::notification_prompt(&candidate);
    let guarded_tail = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        provider_id,
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    assert!(matches!(
        start_terminal_coordination_notification(
            &state.context,
            &state.tmux_bin,
            &candidate,
            &prompt,
            guarded_tail,
            AgentKind::Codex,
        ),
        Err(TerminalNotificationStartError::Known("provider-not-ready"))
    ));
    assert!(!captured_prompt.exists());
    assert_eq!(notification_fixture(tmp.path())["state"], "queued");
    fs::remove_file(&runtime_race).expect("clear runtime race");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&coordination_registry).expect("registry"))
            .expect("registry json");
    registry["notifications"]
        .as_object_mut()
        .expect("notifications")
        .values_mut()
        .next()
        .expect("notification")["next_attempt_at_epoch"] = json!(0);
    fs::write(
        &coordination_registry,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("reset retry boundary");

    let mut registry: Value =
        serde_json::from_slice(&fs::read(&coordination_registry).expect("registry"))
            .expect("registry json");
    registry["claims"]
        .as_array_mut()
        .expect("claims")
        .push(json!({
            "schema_version": "agent-session.work-context.v1",
            "session_id": "beta",
            "session_incarnation": beta.runtime.as_ref().expect("runtime").launch_id,
            "claim_id": "terminal-notification-race-claim",
            "revision": 1,
            "state": "active",
            "intent": "implementation",
            "tier": "direct",
            "repositories": [],
            "worktrees": [],
            "provider_refs": [],
            "plan_refs": [],
            "scopes": [],
            "summary": "terminal notification race fixture",
            "updated_at": "2030-01-01T00:00:00Z",
            "expires_at": "9999-12-31T23:59:59Z",
            "expires_at_epoch": i64::MAX
        }));
    fs::write(
        &coordination_registry,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write claim");
    let guarded_tail = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        provider_id,
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    assert!(matches!(
        start_terminal_coordination_notification(
            &state.context,
            &state.tmux_bin,
            &candidate,
            &prompt,
            guarded_tail,
            AgentKind::Codex,
        ),
        Err(TerminalNotificationStartError::Known(
            "recipient-coordination-not-quiescent"
        ))
    ));
    assert!(!captured_prompt.exists());
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&coordination_registry).expect("registry"))
            .expect("registry json");
    registry["claims"] = json!([]);
    registry["operations"]
        .as_array_mut()
        .expect("operations")
        .push(json!({
            "schema_version": "agent-session.operation-lease.v1",
            "lease_id": "terminal-notification-race-operation",
            "session_id": "beta",
            "session_incarnation": beta.runtime.as_ref().expect("runtime").launch_id,
            "claim_id": "terminal-notification-race-claim",
            "claim_revision": 1,
            "operation": "test mutation",
            "targets": [],
            "provider_targets": [],
            "state": "active",
            "revision": 1,
            "started_at": "2030-01-01T00:00:00Z",
            "expires_at": "9999-12-31T23:59:59Z",
            "expires_at_epoch": i64::MAX,
            "terminal_at_epoch": null,
            "execution_token_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "activity_revision": 1,
            "activity_identity_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runtime_identity_digest": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "descendant": null,
            "reconcile_observed_at_epoch": null,
            "outcome": null
        }));
    fs::write(
        &coordination_registry,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("replace claim with operation");
    let guarded_tail = ProviderPromptTail::open_path(
        ProviderKind::Codex,
        provider_id,
        transcript.clone(),
        Duration::ZERO,
    )
    .expect("tail");
    assert!(matches!(
        start_terminal_coordination_notification(
            &state.context,
            &state.tmux_bin,
            &candidate,
            &prompt,
            guarded_tail,
            AgentKind::Codex,
        ),
        Err(TerminalNotificationStartError::Known(
            "recipient-coordination-not-quiescent"
        ))
    ));
    assert!(!captured_prompt.exists());
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&coordination_registry).expect("registry"))
            .expect("registry json");
    registry["operations"] = json!([]);
    fs::write(
        &coordination_registry,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("clear operation");

    let observed_transcript = transcript.clone();
    let observed_prompt = captured_prompt.clone();
    let observer = tokio::spawn(async move {
        for _ in 0..100 {
            if observed_prompt.exists() {
                let prompt =
                    fs::read_to_string(&observed_prompt).expect("captured notification prompt");
                let observed = format!(
                    "{}\n",
                    json!({
                        "type": "event_msg",
                        "timestamp": jiff::Timestamp::now().to_string(),
                        "payload": {"type": "user_message", "message": prompt}
                    })
                );
                fs::OpenOptions::new()
                    .append(true)
                    .open(&observed_transcript)
                    .expect("open transcript")
                    .write_all(observed.as_bytes())
                    .expect("append observed prompt");
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("notification prompt was not submitted");
    });

    drain_coordination_notifications(state.clone()).await;
    observer.await.expect("prompt observer");

    assert_eq!(
        fs::read_to_string(&captured_prompt).expect("captured prompt"),
        prompt
    );
    assert!(!prompt.contains(canary));
    let commands = fs::read_to_string(&command_log).expect("command log");
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.starts_with("send-keys "))
            .count(),
        1,
        "one notification generation must send Enter exactly once"
    );
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "prompt_submitted");
    assert_eq!(notification["generation"], 1);
    assert_eq!(notification["notified_generation"], 1);

    send_notification_fixture(
        &state,
        &alpha_capability,
        "terminal-notification-reentry-timeout-0002",
    )
    .await;
    // Identical wake bytes still belong to separate, timestamp-fenced attempts.
    assert_eq!(fixture_notification_prompt(tmp.path()), prompt);
    drain_coordination_notifications(state.clone()).await;
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "attempt_unknown");
    assert_eq!(notification["generation"], 2);
    assert_eq!(notification["notified_generation"], 1);
    let enter_count_after_unknown = fs::read_to_string(&command_log)
        .expect("command log")
        .lines()
        .filter(|line| line.starts_with("send-keys "))
        .count();
    assert_eq!(enter_count_after_unknown, 2);
    // Generation one's delivered line cannot prove generation two, so an
    // undated exact match of the new prompt makes the outcome ambiguous;
    // the attempt must stay parked instead of pressing Enter again.
    let second_prompt = fixture_notification_prompt(tmp.path());
    fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("open transcript")
        .write_all(
            format!(
                "{}\n",
                json!({
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": second_prompt}
                })
            )
            .as_bytes(),
        )
        .expect("append ambiguous prompt");
    drain_coordination_notifications(state.clone()).await;
    assert_eq!(
        fs::read_to_string(&command_log)
            .expect("command log")
            .lines()
            .filter(|line| line.starts_with("send-keys "))
            .count(),
        enter_count_after_unknown,
        "an unresolved terminal submission must not send another Enter"
    );
    assert_eq!(notification_fixture(tmp.path())["state"], "attempt_unknown");
}

#[tokio::test]
async fn claude_notification_rechecks_attach_after_attempt_before_paste() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
    seed_session_with_runtime(tmp.path(), "beta", "claude", "hs-claude-beta");
    let command_log = tmp.path().join("tmux-commands.log");
    let captured_prompt = tmp.path().join("captured-prompt");
    let registry = tmp.path().join("coordination/registry.json");
    let tmux = tmp.path().join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$1" in
  has-session) exit 0 ;;
  display-message)
if grep -q '"state"[[:space:]]*:[[:space:]]*"attempting"' {}; then
  printf '1\n'
else
  printf '0\n'
fi
exit 0
;;
  capture-pane)
    printf '────────────────────\n❯ \n────────────────────\n  status\n'; exit 0 ;;
  load-buffer)
for value in "$@"; do last="$value"; done
cp "$last" {}
exit 0
;;
  paste-buffer|send-keys|delete-buffer) exit 0 ;;
  *) exit 1 ;;
esac
"#,
            shell_words::quote(&command_log.to_string_lossy()),
            shell_words::quote(&registry.to_string_lossy()),
            shell_words::quote(&captured_prompt.to_string_lossy())
        ),
    )
    .expect("tmux script");
    let mut permissions = fs::metadata(&tmux).expect("tmux metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&tmux, permissions).expect("tmux permissions");

    let state = state(tmp.path(), Some(TOKEN), tmux.clone());
    let alpha = load_session_record(&state.context, "alpha").expect("alpha");
    let beta = load_session_record(&state.context, "beta").expect("beta");
    let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
    provision_ready_coordination_fixture(&state.context, &beta);
    crate::activity::activate_runtime(&state.context, &beta).expect("activate beta");
    let stop = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "claude-notification-attach-race-stop",
        "runtime_id": beta.runtime.as_ref().expect("beta runtime").launch_id,
        "provider": "claude",
        "kind": "stop_observed",
        "confidence": "observed"
    }))
    .expect("stop event");
    crate::activity::ingest_event(&state.context, &beta.id, stop).expect("ingest stop");
    let activity_path = session_dir(&state.context, &beta.id).join("activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&activity_path).expect("activity"))
        .expect("activity json");
    activity["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(
        &activity_path,
        serde_json::to_vec_pretty(&activity).expect("serialize activity"),
    )
    .expect("write activity");

    send_notification_fixture(&state, &alpha_capability, "claude-notification-attach-race").await;
    let candidate = crate::coordination::pending_notifications(&state.context)
        .expect("pending notifications")
        .pop()
        .expect("candidate");
    let transcript = tmp.path().join("claude.jsonl");
    fs::write(&transcript, b"").expect("transcript");
    let tail = ProviderPromptTail::open_path(
        ProviderKind::Claude,
        "provider-beta",
        transcript,
        Duration::ZERO,
    )
    .expect("tail");
    let prompt = crate::coordination::notification_prompt(&candidate);

    assert!(matches!(
        start_terminal_coordination_notification(
            &state.context,
            &tmux,
            &candidate,
            &prompt,
            tail,
            AgentKind::Claude,
        ),
        Err(TerminalNotificationStartError::Known("recipient-attached"))
    ));
    assert!(
        !captured_prompt.exists(),
        "the immediate attach fence must run before any buffer load or paste"
    );
    let commands = fs::read_to_string(&command_log).expect("command log");
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.starts_with("display-message "))
            .count(),
        3,
        "attach is checked for eligibility, after composer observation, and after attempt persistence"
    );
    assert!(!commands.contains("load-buffer"));
    assert!(!commands.contains("paste-buffer"));
    assert!(!commands.contains("send-keys"));
    let notification = notification_fixture(tmp.path());
    assert_eq!(notification["state"], "queued");
    assert_eq!(notification["last_reason"], "recipient-attached");
}
